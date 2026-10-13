// Corinzia application entry point.
//
//   corinzia [--no-audio] [project.project]
//   corinzia --smoke out.png      headless end-to-end check (used by ctest)
#include "corinzia/app/AppController.hpp"
#include "corinzia/ui/ScoreCanvas.hpp"

#include <QCommandLineParser>
#include <QDir>
#include <QElapsedTimer>
#include <QEventLoop>
#include <QGuiApplication>
#include <QQmlApplicationEngine>
#include <QQuickWindow>
#include <QTimer>
#include <QtQml>

#include <cstdio>
#include <cstdlib>

using corinzia::app::AppController;

namespace {

// Spin the event loop until `pred` holds or `ms` elapse.
bool waitFor(const std::function<bool()>& pred, int ms) {
    QElapsedTimer t;
    t.start();
    while (!pred()) {
        if (t.elapsed() > ms) return false;
        QEventLoop loop;
        QTimer::singleShot(10, &loop, &QEventLoop::quit);
        loop.exec();
    }
    return true;
}

int runSmoke(AppController& app, QQmlApplicationEngine& engine, const QString& png) {
    int failures = 0;
    auto check = [&](bool ok, const char* what) {
        std::printf("%s %s\n", ok ? "[ OK ]" : "[FAIL]", what);
        failures += ok ? 0 : 1;
    };
    auto* window = qobject_cast<QQuickWindow*>(engine.rootObjects().value(0));
    check(window != nullptr, "QML main window loaded");
    if (!window) return 1;

    check(waitFor([&] { return app.engraveCount() >= 1; }, 60000), "demo score engraved by LilyPond");
    check(app.selectFirstNote() && !app.selectionLabel().isEmpty(), "note selected");
    const QString before = app.selectionLabel();
    app.transposeSelection(2);
    check(app.selectionLabel() != before && app.canUndo(), "transpose applied through the undo stack");
    check(waitFor([&] { return app.engraveCount() >= 2; }, 60000), "edit re-engraved in the background");

    app.play();
    waitFor([] { return false; }, 600);
    const auto ph = app.engine().playhead();
    check(ph.playing && ph.blockStartSample > 48000 / 4, "transport advanced in real time");
    app.stop();

    app.undo();
    check(app.selectionLabel() == before && !app.canUndo(), "undo restored the pitch");

    const QString file = QDir::temp().filePath(QStringLiteral("corinzia-smoke.project"));
    check(app.saveProjectAs(file), "saved to SQLite project");
    const int engravedBeforeOpen = app.engraveCount();
    check(app.openProject(file), "reopened project");
    check(waitFor([&] { return app.engraveCount() > engravedBeforeOpen; }, 60000), "reopened project engraved");

    waitFor([] { return false; }, 200);
    const QImage frame = window->grabWindow();
    check(!frame.isNull() && frame.save(png), "window rendered to PNG");

    // The engraved page must actually be on screen: count dark "ink" pixels
    // (staff lines, noteheads) inside the visible part of page 1.
    auto* canvas = window->findChild<corinzia::ui::ScoreCanvas*>(QStringLiteral("score"));
    int ink = 0;
    if (canvas) {
        const QRectF page = canvas->pageRectInItem(0);
        const QRectF visible = QRectF(canvas->mapToScene(page.topLeft()), page.size())
                                   .intersected(QRectF(canvas->mapToScene(QPointF(0, 0)), canvas->size()));
        const qreal dpr = frame.devicePixelRatio();
        std::printf("  canvas %.0fx%.0f, page 1 at (%.0f,%.0f) %.0fx%.0f, zoom %.2f\n", canvas->width(), canvas->height(),
                    page.x(), page.y(), page.width(), page.height(), canvas->zoom());
        for (int y = int(visible.top() * dpr); y < int(visible.bottom() * dpr); y += 2)
            for (int x = int(visible.left() * dpr); x < int(visible.right() * dpr); x += 2)
                if (qGray(frame.pixel(x, y)) < 96) ++ink;
    }
    std::printf("  ink pixels on page 1: %d\n", ink);
    check(ink > 500, "engraved music visible on the canvas");
    std::printf("%s (%d failures)\n", failures ? "SMOKE FAILED" : "SMOKE OK", failures);
    return failures ? 1 : 0;
}

}  // namespace

int main(int argc, char** argv) {
    QGuiApplication qtApp(argc, argv);
    QGuiApplication::setApplicationName(QStringLiteral("Corinzia"));

    QCommandLineParser args;
    args.addHelpOption();
    QCommandLineOption smokeOpt(QStringLiteral("smoke"), QStringLiteral("Headless end-to-end check; writes a screenshot."),
                                QStringLiteral("png"));
    QCommandLineOption noAudioOpt(QStringLiteral("no-audio"), QStringLiteral("Use the silent dummy driver."));
    args.addOptions({smokeOpt, noAudioOpt});
    args.addPositionalArgument(QStringLiteral("project"), QStringLiteral("Project file to open."));
    args.process(qtApp);

    const bool smoke = args.isSet(smokeOpt);
    if (smoke) QQuickWindow::setGraphicsApi(QSGRendererInterface::Software);   // headless: no GPU

    AppController::Options opt;
    opt.enableAudio = !args.isSet(noAudioOpt);
    if (const char* lily = std::getenv("CORINZIA_LILYPOND")) opt.lilypond = QString::fromLocal8Bit(lily);
    if (smoke) opt.engraveDebounceMs = 50;
    AppController app(opt);
    if (!args.positionalArguments().isEmpty()) app.openProject(args.positionalArguments().front());

    qmlRegisterType<corinzia::ui::ScoreCanvas>("Corinzia.Core", 1, 0, "ScoreCanvas");
    qmlRegisterSingletonInstance("Corinzia.Core", 1, 0, "App", &app);

    QQmlApplicationEngine engine;
    engine.loadFromModule("CorinziaApp", "Main");
    if (engine.rootObjects().isEmpty()) return 1;
    return smoke ? runSmoke(app, engine, args.value(smokeOpt)) : qtApp.exec();
}
