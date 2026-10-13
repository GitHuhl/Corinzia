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
enum Slot { PagesSlot = 0, TilesSlot = 1, OverlaySlot = 2 };
// Overlay children, created once and never deleted (unused ones get an empty
// rect / zero vertices): the software adaptation does not reliably pick up
// nodes that are deleted and recreated between frames.
enum OverlayChild { MasksChild = 0, GlyphsChild, HoverChild, SelectionChild, PlayheadChild, CaretChild, RangeChild,
                    MarqueeChild };
constexpr int kRangePool = 24;   // one highlight rect per system the range touches

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

    tiles_ = new TileRenderer;
    tiles_->moveToThread(&tileThread_);
    connect(&tileThread_, &QThread::finished, tiles_, &QObject::deleteLater);
    connect(tiles_, &TileRenderer::tileReady, this, &ScoreCanvas::onTileReady);   // queued back to the GUI thread
    tileThread_.setObjectName(QStringLiteral("corinzia-tiles"));
    tileThread_.start(QThread::LowPriority);
    tileTimer_.setSingleShot(true);
    connect(&tileTimer_, &QTimer::timeout, this, &ScoreCanvas::requestTiles);
}

ScoreCanvas::~ScoreCanvas() {
    tiles_->latestLevel = -1;   // let queued requests drain as no-ops
    tileThread_.quit();
    tileThread_.wait();
}

qreal ScoreCanvas::devicePixelRatio() const { return window() ? window()->effectiveDevicePixelRatio() : 1.0; }

std::vector<TileKey> ScoreCanvas::visibleTiles() const {
    std::vector<TileKey> out;
    if (!result_ || currentLevel_ <= 0) return out;
    const qreal dpr = devicePixelRatio();
    const double s = levelScale(currentLevel_);                    // device px per staff space
    const QRectF view(0, 0, width(), height());
    for (int i = 0; i < int(result_->pages.size()); ++i) {
        const QRectF page = pageRectInItem(i);
        const QRectF vis = page.intersected(view);
        if (vis.isEmpty()) continue;
        // Visible part of the page in page-device-pixel coordinates.
        const double x0 = (vis.left() - page.left()) * dpr, x1 = (vis.right() - page.left()) * dpr;
        const double y0 = (vis.top() - page.top()) * dpr, y1 = (vis.bottom() - page.top()) * dpr;
        const int maxTx = int(std::ceil(result_->pages[std::size_t(i)].size.width() * s / kTileSize));
        const int maxTy = int(std::ceil(result_->pages[std::size_t(i)].size.height() * s / kTileSize));
        for (int ty = std::max(0, int(y0 / kTileSize)); ty <= std::min(maxTy - 1, int((y1 - 1) / kTileSize)); ++ty)
            for (int tx = std::max(0, int(x0 / kTileSize)); tx <= std::min(maxTx - 1, int((x1 - 1) / kTileSize)); ++tx)
                out.push_back({engravingSerial_, i, currentLevel_, tx, ty});
    }
    if (out.size() > 96) out.resize(96);   // absurd zoom-out on huge scores: base pages cover the rest
    return out;
}

void ScoreCanvas::requestTiles() {
    if (!result_) return;
    tiles_->latestLevel = currentLevel_;
    bool missing = false;
    for (const TileKey& k : visibleTiles()) {
        if (tileCache_.contains(k)) continue;
        missing = true;
        if (requested_.contains(k)) continue;
        requested_.insert(k);
        QMetaObject::invokeMethod(tiles_, [t = tiles_, r = result_, serial = engravingSerial_, k] { t->render(r, serial, k); },
                                  Qt::QueuedConnection);
    }
    if (!missing) emit tilesReady();
    update();
}

void ScoreCanvas::onTileReady(TileKey key, QImage image) {
    requested_.remove(key);
    if (key.engraving != engravingSerial_) return;   // engraving replaced meanwhile
    tileCache_.insert(key, std::move(image));
    // Bound memory: drop tiles of other zoom levels first.
    if (tileCache_.size() > 160)
        for (auto it = tileCache_.begin(); it != tileCache_.end();)
            it = (it.key().level != currentLevel_) ? tileCache_.erase(it) : std::next(it);
    if (tilesSettled()) emit tilesReady();
    update();
}

bool ScoreCanvas::tilesSettled() const {
    const auto vis = visibleTiles();
    return !vis.empty() && std::all_of(vis.begin(), vis.end(), [&](const TileKey& k) { return tileCache_.contains(k); });
}

void ScoreCanvas::geometryChange(const QRectF& n, const QRectF& o) {
    QQuickItem::geometryChange(n, o);
    if (n.size() != o.size()) tileTimer_.start(0);
}

void ScoreCanvas::setZoom(qreal z) {
    z = std::clamp(z, 0.1, 16.0);
    if (qFuzzyCompare(z, zoom_)) return;
    zoom_ = z;
    currentLevel_ = scaleLevel(pageScale() * devicePixelRatio());
    tiles_->latestLevel = currentLevel_;
    emit zoomChanged();
    tileTimer_.start(90);   // crisp vector tiles once the zoom settles
    update();               // meanwhile the GPU rescales what we have: instant feedback
}

