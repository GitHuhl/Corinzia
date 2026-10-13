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
