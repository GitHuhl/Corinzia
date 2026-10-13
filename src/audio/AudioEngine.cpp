#include "corinzia/audio/AudioEngine.hpp"

#include "corinzia/audio/ClapPluginHost.hpp"

#include <algorithm>
#include <cmath>
#include <cstring>

namespace corinzia::audio {

AudioEngine::AudioEngine() = default;

AudioEngine::~AudioEngine() {
    // Engine is torn down after the backend stopped: no RT thread is running.
    collectGarbage();
    while (auto m = commands_.tryPop())          // snapshots posted but never consumed
        if (m->type == rt::UiToAudio::Type::SwapSnapshot) delete m->snapshot;
    delete snap_;
}

void AudioEngine::prepare(double sampleRate, std::uint32_t maxFrames) {
    sampleRate_ = sampleRate;
    maxFrames_ = maxFrames;
    nodes_.clear();
    nodes_.reserve(kMaxNodes);
    for (std::uint32_t i = 0; i < kMaxNodes; ++i) {
        auto ns = std::make_unique<NodeState>();
        ns->inBuf.assign(std::size_t(kChannels) * maxFrames, 0.f);
        ns->outBuf.assign(std::size_t(kChannels) * maxFrames, 0.f);
        for (std::uint32_t c = 0; c < kChannels; ++c) {
            ns->inPtr[c] = ns->inBuf.data() + std::size_t(c) * maxFrames;
            ns->outPtr[c] = ns->outBuf.data() + std::size_t(c) * maxFrames;
        }
        nodes_.push_back(std::move(ns));
    }
    lastLaneValue_.assign(kMaxLanes, 0.0);
    delayLines_.assign(kMaxDelayLines, DelayLine{});
    for (auto& d : delayLines_) d.buf.assign(std::size_t(kChannels) * kDelayLineSize, 0.f);
    edgeLine_.fill(-1);
}

void AudioEngine::collectGarbage(void (*onOther)(const rt::AudioToUi&, void*), void* ctx) {
    while (auto m = telemetry_.tryPop()) {
        if (m->type == rt::AudioToUi::Type::RetireSnapshot) delete m->snapshot;
        else if (onOther) onOther(*m, ctx);
    }
}

// ================================================================ RT path ==
void AudioEngine::process(float* const* out, std::uint32_t frames, std::int64_t callbackTimeNs,
                          std::uint32_t outputLatencyFrames) noexcept {
    frames = std::min(frames, maxFrames_);
    for (auto& n : nodes_) { n->in.clear(); n->out.clear(); }

    handleCommands();

    const SamplePos blockStart = pos_;
    if (snap_ && playing_) {
        SamplePos from = pos_;
        std::uint32_t done = 0;
        while (done < frames) {
            std::uint32_t len = frames - done;
            bool wrap = false;
            if (loopEnd_ > loopStart_ && from < loopEnd_ && from + len >= loopEnd_) {
                len = std::uint32_t(loopEnd_ - from);
                wrap = true;
            }
            scheduleRange(from, from + len, done);
            scheduleAutomation(from, from + len, done, automationDirty_);
            automationDirty_ = false;
            done += len;
            from += len;
            if (wrap) {
                releaseAllNotes(std::min(done, frames - 1));
                from = loopStart_;
                noteCursor_ = snap_->firstNoteAtOrAfter(from);
                automationDirty_ = true;
            }
        }
        pos_ = from;
    }

    fillTransport(blockStart);
    renderGraph(out, frames, blockStart);

    if (dropped_) {
        if (telemetry_.tryPush({rt::AudioToUi::Type::EventsDropped, 0, 0, 0.0, SamplePos(dropped_), nullptr}))
            dropped_ = 0;
    }

    rt::PlayheadState ps;
    ps.blockStartSample = blockStart;
    ps.callbackTimeNs = callbackTimeNs;
    ps.blockFrames = frames;
    ps.outputLatencyFrames = outputLatencyFrames;
    ps.graphLatencyFrames = snap_ ? snap_->graphLatency : 0;
    ps.sampleRate = sampleRate_;
    ps.snapshotGeneration = snap_ ? snap_->generation : 0;
    ps.playing = playing_;
    playhead_.store(ps);
    steadyTime_ += frames;
}

void AudioEngine::handleCommands() noexcept {
    using T = rt::UiToAudio::Type;
    while (auto m = commands_.tryPop()) {
        switch (m->type) {
            case T::SwapSnapshot: swapSnapshot(m->snapshot); break;
            case T::Play:
                playing_ = true;
                noteCursor_ = snap_ ? snap_->firstNoteAtOrAfter(pos_) : 0;
                automationDirty_ = true;
                break;
            case T::Stop:
                playing_ = false;
                releaseAllNotes(0);
                break;
            case T::Seek:
                releaseAllNotes(0);
                pos_ = std::max<SamplePos>(0, m->a);
                noteCursor_ = snap_ ? snap_->firstNoteAtOrAfter(pos_) : 0;
                automationDirty_ = true;
                break;
            case T::SetLoop: loopStart_ = m->a; loopEnd_ = m->b; break;
            case T::SetNodeGain:
                if (snap_ && m->node < snap_->nodes.size()) snap_->nodes[m->node].gain = float(m->value);
                break;
            case T::SetParam:
                if (m->node < kMaxNodes && !nodes_[m->node]->in.pushParamValue(0, m->param, nullptr, m->value))
                    ++dropped_;
                break;
            case T::BypassNode: {
                // The plugin pointer is authoritative: it may already be absent
                // from the current snapshot, but must still leave the processing state.
                if (m->plugin) m->plugin->stopProcessing();
                for (std::uint32_t i = 0; snap_ && i < snap_->nodes.size() && i < kMaxNodes; ++i)
                    if (snap_->nodes[i].id == m->nodeId) nodes_[i]->bypassed = true;
                rt::AudioToUi ack{rt::AudioToUi::Type::NodeBypassed, 0, 0, 0.0, 0, nullptr, m->nodeId};
                if (!telemetry_.tryPush(ack)) ++dropped_;
                break;
            }
            case T::UnbypassNode:
                for (std::uint32_t i = 0; snap_ && i < snap_->nodes.size() && i < kMaxNodes; ++i)
                    if (snap_->nodes[i].id == m->nodeId) nodes_[i]->bypassed = false;
                break;
            case T::PreviewNote: {
                // Address by NodeId (UI side) or by snapshot index (tests/tools).
                std::uint32_t node = m->node;
                if (m->nodeId.valid()) {
                    node = kMaxNodes;
                    for (std::uint32_t i = 0; snap_ && i < snap_->nodes.size() && i < kMaxNodes; ++i)
                        if (snap_->nodes[i].id == m->nodeId) node = i;
                }
                if (node < kMaxNodes) {
                    const bool on = m->value >= 0.0;
                    if (!nodes_[node]->in.pushNote(on ? CLAP_EVENT_NOTE_ON : CLAP_EVENT_NOTE_OFF, 0, -1, m->port,
                                                      m->channel, m->key, on ? m->value : 0.0))
                        ++dropped_;
                }
                break;
            }
        }
    }
}

void AudioEngine::swapSnapshot(rt::PlaybackSnapshot* next) noexcept {
    rt::PlaybackSnapshot* old = snap_;

    // Carry per-node bypass flags across by NodeId (indices may change).
    bool anyBypassed = false;
    for (std::uint32_t i = 0; old && i < old->nodes.size(); ++i) anyBypassed |= nodes_[i]->bypassed;
    if (anyBypassed) {
        bool flags[kMaxNodes] = {};
        for (std::uint32_t i = 0; next && i < next->nodes.size() && i < kMaxNodes; ++i)
            for (std::uint32_t j = 0; j < old->nodes.size(); ++j)
                if (old->nodes[j].id == next->nodes[i].id) { flags[i] = nodes_[j]->bypassed; break; }
        for (std::uint32_t i = 0; i < kMaxNodes; ++i) nodes_[i]->bypassed = flags[i];
    }

    // Sounding notes survive an edit only if the new plan still ends them;
    // otherwise (note deleted / moved) they are released now.
    constexpr std::size_t kScanWindow = 4096;
    std::uint32_t kept = 0;
    for (std::uint32_t a = 0; a < activeCount_; ++a) {
        ActiveNote n = active_[a];
        std::int64_t newNode = -1;
        for (std::uint32_t i = 0; next && i < next->nodes.size(); ++i)
            if (next->nodes[i].id == n.nodeId) { newNode = i; break; }
        bool stillEnds = false;
        if (next && newNode >= 0) {
            const std::size_t first = next->firstNoteAtOrAfter(pos_);
            const std::size_t last = std::min(next->notes.size(), first + kScanWindow);
            for (std::size_t k = first; k < last && !stillEnds; ++k) {
                const auto& sn = next->notes[k];
                stillEnds = sn.edge == rt::NoteEdge::Off && sn.noteId == n.noteId && sn.nodeIndex == newNode;
            }
        }
        if (stillEnds) {
            n.node = std::uint32_t(newNode);
            active_[kept++] = n;
        } else if (newNode >= 0) {
            if (!nodes_[newNode]->in.pushNote(CLAP_EVENT_NOTE_OFF, 0, n.noteId, n.port, n.channel, n.key, 0.0))
                ++dropped_;
        }
    }
    activeCount_ = kept;

    snap_ = next;
    assignDelayLines();
    noteCursor_ = snap_ ? snap_->firstNoteAtOrAfter(pos_) : 0;
    automationDirty_ = true;

    if (old && !telemetry_.tryPush({rt::AudioToUi::Type::RetireSnapshot, 0, 0, 0.0, 0, old})) {
        // Telemetry full: extremely unlikely (4096 slots). Leaking one snapshot
        // is preferable to freeing on the RT thread.
        ++dropped_;
    }
}

void AudioEngine::assignDelayLines() noexcept {
    edgeLine_.fill(-1);
    for (auto& d : delayLines_) d.used = false;
    if (!snap_) return;
    const auto edgeCount = std::min<std::size_t>(snap_->edges.size(), kMaxEdges);
    // Pass 1: edges that already own a line keep it (no click on edits).
    for (std::size_t k = 0; k < edgeCount; ++k) {
        const auto& e = snap_->edges[k];
        if (e.delay == 0) continue;
        for (std::size_t l = 0; l < delayLines_.size(); ++l)
            if (!delayLines_[l].used && delayLines_[l].from == e.fromId && delayLines_[l].to == e.toId) {
                delayLines_[l].used = true;
                edgeLine_[k] = std::int16_t(l);
                break;
            }
    }
    // Pass 2: new delayed edges take a free line, starting from silence.
    for (std::size_t k = 0; k < edgeCount; ++k) {
        const auto& e = snap_->edges[k];
        if (e.delay == 0 || edgeLine_[k] >= 0) continue;
        for (std::size_t l = 0; l < delayLines_.size(); ++l)
            if (!delayLines_[l].used) {
                auto& d = delayLines_[l];
                d.used = true;
                d.from = e.fromId;
                d.to = e.toId;
                d.write = 0;
                std::fill(d.buf.begin(), d.buf.end(), 0.f);
                edgeLine_[k] = std::int16_t(l);
                break;
            }
        if (edgeLine_[k] < 0) ++dropped_;   // pool exhausted: edge plays uncompensated
    }
}

void AudioEngine::releaseAllNotes(std::uint32_t frameOffset) noexcept {
    for (std::uint32_t a = 0; a < activeCount_; ++a) {
        const auto& n = active_[a];
        if (!nodes_[n.node]->in.pushNote(CLAP_EVENT_NOTE_OFF, frameOffset, n.noteId, n.port, n.channel, n.key, 0.0))
            ++dropped_;
    }
    activeCount_ = 0;
}

void AudioEngine::addActive(const rt::ScheduledNote& n) noexcept {
    if (activeCount_ == kMaxActiveNotes) return;
    active_[activeCount_++] = {n.nodeIndex, snap_->nodes[n.nodeIndex].id, n.noteId, n.port, n.channel, n.key};
}

void AudioEngine::removeActive(std::int32_t noteId, std::uint32_t node) noexcept {
    for (std::uint32_t a = 0; a < activeCount_; ++a)
        if (active_[a].noteId == noteId && active_[a].node == node) {
            active_[a] = active_[--activeCount_];
            return;
        }
}

void AudioEngine::scheduleRange(SamplePos from, SamplePos to, std::uint32_t frameOffset) noexcept {
    const auto& notes = snap_->notes;
    while (noteCursor_ < notes.size() && notes[noteCursor_].at < to) {
        const auto& n = notes[noteCursor_++];
        if (n.at < from || n.nodeIndex >= kMaxNodes) continue;   // stale (should not happen after seek)
        const auto t = std::uint32_t(n.at - from) + frameOffset;
        const bool on = n.edge == rt::NoteEdge::On;
        if (!nodes_[n.nodeIndex]->in.pushNote(on ? CLAP_EVENT_NOTE_ON : CLAP_EVENT_NOTE_OFF, t, n.noteId, n.port,
                                              n.channel, n.key, on ? n.velocity : 0.0)) {
            ++dropped_;
            continue;
        }
        on ? addActive(n) : removeActive(n.noteId, n.nodeIndex);
    }
}

void AudioEngine::scheduleAutomation(SamplePos from, SamplePos to, std::uint32_t frameOffset, bool force) noexcept {
    const auto laneCount = std::min<std::size_t>(snap_->automation.size(), kMaxLanes);
    for (std::size_t li = 0; li < laneCount; ++li) {
        const auto& lane = snap_->automation[li];
        if (lane.points.empty() || lane.nodeIndex >= kMaxNodes) continue;
        auto& list = nodes_[lane.nodeIndex]->in;

        SamplePos t = from;
        bool first = true;
        while (t < to) {
            const double v = lane.valueAt(t);
            if ((first && force) || std::abs(v - lastLaneValue_[li]) > 1e-9) {
                const auto time = std::uint32_t(t - from) + frameOffset;
                const bool ok = lane.modulation
                                    ? list.pushParamMod(time, lane.paramId, lane.cookie, v, lane.noteId)
                                    : list.pushParamValue(time, lane.paramId, lane.cookie, v, lane.noteId);
                if (!ok) ++dropped_;
                lastLaneValue_[li] = v;
            }
            first = false;
            // Breakpoints are hit exactly; curved segments are sampled every
            // kAutomationStep; flat regions jump straight to the next breakpoint.
            auto bp = std::upper_bound(lane.points.begin(), lane.points.end(), t,
                                       [](SamplePos s, const rt::Breakpoint& b) { return s < b.at; });
            const bool flat = bp == lane.points.begin() || bp == lane.points.end() ||
                              std::prev(bp)->toNext == rt::Curve::Step;
            if (flat)
                t = bp == lane.points.end() ? to : bp->at;
            else
                t = std::min<SamplePos>(t + kAutomationStep - ((t - from) % kAutomationStep), bp->at);
        }
    }
}

void AudioEngine::fillTransport(SamplePos pos) noexcept {
    transport_ = {};
    transport_.header = {sizeof(clap_event_transport_t), 0, CLAP_CORE_EVENT_SPACE_ID, CLAP_EVENT_TRANSPORT, 0};
    transport_.flags = CLAP_TRANSPORT_HAS_SECONDS_TIMELINE | (playing_ ? CLAP_TRANSPORT_IS_PLAYING : 0);
    transport_.song_pos_seconds = clap_sectime(std::llround(double(pos) / sampleRate_ * double(CLAP_SECTIME_FACTOR)));
    if (loopEnd_ > loopStart_) transport_.flags |= CLAP_TRANSPORT_IS_LOOP_ACTIVE;
    const rt::Section* sec = snap_ ? snap_->sectionAt(pos) : nullptr;
    if (!sec) return;

    const double localQ = sec->tempo.quarterAtSample(double(pos - sec->start));
    const auto bar = sec->tempo.barAt(localQ);
    auto beats = [](double q) { return clap_beattime(std::llround(q * double(CLAP_BEATTIME_FACTOR))); };
    transport_.flags |= CLAP_TRANSPORT_HAS_TEMPO | CLAP_TRANSPORT_HAS_BEATS_TIMELINE | CLAP_TRANSPORT_HAS_TIME_SIGNATURE;
    transport_.song_pos_beats = beats(sec->startQuarterGlobal + localQ);
    transport_.tempo = sec->tempo.tempoAt(localQ);
    transport_.bar_start = beats(sec->startQuarterGlobal + bar.barStartQuarter);
    transport_.bar_number = bar.bar - 1;   // CLAP: bar at song position 0 is number 0
    transport_.tsig_num = std::uint16_t(bar.num);
    transport_.tsig_denom = std::uint16_t(bar.den);
}

void AudioEngine::renderGraph(float* const* out, std::uint32_t frames, SamplePos blockStart) noexcept {
    for (std::uint32_t c = 0; c < kChannels; ++c) std::memset(out[c], 0, sizeof(float) * frames);
    if (!snap_) return;

    const auto n = std::min<std::size_t>(snap_->nodes.size(), kMaxNodes);
    for (std::uint32_t i = 0; i < n; ++i) {
        const rt::RtNode& node = snap_->nodes[i];
        NodeState& ns = *nodes_[i];

        for (std::uint32_t c = 0; c < kChannels; ++c) std::memset(ns.inPtr[c], 0, sizeof(float) * frames);
        const auto edgeCount = std::min<std::size_t>(snap_->edges.size(), kMaxEdges);
        for (std::size_t k = 0; k < edgeCount; ++k) {
            const auto& e = snap_->edges[k];
            if (e.to != i || e.from >= n) continue;
            const NodeState& src = *nodes_[e.from];
            if (edgeLine_[k] < 0) {
                for (std::uint32_t c = 0; c < kChannels; ++c)
                    for (std::uint32_t f = 0; f < frames; ++f) ns.inPtr[c][f] += src.outPtr[c][f] * e.gain;
                continue;
            }
            // Delayed edge: write the source into the ring, read `delay` samples behind.
            DelayLine& d = delayLines_[std::size_t(edgeLine_[k])];
            constexpr std::uint32_t mask = kDelayLineSize - 1;
            const std::uint32_t delay = std::min(e.delay, mask);
            for (std::uint32_t c = 0; c < kChannels; ++c) {
                float* ring = d.buf.data() + std::size_t(c) * kDelayLineSize;
                std::uint32_t w = d.write;
                for (std::uint32_t f = 0; f < frames; ++f, ++w) {
                    ring[w & mask] = src.outPtr[c][f];
                    ns.inPtr[c][f] += ring[(w - delay) & mask] * e.gain;
                }
            }
            d.write = (d.write + frames) & mask;
        }

        bool rendered = false;
        if (node.plugin && !ns.bypassed) {
            if (!node.plugin->ports().notePortAcceptsClapDialect) ns.in.convertNotesToMidi();
            ns.in.sortByTime();
            clap_audio_buffer_t inBuf{ns.inPtr.data(), nullptr, kChannels, 0, 0};
            clap_audio_buffer_t outBuf{ns.outPtr.data(), nullptr, kChannels, 0, 0};
            const bool hasInputs = node.plugin->ports().audioInChannels > 0;
            clap_process_t p{};
            p.steady_time = steadyTime_;   // monotonic, unaffected by seek/loop
            p.frames_count = frames;
            p.transport = &transport_;
            p.audio_inputs = hasInputs ? &inBuf : nullptr;
            p.audio_inputs_count = hasInputs ? 1 : 0;
            p.audio_outputs = &outBuf;
            p.audio_outputs_count = 1;
            p.in_events = ns.in.clapEvents();
            p.out_events = ns.out.clapEvents();
            rendered = node.plugin->process(p) != CLAP_PROCESS_ERROR;
            forwardOutputEvents(i, blockStart);
        }
        if (!rendered)
            for (std::uint32_t c = 0; c < kChannels; ++c)
                std::memcpy(ns.outPtr[c], ns.inPtr[c], sizeof(float) * frames);
        if (node.gain != 1.f)
            for (std::uint32_t c = 0; c < kChannels; ++c)
                for (std::uint32_t f = 0; f < frames; ++f) ns.outPtr[c][f] *= node.gain;
        if (node.isMaster)
            for (std::uint32_t c = 0; c < kChannels; ++c)
                for (std::uint32_t f = 0; f < frames; ++f) out[c][f] += ns.outPtr[c][f];
    }
}

void AudioEngine::forwardOutputEvents(std::uint32_t node, SamplePos blockStart) noexcept {
    const auto& list = nodes_[node]->out;
    for (std::uint32_t k = 0; k < list.size(); ++k) {
        const auto& e = list[k];
        if (e.header.space_id != CLAP_CORE_EVENT_SPACE_ID) continue;
        rt::AudioToUi m{rt::AudioToUi::Type::ParamChanged, node, 0, 0.0, blockStart + e.header.time, nullptr};
        switch (e.header.type) {
            case CLAP_EVENT_PARAM_VALUE: m.param = e.paramValue.param_id; m.value = e.paramValue.value; break;
            case CLAP_EVENT_PARAM_GESTURE_BEGIN: m.type = rt::AudioToUi::Type::GestureBegin; m.param = e.gesture.param_id; break;
            case CLAP_EVENT_PARAM_GESTURE_END: m.type = rt::AudioToUi::Type::GestureEnd; m.param = e.gesture.param_id; break;
            default: continue;
        }
        if (!telemetry_.tryPush(m)) ++dropped_;
    }
}

}  // namespace corinzia::audio
