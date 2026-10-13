// Qt-side tests (run offscreen). Requires CORINZIA_LILYPOND (set by `nix develop`).
#include "Fixture.hpp"
#include "Test.hpp"

#include "corinzia/ui/ArrangeView.hpp"
#include "corinzia/ui/EngravingService.hpp"
#include "corinzia/ui/ScoreCanvas.hpp"

#include <QDir>
#include <QFile>
#include <QProcess>
#include <QTemporaryDir>
#include <QEventLoop>
#include <QPainter>
#include <QSvgRenderer>
#include <QGuiApplication>
#include <QQuickWindow>
#include <QSGRendererInterface>
#include <QTimer>

#include <climits>
#include <limits>
#include <cstdlib>
#include <cstring>

using namespace corinzia;

namespace {

engrave::EngraveContext contextFor(const model::Project& p) {
    engrave::EngraveContext ctx;
    for (const auto& pl : p.players) {
        ctx.playerNames[pl->id] = pl->name;
        for (const auto& i : pl->instruments) {
            ctx.instruments[i->id] = i.get();
            ctx.instrumentTracks[i->id] = i->track;
        }
    }
    return ctx;
}

std::shared_ptr<const ui::EngraveResult> engraveAndWait(ui::EngravingService& svc, const model::MovementSnapshot& s,
                                                        const engrave::EngraveContext& ctx, int* count = nullptr) {
    std::shared_ptr<const ui::EngraveResult> got;
    QEventLoop loop;
    auto c1 = QObject::connect(&svc, &ui::EngravingService::engraved, [&](auto r) {
        got = r;
        if (count) ++*count;
        loop.quit();
    });
    auto c2 = QObject::connect(&svc, &ui::EngravingService::engraveFailed, [&](MovementId, QString log) {
        std::fprintf(stderr, "  engrave failed: %s\n", qPrintable(log));
        loop.quit();
    });
    QTimer::singleShot(60000, &loop, &QEventLoop::quit);
    svc.requestEngrave(s, ctx);
    loop.exec();
    QObject::disconnect(c1);
    QObject::disconnect(c2);
    return got;
}

struct Env {
    model::Project p;
    fixture::Ids ids;
    ui::EngravingService svc;
    Env() {
        ids = fixture::makeProject(p);
        svc.setLilyPondPath(QString::fromLocal8Bit(std::getenv("CORINZIA_LILYPOND")));
        svc.setDebounceMs(10);
    }
};

}  // namespace

TEST(engraving_service_definitive_pass_and_hit_testing) {
    Env env;
    auto r = engraveAndWait(env.svc, env.p.snapshot(env.ids.mov), contextFor(env.p));
    CHECK(r && r->ok && r->pages.size() == 1);
    if (!r || !r->ok) return;
    std::printf("  compiled in %lld ms, %zu tagged elements\n", (long long)r->compileMs, r->index->entries().size());
    CHECK(!r->pages[0].raster.isNull());

    const auto& evs = *env.p.movements[0]->lanes[0].voices[0].events;
    for (const auto& ev : evs)
        for (const auto& h : ev.heads) {
            const auto* e = r->index->find(h.id.id);
            CHECK(e != nullptr);
            if (!e) continue;
            // Clicking the centre of a head resolves to exactly that head and its DAW track.
            const auto* hit = r->index->hitTest(e->page, e->bounds.center());
            CHECK(hit && hit->id == h.id.id);
            const bool bar2 = ev.start >= Fraction{1};
            CHECK(hit && hit->track == (bar2 ? env.ids.t2 : env.ids.t1).id.value);
            CHECK(hit && hit->voice == env.ids.voice.id.value);
        }
    // Pitch -> y: higher notes are higher on the page (smaller y).
    CHECK(r->index->find(evs[2].heads[1].id.id)->bounds.center().y() < r->index->find(evs[2].heads[0].id.id)->bounds.center().y());

    // Cursor: at bar 2 beat 1 the playhead sits on the A5 head.
    const auto a5 = r->index->find(evs[3].heads[0].id.id)->bounds.center();
    auto cur = r->anchors->cursorAt(4.0);
    CHECK(cur.valid && std::abs(cur.x - a5.x()) < 1e-6 && cur.top < a5.y() && cur.bottom > a5.y());
    auto mid = r->anchors->cursorAt(4.5);   // half-way between A5 and B5
    const auto b5 = r->index->find(evs[4].heads[0].id.id)->bounds.center();
    CHECK(mid.valid && std::abs(mid.x - (a5.x() + b5.x()) / 2) < 1e-6);
}

