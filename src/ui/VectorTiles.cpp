#include "corinzia/ui/VectorTiles.hpp"

#include <QPainter>
#include <QRegularExpression>
#include <QSvgRenderer>

namespace corinzia::ui {

QImage renderTile(QSvgRenderer& page, QSizeF pageSize, const TileKey& key) {
    const double s = levelScale(key.level);
    QImage img(kTileSize, kTileSize, QImage::Format_ARGB32_Premultiplied);
    img.fill(Qt::white);
    QPainter p(&img);
    p.setRenderHint(QPainter::Antialiasing);
    p.setRenderHint(QPainter::SmoothPixmapTransform);
    p.translate(-double(key.tx) * kTileSize, -double(key.ty) * kTileSize);
    p.scale(s, s);
    // Map the SVG viewBox onto the page rectangle in staff spaces; the
    // painter's scale turns that into device pixels: pure vector rasterization.
    p.setClipRect(QRectF(key.tx * kTileSize / s, key.ty * kTileSize / s, kTileSize / s, kTileSize / s));
    page.render(&p, QRectF(QPointF(0, 0), pageSize));
    return img;
}

QByteArray normalizeSvgText(const QByteArray& svg) {
    static const QRegularExpression re(QStringLiteral("<text ([^>]*?)font-size=\"([0-9.]+)\""));
    QString s = QString::fromUtf8(svg);
    QString out;
    out.reserve(s.size() + 256);
    qsizetype last = 0;
    for (auto it = re.globalMatch(s); it.hasNext();) {
        const auto m = it.next();
        if (m.captured(1).contains(QStringLiteral("transform="))) continue;   // leave anything unusual alone
        out += QStringView(s).mid(last, m.capturedStart() - last);
        const double size = m.captured(2).toDouble() * 100.0;
        out += QStringLiteral("<text transform=\"scale(0.01)\" %1font-size=\"%2\"").arg(m.captured(1)).arg(size, 0, 'f', 2);
        last = m.capturedEnd();
    }
    out += QStringView(s).mid(last);
    // LilyPond places text runs like "Allegro" + " (" + "♩" + " = 132)" and
    // relies on a run's leading spaces, which Qt SVG strips (it simplifies
    // text; xml:space="preserve" changes its font fallback). Each run is
    // positioned absolutely, so only leading spaces matter: drop them and move
    // the run by their advance (0.278 em in LilyPond's serif text fonts).
    static const QRegularExpression lead(
        QStringLiteral("(<text [^>]*font-size=\"([0-9.]+)\"[^>]*)>(\\s*)<tspan>( +)"));
    QString fixed;
    fixed.reserve(out.size());
    last = 0;
    for (auto it = lead.globalMatch(out); it.hasNext();) {
        const auto m = it.next();
        if (m.captured(1).contains(QStringLiteral(" x="))) continue;
        fixed += QStringView(out).mid(last, m.capturedStart() - last);
        const double dx = double(m.capturedLength(4)) * 0.278 * m.captured(2).toDouble();
        fixed += QStringLiteral("%1 x=\"%2\">%3<tspan>").arg(m.captured(1)).arg(dx, 0, 'f', 2).arg(m.captured(3));
        last = m.capturedEnd();
    }
    fixed += QStringView(out).mid(last);
    return fixed.toUtf8();
}

TileRenderer::TileRenderer() { qRegisterMetaType<TileKey>(); }
TileRenderer::~TileRenderer() = default;

void TileRenderer::render(std::shared_ptr<const EngraveResult> result, std::uint64_t serial, TileKey key) {
    if (key.level != latestLevel.load(std::memory_order_relaxed)) return;   // zoom moved on: skip
    if (!result || key.page >= int(result->pages.size())) return;
    if (serial != serial_) {   // new engraving: parse its pages once, on this thread
        pages_.clear();
        for (const auto& pg : result->pages) pages_.push_back(std::make_unique<QSvgRenderer>(pg.svg));
        serial_ = serial;
    }
    emit tileReady(key, renderTile(*pages_[std::size_t(key.page)], result->pages[std::size_t(key.page)].size, key));
}

}  // namespace corinzia::ui
