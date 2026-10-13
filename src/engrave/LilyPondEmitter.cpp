#include "corinzia/engrave/LilyPondEmitter.hpp"

#include <algorithm>
#include <cmath>
#include <sstream>

namespace corinzia::engrave {

namespace {

std::string escape(const std::string& s) {
    std::string o;
    for (char c : s) {
        if (c == '"' || c == '\\') o.push_back('\\');
        o.push_back(c);
    }
    return o;
}

std::string tag(ElementId id, const char* cls, VoiceId voice, TrackId track) {
    std::ostringstream o;
    o << "\\tweak output-attributes #'((id . \"" << id.toSvgId() << "\") (class . \"" << cls << " v-"
      << voice.id.toSvgId().substr(2) << " t-" << track.id.toSvgId().substr(2) << "\")) ";
    return o.str();
}

const char* dynamicName(std::uint8_t m) {
    static const char* kNames[] = {"ppp", "pp", "p", "mp", "mf", "f", "ff", "fff"};
    return kNames[m > 7 ? 7 : m];
}

std::string articulations(std::uint16_t a) {
    std::string s;
    if (a & model::ArtStaccato) s += "-.";
    if (a & model::ArtAccent) s += "->";
    if (a & model::ArtTenuto) s += "--";
    if (a & model::ArtMarcato) s += "-^";
    if (a & model::ArtFermata) s += "\\fermata";
    return s;
}

}  // namespace

std::string lilyPitch(const model::Pitch& p) {
    static const char* kSteps = "cdefgab";
    std::string s(1, kSteps[p.step]);
    for (int i = 0; i < p.alter; ++i) s += "is";
    for (int i = 0; i > p.alter; --i) s += "es";
    const int marks = p.octave - 3;               // c' = C4
    s.append(std::size_t(marks > 0 ? marks : 0), '\'');
    s.append(std::size_t(marks < 0 ? -marks : 0), ',');
    return s;
}

std::string lilyDuration(Fraction d) {
    d.normalize();
    // Plain or dotted power-of-two values: d = (2 - 2^-dots) / base
    for (int base = 1; base <= 128; base *= 2)
        for (int dots = 0; dots <= 3; ++dots) {
            const std::int64_t num = (std::int64_t(1) << (dots + 1)) - 1;   // 1, 3, 7, 15
            const std::int64_t den = std::int64_t(base) << dots;            // base * 2^dots
            if (Fraction(num, den) == d) return std::to_string(base) + std::string(std::size_t(dots), '.');
        }
    // Anything else (tuplets, odd groupings): scale the smallest plain value that
    // is still >= d, matching LilyPond's own tuplet idiom (triplet crotchet = 4*2/3).
    int base = 1;
    while (Fraction(1, base * 2) >= d && base < 128) base *= 2;
    const Fraction scale = d * Fraction(base, 1);
    return std::to_string(base) + "*" + std::to_string(scale.num) +
           (scale.den != 1 ? "/" + std::to_string(scale.den) : "");
}

std::string lilyKey(int fifths, bool minor) {
    static const char* kMajor[] = {"ces", "ges", "des", "aes", "ees", "bes", "f", "c",
                                   "g",   "d",   "a",   "e",   "b",   "fis", "cis"};
    static const char* kMinor[] = {"aes", "ees", "bes", "f", "c", "g", "d", "a",
                                   "e",   "b",   "fis", "cis", "gis", "dis", "ais"};
    const int i = std::clamp(fifths, -7, 7) + 7;
    return std::string("\\key ") + (minor ? kMinor[i] : kMajor[i]) + (minor ? " \\minor" : " \\major");
}

std::string emitLilyPond(const model::Movement& m, const EngraveContext& ctx) {
    std::ostringstream o;
    o << "\\version \"2.24.0\"\n"
      << "#(set-global-staff-size " << ctx.staffSizePt << ")\n"
      << (ctx.panorama ? std::string("\\paper { page-breaking = #ly:one-line-auto-height-breaking\n"
                                     "  top-margin = 4 bottom-margin = 4 right-margin = 8\n"
                                     // one-line mode drops the indent; the staff names need the room
                                     "  left-margin = 35 }\n")
                       : "\\paper { #(set-paper-size \"" + ctx.paperSize + "\") }\n")
      << "\\header { tagline = ##f }\n\n";

    // Global context: meter / key / tempo, as spacer music shared by every staff.
    o << "global = {\n";
    Fraction pos{0};
    std::size_t mi = 0, ki = 0, ti = 0;
    int printedBpm = -1;
    while (pos < m.length) {
        Fraction next = m.length;
        if (mi < m.meter.size() && m.meter[mi].at <= pos)
            o << "  \\time " << m.meter[mi].numerator << "/" << m.meter[mi].denominator << "\n", ++mi;
        if (ki < m.keys.size() && m.keys[ki].at <= pos) o << "  " << lilyKey(m.keys[ki].fifths, m.keys[ki].minor) << "\n", ++ki;
        if (ti < m.tempo.size() && m.tempo[ti].at <= pos) {
            // A metronome mark only when the tempo actually changes; the start
            // of a ramp gets "rit."/"accel." and its arrival the new marking.
            const auto& t = m.tempo[ti];
            const int bpm = int(std::lround(t.quarterBpm));
            if (bpm != printedBpm) o << "  \\tempo 4 = " << bpm << "\n";
            printedBpm = bpm;
            if (t.rampToNext && ti + 1 < m.tempo.size() && m.tempo[ti + 1].quarterBpm != t.quarterBpm)
                o << "  \\tempo \"" << (m.tempo[ti + 1].quarterBpm < t.quarterBpm ? "rit." : "accel.") << "\"\n";
            ++ti;
        }
        if (mi < m.meter.size()) next = std::min(next, m.meter[mi].at);
        if (ki < m.keys.size()) next = std::min(next, m.keys[ki].at);
        if (ti < m.tempo.size()) next = std::min(next, m.tempo[ti].at);
        if (next > pos) o << "  s" << lilyDuration(next - pos) << "\n";
        pos = next;
    }
    o << "}\n\n";

    o << "\\score {\n  \\header { piece = \"" << escape(m.title) << "\" }\n  <<\n";
    for (std::size_t laneIndex = 0; laneIndex < m.lanes.size(); ++laneIndex) {
        const auto& lane = m.lanes[laneIndex];
        const auto pn = ctx.playerNames.find(lane.player);
        const model::Instrument* firstInst = nullptr;
        if (!lane.changes.empty())
            if (auto it = ctx.instruments.find(lane.changes.front().instrument); it != ctx.instruments.end())
                firstInst = it->second;
        o << "    \\new Staff \\with { instrumentName = \""
          << escape(firstInst ? firstInst->engraverName : (pn != ctx.playerNames.end() ? pn->second : ""))
          << "\"\n      \\override StaffSymbol.output-attributes = #'((class . \"staff ln-" << laneIndex << "\"))"
          << " } {\n      \\clef " << (firstInst ? firstInst->clef : "treble") << "\n      << \\global\n";

        for (std::size_t vi = 0; vi < lane.voices.size(); ++vi) {
            const auto& voice = lane.voices[vi];
            static const char* kVoiceCmd[] = {"\\voiceOne", "\\voiceTwo", "\\voiceThree", "\\voiceFour"};
            o << "        \\new Voice { " << (lane.voices.size() > 1 ? kVoiceCmd[vi % 4] : "") << "\n";
            Fraction at{0};
            std::size_t ci = 1;   // next instrument change (changes[0] is the initial one)
            bool inTuplet = false;
            auto closeTuplet = [&] {
                if (inTuplet) o << "          }\n";
                inTuplet = false;
            };
            for (const auto& ev : *voice.events) {
                // Triplet values (e.g. 1/12) are written as their plain value
                // inside \tuplet 3/2 { } so LilyPond draws the bracket.
                const Fraction plain = ev.duration * Fraction(3, 2);
                const bool triplet = lilyDuration(ev.duration).find('*') != std::string::npos &&
                                     lilyDuration(plain).find('*') == std::string::npos;
                if (ev.start > at || !triplet) closeTuplet();
                if (ev.start > at) o << "          s" << lilyDuration(ev.start - at) << "\n";   // exact gaps
                if (vi == 0)
                    while (ci < lane.changes.size() && lane.changes[ci].at <= ev.start) {
                        auto it = ctx.instruments.find(lane.changes[ci].instrument);
                        if (it != ctx.instruments.end())
                            o << "          \\set Staff.instrumentName = \"" << escape(it->second->engraverName)
                              << "\" <>^\\markup \\italic \"" << escape(it->second->name) << "\"\n";
                        ++ci;
                    }
                if (triplet && !inTuplet) {
                    o << "          \\tuplet 3/2 {\n";
                    inTuplet = true;
                }
                const InstrumentId inst = lane.instrumentAt(ev.start);
                const auto tr = ctx.instrumentTracks.find(inst);
                const TrackId track = tr != ctx.instrumentTracks.end() ? tr->second : TrackId{};
                const std::string dur = lilyDuration(triplet ? plain : ev.duration);
                o << "          ";
                if (ev.kind == model::EventKind::Rest || ev.heads.empty()) {
                    o << tag(ev.id.id, "ev rest", voice.id, track) << "r" << dur;
                } else if (ev.heads.size() == 1) {
                    const auto& h = ev.heads.front();
                    o << tag(h.id.id, "ev note", voice.id, track) << lilyPitch(h.pitch) << dur
                      << (h.tieToNext ? "~" : "");
                } else {
                    o << "< ";
                    bool anyTie = false;
                    for (const auto& h : ev.heads) {
                        o << tag(h.id.id, "ev head", voice.id, track) << lilyPitch(h.pitch) << " ";
                        anyTie |= h.tieToNext;
                    }
                    o << ">" << dur << (anyTie ? "~" : "");
                }
                o << articulations(ev.articulations);
                if (ev.dynamicMark) o << "\\" << dynamicName(*ev.dynamicMark);
                // Spanner marks (LilyPond post-events): a hairpin end comes
                // before a new start on the same note; slurs close/open likewise.
                if ((ev.spans & model::HairpinEnd) && !ev.dynamicMark) o << "\\!";
                if (ev.spans & model::SlurEnd) o << ")";
                if (ev.spans & model::SlurStart) o << "(";
                if (ev.spans & model::CrescStart) o << "\\<";
                if (ev.spans & model::DimStart) o << "\\>";
                o << "\n";
                at = ev.start + ev.duration;
            }
            closeTuplet();
            o << "        }\n";
        }
        o << "      >>\n    }\n";
    }
    // Completion engravers split notes and rests that cross a barline into
    // tied notes / several rests, so overwrite input never yields bad bars.
    o << "  >>\n  \\layout {\n    \\context { \\Voice\n"
         "      \\remove \"Note_heads_engraver\" \\consists \"Completion_heads_engraver\"\n"
         "      \\remove \"Rest_engraver\" \\consists \"Completion_rest_engraver\" }\n  }\n}\n";
    return o.str();
}

}  // namespace corinzia::engrave
