#pragma once
// Core value types shared by every layer (model, RT engine, engraving, storage).
// Nothing in here allocates or depends on Qt, so it is safe on the audio thread.

#include <compare>
#include <cstdint>
#include <functional>
#include <numeric>
#include <string>

namespace corinzia {

// ---------------------------------------------------------------------------
// ElementId: a project-unique, persistent 64-bit identity for every semantic
// object (movement, player, voice, note, automation point, plugin node...).
// It is the single key that ties together:
//   * the in-memory model object,
//   * its SQLite row,
//   * its <g id="..."> group in the engraved SVG,
//   * its CLAP note_id when it sounds.
// Ids are allocated from project.next_id and never reused.
// ---------------------------------------------------------------------------
struct ElementId {
    std::uint64_t value = 0;

    constexpr bool valid() const noexcept { return value != 0; }
    constexpr auto operator<=>(const ElementId&) const = default;

    // Compact textual form used in generated notation and SVG ("e-" + base36).
    std::string toSvgId() const;
    static ElementId fromSvgId(std::string_view s) noexcept;  // invalid on failure
};

// Strongly typed aliases so a VoiceId can't be passed where a TrackId is expected.
template <class Tag>
struct TypedId {
    ElementId id{};
    constexpr bool valid() const noexcept { return id.valid(); }
    constexpr auto operator<=>(const TypedId&) const = default;
};

using ProjectId    = TypedId<struct ProjectTag>;
using MovementId   = TypedId<struct MovementTag>;
using PlayerId     = TypedId<struct PlayerTag>;
using InstrumentId = TypedId<struct InstrumentTag>;
using TrackId      = TypedId<struct TrackTag>;
using VoiceId      = TypedId<struct VoiceTag>;
using EventId      = TypedId<struct EventTag>;
using LaneId       = TypedId<struct LaneTag>;
using NodeId       = TypedId<struct NodeTag>;    // audio-graph node (plugin, bus, output)

// ---------------------------------------------------------------------------
// Fraction: exact musical time in whole notes (1/4 = crotchet). Notation needs
// exact rationals (tuplets, grace notes); the audio engine never sees these -
// the snapshot compiler converts them to sample positions via the TempoMap.
// ---------------------------------------------------------------------------
struct Fraction {
    std::int64_t num = 0;
    std::int64_t den = 1;

    constexpr Fraction() = default;
    constexpr Fraction(std::int64_t n, std::int64_t d = 1) : num(n), den(d) { normalize(); }

    constexpr void normalize() {
        if (den < 0) { num = -num; den = -den; }
        const std::int64_t g = std::gcd(num < 0 ? -num : num, den);
        if (g > 1) { num /= g; den /= g; }
    }
    constexpr double toDouble() const noexcept { return double(num) / double(den); }
    constexpr double toQuarters() const noexcept { return 4.0 * toDouble(); }

    friend constexpr Fraction operator+(Fraction a, Fraction b) {
        return {a.num * b.den + b.num * a.den, a.den * b.den};
    }
    friend constexpr Fraction operator-(Fraction a, Fraction b) {
        return {a.num * b.den - b.num * a.den, a.den * b.den};
    }
    friend constexpr Fraction operator*(Fraction a, Fraction b) { return {a.num * b.num, a.den * b.den}; }
    friend constexpr bool operator==(Fraction a, Fraction b) { return a.num * b.den == b.num * a.den; }
    friend constexpr std::strong_ordering operator<=>(Fraction a, Fraction b) {
        return a.num * b.den <=> b.num * a.den;
    }
};

using SamplePos = std::int64_t;   // absolute sample index on the project timeline

}  // namespace corinzia

template <>
struct std::hash<corinzia::ElementId> {
    std::size_t operator()(corinzia::ElementId e) const noexcept { return std::hash<std::uint64_t>{}(e.value); }
};
template <class Tag>
struct std::hash<corinzia::TypedId<Tag>> {
    std::size_t operator()(corinzia::TypedId<Tag> t) const noexcept { return std::hash<corinzia::ElementId>{}(t.id); }
};
