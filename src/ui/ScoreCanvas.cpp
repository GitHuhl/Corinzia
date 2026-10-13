#include "corinzia/ui/ScoreCanvas.hpp"

#include <QQuickWindow>
#include <QPainter>
#include <QSGSimpleRectNode>
#include <QSGSimpleTextureNode>
#include <QSGTransformNode>

#include <cmath>

namespace corinzia::ui {

namespace {
// Child slots of the root transform node, in paint order.
enum Slot { PagesSlot = 0, OverlaySlot = 1 };
// Overlay children, created once and never deleted (unused ones get an empty
// rect / zero vertices): the software adaptation does not reliably pick up
// nodes that are deleted and recreated between frames.
enum OverlayChild { MasksChild = 0, GlyphsChild, HoverChild, SelectionChild, PlayheadChild, CaretChild };

// Provisional glyph sprites. Drawn as textured quads rather than custom
// geometry so they render on every scene-graph backend, including the
// software adaptation (which ignores custom QSGGeometryNodes).
QImage glyphImage(bool rest, QColor color) {
    const QSize px = rest ? QSize(32, 80) : QSize(52, 38);   // 40 px per staff space
    QImage img(px, QImage::Format_ARGB32_Premultiplied);
    img.fill(Qt::transparent);
    QPainter g(&img);
    g.setRenderHint(QPainter::Antialiasing);
    g.setPen(Qt::NoPen);
    g.setBrush(color);
    if (rest) {
        g.drawRoundedRect(QRectF(2, 2, px.width() - 4, px.height() - 4), 6, 6);
    } else {
        g.translate(px.width() / 2.0, px.height() / 2.0);
        g.rotate(-20);
        g.drawEllipse(QPointF(0, 0), 24, 16);
    }
    return img;
}
constexpr QSizeF kHeadSize{1.3, 0.95}, kRestSize{0.8, 2.0};   // staff spaces
// Pools are allocated with the tree: the software adaptation only renders
// nodes present when its render list was first built for a subtree.
constexpr int kMaskPool = 32, kGlyphPool = 256;
// Sprites [0, kHeadSprites) always show a notehead, the rest always a rest:
// a node's material is fixed for life (the software adaptation does not pick
// up material changes on existing nodes, only geometry/rect changes).
constexpr int kHeadSprites = 192;

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

void ScoreCanvas::setCaret(std::size_t lane, double quarter) {
    caretLane_ = lane;
    caretQ_ = quarter;
    update();
}

void ScoreCanvas::setInputMode(bool on) {
    if (on == inputMode_) return;
    inputMode_ = on;
    emit inputModeChanged();
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
    forceActiveFocus();
    if (inputMode_ && e->button() == Qt::LeftButton) {
        emit inputClicked(pp.page, pp.pt, e->modifiers());
        e->accept();
        return;
    }
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
    // More pages than page nodes: rebuild the whole tree rather than inserting
    // into a live one (the software adaptation mis-transforms nodes inserted
    // into an already-rendered subtree; see ArrangeView).
    if (root && texturesDirty_ && result_ &&
        int(result_->pages.size()) > root->childAtIndex(PagesSlot)->childCount()) {
        delete root;
        root = nullptr;
    }
    if (!root) {
        root = new QSGTransformNode;
        root->appendChildNode(new QSGNode);   // PagesSlot
        auto* overlay = new QSGNode;          // OverlaySlot
        auto* maskPool = new QSGNode;                                           // MasksChild (pool)
        for (int i = 0; i < kMaskPool; ++i) maskPool->appendChildNode(new QSGSimpleRectNode(QRectF(), paper_));
        overlay->appendChildNode(maskPool);
        // GlyphsChild: [0] head template, [1] rest template (own the textures,
        // never drawn), [2..] sprite instances sharing those textures.
        auto* glyphs = new QSGNode;
        for (bool rest : {false, true}) {
            auto* tmpl = new QSGSimpleTextureNode;
            tmpl->setTexture(window()->createTextureFromImage(glyphImage(rest, QColor(20, 70, 160))));
            tmpl->setOwnsTexture(true);
            tmpl->setRect(QRectF());
            glyphs->appendChildNode(tmpl);
        }
        const QSGTexture* head = static_cast<QSGSimpleTextureNode*>(glyphs->childAtIndex(0))->texture();
        const QSGTexture* rest = static_cast<QSGSimpleTextureNode*>(glyphs->childAtIndex(1))->texture();
        for (int i = 0; i < kGlyphPool; ++i) {
            auto* sprite = new QSGSimpleTextureNode;
            sprite->setOwnsTexture(false);
            sprite->setTexture(const_cast<QSGTexture*>(i < kHeadSprites ? head : rest));   // never texture-less (software renderer derefs it)
            sprite->setFiltering(QSGTexture::Linear);
            sprite->setRect(QRectF());
            glyphs->appendChildNode(sprite);
        }
        overlay->appendChildNode(glyphs);
        overlay->appendChildNode(new QSGSimpleRectNode(QRectF(), QColor(255, 170, 0, 70)));   // HoverChild
        overlay->appendChildNode(new QSGSimpleRectNode(QRectF(), QColor(0, 120, 255, 90)));   // SelectionChild
        overlay->appendChildNode(new QSGSimpleRectNode(QRectF(), QColor(220, 30, 30, 200)));  // PlayheadChild
        overlay->appendChildNode(new QSGSimpleRectNode(QRectF(), QColor(0, 150, 70, 220)));   // CaretChild
        root->appendChildNode(overlay);
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
        // Existing page nodes are reused (texture swapped in place). New nodes
        // are fully configured *before* insertion: the software adaptation
        // visits a node as soon as it is appended and keeps what it sees.
        const int want = result_ ? int(result_->pages.size()) : 0;
        while (pages->childCount() > want) {
            QSGNode* c = pages->lastChild();
            pages->removeChildNode(c);
            delete c;
        }
        for (int i = 0; i < want; ++i) {
            const auto& pg = result_->pages[std::size_t(i)];
            QImage img = pg.raster.isNull() ? QImage(1, 1, QImage::Format_ARGB32_Premultiplied) : pg.raster;
            if (pg.raster.isNull()) img.fill(Qt::white);
            QSGTexture* fresh = window()->createTextureFromImage(img);
            const QRectF rect(0, tops[std::size_t(i)], pg.size.width(), pg.size.height());
            if (i < pages->childCount()) {
                auto* tex = static_cast<QSGSimpleTextureNode*>(pages->childAtIndex(i));
                QSGTexture* old = tex->texture();
                tex->setOwnsTexture(false);             // we delete the previous texture ourselves
                tex->setTexture(fresh);
                tex->setOwnsTexture(true);
                delete old;
                tex->setRect(rect);
            } else {
                auto* tex = new QSGSimpleTextureNode;
                tex->setTexture(fresh);
                tex->setOwnsTexture(true);
                tex->setFiltering(QSGTexture::Linear);
                tex->setRect(rect);
                pages->appendChildNode(tex);
            }
        }
        texturesDirty_ = false;
    }

    // ---- overlay: persistent nodes, updated in place every frame ------------
    QSGNode* overlay = root->childAtIndex(OverlaySlot);
    auto pageY = [&](int page) { return page < int(tops.size()) ? tops[std::size_t(page)] : 0.0; };

    QSGNode* masks = overlay->childAtIndex(MasksChild);
    const int maskCount = result_ ? std::min<int>(int(masks_.size()), masks->childCount()) : 0;
    for (int i = 0; i < masks->childCount(); ++i) {
        auto* rn = static_cast<QSGSimpleRectNode*>(masks->childAtIndex(i));
        rn->setColor(paper_);
        rn->setRect(i < maskCount ? masks_[std::size_t(i)].rect.translated(0, pageY(masks_[std::size_t(i)].page)) : QRectF());
    }

    QSGNode* glyphs = overlay->childAtIndex(GlyphsChild);
    int nextHead = 2, nextRest = 2 + kHeadSprites;          // child indices (0,1 are the templates)
    const int restEnd = glyphs->childCount();
    std::vector<bool> used(std::size_t(glyphs->childCount()), false);
    for (const auto& gl : result_ ? glyphs_ : std::vector<LayoutAnchors::Glyph>{}) {
        int& slot = gl.rest ? nextRest : nextHead;
        if (slot >= (gl.rest ? restEnd : 2 + kHeadSprites)) continue;   // pool exhausted: skip extra glyphs
        auto* sprite = static_cast<QSGSimpleTextureNode*>(glyphs->childAtIndex(slot));
        const QSizeF sz = gl.rest ? kRestSize : kHeadSize;
        sprite->setRect(QRectF(gl.center.x() - sz.width() / 2, gl.center.y() + pageY(gl.page) - sz.height() / 2,
                               sz.width(), sz.height()));
        used[std::size_t(slot++)] = true;
    }
    for (int i = 2; i < glyphs->childCount(); ++i)
        if (!used[std::size_t(i)]) static_cast<QSGSimpleTextureNode*>(glyphs->childAtIndex(i))->setRect(QRectF());

    auto highlight = [&](int child, ElementId id) {
        QRectF r;
        if (result_ && id.valid() && result_->index)
            if (const auto* e = result_->index->find(id)) r = e->bounds.adjusted(-0.2, -0.2, 0.2, 0.2).translated(0, pageY(e->page));
        static_cast<QSGSimpleRectNode*>(overlay->childAtIndex(child))->setRect(r);
    };
    highlight(HoverChild, hover_);
    highlight(SelectionChild, selection_);

    QRectF playhead;
    if (result_ && !std::isnan(playheadQ_) && result_->anchors) {
        const auto c = result_->anchors->cursorAt(playheadQ_);
        if (c.valid) playhead = QRectF(c.x - 0.12, c.top + pageY(c.page), 0.24, c.bottom - c.top);
    }
    static_cast<QSGSimpleRectNode*>(overlay->childAtIndex(PlayheadChild))->setRect(playhead);

    QRectF caret;
    if (result_ && inputMode_ && !std::isnan(caretQ_) && result_->anchors) {
        const auto c = result_->anchors->caretAt(caretLane_, caretQ_);
        if (c.valid) caret = QRectF(c.x - 0.9, c.top + pageY(c.page), 0.3, c.bottom - c.top);   // just left of the onset
    }
    static_cast<QSGSimpleRectNode*>(overlay->childAtIndex(CaretChild))->setRect(caret);
    return root;
}

}  // namespace corinzia::ui
