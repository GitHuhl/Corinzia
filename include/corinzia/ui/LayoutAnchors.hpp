#pragma once
// Bridges musical time and engraved geometry, from the last definitive pass.
//
//  * cursorAt(quarter): playhead placement (page, x, system top/bottom). The
//    audio thread gives samples -> TempoMap gives quarters -> this gives pixels.
//  * provisional(...): the instant-feedback layout. Edited events are placed by
//    interpolating x between the unchanged neighbours' engraved positions and by
//    fitting y = a + b * diatonicStep to the lane's engraved noteheads in that
//    system (clef/staff-size independent, no font metrics needed). No collision
//    avoidance, no respacing: it only has to look right for ~1 s.
//  * staleRegions(...): rectangles of the definitive raster to mask while the
//    provisional glyphs are shown.

#include "corinzia/model/Model.hpp"
#include "corinzia/ui/SvgHitIndex.hpp"

#include <QPointF>
#include <QRectF>

#include <vector>

namespace corinzia::ui {

class LayoutAnchors {
public:
    // `initialClefs`: each lane's starting clef (its instrument's); missing
    // lanes count as treble. Mid-movement clef changes come from the lanes.
    void build(const SvgHitIndex& index, const model::Movement& movement, std::vector<std::string> initialClefs = {});

    struct CursorPos { bool valid = false; int page = 0; qreal x = 0, top = 0, bottom = 0; };
    CursorPos cursorAt(double quarter) const;

    struct Glyph { ElementId id; int page; QPointF center; bool rest; bool hollow = false; };
    std::vector<Glyph> provisional(const model::Movement& edited, Fraction from, Fraction to) const;

    struct Region { int page; QRectF rect; };
    std::vector<Region> staleRegions(Fraction from, Fraction to) const;

    // Everything the instant-feedback layer draws for an edit in [from, to):
    // only staves whose music changed are touched; masks cover just the old
    // notes (heads, stems) on that staff, staff lines are redrawn over them,
    // and provisional notes get stems and ledger lines.
    struct ProvisionalScene {
        std::vector<Glyph> glyphs;
        std::vector<Region> masks;        // page white
        std::vector<Region> staffLines;   // redrawn lines inside masks (ink)
        std::vector<Region> stems, ledgers;   // provisional colour
        std::vector<std::size_t> editedLanes;
    };
    ProvisionalScene provisionalScene(const SvgHitIndex& index, const model::Movement& engraved,
                                      const model::Movement& edited, Fraction from, Fraction to) const;

    // Input caret: like cursorAt, but limited vertically to one lane's staff.
    CursorPos caretAt(std::size_t lane, double quarter) const;

    // Mouse note entry: page point -> (lane, musical position, diatonic step).
    struct Hit { bool valid = false; std::size_t lane = 0; double quarter = 0; int diatonic = 0; };
    Hit locate(int page, QPointF pt) const;

    // Range selection highlight: one rectangle per system the range touches,
    // spanning the staves of lanes [laneFrom, laneTo] and the beats [from, to).
    std::vector<Region> rangeRects(std::size_t laneFrom, std::size_t laneTo, double fromQ, double toQ) const;

    std::size_t size() const noexcept { return timeline_.size(); }

private:
    struct Point { double quarter; int page; int system; qreal x; };
    struct HeadSample { std::size_t lane; int system; int diatonic; qreal y; };
    struct System { int page; qreal top, bottom, left, right; };

    const Point* before(double q) const;     // last point with quarter <  q
    const Point* atOrAfter(double q) const;  // first point with quarter >= q
    qreal xAt(double q, const Point* a, const Point* b) const;
    // y = c0 + c1 * diatonic, least squares over the lane's heads in `system`.
    bool fitLane(std::size_t lane, int system, double& c0, double& c1) const;
    int systemAt(int page, qreal y) const;
    // Heads are fitted in treble-equivalent steps: pitch step + this shift
    // (0 in treble, +12 in bass ...), so one fit spans a mid-system clef change.
    // Uses the engraved clefs: provisional glyphs must agree with the staff drawn.
    int clefShift(std::size_t lane, Fraction pos) const;
    std::vector<std::string> initialClefs_;
    std::vector<model::PlayerLane> laneClefs_;   // only .clefs is filled

    std::vector<Point> timeline_;            // one per distinct engraved onset, sorted
    std::vector<HeadSample> heads_;
    std::vector<System> systems_;
    struct StaffGeo { std::size_t lane; int system; int page; std::vector<qreal> ys; qreal left, right, thickness; };
    std::vector<StaffGeo> staves_;
    const StaffGeo* staffOf(std::size_t lane, int system) const;
};

}  // namespace corinzia::ui
