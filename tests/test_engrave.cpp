#include "Fixture.hpp"
#include "Test.hpp"

#include "corinzia/engrave/LilyPondEmitter.hpp"

#include <cstdio>
#include <cstdlib>
#include <fstream>
#include <sstream>

using namespace corinzia;

TEST(lily_building_blocks) {
    CHECK(engrave::lilyPitch({0, 0, 4}) == "c'");
    CHECK(engrave::lilyPitch({3, 1, 5}) == "fis''");
    CHECK(engrave::lilyPitch({6, -1, 2}) == "bes,");
    CHECK(engrave::lilyDuration({1, 4}) == "4");
    CHECK(engrave::lilyDuration({3, 8}) == "4.");
    CHECK(engrave::lilyDuration({7, 16}) == "4..");
    CHECK(engrave::lilyDuration({1, 6}) == "4*2/3");            // triplet quarter
    CHECK(engrave::lilyKey(-3, true) == "\\key c \\minor");
}

static std::string emitFixture(model::Project& p, fixture::Ids& ids) {
    ids = fixture::makeProject(p);
    engrave::EngraveContext ctx;
    for (auto& pl : p.players) {
        ctx.playerNames[pl->id] = pl->name;
        for (auto& i : pl->instruments) { ctx.instruments[i->id] = i.get(); ctx.instrumentTracks[i->id] = i->track; }
    }
    return engrave::emitLilyPond(*p.movements[0], ctx);
}

TEST(lily_emitter_tags_every_event) {
    model::Project p;
    fixture::Ids ids;
    const std::string ly = emitFixture(p, ids);
    for (const auto& e : *p.movements[0]->lanes[0].voices[0].events)
        for (const auto& h : e.heads) CHECK(ly.find("\"" + h.id.id.toSvgId() + "\"") != std::string::npos);
    CHECK(ly.find("\\set Staff.instrumentName = \"Picc.\"") != std::string::npos);
    CHECK(ly.find(" t-" + ids.t2.id.toSvgId().substr(2)) != std::string::npos);
}

// Integration: only when LilyPond is available (the nix dev shell exports it).
TEST(lily_svg_roundtrip_ids) {
    const char* lily = std::getenv("CORINZIA_LILYPOND");
    if (!lily) { std::printf("  (skipped: CORINZIA_LILYPOND not set)\n"); return; }
    model::Project p;
    fixture::Ids ids;
    const std::string ly = emitFixture(p, ids);
    const std::string dir = std::string(std::getenv("TMPDIR") ? std::getenv("TMPDIR") : "/tmp");
    const std::string base = dir + "/corinzia_engrave_test";
    { std::ofstream(base + ".ly") << ly; }
    const std::string cmd = std::string(lily) + " -dbackend=svg -dno-point-and-click -s -o " + base + " " + base + ".ly";
    CHECK(std::system(cmd.c_str()) == 0);
    std::stringstream svg;
    svg << std::ifstream(base + ".svg").rdbuf();
    for (const auto& e : *p.movements[0]->lanes[0].voices[0].events)
        for (const auto& h : e.heads) CHECK(svg.str().find("id=\"" + h.id.id.toSvgId() + "\"") != std::string::npos);
}

TEST(lily_tempo_ramps_print_rit_not_duplicate_marks) {
    model::Project p;
    fixture::Ids ids;
    emitFixture(p, ids);
    auto& m = *p.movements[0];
    m.tempo = {{Fraction{0}, 96.0, false}, {Fraction{1, 2}, 96.0, true}, {Fraction{3, 2}, 72.0, false}};
    engrave::EngraveContext ctx;
    const std::string ly = engrave::emitLilyPond(m, ctx);
    auto count = [&](const std::string& needle) {
        std::size_t n = 0;
        for (auto pos = ly.find(needle); pos != std::string::npos; pos = ly.find(needle, pos + 1)) ++n;
        return n;
    };
    CHECK(count("\\tempo 4 = 96") == 1);
    CHECK(count("\\tempo \"rit.\"") == 1);
    CHECK(count("\\tempo 4 = 72") == 1);
}

TEST(lily_triplets_and_barline_completion) {
    model::Project p;
    fixture::Ids ids;
    emitFixture(p, ids);
    auto& m = *p.movements[0];
    auto evs = std::make_shared<model::EventList>();
    for (int i = 0; i < 3; ++i) evs->push_back(fixture::ev(p, Fraction(i, 12), Fraction(1, 12), {fixture::head(p, i, 5)}));
    evs->push_back(fixture::ev(p, Fraction(1, 4), Fraction(1, 2), {fixture::head(p, 4, 4)}));
    evs->push_back(fixture::ev(p, Fraction(3, 4), Fraction(1, 2), {fixture::head(p, 5, 4)}));   // crosses the barline
    m.lanes[0].voices[0].events = evs;
    engrave::EngraveContext ctx;
    const std::string ly = engrave::emitLilyPond(m, ctx);
    CHECK(ly.find("\\tuplet 3/2 {") != std::string::npos);
    CHECK(ly.find("*2/3") == std::string::npos);
    CHECK(ly.find("Completion_heads_engraver") != std::string::npos);
    if (const char* lily = std::getenv("CORINZIA_LILYPOND")) {
        const std::string base = std::string(std::getenv("TMPDIR") ? std::getenv("TMPDIR") : "/tmp") + "/corinzia_tuplet";
        { std::ofstream(base + ".ly") << ly; }
        const std::string cmd = std::string(lily) + " -dbackend=svg -s -o " + base + " " + base + ".ly";
        CHECK(std::system(cmd.c_str()) == 0);
    }
}