TEST(provisional_layout_tracks_engraved_geometry) {
    Env env;
    auto r = engraveAndWait(env.svc, env.p.snapshot(env.ids.mov), contextFor(env.p));
    CHECK(r && r->ok);
    if (!r || !r->ok) return;
    // Edit: A5 -> C6 at bar 2 beat 1. The provisional head must land where an
    // engraved C6 sits (the C6 at beat 3 provides the reference).
    auto edited = std::make_shared<model::Movement>(*env.p.movements[0]);
    auto evs = std::make_shared<model::EventList>(*edited->lanes[0].voices[0].events);
    (*evs)[3].heads[0].pitch = {0, 0, 6};
    edited->lanes[0].voices[0].events = evs;
    auto glyphs = r->anchors->provisional(*edited, Fraction{1}, Fraction{5, 4});
    CHECK(glyphs.size() == 1);
    if (glyphs.size() != 1) return;
    const auto c6 = r->index->find((*evs)[5].heads[0].id.id)->bounds.center();
    const auto a5 = r->index->find((*evs)[3].heads[0].id.id)->bounds.center();
    CHECK(std::abs(glyphs[0].center.y() - c6.y()) < 0.3);   // within ~a quarter staff space
    CHECK(std::abs(glyphs[0].center.x() - a5.x()) < 1e-6);
    auto masks = r->anchors->staleRegions(Fraction{1}, Fraction{5, 4});
    CHECK(masks.size() == 1 && masks[0].rect.contains(a5));
}

TEST(engraving_service_drops_stale_revisions) {
    Env env;
    auto ctx = contextFor(env.p);
    env.svc.setDebounceMs(0);
    int delivered = 0;
    std::vector<std::uint64_t> revisions;
    QObject::connect(&env.svc, &ui::EngravingService::engraved, [&](auto r) { revisions.push_back(r->revision); });
    // Three rapid edits: only the last revision may ever be delivered.
    for (int i = 0; i < 3; ++i) {
        env.p.movements[0]->title = "Take " + std::to_string(i);
        ++env.p.movements[0]->revision;
        env.svc.requestEngrave(env.p.snapshot(env.ids.mov), ctx);
    }
    const auto last = env.p.movements[0]->revision;
    QEventLoop loop;
    QTimer poll;
    QObject::connect(&poll, &QTimer::timeout, [&] { if (!revisions.empty() && revisions.back() == last) loop.quit(); });
    poll.start(20);
    QTimer::singleShot(60000, &loop, &QEventLoop::quit);
    loop.exec();
    (void)delivered;
    CHECK(!revisions.empty() && revisions.back() == last);
    for (auto rv : revisions) CHECK(rv == last);
}

