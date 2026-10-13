#pragma once
// Shared test fixtures: an in-process CLAP "recorder" plugin and a small
// two-instrument project (one player doubling Flute -> Piccolo mid-movement).

#include "corinzia/audio/ClapPluginHost.hpp"
#include "corinzia/model/Model.hpp"

#include <clap/clap.h>

#include <cstring>
#include <vector>

namespace fixture {

struct Recorded {
    std::int64_t sample;       // steady_time + header.time (== timeline while not seeking/looping)
    std::uint32_t time;        // offset in block
    std::uint16_t type;
    std::int32_t noteId;
    std::int16_t key, channel;
    double value;              // velocity / param value
    std::uint32_t param;
};

struct RecorderState {
    std::vector<Recorded> events;
    std::vector<clap_event_transport_t> transports;
    bool processing = false;
    bool active = false;
    int sounding = 0;
};

inline const clap_plugin_descriptor_t kDesc{CLAP_VERSION, "test.recorder", "Recorder", "corinzia", "", "", "", "1",
                                            "records events", nullptr};

inline const clap_plugin_t* createRecorder(const clap_host_t* host) {
    auto* p = new clap_plugin_t{};
    p->desc = &kDesc;
    p->plugin_data = new RecorderState;
    (void)host;
    auto st = [](const clap_plugin_t* pl) { return static_cast<RecorderState*>(pl->plugin_data); };
    p->init = [](const clap_plugin_t*) { return true; };
    p->destroy = [](const clap_plugin_t* pl) {
        delete static_cast<RecorderState*>(pl->plugin_data);
        delete pl;
    };
    p->activate = [](const clap_plugin_t* pl, double, uint32_t, uint32_t) {
        static_cast<RecorderState*>(pl->plugin_data)->active = true;
        return true;
    };
    p->deactivate = [](const clap_plugin_t* pl) { static_cast<RecorderState*>(pl->plugin_data)->active = false; };
    p->start_processing = [](const clap_plugin_t* pl) {
        static_cast<RecorderState*>(pl->plugin_data)->processing = true;
        return true;
    };
    p->stop_processing = [](const clap_plugin_t* pl) { static_cast<RecorderState*>(pl->plugin_data)->processing = false; };
    p->reset = [](const clap_plugin_t*) {};
    p->process = [](const clap_plugin_t* pl, const clap_process_t* pr) -> clap_process_status {
        auto* s = static_cast<RecorderState*>(pl->plugin_data);
        if (pr->transport) s->transports.push_back(*pr->transport);
        const uint32_t n = pr->in_events->size(pr->in_events);
        uint32_t lastTime = 0;
        for (uint32_t i = 0; i < n; ++i) {
            const auto* h = pr->in_events->get(pr->in_events, i);
            if (h->time < lastTime || h->time >= pr->frames_count) std::abort();   // CLAP ordering contract
            lastTime = h->time;
            Recorded r{pr->steady_time + h->time, h->time, h->type, -1, 0, 0, 0.0, 0};
            if (h->type == CLAP_EVENT_NOTE_ON || h->type == CLAP_EVENT_NOTE_OFF) {
                auto* e = reinterpret_cast<const clap_event_note_t*>(h);
                r.noteId = e->note_id; r.key = e->key; r.channel = e->channel; r.value = e->velocity;
                s->sounding += h->type == CLAP_EVENT_NOTE_ON ? 1 : -1;
            } else if (h->type == CLAP_EVENT_PARAM_VALUE) {
                auto* e = reinterpret_cast<const clap_event_param_value_t*>(h);
                r.param = e->param_id; r.value = e->value;
            } else if (h->type == CLAP_EVENT_PARAM_MOD) {
                auto* e = reinterpret_cast<const clap_event_param_mod_t*>(h);
                r.param = e->param_id; r.value = e->amount; r.noteId = e->note_id;
            }
            s->events.push_back(r);
        }
        // Emit DC proportional to the number of sounding notes (graph test).
        for (uint32_t c = 0; c < pr->audio_outputs[0].channel_count; ++c)
            for (uint32_t f = 0; f < pr->frames_count; ++f) pr->audio_outputs[0].data32[c][f] = 0.25f * float(s->sounding);
        return CLAP_PROCESS_CONTINUE;
    };
    p->get_extension = [](const clap_plugin_t*, const char* id) -> const void* {
        static const clap_plugin_state_t kState{
            [](const clap_plugin_t*, const clap_ostream_t* os) { return os->write(os, "CORZ", 4) == 4; },
            [](const clap_plugin_t*, const clap_istream_t* is) {
                char b[4];
                return is->read(is, b, 4) == 4 && std::memcmp(b, "CORZ", 4) == 0;
            }};
        return std::strcmp(id, CLAP_EXT_STATE) == 0 ? &kState : nullptr;
    };
    p->on_main_thread = [](const clap_plugin_t*) {};
    (void)st;
    return p;
}

inline const clap_plugin_factory_t kFactory{
    [](const clap_plugin_factory_t*) -> uint32_t { return 1; },
    [](const clap_plugin_factory_t*, uint32_t) -> const clap_plugin_descriptor_t* { return &kDesc; },
    [](const clap_plugin_factory_t*, const clap_host_t* host, const char* id) -> const clap_plugin_t* {
        return std::strcmp(id, kDesc.id) == 0 ? createRecorder(host) : nullptr;
    }};

inline const clap_plugin_entry_t kEntry{
    CLAP_VERSION, [](const char*) { return true; }, [] {},
    [](const char* id) -> const void* { return std::strcmp(id, CLAP_PLUGIN_FACTORY_ID) == 0 ? &kFactory : nullptr; }};

inline RecorderState* recorder(const corinzia::audio::ClapPluginInstance& i) {
    return static_cast<RecorderState*>(i.raw()->plugin_data);
}

// ---------------------------------------------------------------- project --
struct Ids {
    corinzia::NodeId synthA, synthB, master;
    corinzia::TrackId t1, t2;
    corinzia::InstrumentId flute, piccolo;
    corinzia::PlayerId player;
    corinzia::MovementId mov;
    corinzia::VoiceId voice;
    corinzia::LaneId lane;
};

inline corinzia::model::NoteHead head(corinzia::model::Project& p, int step, int octave, bool tie = false) {
    return {{p.allocateId()}, {std::int8_t(step), 0, std::int8_t(octave)}, tie, 0};
}

inline corinzia::model::MusicEvent ev(corinzia::model::Project& p, corinzia::Fraction start, corinzia::Fraction dur,
                                      std::vector<corinzia::model::NoteHead> heads, std::uint16_t art = 0) {
    using namespace corinzia::model;
    MusicEvent e;
    e.id = {p.allocateId()};
    e.kind = heads.empty() ? EventKind::Rest : heads.size() == 1 ? EventKind::Note : EventKind::Chord;
    e.start = start;
    e.duration = dur;
    e.heads = std::move(heads);
    e.articulations = art;
    return e;
}

// 2 bars of 4/4 at 120 bpm (1 quarter = 24000 samples @48k).
// Bar 1 (Flute -> synthA):   c'4~ c'4 <e' g'>2         (tie merges into one half note)
// Bar 2 (Piccolo -> synthB): a'4-. b'4 c''2\f
inline Ids makeProject(corinzia::model::Project& p) {
    using namespace corinzia;
    using namespace corinzia::model;
    Ids ids;
    auto nid = [&] { return NodeId{p.allocateId()}; };
    ids.synthA = nid(); ids.synthB = nid(); ids.master = nid();
    p.graph.nodes = {{ids.synthA, NodeKind::ClapPlugin, "A", "", "test.recorder", nullptr, false},
                     {ids.synthB, NodeKind::ClapPlugin, "B", "", "test.recorder", nullptr, false},
                     {ids.master, NodeKind::MasterOut, "Master", "", "", nullptr, false}};
    p.graph.edges = {{ids.synthA, 0, ids.master, 0, 1.f}, {ids.synthB, 0, ids.master, 0, 1.f}};

    ids.t1 = {p.allocateId()}; ids.t2 = {p.allocateId()};
    p.tracks.push_back(std::make_unique<Track>(Track{ids.t1, "Flute", ids.synthA, 0, 0, 0.f, 0.f, false, false, ids.master}));
    p.tracks.push_back(std::make_unique<Track>(Track{ids.t2, "Piccolo", ids.synthB, 0, 1, 0.f, 0.f, false, false, ids.master}));

    ids.player = {p.allocateId()};
    auto pl = std::make_unique<Player>();
    pl->id = ids.player;
    pl->name = "Player 1";
    ids.flute = {p.allocateId()}; ids.piccolo = {p.allocateId()};
    pl->instruments.push_back(std::make_unique<Instrument>(Instrument{ids.flute, ids.player, "Flute", "Fl.", 0, "treble", ids.t1}));
    pl->instruments.push_back(std::make_unique<Instrument>(Instrument{ids.piccolo, ids.player, "Piccolo", "Picc.", 12, "treble", ids.t2}));
    p.players.push_back(std::move(pl));

    auto m = std::make_unique<Movement>();
    ids.mov = {p.allocateId()};
    m->id = ids.mov;
    m->title = "I. Allegro";
    m->length = Fraction{2};
    PlayerLane lane;
    lane.player = ids.player;
    lane.changes = {{{p.allocateId()}, Fraction{0}, ids.flute}, {{p.allocateId()}, Fraction{1}, ids.piccolo}};
    Voice v;
    ids.voice = {p.allocateId()};
    v.id = ids.voice;
    EventList evs;
    evs.push_back(ev(p, {0}, {1, 4}, {head(p, 0, 4, true)}));
    evs.push_back(ev(p, {1, 4}, {1, 4}, {head(p, 0, 4)}));
    evs.push_back(ev(p, {1, 2}, {1, 2}, {head(p, 2, 4), head(p, 4, 4)}));
    evs.push_back(ev(p, {1}, {1, 4}, {head(p, 5, 5)}, ArtStaccato));
    evs.push_back(ev(p, {5, 4}, {1, 4}, {head(p, 6, 5)}));
    auto last = ev(p, {3, 2}, {1, 2}, {head(p, 0, 6)});
    last.dynamicMark = 5;   // f
    evs.push_back(last);
    v.events = std::make_shared<const EventList>(std::move(evs));
    lane.voices.push_back(v);
    m->lanes.push_back(std::move(lane));

    AutomationLane al;
    ids.lane = {p.allocateId()};
    al.id = ids.lane;
    al.name = "Cutoff";
    al.target = {ids.synthA, 7, AutomationMode::Value, -1};
    al.points = std::make_shared<const std::vector<AutomationPoint>>(std::vector<AutomationPoint>{
        {{p.allocateId()}, Fraction{0}, 0.0, CurveShape::Linear},
        {{p.allocateId()}, Fraction{1, 2}, 1.0, CurveShape::Step}});
    m->automation.push_back(al);

    p.movements.push_back(std::move(m));
    p.reindexMovement(*p.movements.back());
    return ids;
}

}  // namespace fixture
