// Qt-side tests (run offscreen). Requires CORINZIA_LILYPOND (set by `nix develop`).
#include "Fixture.hpp"
#include "Test.hpp"

#include "corinzia/ui/EngravingService.hpp"
#include "corinzia/ui/ScoreCanvas.hpp"

#include <QEventLoop>
#include <QGuiApplication>
#include <QQuickWindow>
#include <QSGRendererInterface>
#include <QTimer>

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
