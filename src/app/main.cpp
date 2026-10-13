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
#include <QKeyEvent>
#include <QMouseEvent>
#include <QQmlApplicationEngine>
#include <QQuickItem>
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
    const int engravedBeforeInput = app.engraveCount();

    // ---- note input, driven like a user: keys into the canvas, a mouse click,
    //      a toolbar button ----------------------------------------------------
    auto* canvas = window->findChild<corinzia::ui::ScoreCanvas*>(QStringLiteral("score"));
    check(canvas != nullptr, "score canvas found");
    if (!canvas) return 1;
    canvas->forceActiveFocus();
    auto key = [&](int k, Qt::KeyboardModifiers mods = Qt::NoModifier, const QString& text = {}) {
        QKeyEvent press(QEvent::KeyPress, k, mods, text);
        QCoreApplication::sendEvent(window, &press);
        QKeyEvent release(QEvent::KeyRelease, k, mods, text);
        QCoreApplication::sendEvent(window, &release);
    };
    const auto& flute = [&]() -> const corinzia::model::EventList& {
        return *app.project().movements.front()->lanes[0].voices[0].events;
    };
    key(Qt::Key_N, Qt::NoModifier, QStringLiteral("n"));
    check(app.inputMode() && canvas->inputMode(), "N enters note input");
    key(Qt::Key_4, Qt::NoModifier, QStringLiteral("4"));                 // eighth notes
    check(app.duration() == 3, "4 selects eighth notes");
    for (auto [k, t] : {std::pair{Qt::Key_C, "c"}, {Qt::Key_D, "d"}, {Qt::Key_E, "e"}}) key(k, Qt::NoModifier, QString::fromLatin1(t));
    key(Qt::Key_G, Qt::ShiftModifier, QStringLiteral("G"));              // add G above E as a chord
    key(Qt::Key_R, Qt::NoModifier, QStringLiteral("r"));                 // eighth rest
    {
        const auto& v = flute();
        const bool notesOk = v.size() >= 4 && v[0].heads.size() == 1 && v[0].heads[0].pitch.step == 0 &&
                             v[0].heads[0].pitch.octave == 5 && v[1].heads[0].pitch.step == 1 &&
                             v[2].heads.size() == 2 && v[2].heads[1].pitch.step == 4 &&
                             v[3].kind == corinzia::model::EventKind::Rest && v[0].duration == corinzia::Fraction(1, 8);
        check(notesOk, "typed C D E +G(chord) R as eighths (nearest octave, key-aware)");
        check(app.caretPosition() == corinzia::Fraction(1, 2), "caret advanced to beat 3");
    }
    app.undo();
    check(flute().size() >= 3 && flute()[2].heads.size() == 2 && flute()[3].kind != corinzia::model::EventKind::Rest,
          "undo removes exactly the last entered item (the rest)");
    check(waitFor([&] { return app.engraveCount() > engravedBeforeInput; }, 60000), "typed notes engraved");

    // Mouse entry: click on an engraved clarinet note -> same pitch, clarinet staff.
    waitFor([] { return false; }, 300);
    const auto* res = canvas->engraving();
    const auto& clar = *app.project().movements.front()->lanes[1].voices[0].events;
    const auto* target = res ? res->index->find(clar[4].heads[0].id.id) : nullptr;   // bar 3, beat 1
    check(target != nullptr, "clarinet note located in the engraving");
    if (target) {
        const QRectF page = canvas->pageRectInItem(target->page);
        const qreal sc = page.width() / res->pages[std::size_t(target->page)].size.width();
        const QPointF itemPos = page.topLeft() + target->bounds.center() * sc;
        const QPointF scenePos = canvas->mapToScene(itemPos);
        const auto expected = clar[4].heads[0].pitch;
        const auto at = clar[4].start;
        QMouseEvent press(QEvent::MouseButtonPress, scenePos, scenePos, Qt::LeftButton, Qt::LeftButton, Qt::NoModifier);
        QCoreApplication::sendEvent(window, &press);
        QMouseEvent release(QEvent::MouseButtonRelease, scenePos, scenePos, Qt::LeftButton, Qt::NoButton, Qt::NoModifier);
        QCoreApplication::sendEvent(window, &release);
        const auto& clarNow = *app.project().movements.front()->lanes[1].voices[0].events;
        const auto* placed = corinzia::edit::eventAt(clarNow, at);
        check(placed && placed->start == at && placed->duration == corinzia::Fraction(1, 8) && !placed->heads.empty() &&
                  placed->heads[0].pitch.step == expected.step && placed->heads[0].pitch.octave == expected.octave,
              "mouse click placed an eighth on the clarinet staff at the clicked pitch and beat");
        check(app.selectionLabel().contains(QStringLiteral("Clarinet")), "selection follows the placed note");
    }

    // Voice 2 on the flute staff is created on demand (Alt+Up, V, A).
    key(Qt::Key_Up, Qt::AltModifier);
    key(Qt::Key_V, Qt::NoModifier, QStringLiteral("v"));
    key(Qt::Key_A, Qt::NoModifier, QStringLiteral("a"));
    {
        const auto& lane = app.project().movements.front()->lanes[0];
        check(lane.voices.size() == 2 && lane.voices[1].index == 2 && lane.voices[1].events->size() == 1,
              "typing in voice 2 creates the voice");
    }
    // Typing past the final barline extends the movement by whole bars.
    for (int i = 0; i < 80; ++i) key(Qt::Key_Right);
    key(Qt::Key_C, Qt::NoModifier, QStringLiteral("c"));
    check(app.project().movements.front()->length == corinzia::Fraction(9), "input past the end added a bar");
    app.undo();
    check(app.project().movements.front()->length == corinzia::Fraction(8), "undo removes the added bar too");

    // Toolbar: the panel's half-note button.
    // Repeater delegates are visual children only: search the item tree.
    std::function<QQuickItem*(QQuickItem*, const QString&)> findItem = [&](QQuickItem* it, const QString& name) -> QQuickItem* {
        if (it->objectName() == name) return it;
        for (QQuickItem* c : it->childItems())
            if (QQuickItem* f = findItem(c, name)) return f;
        return nullptr;
    };
    QQuickItem* half = findItem(window->contentItem(), QStringLiteral("duration1"));
    if (half) QMetaObject::invokeMethod(half, "click");
    check(half && app.duration() == 1, "notes panel button selects half notes");
    key(Qt::Key_Escape);
    check(!app.inputMode(), "Esc leaves note input");

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
