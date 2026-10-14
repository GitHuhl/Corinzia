#include "corinzia/ui/EngravingService.hpp"

#include "corinzia/export/Export.hpp"

#include "corinzia/ui/VectorTiles.hpp"

#include <QDir>
#include <QElapsedTimer>
#include <QFile>
#include <QMetaObject>
#include <QPainter>
#include <QPointer>
#include <QProcess>
#include <QSvgRenderer>
#include <QTemporaryDir>
#include <QThreadPool>

namespace corinzia::ui {

EngravingService::EngravingService(QObject* parent) : QObject(parent) {
    qRegisterMetaType<std::shared_ptr<const EngraveResult>>();
    qRegisterMetaType<corinzia::MovementId>();
}

EngravingService::~EngravingService() {
    // Outstanding jobs see a bumped revision and abort; the pool is waited on so
    // no job can post back into a destroyed object.
    for (Slot* s : std::as_const(slots_)) s->latestRequested->store(~std::uint64_t{0});
    QThreadPool::globalInstance()->waitForDone();
    qDeleteAll(slots_);
}

void EngravingService::requestEngrave(const model::MovementSnapshot& snap, engrave::EngraveContext ctx) {
    if (!snap.movement) return;
    const auto key = snap.movement->id.id.value;
    Slot*& slot = slots_[key];
    if (!slot) {
        slot = new Slot;
        slot->timer = new QTimer(this);
        slot->timer->setSingleShot(true);
        connect(slot->timer, &QTimer::timeout, this, [this, key] { launch(*slots_[key]); });
    }
    slot->pending = snap;
    slot->ctx = std::move(ctx);
    slot->latestRequested->store(snap.revision);   // running job notices it is stale
    slot->timer->start(debounceMs_);                // restart: debounce while the user keeps editing
}

void EngravingService::engraveNow(MovementId movement) {
    if (Slot* s = slots_.value(movement.id.value)) {
        s->timer->stop();
        launch(*s);
    }
}

void EngravingService::launch(Slot& slot) {
    if (slot.inFlight) {           // coalesce: exactly one follow-up after the running job
        slot.rerun = true;
        return;
    }
    slot.inFlight = true;
    QPointer<EngravingService> self(this);
    const auto key = slot.pending.movement->id.id.value;
    QThreadPool::globalInstance()->start([self, key, lily = lilypond_, snap = slot.pending, ctx = slot.ctx,
                                          scale = rasterScale_, latest = slot.latestRequested] {
        auto result = runJob(lily, snap, ctx, scale, latest);
        QMetaObject::invokeMethod(
            self.data(),
            [self, key, result] {
                if (!self) return;
                Slot* s = self->slots_.value(key);
                if (!s) return;
                s->inFlight = false;
                const bool stale = !result || result->revision < s->latestRequested->load();
                if (result && result->ok && !stale) emit self->engraved(result);
                else if (result && !result->ok && !stale) emit self->engraveFailed(result->movement, result->log);
                if (s->rerun || stale) {
                    s->rerun = false;
                    if (!s->timer->isActive()) self->launch(*s);   // otherwise the debounce will launch it
                }
            },
            Qt::QueuedConnection);
    });
}

namespace {

// What the native engraver needs to know about each staff, from the context.
exporter::PartResolver contextResolver(const engrave::EngraveContext& ctx) {
    return [&ctx](const model::PlayerLane& lane) {
        exporter::PartInfo info;
        const model::Instrument* first = nullptr;
        if (!lane.changes.empty())
            if (auto it = ctx.instruments.find(lane.changes.front().instrument); it != ctx.instruments.end()) first = it->second;
        const auto pn = ctx.playerNames.find(lane.player);
        info.name = first ? first->name : pn != ctx.playerNames.end() ? pn->second : "";
        info.abbreviation = first ? first->engraverName : "";
        info.clef = first ? first->clef : "treble";
        for (std::size_t ci = 1; ci < lane.changes.size(); ++ci) {
            auto it = ctx.instruments.find(lane.changes[ci].instrument);
            info.changeNames.push_back(it != ctx.instruments.end() ? it->second->name : "");
        }
        return info;
    };
}

enum class LilyOutcome { Ok, Failed, Cancelled };

// Runs LilyPond (SVG backend) on the movement; a newer request cancels it.
LilyOutcome runLilyPond(const QString& lilypond, const model::MovementSnapshot& snap, const engrave::EngraveContext& ctx,
                        const std::shared_ptr<std::atomic<std::uint64_t>>& latest, EngraveResult& r,
                        std::vector<QByteArray>& out) {
    const QByteArray source = QByteArray::fromStdString(engrave::emitLilyPond(*snap.movement, ctx));
    QTemporaryDir dir;
    if (!dir.isValid()) { r.log = QStringLiteral("cannot create temp dir"); return LilyOutcome::Failed; }
    const QString base = dir.filePath(QStringLiteral("movement"));

    QProcess proc;
    proc.setProgram(lilypond);
    proc.setArguments({QStringLiteral("-dbackend=svg"), QStringLiteral("-dno-point-and-click"), QStringLiteral("-s"),
                       QStringLiteral("-o"), base, QStringLiteral("-")});
    proc.start();
    if (!proc.waitForStarted(5000)) { r.log = QStringLiteral("cannot start ") + lilypond; return LilyOutcome::Failed; }
    proc.write(source);
    proc.closeWriteChannel();
    // Poll so a newer edit can cancel this compile within ~30 ms.
    while (!proc.waitForFinished(30)) {
        if (latest->load() > snap.revision) {
            proc.kill();
            proc.waitForFinished(1000);
            return LilyOutcome::Cancelled;
        }
        if (proc.state() == QProcess::NotRunning) break;
    }
    r.log = QString::fromUtf8(proc.readAllStandardError());
    if (proc.exitStatus() != QProcess::NormalExit || proc.exitCode() != 0) return LilyOutcome::Failed;

    // Single page -> movement.svg; multi-page -> movement-1.svg, movement-2.svg, ...
    QStringList files;
    if (QFile::exists(base + QStringLiteral(".svg"))) files << base + QStringLiteral(".svg");
    for (int i = 1; QFile::exists(base + QStringLiteral("-%1.svg").arg(i)); ++i) files << base + QStringLiteral("-%1.svg").arg(i);
    for (const QString& file : files) {
        QFile f(file);
        if (!f.open(QIODevice::ReadOnly)) return LilyOutcome::Failed;
        out.push_back(f.readAll());
    }
    return LilyOutcome::Ok;
}

}  // namespace

std::shared_ptr<EngraveResult> EngravingService::runJob(QString lilypond, model::MovementSnapshot snap,
                                                        engrave::EngraveContext ctx, qreal scale,
                                                        std::shared_ptr<std::atomic<std::uint64_t>> latest) {
    auto r = std::make_shared<EngraveResult>();
    r->movement = snap.movement->id;
    r->revision = snap.revision;
    r->source = snap.movement;
    r->rasterScale = scale;
    QElapsedTimer clock;
    clock.start();

    std::vector<QByteArray> pageSvgs;
    if (ctx.nativeEngraver) {
        // Native engraver (Rust): milliseconds, in this thread.
        std::string err;
        const auto svgs = exporter::engraveNativeSvg(*snap.movement, contextResolver(ctx), !ctx.panorama, err);
        if (svgs.empty()) {
            r->log = QString::fromStdString(err);
            return r;
        }
        for (const auto& svg : svgs) pageSvgs.push_back(QByteArray::fromStdString(svg));
    } else {
        switch (runLilyPond(lilypond, snap, ctx, latest, *r, pageSvgs)) {
            case LilyOutcome::Cancelled: return nullptr;
            case LilyOutcome::Failed: return r;
            case LilyOutcome::Ok: break;
        }
    }

    auto index = std::make_shared<SvgHitIndex>();
    for (int page = 0; page < int(pageSvgs.size()); ++page) {
        if (latest->load() > snap.revision) return nullptr;
        EngravedPage pg;
        pg.svg = normalizeSvgText(pageSvgs[std::size_t(page)]);   // keep text renderable at every zoom
        index->addPage(page, pg.svg);
        QSvgRenderer renderer(pg.svg);
        pg.size = renderer.viewBoxF().size();
        // Base image only (vector tiles provide the sharp view): cap its size so
        // a panorama page hundreds of bars wide stays a valid texture.
        const qreal pageScale = std::min<qreal>(scale, 8192.0 / std::max(pg.size.width(), pg.size.height()));
        pg.raster = QImage((pg.size * pageScale).toSize(), QImage::Format_ARGB32_Premultiplied);
        pg.raster.fill(Qt::white);
        QPainter painter(&pg.raster);
        painter.setRenderHint(QPainter::Antialiasing);
        renderer.render(&painter);
        painter.end();
        r->pages.push_back(std::move(pg));
    }
    auto anchors = std::make_shared<LayoutAnchors>();
    std::vector<std::string> clefs;   // each lane's starting clef, from its first instrument
    for (const auto& lane : snap.movement->lanes) {
        auto it = lane.changes.empty() ? ctx.instruments.end() : ctx.instruments.find(lane.changes.front().instrument);
        clefs.push_back(it != ctx.instruments.end() ? it->second->clef : "treble");
    }
    anchors->build(*index, *snap.movement, std::move(clefs));
    r->index = std::move(index);
    r->anchors = std::move(anchors);
    r->compileMs = clock.elapsed();
    r->ok = !r->pages.empty();
    return r;
}

}  // namespace corinzia::ui
