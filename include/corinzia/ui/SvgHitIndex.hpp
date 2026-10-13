#pragma once
// Spatial index over a definitive engraving: ElementId <-> page rectangle.
//
// Built on the engraving worker thread from the SVG pages LilyPond returns.
// Every <g id="e-…" class="ev … v-… t-…"> group emitted through
// output-attributes becomes an Entry; its bounds come from
// QSvgRenderer::boundsOnElement (SVG user units = staff spaces, viewBox space).
//
// Used for: point-and-click hit testing, highlight rectangles, the playhead's
// time->x anchors, and the provisional (instant-feedback) layout.

#include "corinzia/core/Types.hpp"

#include <QByteArray>
#include <QPointF>
#include <QRectF>
#include <QSizeF>

#include <unordered_map>
#include <vector>

namespace corinzia::ui {

class SvgHitIndex {
public:
    enum class Kind : std::uint8_t { Note, Head, Rest, Other };
    struct Entry {
        ElementId id;
        int page = 0;
        QRectF bounds;              // viewBox units of `page`
        std::uint64_t voice = 0;    // ElementId values decoded from class "v-…"
        std::uint64_t track = 0;    //                         and class "t-…"
        Kind kind = Kind::Other;
    };

    // Parses one SVG page. Thread-safe w.r.t. other instances (no shared state).
    void addPage(int page, const QByteArray& svg);

    const Entry* find(ElementId id) const;
    // Smallest entry containing `pt` (inflated by `tolerance`) on `page`.
    const Entry* hitTest(int page, QPointF pt, qreal tolerance = 0.25) const;

    const std::vector<Entry>& entries() const noexcept { return entries_; }
    int pageCount() const noexcept { return int(pageSizes_.size()); }
    QSizeF pageSize(int page) const { return page < pageCount() ? pageSizes_[std::size_t(page)] : QSizeF(); }

private:
    static constexpr qreal kCell = 8.0;    // grid cell edge in staff spaces
    using CellKey = std::uint64_t;
    static CellKey cellKey(int page, int cx, int cy) noexcept {
        return (std::uint64_t(std::uint16_t(page)) << 48) | (std::uint64_t(std::uint32_t(cx) & 0xffffff) << 24) |
               (std::uint64_t(std::uint32_t(cy) & 0xffffff));
    }

    std::vector<Entry> entries_;
    std::unordered_map<ElementId, std::size_t> byId_;
    std::unordered_map<CellKey, std::vector<std::size_t>> grid_;
    std::vector<QSizeF> pageSizes_;
};

}  // namespace corinzia::ui