TEST(score_canvas_click_maps_to_element_and_track) {
    Env env;
    auto r = engraveAndWait(env.svc, env.p.snapshot(env.ids.mov), contextFor(env.p));
    CHECK(r && r->ok);
    if (!r || !r->ok) return;

    QQuickWindow window;
    window.resize(1200, 1600);
    ui::ScoreCanvas canvas(window.contentItem());
    canvas.setSize(QSizeF(1200, 1600));
    canvas.setZoom(1.5);
    canvas.setPan(QPointF(10, 20));
    canvas.setEngraving(r);
    canvas.setPlayheadQuarter(4.5);

    quint64 clicked = 0, track = 0;
    QObject::connect(&canvas, &ui::ScoreCanvas::elementClicked,
                     [&](quint64 id, quint64, quint64 t, Qt::KeyboardModifiers) { clicked = id; track = t; });

    const auto& evs = *env.p.movements[0]->lanes[0].voices[0].events;
    const auto* e = r->index->find(evs[4].heads[0].id.id);   // B5, piccolo
    const QRectF page = canvas.pageRectInItem(e->page);
    const qreal s = page.width() / r->pages[0].size.width();
    const QPointF pos = page.topLeft() + e->bounds.center() * s;
    CHECK(canvas.toPage(pos).page == 0);

    window.show();
    QMouseEvent press(QEvent::MouseButtonPress, pos, canvas.mapToScene(pos), canvas.mapToGlobal(pos), Qt::LeftButton,
                      Qt::LeftButton, Qt::NoModifier);
    QCoreApplication::sendEvent(&window, &press);
    CHECK(clicked == evs[4].heads[0].id.id.value);
    CHECK(track == env.ids.t2.id.value);

    // Exercise the scene-graph path (provisional overlay + playhead) for one frame.
    auto edited = std::make_shared<model::Movement>(*env.p.movements[0]);
    canvas.showProvisional(edited, Fraction{1}, Fraction{5, 4});
    const QImage frame = window.grabWindow();
    CHECK(!frame.isNull());
}

TEST(layout_locate_inverts_engraved_heads) {
    Env env;
    auto r = engraveAndWait(env.svc, env.p.snapshot(env.ids.mov), contextFor(env.p));
    CHECK(r && r->ok);
    if (!r || !r->ok) return;
    int checked = 0;
    for (const auto& ev : *env.p.movements[0]->lanes[0].voices[0].events)
        for (const auto& h : ev.heads) {
            const auto* e = r->index->find(h.id.id);
            if (!e) continue;
            const auto hit = r->anchors->locate(e->page, e->bounds.center());
            CHECK(hit.valid && hit.lane == 0);
            CHECK(hit.diatonic == h.pitch.step + 7 * h.pitch.octave);
            CHECK(std::abs(hit.quarter - ev.start.toQuarters()) < 0.15);
            ++checked;
        }
    CHECK(checked == 7);
    // The caret sits on the lane's staff, inside the system band.
    const auto caret = r->anchors->caretAt(0, 4.0);
    const auto sys = r->anchors->cursorAt(4.0);
    CHECK(caret.valid && caret.top >= sys.top - 1e-9 && caret.bottom - caret.top <= 6.0 + 1e-9);
    // Far away from any system: no hit.
    CHECK(!r->anchors->locate(0, QPointF(5, 160)).valid);
}

