#include "corinzia/ui/ScoreCanvas.hpp"

#include <QQuickWindow>
#include <QSGFlatColorMaterial>
#include <QSGGeometryNode>
#include <QSGSimpleRectNode>
#include <QSGSimpleTextureNode>
#include <QSGTransformNode>

#include <cmath>

namespace corinzia::ui {

namespace {
// Child slots of the root transform node, in paint order.
enum Slot { PagesSlot = 0, OverlaySlot = 1 };

QSGGeometryNode* makeGlyphNode(const std::vector<LayoutAnchors::Glyph>& glyphs, const std::vector<qreal>& pageTops,
                               QColor color) {
    constexpr int kSeg = 10;
    int verts = 0;
    for (const auto& g : glyphs) verts += g.rest ? 6 : kSeg * 3;
    auto* geom = new QSGGeometry(QSGGeometry::defaultAttributes_Point2D(), verts);
    geom->setDrawingMode(QSGGeometry::DrawTriangles);
    auto* v = geom->vertexDataAsPoint2D();
    int k = 0;
    for (const auto& g : glyphs) {
        const qreal top = g.page < int(pageTops.size()) ? pageTops[std::size_t(g.page)] : 0;
        const float cx = float(g.center.x()), cy = float(g.center.y() + top);
        if (g.rest) {   // placeholder block, 0.8 x 2 spaces
            const float x0 = cx - 0.4f, x1 = cx + 0.4f, y0 = cy - 1.f, y1 = cy + 1.f;
            v[k++].set(x0, y0); v[k++].set(x1, y0); v[k++].set(x1, y1);
            v[k++].set(x0, y0); v[k++].set(x1, y1); v[k++].set(x0, y1);
            continue;
        }
        // Notehead: slightly tilted ellipse 1.3 x 0.95 staff spaces.
        const float rx = 0.65f, ry = 0.475f, tilt = -0.35f;
        for (int s = 0; s < kSeg; ++s) {
            auto pt = [&](int i) {
                const float a = 2.f * float(M_PI) * float(i) / kSeg;
                const float x = rx * std::cos(a), y = ry * std::sin(a);
                return QPointF(cx + x * std::cos(tilt) - y * std::sin(tilt), cy + x * std::sin(tilt) + y * std::cos(tilt));
            };
            const QPointF p0 = pt(s), p1 = pt(s + 1);
            v[k++].set(cx, cy);
            v[k++].set(float(p0.x()), float(p0.y()));
            v[k++].set(float(p1.x()), float(p1.y()));
        }
    }
    auto* node = new QSGGeometryNode;
    node->setGeometry(geom);
    node->setFlag(QSGNode::OwnsGeometry);
    auto* mat = new QSGFlatColorMaterial;
    mat->setColor(color);
    node->setMaterial(mat);
    node->setFlag(QSGNode::OwnsMaterial);
    return node;
}
}  // namespace

ScoreCanvas::ScoreCanvas(QQuickItem* parent) : QQuickItem(parent) {
    setFlag(ItemHasContents, true);
    setAcceptedMouseButtons(Qt::LeftButton | Qt::RightButton);
    setAcceptHoverEvents(true);
}

void ScoreCanvas::setZoom(qreal z) {
    z = std::clamp(z, 0.1, 16.0);
    if (qFuzzyCompare(z, zoom_)) return;
    zoom_ = z;
    emit zoomChanged();
    update();   // GPU rescales existing textures this frame; no re-engrave needed
}

void ScoreCanvas::setPan(QPointF p) {
    if (p == pan_) return;
    pan_ = p;
    emit panChanged();
    update();
}

void ScoreCanvas::setEngraving(std::shared_ptr<const EngraveResult> result) {
    if (result_ && result && result->movement == result_->movement && result->revision < result_->revision) return;
    result_ = std::move(result);
    glyphs_.clear();   // the definitive layer now reflects the edit
    masks_.clear();
    texturesDirty_ = true;
    update();
}

void ScoreCanvas::showProvisional(std::shared_ptr<const model::Movement> edited, Fraction from, Fraction to) {
    if (!result_ || !result_->anchors || !edited) return;
    masks_ = result_->anchors->staleRegions(from, to);
    auto glyphs = result_->anchors->provisional(*edited, from, to);
    // Keep provisional glyphs from earlier edits outside this range (several
    // quick edits can land before the next definitive pass).
    std::erase_if(glyphs_, [&](const LayoutAnchors::Glyph& g) {
        return std::any_of(glyphs.begin(), glyphs.end(), [&](const auto& n) { return n.id == g.id; });
    });
    glyphs_.insert(glyphs_.end(), glyphs.begin(), glyphs.end());
    update();
}

void ScoreCanvas::setPlayheadQuarter(double q) {
    playheadQ_ = q;
    update();
}

void ScoreCanvas::setSelection(ElementId id) {
    selection_ = id;
    update();
}

qreal ScoreCanvas::pageTop(int page) const {
    qreal y = 0;
    for (int i = 0; result_ && i < page && i < int(result_->pages.size()); ++i)
        y += result_->pages[std::size_t(i)].size.height() + kPageGap;
    return y;
}

QRectF ScoreCanvas::pageRectInItem(int page) const {
    if (!result_ || page >= int(result_->pages.size())) return {};
    const auto& sz = result_->pages[std::size_t(page)].size;
    const qreal s = pageScale();
    return QRectF(pan_.x(), pan_.y() + pageTop(page) * s, sz.width() * s, sz.height() * s);
}

ScoreCanvas::PagePoint ScoreCanvas::toPage(QPointF itemPos) const {
    if (!result_) return {};
    const qreal s = pageScale();
    const QPointF doc = (itemPos - pan_) / s;
    for (int i = 0; i < int(result_->pages.size()); ++i) {
        const qreal top = pageTop(i);
        const auto& sz = result_->pages[std::size_t(i)].size;
        if (doc.y() >= top && doc.y() < top + sz.height()) return {i, QPointF(doc.x(), doc.y() - top)};
    }
    return {};
}

void ScoreCanvas::mousePressEvent(QMouseEvent* e) {
    const PagePoint pp = toPage(e->position());
    if (pp.page < 0 || !result_ || !result_->index) return QQuickItem::mousePressEvent(e);
    if (const auto* hit = result_->index->hitTest(pp.page, pp.pt)) {
        setSelection(hit->id);
        emit elementClicked(hit->id.value, hit->voice, hit->track, e->modifiers());
    } else {
        emit emptyClicked(pp.page, pp.pt);
    }
    e->accept();
}

void ScoreCanvas::hoverMoveEvent(QHoverEvent* e) {
    const PagePoint pp = toPage(e->position());
    ElementId h;
    if (pp.page >= 0 && result_ && result_->index)
        if (const auto* hit = result_->index->hitTest(pp.page, pp.pt)) h = hit->id;
    if (h != hover_) {
        hover_ = h;
        update();
    }
}

void ScoreCanvas::wheelEvent(QWheelEvent* e) {
    if (e->modifiers() & Qt::ControlModifier) {
        // Zoom around the cursor: keep the document point under the mouse fixed.
        const QPointF before = (e->position() - pan_) / pageScale();
        setZoom(zoom_ * std::pow(1.0015, e->angleDelta().y()));
        setPan(e->position() - before * pageScale());
    } else {
        setPan(pan_ + QPointF(e->angleDelta().x(), e->angleDelta().y()) * 0.5);
    }
    e->accept();
}

QSGNode* ScoreCanvas::updatePaintNode(QSGNode* old, UpdatePaintNodeData*) {
    auto* root = static_cast<QSGTransformNode*>(old);
    if (!root) {
        root = new QSGTransformNode;
        root->appendChildNode(new QSGNode);   // PagesSlot
        root->appendChildNode(new QSGNode);   // OverlaySlot
        texturesDirty_ = true;
    }
    QMatrix4x4 m;
    m.translate(float(pan_.x()), float(pan_.y()));
    m.scale(float(pageScale()));
    root->setMatrix(m);

    std::vector<qreal> tops;
    for (int i = 0; result_ && i < int(result_->pages.size()); ++i) tops.push_back(pageTop(i));

    // ---- definitive pages (re-uploaded only when a new engraving arrives) ----
    QSGNode* pages = root->childAtIndex(PagesSlot);
    if (texturesDirty_) {
        while (QSGNode* c = pages->firstChild()) { pages->removeChildNode(c); delete c; }
        for (int i = 0; result_ && i < int(result_->pages.size()); ++i) {
            const auto& pg = result_->pages[std::size_t(i)];
            auto* tex = new QSGSimpleTextureNode;
            tex->setTexture(window()->createTextureFromImage(pg.raster));
            tex->setOwnsTexture(true);
            tex->setFiltering(QSGTexture::Linear);
            tex->setRect(QRectF(0, tops[std::size_t(i)], pg.size.width(), pg.size.height()));
            pages->appendChildNode(tex);
        }
        texturesDirty_ = false;
    }

    // ---- overlay: cheap, rebuilt every update ------------------------------
    QSGNode* overlay = root->childAtIndex(OverlaySlot);
    while (QSGNode* c = overlay->firstChild()) { overlay->removeChildNode(c); delete c; }
    if (result_) {
        for (const auto& r : masks_)
            if (r.page < int(tops.size()))
                overlay->appendChildNode(new QSGSimpleRectNode(r.rect.translated(0, tops[std::size_t(r.page)]), paper_));
        if (!glyphs_.empty()) overlay->appendChildNode(makeGlyphNode(glyphs_, tops, QColor(20, 70, 160)));

        auto highlight = [&](ElementId id, QColor c) {
            if (!id.valid() || !result_->index) return;
            if (const auto* e = result_->index->find(id))
                overlay->appendChildNode(new QSGSimpleRectNode(
                    e->bounds.adjusted(-0.2, -0.2, 0.2, 0.2).translated(0, tops[std::size_t(e->page)]), c));
        };
        highlight(hover_, QColor(255, 170, 0, 70));
        highlight(selection_, QColor(0, 120, 255, 90));

        if (!std::isnan(playheadQ_) && result_->anchors) {
            const auto c = result_->anchors->cursorAt(playheadQ_);
            if (c.valid && c.page < int(tops.size()))
                overlay->appendChildNode(new QSGSimpleRectNode(
                    QRectF(c.x - 0.12, c.top + tops[std::size_t(c.page)], 0.24, c.bottom - c.top), QColor(220, 30, 30, 200)));
        }
    }
    return root;
}

}  // namespace corinzia::ui
