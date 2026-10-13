#pragma once
// Resolution-independent page rendering for ScoreCanvas.
//
// The engraving stays vector (LilyPond SVG). For the visible part of each page
// a worker thread rasterizes 512x512 device-pixel tiles directly from the
// vectors at the *exact* current device scale, so notation is pixel-exact at
// any zoom. During a zoom gesture the canvas shows the previous texture scaled
// (instant feedback); crisp tiles replace it as soon as they are rendered.
//
// Threading: TileRenderer lives in its own QThread and owns one QSvgRenderer
// per page of the current engraving (parsed once, reused for every tile).
// Requests carry a level (scale) stamp; the worker skips requests whose level
// is no longer the latest, so fast zooming never builds a backlog.

#include "corinzia/ui/EngravingService.hpp"

#include <QHash>
#include <QImage>
#include <QObject>
#include <QThread>

#include <atomic>
#include <memory>

class QSvgRenderer;

namespace corinzia::ui {

struct TileKey {
    std::uint64_t engraving = 0;   // EngraveResult serial
    int page = 0;
    int level = 0;                 // device px per staff space * 64, rounded
    int tx = 0, ty = 0;            // tile column/row in page device pixels
    friend bool operator==(const TileKey&, const TileKey&) = default;
};
inline size_t qHash(const TileKey& k, size_t seed = 0) noexcept {
    return qHashMulti(seed, k.engraving, k.page, k.level, k.tx, k.ty);
}

inline constexpr int kTileSize = 512;
inline double levelScale(int level) noexcept { return level / 64.0; }
inline int scaleLevel(double devicePxPerSpace) noexcept { return int(devicePxPerSpace * 64.0 + 0.5); }

class TileRenderer : public QObject {
    Q_OBJECT
public:
    TileRenderer();
    ~TileRenderer() override;
    std::atomic<int> latestLevel{0};

public slots:
    void render(std::shared_ptr<const corinzia::ui::EngraveResult> result, std::uint64_t serial,
                corinzia::ui::TileKey key);

signals:
    void tileReady(corinzia::ui::TileKey key, QImage image);

private:
    std::uint64_t serial_ = 0;
    std::vector<std::unique_ptr<QSvgRenderer>> pages_;
};

// LilyPond writes text with tiny font sizes in staff units (font-size="2.2")
// that the painter then scales up; FreeType fails to rasterize glyphs at such
// nominal sizes under large transforms (text silently disappears when zoomed
// in). Rewrite each <text> to a 100x font with a scale(0.01) transform: same
// geometry, sane glyph sizes. Applied once when an engraving is read.
QByteArray normalizeSvgText(const QByteArray& svg);

// Renders one tile synchronously (also used by tests as the ground truth).
QImage renderTile(QSvgRenderer& page, QSizeF pageSizeSpaces, const TileKey& key);

}  // namespace corinzia::ui

Q_DECLARE_METATYPE(corinzia::ui::TileKey)
