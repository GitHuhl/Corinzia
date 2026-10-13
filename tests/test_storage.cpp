#include "Fixture.hpp"
#include "Test.hpp"

#include "corinzia/storage/ProjectStore.hpp"

#include <sqlite3.h>

#include <cstdio>
#include <cstdlib>
#include <filesystem>

using namespace corinzia;

namespace {

std::string tempPath(const char* name) {
    auto dir = std::filesystem::temp_directory_path() / "corinzia-tests";
    std::filesystem::create_directories(dir);
    auto p = dir / name;
    for (const char* ext : {"", "-wal", "-shm"}) std::filesystem::remove(p.string() + ext);
    return p.string();
}

storage::SaveBatch fullBatch(const model::Project& p) {
    storage::SaveBatch b;
    b.header = storage::ProjectHeader::capture(p);
    for (const auto& m : p.movements) b.movements.push_back(p.snapshot(m->id));
    return b;
}

// Adds a second chunk (events at whole note 5+) so chunk isolation is observable.
void addLateEvents(model::Project& p, fixture::Ids& ids) {
    auto& voice = p.movements[0]->lanes[0].voices[0];
    auto evs = std::make_shared<model::EventList>(*voice.events);
    evs->push_back(fixture::ev(p, {5}, {1, 4}, {fixture::head(p, 1, 4)}));
    evs->push_back(fixture::ev(p, {21, 4}, {1, 4}, {}));    // rest
    voice.events = evs;
    p.movements[0]->length = Fraction{6};
    (void)ids;
}

}  // namespace

TEST(storage_roundtrip) {
    model::Project p;
    auto ids = fixture::makeProject(p);
    addLateEvents(p, ids);
    p.graph.nodes[0].stateChunk = std::make_shared<const std::vector<std::byte>>(
        std::vector<std::byte>{std::byte{1}, std::byte{2}, std::byte{3}});

    const auto path = tempPath("roundtrip.project");
    std::string err;
    {
        auto store = storage::ProjectStore::open(path, &err);
        CHECK(store != nullptr);
        if (!store) { std::printf("  %s\n", err.c_str()); return; }
        CHECK(store->save(fullBatch(p), &err));
        CHECK(store->countRows("voice_chunk") == 2);
        CHECK(store->countRows("automation_chunk") == 1);
    }

    model::Project q;
    auto store = storage::ProjectStore::open(path, &err);
    CHECK(store && store->load(q, &err));
    if (!err.empty()) std::printf("  %s\n", err.c_str());
    CHECK(q.nextId() == p.nextId());
    CHECK(q.players.size() == 1 && q.players[0]->instruments.size() == 2);
    CHECK(q.tracks.size() == 2 && q.tracks[1]->midiChannel == 1);
    CHECK(q.graph.nodes.size() == 3 && q.graph.edges.size() == 2);
    CHECK(q.graph.nodes[0].stateChunk && q.graph.nodes[0].stateChunk->size() == 3);
    CHECK(q.movements.size() == 1);
    if (q.movements.size() != 1) return;
    const auto& a = *p.movements[0];
    const auto& b = *q.movements[0];
    CHECK(b.title == a.title && b.length == a.length);
    CHECK(b.tempo.size() == 1 && b.tempo[0].quarterBpm == 120.0);
    CHECK(b.lanes.size() == 1 && b.lanes[0].changes.size() == 2);
    CHECK(b.lanes[0].instrumentAt(Fraction{1}) == ids.piccolo);
    const auto& ea = *a.lanes[0].voices[0].events;
    const auto& eb = *b.lanes[0].voices[0].events;
    CHECK(ea.size() == eb.size());
    for (std::size_t i = 0; i < std::min(ea.size(), eb.size()); ++i) {
        CHECK(ea[i].id == eb[i].id && ea[i].start == eb[i].start && ea[i].duration == eb[i].duration);
        CHECK(ea[i].kind == eb[i].kind && ea[i].articulations == eb[i].articulations && ea[i].dynamicMark == eb[i].dynamicMark);
        CHECK(ea[i].heads.size() == eb[i].heads.size());
        for (std::size_t h = 0; h < std::min(ea[i].heads.size(), eb[i].heads.size()); ++h)
            CHECK(ea[i].heads[h].id == eb[i].heads[h].id &&
                  ea[i].heads[h].pitch.midiKey() == eb[i].heads[h].pitch.midiKey() &&
                  ea[i].heads[h].tieToNext == eb[i].heads[h].tieToNext);
    }
    CHECK(b.automation.size() == 1 && b.automation[0].points->size() == 2);
    CHECK(b.automation[0].target.clapParamId == 7);
    // Point-and-click index is rebuilt on load.
    CHECK(q.locate(eb[2].heads[1].id.id) != nullptr);
}

