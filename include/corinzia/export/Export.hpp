#pragma once
// Score export. The C++ side serializes a movement into an export document
// (JSON: the notation plus the performed notes from compile::perform, so MIDI
// gets repeats, ornaments and dynamics exactly as played back); the Rust
// crate rust/corinzia-export writes MIDI and MusicXML from it. PDF export is
// LilyPond itself (see AppController::exportPdf).

#include "corinzia/model/Model.hpp"

#include <string>

namespace corinzia::exporter {

std::string exportDocument(const model::Project& project, const model::Movement& movement, double sampleRate = 48000.0);

// Return an empty string on success, else the error message.
std::string writeMidi(const model::Project& project, const model::Movement& movement, const std::string& path);
std::string writeMusicXml(const model::Project& project, const model::Movement& movement, const std::string& path);

}  // namespace corinzia::exporter
