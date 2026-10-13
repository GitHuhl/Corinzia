// DAW-tab side of AppController: arrange view content, timeline locate.
#include "corinzia/app/AppController.hpp"

#include <QVariantMap>

#include <algorithm>

namespace corinzia::app {

QColor AppController::trackColor(std::size_t index) {
    static const QColor kPalette[] = {QColor(47, 111, 181), QColor(214, 120, 34), QColor(46, 150, 90),
                                      QColor(176, 60, 140), QColor(120, 100, 200), QColor(190, 70, 60),
                                      QColor(40, 150, 160), QColor(150, 130, 40)};
    return kPalette[index % (sizeof(kPalette) / sizeof(kPalette[0]))];
}

void AppController::attachArrange(ui::ArrangeView* view) {
    arrange_ = view;
    if (!view) return;
    disconnect(view, nullptr, this, nullptr);   // re-attach safe (lambdas can't use UniqueConnection)
    connect(view, &ui::ArrangeView::blockClicked, this, [this](quint64 id) { onElementClicked(id, 0, 0, Qt::NoModifier); });
    connect(view, &ui::ArrangeView::timelineClicked, this, &AppController::locate);
    refreshArrange();
}

void AppController::refreshArrange() {
    emit arrangeChanged();
    if (!arrange_) return;
    ui::ArrangeContent c;
    for (std::size_t i = 0; i < project_.tracks.size(); ++i)
        c.tracks.push_back({QString::fromStdString(project_.tracks[i]->name), trackColor(i), 127, 0});
    const model::Movement* m = project_.movement(displayedMovement());
    if (m) {
        c.lengthQ = m->length.toQuarters();
        for (const auto& lane : m->lanes)
            for (const auto& v : lane.voices)
                for (const auto& ev : *v.events) {
                    if (ev.heads.empty()) continue;
                    // The block lives on the track that actually sounds the note
                    // (instrument changes move it to another lane).
                    const model::Instrument* inst = project_.instrument(lane.instrumentAt(ev.start));
                    if (!inst) continue;
                    int track = -1;
                    for (std::size_t t = 0; t < project_.tracks.size(); ++t)
                        if (project_.tracks[t]->id == inst->track) track = int(t);
                    if (track < 0) continue;
                    for (const auto& h : ev.heads) {
                        const int key = h.pitch.midiKey();
                        c.blocks.push_back({track, ev.start.toQuarters(), ev.duration.toQuarters(), key, h.id.id});
                        auto& tr = c.tracks[std::size_t(track)];
                        tr.minKey = std::min(tr.minKey, key);
                        tr.maxKey = std::max(tr.maxKey, key);
                    }
                }
        for (auto& tr : c.tracks)
            if (tr.minKey > tr.maxKey) tr.minKey = 60, tr.maxKey = 72;   // empty track
        for (const auto& b : arrangeBars()) c.barQuarters.push_back(b.toMap().value(QStringLiteral("quarter")).toDouble());
        c.barQuarters.push_back(c.lengthQ);
    }
    arrange_->setContent(std::move(c));
    arrange_->setSelection(selection_);
}

QVariantList AppController::arrangeBars() const {
    QVariantList out;
    const model::Movement* m = const_cast<model::Project&>(project_).movement(displayedMovement());
    if (!m) return out;
    int bar = 1;
    for (std::size_t i = 0; i < m->meter.size(); ++i) {
        const Fraction start = m->meter[i].at;
        const Fraction end = i + 1 < m->meter.size() ? m->meter[i + 1].at : m->length;
        const Fraction len(m->meter[i].numerator, m->meter[i].denominator);
        for (Fraction at = start; at < end; at = at + len)
            out.push_back(QVariantMap{{QStringLiteral("bar"), bar++}, {QStringLiteral("quarter"), at.toQuarters()}});
    }
    return out;
}

void AppController::locate(double quarter) {
    if (!sections_) return;
    for (const auto& sec : *sections_) {
        if (sec.movement != displayedMovement()) continue;
        rt::UiToAudio seek{rt::UiToAudio::Type::Seek};
        seek.a = sec.start + SamplePos(std::llround(sec.tempo.samplesAt(sec.performedQuarter(std::max(0.0, quarter)))));
        engine_.post(seek);
        if (arrange_) arrange_->setPlayheadQuarter(quarter);
        if (canvas_) canvas_->setPlayheadQuarter(quarter);
        locatedQ_ = quarter;
        return;
    }
}

}  // namespace corinzia::app