TEST(score_canvas_renders_engraving_pixels) {
    Env env;
    auto r = engraveAndWait(env.svc, env.p.snapshot(env.ids.mov), contextFor(env.p));
    CHECK(r && r->ok);
    if (!r || !r->ok) return;
    QQuickWindow window;
    window.resize(800, 600);
    window.setColor(QColor(128, 128, 128));
    ui::ScoreCanvas canvas(window.contentItem());
    canvas.setSize(QSizeF(800, 600));
    canvas.setZoom(1.0);
    canvas.setPan(QPointF(10, 10));
    canvas.setEngraving(r);
    window.show();
    const QImage frame = window.grabWindow();
    int white = 0, ink = 0;
    QRect whiteBox;
    for (int y = 0; y < frame.height(); ++y)
        for (int x = 0; x < frame.width(); ++x) {
            const int g = qGray(frame.pixel(x, y));
            if (g > 240) { ++white; whiteBox |= QRect(x, y, 1, 1); }
            if (g < 80) ++ink;
        }
    std::printf("  frame %dx%d white=%d box=(%d,%d %dx%d) ink=%d\n", frame.width(), frame.height(), white, whiteBox.x(),
                whiteBox.y(), whiteBox.width(), whiteBox.height(), ink);
    // Page 1 starts at (10,10) and covers the rest of the 800x600 view at 8 px/space.
    CHECK(whiteBox.left() <= 12 && whiteBox.top() <= 12 && whiteBox.width() > 700);
    CHECK(ink > 500);

    // A newer engraving replaces the page textures on a live window.
    ++env.p.movements[0]->revision;
    auto r2 = engraveAndWait(env.svc, env.p.snapshot(env.ids.mov), contextFor(env.p));
    CHECK(r2 && r2->ok);
    if (!r2 || !r2->ok) return;
    canvas.setEngraving(r2);
    const QImage frame2 = window.grabWindow();
    int ink2 = 0;
    for (int y = 0; y < frame2.height(); ++y)
        for (int x = 0; x < frame2.width(); ++x)
            if (qGray(frame2.pixel(x, y)) < 80) ++ink2;
    std::printf("  after re-engrave: ink=%d\n", ink2);
    CHECK(ink2 > 500);

    // Overlay layer across several frames: the playhead (red) must follow.
    auto redColumns = [](const QImage& f) {
        int lo = INT_MAX, hi = -1;
        for (int y = 0; y < f.height(); y += 2)
            for (int x = 0; x < f.width(); ++x) {
                const QColor c = f.pixelColor(x, y);
                if (c.red() > 150 && c.green() < 90 && c.blue() < 90) { lo = std::min(lo, x); hi = std::max(hi, x); }
            }
        return std::pair{lo, hi};
    };
    canvas.setPlayheadQuarter(0.0);
    const auto p0 = redColumns(window.grabWindow());
    canvas.setPlayheadQuarter(4.0);
    const auto p1 = redColumns(window.grabWindow());
    std::printf("  playhead columns q=0: %d..%d, q=4: %d..%d\n", p0.first, p0.second, p1.first, p1.second);
    CHECK(p0.second >= 0 && p1.second >= 0 && p1.first > p0.second);   // moved right, old one gone

    // Provisional layer: an edit shows blue sprite noteheads plus a paper mask.
    auto edited = std::make_shared<model::Movement>(*env.p.movements[0]);
    auto evs = std::make_shared<model::EventList>(*edited->lanes[0].voices[0].events);
    (*evs)[3].heads[0].pitch = {0, 0, 6};
    edited->lanes[0].voices[0].events = evs;
    canvas.setPlayheadQuarter(std::numeric_limits<double>::quiet_NaN());
    canvas.showProvisional(edited, Fraction{1}, Fraction{5, 4});
    const QImage frame3 = window.grabWindow();
    int blue = 0;
    for (int y = 0; y < frame3.height(); ++y)
        for (int x = 0; x < frame3.width(); ++x) {
            const QColor c = frame3.pixelColor(x, y);
            if (c.blue() > 120 && c.red() < 80) ++blue;
        }
    std::printf("  provisional sprite pixels: %d\n", blue);
    CHECK(blue > 20);
    canvas.setEngraving(r2);   // definitive result clears the provisional layer
    int blueAfter = 0;
    const QImage frame4 = window.grabWindow();
    for (int y = 0; y < frame4.height(); ++y)
        for (int x = 0; x < frame4.width(); ++x) {
            const QColor c = frame4.pixelColor(x, y);
            if (c.blue() > 120 && c.red() < 80) ++blueAfter;
        }
    CHECK(blueAfter == 0);
}

