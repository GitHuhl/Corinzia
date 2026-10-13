#pragma once
// The interactive score viewport: a QQuickItem that builds raw QSGNodes on the
// render thread (Qt RHI -> Vulkan/OpenGL), never QPainter on the GUI thread.
//
// Node tree (rebuilt incrementally in updatePaintNode):
//
//   root (QSGTransformNode: pan/zoom — changing zoom is one matrix update)
//    ├─ page[i]   QSGSimpleTextureNode   definitive raster from the worker
//    ├─ mask[k]   QSGSimpleRectNode      paper-coloured cover over stale regions
//    ├─ glyphs    QSGGeometryNode        provisional heads/rests (one draw call)
//    ├─ select    QSGSimpleRectNode      selection / hover highlight
//    └─ playhead  QSGSimpleRectNode      latency-compensated cursor
//
// Dual-layer illusion: an edit calls showProvisional() synchronously (the next
// frame shows the change); when the EngravingService delivers a newer
// definitive result, setEngraving() swaps textures and drops the overlay.

#include "corinzia/model/Model.hpp"
#include "corinzia/ui/EngravingService.hpp"

#include <QColor>
#include <QQuickItem>

#include <memory>
#include <vector>

namespace corinzia::ui {

class ScoreCanvas : public QQuickItem {
    Q_OBJECT
    Q_PROPERTY(qreal zoom READ zoom WRITE setZoom NOTIFY zoomChanged)
    Q_PROPERTY(QPointF pan READ pan WRITE setPan NOTIFY panChanged)
    Q_PROPERTY(QColor paperColor MEMBER paper_ NOTIFY paperColorChanged)

public:
    explicit ScoreCanvas(QQuickItem* parent = nullptr);

    qreal zoom() const noexcept { return zoom_; }
    void setZoom(qreal z);
    QPointF pan() const noexcept { return pan_; }
    void setPan(QPointF p);

    // Definitive layer (GUI thread; called from EngravingService::engraved).
    void setEngraving(std::shared_ptr<const EngraveResult> result);
    // Instant-feedback layer for an edit in [from, to) of the edited movement.
    void showProvisional(std::shared_ptr<const model::Movement> edited, Fraction from, Fraction to);
    // Playhead in musical time (quarters from movement start); NaN hides it.
    Q_INVOKABLE void setPlayheadQuarter(double quarter);
    void setSelection(ElementId id);

    // Geometry helpers (item coordinates <-> page/staff-space coordinates).
    struct PagePoint { int page = -1; QPointF pt; };
    PagePoint toPage(QPointF itemPos) const;
    QRectF pageRectInItem(int page) const;

    const EngraveResult* engraving() const noexcept { return result_.get(); }

signals:
    void zoomChanged();
    void panChanged();
    void paperColorChanged();
    // Emitted on click: the semantic object and the DAW track it sounds through.
    void elementClicked(quint64 elementId, quint64 voiceId, quint64 trackId, Qt::KeyboardModifiers mods);
    void emptyClicked(int page, QPointF staffSpacePos);

protected:
    QSGNode* updatePaintNode(QSGNode* old, UpdatePaintNodeData*) override;
    void mousePressEvent(QMouseEvent* e) override;
    void hoverMoveEvent(QHoverEvent* e) override;
    void wheelEvent(QWheelEvent* e) override;

private:
    qreal pageScale() const noexcept { return zoom_ * kBasePxPerSpace; }
    qreal pageTop(int page) const;

    static constexpr qreal kBasePxPerSpace = 8.0;
    static constexpr qreal kPageGap = 6.0;   // staff spaces between pages

    std::shared_ptr<const EngraveResult> result_;
    std::vector<LayoutAnchors::Glyph> glyphs_;
    std::vector<LayoutAnchors::Region> masks_;
    ElementId selection_, hover_;
    double playheadQ_ = std::numeric_limits<double>::quiet_NaN();
    qreal zoom_ = 1.0;
    QPointF pan_;
    QColor paper_ = QColor(255, 255, 250);
    bool texturesDirty_ = false;
};

}  // namespace corinzia::ui