TEST(lily_panorama_uses_one_line_breaking) {
    model::Project p;
    fixture::Ids ids;
    emitFixture(p, ids);
    engrave::EngraveContext ctx;
    CHECK(engrave::emitLilyPond(*p.movements[0], ctx).find("set-paper-size") != std::string::npos);
    ctx.panorama = true;
    const std::string ly = engrave::emitLilyPond(*p.movements[0], ctx);
    CHECK(ly.find("ly:one-line-auto-height-breaking") != std::string::npos && ly.find("set-paper-size") == std::string::npos);
}

TEST(lily_slurs_and_hairpins_compile) {
    model::Project p;
    fixture::Ids ids;
    emitFixture(p, ids);
    auto& m = *p.movements[0];
    auto evs = std::make_shared<model::EventList>(*m.lanes[0].voices[0].events);
    (*evs)[1].spans = model::SlurStart | model::CrescStart;
    (*evs)[2].spans = model::SlurEnd | model::HairpinEnd;
    (*evs)[3].spans = model::DimStart;
    (*evs)[5].spans = model::HairpinEnd;   // has a dynamic (f): ends the hairpin itself
    m.lanes[0].voices[0].events = evs;
    engrave::EngraveContext ctx;
    const std::string ly = engrave::emitLilyPond(m, ctx);
    CHECK(ly.find("(\\<") != std::string::npos && ly.find("\\!)") != std::string::npos && ly.find("\\>") != std::string::npos);
    if (const char* lily = std::getenv("CORINZIA_LILYPOND")) {
        const std::string base = std::string(std::getenv("TMPDIR") ? std::getenv("TMPDIR") : "/tmp") + "/corinzia_spans";
        { std::ofstream(base + ".ly") << ly; }
        const std::string cmd = std::string(lily) + " -dbackend=svg -s -o " + base + " " + base + ".ly 2>" + base + ".err";
        CHECK(std::system(cmd.c_str()) == 0);
        std::stringstream err;
        err << std::ifstream(base + ".err").rdbuf();
        CHECK(err.str().find("warning") == std::string::npos);   // no unterminated/unmatched spanners
    }
}

TEST(lily_structure_and_text_compile) {
    model::Project p;
    fixture::Ids ids;
    emitFixture(p, ids);
    auto& m = *p.movements[0];
    using K = model::BarlineKind;
    m.tempo[0].text = "Allegro \"vivo\"";
    m.barlines = {{{p.allocateId()}, Fraction{1}, K::RepeatEnd}, {{p.allocateId()}, m.length, K::Final}};
    const ElementId markId = p.allocateId(), textId = p.allocateId();
    m.rehearsal = {{{markId}, Fraction{1}}};
    m.lanes[0].clefs = {{{p.allocateId()}, Fraction(3, 2), "bass"}};
    m.lanes[0].texts = {{{textId}, Fraction{0}, "dolce", model::TextKind::Expression},
                        {{p.allocateId()}, Fraction(1, 2), "pizz.", model::TextKind::Technique}};
    engrave::EngraveContext ctx;
    const std::string ly = engrave::emitLilyPond(m, ctx);
    CHECK(ly.find("\\tempo \"Allegro \\\"vivo\\\"\" 4 = 120") != std::string::npos);
    CHECK(ly.find("\\bar \":|.\"") != std::string::npos && ly.find("\\bar \"|.\"") != std::string::npos);
    CHECK(ly.find("\\mark \\default") != std::string::npos);
    CHECK(ly.find("\\clef \"bass\"") != std::string::npos);
    CHECK(ly.find("\\italic \"dolce\"") != std::string::npos && ly.find("^\\markup \"pizz.\"") != std::string::npos);
    if (const char* lily = std::getenv("CORINZIA_LILYPOND")) {
        const std::string base = std::string(std::getenv("TMPDIR") ? std::getenv("TMPDIR") : "/tmp") + "/corinzia_structure";
        { std::ofstream(base + ".ly") << ly; }
        const std::string cmd = std::string(lily) + " -dbackend=svg -s -o " + base + " " + base + ".ly 2>" + base + ".err";
        CHECK(std::system(cmd.c_str()) == 0);
        std::stringstream err, svg;
        err << std::ifstream(base + ".err").rdbuf();
        CHECK(err.str().find("warning") == std::string::npos);
        if (err.str().find("warning") != std::string::npos) std::printf("%s\n", err.str().c_str());
        svg << std::ifstream(base + ".svg").rdbuf();
        CHECK(svg.str().find(markId.toSvgId()) != std::string::npos);   // clickable mark and text
        CHECK(svg.str().find(textId.toSvgId()) != std::string::npos);
    }
}