TEST(arrange_view_pixels_track_content_growth_and_scroll) {
    // Offset inside the window on purpose: a node that loses the item's
    // transform shows up at the wrong place.
    QQuickWindow window;
    window.resize(700, 420);
    window.setColor(Qt::white);
    ui::ArrangeView view(window.contentItem());
    view.setPosition(QPointF(50, 40));
    view.setSize(QSizeF(600, 360));
    view.setLaneHeight(60);
    const QColor colors[5] = {QColor(47, 111, 181), QColor(214, 120, 34), QColor(46, 150, 90), QColor(176, 60, 140),
                              QColor(120, 100, 200)};
    auto content = [&](int tracks, int blocks, bool swapped) {
        ui::ArrangeContent c;
        for (int t = 0; t < tracks; ++t) c.tracks.push_back({QString("T%1").arg(t), colors[t], 60, 72});
        for (int i = 0; i < blocks; ++i) {
            const int track = swapped ? (tracks - 1 - i % tracks) : i % tracks;
            c.blocks.push_back({track, i * 0.5, 0.4, 60 + (i * 5) % 13, ElementId{std::uint64_t(i + 1)}});
        }
        c.barQuarters = {0, 4, 8, 12, 16, 20, 24};
        c.lengthQ = 24;
        return c;
    };
    window.show();
    int wrong = 0, checked = 0;
    auto verify = [&](const char* phase) {
        const QImage f = window.grabWindow();
        int bad = 0;
        for (std::size_t i = 0; i < view.content().blocks.size(); ++i) {
            const QRectF r = view.blockRect(i);
            if (r.left() < 2 || r.right() > view.width() - 2) continue;   // partly culled / clipped
            const QPoint p = view.mapToScene(r.center()).toPoint();
            const QColor want = colors[view.content().blocks[i].track];
            const QColor got = f.pixelColor(p);
            ++checked;
            if (std::abs(got.red() - want.red()) > 10 || std::abs(got.green() - want.green()) > 10 ||
                std::abs(got.blue() - want.blue()) > 10)
                ++bad;
        }
        // Nothing may be drawn outside the item (clip + transform intact).
        int outside = 0;
        for (int y = 0; y < f.height(); ++y)
            for (int x = 0; x < f.width(); ++x) {
                const QPointF local = view.mapFromScene(QPointF(x, y));
                if (local.x() >= 0 && local.y() >= 0 && local.x() < view.width() && local.y() < view.height()) continue;
                if (f.pixelColor(x, y) != QColor(Qt::white)) ++outside;
            }
        std::printf("  %-22s wrong blocks %d, pixels outside item %d\n", phase, bad, outside);
        wrong += bad + (outside > 0 ? 1 : 0);
    };
    view.setContent(content(3, 9, false));
    verify("initial");
    view.setContent(content(3, 9, true));
    verify("tracks swapped");
    view.setContent(content(5, 60, false));      // more tracks and blocks: tree must grow
    verify("grown");
    view.setScrollX(120);
    verify("scrolled");
    view.setContent(content(2, 4, false));       // shrink: surplus nodes hidden
    verify("shrunk");
    CHECK(wrong == 0 && checked > 50);
}

TEST(score_canvas_grows_from_one_to_two_pages) {
    Env env;
    auto r1 = engraveAndWait(env.svc, env.p.snapshot(env.ids.mov), contextFor(env.p));
    CHECK(r1 && r1->ok && r1->pages.size() == 1);
    // ~100 bars of eighths: LilyPond needs a second page.
    auto& m = *env.p.movements[0];
    auto evs = std::make_shared<model::EventList>();
    for (int i = 0; i < 800; ++i) evs->push_back(fixture::ev(env.p, Fraction(i, 8), Fraction(1, 8), {fixture::head(env.p, i % 7, 5)}));
    m.lanes[0].voices[0].events = evs;
    m.length = Fraction(100);
    ++m.revision;
    auto r2 = engraveAndWait(env.svc, env.p.snapshot(env.ids.mov), contextFor(env.p));
    CHECK(r2 && r2->ok && r2->pages.size() >= 2);
    if (!r1 || !r2 || r2->pages.size() < 2) return;

    QQuickWindow window;
    window.resize(700, 900);
    window.setColor(QColor(128, 128, 128));
    ui::ScoreCanvas canvas(window.contentItem());
    canvas.setPosition(QPointF(40, 30));     // offset: transform errors become visible
    canvas.setSize(QSizeF(640, 860));
    canvas.setZoom(0.4);
    canvas.setPan(QPointF(10, 10));
    canvas.setEngraving(r1);
    window.show();
    (void)window.grabWindow();               // render the 1-page tree first
    canvas.setEngraving(r2);
    const QImage f = window.grabWindow();
    auto inkIn = [&](int page) {
        const QRectF pr = canvas.pageRectInItem(page).intersected(QRectF(0, 0, canvas.width(), canvas.height()));
        int ink = 0;
        for (int y = int(pr.top()); y < int(pr.bottom()); y += 2)
            for (int x = int(pr.left()); x < int(pr.right()); x += 2)
                if (qGray(f.pixel(canvas.mapToScene(QPointF(x, y)).toPoint())) < 80) ++ink;
        return ink;
    };
    const int ink1 = inkIn(0), ink2 = inkIn(1);
    int outside = 0;   // pixels left of / above the canvas must stay window grey
    for (int y = 0; y < 30; ++y)
        for (int x = 0; x < f.width(); ++x) if (f.pixelColor(x, y) != QColor(128, 128, 128)) ++outside;
    std::printf("  pages %zu, ink page1 %d page2 %d, stray pixels %d\n", r2->pages.size(), ink1, ink2, outside);
    CHECK(ink1 > 300 && ink2 > 300 && outside == 0);
}

