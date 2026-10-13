#include "Fixture.hpp"
#include "Test.hpp"

#include "corinzia/audio/AudioEngine.hpp"
#include "corinzia/compile/SnapshotCompiler.hpp"
#include "corinzia/model/Commands.hpp"

#include <algorithm>

using namespace corinzia;
using fixture::Recorded;

namespace {

struct Rig {
    model::Project project;
    fixture::Ids ids;
    std::shared_ptr<audio::ClapLibrary> lib = audio::ClapLibrary::fromEntry(&fixture::kEntry, "mem://recorder");
    std::unique_ptr<audio::ClapPluginInstance> a, b;
    audio::AudioEngine engine;
    std::vector<float> l = std::vector<float>(256), r = std::vector<float>(256);
    std::uint64_t generation = 0;

    std::uint32_t latencyA = 0;

    explicit Rig(const char* pluginB = "test.recorder", std::uint32_t latA = 0) : latencyA(latA) {
        audio::ThreadRole::markMainThread();
        ids = fixture::makeProject(project);
        a = audio::ClapPluginInstance::create(lib, "test.recorder", ids.synthA);
        b = audio::ClapPluginInstance::create(lib, pluginB, ids.synthB);
        a->activate(48000, 1, 1024);
        b->activate(48000, 1, 1024);
        engine.prepare(48000, 1024);
        swap();
    }
    ~Rig() {
        a->stopProcessing();
        b->stopProcessing();
    }
    void swap() {
        compile::CompileOptions opt;
        opt.generation = ++generation;
        opt.plugin = [this](NodeId n) { return n == ids.synthA ? a.get() : n == ids.synthB ? b.get() : nullptr; };
        opt.latency = [this](NodeId n) { return n == ids.synthA ? latencyA : 0u; };
        auto snap = compile::compileSnapshot({project.snapshot(ids.mov)}, compile::RoutingSnapshot::capture(project), opt);
        rt::UiToAudio m{rt::UiToAudio::Type::SwapSnapshot};
        m.snapshot = snap.release();
        engine.post(m);
    }
    void post(rt::UiToAudio::Type t, SamplePos x = 0, SamplePos y = 0) {
        rt::UiToAudio m{t};
        m.a = x;
        m.b = y;
        engine.post(m);
    }
    void run(SamplePos frames) {
        float* out[2] = {l.data(), r.data()};
        for (SamplePos done = 0; done < frames; done += 256) {
            engine.process(out, 256, 0, 0);
            engine.collectGarbage();
        }
    }
    static std::vector<Recorded> notes(const audio::ClapPluginInstance& i, std::uint16_t type) {
        std::vector<Recorded> v;
        for (const auto& e : fixture::recorder(i)->events) if (e.type == type) v.push_back(e);
        return v;
    }
};

}  // namespace

TEST(engine_sample_accurate_notes_and_instrument_change) {
    Rig rig;
    rig.post(rt::UiToAudio::Type::Play);
    rig.run(200000);

    auto onA = Rig::notes(*rig.a, CLAP_EVENT_NOTE_ON);
    auto offA = Rig::notes(*rig.a, CLAP_EVENT_NOTE_OFF);
    auto onB = Rig::notes(*rig.b, CLAP_EVENT_NOTE_ON);
    auto offB = Rig::notes(*rig.b, CLAP_EVENT_NOTE_OFF);

    // Flute (synthA): tied C4 = one note of a half note at 90% gate; chord at beat 3.
    CHECK(onA.size() == 3 && offA.size() == 3);
    if (onA.size() == 3) {
        CHECK(onA[0].sample == 0 && onA[0].key == 60);
        CHECK(onA[1].sample == 48000 && onA[2].sample == 48000);
        CHECK_NEAR(onA[0].value, 0.62, 1e-6);                // default mf
    }
    if (offA.size() == 3) CHECK(offA[0].sample == 43200 && offA[0].noteId == onA[0].noteId);

    // Piccolo (synthB, channel 1) from bar 2: A5 staccato, B5, C6 forte.
    CHECK(onB.size() == 3 && offB.size() == 3);
    if (onB.size() == 3 && offB.size() == 3) {
        CHECK(onB[0].sample == 96000 && onB[0].key == 81 && onB[0].channel == 1);
        CHECK(offB[0].sample == 96000 + 12000);               // staccato = 50% gate
        CHECK(onB[2].key == 84 && onB[2].sample == 144000);
        CHECK_NEAR(onB[2].value, 0.75, 1e-6);                 // f
    }
}

