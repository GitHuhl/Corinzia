#pragma once
// The application's composition root, living on the GUI thread. It owns the
// Project and wires every subsystem together along the ChangeSet fan-out of
// docs/ARCHITECTURE.md §2.3:
//
//   QML action ─▶ Command ─▶ UndoStack ─▶ ChangeSet ─┬▶ ScoreCanvas::showProvisional   (next frame)
//                                                    ├▶ compile worker ─▶ AudioEngine  (SPSC swap)
//                                                    ├▶ EngravingService               (debounced)
//                                                    └▶ PersistenceWorker              (if a file is open)
//
// and runs the 60 Hz main-thread pump: RT telemetry and garbage, the
// PluginManager handshakes, and the latency-compensated playhead.

#include "corinzia/audio/AudioEngine.hpp"
#include "corinzia/audio/PluginManager.hpp"
#include "corinzia/model/Commands.hpp"
#include "corinzia/rt/PlayheadTracker.hpp"
#include "corinzia/storage/ProjectStore.hpp"
#include "corinzia/ui/EngravingService.hpp"
#include "corinzia/ui/ScoreCanvas.hpp"

#include <QObject>
#include <QPointer>
#include <QTimer>
#include <QVariantList>

#include <memory>

namespace corinzia::audio { class PipeWireBackend; }

namespace corinzia::app {

class DummyBackend;

class AppController : public QObject {
    Q_OBJECT
    Q_PROPERTY(bool playing READ playing NOTIFY transportChanged)
    Q_PROPERTY(bool canUndo READ canUndo NOTIFY historyChanged)
    Q_PROPERTY(bool canRedo READ canRedo NOTIFY historyChanged)
    Q_PROPERTY(QString title READ title NOTIFY projectChanged)
    Q_PROPERTY(QVariantList tracks READ tracks NOTIFY tracksChanged)
    Q_PROPERTY(QVariantList plugins READ plugins NOTIFY pluginsChanged)
    Q_PROPERTY(QString selectionLabel READ selectionLabel NOTIFY selectionChanged)
    Q_PROPERTY(QString status READ status NOTIFY statusChanged)
    Q_PROPERTY(int engraveCount READ engraveCount NOTIFY statusChanged)

public:
    struct Options {
        bool enableAudio = true;            // start the PipeWire backend if available
        QString lilypond = QStringLiteral("lilypond");
        int engraveDebounceMs = 350;
    };

    explicit AppController(Options opt, QObject* parent = nullptr);
    ~AppController() override;

    // ---- QML API --------------------------------------------------------------
    Q_INVOKABLE void attachCanvas(corinzia::ui::ScoreCanvas* canvas);
    Q_INVOKABLE void newDemoProject();
    Q_INVOKABLE bool openProject(const QString& path);
    Q_INVOKABLE bool saveProjectAs(const QString& path);

    Q_INVOKABLE void play();
    Q_INVOKABLE void stop();
    Q_INVOKABLE void togglePlay() { playing_ ? stop() : play(); }
    Q_INVOKABLE void undo();
    Q_INVOKABLE void redo();

    Q_INVOKABLE void transposeSelection(int diatonicSteps);
    Q_INVOKABLE bool selectFirstNote();          // keyboard entry point / smoke test
    Q_INVOKABLE void setTrackGain(int trackIndex, double gainDb);
    Q_INVOKABLE void rescanPlugins();
    Q_INVOKABLE bool assignPlugin(int trackIndex, int pluginIndex);

    bool playing() const noexcept { return playing_; }
    bool canUndo() const noexcept { return undo_.canUndo(); }
    bool canRedo() const noexcept { return undo_.canRedo(); }
    QString title() const;
    QVariantList tracks() const;
    QVariantList plugins() const;
    QString selectionLabel() const { return selectionLabel_; }
    QString status() const { return status_; }
    int engraveCount() const noexcept { return engraveCount_; }

    // For tests / smoke runs.
    const model::Project& project() const noexcept { return project_; }
    audio::AudioEngine& engine() noexcept { return engine_; }
    bool audioRunning() const noexcept;
    ElementId selection() const noexcept { return selection_; }

signals:
    void transportChanged();
    void historyChanged();
    void projectChanged();
    void tracksChanged();
    void pluginsChanged();
    void selectionChanged();
    void statusChanged();
    void engraved();

private slots:
    void onElementClicked(quint64 elementId, quint64 voiceId, quint64 trackId, Qt::KeyboardModifiers mods);
    void pump();

private:
    void resetProject();
    void push(std::unique_ptr<model::Command> cmd);
    void fanOut(const model::ChangeSet& cs);
    void requestCompile();
    void requestEngrave(MovementId movement);
    void enqueueSave(const model::ChangeSet& cs, bool full);
    void updateSelectionLabel();
    void setStatus(QString s);
    engrave::EngraveContext engraveContext() const;
    MovementId displayedMovement() const;

    Options opt_;
    model::Project project_;
    model::UndoStack undo_;

    audio::AudioEngine engine_;
    audio::PluginManager plugins_{engine_};
    std::unique_ptr<audio::PipeWireBackend> backend_;
    std::unique_ptr<DummyBackend> dummy_;                 // silent driver when PipeWire is unreachable
    std::vector<audio::PluginInfo> pluginInfos_;

    ui::EngravingService engraving_;
    QPointer<ui::ScoreCanvas> canvas_;
    std::unique_ptr<storage::PersistenceWorker> persistence_;

    QTimer pumpTimer_;
    rt::PlayheadTracker tracker_;
    std::shared_ptr<const std::vector<rt::Section>> sections_;   // tempo maps of the live snapshot
    std::uint64_t compileRequested_ = 0;

    ElementId selection_;
    QString selectionLabel_, status_;
    bool playing_ = false;
    int engraveCount_ = 0;
};

}  // namespace corinzia::app
