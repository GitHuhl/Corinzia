#pragma once
// MovementSnapshot -> LilyPond source text, with every engraved object tagged
// by its persistent ElementId through LilyPond's `output-attributes` property:
//
//   \tweak output-attributes #'((id . "e-2s") (class . "ev v-3 t-9")) c'4
//
// LilyPond's SVG backend (-dbackend=svg) wraps each tagged grob in
//   <g id="e-2s" class="ev v-3 t-9"> ... </g>
// so the definitive SVG carries the C++ object identity (id), its voice (v-)
// and its DAW track (t-), which is all point-and-click needs.
//
// Pure function of immutable inputs: safe to call on the compilation worker.

#include "corinzia/model/Model.hpp"

#include <string>
#include <unordered_map>

namespace corinzia::engrave {

struct EngraveContext {
    // Display data captured on the UI thread along with the MovementSnapshot.
    std::unordered_map<PlayerId, std::string> playerNames;
    std::unordered_map<InstrumentId, const model::Instrument*> instruments;  // must outlive the call
    std::unordered_map<InstrumentId, TrackId> instrumentTracks;
    std::string paperSize = "a4";
    double staffSizePt = 18.0;
};

std::string emitLilyPond(const model::Movement& movement, const EngraveContext& ctx);

// Building blocks (exposed for tests).
std::string lilyPitch(const model::Pitch& p);
std::string lilyDuration(Fraction wholeNotes);   // "4.", "8", or "4*2/3" for tuplet-scaled values
std::string lilyKey(int fifths, bool minor);

}  // namespace corinzia::engrave
