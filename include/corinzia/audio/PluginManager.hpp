#pragma once
// Main-thread owner of every CLAP plugin instance.
//
//  * scan(): finds .clap bundles (CLAP_PATH, ~/.clap, /usr/lib/clap and the
//    NixOS profile directories) and lists their descriptors.
//  * instantiate(): load (cached per bundle), create, restore state, activate.
//  * pump() + handleTelemetry(): drive the realtime-safe handshakes:
//
//    restart (plugin called request_restart, e.g. latency changed)
//      Running ─pump─▶ post BypassNode ─RT: stop_processing, ack─▶ NodeBypassed
//      ─▶ deactivate(); activate() ─▶ post UnbypassNode ─▶ Running, graphChanged()
//
//    removal
//      remove() ─▶ post BypassNode ─ack─▶ instance moved to the graveyard,
//      graphChanged() (caller recompiles without it) ─▶ destroyed once the
//      engine reports a newer snapshot generation than the one at ack time.
//
// The audio thread therefore never sees a plugin being deactivated or freed
// while it might call process() on it.

#include "corinzia/audio/AudioEngine.hpp"
#include "corinzia/audio/ClapPluginHost.hpp"

#include <functional>
#include <map>
#include <memory>
#include <string>
#include <unordered_map>
#include <vector>

namespace corinzia::audio {

struct PluginInfo {
    std::string bundlePath;
    std::string id, name, vendor, version, description;
    std::vector<std::string> features;   // "instrument", "audio-effect", "reverb", ...
};

class PluginManager {
public:
    explicit PluginManager(AudioEngine& engine) : engine_(engine) {}
    ~PluginManager();

    // CLAP search order on Linux plus NixOS profile locations; only existing dirs.
    static std::vector<std::string> defaultSearchPaths();
    std::vector<PluginInfo> scan(const std::vector<std::string>& paths, std::vector<std::string>* errors = nullptr);

    void setEngineFormat(double sampleRate, std::uint32_t maxFrames);

    ClapPluginInstance* instantiate(NodeId node, const std::string& bundlePath, const std::string& pluginId,
                                    const std::vector<std::byte>* state = nullptr, std::string* error = nullptr);
    void remove(NodeId node);

    ClapPluginInstance* instance(NodeId node) const;
    std::uint32_t latency(NodeId node) const;
    std::size_t instanceCount() const noexcept { return instances_.size(); }
    std::size_t graveyardSize() const noexcept { return graveyard_.size(); }

    // Called by the app's ~60 Hz main-thread timer.
    void pump();
    // Feed every AudioToUi message here; returns true if consumed.
    bool handleTelemetry(const rt::AudioToUi& m);
    // Invoked when routing-relevant plugin properties changed (latency,
    // removal): the owner must recompile and swap a new PlaybackSnapshot.
    void setGraphChangedHandler(std::function<void()> h) { graphChanged_ = std::move(h); }

    // After the audio backend stopped: leave processing state, deactivate all.
    void shutdown();

private:
    enum class State : std::uint8_t { Running, RestartPending, RemovePending };
    struct Entry { std::unique_ptr<ClapPluginInstance> inst; State state = State::Running; };
    struct Grave { std::unique_ptr<ClapPluginInstance> inst; std::uint64_t generationAtAck; };

    void postBypass(NodeId node, ClapPluginInstance* inst);
    std::shared_ptr<ClapLibrary> library(const std::string& path, std::string* error);

    AudioEngine& engine_;
    double sampleRate_ = 48000.0;
    std::uint32_t maxFrames_ = 1024;
    std::map<std::string, std::shared_ptr<ClapLibrary>> libraries_;
    std::unordered_map<NodeId, Entry> instances_;
    std::vector<Grave> graveyard_;
    std::function<void()> graphChanged_;
};

}  // namespace corinzia::audio