TEST(storage_incremental_save_touches_one_chunk) {
    model::Project p;
    auto ids = fixture::makeProject(p);
    addLateEvents(p, ids);
    const auto path = tempPath("incremental.project");
    auto store = storage::ProjectStore::open(path);
    CHECK(store && store->save(fullBatch(p)));
    if (!store) return;

    // Count chunk writes with temp triggers (connection-local, not persisted).
    sqlite3_exec(store->handle(),
                 "CREATE TEMP TABLE chunk_writes(idx INTEGER);"
                 "CREATE TEMP TRIGGER cw_u AFTER UPDATE ON main.voice_chunk BEGIN INSERT INTO chunk_writes VALUES (new.chunk_index); END;"
                 "CREATE TEMP TRIGGER cw_i AFTER INSERT ON main.voice_chunk BEGIN INSERT INTO chunk_writes VALUES (new.chunk_index); END;",
                 nullptr, nullptr, nullptr);

    // Edit the note in the second chunk only.
    auto& voice = p.movements[0]->lanes[0].voices[0];
    auto evs = std::make_shared<model::EventList>(*voice.events);
    (*evs)[6].heads[0].pitch.step = 3;
    voice.events = evs;
    ++p.movements[0]->revision;

    storage::SaveBatch b;
    b.movements.push_back(p.snapshot(ids.mov));
    b.dirtyVoices = {ids.voice};
    CHECK(store->save(b));
    CHECK(store->countRows("chunk_writes") == 1);
    sqlite3_stmt* s = nullptr;
    sqlite3_prepare_v2(store->handle(), "SELECT idx FROM chunk_writes", -1, &s, nullptr);
    CHECK(sqlite3_step(s) == SQLITE_ROW && sqlite3_column_int(s, 0) == 1);
    sqlite3_finalize(s);
}

TEST(storage_plugin_state_skips_unchanged) {
    model::Project p;
    fixture::makeProject(p);
    const auto path = tempPath("plugin.project");
    auto store = storage::ProjectStore::open(path);
    CHECK(store && store->save(fullBatch(p)));
    if (!store) return;
    std::vector<std::byte> blob(1 << 20, std::byte{7});   // 1 MiB state
    storage::SaveBatch b;
    b.pluginStates.push_back({p.graph.nodes[0].id, blob});
    CHECK(store->save(b));
    const int before = sqlite3_total_changes(store->handle());
    CHECK(store->save(b));                                // identical hash: no rewrite
    // Only the project.modified_at touch counts as a change.
    CHECK(sqlite3_total_changes(store->handle()) - before == 1);
    auto back = store->pluginState(p.graph.nodes[0].id);
    CHECK(back && back->size() == blob.size());
}

TEST(storage_worker_coalesces_and_flushes) {
    model::Project p;
    auto ids = fixture::makeProject(p);
    const auto path = tempPath("worker.project");
    {
        storage::PersistenceWorker worker(storage::ProjectStore::open(path));
        worker.enqueue(fullBatch(p));
        for (int i = 0; i < 50; ++i) {
            p.movements[0]->title = "Take " + std::to_string(i);
            storage::SaveBatch b;
            b.movements.push_back(p.snapshot(ids.mov));
            worker.enqueue(std::move(b));
        }
        worker.flush();
    }
    model::Project q;
    auto store = storage::ProjectStore::open(path);
    CHECK(store && store->load(q));
    CHECK(!q.movements.empty() && q.movements[0]->title == "Take 49");
}

TEST(storage_undo_journal) {
    const auto path = tempPath("undo.project");
    auto store = storage::ProjectStore::open(path);
    model::Project p;
    fixture::makeProject(p);
    CHECK(store && store->save(fullBatch(p)));
    if (!store) return;
    CHECK(store->appendUndo(p.movements[0]->id, "Move note", R"({"op":"replaceVoice","voice":12})",
                            R"({"op":"replaceVoice","voice":12})"));
    CHECK(store->appendUndo(std::nullopt, "Add player", "{}", "{}"));
    CHECK(store->countRows("undo_journal") == 2);
}
