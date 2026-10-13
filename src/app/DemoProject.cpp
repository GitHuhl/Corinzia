#include "corinzia/app/DemoProject.hpp"

namespace corinzia::app {

using namespace model;

namespace {

struct Builder {
    Project& p;
    EventList evs{};
    Fraction at{0};

    // `pitches`: (step, alter, octave); empty = rest.
    Builder& add(Fraction dur, std::initializer_list<Pitch> pitches, std::uint16_t art = 0,
                 std::optional<std::uint8_t> dyn = std::nullopt, bool tie = false) {
        MusicEvent e;
        e.id = {p.allocateId()};
        e.start = at;
        e.duration = dur;
        e.articulations = art;
        e.dynamicMark = dyn;
        for (const auto& pi : pitches) e.heads.push_back({{p.allocateId()}, pi, tie, 0});
        e.kind = e.heads.empty() ? EventKind::Rest : e.heads.size() == 1 ? EventKind::Note : EventKind::Chord;
        evs.push_back(std::move(e));
        at = at + dur;
        return *this;
    }
    std::shared_ptr<const EventList> done() { return std::make_shared<const EventList>(std::move(evs)); }
};

constexpr Fraction q{1, 4}, h{1, 2}, e8{1, 8}, dq{3, 8}, w{1};

}  // namespace

void buildDemoProject(Project& p) {
    p.meta = {"Serenade (demo)", "Corinzia", 48000.0};

    NodeId master{p.allocateId()};
    p.graph.nodes.push_back({master, NodeKind::MasterOut, "Master", "", "", nullptr, false});

    auto addTrack = [&](const char* name, std::int16_t channel) {
        auto t = std::make_unique<Track>();
        t->id = {p.allocateId()};
        t->name = name;
        t->midiChannel = channel;
        t->outputBus = master;
        TrackId id = t->id;
        p.tracks.push_back(std::move(t));
        return id;
    };
    const TrackId tFlute = addTrack("Flute", 0), tPicc = addTrack("Piccolo", 1), tClar = addTrack("Clarinet", 2);

    auto addPlayer = [&](const char* name) {
        auto pl = std::make_unique<Player>();
        pl->id = {p.allocateId()};
        pl->name = name;
        p.players.push_back(std::move(pl));
        return p.players.back().get();
    };
    auto addInstrument = [&](Player* pl, const char* name, const char* shortName, int transp, TrackId track) {
        pl->instruments.push_back(std::make_unique<Instrument>(
            Instrument{{p.allocateId()}, pl->id, name, shortName, transp, "treble", track}));
        return pl->instruments.back()->id;
    };
    Player* fl = addPlayer("Flute 1");
    const InstrumentId flute = addInstrument(fl, "Flute", "Fl.", 0, tFlute);
    const InstrumentId picc = addInstrument(fl, "Piccolo", "Picc.", 12, tPicc);
    Player* cl = addPlayer("Clarinet");
    const InstrumentId clar = addInstrument(cl, "Clarinet in B♭", "Cl.", -2, tClar);

    auto m = std::make_unique<Movement>();
    m->id = {p.allocateId()};
    m->title = "I. Andante";
    m->length = Fraction{8};
    m->tempo = {{Fraction{0}, 96.0, false}, {Fraction{6}, 96.0, true}, {Fraction{8}, 72.0, false}};
    m->keys = {{Fraction{0}, 1, false}};   // G major

    // ---- Flute 1 (bars 1-4 flute, 5-8 piccolo) ----
    PlayerLane fLane;
    fLane.player = fl->id;
    fLane.changes = {{{p.allocateId()}, Fraction{0}, flute}, {{p.allocateId()}, Fraction{4}, picc}};
    Builder f{p};
    f.add(q, {{4, 0, 5}}, 0, 3).add(e8, {{3, 1, 5}}).add(e8, {{2, 0, 5}}).add(q, {{1, 0, 5}}).add(q, {{0, 0, 5}})
        .add(dq, {{6, 0, 4}}).add(e8, {{0, 0, 5}}).add(h, {{1, 0, 5}})
        .add(q, {{2, 0, 5}}, ArtStaccato).add(q, {{3, 1, 5}}, ArtStaccato).add(q, {{4, 0, 5}}).add(q, {{1, 0, 5}})
        .add(w, {{4, 0, 5}}, ArtTenuto)
        .add(q, {{4, 0, 5}}, ArtAccent, 5).add(q, {{6, 0, 5}}).add(q, {{1, 0, 6}}).add(q, {{6, 0, 5}})
        .add(h, {{4, 0, 5}}).add(h, {}, 0)
        .add(q, {{2, 0, 5}}).add(q, {{3, 1, 5}}).add(q, {{4, 0, 5}}).add(q, {{1, 0, 6}})
        .add(w, {{4, 0, 5}}, ArtFermata, 2);
    Voice fv;
    fv.id = {p.allocateId()};
    fv.events = f.done();
    fLane.voices.push_back(fv);

    // ---- Clarinet: two voices (melody in thirds below, sustained bass) ----
    PlayerLane cLane;
    cLane.player = cl->id;
    cLane.changes = {{{p.allocateId()}, Fraction{0}, clar}};
    Builder c1{p}, c2{p};
    for (int bar = 0; bar < 8; ++bar) {
        const bool odd = bar % 2;
        c1.add(h, {{std::int8_t(odd ? 1 : 2), 0, 5}}, 0, bar == 0 ? std::optional<std::uint8_t>(3) : std::nullopt)
            .add(h, {{std::int8_t(odd ? 0 : 1), 0, 5}});
        c2.add(w, {{std::int8_t(odd ? 3 : 4), std::int8_t(odd ? 1 : 0), 4}}, 0, bar == 0 ? std::optional<std::uint8_t>(2) : std::nullopt);
    }
    Voice v1, v2;
    v1.id = {p.allocateId()};
    v1.index = 1;
    v1.events = c1.done();
    v2.id = {p.allocateId()};
    v2.index = 2;
    v2.events = c2.done();
    cLane.voices = {v1, v2};

    m->lanes = {std::move(fLane), std::move(cLane)};
    p.movements.push_back(std::move(m));
    p.reindexMovement(*p.movements.back());
}

}  // namespace corinzia::app
