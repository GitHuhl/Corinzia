#pragma once
// Score structure and text: Dorico-style popover parsing and pure edits on a
// MovementHeader (key/time/tempo maps, barlines, rehearsal marks, per-staff
// clefs and texts) plus insert/delete bars. All functions are pure; the app
// wraps results in EditMovementHeader / ReplaceVoiceEvents commands.

#include "corinzia/edit/NoteInput.hpp"

#include <optional>
#include <string>

namespace corinzia::edit {

// ---------------------------------------------------------------- parsing --
struct KeySig { int fifths; bool minor; };
// "C", "Eb", "F#", "Bbm", "f#" (lower case = minor, as in Dorico), "a minor", "D major", "E♭".
std::optional<KeySig> parseKey(std::string text);
// "3/4", "6/8", "c" (= 4/4), "C|" or "cut" (= 2/2).
std::optional<std::pair<int, int>> parseTime(std::string text);
// treble/g, bass/f, alto/c, tenor, treble_8, bass_8, percussion -> LilyPond clef name.
std::optional<std::string> parseClef(std::string text);
// "||", "|.", "|:", ":|", ":|:" or words (double, final, start/end repeat).
std::optional<model::BarlineKind> parseBarline(std::string text);
// "Allegro 132", "q=96", "♩=96", "96", "Adagio" (word -> conventional tempo).
struct TempoSpec { std::string text; double bpm; };
std::optional<TempoSpec> parseTempo(std::string text);

// ------------------------------------------------------------ positions --
// Start of the bar containing `pos` (from the meter map).
Fraction barStartAtOrBefore(const model::MovementHeader& h, Fraction pos);
// The barline closest to `pos` (bar starts plus the final barline).
Fraction nearestBarline(const model::MovementHeader& h, Fraction pos);
Fraction barLength(const model::MovementHeader& h, Fraction pos);

// --------------------------------------------------------- header edits --
// Each replaces an existing item of the same kind at the same position.
void setKey(model::MovementHeader& h, Fraction at, KeySig key);
void setMeter(model::MovementHeader& h, Fraction at, int num, int den);
void setTempo(model::MovementHeader& h, Fraction at, const TempoSpec& t);
void setBarline(model::MovementHeader& h, Fraction at, model::BarlineKind kind, IdAlloc& ids);
void toggleRehearsalMark(model::MovementHeader& h, Fraction at, IdAlloc& ids);
void setClef(model::MovementHeader& h, PlayerId lane, Fraction at, const std::string& clef, IdAlloc& ids);
void addText(model::MovementHeader& h, PlayerId lane, Fraction at, const std::string& text, model::TextKind kind, IdAlloc& ids);
// Removes the barline, rehearsal mark, clef change or text with this id.
// Returns a description ("Rehearsal mark", "Text \"dolce\"", …) or nullopt.
std::optional<std::string> removeItem(model::MovementHeader& h, ElementId id);
// Describes the item with this id without changing anything.
std::optional<std::string> describeItem(const model::MovementHeader& h, ElementId id);

// ------------------------------------------------------ insert / delete --
// Voice content: events at/after `at` move right by `length`.
model::EventList insertSpan(const model::EventList& list, Fraction at, Fraction length);
// Removes [from, to) (truncating an event that crosses `from`) and closes the gap.
model::EventList deleteSpan(const model::EventList& list, Fraction from, Fraction to);
// Header: items at/after `at` move and the movement grows by `length`. Map
// entries at 0 (initial tempo/meter/key/instrument) stay; a barline at `at`
// ends the previous bar and stays (unless it is a start repeat).
void insertSpan(model::MovementHeader& h, Fraction at, Fraction length);
// Header: items inside [from, to) go (map entries at 0 stay; one shifted onto
// `from` replaces the one there), later ones move left. Barlines strictly
// inside go; the one at `to` replaces the one at `from`.
void deleteSpan(model::MovementHeader& h, Fraction from, Fraction to);

}  // namespace corinzia::edit