TEST(score_canvas_is_vector_sharp_at_high_zoom) {
    Env env;
    auto r = engraveAndWait(env.svc, env.p.snapshot(env.ids.mov), contextFor(env.p));
    CHECK(r && r->ok);
    if (!r || !r->ok) return;
    QQuickWindow window;
    window.resize(900, 700);
    window.setColor(QColor(128, 128, 128));
    ui::ScoreCanvas canvas(window.contentItem());
    canvas.setPosition(QPointF(30, 20));
    canvas.setSize(QSizeF(840, 660));
    canvas.setPan(QPointF(-600, -300));      // look at the music, not the margin
    canvas.setZoom(1.0);
    canvas.setEngraving(r);
    window.show();
    (void)window.grabWindow();
    canvas.setZoom(4.0);                      // 32 px per staff space

    // Ground truth: the SVG rendered directly, at the same scale and origin.
    auto reference = [&] {
        QImage ref(window.size(), QImage::Format_ARGB32_Premultiplied);
        ref.fill(QColor(128, 128, 128));
        QPainter p(&ref);
        p.setRenderHint(QPainter::Antialiasing);
        const QRectF canvasScene(canvas.mapToScene(QPointF(0, 0)), canvas.size());
        p.setClipRect(canvasScene);
        const QPointF origin = canvas.mapToScene(canvas.pageRectInItem(0).topLeft());
        p.translate(origin);
        p.scale(32.0, 32.0);
        p.fillRect(QRectF(QPointF(0, 0), r->pages[0].size), Qt::white);
        QSvgRenderer svg(r->pages[0].svg);
        svg.render(&p, QRectF(QPointF(0, 0), r->pages[0].size));
        return ref;
    };
    auto meanDiff = [&](const QImage& got, const QImage& ref) {
        const QRect area = QRectF(canvas.mapToScene(QPointF(0, 0)), canvas.size()).toRect().adjusted(2, 2, -2, -2);
        double sum = 0;
        int n = 0, dark = 0;
        for (int y = area.top(); y < area.bottom(); ++y)
            for (int x = area.left(); x < area.right(); ++x) {
                const int a = qGray(got.pixel(x, y)), b = qGray(ref.pixel(x, y));
                sum += std::abs(a - b);
                ++n;
                if (b < 100) ++dark;
            }
        std::printf("    (reference ink pixels in view: %d)\n", dark);
        return sum / n;
    };
    const QImage ref = reference();
    const double blurry = meanDiff(window.grabWindow(), ref);   // tiles not rendered yet: scaled texture
    QEventLoop loop;
    QObject::connect(&canvas, &ui::ScoreCanvas::tilesReady, &loop, &QEventLoop::quit);
    QTimer::singleShot(5000, &loop, &QEventLoop::quit);
    if (!canvas.tilesSettled()) loop.exec();
    CHECK(canvas.tilesSettled());
    const QImage crispFrame = window.grabWindow();
    const double crisp = meanDiff(crispFrame, ref);
    std::printf("  mean |canvas - direct vector render| at 4x: before tiles %.2f, after tiles %.2f\n", blurry, crisp);
    crispFrame.save(QDir::temp().filePath(QStringLiteral("corinzia-zoom4.png")));
    CHECK(crisp < 0.5);                 // indistinguishable from direct vector rendering
    CHECK(blurry > 4 * crisp + 0.5);    // and the test can tell the difference
}