TEST(engine_automation_is_sample_positioned) {
    Rig rig;
    rig.post(rt::UiToAudio::Type::Play);
    rig.run(60000);
    std::vector<Recorded> pv;
    for (const auto& e : fixture::recorder(*rig.a)->events)
        if (e.type == CLAP_EVENT_PARAM_VALUE && e.param == 7) pv.push_back(e);
    CHECK(!pv.empty());
    if (pv.empty()) return;
    CHECK(pv.front().sample == 0 && pv.front().value == 0.0);
    auto mid = std::find_if(pv.begin(), pv.end(), [](const Recorded& r) { return r.sample == 24000; });
    CHECK(mid != pv.end() && std::abs(mid->value - 0.5) < 1e-9);
    CHECK(pv.back().value == 1.0 && pv.back().sample == 48000);  // lands exactly on the breakpoint
    for (std::size_t i = 1; i < pv.size(); ++i) CHECK(pv[i].sample - pv[i - 1].sample <= 32);
}

TEST(engine_transport_info) {
    Rig rig;
    rig.post(rt::UiToAudio::Type::Play);
    rig.run(96000 + 256);
    const auto& tr = fixture::recorder(*rig.a)->transports;
    CHECK(tr.size() == 376);
    if (tr.size() == 376) {
        const auto& t = tr[375];                              // block starting at sample 96000 = bar 2
        CHECK(t.bar_number == 1);
        CHECK(t.song_pos_beats == 4 * CLAP_BEATTIME_FACTOR);
        CHECK(t.bar_start == 4 * CLAP_BEATTIME_FACTOR);
        CHECK_NEAR(t.tempo, 120.0, 1e-9);
        CHECK(t.tsig_num == 4 && t.tsig_denom == 4);
        CHECK(t.flags & CLAP_TRANSPORT_IS_PLAYING);
    }
}

TEST(engine_renders_graph_to_master) {
    Rig rig;
    rig.post(rt::UiToAudio::Type::Play);
    rig.run(1024);                                            // C4 sounding on synthA
    CHECK_NEAR(rig.l[100], 0.25f, 1e-6);
    CHECK_NEAR(rig.r[255], 0.25f, 1e-6);
}

TEST(engine_snapshot_swap_releases_deleted_notes_only) {
    // Case 1: an unrelated edit keeps the sounding C4 alive.
    {
        Rig rig;
        rig.post(rt::UiToAudio::Type::Play);
        rig.run(10240);
        auto edited = std::make_shared<model::EventList>(*rig.project.movements[0]->lanes[0].voices[0].events);
        (*edited)[3].heads[0].pitch.step = 4;                 // bar 2 edit
        rig.project.movements[0]->lanes[0].voices[0].events = edited;
        rig.swap();
        rig.run(60000);
        auto offA = Rig::notes(*rig.a, CLAP_EVENT_NOTE_OFF);
        CHECK(!offA.empty() && offA[0].sample == 43200);
    }
    // Case 2: deleting the sounding note releases it on the next block.
    {
        Rig rig;
        rig.post(rt::UiToAudio::Type::Play);
        rig.run(10240);
        auto edited = std::make_shared<model::EventList>(*rig.project.movements[0]->lanes[0].voices[0].events);
        edited->erase(edited->begin(), edited->begin() + 2);
        rig.project.movements[0]->lanes[0].voices[0].events = edited;
        rig.swap();
        rig.run(256);
        auto offA = Rig::notes(*rig.a, CLAP_EVENT_NOTE_OFF);
        CHECK(offA.size() == 1 && offA[0].sample == 10240 && offA[0].time == 0);
    }
}

