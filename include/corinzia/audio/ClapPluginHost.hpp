#pragma once
// CLAP hosting: bundle loading and per-instance lifecycle.
//
// Lifecycle & thread contract (from the CLAP spec, enforced by asserts here):
//
//   [main]  ClapLibrary::open      dlopen(.clap) -> clap_entry->init(path)
//   [main]  factory->create_plugin(host, id)     -> plugin->init()
//   [main]  query extensions: params, state, audio-ports, note-ports, latency, gui...
//   [main]  plugin->activate(sr, minFrames, maxFrames)   (allocations happen here)
//   [audio] plugin->start_processing()            (first block after activation)
//   [audio] plugin->process(&clap_process)        (every block)
//   [audio] plugin->stop_processing()             (before deactivate)
//   [main]  plugin->deactivate()
//   [main]  plugin->destroy();  ClapLibrary refcount -> entry->deinit(); dlclose
//
// Host callbacks may arrive on ANY thread; they only set atomic flags which the
// main thread services in pumpMainThread() (driven by a QTimer, ~60 Hz).
// Activating/deactivating a node that the audio thread is using is a two-phase
// hand-shake: UI sends BypassNode -> audio thread calls stop_processing and acks
// -> main thread deactivates/reactivates -> UI sends UnbypassNode.

#include "corinzia/core/Types.hpp"

#include <clap/clap.h>

#include <atomic>
#include <cstdint>
#include <memory>
#include <string>
#include <vector>

namespace corinzia::audio {

// Thread identity used by the host's clap_host_thread_check extension.
struct ThreadRole {
    static void markMainThread() noexcept;
    static void markAudioThread() noexcept;
    static bool isMain() noexcept;
    static bool isAudio() noexcept;
};

// A loaded .clap bundle. Shared between all instances created from it.
class ClapLibrary {
public:
    static std::shared_ptr<ClapLibrary> open(const std::string& path, std::string* error = nullptr);
    // For statically linked / in-process plugins (tests, built-in instruments).
    static std::shared_ptr<ClapLibrary> fromEntry(const clap_plugin_entry_t* entry, std::string pseudoPath);
    ~ClapLibrary();

    const clap_plugin_factory_t* factory() const noexcept { return factory_; }
    std::vector<const clap_plugin_descriptor_t*> descriptors() const;
    const std::string& path() const noexcept { return path_; }

private:
    ClapLibrary() = default;
    void* handle_ = nullptr;              // dlopen handle (null for fromEntry)
    const clap_plugin_entry_t* entry_ = nullptr;
    const clap_plugin_factory_t* factory_ = nullptr;
    std::string path_;
};

struct PluginPorts {
    std::uint32_t audioInChannels = 0, audioOutChannels = 2;
    std::uint32_t notePorts = 0;
    bool notePortAcceptsClapDialect = true;
};

class ClapPluginInstance {
public:
    static std::unique_ptr<ClapPluginInstance> create(std::shared_ptr<ClapLibrary> lib, const std::string& pluginId,
                                                      NodeId node, std::string* error = nullptr);
    ~ClapPluginInstance();
    ClapPluginInstance(const ClapPluginInstance&) = delete;
    ClapPluginInstance& operator=(const ClapPluginInstance&) = delete;

    // ---- main thread --------------------------------------------------------
    bool activate(double sampleRate, std::uint32_t minFrames, std::uint32_t maxFrames);
    void deactivate();
    bool isActive() const noexcept { return active_; }
    std::uint32_t latencySamples() const;           // clap_plugin_latency
    const PluginPorts& ports() const noexcept { return ports_; }

    bool saveState(std::vector<std::byte>& out) const;   // clap_plugin_state.save
    bool loadState(const std::vector<std::byte>& in);    // clap_plugin_state.load

    struct ParamInfo { clap_id id; std::string name; double min, max, def; void* cookie; std::uint32_t flags; };
    std::vector<ParamInfo> params() const;

    // Services request_restart / request_callback / latency changes.
    // Returns true if the instance needs to be bypassed + restarted.
    bool pumpMainThread();

    // ---- audio thread -------------------------------------------------------
    // Returns CLAP_PROCESS_* status. Starts processing lazily on first call.
    clap_process_status process(const clap_process_t& p) noexcept;
    void stopProcessing() noexcept;                 // audio thread, before deactivate
    // Flush param changes while not processing (params.flush).
    void flushParams(const clap_input_events_t* in, const clap_output_events_t* out) noexcept;

    // ---- any thread ---------------------------------------------------------
    NodeId node() const noexcept { return node_; }
    const clap_plugin_t* raw() const noexcept { return plugin_; }
    const char* name() const noexcept;
    bool restartRequested() const noexcept { return restartRequested_.load(std::memory_order_acquire); }

private:
    ClapPluginInstance() = default;
    void initHost();
    void queryExtensions();
    static const void* hostGetExtension(const clap_host_t* host, const char* id);

    std::shared_ptr<ClapLibrary> lib_;
    clap_host_t host_{};
    const clap_plugin_t* plugin_ = nullptr;
    NodeId node_;

    const clap_plugin_params_t* params_ = nullptr;
    const clap_plugin_state_t* state_ = nullptr;
    const clap_plugin_latency_t* latency_ = nullptr;
    const clap_plugin_audio_ports_t* audioPorts_ = nullptr;
    const clap_plugin_note_ports_t* notePorts_ = nullptr;
    PluginPorts ports_;

    bool active_ = false;
    bool processing_ = false;                       // audio-thread owned
    std::atomic<bool> restartRequested_{false};
    std::atomic<bool> callbackRequested_{false};
    std::atomic<bool> processRequested_{false};
    std::atomic<bool> paramsFlushRequested_{false};
};

}  // namespace corinzia::audio
