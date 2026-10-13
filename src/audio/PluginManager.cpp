#include "corinzia/audio/PluginManager.hpp"

#include <algorithm>
#include <cstdlib>
#include <filesystem>
#include <sstream>

namespace corinzia::audio {

namespace fs = std::filesystem;

PluginManager::~PluginManager() { shutdown(); }

std::vector<std::string> PluginManager::defaultSearchPaths() {
    std::vector<std::string> out;
    auto add = [&](const std::string& p) {
        std::error_code ec;
        if (!p.empty() && fs::is_directory(p, ec) && std::find(out.begin(), out.end(), p) == out.end()) out.push_back(p);
    };
    if (const char* env = std::getenv("CLAP_PATH")) {
        std::stringstream ss(env);
        for (std::string part; std::getline(ss, part, ':');) add(part);
    }
    const std::string home = std::getenv("HOME") ? std::getenv("HOME") : "";
    const std::string user = std::getenv("USER") ? std::getenv("USER") : "";
    if (!home.empty()) {
        add(home + "/.clap");
        add(home + "/.nix-profile/lib/clap");             // nix profile installs
    }
    if (!user.empty()) add("/etc/profiles/per-user/" + user + "/lib/clap");   // NixOS users.users.<name>.packages
    add("/run/current-system/sw/lib/clap");               // NixOS environment.systemPackages
    add("/usr/lib/clap");
    add("/usr/local/lib/clap");
    return out;
}

std::shared_ptr<ClapLibrary> PluginManager::library(const std::string& path, std::string* error) {
    if (auto it = libraries_.find(path); it != libraries_.end()) return it->second;
    auto lib = ClapLibrary::open(path, error);
    if (lib) libraries_[path] = lib;
    return lib;
}

std::vector<PluginInfo> PluginManager::scan(const std::vector<std::string>& paths, std::vector<std::string>* errors) {
    std::vector<PluginInfo> out;
    for (const auto& root : paths) {
        std::error_code ec;
        for (auto it = fs::recursive_directory_iterator(root, fs::directory_options::follow_directory_symlink, ec);
             !ec && it != fs::recursive_directory_iterator(); it.increment(ec)) {
            if (it->path().extension() != ".clap" || !it->is_regular_file(ec)) continue;
            std::string err;
            auto lib = library(it->path().string(), &err);
            if (!lib) {
                if (errors) errors->push_back(it->path().string() + ": " + err);
                continue;
            }
            for (const auto* d : lib->descriptors()) {
                PluginInfo info{it->path().string(), d->id ? d->id : "", d->name ? d->name : "",
                                d->vendor ? d->vendor : "", d->version ? d->version : "",
                                d->description ? d->description : "", {}};
                for (auto f = d->features; f && *f; ++f) info.features.emplace_back(*f);
                out.push_back(std::move(info));
            }
        }
    }
    std::sort(out.begin(), out.end(), [](const PluginInfo& a, const PluginInfo& b) { return a.name < b.name; });
    return out;
}

void PluginManager::setEngineFormat(double sampleRate, std::uint32_t maxFrames) {
    sampleRate_ = sampleRate;
    maxFrames_ = maxFrames;
}

ClapPluginInstance* PluginManager::instantiate(NodeId node, const std::string& bundlePath, const std::string& pluginId,
                                               const std::vector<std::byte>* state, std::string* error) {
    if (instances_.count(node)) {
        if (error) *error = "node already has a plugin";
        return nullptr;
    }
    auto lib = library(bundlePath, error);
    if (!lib) return nullptr;
    auto inst = ClapPluginInstance::create(lib, pluginId, node, error);
    if (!inst) return nullptr;
    if (state && !state->empty() && !inst->loadState(*state) && error) *error = "state load failed (continuing)";
    if (!inst->activate(sampleRate_, 1, maxFrames_)) {
        if (error) *error = "activate failed: " + pluginId;
        return nullptr;
    }
    auto* raw = inst.get();
    instances_[node] = Entry{std::move(inst), State::Running};
    return raw;
}

ClapPluginInstance* PluginManager::instance(NodeId node) const {
    auto it = instances_.find(node);
    return it == instances_.end() || it->second.state == State::RemovePending ? nullptr : it->second.inst.get();
}

std::uint32_t PluginManager::latency(NodeId node) const {
    auto* i = instance(node);
    return i ? i->latencySamples() : 0;
}

void PluginManager::postBypass(NodeId node, ClapPluginInstance* inst) {
    rt::UiToAudio m{rt::UiToAudio::Type::BypassNode};
    m.nodeId = node;
    m.plugin = inst;
    engine_.post(m);
}

void PluginManager::remove(NodeId node) {
    auto it = instances_.find(node);
    if (it == instances_.end() || it->second.state == State::RemovePending) return;
    it->second.state = State::RemovePending;
    postBypass(node, it->second.inst.get());
}

void PluginManager::pump() {
    for (auto& [node, e] : instances_)
        if (e.state == State::Running && e.inst->pumpMainThread()) {
            e.state = State::RestartPending;
            postBypass(node, e.inst.get());
        }
    const auto gen = engine_.playhead().snapshotGeneration;
    std::erase_if(graveyard_, [gen](const Grave& g) { return gen > g.generationAtAck; });
}

bool PluginManager::handleTelemetry(const rt::AudioToUi& m) {
    if (m.type != rt::AudioToUi::Type::NodeBypassed) return false;
    auto it = instances_.find(m.nodeId);
    if (it == instances_.end()) return true;
    Entry& e = it->second;
    if (e.state == State::RestartPending) {
        e.inst->deactivate();
        e.inst->activate(sampleRate_, 1, maxFrames_);
        e.state = State::Running;
        rt::UiToAudio un{rt::UiToAudio::Type::UnbypassNode};
        un.nodeId = m.nodeId;
        engine_.post(un);
    } else if (e.state == State::RemovePending) {
        graveyard_.push_back({std::move(e.inst), engine_.playhead().snapshotGeneration});
        instances_.erase(it);
    } else {
        return true;   // bypass requested by someone else (e.g. user bypass button)
    }
    if (graphChanged_) graphChanged_();
    return true;
}

void PluginManager::shutdown() {
    for (auto& [node, e] : instances_) {
        (void)node;
        e.inst->stopProcessing();   // audio thread is gone; leave processing state from here
        e.inst->deactivate();
    }
    instances_.clear();
    graveyard_.clear();
}

}  // namespace corinzia::audio
