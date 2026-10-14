// Corinzia application entry point.
//
//   corinzia [--no-audio] [project.project]
//   corinzia --smoke out.png      headless end-to-end check (used by ctest)
#include "corinzia/app/AppController.hpp"
#include "corinzia/edit/NoteInput.hpp"
#include "corinzia/ui/ArrangeView.hpp"
#include "corinzia/ui/ScoreCanvas.hpp"

#include <QCommandLineParser>
#include <QDir>
#include <QUrl>
#include <QFile>
#include <QElapsedTimer>
#include <QEventLoop>
#include <QGuiApplication>
#include <QKeyEvent>
#include <QMouseEvent>
#include <QQmlApplicationEngine>
#include <QQuickItem>
#include <QSvgRenderer>
#include <QQuickWindow>
#include <QTimer>
#include <QtQml>

#include <algorithm>
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

    // ---- Range selection: marquee, clipboard, repeat, spanners -------------
    {
        waitFor([&] { return canvas->tilesSettled(); }, 3000);
        const auto* e = canvas->engraving();
        const auto& mv = *app.project().movements.front();
        // Marquee over the clarinet's bars 1-2 (both voices), from empty paper.
        QRectF box;
        for (const auto& v : mv.lanes[1].voices)
            for (const auto& ev : *v.events)
                if (ev.start < corinzia::Fraction{2})
                    for (const auto& h : ev.heads)
                        if (const auto* en = e->index->find(h.id.id)) box |= en->bounds;
        const QRectF page = canvas->pageRectInItem(0);
        const qreal sc = page.width() / e->pages[0].size.width();
        auto toScene = [&](QPointF staff) { return canvas->mapToScene(page.topLeft() + staff * sc); };
        const QPointF a = toScene(QPointF(box.left() - 0.6, box.bottom() + 1.2));   // below-left: empty paper
        const QPointF b = toScene(QPointF(box.right() + 0.3, box.top() - 0.3));
        auto mouse = [&](QEvent::Type t, QPointF p, Qt::MouseButtons buttons, Qt::KeyboardModifiers mods = Qt::NoModifier) {
            QMouseEvent ev(t, p, p, Qt::LeftButton, buttons, mods);
            QCoreApplication::sendEvent(window, &ev);
        };
        mouse(QEvent::MouseButtonPress, a, Qt::LeftButton);
        mouse(QEvent::MouseMove, (a + b) / 2, Qt::LeftButton);
        const QImage dragging = window->grabWindow();
        mouse(QEvent::MouseMove, b, Qt::LeftButton);
        mouse(QEvent::MouseButtonRelease, b, Qt::NoButton);
        const auto& r = app.range();
        check(app.hasRange() && r.laneFrom == 1 && r.laneTo == 1 && r.from == corinzia::Fraction{0} && r.to == corinzia::Fraction{2},
              "marquee over clarinet bars 1-2 selects that passage");
        int bandPixels = 0;
        for (int y = int(std::min(a.y(), b.y())); y < int(std::max(a.y(), b.y())); y += 3)
            for (int x = int(a.x()) + 4; x < int(b.x()) - 4; x += 3) {
                const QColor c = dragging.pixelColor(x, y);
                if (c.blue() > c.red() + 8 && c.blue() > 200) ++bandPixels;   // translucent blue band over white paper
            }
        check(bandPixels > 50, "rubber band drawn while dragging");
        const QImage selected = window->grabWindow();
        int hl = 0;
        const QPointF mid = (a + b) / 2;
        for (int x = int(a.x()) + 4; x < int(b.x()) - 4; x += 2) {
            const QColor c = selected.pixelColor(x, int(mid.y()));
            if (c.blue() > c.red() + 8) ++hl;
        }
        check(hl > 20, "passage highlight drawn over the selection");
        check(app.selectionLabel().contains(QStringLiteral("Bars 1–2")), "selection label names the bars");

        // Copy, select a note in bar 5, paste there.
        key(Qt::Key_C, Qt::ControlModifier);
        check(app.canPaste(), "Ctrl+C filled the clipboard");
        const auto clar = [&](int voice) -> const corinzia::model::EventList& {
            for (const auto& v : app.project().movements.front()->lanes[1].voices) if (v.index == voice) return *v.events;
            return *app.project().movements.front()->lanes[1].voices[0].events;
        };
        const auto bar1 = clar(1);
        const auto* bar5note = corinzia::edit::eventAt(clar(1), corinzia::Fraction{4});
        const corinzia::ElementId bar5Original = bar5note->id.id;   // the demo repeats every 2 bars: compare identity
        if (const auto* en = e->index->find(bar5note->heads[0].id.id)) {
            const QPointF p = toScene(en->bounds.center());
            mouse(QEvent::MouseButtonPress, p, Qt::LeftButton);
            mouse(QEvent::MouseButtonRelease, p, Qt::NoButton);
        }
        key(Qt::Key_V, Qt::ControlModifier);
        auto samePitches = [&](corinzia::Fraction at, corinzia::Fraction src) {
            const auto* x = corinzia::edit::eventAt(clar(1), at);
            const auto* y = corinzia::edit::eventAt(bar1, src);
            return x && y && x->heads.size() == y->heads.size() && !x->heads.empty() &&
                   x->heads[0].pitch.midiKey() == y->heads[0].pitch.midiKey() && x->start - at == y->start - src;
        };
        check(samePitches(corinzia::Fraction{4}, corinzia::Fraction{0}) && samePitches(corinzia::Fraction{5}, corinzia::Fraction{1}) &&
                  corinzia::edit::eventAt(clar(1), corinzia::Fraction{4})->id.id != bar5Original,
              "Ctrl+V pasted bars 1-2 at the selected bar-5 note (both bars, fresh notes)");
        check(app.hasRange() && app.range().from == corinzia::Fraction{4}, "pasted passage is selected");
        key(Qt::Key_R, Qt::NoModifier, QStringLiteral("r"));
        check(samePitches(corinzia::Fraction{6}, corinzia::Fraction{0}), "R repeats the passage right after itself");
        app.undo();
        app.undo();
        check(corinzia::edit::eventAt(clar(1), corinzia::Fraction{4})->id.id == bar5Original,
              "two undos remove repeat and paste (original bar 5 back)");

        // Shift-click passage on the flute bar 2, then slur + hairpin.
        const auto& fl = *app.project().movements.front()->lanes[0].voices[0].events;
        const corinzia::model::MusicEvent* f0 = corinzia::edit::eventAt(fl, corinzia::Fraction{1});
        const corinzia::model::MusicEvent* f1 = corinzia::edit::eventAt(fl, corinzia::Fraction{3, 2});
        const auto* e0 = f0 && !f0->heads.empty() ? e->index->find(f0->heads[0].id.id) : nullptr;
        const auto* e1 = f1 && !f1->heads.empty() ? e->index->find(f1->heads[0].id.id) : nullptr;
        if (e0 && e1) {
            const QPointF p0 = toScene(e0->bounds.center()), p1 = toScene(e1->bounds.center());
            mouse(QEvent::MouseButtonPress, p0, Qt::LeftButton);
            mouse(QEvent::MouseButtonRelease, p0, Qt::NoButton);
            mouse(QEvent::MouseButtonPress, p1, Qt::LeftButton, Qt::ShiftModifier);
            mouse(QEvent::MouseButtonRelease, p1, Qt::NoButton, Qt::ShiftModifier);
        }
        check(app.hasRange() && app.range().laneFrom == 0 && app.range().from == corinzia::Fraction{1}, "shift-click extends to a passage");
        const int beforeSpan = app.engraveCount();
        key(Qt::Key_S, Qt::NoModifier, QStringLiteral("s"));
        key(Qt::Key_H, Qt::NoModifier, QStringLiteral("h"));
        {
            const auto& f = *app.project().movements.front()->lanes[0].voices[0].events;
            const auto* s0 = corinzia::edit::eventAt(f, corinzia::Fraction{1});
            const auto* s1 = corinzia::edit::eventAt(f, corinzia::Fraction{3, 2});
            check(s0 && s1 && (s0->spans & corinzia::model::SlurStart) && (s1->spans & corinzia::model::SlurEnd) &&
                      (s0->spans & corinzia::model::CrescStart) && (s1->spans & corinzia::model::HairpinEnd),
                  "S and H add slur and crescendo over the passage");
        }
        check(waitFor([&] { return app.engraveCount() > beforeSpan; }, 60000), "slur + hairpin engraved");
        waitFor([&] { return canvas->tilesSettled(); }, 3000);
        waitFor([] { return false; }, 150);
        QString spansPng = png;
        spansPng.replace(QStringLiteral(".png"), QStringLiteral("-selection.png"));
        window->grabWindow().save(spansPng);
        // Transpose the passage.
        const int keyBefore = corinzia::edit::eventAt(*app.project().movements.front()->lanes[0].voices[0].events, corinzia::Fraction{1})->heads[0].pitch.midiKey();
        key(Qt::Key_Up);
        const int keyAfter = corinzia::edit::eventAt(*app.project().movements.front()->lanes[0].voices[0].events, corinzia::Fraction{1})->heads[0].pitch.midiKey();
        check(keyAfter > keyBefore, "↑ transposes the whole passage");
        app.undo();
        // Select all + delete, then undo.
        key(Qt::Key_A, Qt::ControlModifier);
        check(app.hasRange() && app.range().laneFrom == 0 && app.range().laneTo == 1 && app.range().to == app.project().movements.front()->length,
              "Ctrl+A selects the whole movement");
        key(Qt::Key_Delete);
        bool allRests = true;
        for (const auto& ev : *app.project().movements.front()->lanes[0].voices[0].events) allRests &= ev.heads.empty();
        check(allRests, "Delete clears the passage (voice 1 becomes rests)");
        app.undo();
        check(!app.project().movements.front()->lanes[0].voices[0].events->front().heads.empty(), "undo restores the deleted music");
        key(Qt::Key_Escape);
        check(!app.hasRange(), "Esc clears the selection");
        app.undo();   // remove slur + hairpin again so later checks see the original flute
        app.undo();
    }

    // ---- Page / Panorama view ------------------------------------------------
    auto clickItem = [&](const char* name) {
        QQuickItem* b = findItem(window->contentItem(), QString::fromLatin1(name));
        if (b) QMetaObject::invokeMethod(b, "click");
        return b != nullptr;
    };
    {
        const int before = app.engraveCount();
        check(clickItem("panoramaViewButton") && app.viewMode() == QStringLiteral("panorama"), "Panorama button switches the view");
        check(waitFor([&] { return app.engraveCount() > before; }, 60000), "panorama engraved");
        const auto* e = canvas->engraving();
        const bool oneStrip = e && e->pages.size() == 1 && e->pages[0].size.width() > 3 * e->pages[0].size.height();
        check(oneStrip, "panorama is a single endless system (one page, much wider than tall)");
        if (e) std::printf("  panorama page %.0f x %.0f staff spaces\n", e->pages[0].size.width(), e->pages[0].size.height());
        waitFor([&] { return canvas->tilesSettled(); }, 5000);
        const QImage pano = window->grabWindow();
        int inkP = 0;
        const QRectF cv(canvas->mapToScene(QPointF(0, 0)), canvas->size());
        for (int y = int(cv.top()); y < int(cv.bottom()); y += 2)
            for (int x = int(cv.left()); x < int(cv.right()); x += 2)
                if (qGray(pano.pixel(x, y)) < 96) ++inkP;
        check(inkP > 500, "panorama music visible");
        int names = 0;
        for (const auto& pg : e ? e->pages : std::vector<corinzia::ui::EngravedPage>{})
            names += pg.svg.contains("<tspan>Fl.</tspan>") + pg.svg.contains("<tspan>Cl.</tspan>");
        {   // the staff names must lie inside the page box (not at negative x)
            QSvgRenderer r(e->pages[0].svg);
            check(names == 2 && r.viewBoxF().left() <= 0, "panorama keeps the staff names");
        }
        QString panoPng = png;
        panoPng.replace(QStringLiteral(".png"), QStringLiteral("-panorama.png"));
        pano.save(panoPng);
        // Mouse entry works on the single system too.
        key(Qt::Key_N, Qt::NoModifier, QStringLiteral("n"));
        const auto& clar2 = *app.project().movements.front()->lanes[1].voices[0].events;
        if (const auto* t = e ? e->index->find(clar2[0].heads[0].id.id) : nullptr) {
            const QRectF page = canvas->pageRectInItem(t->page);
            const QPointF sp = canvas->mapToScene(page.topLeft() + t->bounds.center() * (page.width() / e->pages[0].size.width()));
            QMouseEvent pr(QEvent::MouseButtonPress, sp, sp, Qt::LeftButton, Qt::LeftButton, Qt::NoModifier);
            QCoreApplication::sendEvent(window, &pr);
            QMouseEvent rl(QEvent::MouseButtonRelease, sp, sp, Qt::LeftButton, Qt::NoButton, Qt::NoModifier);
            QCoreApplication::sendEvent(window, &rl);
            check(app.selectionLabel().contains(QStringLiteral("Clarinet")), "mouse note entry works in panorama");
            app.undo();
        }
        key(Qt::Key_Escape);

        // Native engraver (Rust) for Panorama: fast, same canvas machinery.
        {
            const int beforeNative = app.engraveCount();
            check(clickItem("nativeEngraverButton") && app.nativeEngraver(), "Native button switches the engraver");
            check(waitFor([&] { return app.engraveCount() > beforeNative; }, 20000), "native engraving arrived");
            auto ne = canvas->engraving();
            check(ne && ne->index->staves().size() == 2 && ne->compileMs < 2000, "native engraving: both staves, fast");
            if (ne) std::printf("  native engraving: %lld ms, %zu staves\n", (long long)ne->compileMs, ne->index->staves().size());
            const auto& fl = flute();
            const auto firstHead = fl.front().heads.empty() ? fl.front().id.id : fl.front().heads.front().id.id;
            check(ne && ne->index->find(firstHead), "native engraving: noteheads are clickable objects");
            // Edit -> re-engraved quickly (debounce shrinks to tens of ms).
            check(app.selectFirstNote(), "select a note in native panorama");
            const int beforeEdit = app.engraveCount();
            QElapsedTimer t;
            t.start();
            app.transposeSelection(1);
            check(waitFor([&] { return app.engraveCount() > beforeEdit; }, 5000), "edit re-engraved natively");
            std::printf("  edit -> native re-engrave on screen: %lld ms\n", (long long)t.elapsed());
            check(t.elapsed() < 1500, "native re-engrave well under LilyPond's time");
            app.undo();
            waitFor([&] { return canvas->tilesSettled(); }, 3000);
            waitFor([] { return false; }, 150);
            QString nativePng = png;
            nativePng.replace(QStringLiteral(".png"), QStringLiteral("-native.png"));
            window->grabWindow().save(nativePng);
            check(clickItem("nativeEngraverButton") && !app.nativeEngraver(), "Native off again");
            const int beforeLily = app.engraveCount();
            check(waitFor([&] { return app.engraveCount() > beforeLily; }, 60000), "LilyPond panorama again");
        }
        const int before2 = app.engraveCount();
        check(clickItem("pageViewButton") && app.viewMode() == QStringLiteral("page"), "Page button switches back");
        check(waitFor([&] { return app.engraveCount() > before2; }, 60000) && canvas->engraving() &&
                  canvas->engraving()->pages[0].size.height() > canvas->engraving()->pages[0].size.width(),
              "page view is paginated again (A4 portrait)");
    }

    // ---- Score structure: popovers, Structure panel, insert/delete bars -------
    {
        namespace cz = corinzia;
        canvas->forceActiveFocus();
        auto mov = [&]() -> const cz::model::Movement& { return *app.project().movements.front(); };
        const cz::model::MusicEvent* bar3 = cz::edit::eventAt(flute(), cz::Fraction{2});
        check(bar3 != nullptr, "flute has music in bar 3");
        if (!bar3) return 1;
        const corinzia::ElementId bar3Id = bar3->id.id;
        app.selectElements({bar3->heads.empty() ? bar3->id.id : bar3->heads[0].id.id}, false);
        QQuickItem* pop = findItem(window->contentItem(), QStringLiteral("popover"));
        auto type = [&](const QString& s) { for (QChar c : s) key(0, Qt::NoModifier, QString(c)); };

        key(Qt::Key_K, Qt::ShiftModifier, QStringLiteral("K"));
        check(pop && pop->isVisible(), "Shift+K opens the key popover");
        type(QStringLiteral("Eb"));
        key(Qt::Key_Return);
        check(pop && !pop->isVisible() && mov().keys.size() == 2 && mov().keys[1].at == cz::Fraction{2} &&
                  mov().keys[1].fifths == -3 && !mov().keys[1].minor,
              "key popover: \"Eb\" puts E-flat major at bar 3");
        check(canvas->hasActiveFocus(), "focus returns to the score after the popover");

        key(Qt::Key_M, Qt::ShiftModifier, QStringLiteral("M"));
        type(QStringLiteral("3/5"));
        key(Qt::Key_Return);
        QQuickItem* err = findItem(window->contentItem(), QStringLiteral("popoverError"));
        check(pop && pop->isVisible() && err && !err->property("text").toString().isEmpty() && mov().meter.size() == 1,
              "a bad time signature keeps the popover open with a hint");
        key(Qt::Key_Escape);
        check(pop && !pop->isVisible() && mov().meter.size() == 1, "Esc cancels the popover");

        key(Qt::Key_T, Qt::ShiftModifier, QStringLiteral("T"));
        type(QStringLiteral("Allegro 132"));
        key(Qt::Key_Return);
        const auto tempoAt = [&](cz::Fraction at) -> const cz::model::TempoEvent* {
            for (const auto& t : mov().tempo) if (t.at == at) return &t;
            return nullptr;
        };
        check(tempoAt(cz::Fraction{2}) && tempoAt(cz::Fraction{2})->text == "Allegro" &&
                  tempoAt(cz::Fraction{2})->quarterBpm == 132.0,
              "tempo popover: \"Allegro 132\"");

        key(Qt::Key_X, Qt::ShiftModifier, QStringLiteral("X"));
        type(QStringLiteral("dolce"));
        key(Qt::Key_Return);
        check(mov().lanes[0].texts.size() == 1 && mov().lanes[0].texts[0].text == "dolce" &&
                  mov().lanes[0].texts[0].at == cz::Fraction{2},
              "expression popover: \"dolce\" under the flute at bar 3");

        check(clickItem("rehearsalButton") && mov().rehearsal.size() == 1 && mov().rehearsal[0].at == cz::Fraction{2},
              "Structure panel: rehearsal mark at bar 3");
        key(Qt::Key_B, Qt::ShiftModifier, QStringLiteral("B"));
        type(QStringLiteral(":|"));
        key(Qt::Key_Return);
        check(mov().barlines.size() == 1 && mov().barlines[0].at == cz::Fraction{2} &&
                  mov().barlines[0].kind == cz::model::BarlineKind::RepeatEnd,
              "barline popover: end repeat before bar 3");

        const int beforeStruct = app.engraveCount();
        check(waitFor([&] { return app.engraveCount() > beforeStruct && canvas->engraving() &&
                                   canvas->engraving()->source->revision == mov().revision; }, 60000),
              "structure engraved");
        const corinzia::ElementId textId = mov().lanes[0].texts[0].id.id;
        const auto* textEntry = canvas->engraving() ? canvas->engraving()->index->find(textId) : nullptr;
        check(textEntry && canvas->engraving()->index->find(mov().rehearsal[0].id.id),
              "text and rehearsal mark are clickable objects in the engraving");
        waitFor([&] { return canvas->tilesSettled(); }, 3000);
        waitFor([] { return false; }, 150);
        QString structPng = png;
        structPng.replace(QStringLiteral(".png"), QStringLiteral("-structure.png"));
        window->grabWindow().save(structPng);

        // Click the text in the score, Delete removes it, undo brings it back.
        if (textEntry) {
            const QRectF page = canvas->pageRectInItem(textEntry->page);
            const QPointF p = canvas->mapToScene(page.topLeft() + textEntry->bounds.center() * (page.width() / canvas->engraving()->pages[std::size_t(textEntry->page)].size.width()));
            QMouseEvent press(QEvent::MouseButtonPress, p, p, Qt::LeftButton, Qt::LeftButton, Qt::NoModifier);
            QCoreApplication::sendEvent(window, &press);
            QMouseEvent release(QEvent::MouseButtonRelease, p, p, Qt::LeftButton, Qt::NoButton, Qt::NoModifier);
            QCoreApplication::sendEvent(window, &release);
            if (app.selection() != textId)
                std::printf("  text bounds (%.2f,%.2f %.2fx%.2f) page %d; clicked at (%.0f,%.0f) selected '%s'\n",
                            textEntry->bounds.x(), textEntry->bounds.y(), textEntry->bounds.width(), textEntry->bounds.height(),
                            textEntry->page, p.x(), p.y(), qPrintable(app.selectionLabel()));
            check(app.selection() == textId && app.selectionLabel().contains(QStringLiteral("dolce")),
                  "clicking the text selects it");
            key(Qt::Key_Delete);
            check(mov().lanes[0].texts.empty(), "Delete removes the selected text");
            app.undo();
            check(mov().lanes[0].texts.size() == 1, "undo restores it");
        }

        // Insert / delete bars (one undo step each, music and structure move together).
        app.selectElements({bar3Id}, false);
        const cz::Fraction len = mov().length;
        key(Qt::Key_B, Qt::ControlModifier);
        const cz::model::MusicEvent* moved = cz::edit::eventAt(flute(), cz::Fraction{3});
        check(mov().length == len + cz::Fraction{1} && moved && moved->id.id == bar3Id &&
                  mov().rehearsal[0].at == cz::Fraction{3} && mov().keys[1].at == cz::Fraction{3} &&
                  mov().barlines[0].at == cz::Fraction{2},
              "Ctrl+B inserts a bar before bar 3 (music, key, mark and text move; the repeat stays)");
        app.undo();
        check(mov().length == len && cz::edit::eventAt(flute(), cz::Fraction{2})->id.id == bar3Id,
              "undo removes the inserted bar in one step");
        app.selectElements({bar3Id}, false);
        check(clickItem("deleteBarsButton") && mov().length == len - cz::Fraction{1} && mov().rehearsal.empty() &&
                  mov().keys.size() == 1 && mov().barlines.size() == 1,
              "Delete bars removes bar 3 with its key, mark and text");
        app.undo();
        check(mov().length == len && mov().rehearsal.size() == 1 && mov().keys.size() == 2 && mov().lanes[0].texts.size() == 1,
              "undo restores the deleted bar");

        // Repeat endings: the panel's 1st/2nd button over bar 3, and the Shift+R popover.
        app.selectElements({bar3Id}, false);
        check(clickItem("endingsButton") && mov().endings.size() == 2 && mov().endings[0].from == cz::Fraction{2} &&
                  mov().endings[0].to == cz::Fraction{3} && mov().endings[1].passes == 2 &&
                  std::any_of(mov().barlines.begin(), mov().barlines.end(),
                              [](const auto& b) { return b.at == cz::Fraction{3} && b.kind == cz::model::BarlineKind::RepeatEnd; }),
              "1st/2nd ending button: bar 3 = 1st ending + end repeat, bar 4 = 2nd ending");
        key(Qt::Key_R, Qt::ShiftModifier, QStringLiteral("R"));
        type(QStringLiteral("1-3"));
        key(Qt::Key_Return);
        check(pop && !pop->isVisible() && mov().endings.size() == 2 && mov().endings[0].passes == 7,
              "Shift+R \"1-3\" replaces the 1st ending with 1.–3.");
        const int beforeEnd = app.engraveCount();
        check(waitFor([&] { return app.engraveCount() > beforeEnd && canvas->engraving() &&
                                   canvas->engraving()->index->find(mov().endings[0].id.id); }, 60000),
              "endings engraved and clickable");
        app.undo();
        app.undo();
        check(mov().endings.empty(), "undo removes the endings");

        // Ornaments: Shift+O popover and the Notes panel, on the selected passage.
        app.selectElements({bar3Id}, false);
        key(Qt::Key_O, Qt::ShiftModifier, QStringLiteral("O"));
        type(QStringLiteral("tr"));
        key(Qt::Key_Return);
        const auto artAt2 = [&] { return cz::edit::eventAt(flute(), cz::Fraction{2})->articulations; };
        check(pop && !pop->isVisible() && (artAt2() & cz::model::ArtTrill), "Shift+O \"tr\" adds a trill");
        check(clickItem("ornament4096") && (artAt2() & cz::model::ArtCaesura), "Ornaments panel adds a caesura");
        const int beforeOrn = app.engraveCount();
        check(waitFor([&] { return app.engraveCount() > beforeOrn; }, 60000), "ornaments engraved");
        app.undo();
        app.undo();
        check(!(artAt2() & (cz::model::ArtTrill | cz::model::ArtCaesura)), "undo removes both");
        key(Qt::Key_Escape);
    }

    // ---- Mix (DAW) tab --------------------------------------------------------
    QQuickItem* tabs = findItem(window->contentItem(), QStringLiteral("tabs"));
    if (!tabs) tabs = findItem(window->contentItem()->parentItem() ? window->contentItem()->parentItem() : window->contentItem(),
                               QStringLiteral("tabs"));
    key(Qt::Key_2, Qt::ControlModifier);
    check(tabs && tabs->property("currentIndex").toInt() == 1, "Ctrl+2 shows the Mix tab");
    auto* arrange = qobject_cast<corinzia::ui::ArrangeView*>(findItem(window->contentItem(), QStringLiteral("arrange")));
    check(arrange && arrange->isVisible() && !canvas->isVisible(), "arrange view visible, score hidden");
    if (arrange) {
        int heads = 0;
        for (const auto& lane : app.project().movements.front()->lanes)
            for (const auto& v : lane.voices)
                for (const auto& ev : *v.events) heads += int(ev.heads.size());
        check(arrange->blockCount() == heads && arrange->trackCount() == 3, "one arrange block per note head, three tracks");
        bool picc = false;
        for (const auto& b : arrange->content().blocks) picc |= (b.track == 1 && b.startQ >= 16.0);
        check(picc, "doubling: bars 5-8 of Flute 1 sit on the Piccolo track");
        if (std::getenv("CORINZIA_SMOKE_DEBUG"))
            for (std::size_t i = 0; i < arrange->content().blocks.size(); ++i) {
                const auto& b = arrange->content().blocks[i];
                if (b.startQ >= 24 && b.startQ < 28) {
                    const QRectF r = arrange->blockRect(i);
                    std::printf("  block track %d q %.2f len %.2f key %d y %.1f\n", b.track, b.startQ, b.lengthQ, b.key, r.y());
                }
            }

        waitFor([] { return false; }, 100);
        const QImage mixFrame = window->grabWindow();
        int blue = 0;
        for (int y = 0; y < mixFrame.height(); ++y)
            for (int x = 0; x < mixFrame.width(); ++x) {
                const QColor c = mixFrame.pixelColor(x, y);
                if (std::abs(c.red() - 47) < 12 && std::abs(c.green() - 111) < 12 && std::abs(c.blue() - 181) < 12) ++blue;
            }
        check(blue > 100, "arrange blocks drawn on screen");
        QString mixPng = png;
        mixPng.replace(QStringLiteral(".png"), QStringLiteral("-mix.png"));
        mixFrame.save(mixPng);

        // Click a clarinet block -> shared selection.
        std::size_t clarBlock = 0;
        for (std::size_t i = 0; i < arrange->content().blocks.size(); ++i)
            if (arrange->content().blocks[i].track == 2) { clarBlock = i; break; }
        const QPointF bp = arrange->mapToScene(arrange->blockRect(clarBlock).center());
        QMouseEvent bpress(QEvent::MouseButtonPress, bp, bp, Qt::LeftButton, Qt::LeftButton, Qt::NoModifier);
        QCoreApplication::sendEvent(window, &bpress);
        QMouseEvent brelease(QEvent::MouseButtonRelease, bp, bp, Qt::LeftButton, Qt::NoButton, Qt::NoModifier);
        QCoreApplication::sendEvent(window, &brelease);
        check(app.selection() == arrange->content().blocks[clarBlock].element &&
                  app.selectionLabel().contains(QStringLiteral("Clarinet")),
              "clicking an arrange block selects that note (shared with the score)");

        // Click empty timeline below the lanes -> transport locates there.
        const double q = 6.5;
        const QPointF tp = arrange->mapToScene(QPointF(arrange->xForQuarter(q), 3 * arrange->laneHeight() + 20));
        QMouseEvent tpress(QEvent::MouseButtonPress, tp, tp, Qt::LeftButton, Qt::LeftButton, Qt::NoModifier);
        QCoreApplication::sendEvent(window, &tpress);
        QMouseEvent trelease(QEvent::MouseButtonRelease, tp, tp, Qt::LeftButton, Qt::NoButton, Qt::NoModifier);
        QCoreApplication::sendEvent(window, &trelease);
        const double expected = q * 60.0 / 96.0 * 48000.0;
        check(waitFor([&] { return std::abs(double(app.engine().playhead().blockStartSample) - expected) < 300; }, 2000),
              "timeline click locates the transport (sample-exact via the tempo map)");
        check(findItem(window->contentItem(), QStringLiteral("strip2")) != nullptr, "mixer strips present");
    }
    key(Qt::Key_1, Qt::ControlModifier);
    check(tabs && tabs->property("currentIndex").toInt() == 0 && canvas->hasActiveFocus(), "Ctrl+1 returns to the score with keyboard focus");
    key(Qt::Key_N, Qt::NoModifier, QStringLiteral("n"));
    check(app.inputMode(), "score keys work again after switching back");
    key(Qt::Key_Escape);

    // ---- Export: PDF (LilyPond, async), MIDI and MusicXML (Rust crate) -------
    {
        const QString base = QDir::temp().filePath(QStringLiteral("corinzia-smoke-export"));
        for (const char* ext : {".pdf", ".mid", ".musicxml"}) QFile::remove(base + QString::fromLatin1(ext));
        bool pdfDone = false, pdfOk = false;
        auto conn = QObject::connect(&app, &AppController::exportFinished, [&](bool ok, const QString&) { pdfDone = true; pdfOk = ok; });
        check(app.exportPdf(base), "PDF export started");
        check(waitFor([&] { return pdfDone; }, 60000) && pdfOk, "PDF export finished");
        QObject::disconnect(conn);
        QFile pdf(base + QStringLiteral(".pdf"));
        check(pdf.open(QIODevice::ReadOnly) && pdf.read(5) == "%PDF-" && pdf.size() > 5000, "PDF written (LilyPond page layout)");
        check(app.exportMidi(QUrl::fromLocalFile(base).toString()), "MIDI export (file:// URL as from the dialog)");
        QFile mid(base + QStringLiteral(".mid"));
        check(mid.open(QIODevice::ReadOnly) && mid.read(4) == "MThd" && mid.size() > 200, "MIDI file written");
        check(app.exportMusicXml(base), "MusicXML export");
        QFile xml(base + QStringLiteral(".musicxml"));
        const QByteArray xmlData = xml.open(QIODevice::ReadOnly) ? xml.readAll() : QByteArray();
        check(xmlData.contains("<score-partwise") && xmlData.count("<part id=") == 2 && xmlData.contains("<measure number=\"8\""),
              "MusicXML written (both staves, all bars)");
    }

    const QString file = QDir::temp().filePath(QStringLiteral("corinzia-smoke.project"));
    check(app.saveProjectAs(file), "saved to SQLite project");
    const int engravedBeforeOpen = app.engraveCount();
    check(app.openProject(file), "reopened project");
    check(waitFor([&] { return app.engraveCount() > engravedBeforeOpen; }, 60000), "reopened project engraved");
    {
        const auto& m = *app.project().movements.front();
        check(m.keys.size() == 2 && m.rehearsal.size() == 1 && m.barlines.size() == 1 && m.lanes[0].texts.size() == 1 &&
                  m.lanes[0].texts[0].text == "dolce" && m.tempo.size() >= 2,
              "key, tempo, barline, rehearsal mark and text survive save/reopen");
    }

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
    qmlRegisterType<corinzia::ui::ArrangeView>("Corinzia.Core", 1, 0, "ArrangeView");
    qmlRegisterSingletonInstance("Corinzia.Core", 1, 0, "App", &app);

    QQmlApplicationEngine engine;
    engine.loadFromModule("CorinziaApp", "Main");
    if (engine.rootObjects().isEmpty()) return 1;
    return smoke ? runSmoke(app, engine, args.value(smokeOpt)) : qtApp.exec();
}
