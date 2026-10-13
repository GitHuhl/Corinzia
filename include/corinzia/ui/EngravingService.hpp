#pragma once
// The asynchronous definitive pass.
//
//  UI thread                          worker (QThreadPool)                LilyPond
//  ─────────                          ────────────────────                ────────
//  requestEngrave(snapshot) ──┐
//    (restart 350 ms debounce)│
//  timer fires ───────────────┴──▶ emitLilyPond(snapshot)
//                                   QProcess lilypond -dbackend=svg ───▶ compile
//                                   poll: newer revision requested? ──▶ kill()
//                                   read page SVGs ◀─────────────────── *.svg
//                                   SvgHitIndex + LayoutAnchors
//                                   rasterize pages (QSvgRenderer→QImage)
//  engraved(result) ◀── queued ────┘
//  (dropped if result.revision < latest requested)
//
// One job in flight per movement; edits during a compile coalesce into exactly
// one follow-up job. Nothing here blocks the UI thread.

#include "corinzia/engrave/LilyPondEmitter.hpp"
#include "corinzia/model/Model.hpp"
#include "corinzia/ui/LayoutAnchors.hpp"
#include "corinzia/ui/SvgHitIndex.hpp"

#include <QByteArray>
#include <QHash>
#include <QImage>
#include <QObject>
#include <QString>
#include <QTimer>

#include <atomic>
#include <memory>

namespace corinzia::ui {

struct EngravedPage {
    QByteArray svg;
    QImage raster;          // pre-rendered on the worker at `rasterScale` px per staff space
    QSizeF size;            // viewBox size, staff spaces
};

struct EngraveResult {
    MovementId movement;
    std::uint64_t revision = 0;
    std::vector<EngravedPage> pages;
    std::shared_ptr<const SvgHitIndex> index;
    std::shared_ptr<const LayoutAnchors> anchors;
    std::shared_ptr<const model::Movement> source;   // what was engraved
    qreal rasterScale = 0;
    qint64 compileMs = 0;
    QString log;           // LilyPond stderr (warnings surface in the UI)
    bool ok = false;
};

class EngravingService : public QObject {
    Q_OBJECT
public:
    explicit EngravingService(QObject* parent = nullptr);
    ~EngravingService() override;

    void setLilyPondPath(QString path) { lilypond_ = std::move(path); }
    void setDebounceMs(int ms) { debounceMs_ = ms; }
    void setRasterScale(qreal pxPerStaffSpace) { rasterScale_ = pxPerStaffSpace; }

    // UI thread. Context must hold copies/pointers valid for the job's lifetime:
    // the service keeps the shared_ptr'd movement snapshot alive itself.
    void requestEngrave(const model::MovementSnapshot& snap, engrave::EngraveContext ctx);
    // Skip the debounce (e.g. on project open or explicit "Engrave now").
    void engraveNow(MovementId movement);

signals:
    void engraved(std::shared_ptr<const corinzia::ui::EngraveResult> result);
    void engraveFailed(corinzia::MovementId movement, QString log);

private:
    struct Slot {
        model::MovementSnapshot pending;
        engrave::EngraveContext ctx;
        std::shared_ptr<std::atomic<std::uint64_t>> latestRequested = std::make_shared<std::atomic<std::uint64_t>>(0);
        QTimer* timer = nullptr;
        bool inFlight = false;
        bool rerun = false;
    };

    void launch(Slot& slot);
    static std::shared_ptr<EngraveResult> runJob(QString lilypond, model::MovementSnapshot snap,
                                                 engrave::EngraveContext ctx, qreal scale,
                                                 std::shared_ptr<std::atomic<std::uint64_t>> latest);

    QHash<std::uint64_t, Slot*> slots_;
    QString lilypond_ = QStringLiteral("lilypond");
    int debounceMs_ = 350;
    qreal rasterScale_ = 12.0;
};

}  // namespace corinzia::ui

Q_DECLARE_METATYPE(std::shared_ptr<const corinzia::ui::EngraveResult>)
