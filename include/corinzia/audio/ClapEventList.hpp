#pragma once
// Fixed-capacity, allocation-free CLAP event lists used inside the audio
// callback. One input list per plugin node per block.
//
// CLAP requires input events to be sorted by header.time within a block.
// Notes and automation are produced by independent cursors, so we append and
// then sort once; std::sort (introsort) never allocates. A per-push sequence
// number makes the order deterministic for identical timestamps (note-off
// before note-on at the same sample, so retriggers are not swallowed).

#include <clap/clap.h>

#include <algorithm>
#include <array>
#include <cmath>
#include <cstdint>
#include <cstring>

namespace corinzia::audio {

union ClapEventStorage {
    clap_event_header_t header;
    clap_event_note_t note;
    clap_event_note_expression_t expression;
    clap_event_param_value_t paramValue;
    clap_event_param_mod_t paramMod;
    clap_event_param_gesture_t gesture;
    clap_event_midi_t midi;
};

template <std::size_t Capacity = 2048>
class ClapInputEventList {
public:
    ClapInputEventList() {
        iface_.ctx = this;
        iface_.size = [](const clap_input_events_t* l) -> uint32_t {
            return static_cast<const ClapInputEventList*>(l->ctx)->count_;
        };
        iface_.get = [](const clap_input_events_t* l, uint32_t i) -> const clap_event_header_t* {
            auto* self = static_cast<const ClapInputEventList*>(l->ctx);
            return i < self->count_ ? &self->storage_[self->order_[i].slot].header : nullptr;
        };
    }
    ClapInputEventList(const ClapInputEventList&) = delete;
    ClapInputEventList& operator=(const ClapInputEventList&) = delete;

    void clear() noexcept { count_ = 0; }
    std::uint32_t size() const noexcept { return count_; }
    bool full() const noexcept { return count_ == Capacity; }
    const clap_input_events_t* clapEvents() const noexcept { return &iface_; }
    const clap_event_header_t* at(std::uint32_t i) const noexcept { return iface_.get(&iface_, i); }

    // `priority` breaks timestamp ties: lower first (0 = note-off/choke,
    // 1 = parameter values, 2 = note-on, 3 = per-note modulation/expression).
    bool pushNote(std::uint16_t type, std::uint32_t time, std::int32_t noteId, std::int16_t port,
                  std::int16_t channel, std::int16_t key, double velocity) noexcept {
        auto* e = slot(type == CLAP_EVENT_NOTE_ON ? 2 : 0, time);
        if (!e) return false;
        e->note = clap_event_note_t{header(sizeof(clap_event_note_t), time, type), noteId, port, channel, key,
                                    velocity};
        return true;
    }

    bool pushParamValue(std::uint32_t time, clap_id paramId, void* cookie, double value,
                        std::int32_t noteId = -1) noexcept {
        auto* e = slot(1, time);
        if (!e) return false;
        e->paramValue = clap_event_param_value_t{header(sizeof(clap_event_param_value_t), time,
                                                        CLAP_EVENT_PARAM_VALUE),
                                                 paramId, cookie, noteId, -1, -1, -1, value};
        return true;
    }

    bool pushParamMod(std::uint32_t time, clap_id paramId, void* cookie, double amount,
                      std::int32_t noteId = -1) noexcept {
        auto* e = slot(noteId >= 0 ? 3 : 1, time);
        if (!e) return false;
        e->paramMod = clap_event_param_mod_t{header(sizeof(clap_event_param_mod_t), time, CLAP_EVENT_PARAM_MOD),
                                             paramId, cookie, noteId, -1, -1, -1, amount};
        return true;
    }

    bool pushNoteExpression(std::uint32_t time, clap_note_expression expr, std::int32_t noteId,
                            std::int16_t port, std::int16_t channel, std::int16_t key, double value) noexcept {
        auto* e = slot(3, time);
        if (!e) return false;
        e->expression = clap_event_note_expression_t{
            header(sizeof(clap_event_note_expression_t), time, CLAP_EVENT_NOTE_EXPRESSION), expr, noteId, port,
            channel, key, value};
        return true;
    }