TEST(svg_text_renders_at_every_zoom) {
    // Real LilyPond text: a tempo mark and a markup.
    QTemporaryDir dir;
    const QString base = dir.filePath(QStringLiteral("text"));
    QProcess lily;
    lily.start(QString::fromLocal8Bit(std::getenv("CORINZIA_LILYPOND")),
               {QStringLiteral("-dbackend=svg"), QStringLiteral("-s"), QStringLiteral("-o"), base, QStringLiteral("-")});
    lily.waitForStarted();
    lily.write("\\version \"2.24.0\"\n\\header{tagline=##f}\n{ \\tempo 4 = 96 c'4^\\markup\\italic \"Piccolo\" d' e' f' }\n");
    lily.closeWriteChannel();
    lily.waitForFinished(30000);
    QFile f(base + QStringLiteral(".svg"));
    CHECK(f.open(QIODevice::ReadOnly));
    const QByteArray raw = f.readAll();
    const QByteArray fixed = ui::normalizeSvgText(raw);
    CHECK(fixed.contains("scale(0.01)") && !fixed.contains("font-size=\"2.2000\""));

    // Ink inside the page's top area (where the tempo text and markup sit),
    // per staff space squared, must be roughly zoom-independent.
    auto inkDensity = [](const QByteArray& svg, double s) {
        QSvgRenderer r(svg);
        const QSizeF page = r.viewBoxF().size();
        const QRectF area(0, 0, page.width(), 12.0);   // staff spaces: header region with the text
        QImage img(int(area.width() * s), int(area.height() * s), QImage::Format_ARGB32_Premultiplied);
        img.fill(Qt::white);
        QPainter p(&img);
        p.setRenderHint(QPainter::Antialiasing);
        p.scale(s, s);
        r.render(&p, QRectF(QPointF(0, 0), page));
        p.end();
        long ink = 0;
        for (int y = 0; y < img.height(); ++y)
            for (int x = 0; x < img.width(); ++x) ink += 255 - qGray(img.pixel(x, y));
        return double(ink) / (s * s);
    };
    double lo = 1e18, hi = 0;
    for (double s : {6.0, 12.0, 32.0, 64.0}) {
        const double d = inkDensity(fixed, s);
        std::printf("  zoom %4.0f px/space: ink per space² %.0f (raw svg: %.0f)\n", s, d, inkDensity(raw, s));
        lo = std::min(lo, d);
        hi = std::max(hi, d);
    }
    CHECK(lo > 0 && hi / lo < 1.5);   // text present at every zoom, same visual weight
}

int main(int argc, char** argv) {
    if (!std::getenv("CORINZIA_LILYPOND")) {
        std::printf("CORINZIA_LILYPOND not set; run inside `nix develop`. Skipping UI tests.\n");
        return 0;
    }
    QQuickWindow::setGraphicsApi(QSGRendererInterface::Software);   // headless CI: no GPU
    QGuiApplication app(argc, argv);
    int run = 0;
    for (auto& c : t::registry()) {
        if (argc > 1 && !std::strstr(c.name, argv[1])) continue;
        const int before = t::failures();
        c.fn();
        std::printf("%s %s\n", t::failures() == before ? "[ OK ]" : "[FAIL]", c.name);
        ++run;
    }
    std::printf("%d tests, %d failed checks\n", run, t::failures());
    return t::failures() == 0 ? 0 : 1;
}
