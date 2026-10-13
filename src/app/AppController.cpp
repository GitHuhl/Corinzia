#include "corinzia/app/AppController.hpp"

#include "corinzia/app/DemoProject.hpp"
#include "corinzia/app/DummyBackend.hpp"
#include "corinzia/audio/PipeWireBackend.hpp"
#include "corinzia/compile/SnapshotCompiler.hpp"

#include <QFile>
#include <QMetaObject>
#include <QThreadPool>
#include <QVariantMap>

#include <algorithm>
#include <ctime>

namespace corinzia::app {

namespace {
std::int64_t monotonicNs() {
    timespec ts{};
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return std::int64_t(ts.tv_sec) * 1'000'000'000 + ts.tv_nsec;
}
QString pitchName(const model::Pitch& p) {
    static const char* kSteps = "CDEFGAB";
    QString s(QChar(kSteps[p.step]));
    for (int i = 0; i < p.alter; ++i) s += QChar(0x266F);   // ♯
    for (int i = 0; i > p.alter; --i) s += QChar(0x266D);   // ♭
    return s + QString::number(p.octave);
}
}  // namespace

AppController::AppController(Options opt, QObject* parent) : QObject(parent), opt_(std::move(opt)) {
    audio::ThreadRole::markMainThread();
    engine_.prepare(48000, 4096);
    plugins_.setEngineFormat(48000, 4096);
    plugins_.setGraphChangedHandler([this] { requestCompile(); });

    engraving_.setLilyPondPath(opt_.lilypond);
    engraving_.setDebounceMs(opt_.engraveDebounceMs);
    connect(&engraving_, &ui::EngravingService::engraved, this, [this](std::shared_ptr<const ui::EngraveResult> r) {
        if (canvas_ && r->movement == displayedMovement()) canvas_->setEngraving(r);
        ++engraveCount_;
        setStatus(QStringLiteral("Engraved in %1 ms").arg(r->compileMs));
        emit engraved();
    });
    connect(&engraving_, &ui::EngravingService::engraveFailed, this,
            [this](MovementId, QString log) { setStatus(QStringLiteral("Engraving failed: ") + log.left(200)); });

    if (opt_.enableAudio) {
        backend_ = std::make_unique<audio::PipeWireBackend>();
        std::string err;
        if (!backend_->start(engine_, {}, &err)) {
            backend_.reset();
            setStatus(QStringLiteral("PipeWire unavailable (%1) - using silent dummy driver").arg(QString::fromStdString(err)));
        }
    }
    if (!backend_) dummy_ = std::make_unique<DummyBackend>(engine_);

    pumpTimer_.setInterval(16);
    connect(&pumpTimer_, &QTimer::timeout, this, &AppController::pump);
    pumpTimer_.start();
    rescanPlugins();
    newDemoProject();
}

AppController::~AppController() {
    pumpTimer_.stop();
    // Order matters: stop the RT side first, then plugins, then pending jobs.
    backend_.reset();
    dummy_.reset();
    plugins_.shutdown();
    if (persistence_) persistence_->flush();
    QThreadPool::globalInstance()->waitForDone();
    engine_.collectGarbage();
}

bool AppController::audioRunning() const noexcept { return backend_ != nullptr; }

// ------------------------------------------------------------------ project --
void AppController::resetProject() {
    stop();
    plugins_.shutdown();
    persistence_.reset();
    project_ = model::Project{};
    undo_ = model::UndoStack{};
    selection_ = {};
    selectionLabel_.clear();
    inputMode_ = false;
    caretLane_ = 0;
    caretPos_ = Fraction{0};
    voice_ = 1;
}

void AppController::newDemoProject() {
    resetProject();
    buildDemoProject(project_);
    model::ChangeSet cs;
    for (const auto& m : project_.movements) cs.movements.push_back(m->id);
    cs.routingChanged = true;
    fanOut(cs);
    refreshArrange();
    emit projectChanged();
    emit selectionChanged();
}

bool AppController::openProject(const QString& path) {
    std::string err;
    auto store = storage::ProjectStore::open(path.toStdString(), &err);
    model::Project loaded;
    if (!store || !store->load(loaded, &err)) {
        setStatus(QStringLiteral("Open failed: ") + QString::fromStdString(err));
        return false;
    }
    resetProject();
    project_ = std::move(loaded);
    for (const auto& n : project_.graph.nodes)
        if (n.kind == model::NodeKind::ClapPlugin && !n.clapBundlePath.empty()) {
            std::string perr;
            if (!plugins_.instantiate(n.id, n.clapBundlePath, n.clapPluginId, n.stateChunk.get(), &perr))
                setStatus(QStringLiteral("Plugin missing: %1 (%2)").arg(QString::fromStdString(n.name), QString::fromStdString(perr)));
        }
    persistence_ = std::make_unique<storage::PersistenceWorker>(std::move(store));
    model::ChangeSet cs;
    for (const auto& m : project_.movements) cs.movements.push_back(m->id);
    cs.routingChanged = true;
    requestCompile();
    for (auto m : cs.movements) requestEngrave(m);
    refreshArrange();
    emit projectChanged();
    emit tracksChanged();
    emit historyChanged();
    return true;
}

bool AppController::saveProjectAs(const QString& path) {
    for (const char* ext : {"", "-wal", "-shm"}) QFile::remove(path + QString::fromLatin1(ext));
    std::string err;
    auto store = storage::ProjectStore::open(path.toStdString(), &err);
    if (!store) {
        setStatus(QStringLiteral("Save failed: ") + QString::fromStdString(err));
        return false;
    }
    persistence_.reset();
    model::ChangeSet all;
    all.routingChanged = true;
    for (const auto& m : project_.movements) all.movements.push_back(m->id);
    storage::SaveBatch b;
    b.header = storage::ProjectHeader::capture(project_);
    for (auto m : all.movements) b.movements.push_back(project_.snapshot(m));
    for (const auto& n : project_.graph.nodes)
        if (auto* inst = plugins_.instance(n.id)) {
            std::vector<std::byte> state;
            if (inst->saveState(state)) b.pluginStates.push_back({n.id, std::move(state)});
        }
    if (!store->save(b, &err)) {
        setStatus(QStringLiteral("Save failed: ") + QString::fromStdString(err));
        return false;
    }
    persistence_ = std::make_unique<storage::PersistenceWorker>(std::move(store));
    undo_.setClean();
    setStatus(QStringLiteral("Saved ") + path);
    return true;
}

QString AppController::title() const { return QString::fromStdString(project_.meta.title); }

MovementId AppController::displayedMovement() const {
    return project_.movements.empty() ? MovementId{} : project_.movements.front()->id;
}

// --------------------------------------------------------------- transport --
void AppController::play() {
    if (playing_) return;
    engine_.post({rt::UiToAudio::Type::Play});
    playing_ = true;
    emit transportChanged();
}

void AppController::stop() {
    if (!playing_) return;
    engine_.post({rt::UiToAudio::Type::Stop});
    // Return to where playback was started from (the located position), like a DAW.
    locate(locatedQ_);
    playing_ = false;
    if (canvas_) canvas_->setPlayheadQuarter(std::numeric_limits<double>::quiet_NaN());
    if (arrange_) arrange_->setPlayheadQuarter(locatedQ_);
    emit transportChanged();
}

// ----------------------------------------------------------------- editing --
void AppController::push(std::unique_ptr<model::Command> cmd) { fanOut(undo_.push(std::move(cmd), project_)); }

void AppController::undo() { fanOut(undo_.undo(project_)); }
void AppController::redo() { fanOut(undo_.redo(project_)); }

void AppController::fanOut(const model::ChangeSet& cs) {
    // 1. instant feedback (next frame)
    if (canvas_ && canvas_->engraving())
        for (const auto& r : cs.ranges)
            if (r.movement == displayedMovement()) canvas_->showProvisional(project_.snapshot(r.movement).movement, r.from, r.to);
    // 2. audible change (next block after the worker finishes)
    if (!cs.movements.empty() || cs.routingChanged) requestCompile();
    // 3. definitive engraving (debounced)
    for (auto m : cs.movements) requestEngrave(m);
    // 4. autosave (coalesced, own thread)
    enqueueSave(cs, false);
    if (cs.routingChanged) emit tracksChanged();
    if (!cs.movements.empty() || cs.routingChanged) refreshArrange();
    updateSelectionLabel();
    emit historyChanged();
}

bool AppController::selectFirstNote() {
    for (const auto& m : project_.movements)
        for (const auto& lane : m->lanes)
            for (const auto& v : lane.voices)
                for (const auto& ev : *v.events)
                    if (!ev.heads.empty()) {
                        selection_ = ev.heads.front().id.id;
                        if (canvas_) canvas_->setSelection(selection_);
                        updateSelectionLabel();
                        return true;
                    }
    return false;
}

void AppController::onElementClicked(quint64 elementId, quint64, quint64, Qt::KeyboardModifiers) {
    selection_ = ElementId{elementId};
    // The caret follows the selection, so pressing N continues from there.
    if (const model::Locator* loc = project_.locate(selection_))
        if (const model::Movement* m = project_.movement(loc->movement))
            for (std::size_t li = 0; li < m->lanes.size(); ++li)
                for (const auto& v : m->lanes[li].voices)
                    if (v.id == loc->voice)
                        if (const auto* ev = edit::findEvent(*v.events, selection_))
                            moveCaretTo(li, v.index, ev->start + ev->duration);
    updateSelectionLabel();
    syncCaret();
}

void AppController::transposeSelection(int steps) {
    const model::Locator* loc = project_.locate(selection_);
    if (!loc || loc->kind != model::Locator::Kind::NoteHead) return;
    model::Movement* m = project_.movement(loc->movement);
    if (!m) return;
    for (auto& lane : m->lanes)
        for (auto& v : lane.voices) {
            if (v.id != loc->voice) continue;
            auto copy = std::make_shared<model::EventList>(*v.events);
            for (auto& ev : *copy)
                for (auto& h : ev.heads)
                    if (h.id.id == selection_) {
                        const int d = h.pitch.step + 7 * h.pitch.octave + steps;
                        h.pitch.step = std::int8_t(((d % 7) + 7) % 7);
                        h.pitch.octave = std::int8_t((d - h.pitch.step) / 7);
                        push(std::make_unique<model::ReplaceVoiceEvents>(
                            m->id, v.id, std::move(copy), model::DirtyRange{m->id, ev.start, ev.start + ev.duration},
                            "Transpose"));
                        return;
                    }
        }
}

void AppController::updateSelectionLabel() {
    if (arrange_) arrange_->setSelection(selection_);
    QString label;
    if (const model::Locator* loc = project_.locate(selection_); loc && loc->kind == model::Locator::Kind::NoteHead)
        if (const model::Movement* m = project_.movement(loc->movement))
            for (const auto& lane : m->lanes)
                for (const auto& v : lane.voices)
                    if (v.id == loc->voice)
                        for (const auto& ev : *v.events)
                            for (const auto& h : ev.heads)
                                if (h.id.id == selection_) {
                                    const model::Instrument* inst = project_.instrument(lane.instrumentAt(ev.start));
                                    const model::Track* track = project_.track(loc->track);
                                    label = pitchName(h.pitch) + QStringLiteral(" · ") +
                                            (inst ? QString::fromStdString(inst->name) : QString()) +
                                            QStringLiteral(" · track ") +
                                            (track ? QString::fromStdString(track->name) : QStringLiteral("—"));
                                }
    if (label != selectionLabel_) {
        selectionLabel_ = label;
        emit selectionChanged();
    }
}

// ------------------------------------------------------------------- mixer --
QVariantList AppController::tracks() const {
    QVariantList out;
    for (std::size_t i = 0; i < project_.tracks.size(); ++i) {
        const auto& t = project_.tracks[i];
        const model::AudioNode* n = project_.node(t->instrumentNode);
        out.push_back(QVariantMap{{QStringLiteral("name"), QString::fromStdString(t->name)},
                                  {QStringLiteral("color"), trackColor(i)},
                                  {QStringLiteral("channel"), int(t->midiChannel) + 1},
                                  {QStringLiteral("gainDb"), t->gainDb},
                                  {QStringLiteral("plugin"), n ? QString::fromStdString(n->name) : QStringLiteral("—")}});
    }
    return out;
}

void AppController::setTrackGain(int idx, double db) {
    if (idx < 0 || idx >= int(project_.tracks.size())) return;
    push(std::make_unique<model::SetTrackGain>(project_.tracks[std::size_t(idx)]->id, float(db)));
}

QVariantList AppController::plugins() const {
    QVariantList out;
    for (const auto& p : pluginInfos_) {
        QStringList features;
        for (const auto& f : p.features) features << QString::fromStdString(f);
        out.push_back(QVariantMap{{QStringLiteral("name"), QString::fromStdString(p.name)},
                                  {QStringLiteral("vendor"), QString::fromStdString(p.vendor)},
                                  {QStringLiteral("features"), features.join(QStringLiteral(", "))}});
    }
    return out;
}

void AppController::rescanPlugins() {
    pluginInfos_ = plugins_.scan(audio::PluginManager::defaultSearchPaths());
    emit pluginsChanged();
}

bool AppController::assignPlugin(int trackIdx, int pluginIdx) {
    if (trackIdx < 0 || trackIdx >= int(project_.tracks.size()) || pluginIdx < 0 || pluginIdx >= int(pluginInfos_.size()))
        return false;
    model::Track& t = *project_.tracks[std::size_t(trackIdx)];
    const audio::PluginInfo& info = pluginInfos_[std::size_t(pluginIdx)];
    // Replace any previous instrument on this track (handshake-safe removal).
    if (t.instrumentNode.valid()) {
        plugins_.remove(t.instrumentNode);
        const NodeId old = t.instrumentNode;
        std::erase_if(project_.graph.nodes, [&](const model::AudioNode& n) { return n.id == old; });
        std::erase_if(project_.graph.edges, [&](const model::AudioEdge& e) { return e.from == old || e.to == old; });
    }
    NodeId node{project_.allocateId()};
    std::string err;
    if (!plugins_.instantiate(node, info.bundlePath, info.id, nullptr, &err)) {
        setStatus(QStringLiteral("Could not load %1: %2").arg(QString::fromStdString(info.name), QString::fromStdString(err)));
        t.instrumentNode = {};
        requestCompile();
        emit tracksChanged();
        return false;
    }
    project_.graph.nodes.push_back({node, model::NodeKind::ClapPlugin, info.name, info.bundlePath, info.id, nullptr, false});
    project_.graph.edges.push_back({node, 0, t.outputBus, 0, 1.f});
    t.instrumentNode = node;
    model::ChangeSet cs;
    cs.routingChanged = true;
    fanOut(cs);
    return true;
}

// ------------------------------------------------------------ fan-out sinks --
void AppController::requestCompile() {
    const std::uint64_t gen = ++compileRequested_;
    std::vector<model::MovementSnapshot> movements;
    for (const auto& m : project_.movements) movements.push_back(project_.snapshot(m->id));
    auto routing = compile::RoutingSnapshot::capture(project_);
    // Resolve plugins now, on the GUI thread: the worker must not touch the manager.
    std::unordered_map<NodeId, std::pair<audio::ClapPluginInstance*, std::uint32_t>> live;
    for (const auto& n : project_.graph.nodes)
        if (auto* inst = plugins_.instance(n.id)) live[n.id] = {inst, inst->latencySamples()};

    QPointer<AppController> self(this);
    QThreadPool::globalInstance()->start([self, gen, movements = std::move(movements), routing = std::move(routing),
                                          live = std::move(live)] {
        compile::CompileOptions opt;
        opt.generation = gen;
        opt.plugin = [&live](NodeId n) { auto it = live.find(n); return it == live.end() ? nullptr : it->second.first; };
        opt.latency = [&live](NodeId n) { auto it = live.find(n); return it == live.end() ? 0u : it->second.second; };
        // Owned until handed to the engine: a queued call that never runs
        // (controller shutting down) or a superseded result frees it.
        auto snap = std::shared_ptr<rt::PlaybackSnapshot>(compile::compileSnapshot(movements, routing, opt));
        QMetaObject::invokeMethod(
            self.data(),
            [self, snap, gen]() mutable {
                if (!self || gen < self->compileRequested_) return;   // superseded by a newer edit
                self->sections_ = std::make_shared<const std::vector<rt::Section>>(snap->sections);
                // The engine takes ownership; hand it a private copy so the
                // shared_ptr never frees what the RT thread now owns.
                rt::UiToAudio m{rt::UiToAudio::Type::SwapSnapshot};
                m.snapshot = new rt::PlaybackSnapshot(std::move(*snap));
                if (!self->engine_.post(m)) delete m.snapshot;
            },
            Qt::QueuedConnection);
    });
}

engrave::EngraveContext AppController::engraveContext() const {
    engrave::EngraveContext ctx;
    auto storage = std::make_shared<std::vector<model::Instrument>>();
    for (const auto& pl : project_.players) {
        ctx.playerNames[pl->id] = pl->name;
        for (const auto& i : pl->instruments) storage->push_back(*i);
    }
    for (const auto& i : *storage) {
        ctx.instruments[i.id] = &i;
        ctx.instrumentTracks[i.id] = i.track;
    }
    ctx.instrumentStorage = std::move(storage);
    return ctx;
}

void AppController::requestEngrave(MovementId movement) {
    if (movement != displayedMovement()) return;   // one visible movement for now
    engraving_.requestEngrave(project_.snapshot(movement), engraveContext());
}

void AppController::enqueueSave(const model::ChangeSet& cs, bool full) {
    if (!persistence_ || (cs.empty() && !full)) return;
    storage::SaveBatch b;
    if (full || cs.routingChanged || cs.metaChanged) b.header = storage::ProjectHeader::capture(project_);
    for (auto m : cs.movements) b.movements.push_back(project_.snapshot(m));
    b.dirtyVoices = cs.voices;
    persistence_->enqueue(std::move(b));
}

void AppController::attachCanvas(ui::ScoreCanvas* canvas) {
    canvas_ = canvas;
    if (!canvas) return;
    connect(canvas, &ui::ScoreCanvas::elementClicked, this, &AppController::onElementClicked, Qt::UniqueConnection);
    connect(canvas, &ui::ScoreCanvas::inputClicked, this, &AppController::placeNoteAt, Qt::UniqueConnection);
    syncCaret();
    if (auto m = displayedMovement(); m.valid()) {
        requestEngrave(m);
        engraving_.engraveNow(m);
    }
}

void AppController::setStatus(QString s) {
    if (s == status_) return;
    status_ = std::move(s);
    emit statusChanged();
}

// -------------------------------------------------------------------- pump --
void AppController::pump() {
    engine_.collectGarbage(
        [](const rt::AudioToUi& m, void* ctx) {
            auto* self = static_cast<AppController*>(ctx);
            if (self->plugins_.handleTelemetry(m)) return;
            // ParamChanged / gestures: automation recording hooks in here.
        },
        this);
    plugins_.pump();

    if ((!canvas_ && !arrange_) || !playing_ || !sections_) return;
    const rt::PlayheadState ps = engine_.playhead();
    const double sample = tracker_.update(ps, monotonicNs());
    for (const auto& sec : *sections_) {
        if (sec.movement != displayedMovement()) continue;
        if (sample >= double(sec.end)) {   // reached the end of the movement
            stop();
            return;
        }
        const double q = sample < double(sec.start) ? 0.0 : sec.tempo.quarterAtSample(sample - double(sec.start));
        if (canvas_) canvas_->setPlayheadQuarter(q);
        if (arrange_) arrange_->setPlayheadQuarter(q);
    }
}

}  // namespace corinzia::app