TEST(engine_loop_and_seek) {
    Rig rig;
    rig.post(rt::UiToAudio::Type::SetLoop, 0, 48000);
    rig.post(rt::UiToAudio::Type::Play);
    rig.run(100096);
    auto onA = Rig::notes(*rig.a, CLAP_EVENT_NOTE_ON);
    CHECK(onA.size() == 3);                                   // C4 at 0, 48000, 96000 (wrapped)
    for (const auto& e : onA) CHECK(e.key == 60);
    CHECK(Rig::notes(*rig.b, CLAP_EVENT_NOTE_ON).empty());

    rig.post(rt::UiToAudio::Type::SetLoop, 0, 0);
    rig.post(rt::UiToAudio::Type::Seek, 96000);
    rig.run(512);
    auto onB = Rig::notes(*rig.b, CLAP_EVENT_NOTE_ON);
    CHECK(onB.size() == 1 && onB[0].key == 81 && onB[0].time == 0);
    CHECK(fixture::recorder(*rig.a)->sounding == 0);          // seek released everything
}

TEST(clap_state_roundtrip) {
    Rig rig;
    std::vector<std::byte> blob;
    CHECK(rig.a->saveState(blob) && blob.size() == 4);
    CHECK(rig.a->loadState(blob));
}

TEST(engine_pdc_aligns_parallel_paths) {
    // synthA reports 100 samples of latency; synthB's path to master must be
    // delayed by the same amount so both arrive together.
    Rig rig("test.recorder", 100);
    rt::UiToAudio note{rt::UiToAudio::Type::PreviewNote};
    note.node = 1;                     // topological order: synthA, synthB, master
    note.key = 60;
    note.value = 0.8;
    rig.engine.post(note);
    rig.run(256);
    CHECK(rig.engine.playhead().graphLatencyFrames == 100);
    CHECK_NEAR(rig.l[99], 0.f, 1e-9);  // still inside the compensation delay
    CHECK_NEAR(rig.l[100], 0.25f, 1e-6);
    CHECK_NEAR(rig.r[255], 0.25f, 1e-6);
    // A re-compiled snapshot keeps the same delay line: no gap, no click.
    rig.swap();
    rig.run(256);
    CHECK_NEAR(rig.l[0], 0.25f, 1e-6);
    CHECK_NEAR(rig.l[150], 0.25f, 1e-6);
}

TEST(engine_midi_dialect_fallback) {
    Rig rig("test.recorder.midi");
    CHECK(!rig.b->ports().notePortAcceptsClapDialect);
    rig.post(rt::UiToAudio::Type::Play);
    rig.run(200000);
    CHECK(Rig::notes(*rig.b, CLAP_EVENT_NOTE_ON).empty());       // never CLAP notes
    const auto midi = Rig::notes(*rig.b, CLAP_EVENT_MIDI);
    CHECK(midi.size() == 6);
    if (midi.size() == 6) {
        CHECK(midi[0].param == 0x90 && midi[0].channel == 1 && midi[0].key == 81 && midi[0].sample == 96000);
        CHECK(midi[0].value == 79);                                  // round(0.62 * 127)
        CHECK(midi[1].param == 0x80 && midi[1].key == 81 && midi[1].sample == 108000);
    }
    CHECK(fixture::recorder(*rig.b)->sounding == 0);
    // The CLAP-dialect plugin on the same engine is unaffected.
    CHECK(Rig::notes(*rig.a, CLAP_EVENT_NOTE_ON).size() == 3);
}

