#pragma once
// Score export. The C++ side serializes a movement into an export document
// (JSON: the notation plus the performed notes from compile::perform, so MIDI
// gets repeats, ornaments and dynamics exactly as played back); the Rust
// crate rust/corinzia-export writes MIDI and MusicXML from it. PDF export is
// LilyPond itself (see AppController::exportPdf).

#include "corinzia/model/Model.hpp"

#include <functional>
#include <string>
#include <vector>

namespace corinzia::exporter {

// What the notation needs to know about a staff besides its music.
struct PartInfo {
    std::string name, abbreviation, clef = "treble";
    std::vector<std::string> changeNames;   // instruments of lane.changes[1..]
};
using PartResolver = std::function<PartInfo(const model::PlayerLane&)>;
PartResolver projectResolver(const model::Project& project);

// The notation of a movement as JSON (the "score" of an export document).
std::string scoreJson(const model::Movement& movement, const PartResolver& resolve);

std::string exportDocument(const model::Project& project, const model::Movement& movement, double sampleRate = 48000.0);

// Return an empty string on success, else the error message.
std::string writeMidi(const model::Project& project, const model::Movement& movement, const std::string& path);
std::string writeMusicXml(const model::Project& project, const model::Movement& movement, const std::string& path);

// The native engraver (rust/corinzia-layout): the movement on one system, or
// on A4 pages, as SVG shaped like LilyPond's (staves, element-id groups).
// Empty on error.
std::vector<std::string> engraveNativeSvg(const model::Movement& movement, const PartResolver& resolve, bool pages,
                                          std::string& error);

}  // namespace corinzia::exporter
