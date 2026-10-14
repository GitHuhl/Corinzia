#include "Fixture.hpp"
#include "Test.hpp"

#include "corinzia/export/Export.hpp"

#include <cstdlib>
#include <fstream>
#include <sstream>
#include <string>
#include <vector>

using namespace corinzia;

namespace {

std::string tmpPath(const char* name) {
    return std::string(std::getenv("TMPDIR") ? std::getenv("TMPDIR") : "/tmp") + "/" + name;
}
std::string slurp(const std::string& path) {
    std::stringstream s;
    s << std::ifstream(path, std::ios::binary).rdbuf();
    return s.str();
}

// Minimal SMF reader: per track, (absolute tick, status, data1, data2) for
// channel events and (tick, 0xff, meta type, 0) for meta events.
struct MidiEv { std::uint32_t tick; int status, a, b; };
struct Smf { int format = -1, ppq = 0; std::vector<std::vector<MidiEv>> tracks; };
Smf parseSmf(const std::string& d) {
    Smf f;
    auto u32 = [&](std::size_t i) { return (std::uint32_t(std::uint8_t(d[i])) << 24) | (std::uint32_t(std::uint8_t(d[i + 1])) << 16) |
                                            (std::uint32_t(std::uint8_t(d[i + 2])) << 8) | std::uint8_t(d[i + 3]); };
    auto u16 = [&](std::size_t i) { return (int(std::uint8_t(d[i])) << 8) | std::uint8_t(d[i + 1]); };
    if (d.size() < 14 || d.compare(0, 4, "MThd") != 0) return f;
    f.format = u16(8);
    const int ntracks = u16(10);
    f.ppq = u16(12);
    std::size_t pos = 14;
    for (int t = 0; t < ntracks && pos + 8 <= d.size(); ++t) {
        if (d.compare(pos, 4, "MTrk") != 0) return f;
        const std::size_t end = pos + 8 + u32(pos + 4);
        pos += 8;
        std::vector<MidiEv> evs;
        std::uint32_t tick = 0;
        auto vlq = [&] {
            std::uint32_t v = 0;
            for (;;) {
                const std::uint8_t c = std::uint8_t(d[pos++]);
                v = (v << 7) | (c & 0x7f);
                if (!(c & 0x80)) return v;
            }
        };
        while (pos < end) {
            tick += vlq();
            const int st = std::uint8_t(d[pos++]);
            if (st == 0xff) {
                const int type = std::uint8_t(d[pos++]);
                const std::uint32_t len = vlq();
                evs.push_back({tick, 0xff, type, len ? std::uint8_t(d[pos]) : 0});
                pos += len;
            } else {
                evs.push_back({tick, st, std::uint8_t(d[pos]), std::uint8_t(d[pos + 1])});
                pos += 2;
            }
        }
        f.tracks.push_back(std::move(evs));
        pos = end;
    }
    return f;
}

}  // namespace

TEST(export_document_is_complete_json) {
    model::Project p;
    auto ids = fixture::makeProject(p);
    const std::string doc = exporter::exportDocument(p, *p.movement(ids.mov));
    CHECK(doc.find("\"version\":1") != std::string::npos && doc.find("\"performance\"") != std::string::npos);
    CHECK(doc.find("\"name\":\"Flute\"") != std::string::npos && doc.find("\"dyn\":\"f\"") != std::string::npos);
    CHECK(doc.find("{\"at\":[1,1],\"text\":\"Piccolo\",\"technique\":true}") != std::string::npos);   // instrument change
}

TEST(export_midi_matches_playback) {
    model::Project p;
    auto ids = fixture::makeProject(p);
    const std::string path = tmpPath("corinzia-export.mid");
    const std::string err = exporter::writeMidi(p, *p.movement(ids.mov), path);
    CHECK(err.empty());
    const Smf f = parseSmf(slurp(path));
    CHECK(f.format == 1 && f.ppq == 960 && f.tracks.size() == 3);   // conductor + Flute + Piccolo
    if (f.tracks.size() != 3) return;
    // Conductor: tempo 120 (500000 us) and 4/4 at tick 0.
    bool tempo = false, meter = false;
    for (const auto& e : f.tracks[0]) {
        tempo |= e.status == 0xff && e.a == 0x51 && e.tick == 0;
        meter |= e.status == 0xff && e.a == 0x58 && e.tick == 0 && e.b == 4;
    }
    CHECK(tempo && meter);
    // Flute (channel 0): c'4~c'4 is one note, gate 90 % of a half note.
    std::uint32_t on = 9999, off = 0;
    for (const auto& e : f.tracks[1]) {
        if (e.status == 0x90 && e.a == 60) on = e.tick;
        if (e.status == 0x80 && e.a == 60) off = e.tick;
    }
    CHECK(on == 0 && off == 1728);
    // Piccolo (channel 1) plays bar 2: a''4 staccato at beat 1.
    bool picc = false;
    for (const auto& e : f.tracks[2])
        picc |= e.status == 0x91 && e.a == 81 && e.tick == 4 * 960;
    CHECK(picc);
}

