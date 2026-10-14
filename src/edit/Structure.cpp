#include "corinzia/edit/Structure.hpp"

#include <algorithm>
#include <cctype>
#include <cmath>
#include <cstring>
#include <map>

namespace corinzia::edit {

using model::BarlineKind;
using model::MovementHeader;

namespace {

std::string trim(std::string s) {
    auto ws = [](unsigned char c) { return std::isspace(c) != 0; };
    while (!s.empty() && ws((unsigned char)s.front())) s.erase(s.begin());
    while (!s.empty() && ws((unsigned char)s.back())) s.pop_back();
    return s;
}
std::string lower(std::string s) {
    for (auto& c : s) c = char(std::tolower((unsigned char)c));
    return s;
}
// Unicode accidentals -> ASCII.
std::string asciiAccidentals(std::string s) {
    auto rep = [&](const std::string& from, const std::string& to) {
        for (std::size_t p; (p = s.find(from)) != std::string::npos;) s.replace(p, from.size(), to);
    };
    rep("♭", "b");
    rep("♯", "#");
    rep("♮", "");
    return s;
}

template <class T>
void replaceAt(std::vector<T>& v, Fraction at, T item) {
    std::erase_if(v, [&](const T& x) { return x.at == at; });
    v.push_back(std::move(item));
    std::sort(v.begin(), v.end(), [](const T& a, const T& b) { return a.at < b.at; });
}

// Maps (tempo, meter, key, instrument changes) always keep their entry at 0
// and hold one entry per position; marks and text are plain items.
enum class Kind { Map, Items };

template <class T>
void shiftItems(std::vector<T>& v, Fraction at, Fraction delta, Kind kind) {
    for (auto& x : v)
        if (x.at >= at && (kind == Kind::Items || x.at > Fraction{0})) x.at = x.at + delta;
}
template <class T>
void deleteItems(std::vector<T>& v, Fraction from, Fraction to, Kind kind) {
    std::erase_if(v, [&](const T& x) { return x.at >= from && x.at < to && (kind == Kind::Items || x.at > Fraction{0}); });
    for (auto& x : v)
        if (x.at >= to) x.at = x.at - (to - from);
    std::stable_sort(v.begin(), v.end(), [](const T& a, const T& b) { return a.at < b.at; });
    if (kind == Kind::Items) return;
    // An entry shifted onto `from` supersedes the one kept there (e.g. the initial key).
    std::vector<T> out;
    for (auto& x : v) {
        if (!out.empty() && out.back().at == x.at && x.at == from) out.back() = std::move(x);
        else out.push_back(std::move(x));
    }
    v = std::move(out);
}

}  // namespace

// ---------------------------------------------------------------- parsing --
std::optional<KeySig> parseKey(std::string text) {
    text = asciiAccidentals(trim(text));
    if (text.empty()) return std::nullopt;
    bool minor = std::islower((unsigned char)text[0]) != 0;
    std::string t = lower(text);
    for (const char* suffix : {" minor", "minor", " min", "min", "m"})
        if (t.size() > std::strlen(suffix) && t.ends_with(suffix)) {
            t.resize(t.size() - std::strlen(suffix));
            minor = true;
            break;
        }
    for (const char* suffix : {" major", "major", " maj", "maj"})
        if (t.size() > std::strlen(suffix) && t.ends_with(suffix)) {
            t.resize(t.size() - std::strlen(suffix));
            minor = false;
            break;
        }
    t = trim(t);
    static const std::map<std::string, int> kMajor = {{"cb", -7}, {"gb", -6}, {"db", -5}, {"ab", -4}, {"eb", -3}, {"bb", -2},
                                                      {"f", -1},  {"c", 0},   {"g", 1},   {"d", 2},   {"a", 3},   {"e", 4},
                                                      {"b", 5},   {"f#", 6},  {"c#", 7}};
    auto it = kMajor.find(t);
    if (it == kMajor.end()) return std::nullopt;
    // A minor key has the signature of its relative major (three fifths lower).
    const int fifths = minor ? it->second - 3 : it->second;
    if (fifths < -7 || fifths > 7) return std::nullopt;
    return KeySig{fifths, minor};
}

std::optional<std::pair<int, int>> parseTime(std::string text) {
    text = lower(trim(text));
    if (text == "c" || text == "common") return std::pair{4, 4};
    if (text == "c|" || text == "cut" || text == "alla breve") return std::pair{2, 2};
    const auto slash = text.find('/');
    if (slash == std::string::npos) return std::nullopt;
    try {
        const int n = std::stoi(text.substr(0, slash)), d = std::stoi(text.substr(slash + 1));
        const bool pow2 = d > 0 && (d & (d - 1)) == 0 && d <= 64;
        if (n < 1 || n > 32 || !pow2) return std::nullopt;
        return std::pair{n, d};
    } catch (...) {
        return std::nullopt;
    }
}

std::optional<std::string> parseClef(std::string text) {
    text = lower(trim(text));
    static const std::map<std::string, std::string> kClefs = {
        {"treble", "treble"}, {"g", "treble"}, {"violin", "treble"}, {"bass", "bass"}, {"f", "bass"},
        {"alto", "alto"}, {"c", "alto"}, {"viola", "alto"}, {"tenor", "tenor"}, {"treble_8", "treble_8"},
        {"g8", "treble_8"}, {"bass_8", "bass_8"}, {"f8", "bass_8"}, {"percussion", "percussion"}, {"perc", "percussion"}};
    auto it = kClefs.find(text);
    return it == kClefs.end() ? std::nullopt : std::optional<std::string>(it->second);
}

std::optional<BarlineKind> parseBarline(std::string text) {
    text = lower(trim(text));
    if (text == "||" || text == "double") return BarlineKind::Double;
    if (text == "|." || text == "final" || text == "end") return BarlineKind::Final;
    if (text == "|:" || text == ".|:" || text == "start repeat" || text == "start-repeat") return BarlineKind::RepeatStart;
    if (text == ":|" || text == ":|." || text == "end repeat" || text == "end-repeat" || text == "repeat") return BarlineKind::RepeatEnd;
    if (text == ":|:" || text == ":|.|:" || text == "double repeat") return BarlineKind::RepeatBoth;
    return std::nullopt;
}

std::optional<TempoSpec> parseTempo(std::string text) {
    text = trim(text);
    if (text.empty()) return std::nullopt;
    // Trailing number = bpm; "q=96" / "♩=96" / "=96" forms too.
    std::string words = text;
    std::optional<double> bpm;
    const auto eq = text.find('=');
    std::size_t numStart = std::string::npos;
    if (eq != std::string::npos) {
        numStart = eq + 1;
        words = trim(text.substr(0, eq));
        if (words == "q" || words == "Q" || words == "♩" || words == "crotchet" || words == "quarter") words.clear();
    } else {
        std::size_t p = text.size();
        while (p > 0 && (std::isdigit((unsigned char)text[p - 1]) || text[p - 1] == '.')) --p;
        if (p < text.size()) {
            numStart = p;
            words = trim(text.substr(0, p));
        }
    }
    if (numStart != std::string::npos) {
        try {
            bpm = std::stod(trim(text.substr(numStart)));
        } catch (...) {
            return std::nullopt;
        }
        if (*bpm < 10 || *bpm > 400) return std::nullopt;
    }
    if (!bpm) {   // a word alone: conventional tempo
        static const std::map<std::string, double> kWords = {
            {"grave", 40},   {"largo", 50},     {"lento", 56},    {"adagio", 70},  {"andante", 90},
            {"andantino", 96}, {"moderato", 110}, {"allegretto", 116}, {"allegro", 132}, {"vivace", 156},
            {"presto", 168}, {"prestissimo", 200}};
        auto it = kWords.find(lower(words));
        if (it == kWords.end()) return std::nullopt;
        bpm = it->second;
    }
    return TempoSpec{words, *bpm};
}

std::optional<std::uint16_t> parseOrnament(std::string text) {
    text = lower(trim(text));
    static const std::map<std::string, std::uint16_t> kOrn = {
        {"tr", model::ArtTrill}, {"trill", model::ArtTrill},
        {"mordent", model::ArtMordent}, {"mord", model::ArtMordent}, {"m", model::ArtMordent},
        {"turn", model::ArtTurn}, {"~", model::ArtTurn},
        {"trem", model::ArtTremolo}, {"tremolo", model::ArtTremolo}, {"///", model::ArtTremolo},
        {"arp", model::ArtArpeggio}, {"arpeggio", model::ArtArpeggio}, {"roll", model::ArtArpeggio},
        {"breath", model::ArtBreath}, {",", model::ArtBreath},
        {"caesura", model::ArtCaesura}, {"//", model::ArtCaesura},
        {"staccatissimo", model::ArtStaccatissimo}, {"!", model::ArtStaccatissimo},
        {"staccato", model::ArtStaccato}, {".", model::ArtStaccato},
        {"accent", model::ArtAccent}, {">", model::ArtAccent},
        {"tenuto", model::ArtTenuto}, {"-", model::ArtTenuto},
        {"marcato", model::ArtMarcato}, {"^", model::ArtMarcato},
        {"fermata", model::ArtFermata}, {"ferm", model::ArtFermata}};
    auto it = kOrn.find(text);
    return it == kOrn.end() ? std::nullopt : std::optional<std::uint16_t>(it->second);
}

// ------------------------------------------------------------ positions --
Fraction barLength(const MovementHeader& h, Fraction pos) {
    Fraction len{1};
    for (const auto& m : h.meter)
        if (m.at <= pos) len = Fraction(m.numerator, m.denominator);
    return len;
}

Fraction barStartAtOrBefore(const MovementHeader& h, Fraction pos) {
    Fraction start{0};
    for (const auto& m : h.meter)
        if (m.at <= pos) start = m.at;
    const Fraction len = barLength(h, pos);
    const Fraction rel = pos - start;
    const std::int64_t bars = (rel.num * len.den) / (rel.den * len.num);   // floor (non-negative)
    return start + len * Fraction(bars);
}

Fraction nearestBarline(const MovementHeader& h, Fraction pos) {
    const Fraction a = barStartAtOrBefore(h, pos);
    const Fraction b = std::min(a + barLength(h, pos), h.length);
    return (pos - a) <= (b - pos) ? a : b;
}

// --------------------------------------------------------- header edits --
void setKey(MovementHeader& h, Fraction at, KeySig key) { replaceAt(h.keys, at, model::KeyEvent{at, key.fifths, key.minor}); }
void setMeter(MovementHeader& h, Fraction at, int num, int den) { replaceAt(h.meter, at, model::MeterEvent{at, num, den}); }
void setTempo(MovementHeader& h, Fraction at, const TempoSpec& t) {
    replaceAt(h.tempo, at, model::TempoEvent{at, t.bpm, false, t.text});
}
void setBarline(MovementHeader& h, Fraction at, BarlineKind kind, IdAlloc& ids) {
    replaceAt(h.barlines, at, model::BarlineMark{{ids()}, at, kind});
}
void toggleRehearsalMark(MovementHeader& h, Fraction at, IdAlloc& ids) {
    if (std::erase_if(h.rehearsal, [&](const model::RehearsalMark& r) { return r.at == at; })) return;
    replaceAt(h.rehearsal, at, model::RehearsalMark{{ids()}, at});
}
void setClef(MovementHeader& h, PlayerId lane, Fraction at, const std::string& clef, IdAlloc& ids) {
    for (auto& l : h.lanes)
        if (l.player == lane) replaceAt(l.clefs, at, model::ClefChange{{ids()}, at, clef});
}
void addText(MovementHeader& h, PlayerId lane, Fraction at, const std::string& text, model::TextKind kind, IdAlloc& ids) {
    for (auto& l : h.lanes)
        if (l.player == lane) {
            l.texts.push_back({{ids()}, at, text, kind});
            std::stable_sort(l.texts.begin(), l.texts.end(), [](const auto& a, const auto& b) { return a.at < b.at; });
        }
}

std::optional<std::string> describeItem(const MovementHeader& h, ElementId id) {
    static const char* kBar[] = {"Double barline", "Final barline", "Start repeat", "End repeat", "End/start repeat"};
    for (const auto& b : h.barlines) if (b.id.id == id) return std::string(kBar[int(b.kind)]);
    for (const auto& r : h.rehearsal) if (r.id.id == id) return std::string("Rehearsal mark");
    for (const auto& l : h.lanes) {
        for (const auto& c : l.clefs) if (c.id.id == id) return "Clef " + c.clef;
        for (const auto& t : l.texts)
            if (t.id.id == id) return std::string(t.kind == model::TextKind::Expression ? "Expression" : "Technique") + " \"" + t.text + "\"";
    }
    return std::nullopt;
}

std::optional<std::string> removeItem(MovementHeader& h, ElementId id) {
    auto what = describeItem(h, id);
    if (!what) return std::nullopt;
    auto match = [&](const auto& x) { return x.id.id == id; };
    std::erase_if(h.barlines, match);
    std::erase_if(h.rehearsal, match);
    for (auto& l : h.lanes) {
        std::erase_if(l.clefs, match);
        std::erase_if(l.texts, match);
    }
    return what;
}

// ------------------------------------------------------ insert / delete --
model::EventList insertSpan(const model::EventList& list, Fraction at, Fraction length) {
    model::EventList out = list;
    for (auto& e : out)
        if (e.start >= at) e.start = e.start + length;
    return out;
}

model::EventList deleteSpan(const model::EventList& list, Fraction from, Fraction to) {
    model::EventList out;
    const Fraction gap = to - from;
    for (const auto& e : list) {
        const Fraction end = e.start + e.duration;
        if (e.start >= to) {
            model::MusicEvent m = e;
            m.start = e.start - gap;
            out.push_back(std::move(m));
        } else if (e.start < from) {
            model::MusicEvent m = e;
            if (end > from) {   // crosses into the deleted bars: cut at `from`
                m.duration = from - e.start;
                for (auto& h : m.heads) h.tieToNext = false;
            }
            out.push_back(std::move(m));
        }
        // events starting inside [from, to) are deleted
    }
    return out;
}

void insertSpan(MovementHeader& h, Fraction at, Fraction length) {
    shiftItems(h.tempo, at, length, Kind::Map);
    shiftItems(h.meter, at, length, Kind::Map);
    shiftItems(h.keys, at, length, Kind::Map);
    // A barline at `at` closes the bar before the insertion point and stays;
    // a start repeat there belongs to the bar that moves.
    for (auto& b : h.barlines)
        if (b.at > at || (b.at == at && b.kind == BarlineKind::RepeatStart)) b.at = b.at + length;
    shiftItems(h.rehearsal, at, length, Kind::Items);
    for (auto& l : h.lanes) {
        shiftItems(l.changes, at, length, Kind::Map);
        shiftItems(l.clefs, at, length, Kind::Items);
        shiftItems(l.texts, at, length, Kind::Items);
    }
    h.length = h.length + length;
}

void deleteSpan(MovementHeader& h, Fraction from, Fraction to) {
    deleteItems(h.tempo, from, to, Kind::Map);
    deleteItems(h.meter, from, to, Kind::Map);
    deleteItems(h.keys, from, to, Kind::Map);
    // A barline sits *between* bars: the one at `from` ends the bar before the
    // cut and stays; one at `to` (e.g. the final barline) closes the gap and
    // supersedes it; those strictly inside go.
    std::erase_if(h.barlines, [&](const model::BarlineMark& b) { return b.at > from && b.at < to; });
    for (auto& b : h.barlines)
        if (b.at >= to) b.at = b.at - (to - from);
    std::stable_sort(h.barlines.begin(), h.barlines.end(), [](const auto& a, const auto& b) { return a.at < b.at; });
    for (std::size_t i = 0; i + 1 < h.barlines.size(); ++i)
        if (h.barlines[i].at == from && h.barlines[i + 1].at == from) {   // the moved one comes second
            h.barlines.erase(h.barlines.begin() + std::ptrdiff_t(i));
            break;
        }
    deleteItems(h.rehearsal, from, to, Kind::Items);
    for (auto& l : h.lanes) {
        deleteItems(l.changes, from, to, Kind::Map);
        deleteItems(l.clefs, from, to, Kind::Items);
        deleteItems(l.texts, from, to, Kind::Items);
    }
    h.length = h.length - (to - from);
}

}  // namespace corinzia::edit
