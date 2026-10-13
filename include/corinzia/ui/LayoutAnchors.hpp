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
    void build(const SvgHitIndex& index, const model::Movement& movement);

    struct CursorPos { bool valid = false; int page = 0; qreal x = 0, top = 0, bottom = 0; };
    CursorPos cursorAt(double quarter) const;

    struct Glyph { ElementId id; int page; QPointF center; bool rest; };
    std::vector<Glyph> provisional(const model::Movement& edited, Fraction from, Fraction to) const;

    struct Region { int page; QRectF rect; };
    std::vector<Region> staleRegions(Fraction from, Fraction to) const;

    // Input caret: like cursorAt, but limited vertically to one lane's staff.
    CursorPos caretAt(std::size_t lane, double quarter) const;

    // Mouse note entry: page point -> (lane, musical position, diatonic step).
    struct Hit { bool valid = false; std::size_t lane = 0; double quarter = 0; int diatonic = 0; };
    Hit locate(int page, QPointF pt) const;

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

    std::vector<Point> timeline_;            // one per distinct engraved onset, sorted
    std::vector<HeadSample> heads_;
    std::vector<System> systems_;
};

}  // namespace corinzia::ui