void ScoreCanvas::setPan(QPointF p) {
    if (p == pan_) return;
    pan_ = p;
    emit panChanged();
    if (!tileTimer_.isActive()) tileTimer_.start(0);   // same zoom: newly exposed tiles right away
    update();
}

void ScoreCanvas::setEngraving(std::shared_ptr<const EngraveResult> result) {
    if (result_ && result && result->movement == result_->movement && result->revision < result_->revision) return;
    result_ = std::move(result);
    glyphs_.clear();   // the definitive layer now reflects the edit
    masks_.clear();
    texturesDirty_ = true;
    ++engravingSerial_;
    tileCache_.clear();
    requested_.clear();
    currentLevel_ = scaleLevel(pageScale() * devicePixelRatio());
    requestTiles();
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

void ScoreCanvas::setRange(std::size_t laneFrom, std::size_t laneTo, double fromQ, double toQ) {
    rangeActive_ = true;
    rangeLaneFrom_ = std::min(laneFrom, laneTo);
    rangeLaneTo_ = std::max(laneFrom, laneTo);
    rangeFromQ_ = fromQ;
    rangeToQ_ = toQ;
    update();
}

void ScoreCanvas::clearRange() {
    rangeActive_ = false;
    update();
}

void ScoreCanvas::mouseMoveEvent(QMouseEvent* e) {
    if (!marqueeActive_) return QQuickItem::mouseMoveEvent(e);
    marqueeEnd_ = e->position();
    update();
    e->accept();
}

void ScoreCanvas::mouseReleaseEvent(QMouseEvent* e) {
    if (!marqueeActive_) return QQuickItem::mouseReleaseEvent(e);
    marqueeActive_ = false;
    setKeepMouseGrab(false);
    const QRectF band = QRectF(marqueeStart_, e->position()).normalized();
    update();
    e->accept();
    if (band.width() < 4 && band.height() < 4) {   // a plain click on empty paper
        const PagePoint pp = toPage(e->position());
        emit emptyClicked(pp.page, pp.pt);
        return;
    }
    // Every engraved element whose bounds touch the band, on every page it covers.
    QVariantList ids;
    if (result_ && result_->index) {
        const qreal s = pageScale();
        for (const auto& entry : result_->index->entries()) {
            const QRectF pageItem = pageRectInItem(entry.page);
            const QRectF bounds(pageItem.topLeft() + entry.bounds.topLeft() * s, entry.bounds.size() * s);
            if (bounds.intersects(band)) ids.push_back(QVariant::fromValue<quint64>(entry.id.value));
        }
    }
    emit marqueeSelected(ids, e->modifiers());
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
    if (!result_ || !result_->index) return QQuickItem::mousePressEvent(e);
    forceActiveFocus();
    const PagePoint pp = toPage(e->position());
    if (inputMode_ && e->button() == Qt::LeftButton && pp.page >= 0) {
        emit inputClicked(pp.page, pp.pt, e->modifiers());
        e->accept();
        return;
    }
    if (pp.page >= 0)
        if (const auto* hit = result_->index->hitTest(pp.page, pp.pt)) {
            setSelection(hit->id);
            emit elementClicked(hit->id.value, hit->voice, hit->track, e->modifiers());
            e->accept();
            return;
        }
    // Empty paper (or the desk around it): start a marquee; a release without
    // dragging counts as a click on empty space (clears the selection).
    if (e->button() == Qt::LeftButton) {
        marqueeActive_ = true;
        marqueeStart_ = marqueeEnd_ = e->position();
        setKeepMouseGrab(true);
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
        // Panorama is one endless system: the wheel scrolls through time.
        const QPoint d = e->angleDelta();
        if (panorama_ && d.x() == 0) setPan(pan_ + QPointF(d.y(), 0) * 0.5);
        else setPan(pan_ + QPointF(d.x(), d.y()) * 0.5);
    }
    e->accept();
}

QSGNode* ScoreCanvas::updatePaintNode(QSGNode* old, UpdatePaintNodeData*) {
    auto* root = static_cast<QSGTransformNode*>(old);
    // Vector tiles ready for the current zoom (GUI thread is blocked here).
    std::vector<TileKey> readyTiles;
    for (const TileKey& k : visibleTiles())
        if (tileCache_.contains(k)) readyTiles.push_back(k);

    // More pages / tiles than pooled nodes: rebuild the whole tree rather than
    // inserting into a live one (the software adaptation mis-transforms nodes
    // inserted into an already-rendered subtree; see ArrangeView).
    if (root && ((texturesDirty_ && result_ && int(result_->pages.size()) > root->childAtIndex(PagesSlot)->childCount()) ||
                 int(readyTiles.size()) > root->childAtIndex(TilesSlot)->childCount())) {
        delete root;
        root = nullptr;
    }
    if (!root) {
        root = new QSGTransformNode;
        root->appendChildNode(new QSGNode);   // PagesSlot
        auto* tilePool = new QSGNode;         // TilesSlot: crisp vector tiles over the base page images
        int cap = 16;
        while (cap < int(readyTiles.size())) cap *= 2;
        QImage blank(1, 1, QImage::Format_ARGB32_Premultiplied);
        blank.fill(Qt::white);
        for (int i = 0; i < cap; ++i) {
            auto* t = new QSGSimpleTextureNode;
            t->setTexture(window()->createTextureFromImage(blank));   // never texture-less
            t->setOwnsTexture(true);
            t->setFiltering(QSGTexture::Linear);
            t->setRect(QRectF());
            tilePool->appendChildNode(t);
        }
        nodeKeys_.assign(std::size_t(cap), TileKey{});
        root->appendChildNode(tilePool);
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
        auto* rangePool = new QSGNode;                                                        // RangeChild
        for (int i = 0; i < kRangePool; ++i) rangePool->appendChildNode(new QSGSimpleRectNode(QRectF(), QColor(0, 110, 230, 45)));
        overlay->appendChildNode(rangePool);
        auto* marquee = new QSGNode;                                                          // MarqueeChild: fill + 4 edges
        marquee->appendChildNode(new QSGSimpleRectNode(QRectF(), QColor(0, 110, 230, 30)));
        for (int i = 0; i < 4; ++i) marquee->appendChildNode(new QSGSimpleRectNode(QRectF(), QColor(0, 90, 200, 200)));
        overlay->appendChildNode(marquee);
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

    // ---- vector tiles: exact device resolution, rendered from the SVG ---------
    QSGNode* tilePool = root->childAtIndex(TilesSlot);
    const double s = levelScale(currentLevel_);
    for (int i = 0; i < tilePool->childCount(); ++i) {
        auto* node = static_cast<QSGSimpleTextureNode*>(tilePool->childAtIndex(i));
        if (i >= int(readyTiles.size())) {
            node->setRect(QRectF());
            continue;
        }
        const TileKey& k = readyTiles[std::size_t(i)];
        if (!(nodeKeys_[std::size_t(i)] == k)) {   // texture changes only when the node shows another tile
            QSGTexture* old = node->texture();
            node->setOwnsTexture(false);
            node->setTexture(window()->createTextureFromImage(tileCache_.value(k)));
            node->setOwnsTexture(true);
            delete old;
            nodeKeys_[std::size_t(i)] = k;
        }
        // Tile extent in staff spaces; the root transform maps it to exactly
        // kTileSize device pixels (s = pageScale * dpr up to level rounding).
        // Clip to the page (edge tiles would otherwise paint white beyond it)
        // and show the matching part of the texture.
        const double side = kTileSize / s;
        const QSizeF pg = result_->pages[std::size_t(k.page)].size;
        const QRectF tile(k.tx * side, k.ty * side, side, side);
        const QRectF vis = tile.intersected(QRectF(QPointF(0, 0), pg));
        node->setRect(vis.translated(0, tops[std::size_t(k.page)]));
        node->setSourceRect(QRectF((vis.left() - tile.left()) * s, (vis.top() - tile.top()) * s, vis.width() * s,
                                   vis.height() * s));
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

    // Range highlight: per-system rectangles from the engraved geometry.
    QSGNode* rangePool = overlay->childAtIndex(RangeChild);
    std::vector<LayoutAnchors::Region> regions;
    if (result_ && rangeActive_ && result_->anchors)
        regions = result_->anchors->rangeRects(rangeLaneFrom_, rangeLaneTo_, rangeFromQ_, rangeToQ_);
    for (int i = 0; i < rangePool->childCount(); ++i)
        static_cast<QSGSimpleRectNode*>(rangePool->childAtIndex(i))
            ->setRect(i < int(regions.size()) ? regions[std::size_t(i)].rect.translated(0, pageY(regions[std::size_t(i)].page))
                                              : QRectF());

    // Marquee: item-space drag rectangle mapped into the root's staff space.
    QSGNode* marquee = overlay->childAtIndex(MarqueeChild);
    QRectF band;
    if (marqueeActive_) {
        const QRectF item = QRectF(marqueeStart_, marqueeEnd_).normalized();
        band = QRectF((item.topLeft() - pan_) / pageScale(), item.size() / pageScale());
    }
    const double edge = 1.0 / pageScale();   // one device-independent pixel
    static_cast<QSGSimpleRectNode*>(marquee->childAtIndex(0))->setRect(band);
    static_cast<QSGSimpleRectNode*>(marquee->childAtIndex(1))->setRect(band.isEmpty() ? QRectF() : QRectF(band.left(), band.top(), band.width(), edge));
    static_cast<QSGSimpleRectNode*>(marquee->childAtIndex(2))->setRect(band.isEmpty() ? QRectF() : QRectF(band.left(), band.bottom() - edge, band.width(), edge));
    static_cast<QSGSimpleRectNode*>(marquee->childAtIndex(3))->setRect(band.isEmpty() ? QRectF() : QRectF(band.left(), band.top(), edge, band.height()));
    static_cast<QSGSimpleRectNode*>(marquee->childAtIndex(4))->setRect(band.isEmpty() ? QRectF() : QRectF(band.right() - edge, band.top(), edge, band.height()));
    return root;
}

}  // namespace corinzia::ui
