#include "corinzia/audio/ClapPluginHost.hpp"

#include <dlfcn.h>

#include <cassert>
#include <cstdio>
#include <cstring>

namespace corinzia::audio {

// ------------------------------------------------------------- ThreadRole --
namespace {
thread_local bool tlMain = false;
thread_local bool tlAudio = false;
}  // namespace

void ThreadRole::markMainThread() noexcept { tlMain = true; }
void ThreadRole::markAudioThread() noexcept { tlAudio = true; }
bool ThreadRole::isMain() noexcept { return tlMain; }
bool ThreadRole::isAudio() noexcept { return tlAudio; }

// ------------------------------------------------------------- ClapLibrary --
std::shared_ptr<ClapLibrary> ClapLibrary::open(const std::string& path, std::string* error) {
    // RTLD_LOCAL keeps plugins' symbols (often statically linked JUCE/etc.)
    // from colliding with each other or with Qt.
    void* h = ::dlopen(path.c_str(), RTLD_NOW | RTLD_LOCAL);
    if (!h) {
        if (error) *error = ::dlerror();
        return nullptr;
    }
    auto* entry = static_cast<const clap_plugin_entry_t*>(::dlsym(h, "clap_entry"));
    if (!entry || !clap_version_is_compatible(entry->clap_version)) {
        if (error) *error = "no compatible clap_entry in " + path;
        ::dlclose(h);
        return nullptr;
    }
    auto lib = fromEntry(entry, path);
    if (!lib) {
        if (error) *error = "clap_entry->init failed for " + path;
        ::dlclose(h);
        return nullptr;
    }
    lib->handle_ = h;
    return lib;
}

std::shared_ptr<ClapLibrary> ClapLibrary::fromEntry(const clap_plugin_entry_t* entry, std::string pseudoPath) {
    if (!entry->init(pseudoPath.c_str())) return nullptr;
    std::shared_ptr<ClapLibrary> lib(new ClapLibrary);
    lib->entry_ = entry;
    lib->path_ = std::move(pseudoPath);
    lib->factory_ = static_cast<const clap_plugin_factory_t*>(entry->get_factory(CLAP_PLUGIN_FACTORY_ID));
    if (!lib->factory_) return nullptr;   // destructor still calls deinit
    return lib;
}

ClapLibrary::~ClapLibrary() {
    if (entry_) entry_->deinit();
    if (handle_) ::dlclose(handle_);
}

std::vector<const clap_plugin_descriptor_t*> ClapLibrary::descriptors() const {
    std::vector<const clap_plugin_descriptor_t*> out;
    const auto n = factory_->get_plugin_count(factory_);
    for (uint32_t i = 0; i < n; ++i)
        if (auto* d = factory_->get_plugin_descriptor(factory_, i)) out.push_back(d);
    return out;
}

// ------------------------------------------------------ host extensions --
namespace {
ClapPluginInstance* self(const clap_host_t* h) { return static_cast<ClapPluginInstance*>(h->host_data); }

const clap_host_thread_check_t kThreadCheck{
    [](const clap_host_t*) { return ThreadRole::isMain(); },
    [](const clap_host_t*) { return ThreadRole::isAudio(); },
};

const clap_host_log_t kLog{
    [](const clap_host_t*, clap_log_severity sev, const char* msg) {
        // Never block on the audio thread: drop unless main thread.
        if (!ThreadRole::isAudio()) std::fprintf(stderr, "[clap:%d] %s\n", int(sev), msg);
    },
};
}  // namespace

const void* ClapPluginInstance::hostGetExtension(const clap_host_t*, const char* id) {
    static const clap_host_params_t kParams{
        [](const clap_host_t*, clap_param_rescan_flags) { /* UI re-queries params() on next pump */ },
        [](const clap_host_t*, clap_id, clap_param_clear_flags) { /* drop automation refs (UI) */ },
        [](const clap_host_t* h) {
            self(h)->paramsFlushRequested_.store(true, std::memory_order_release);
            self(h)->callbackRequested_.store(true, std::memory_order_release);
        },
    };
    static const clap_host_latency_t kLatency{
        [](const clap_host_t* h) { self(h)->restartRequested_.store(true, std::memory_order_release); },
    };
    static const clap_host_state_t kState{
        [](const clap_host_t*) { /* project marked dirty -> autosave picks up state chunk */ },
    };
    if (!std::strcmp(id, CLAP_EXT_THREAD_CHECK)) return &kThreadCheck;
    if (!std::strcmp(id, CLAP_EXT_LOG)) return &kLog;
    if (!std::strcmp(id, CLAP_EXT_PARAMS)) return &kParams;
    if (!std::strcmp(id, CLAP_EXT_LATENCY)) return &kLatency;
    if (!std::strcmp(id, CLAP_EXT_STATE)) return &kState;
    // Linux GUI plugins additionally need CLAP_EXT_TIMER_SUPPORT and
    // CLAP_EXT_POSIX_FD_SUPPORT, bridged to QTimer / QSocketNotifier in the UI layer.
    return nullptr;
}

void ClapPluginInstance::initHost() {
    host_.clap_version = CLAP_VERSION;
    host_.host_data = this;
    host_.name = "Corinzia";
    host_.vendor = "Corinzia";
    host_.url = "";
    host_.version = "0.1.0";
    host_.get_extension = &ClapPluginInstance::hostGetExtension;
    host_.request_restart = [](const clap_host_t* h) {
        self(h)->restartRequested_.store(true, std::memory_order_release);
    };
    host_.request_process = [](const clap_host_t* h) {
        self(h)->processRequested_.store(true, std::memory_order_release);
    };
    host_.request_callback = [](const clap_host_t* h) {
        self(h)->callbackRequested_.store(true, std::memory_order_release);
    };
}

// ------------------------------------------------------ ClapPluginInstance --
std::unique_ptr<ClapPluginInstance> ClapPluginInstance::create(std::shared_ptr<ClapLibrary> lib,
                                                               const std::string& pluginId, NodeId node,
                                                               std::string* error) {
    std::unique_ptr<ClapPluginInstance> inst(new ClapPluginInstance);
    inst->lib_ = std::move(lib);
    inst->node_ = node;
    inst->initHost();
    const auto* f = inst->lib_->factory();
    inst->plugin_ = f->create_plugin(f, &inst->host_, pluginId.c_str());
    if (!inst->plugin_) {
        if (error) *error = "create_plugin failed: " + pluginId;
        return nullptr;
    }
    if (!inst->plugin_->init(inst->plugin_)) {
        if (error) *error = "plugin init failed: " + pluginId;
        inst->plugin_->destroy(inst->plugin_);
        inst->plugin_ = nullptr;
        return nullptr;
    }
    inst->queryExtensions();
    return inst;
}

ClapPluginInstance::~ClapPluginInstance() {
    if (!plugin_) return;
    assert(!processing_ && "stopProcessing() must run on the audio thread before destruction");
    if (active_) plugin_->deactivate(plugin_);
    plugin_->destroy(plugin_);
}

void ClapPluginInstance::queryExtensions() {
    auto ext = [this](const char* id) { return plugin_->get_extension(plugin_, id); };
    params_ = static_cast<const clap_plugin_params_t*>(ext(CLAP_EXT_PARAMS));
    state_ = static_cast<const clap_plugin_state_t*>(ext(CLAP_EXT_STATE));
    latency_ = static_cast<const clap_plugin_latency_t*>(ext(CLAP_EXT_LATENCY));
    audioPorts_ = static_cast<const clap_plugin_audio_ports_t*>(ext(CLAP_EXT_AUDIO_PORTS));
    notePorts_ = static_cast<const clap_plugin_note_ports_t*>(ext(CLAP_EXT_NOTE_PORTS));

    ports_ = {};
    if (audioPorts_) {
        clap_audio_port_info_t info{};
        ports_.audioOutChannels = 0;
        if (audioPorts_->count(plugin_, false) > 0 && audioPorts_->get(plugin_, 0, false, &info))
            ports_.audioOutChannels = info.channel_count;
        if (audioPorts_->count(plugin_, true) > 0 && audioPorts_->get(plugin_, 0, true, &info))
            ports_.audioInChannels = info.channel_count;
    }
    if (notePorts_) {
        ports_.notePorts = notePorts_->count(plugin_, true);
        clap_note_port_info_t np{};
        if (ports_.notePorts > 0 && notePorts_->get(plugin_, 0, true, &np))
            ports_.notePortAcceptsClapDialect = (np.supported_dialects & CLAP_NOTE_DIALECT_CLAP) != 0;
    }
}

bool ClapPluginInstance::activate(double sampleRate, std::uint32_t minFrames, std::uint32_t maxFrames) {
    if (active_) return true;
    active_ = plugin_->activate(plugin_, sampleRate, minFrames, maxFrames);
    return active_;
}

void ClapPluginInstance::deactivate() {
    if (!active_) return;
    plugin_->deactivate(plugin_);
    active_ = false;
}

std::uint32_t ClapPluginInstance::latencySamples() const {
    return (latency_ && active_) ? latency_->get(plugin_) : 0;
}

bool ClapPluginInstance::saveState(std::vector<std::byte>& out) const {
    if (!state_) return false;
    out.clear();
    clap_ostream_t os{&out, [](const clap_ostream_t* s, const void* buf, uint64_t size) -> int64_t {
                          auto* v = static_cast<std::vector<std::byte>*>(s->ctx);
                          auto* b = static_cast<const std::byte*>(buf);
                          v->insert(v->end(), b, b + size);
                          return int64_t(size);
                      }};
    return state_->save(plugin_, &os);
}

bool ClapPluginInstance::loadState(const std::vector<std::byte>& in) {
    if (!state_) return false;
    struct Cursor { const std::vector<std::byte>* v; std::size_t pos; } cur{&in, 0};
    clap_istream_t is{&cur, [](const clap_istream_t* s, void* buf, uint64_t size) -> int64_t {
                          auto* c = static_cast<Cursor*>(s->ctx);
                          const auto n = std::min<std::size_t>(size, c->v->size() - c->pos);
                          std::memcpy(buf, c->v->data() + c->pos, n);
                          c->pos += n;
                          return int64_t(n);
                      }};
    return state_->load(plugin_, &is);
}

std::vector<ClapPluginInstance::ParamInfo> ClapPluginInstance::params() const {
    std::vector<ParamInfo> out;
    if (!params_) return out;
    const auto n = params_->count(plugin_);
    for (uint32_t i = 0; i < n; ++i) {
        clap_param_info_t pi{};
        if (params_->get_info(plugin_, i, &pi))
            out.push_back({pi.id, pi.name, pi.min_value, pi.max_value, pi.default_value, pi.cookie, pi.flags});
    }
    return out;
}

bool ClapPluginInstance::pumpMainThread() {
    if (callbackRequested_.exchange(false, std::memory_order_acq_rel)) plugin_->on_main_thread(plugin_);
    return restartRequested_.exchange(false, std::memory_order_acq_rel);
}

clap_process_status ClapPluginInstance::process(const clap_process_t& p) noexcept {
    if (!active_) return CLAP_PROCESS_ERROR;
    if (!processing_) {
        if (!plugin_->start_processing(plugin_)) return CLAP_PROCESS_ERROR;
        processing_ = true;
    }
    return plugin_->process(plugin_, &p);
}

void ClapPluginInstance::stopProcessing() noexcept {
    if (processing_) {
        plugin_->stop_processing(plugin_);
        processing_ = false;
    }
}

void ClapPluginInstance::flushParams(const clap_input_events_t* in, const clap_output_events_t* out) noexcept {
    if (params_) params_->flush(plugin_, in, out);
}

const char* ClapPluginInstance::name() const noexcept { return plugin_->desc ? plugin_->desc->name : "?"; }

}  // namespace corinzia::audio
