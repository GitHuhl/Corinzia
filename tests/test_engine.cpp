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

    Rig() {
        audio::ThreadRole::markMainThread();
        ids = fixture::makeProject(project);
        a = audio::ClapPluginInstance::create(lib, "test.recorder", ids.synthA);
        b = audio::ClapPluginInstance::create(lib, "test.recorder", ids.synthB);
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
