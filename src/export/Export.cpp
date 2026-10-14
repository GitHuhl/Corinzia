#include "corinzia/export/Export.hpp"

#include "corinzia/compile/SnapshotCompiler.hpp"
#include "corinzia/export/corinzia_export.h"

#include <cmath>
#include <cstdio>
#include <map>
#include <sstream>

namespace corinzia::exporter {

namespace {

std::string str(const std::string& s) {
    std::string o = "\"";
    for (unsigned char c : s) {
        if (c == '"' || c == '\\') o += '\\', o += char(c);
        else if (c < 0x20) {
            char buf[8];
            std::snprintf(buf, sizeof buf, "\\u%04x", c);
            o += buf;
        } else o += char(c);
    }
    return o + "\"";
}
std::string frac(Fraction f) { return "[" + std::to_string(f.num) + "," + std::to_string(f.den) + "]"; }
std::string num(double v) {
    char buf[32];
    std::snprintf(buf, sizeof buf, "%.9g", std::isfinite(v) ? v : 0.0);
    return buf;
}
template <class T, class F>
std::string array(const T& items, F&& fn) {
    std::string o = "[";
    bool first = true;
    for (const auto& x : items) {
        if (!first) o += ',';
        first = false;
        o += fn(x);
    }
    return o + "]";
}

const char* barKind(model::BarlineKind k) {
    switch (k) {
        case model::BarlineKind::Double: return "double";
        case model::BarlineKind::Final: return "final";
        case model::BarlineKind::RepeatStart: return "repeat-start";
        case model::BarlineKind::RepeatEnd: return "repeat-end";
        case model::BarlineKind::RepeatBoth: return "repeat-both";
    }
    return "double";
}

const char* dynamicName(std::uint8_t m) {
    static const char* kNames[] = {"ppp", "pp", "p", "mp", "mf", "f", "ff", "fff"};
    return kNames[m > 7 ? 7 : m];
}

std::string event(const model::MusicEvent& e) {
    std::string o = "{\"id\":" + std::to_string(e.id.id.value) + ",\"start\":" + frac(e.start) + ",\"dur\":" + frac(e.duration);
    if (!e.heads.empty() && e.kind != model::EventKind::Rest)
        o += ",\"pitches\":" + array(e.heads, [](const model::NoteHead& h) {
                 return "{\"id\":" + std::to_string(h.id.id.value) + ",\"step\":" + std::to_string(h.pitch.step) + ",\"alter\":" + std::to_string(h.pitch.alter) +
                        ",\"octave\":" + std::to_string(h.pitch.octave) + (h.tieToNext ? ",\"tie\":true" : "") + "}";
             });
    if (e.articulations) o += ",\"art\":" + std::to_string(e.articulations);
    if (e.dynamicMark) o += ",\"dyn\":" + str(dynamicName(*e.dynamicMark));
    if (e.spans) o += ",\"spans\":" + std::to_string(e.spans);
    return o + "}";
}

}  // namespace

std::string scoreJson(const model::Movement& m, const PartResolver& resolve) {
    std::ostringstream o;
    o << "{\"title\":" << str(m.title) << ",\"length\":" << frac(m.length);
    o << ",\"tempo\":" << array(m.tempo, [](const model::TempoEvent& t) {
        return "{\"at\":" + frac(t.at) + ",\"bpm\":" + num(t.quarterBpm) + ",\"text\":" + str(t.text) + "}";
    });
    o << ",\"meter\":" << array(m.meter, [](const model::MeterEvent& t) {
        return "{\"at\":" + frac(t.at) + ",\"num\":" + std::to_string(t.numerator) + ",\"den\":" + std::to_string(t.denominator) + "}";
    });
    o << ",\"keys\":" << array(m.keys, [](const model::KeyEvent& k) {
        return "{\"at\":" + frac(k.at) + ",\"fifths\":" + std::to_string(k.fifths) + ",\"minor\":" + (k.minor ? "true" : "false") + "}";
    });
    o << ",\"barlines\":" << array(m.barlines, [](const model::BarlineMark& b) {
        return "{\"at\":" + frac(b.at) + ",\"kind\":\"" + barKind(b.kind) + "\"}";
    });
    o << ",\"endings\":" << array(m.endings, [](const model::Ending& e) {
        std::string nums;
        for (int n = 1; n <= 8; ++n)
            if (e.passes & (1u << (n - 1))) nums += (nums.empty() ? "" : ",") + std::to_string(n);
        return "{\"from\":" + frac(e.from) + ",\"to\":" + frac(e.to) + ",\"numbers\":[" + nums + "]}";
    });
    o << ",\"marks\":" << array(m.rehearsal, [](const model::RehearsalMark& r) { return frac(r.at); });
    o << ",\"parts\":" << array(m.lanes, [&](const model::PlayerLane& lane) {
        const PartInfo info = resolve(lane);
        std::string s = "{\"name\":" + str(info.name) + ",\"abbr\":" + str(info.abbreviation) + ",\"clef\":" + str(info.clef);
        s += ",\"clefs\":" + array(lane.clefs, [](const model::ClefChange& c) {
            return "{\"at\":" + frac(c.at) + ",\"clef\":" + str(c.clef) + "}";
        });
        // Staff text, plus instrument changes written as text ("Piccolo"),
        // as the engraving shows them; MusicXML parts keep one name.
        std::vector<std::string> texts;
        for (const auto& t : lane.texts)
            texts.push_back("{\"at\":" + frac(t.at) + ",\"text\":" + str(t.text) +
                            (t.kind == model::TextKind::Technique ? ",\"technique\":true" : "") + "}");
        for (std::size_t ci = 1; ci < lane.changes.size() && ci - 1 < info.changeNames.size(); ++ci)
            texts.push_back("{\"at\":" + frac(lane.changes[ci].at) + ",\"text\":" + str(info.changeNames[ci - 1]) +
                            ",\"technique\":true}");
        s += ",\"texts\":" + array(texts, [](const std::string& x) { return x; });
        s += ",\"voices\":" + array(lane.voices, [](const model::Voice& v) {
            return "{\"index\":" + std::to_string(v.index) + ",\"events\":" + array(*v.events, event) + "}";
        });
        return s + "}";
    });
    o << "}";
    return o.str();
}

PartResolver projectResolver(const model::Project& project) {
    return [&project](const model::PlayerLane& lane) {
        auto& p = const_cast<model::Project&>(project);   // lookups only
        PartInfo info;
        const model::Player* pl = p.player(lane.player);
        const model::Instrument* inst = lane.changes.empty() ? nullptr : p.instrument(lane.changes.front().instrument);
        info.name = inst ? inst->name : pl ? pl->name : "";
        info.abbreviation = inst ? inst->engraverName : "";
        info.clef = inst ? inst->clef : "treble";
        for (std::size_t ci = 1; ci < lane.changes.size(); ++ci) {
            const model::Instrument* to = p.instrument(lane.changes[ci].instrument);
            info.changeNames.push_back(to ? to->name : "");
        }
        return info;
    };
}

std::string exportDocument(const model::Project& project, const model::Movement& m, double sampleRate) {
    auto& p = const_cast<model::Project&>(project);   // lookups only
    std::ostringstream o;
    o << "{\"version\":1,\"title\":" << str(project.meta.title) << ",\"composer\":" << str(project.meta.composer);
    o << ",\"score\":" << scoreJson(m, projectResolver(project));

    // ---- performance ---------------------------------------------------------
    const compile::Performance perf = compile::perform(m, sampleRate);
    o << ",\"performance\":{\"length_q\":" << num(perf.lengthQ);
    o << ",\"tempo\":" << array(perf.tempoPoints, [](const rt::TempoMap::TempoPoint& t) {
        return "{\"q\":" + num(t.quarter) + ",\"bpm\":" + num(t.bpm) + (t.rampToNext ? ",\"ramp\":true" : "") + "}";
    });
    o << ",\"meters\":" << array(perf.meterPoints, [](const rt::TempoMap::Meter& me) {
        return "{\"q\":" + num(me.startQuarter) + ",\"num\":" + std::to_string(me.num) + ",\"den\":" + std::to_string(me.den) + "}";
    });
    // One MIDI track per DAW track, in mixer order; notes go to the track
    // that sounds them (instrument changes switch tracks, as in playback).
    std::map<std::uint64_t, std::vector<const compile::PerformedNote*>> byTrack;
    for (const auto& n : perf.notes) {
        const model::Instrument* inst = p.instrument(n.instrument);
        if (inst) byTrack[inst->track.id.value].push_back(&n);
    }
    std::vector<std::string> tracks;
    for (const auto& t : project.tracks) {
        auto it = byTrack.find(t->id.id.value);
        if (it == byTrack.end()) continue;
        std::string s = "{\"name\":" + str(t->name) + ",\"channel\":" + std::to_string(t->midiChannel & 15) + ",\"notes\":[";
        bool first = true;
        for (const auto* n : it->second) {
            // Gate and arpeggio roll are applied in time, as in playback.
            const double onSec = perf.tempo.secondsAt(n->onQ) + n->rollSec;
            const double offSec = onSec + (perf.tempo.secondsAt(n->fullOffQ) - onSec) * n->gate;
            if (!first) s += ',';
            first = false;
            s += "{\"on\":" + num(perf.tempo.quarterAt(onSec)) + ",\"off\":" + num(perf.tempo.quarterAt(offSec)) +
                 ",\"key\":" + std::to_string(std::clamp<int>(n->key, 0, 127)) + ",\"vel\":" + num(n->velocity) + "}";
        }
        tracks.push_back(s + "]}");
    }
    o << ",\"tracks\":" << array(tracks, [](const std::string& x) { return x; }) << "}}";
    return o.str();
}

namespace {
using ExportFn = int (*)(const unsigned char*, size_t, const char*, char*, size_t);
std::string call(ExportFn fn, const std::string& doc, const std::string& path) {
    char err[512] = {0};
    if (fn(reinterpret_cast<const unsigned char*>(doc.data()), doc.size(), path.c_str(), err, sizeof err) != 0)
        return err[0] ? std::string(err) : std::string("export failed");
    return {};
}
}  // namespace

std::string writeMidi(const model::Project& project, const model::Movement& movement, const std::string& path) {
    return call(cz_export_midi, exportDocument(project, movement), path);
}

std::string writeMusicXml(const model::Project& project, const model::Movement& movement, const std::string& path) {
    return call(cz_export_musicxml, exportDocument(project, movement), path);
}

std::string engraveNativeSvg(const model::Movement& movement, const PartResolver& resolve, std::string& error) {
    const std::string doc = "{\"version\":1,\"score\":" + scoreJson(movement, resolve) + "}";
    char err[512] = {0};
    char* out = nullptr;
    if (cz_layout_svg(reinterpret_cast<const unsigned char*>(doc.data()), doc.size(), &out, err, sizeof err) != 0 || !out) {
        error = err[0] ? err : "native engraving failed";
        return {};
    }
    std::string svg(out);
    cz_free_string(out);
    return svg;
}

}  // namespace corinzia::exporter
