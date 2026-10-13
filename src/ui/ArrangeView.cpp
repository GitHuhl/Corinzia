#include "corinzia/ui/ArrangeView.hpp"

#include <QSGSimpleRectNode>

#include <algorithm>
#include <cmath>

namespace corinzia::ui {

namespace {

enum Child { LanesChild = 0, BarsChild, BlocksChild, SelectionChild, PlayheadChild, ChildCount };

// Node strategy (verified by pixel tests under the software adaptation, which
// mishandles nodes inserted into an already-rendered tree): the tree has one
// pool per (layer, colour) with a power-of-two capacity, so a node's colour
// never changes. Per frame only rects change. When a pool would overflow or a
// new colour appears, the whole tree is rebuilt and returned as a new root,
// fully built before the scene graph sees it.
using Pool = ArrangeView::PoolLayout;

QSGNode* buildTree(const std::vector<std::vector<Pool>>& layers) {
    auto* root = new QSGNode;
    for (const auto& layer : layers) {
        auto* l = new QSGNode;
        for (const auto& p : layer) {
            auto* pool = new QSGNode;
            for (int i = 0; i < p.capacity; ++i) pool->appendChildNode(new QSGSimpleRectNode(QRectF(), QColor::fromRgba(p.color)));
            l->appendChildNode(pool);
        }
        root->appendChildNode(l);
    }
    return root;
}

constexpr double kRuler = 0.0;      // the QML ruler sits above the item
constexpr double kBlockH = 6.0;

}  // namespace

ArrangeView::ArrangeView(QQuickItem* parent) : QQuickItem(parent) {
    setFlag(ItemHasContents, true);
    setAcceptedMouseButtons(Qt::LeftButton);
    setClip(true);
}

void ArrangeView::setContent(ArrangeContent c) {
    content_ = std::move(c);
    emit contentChanged();
    update();
}

void ArrangeView::setSelection(ElementId id) {
    selection_ = id;
    update();
}

void ArrangeView::setPlayheadQuarter(double q) {
    playheadQ_ = q;
    update();
}

void ArrangeView::setPixelsPerQuarter(double v) {
    v = std::clamp(v, 4.0, 400.0);
    if (v == ppq_) return;
    ppq_ = v;
    emit viewChanged();
    update();
}

void ArrangeView::setScrollX(double v) {
    v = std::max(0.0, v);
    if (v == scrollX_) return;
    scrollX_ = v;
    emit viewChanged();
    update();
}

void ArrangeView::setLaneHeight(double v) {
    v = std::clamp(v, 24.0, 240.0);
    if (v == laneH_) return;
    laneH_ = v;
    emit viewChanged();
    update();
}

QRectF ArrangeView::blockRect(std::size_t i) const {
    const auto& b = content_.blocks[i];
    const auto& t = content_.tracks[std::size_t(b.track)];
    const double top = kRuler + b.track * laneH_;
    const int range = std::max(1, t.maxKey - t.minKey);
    const double usable = laneH_ - 2 * 6.0 - kBlockH;
    const double y = top + 6.0 + double(t.maxKey - b.key) / range * usable;
    return QRectF(xForQuarter(b.startQ), y, std::max(2.0, b.lengthQ * ppq_ - 1.0), kBlockH);
}

void ArrangeView::mousePressEvent(QMouseEvent* e) {
    forceActiveFocus();
    const QPointF p = e->position();
    for (std::size_t i = 0; i < content_.blocks.size(); ++i)
        if (blockRect(i).adjusted(-1, -2, 1, 2).contains(p)) {
            setSelection(content_.blocks[i].element);
            emit blockClicked(content_.blocks[i].element.value);
            e->accept();
            return;
        }
    emit timelineClicked(std::max(0.0, (p.x() + scrollX_) / ppq_));
    e->accept();
}

void ArrangeView::wheelEvent(QWheelEvent* e) {
    if (e->modifiers() & Qt::ControlModifier) {
        // Zoom around the mouse: keep the quarter under the cursor fixed.
        const double q = (e->position().x() + scrollX_) / ppq_;
        setPixelsPerQuarter(ppq_ * std::pow(1.0015, e->angleDelta().y()));
        setScrollX(q * ppq_ - e->position().x());
    } else {
        const int d = e->angleDelta().x() ? e->angleDelta().x() : e->angleDelta().y();
        setScrollX(scrollX_ - d * 0.5);
    }
    e->accept();
}

QSGNode* ArrangeView::updatePaintNode(QSGNode* old, UpdatePaintNodeData*) {
    const double w = width(), h = height();
    using Rects = std::vector<std::pair<QRectF, QColor>>;
    Rects layers[ChildCount];
    Rects& lanes = layers[LanesChild];
    Rects& bars = layers[BarsChild];
    Rects& blocks = layers[BlocksChild];
    Rects& sel = layers[SelectionChild];
    Rects& play = layers[PlayheadChild];

    for (std::size_t t = 0; t < content_.tracks.size(); ++t) {
        const QRectF lane(0, kRuler + t * laneH_, w, laneH_);
        lanes.push_back({lane, t % 2 ? QColor(238, 240, 244) : QColor(246, 247, 250)});
        lanes.push_back({QRectF(0, lane.bottom() - 1, w, 1), QColor(210, 214, 222)});
        lanes.push_back({QRectF(0, lane.top(), 4, laneH_), content_.tracks[t].color});   // track colour tab
    }
    const double lanesBottom = kRuler + content_.tracks.size() * laneH_;
    for (std::size_t i = 0; i < content_.barQuarters.size(); ++i) {
        const double x = xForQuarter(content_.barQuarters[i]);
        if (x < -1 || x > w + 1) continue;
        const bool last = i + 1 == content_.barQuarters.size();
        bars.push_back({QRectF(x, 0, last ? 2 : 1, std::min(h, lanesBottom)), last ? QColor(90, 90, 100) : QColor(190, 195, 205)});
    }
    for (std::size_t i = 0; i < content_.blocks.size(); ++i) {
        const QRectF r = blockRect(i);
        if (r.right() < 0 || r.left() > w) continue;   // culled
        blocks.push_back({r, content_.tracks[std::size_t(content_.blocks[i].track)].color});
        if (content_.blocks[i].element == selection_) sel.push_back({r.adjusted(-2, -2, 2, 2), QColor(0, 120, 255, 110)});
    }
    if (!std::isnan(playheadQ_))
        play.push_back({QRectF(xForQuarter(playheadQ_) - 1, 0, 2, std::min(h, lanesBottom)), QColor(220, 30, 30)});

    // Group by colour and check the current tree's capacity.
    std::vector<std::vector<std::pair<QRgb, std::vector<QRectF>>>> grouped(ChildCount);
    bool rebuild = !old || layout_.size() != ChildCount;
    for (int l = 0; l < ChildCount; ++l) {
        for (const auto& [rect, color] : layers[l]) {
            auto& g = grouped[std::size_t(l)];
            auto it = std::find_if(g.begin(), g.end(), [&](const auto& e) { return e.first == color.rgba(); });
            if (it == g.end()) {
                g.push_back({color.rgba(), {}});
                it = g.end() - 1;
            }
            it->second.push_back(rect);
        }
        if (rebuild) continue;
        for (const auto& [rgb, rects] : grouped[std::size_t(l)]) {
            auto it = std::find_if(layout_[std::size_t(l)].begin(), layout_[std::size_t(l)].end(),
                                   [&](const Pool& p) { return p.color == rgb; });
            if (it == layout_[std::size_t(l)].end() || it->capacity < int(rects.size())) rebuild = true;
        }
    }
    QSGNode* root = old;
    if (rebuild) {
        // Keep existing pools (with at least their old capacity) so the tree
        // only grows; double capacity for anything that overflowed.
        std::vector<std::vector<Pool>> next(ChildCount);
        for (int l = 0; l < ChildCount; ++l) {
            if (std::size_t(l) < layout_.size()) next[std::size_t(l)] = layout_[std::size_t(l)];
            for (const auto& [rgb, rects] : grouped[std::size_t(l)]) {
                auto it = std::find_if(next[std::size_t(l)].begin(), next[std::size_t(l)].end(),
                                       [&](const Pool& p) { return p.color == rgb; });
                int cap = 8;
                while (cap < int(rects.size())) cap *= 2;
                if (it == next[std::size_t(l)].end()) next[std::size_t(l)].push_back({rgb, cap});
                else it->capacity = std::max(it->capacity, cap);
            }
        }
        layout_ = std::move(next);
        delete old;
        root = buildTree(layout_);
    }
    // Per frame: set rects; unused nodes get an empty rect.
    for (int l = 0; l < ChildCount; ++l) {
        QSGNode* layerNode = root->childAtIndex(l);
        for (std::size_t p = 0; p < layout_[std::size_t(l)].size(); ++p) {
            QSGNode* pool = layerNode->childAtIndex(int(p));
            const std::vector<QRectF>* want = nullptr;
            for (const auto& [rgb, rects] : grouped[std::size_t(l)])
                if (rgb == layout_[std::size_t(l)][p].color) want = &rects;
            for (int i = 0; i < pool->childCount(); ++i)
                static_cast<QSGSimpleRectNode*>(pool->childAtIndex(i))
                    ->setRect(want && i < int(want->size()) ? (*want)[std::size_t(i)] : QRectF());
        }
    }
    return root;
}

}  // namespace corinzia::ui
