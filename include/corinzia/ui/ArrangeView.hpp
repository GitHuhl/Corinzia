#pragma once
// DAW arrange view: one horizontal lane per DAW track, bar grid, note blocks
// (pitch-scaled within the lane, like a mini piano roll), selection and the
// latency-compensated playhead. Pure scene graph (no QPainter on the GUI
// thread); text (track names, bar numbers) is drawn by QML on top.
//
// Node rules for the software adaptation (see ScoreCanvas): nodes are pooled
// and only ever appended fully configured; unused ones get an empty rect.

#include "corinzia/core/Types.hpp"

#include <QColor>
#include <QQuickItem>

#include <limits>
#include <vector>

namespace corinzia::ui {

struct ArrangeContent {
    struct Track { QString name; QColor color; int minKey = 60, maxKey = 72; };
    struct Block { int track; double startQ, lengthQ; int key; ElementId element; };
    std::vector<Track> tracks;
    std::vector<Block> blocks;
    std::vector<double> barQuarters;   // bar line positions (quarters), incl. the final barline
    double lengthQ = 0;
};

class ArrangeView : public QQuickItem {
    Q_OBJECT
    Q_PROPERTY(double pixelsPerQuarter READ pixelsPerQuarter WRITE setPixelsPerQuarter NOTIFY viewChanged)
    Q_PROPERTY(double scrollX READ scrollX WRITE setScrollX NOTIFY viewChanged)
    Q_PROPERTY(double laneHeight READ laneHeight WRITE setLaneHeight NOTIFY viewChanged)
    Q_PROPERTY(int trackCount READ trackCount NOTIFY contentChanged)
    Q_PROPERTY(int blockCount READ blockCount NOTIFY contentChanged)
    Q_PROPERTY(double lengthQuarters READ lengthQuarters NOTIFY contentChanged)

public:
    struct PoolLayout { QRgb color; int capacity; };   // internal scene-graph bookkeeping

    explicit ArrangeView(QQuickItem* parent = nullptr);

    void setContent(ArrangeContent c);
    void setSelection(ElementId id);
    Q_INVOKABLE void setPlayheadQuarter(double q);

    double pixelsPerQuarter() const noexcept { return ppq_; }
    void setPixelsPerQuarter(double v);
    double scrollX() const noexcept { return scrollX_; }
    void setScrollX(double v);
    double laneHeight() const noexcept { return laneH_; }
    void setLaneHeight(double v);
    int trackCount() const noexcept { return int(content_.tracks.size()); }
    int blockCount() const noexcept { return int(content_.blocks.size()); }
    double lengthQuarters() const noexcept { return content_.lengthQ; }

    // Geometry (item coordinates), exposed for QML overlays and tests.
    Q_INVOKABLE double xForQuarter(double q) const { return q * ppq_ - scrollX_; }
    QRectF blockRect(std::size_t i) const;
    const ArrangeContent& content() const noexcept { return content_; }

signals:
    void viewChanged();
    void contentChanged();
    void blockClicked(quint64 elementId);
    void timelineClicked(double quarter);   // empty space: locate the transport

protected:
    QSGNode* updatePaintNode(QSGNode* old, UpdatePaintNodeData*) override;
    void mousePressEvent(QMouseEvent* e) override;
    void wheelEvent(QWheelEvent* e) override;

private:
    ArrangeContent content_;
    ElementId selection_;
    double playheadQ_ = std::numeric_limits<double>::quiet_NaN();
    double ppq_ = 40.0, scrollX_ = 0.0, laneH_ = 64.0;
    std::vector<std::vector<PoolLayout>> layout_;   // render-thread state: current tree's pools per layer
};

}  // namespace corinzia::ui