TEST(export_midi_unrolls_repeats) {
    model::Project p;
    auto ids = fixture::makeProject(p);
    auto* m = p.movement(ids.mov);
    m->barlines = {{{p.allocateId()}, Fraction{2}, model::BarlineKind::RepeatEnd}};
    const std::string path = tmpPath("corinzia-export-repeat.mid");
    CHECK(exporter::writeMidi(p, *m, path).empty());
    const Smf f = parseSmf(slurp(path));
    int ons = 0;
    std::uint32_t last = 0;
    for (std::size_t t = 1; t < f.tracks.size(); ++t)
        for (const auto& e : f.tracks[t])
            if ((e.status & 0xf0) == 0x90) ++ons, last = std::max(last, e.tick);
    CHECK(ons == 2 * 6);              // 6 note-ons per pass (the tie merges two events)
    CHECK(last == 8 * 960 + 6 * 960);   // c''' of the second pass
}

TEST(export_musicxml_structure) {
    model::Project p;
    auto ids = fixture::makeProject(p);
    auto* m = p.movement(ids.mov);
    // A note crossing the barline and some notation to map.
    auto evs = std::make_shared<model::EventList>(*m->lanes[0].voices[0].events);
    (*evs)[2].duration = Fraction(3, 4);   // <e' g'> from beat 3 over the barline
    (*evs)[2].articulations = model::ArtArpeggio;
    evs->erase(evs->begin() + 3);          // make room: a'' gone
    (*evs)[3].start = Fraction(5, 4);
    (*evs)[1].spans = model::SlurStart;
    (*evs)[2].spans = model::SlurEnd;
    m->lanes[0].voices[0].events = evs;
    // Bar 1, [1. bar 2 :|] [2. bar 3 (empty: a whole-bar rest)]
    m->length = Fraction{3};
    m->barlines = {{{p.allocateId()}, Fraction{2}, model::BarlineKind::RepeatEnd}};
    m->endings = {{{p.allocateId()}, Fraction{1}, Fraction{2}, 1}, {{p.allocateId()}, Fraction{2}, Fraction{3}, 2}};
    m->tempo[0].text = "Allegro & vivo";
    const std::string path = tmpPath("corinzia-export.musicxml");
    const std::string err = exporter::writeMusicXml(p, *m, path);
    CHECK(err.empty());
    if (!err.empty()) std::printf("  %s\n", err.c_str());
    const std::string x = slurp(path);
    auto count = [&](const std::string& s) {
        std::size_t n = 0;
        for (auto i = x.find(s); i != std::string::npos; i = x.find(s, i + 1)) ++n;
        return n;
    };
    CHECK(x.find("<score-partwise version=\"4.0\">") != std::string::npos);
    CHECK(count("<measure number=") == 3);
    CHECK(x.find("<rest measure=\"yes\"/>") != std::string::npos);
    CHECK(x.find("<words font-weight=\"bold\">Allegro &amp; vivo</words>") != std::string::npos);
    CHECK(count("<chord/>") >= 2);                         // the chord, split over the barline
    CHECK(count("<tie type=\"start\"/>") >= 3);            // c'~c' and both chord heads over the bar
    CHECK(x.find("<arpeggiate/>") != std::string::npos);
    CHECK(x.find("<repeat direction=\"backward\"/>") != std::string::npos);
    CHECK(x.find("<ending number=\"1\" type=\"start\">") != std::string::npos);
    CHECK(x.find("<ending number=\"1\" type=\"stop\"/>") != std::string::npos);
    CHECK(x.find("<ending number=\"2\" type=\"discontinue\"/>") != std::string::npos);
    CHECK(x.find("<dynamics><f/></dynamics>") != std::string::npos);
    CHECK(x.find("<slur type=\"start\"") != std::string::npos);

    // The real check: LilyPond's own MusicXML importer reads it back.
    if (const char* lily = std::getenv("CORINZIA_LILYPOND")) {
        std::string dir = lily;
        dir = dir.substr(0, dir.rfind('/'));
        const std::string base = tmpPath("corinzia-export-import");
        const std::string cmd = dir + "/musicxml2ly -o " + base + ".ly " + path + " 2>" + base + ".err && " + lily +
                                " -dbackend=svg -s -o " + base + " " + base + ".ly 2>>" + base + ".err";
        const int rc = std::system(("timeout 60 sh -c '" + cmd + "'").c_str());   // musicxml2ly can loop on odd input
        CHECK(rc == 0);
        if (rc != 0) std::printf("%s\n", slurp(base + ".err").c_str());
        CHECK(slurp(base + ".ly").find("\\repeat volta") != std::string::npos);
    }
}
