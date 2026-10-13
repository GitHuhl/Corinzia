#include "Fixture.hpp"
#include "Test.hpp"

#include "corinzia/audio/PluginManager.hpp"
#include "corinzia/compile/SnapshotCompiler.hpp"

using namespace corinzia;

namespace {

struct ManagerRig {
    model::Project project;
    fixture::Ids ids = fixture::makeProject(project);
    audio::AudioEngine engine;
    audio::PluginManager manager{engine};
    std::vector<float> l = std::vector<float>(256), r = std::vector<float>(256);
    std::uint64_t generation = 0;
    int graphChanges = 0;

    ManagerRig() {
        audio::ThreadRole::markMainThread();
        engine.prepare(48000, 1024);
        manager.setEngineFormat(48000, 1024);
        manager.setGraphChangedHandler([this] { ++graphChanges; swap(); });
    }
    ~ManagerRig() { manager.shutdown(); }

    void swap() {
        compile::CompileOptions opt;
        opt.generation = ++generation;
        opt.plugin = [this](NodeId n) { return manager.instance(n); };
        opt.latency = [this](NodeId n) { return manager.latency(n); };
        auto snap = compile::compileSnapshot({project.snapshot(ids.mov)}, compile::RoutingSnapshot::capture(project), opt);
        rt::UiToAudio m{rt::UiToAudio::Type::SwapSnapshot};
        m.snapshot = snap.release();
        engine.post(m);
    }
    // One audio block followed by one main-thread pump, like the real app.
    void tick() {
        float* out[2] = {l.data(), r.data()};
        engine.process(out, 256, 0, 0);
        engine.collectGarbage([](const rt::AudioToUi& m, void* ctx) { static_cast<ManagerRig*>(ctx)->manager.handleTelemetry(m); },
                              this);
        manager.pump();
    }
};

}  // namespace

TEST(plugin_scan_finds_real_bundle) {
    audio::AudioEngine engine;
    audio::PluginManager pm(engine);
    std::vector<std::string> errors;
    const auto infos = pm.scan({CORINZIA_TEST_PLUGIN_DIR}, &errors);
    CHECK(errors.empty());
    CHECK(infos.size() == 2);
    if (infos.size() == 2) {
        CHECK(infos[0].id == "test.recorder" && infos[1].id == "test.recorder.midi");
        CHECK(infos[0].bundlePath.ends_with("corinzia-test-recorder.clap"));
        CHECK(infos[0].vendor == "corinzia");
    }
    CHECK(pm.scan({"/nonexistent/dir"}).empty());
}

TEST(plugin_manager_restart_and_remove_handshakes) {
    ManagerRig rig;
    const std::string bundle = std::string(CORINZIA_TEST_PLUGIN_DIR) + "/corinzia-test-recorder.clap";
    std::string err;
    auto* a = rig.manager.instantiate(rig.ids.synthA, bundle, "test.recorder", nullptr, &err);
    auto* b = rig.manager.instantiate(rig.ids.synthB, bundle, "test.recorder.midi", nullptr, &err);
    CHECK(a && b);
    if (!a || !b) { std::printf("  %s\n", err.c_str()); return; }
    rig.swap();
    rig.engine.post({rt::UiToAudio::Type::Play});
    for (int i = 0; i < 4; ++i) rig.tick();
    auto* stA = fixture::recorder(*a);
    CHECK(stA->processing && stA->sounding == 1);   // C4 from bar 1, through dlopen'd code

    // Plugin asks for a restart (e.g. its latency changed) from any thread.
    stA->host->request_restart(stA->host);
    rig.tick();                                     // pump: RestartPending, BypassNode posted
    CHECK(rig.graphChanges == 0);
    rig.tick();                                     // RT: stop_processing + ack -> reactivate
    CHECK(rig.graphChanges == 1);
    CHECK(stA->active);
    rig.tick();                                     // unbypassed: processing resumes lazily
    CHECK(stA->processing);

    // Removal: bypass -> graveyard -> destroyed after a newer snapshot is live.
    rig.manager.remove(rig.ids.synthB);
    CHECK(rig.manager.instance(rig.ids.synthB) == nullptr);
    rig.tick();                                     // ack -> graveyard, recompile posted
    CHECK(rig.graphChanges == 2 && rig.manager.graveyardSize() == 1 && rig.manager.instanceCount() == 1);
    rig.tick();                                     // new snapshot live -> grave cleared
    CHECK(rig.manager.graveyardSize() == 0);
}
