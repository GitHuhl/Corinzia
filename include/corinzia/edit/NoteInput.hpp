#pragma once
// Pure note-input operations on a voice's EventList.
//
// Every function takes the current (immutable, COW) list and returns a new
// one, so the caller wraps the result in a ReplaceVoiceEvents command: undo is
// free and nothing here touches the Project or Qt. Semantics follow the
// "overwrite" input model of Sibelius/Dorico (insert-mode off):
//
//   * a new event occupies [pos, pos + duration); anything it covers is
//     removed, anything it partially covers is truncated,
//   * in voice 1 the music stays contiguous: cut-off remainders and gaps
//     before the caret become rests (other voices may have holes, which are
//     engraved as invisible spacers),
//   * pitches are chosen like a human copyist would: the letter name comes
//     from the key, the octave is the one nearest the previous note.

#include "corinzia/model/Model.hpp"

#include <functional>
#include <memory>
#include <optional>

namespace corinzia::edit {

using IdAlloc = std::function<ElementId()>;

enum class BaseDuration : std::uint8_t { Whole = 0, Half, Quarter, Eighth, Sixteenth, ThirtySecond, SixtyFourth };

struct DurationSpec {
    BaseDuration base = BaseDuration::Quarter;
    int dots = 0;            // 0..2
    bool triplet = false;    // 3 in the time of 2
    Fraction value() const;
};

// ------------------------------------------------------------- pitch helpers --
int diatonic(const model::Pitch& p) noexcept;                  // step + 7 * octave
model::Pitch fromDiatonic(int d, int alter) noexcept;
int keyAlter(int fifths, int step) noexcept;                   // alteration implied by the key signature
// Letter `step` (0=C..6=B) in the octave nearest `previous`; `alter` defaults to the key's.
model::Pitch nearestPitch(int step, std::optional<int> alter, int keyFifths, const model::Pitch& previous) noexcept;

// ------------------------------------------------------------------- results --
struct EditResult {
    std::shared_ptr<const model::EventList> events;
    ElementId focus;          // the element the caret/selection should follow
    Fraction from, to;        // musically affected range (for provisional redraw)
    Fraction caret;           // where the caret goes next
};

model::MusicEvent makeEvent(IdAlloc& ids, Fraction start, Fraction duration, const std::vector<model::Pitch>& pitches,
                            std::uint16_t articulations = 0);

// Overwrite-insert `ev` (must have a valid id). `fillWithRests`: voice-1 policy.
EditResult insertEvent(const model::EventList& list, model::MusicEvent ev, bool fillWithRests, IdAlloc& ids);

// Adds `pitch` to the note/chord starting exactly at `pos` (no-op on rests or duplicates).
std::optional<EditResult> addToChord(const model::EventList& list, Fraction pos, model::Pitch pitch, IdAlloc& ids);

// Deletes a head (from a chord) or a whole event. A deleted single note or a
// chord's last head becomes a rest when `fillWithRests`, otherwise a hole.
std::optional<EditResult> deleteElement(const model::EventList& list, ElementId element, bool fillWithRests, IdAlloc& ids);

// Applies `fn` to the event containing `element` (event id or one of its heads).
std::optional<EditResult> modifyEvent(const model::EventList& list, ElementId element,
                                      const std::function<void(model::MusicEvent&, model::NoteHead*)>& fn);

// Lookup helpers.
const model::MusicEvent* findEvent(const model::EventList& list, ElementId element, const model::NoteHead** head = nullptr);
const model::MusicEvent* eventAt(const model::EventList& list, Fraction pos);   // event sounding at pos
std::optional<model::Pitch> previousPitch(const model::EventList& list, Fraction pos);
Fraction voiceEnd(const model::EventList& list);

}  // namespace corinzia::edit