    // For plugins whose note port only speaks the MIDI dialect: rewrite
    // NOTE_ON/OFF in place as CLAP_EVENT_MIDI (same slot, same time/priority).
    // note_id is lost - per-note modulation needs the CLAP dialect.
    void convertNotesToMidi() noexcept {
        for (std::uint32_t i = 0; i < count_; ++i) {
            auto& s = storage_[order_[i].slot];
            const auto type = s.header.type;
            if (s.header.space_id != CLAP_CORE_EVENT_SPACE_ID ||
                (type != CLAP_EVENT_NOTE_ON && type != CLAP_EVENT_NOTE_OFF))
                continue;
            const clap_event_note_t n = s.note;
            const auto ch = std::uint8_t(n.channel < 0 ? 0 : n.channel & 0x0f);
            const auto key = std::uint8_t(n.key < 0 ? 0 : n.key & 0x7f);
            const bool on = type == CLAP_EVENT_NOTE_ON;
            const auto vel = std::uint8_t(on ? std::clamp(int(std::lround(n.velocity * 127.0)), 1, 127) : 0);
            s.midi = clap_event_midi_t{header(sizeof(clap_event_midi_t), n.header.time, CLAP_EVENT_MIDI),
                                       std::uint16_t(n.port_index < 0 ? 0 : n.port_index),
                                       {std::uint8_t((on ? 0x90 : 0x80) | ch), key, vel}};
        }
    }

    void sortByTime() noexcept {
        std::sort(order_.begin(), order_.begin() + count_, [](const Key& a, const Key& b) {
            if (a.time != b.time) return a.time < b.time;
            if (a.priority != b.priority) return a.priority < b.priority;
            return a.seq < b.seq;
        });
    }

private:
    struct Key { std::uint32_t time; std::uint16_t priority; std::uint16_t slot; std::uint32_t seq; };
    static_assert(Capacity <= 65536);

    static clap_event_header_t header(std::uint32_t size, std::uint32_t time, std::uint16_t type) noexcept {
        return clap_event_header_t{size, time, CLAP_CORE_EVENT_SPACE_ID, type, 0};
    }

    ClapEventStorage* slot(std::uint16_t priority, std::uint32_t time) noexcept {
        if (count_ == Capacity) return nullptr;   // caller counts drops; never allocate on RT
        const auto idx = static_cast<std::uint16_t>(count_);
        order_[count_] = Key{time, priority, idx, count_};
        ++count_;
        return &storage_[idx];
    }

    clap_input_events_t iface_{};
    std::array<ClapEventStorage, Capacity> storage_{};
    std::array<Key, Capacity> order_{};
    std::uint32_t count_ = 0;
};

// Output events from a plugin (parameter changes from its GUI, gestures, MIDI
// out). Copied into a fixed buffer; the engine forwards the interesting ones
// to the UI thread through an SPSC queue for automation recording.
template <std::size_t Capacity = 512>
class ClapOutputEventList {
public:
    ClapOutputEventList() {
        iface_.ctx = this;
        iface_.try_push = [](const clap_output_events_t* l, const clap_event_header_t* ev) -> bool {
            auto* self = static_cast<ClapOutputEventList*>(l->ctx);
            if (self->count_ == Capacity || ev->size > sizeof(ClapEventStorage)) return false;
            std::memcpy(&self->storage_[self->count_++], ev, ev->size);
            return true;
        };
    }
    ClapOutputEventList(const ClapOutputEventList&) = delete;
    ClapOutputEventList& operator=(const ClapOutputEventList&) = delete;

    void clear() noexcept { count_ = 0; }
    std::uint32_t size() const noexcept { return count_; }
    const ClapEventStorage& operator[](std::uint32_t i) const noexcept { return storage_[i]; }
    const clap_output_events_t* clapEvents() const noexcept { return &iface_; }

private:
    clap_output_events_t iface_{};
    std::array<ClapEventStorage, Capacity> storage_{};
    std::uint32_t count_ = 0;
};

}  // namespace corinzia::audio