TEST(compile_hairpins_and_slurs_shape_playback) {
    model::Project p;
    fixture::makeProject(p);
    model::EventList v;
    for (int i = 0; i < 5; ++i) v.push_back(fixture::ev(p, Fraction(i, 4), {1, 4}, {fixture::head(p, i, 4)}));
    v[0].dynamicMark = 2;             // p
    v[0].spans = model::CrescStart | model::SlurStart;
    v[3].spans = model::SlurEnd;
    v[4].dynamicMark = 5;             // f, ends the hairpin
    const auto shape = compile::expressiveShape(v);
    CHECK_NEAR(shape[0].velocity, compile::velocityForDynamic(2), 1e-6);
    CHECK_NEAR(shape[4].velocity, compile::velocityForDynamic(5), 1e-6);
    CHECK(shape[1].velocity > shape[0].velocity && shape[2].velocity > shape[1].velocity && shape[3].velocity > shape[2].velocity);
    CHECK_NEAR(shape[2].velocity, (compile::velocityForDynamic(2) + compile::velocityForDynamic(5)) / 2, 1e-5);   // linear in time
    CHECK(shape[0].legato && shape[2].legato && !shape[3].legato && !shape[4].legato);
    // Diminuendo with no target dynamic: two steps down, then holds.
    model::EventList d = v;
    for (auto& e : d) { e.spans = 0; e.dynamicMark.reset(); }
    d[0].dynamicMark = 5;
    d[0].spans = model::DimStart;
    d[2].spans = model::HairpinEnd;
    const auto ds = compile::expressiveShape(d);
    CHECK_NEAR(ds[2].velocity, compile::velocityForDynamic(3), 1e-6);
    CHECK_NEAR(ds[4].velocity, compile::velocityForDynamic(3), 1e-6);
}

TEST(compile_unrolls_repeats) {
    model::Project p;
    auto ids = fixture::makeProject(p);
    auto* m = p.movement(ids.mov);
    using K = model::BarlineKind;
    // No repeats: one identity span.
    auto order = compile::playbackOrder(*m);
    CHECK(order.size() == 1 && order[0].first == Fraction{0} && order[0].second == Fraction{2});
    // ":|" at bar 2 with no start repeat -> bars 1,1,2.
    m->barlines = {{{p.allocateId()}, Fraction{1}, K::RepeatEnd}};
    order = compile::playbackOrder(*m);
    CHECK(order.size() == 2 && order[0] == std::pair{Fraction{0}, Fraction{1}} && order[1] == std::pair{Fraction{0}, Fraction{2}});
    // "|:" bar 2 ":|" at the end -> bars 1,2,2.
    m->barlines = {{{p.allocateId()}, Fraction{1}, K::RepeatStart}, {{p.allocateId()}, Fraction{2}, K::RepeatEnd}};
    order = compile::playbackOrder(*m);
    CHECK(order.size() == 2 && order[0] == std::pair{Fraction{0}, Fraction{2}} && order[1] == std::pair{Fraction{1}, Fraction{2}});

    compile::CompileOptions opt;
    auto snap = compile::compileSnapshot({p.snapshot(ids.mov)}, compile::RoutingSnapshot::capture(p), opt);
    const auto& sec = snap->sections.front();
    // 3 bars at 120 bpm = 12 quarters = 288000 samples.
    CHECK(sec.end - sec.start == 288000);
    // Bar 2 is played twice: the a'' staccato note-on at q4 and again at q8.
    int aOn = 0;
    for (const auto& n : snap->notes)
        if (n.edge == rt::NoteEdge::On && n.key == 81) {
            CHECK(n.at == sec.start + 4 * 24000 || n.at == sec.start + 8 * 24000);
            ++aOn;
        }
    CHECK(aOn == 2);
    // The playhead maps performed time back into the written bar.
    CHECK_NEAR(sec.writtenQuarter(9.0), 5.0, 1e-9);
    CHECK_NEAR(sec.performedQuarter(5.0), 5.0, 1e-9);
    CHECK_NEAR(sec.writtenQuarter(3.0), 3.0, 1e-9);
    // Automation is repeated only for points inside the repeated bar (none here).
    CHECK(snap->automation.size() == 1 && snap->automation[0].points.size() == 2);
}
