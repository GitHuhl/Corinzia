#include "corinzia/storage/ProjectStore.hpp"

#include <sqlite3.h>

#include <algorithm>
#include <cstdio>
#include <cstring>
#include <map>
#include <sstream>

namespace corinzia::storage {

extern const char* const kSchemaSql;

namespace {

// ------------------------------------------------------------ tiny RAII --
class Stmt {
public:
    Stmt(sqlite3* db, const char* sql) { ok_ = sqlite3_prepare_v2(db, sql, -1, &s_, nullptr) == SQLITE_OK; }
    ~Stmt() { sqlite3_finalize(s_); }
    Stmt(const Stmt&) = delete;
    Stmt& operator=(const Stmt&) = delete;

    bool ok() const { return ok_; }
    Stmt& bind(int i, std::int64_t v) { sqlite3_bind_int64(s_, i, v); return *this; }
    Stmt& bind(int i, std::uint64_t v) { return bind(i, std::int64_t(v)); }
    Stmt& bind(int i, int v) { return bind(i, std::int64_t(v)); }
    Stmt& bind(int i, double v) { sqlite3_bind_double(s_, i, v); return *this; }
    Stmt& bind(int i, const std::string& v) {
        sqlite3_bind_text(s_, i, v.data(), int(v.size()), SQLITE_TRANSIENT);
        return *this;
    }
    Stmt& bindNull(int i) { sqlite3_bind_null(s_, i); return *this; }
    Stmt& bindBlob(int i, const void* p, std::size_t n) {
        sqlite3_bind_blob64(s_, i, p, n, SQLITE_TRANSIENT);
        return *this;
    }
    bool step() { return sqlite3_step(s_) == SQLITE_ROW; }
    bool run() {
        const int rc = sqlite3_step(s_);
        sqlite3_reset(s_);
        sqlite3_clear_bindings(s_);
        return rc == SQLITE_DONE || rc == SQLITE_ROW;
    }
    void reset() { sqlite3_reset(s_); sqlite3_clear_bindings(s_); }
    std::int64_t i64(int c) const { return sqlite3_column_int64(s_, c); }
    double f64(int c) const { return sqlite3_column_double(s_, c); }
    bool isNull(int c) const { return sqlite3_column_type(s_, c) == SQLITE_NULL; }
    std::string text(int c) const {
        auto* p = sqlite3_column_text(s_, c);
        return p ? std::string(reinterpret_cast<const char*>(p), std::size_t(sqlite3_column_bytes(s_, c))) : std::string();
    }
    std::vector<std::byte> blob(int c) const {
        auto* p = static_cast<const std::byte*>(sqlite3_column_blob(s_, c));
        return p ? std::vector<std::byte>(p, p + sqlite3_column_bytes(s_, c)) : std::vector<std::byte>{};
    }

private:
    sqlite3_stmt* s_ = nullptr;
    bool ok_ = false;
};

// ------------------------------------------------------- JSON writing --
std::string jnum(double v) { char b[32]; std::snprintf(b, sizeof b, "%.17g", v); return b; }
std::string jfrac(Fraction f) { return "[" + std::to_string(f.num) + "," + std::to_string(f.den) + "]"; }

template <class T, class F>
std::string jarray(const std::vector<T>& v, F&& f) {
    std::string o = "[";
    for (std::size_t i = 0; i < v.size(); ++i) o += (i ? "," : "") + f(v[i]);
    return o + "]";
}

template <class Id>
std::string idArray(const std::vector<Id>& ids) {
    return jarray(ids, [](const Id& i) { return std::to_string(std::int64_t(i.id.value)); });
}

const char* curveName(model::CurveShape c) {
    switch (c) {
        case model::CurveShape::Step: return "step";
        case model::CurveShape::Linear: return "lin";
        case model::CurveShape::Exponential: return "exp";
        case model::CurveShape::SCurve: return "s";
    }
    return "lin";
}
model::CurveShape curveFrom(const std::string& s) {
    if (s == "step") return model::CurveShape::Step;
    if (s == "exp") return model::CurveShape::Exponential;
    if (s == "s") return model::CurveShape::SCurve;
    return model::CurveShape::Linear;
}
const char* nodeKindName(model::NodeKind k) {
    return k == model::NodeKind::ClapPlugin ? "clap" : k == model::NodeKind::MasterOut ? "master" : "bus";
}
model::NodeKind nodeKindFrom(const std::string& s) {
    return s == "clap" ? model::NodeKind::ClapPlugin : s == "master" ? model::NodeKind::MasterOut : model::NodeKind::Bus;
}

std::uint64_t fnv1a(const std::vector<std::byte>& v) {
    std::uint64_t h = 1469598103934665603ull;
    for (auto b : v) { h ^= std::uint64_t(b); h *= 1099511628211ull; }
    return h;
}

}  // namespace

// ================================================================== API ==
std::int64_t chunkIndexOf(Fraction start) noexcept {
    // floor(start / span) for non-negative positions
    return start.num / (start.den * kChunkSpanWholeNotes);
}

std::string eventsToJson(const model::EventList& evs, std::size_t begin, std::size_t end) {
    std::string o = "[";
    for (std::size_t i = begin; i < end; ++i) {
        const auto& e = evs[i];
        const char* k = e.kind == model::EventKind::Rest ? "r" : e.kind == model::EventKind::Chord ? "c" : "n";
        o += (i > begin ? "," : "");
        o += "{\"id\":" + std::to_string(e.id.id.value) + ",\"k\":\"" + k + "\",\"s\":" + jfrac(e.start) +
             ",\"d\":" + jfrac(e.duration) + ",\"a\":" + std::to_string(e.articulations);
        if (e.dynamicMark) o += ",\"dyn\":" + std::to_string(*e.dynamicMark);
        if (e.spans) o += ",\"sp\":" + std::to_string(e.spans);
        o += ",\"h\":" + jarray(e.heads, [](const model::NoteHead& h) {
                 return "{\"id\":" + std::to_string(h.id.id.value) + ",\"p\":[" + std::to_string(h.pitch.step) + "," +
                        std::to_string(h.pitch.alter) + "," + std::to_string(h.pitch.octave) +
                        "],\"t\":" + (h.tieToNext ? "1" : "0") + ",\"v\":" + std::to_string(h.velocity) + "}";
             });
        o += "}";
    }
    return o + "]";
}

ProjectHeader ProjectHeader::capture(const model::Project& p) {
    ProjectHeader h;
    h.meta = p.meta;
    h.nextId = p.nextId();
    for (const auto& pl : p.players) {
        PlayerRow row;
        row.id = pl->id;
        row.name = pl->name;
        for (const auto& i : pl->instruments) row.instruments.push_back(*i);
        h.players.push_back(std::move(row));
    }
    for (const auto& t : p.tracks) h.tracks.push_back(*t);
    h.graph = p.graph;
    return h;
}

std::unique_ptr<ProjectStore> ProjectStore::open(const std::string& path, std::string* error) {
    sqlite3* db = nullptr;
    if (sqlite3_open_v2(path.c_str(), &db, SQLITE_OPEN_READWRITE | SQLITE_OPEN_CREATE | SQLITE_OPEN_NOMUTEX,
                        nullptr) != SQLITE_OK) {
        if (error) *error = db ? sqlite3_errmsg(db) : "sqlite3_open failed";
        sqlite3_close(db);
        return nullptr;
    }
    std::unique_ptr<ProjectStore> s(new ProjectStore(db));
    // NORMAL is durable across application crashes in WAL mode (only an OS
    // crash/power loss can drop the last commit) and keeps autosave cheap.
    if (!s->exec(kSchemaSql, error) || !s->exec("PRAGMA synchronous = NORMAL;", error)) return nullptr;
    return s;
}

ProjectStore::~ProjectStore() { sqlite3_close(db_); }

bool ProjectStore::exec(const char* sql, std::string* error) {
    char* msg = nullptr;
    if (sqlite3_exec(db_, sql, nullptr, nullptr, &msg) != SQLITE_OK) {
        if (error) *error = msg ? msg : "sqlite error";
        sqlite3_free(msg);
        return false;
    }
    return true;
}

std::int64_t ProjectStore::countRows(const char* table) {
    Stmt s(db_, (std::string("SELECT count(*) FROM ") + table).c_str());
    return s.ok() && s.step() ? s.i64(0) : -1;
}

bool ProjectStore::save(const SaveBatch& b, std::string* error) {
    if (!exec("BEGIN IMMEDIATE;", error)) return false;
    bool ok = true;
    if (b.header) ok = ok && saveHeader(*b.header);
    for (auto m : b.deletedMovements) {
        Stmt d(db_, "DELETE FROM movement WHERE id = ?");
        ok = ok && d.bind(1, m.id.value).run();
    }
    for (const auto& ms : b.movements)
        if (ms.movement) ok = ok && saveMovement(*ms.movement, b.dirtyVoices);
    for (const auto& [node, state] : b.pluginStates) ok = ok && savePluginState(node, state);
    if (ok) {
        Stmt t(db_, "UPDATE project SET modified_at = strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id = 1");
        ok = t.run();
    }
    if (!ok) {
        if (error) *error = sqlite3_errmsg(db_);
        exec("ROLLBACK;");
        return false;
    }
    return exec("COMMIT;", error);
}

bool ProjectStore::saveHeader(const ProjectHeader& h) {
    bool ok = true;
    {
        Stmt s(db_, "INSERT INTO project(id, title, composer, sample_rate, next_id) VALUES (1, ?, ?, ?, ?) "
                    "ON CONFLICT(id) DO UPDATE SET title=excluded.title, composer=excluded.composer, "
                    "sample_rate=excluded.sample_rate, next_id=excluded.next_id");
        ok = ok && s.bind(1, h.meta.title).bind(2, h.meta.composer).bind(3, h.meta.sampleRate).bind(4, h.nextId).run();
    }
    // Nodes first (tracks/instruments reference them). Upsert + delete-missing,
    // never delete-all: cascades would wipe plugin_state and automation lanes.
    {
        Stmt s(db_, "INSERT INTO audio_node(id, kind, name, clap_bundle, clap_plugin_id, bypassed) VALUES (?,?,?,?,?,?) "
                    "ON CONFLICT(id) DO UPDATE SET kind=excluded.kind, name=excluded.name, clap_bundle=excluded.clap_bundle, "
                    "clap_plugin_id=excluded.clap_plugin_id, bypassed=excluded.bypassed");
        std::vector<NodeId> ids;
        for (const auto& n : h.graph.nodes) {
            ok = ok && s.bind(1, n.id.id.value).bind(2, std::string(nodeKindName(n.kind))).bind(3, n.name)
                           .bind(4, n.clapBundlePath).bind(5, n.clapPluginId).bind(6, int(n.bypassed)).run();
            if (n.stateChunk) ok = ok && savePluginState(n.id, *n.stateChunk);
            ids.push_back(n.id);
        }
        Stmt d(db_, "DELETE FROM audio_node WHERE id NOT IN (SELECT value FROM json_each(?))");
        ok = ok && d.bind(1, idArray(ids)).run();
    }
    {
        Stmt d(db_, "DELETE FROM audio_edge");
        ok = ok && d.run();
        Stmt s(db_, "INSERT INTO audio_edge(from_node, from_port, to_node, to_port, gain) VALUES (?,?,?,?,?)");
        for (const auto& e : h.graph.edges)
            ok = ok && s.bind(1, e.from.id.value).bind(2, int(e.fromPort)).bind(3, e.to.id.value).bind(4, int(e.toPort))
                           .bind(5, double(e.gain)).run();
    }
    {
        Stmt s(db_, "INSERT INTO track(id, ordinal, name, instrument_node_id, note_port, midi_channel, gain_db, pan, mute, "
                    "solo, output_bus_id) VALUES (?,?,?,?,?,?,?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET "
                    "ordinal=excluded.ordinal, name=excluded.name, instrument_node_id=excluded.instrument_node_id, "
                    "note_port=excluded.note_port, midi_channel=excluded.midi_channel, gain_db=excluded.gain_db, "
                    "pan=excluded.pan, mute=excluded.mute, solo=excluded.solo, output_bus_id=excluded.output_bus_id");
        std::vector<TrackId> ids;
        int ord = 0;
        for (const auto& t : h.tracks) {
            s.bind(1, t.id.id.value).bind(2, ord++).bind(3, t.name);
            t.instrumentNode.valid() ? s.bind(4, t.instrumentNode.id.value) : s.bindNull(4);
            s.bind(5, int(t.notePort)).bind(6, int(t.midiChannel)).bind(7, double(t.gainDb)).bind(8, double(t.pan))
                .bind(9, int(t.mute)).bind(10, int(t.solo));
            t.outputBus.valid() ? s.bind(11, t.outputBus.id.value) : s.bindNull(11);
            ok = ok && s.run();
            ids.push_back(t.id);
        }
        Stmt d(db_, "DELETE FROM track WHERE id NOT IN (SELECT value FROM json_each(?))");
        ok = ok && d.bind(1, idArray(ids)).run();
    }
    {
        Stmt sp(db_, "INSERT INTO player(id, ordinal, name) VALUES (?,?,?) ON CONFLICT(id) DO UPDATE SET "
                     "ordinal=excluded.ordinal, name=excluded.name");
        Stmt si(db_, "INSERT INTO instrument(id, player_id, ordinal, name, engraver_name, transposition, clef, track_id) "
                     "VALUES (?,?,?,?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET player_id=excluded.player_id, "
                     "ordinal=excluded.ordinal, name=excluded.name, engraver_name=excluded.engraver_name, "
                     "transposition=excluded.transposition, clef=excluded.clef, track_id=excluded.track_id");
        std::vector<PlayerId> pids;
        std::vector<InstrumentId> iids;
        int pord = 0;
        for (const auto& row : h.players) {
            ok = ok && sp.bind(1, row.id.id.value).bind(2, pord++).bind(3, row.name).run();
            pids.push_back(row.id);
            int iord = 0;
            for (const auto& i : row.instruments) {
                si.bind(1, i.id.id.value).bind(2, row.id.id.value).bind(3, iord++).bind(4, i.name)
                    .bind(5, i.engraverName).bind(6, i.transpositionSemis).bind(7, i.clef);
                i.track.valid() ? si.bind(8, i.track.id.value) : si.bindNull(8);
                ok = ok && si.run();
                iids.push_back(i.id);
            }
        }
        Stmt di(db_, "DELETE FROM instrument WHERE id NOT IN (SELECT value FROM json_each(?))");
        ok = ok && di.bind(1, idArray(iids)).run();
        Stmt dp(db_, "DELETE FROM player WHERE id NOT IN (SELECT value FROM json_each(?))");
        ok = ok && dp.bind(1, idArray(pids)).run();
    }
    return ok;
}

bool ProjectStore::saveMovement(const model::Movement& m, const std::vector<VoiceId>& dirtyVoices) {
    bool ok = true;
    const auto mid = m.id.id.value;
    {
        const auto tempo = jarray(m.tempo, [](const model::TempoEvent& t) {
            return "{\"at\":" + jfrac(t.at) + ",\"bpm\":" + jnum(t.quarterBpm) + ",\"ramp\":" + (t.rampToNext ? "true" : "false") + "}";
        });
        const auto meter = jarray(m.meter, [](const model::MeterEvent& t) {
            return "{\"at\":" + jfrac(t.at) + ",\"num\":" + std::to_string(t.numerator) + ",\"den\":" + std::to_string(t.denominator) + "}";
        });
        const auto keys = jarray(m.keys, [](const model::KeyEvent& k) {
            return "{\"at\":" + jfrac(k.at) + ",\"fifths\":" + std::to_string(k.fifths) + ",\"minor\":" + (k.minor ? "true" : "false") + "}";
        });
        Stmt s(db_, "INSERT INTO movement(id, ordinal, title, length_num, length_den, revision, tempo, meter, keys) "
                    "VALUES (?,?,?,?,?,?,jsonb(?),jsonb(?),jsonb(?)) ON CONFLICT(id) DO UPDATE SET ordinal=excluded.ordinal, "
                    "title=excluded.title, length_num=excluded.length_num, length_den=excluded.length_den, "
                    "revision=excluded.revision, tempo=excluded.tempo, meter=excluded.meter, keys=excluded.keys");
        ok = ok && s.bind(1, mid).bind(2, m.ordinal).bind(3, m.title).bind(4, m.length.num).bind(5, m.length.den)
                       .bind(6, m.revision).bind(7, tempo).bind(8, meter).bind(9, keys).run();
    }

    std::vector<PlayerId> lanePlayers;
    std::vector<VoiceId> allVoices;
    Stmt sl(db_, "INSERT INTO player_lane(movement_id, player_id, ordinal) VALUES (?,?,?) "
                 "ON CONFLICT DO UPDATE SET ordinal=excluded.ordinal");
    Stmt dc(db_, "DELETE FROM instrument_change WHERE movement_id = ? AND player_id = ?");
    Stmt sc(db_, "INSERT INTO instrument_change(id, movement_id, player_id, at_num, at_den, instrument_id) VALUES (?,?,?,?,?,?)");
    Stmt sv(db_, "INSERT INTO voice(id, movement_id, player_id, voice_index) VALUES (?,?,?,?) "
                 "ON CONFLICT(id) DO UPDATE SET movement_id=excluded.movement_id, player_id=excluded.player_id, "
                 "voice_index=excluded.voice_index");
    // Only rewrites a chunk whose content actually changed (the WHERE on the upsert).
    Stmt sch(db_, "INSERT INTO voice_chunk(voice_id, chunk_index, events) VALUES (?,?,jsonb(?)) "
                  "ON CONFLICT(voice_id, chunk_index) DO UPDATE SET events=excluded.events "
                  "WHERE voice_chunk.events IS NOT excluded.events");
    Stmt dch(db_, "DELETE FROM voice_chunk WHERE voice_id = ? AND chunk_index NOT IN (SELECT value FROM json_each(?))");

    int lord = 0;
    for (const auto& lane : m.lanes) {
        const auto pid = lane.player.id.value;
        ok = ok && sl.bind(1, mid).bind(2, pid).bind(3, lord++).run();
        lanePlayers.push_back(lane.player);
        ok = ok && dc.bind(1, mid).bind(2, pid).run();
        for (const auto& c : lane.changes)
            ok = ok && sc.bind(1, c.id.id.value).bind(2, mid).bind(3, pid).bind(4, c.at.num).bind(5, c.at.den)
                           .bind(6, c.instrument.id.value).run();
        for (const auto& v : lane.voices) {
            ok = ok && sv.bind(1, v.id.id.value).bind(2, mid).bind(3, pid).bind(4, int(v.index)).run();
            allVoices.push_back(v.id);
            const bool dirty = dirtyVoices.empty() ||
                               std::find(dirtyVoices.begin(), dirtyVoices.end(), v.id) != dirtyVoices.end();
            if (!dirty) continue;
            const auto& evs = *v.events;
            std::vector<std::int64_t> chunkIds;
            for (std::size_t i = 0; i < evs.size();) {
                const auto ci = chunkIndexOf(evs[i].start);
                std::size_t j = i;
                while (j < evs.size() && chunkIndexOf(evs[j].start) == ci) ++j;
                ok = ok && sch.bind(1, v.id.id.value).bind(2, ci).bind(3, eventsToJson(evs, i, j)).run();
                chunkIds.push_back(ci);
                i = j;
            }
            ok = ok && dch.bind(1, v.id.id.value)
                           .bind(2, jarray(chunkIds, [](std::int64_t c) { return std::to_string(c); }))
                           .run();
        }
    }
    {
        Stmt d(db_, "DELETE FROM voice WHERE movement_id = ? AND id NOT IN (SELECT value FROM json_each(?))");
        ok = ok && d.bind(1, mid).bind(2, idArray(allVoices)).run();
        Stmt dl(db_, "DELETE FROM player_lane WHERE movement_id = ? AND player_id NOT IN (SELECT value FROM json_each(?))");
        ok = ok && dl.bind(1, mid).bind(2, idArray(lanePlayers)).run();
    }

    // Automation lanes + chunks (same chunking / change-detection scheme).
    Stmt sa(db_, "INSERT INTO automation_lane(id, movement_id, name, node_id, clap_param_id, mode, note_id) VALUES (?,?,?,?,?,?,?) "
                 "ON CONFLICT(id) DO UPDATE SET name=excluded.name, node_id=excluded.node_id, "
                 "clap_param_id=excluded.clap_param_id, mode=excluded.mode, note_id=excluded.note_id");
    Stmt sac(db_, "INSERT INTO automation_chunk(lane_id, chunk_index, points) VALUES (?,?,jsonb(?)) "
                  "ON CONFLICT(lane_id, chunk_index) DO UPDATE SET points=excluded.points "
                  "WHERE automation_chunk.points IS NOT excluded.points");
    Stmt dac(db_, "DELETE FROM automation_chunk WHERE lane_id = ? AND chunk_index NOT IN (SELECT value FROM json_each(?))");
    std::vector<LaneId> laneIds;
    for (const auto& a : m.automation) {
        ok = ok && sa.bind(1, a.id.id.value).bind(2, mid).bind(3, a.name).bind(4, a.target.node.id.value)
                       .bind(5, std::int64_t(a.target.clapParamId))
                       .bind(6, std::string(a.target.mode == model::AutomationMode::Value ? "value" : "mod"))
                       .bind(7, a.target.noteId).run();
        laneIds.push_back(a.id);
        const auto& pts = *a.points;
        std::vector<std::int64_t> chunkIds;
        for (std::size_t i = 0; i < pts.size();) {
            const auto ci = chunkIndexOf(pts[i].at);
            std::string json = "[";
            std::size_t j = i;
            for (; j < pts.size() && chunkIndexOf(pts[j].at) == ci; ++j)
                json += (j > i ? "," : "") + std::string("{\"id\":") + std::to_string(pts[j].id.id.value) +
                        ",\"at\":" + jfrac(pts[j].at) + ",\"v\":" + jnum(pts[j].value) + ",\"c\":\"" +
                        curveName(pts[j].shapeToNext) + "\"}";
            ok = ok && sac.bind(1, a.id.id.value).bind(2, ci).bind(3, json + "]").run();
            chunkIds.push_back(ci);
            i = j;
        }
        ok = ok && dac.bind(1, a.id.id.value)
                       .bind(2, jarray(chunkIds, [](std::int64_t c) { return std::to_string(c); }))
                       .run();
    }
    Stmt da(db_, "DELETE FROM automation_lane WHERE movement_id = ? AND id NOT IN (SELECT value FROM json_each(?))");
    ok = ok && da.bind(1, mid).bind(2, idArray(laneIds)).run();
    return ok;
}

bool ProjectStore::savePluginState(NodeId node, const std::vector<std::byte>& state) {
    const std::uint64_t h = fnv1a(state);
    Stmt s(db_, "INSERT INTO plugin_state(node_id, state, state_hash) VALUES (?,?,?) "
                "ON CONFLICT(node_id) DO UPDATE SET state=excluded.state, state_hash=excluded.state_hash, "
                "saved_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE plugin_state.state_hash IS NOT excluded.state_hash");
    return s.bind(1, node.id.value).bindBlob(2, state.data(), state.size()).bindBlob(3, &h, sizeof h).run();
}

std::optional<std::vector<std::byte>> ProjectStore::pluginState(NodeId node) {
    Stmt s(db_, "SELECT state FROM plugin_state WHERE node_id = ?");
    s.bind(1, node.id.value);
    if (!s.step()) return std::nullopt;
    return s.blob(0);
}

bool ProjectStore::appendUndo(std::optional<MovementId> movement, const std::string& label,
                              const std::string& forwardJson, const std::string& inverseJson) {
    // Recording a new step discards the persisted redo tail, mirroring UndoStack.
    Stmt d(db_, "DELETE FROM undo_journal WHERE undone = 1");
    Stmt s(db_, "INSERT INTO undo_journal(movement_id, label, forward, inverse) VALUES (?,?,jsonb(?),jsonb(?))");
    movement ? s.bind(1, movement->id.value) : s.bindNull(1);
    return d.run() && s.bind(2, label).bind(3, forwardJson).bind(4, inverseJson).run();
}

// ================================================================= load ==
bool ProjectStore::load(model::Project& p, std::string* error) {
    auto fail = [&](const char* what) {
        if (error) *error = std::string(what) + ": " + sqlite3_errmsg(db_);
        return false;
    };
    {
        Stmt s(db_, "SELECT title, composer, sample_rate, next_id FROM project WHERE id = 1");
        if (!s.step()) return fail("project row");
        p.meta = {s.text(0), s.text(1), s.f64(2)};
        p.setNextId(std::uint64_t(s.i64(3)));
    }
    {
        Stmt s(db_, "SELECT n.id, n.kind, n.name, n.clap_bundle, n.clap_plugin_id, n.bypassed, ps.state "
                    "FROM audio_node n LEFT JOIN plugin_state ps ON ps.node_id = n.id ORDER BY n.id");
        while (s.step()) {
            model::AudioNode n{{ElementId{std::uint64_t(s.i64(0))}}, nodeKindFrom(s.text(1)), s.text(2), s.text(3),
                               s.text(4), nullptr, s.i64(5) != 0};
            if (!s.isNull(6)) n.stateChunk = std::make_shared<const std::vector<std::byte>>(s.blob(6));
            p.graph.nodes.push_back(std::move(n));
        }
        Stmt e(db_, "SELECT from_node, from_port, to_node, to_port, gain FROM audio_edge");
        while (e.step())
            p.graph.edges.push_back({{ElementId{std::uint64_t(e.i64(0))}}, std::uint16_t(e.i64(1)),
                                     {ElementId{std::uint64_t(e.i64(2))}}, std::uint16_t(e.i64(3)), float(e.f64(4))});
    }
    {
        Stmt s(db_, "SELECT id, name, instrument_node_id, note_port, midi_channel, gain_db, pan, mute, solo, output_bus_id "
                    "FROM track ORDER BY ordinal");
        while (s.step()) {
            auto t = std::make_unique<model::Track>();
            t->id = {ElementId{std::uint64_t(s.i64(0))}};
            t->name = s.text(1);
            t->instrumentNode = {ElementId{std::uint64_t(s.i64(2))}};
            t->notePort = std::uint16_t(s.i64(3));
            t->midiChannel = std::int16_t(s.i64(4));
            t->gainDb = float(s.f64(5));
            t->pan = float(s.f64(6));
            t->mute = s.i64(7) != 0;
            t->solo = s.i64(8) != 0;
            t->outputBus = {ElementId{std::uint64_t(s.i64(9))}};
            p.tracks.push_back(std::move(t));
        }
    }
    {
        Stmt sp(db_, "SELECT id, name FROM player ORDER BY ordinal");
        Stmt si(db_, "SELECT id, name, engraver_name, transposition, clef, track_id FROM instrument "
                     "WHERE player_id = ? ORDER BY ordinal");
        while (sp.step()) {
            auto pl = std::make_unique<model::Player>();
            pl->id = {ElementId{std::uint64_t(sp.i64(0))}};
            pl->name = sp.text(1);
            si.bind(1, sp.i64(0));
            while (si.step())
                pl->instruments.push_back(std::make_unique<model::Instrument>(model::Instrument{
                    {ElementId{std::uint64_t(si.i64(0))}}, pl->id, si.text(1), si.text(2), int(si.i64(3)), si.text(4),
                    {ElementId{std::uint64_t(si.i64(5))}}}));
            si.reset();
            p.players.push_back(std::move(pl));
        }
    }

    // Movements: JSONB is decoded by SQLite itself (json_each / ->>), so the
    // C++ side never needs its own JSON parser.
    Stmt sm(db_, "SELECT id, ordinal, title, length_num, length_den, revision FROM movement ORDER BY ordinal");
    Stmt st(db_, "SELECT j.value->>'$.at[0]', j.value->>'$.at[1]', j.value->>'bpm', j.value->>'ramp' "
                 "FROM movement m, json_each(m.tempo) j WHERE m.id = ? ORDER BY j.key");
    Stmt sme(db_, "SELECT j.value->>'$.at[0]', j.value->>'$.at[1]', j.value->>'num', j.value->>'den' "
                  "FROM movement m, json_each(m.meter) j WHERE m.id = ? ORDER BY j.key");
    Stmt sk(db_, "SELECT j.value->>'$.at[0]', j.value->>'$.at[1]', j.value->>'fifths', j.value->>'minor' "
                 "FROM movement m, json_each(m.keys) j WHERE m.id = ? ORDER BY j.key");
    Stmt sl(db_, "SELECT player_id FROM player_lane WHERE movement_id = ? ORDER BY ordinal");
    Stmt sc(db_, "SELECT id, at_num, at_den, instrument_id FROM instrument_change WHERE movement_id = ? AND player_id = ? "
                 "ORDER BY at_num * 1.0 / at_den");
    Stmt sv(db_, "SELECT id, voice_index FROM voice WHERE movement_id = ? AND player_id = ? ORDER BY voice_index");
    Stmt se(db_, "SELECT e.value->>'id', e.value->>'k', e.value->>'$.s[0]', e.value->>'$.s[1]', e.value->>'$.d[0]', "
                 "e.value->>'$.d[1]', e.value->>'a', e.value->>'dyn', h.value->>'id', h.value->>'$.p[0]', "
                 "h.value->>'$.p[1]', h.value->>'$.p[2]', h.value->>'t', h.value->>'v', e.value->>'sp' "
                 "FROM voice_chunk c, json_each(c.events) e LEFT JOIN json_each(e.value, '$.h') h "
                 "WHERE c.voice_id = ? ORDER BY c.chunk_index, e.key, h.key");
    Stmt sa(db_, "SELECT id, name, node_id, clap_param_id, mode, note_id FROM automation_lane WHERE movement_id = ? ORDER BY id");
    Stmt sap(db_, "SELECT j.value->>'id', j.value->>'$.at[0]', j.value->>'$.at[1]', j.value->>'v', j.value->>'c' "
                  "FROM automation_chunk c, json_each(c.points) j WHERE c.lane_id = ? ORDER BY c.chunk_index, j.key");
    if (!se.ok() || !sap.ok()) return fail("prepare json queries");

    auto eid = [](const Stmt& s, int c) { return ElementId{std::uint64_t(s.i64(c))}; };
    while (sm.step()) {
        auto m = std::make_unique<model::Movement>();
        m->id = {eid(sm, 0)};
        m->ordinal = int(sm.i64(1));
        m->title = sm.text(2);
        m->length = Fraction(sm.i64(3), sm.i64(4));
        m->revision = std::uint64_t(sm.i64(5));
        const auto mid = sm.i64(0);

        m->tempo.clear();
        st.bind(1, mid);
        while (st.step()) m->tempo.push_back({Fraction(st.i64(0), st.i64(1)), st.f64(2), st.i64(3) != 0});
        st.reset();
        m->meter.clear();
        sme.bind(1, mid);
        while (sme.step()) m->meter.push_back({Fraction(sme.i64(0), sme.i64(1)), int(sme.i64(2)), int(sme.i64(3))});
        sme.reset();
        m->keys.clear();
        sk.bind(1, mid);
        while (sk.step()) m->keys.push_back({Fraction(sk.i64(0), sk.i64(1)), int(sk.i64(2)), sk.i64(3) != 0});
        sk.reset();

        sl.bind(1, mid);
        while (sl.step()) {
            model::PlayerLane lane;
            lane.player = {eid(sl, 0)};
            sc.bind(1, mid).bind(2, sl.i64(0));
            while (sc.step()) lane.changes.push_back({{eid(sc, 0)}, Fraction(sc.i64(1), sc.i64(2)), {eid(sc, 3)}});
            sc.reset();
            sv.bind(1, mid).bind(2, sl.i64(0));
            while (sv.step()) {
                model::Voice v;
                v.id = {eid(sv, 0)};
                v.index = std::uint8_t(sv.i64(1));
                auto evs = std::make_shared<model::EventList>();
                se.bind(1, sv.i64(0));
                while (se.step()) {
                    const auto id = std::uint64_t(se.i64(0));
                    if (evs->empty() || evs->back().id.id.value != id) {
                        model::MusicEvent e;
                        e.id = {ElementId{id}};
                        const auto k = se.text(1);
                        e.kind = k == "r" ? model::EventKind::Rest : k == "c" ? model::EventKind::Chord : model::EventKind::Note;
                        e.start = Fraction(se.i64(2), se.i64(3));
                        e.duration = Fraction(se.i64(4), se.i64(5));
                        e.articulations = std::uint16_t(se.i64(6));
                        if (!se.isNull(7)) e.dynamicMark = std::uint8_t(se.i64(7));
                        if (!se.isNull(14)) e.spans = std::uint8_t(se.i64(14));
                        evs->push_back(std::move(e));
                    }
                    if (!se.isNull(8))
                        evs->back().heads.push_back({{eid(se, 8)},
                                                     {std::int8_t(se.i64(9)), std::int8_t(se.i64(10)), std::int8_t(se.i64(11))},
                                                     se.i64(12) != 0,
                                                     std::uint8_t(se.i64(13))});
                }
                se.reset();
                v.events = std::move(evs);
                lane.voices.push_back(std::move(v));
            }
            sv.reset();
            m->lanes.push_back(std::move(lane));
        }
        sl.reset();

        sa.bind(1, mid);
        while (sa.step()) {
            model::AutomationLane a;
            a.id = {eid(sa, 0)};
            a.name = sa.text(1);
            a.target = {{eid(sa, 2)}, std::uint32_t(sa.i64(3)),
                        sa.text(4) == "mod" ? model::AutomationMode::Modulation : model::AutomationMode::Value,
                        std::int32_t(sa.i64(5))};
            auto pts = std::make_shared<std::vector<model::AutomationPoint>>();
            sap.bind(1, sa.i64(0));
            while (sap.step())
                pts->push_back({{eid(sap, 0)}, Fraction(sap.i64(1), sap.i64(2)), sap.f64(3), curveFrom(sap.text(4))});
            sap.reset();
            a.points = std::move(pts);
            m->automation.push_back(std::move(a));
        }
        sa.reset();

        p.reindexMovement(*m);
        p.movements.push_back(std::move(m));
    }
    return true;
}

// ===================================================== PersistenceWorker ==
PersistenceWorker::PersistenceWorker(std::unique_ptr<ProjectStore> store)
    : store_(std::move(store)), thread_([this] { run(); }) {}

PersistenceWorker::~PersistenceWorker() {
    flush();
    {
        std::lock_guard lk(mutex_);
        stop_ = true;
    }
    cv_.notify_all();
    thread_.join();
}

void PersistenceWorker::enqueue(SaveBatch batch) {
    {
        std::lock_guard lk(mutex_);
        if (pending_) mergeInto(*pending_, std::move(batch));
        else pending_ = std::move(batch);
    }
    cv_.notify_one();
}

void PersistenceWorker::flush() {
    std::unique_lock lk(mutex_);
    idleCv_.wait(lk, [this] { return !pending_ && !busy_; });
}

void PersistenceWorker::mergeInto(SaveBatch& dst, SaveBatch&& src) {
    if (src.header) dst.header = std::move(src.header);
    for (auto& m : src.movements) {
        auto it = std::find_if(dst.movements.begin(), dst.movements.end(),
                               [&](const model::MovementSnapshot& d) { return d.movement->id == m.movement->id; });
        if (it != dst.movements.end()) *it = std::move(m);   // newer snapshot wins
        else dst.movements.push_back(std::move(m));
    }
    // An empty dirty list means "all voices"; merging must preserve that.
    if (dst.dirtyVoices.empty() || src.dirtyVoices.empty()) dst.dirtyVoices.clear();
    else dst.dirtyVoices.insert(dst.dirtyVoices.end(), src.dirtyVoices.begin(), src.dirtyVoices.end());
    for (auto& s : src.pluginStates) dst.pluginStates.push_back(std::move(s));
    dst.deletedMovements.insert(dst.deletedMovements.end(), src.deletedMovements.begin(), src.deletedMovements.end());
}

void PersistenceWorker::run() {
    for (;;) {
        SaveBatch batch;
        {
            std::unique_lock lk(mutex_);
            cv_.wait(lk, [this] { return stop_ || pending_; });
            if (!pending_) return;   // stop requested and nothing left
            batch = std::move(*pending_);
            pending_.reset();
            busy_ = true;
        }
        std::string err;
        if (!store_->save(batch, &err) && onError_) onError_(err);
        {
            std::lock_guard lk(mutex_);
            busy_ = false;
        }
        idleCv_.notify_all();
    }
}

}  // namespace corinzia::storage
