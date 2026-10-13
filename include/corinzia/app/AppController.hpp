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
#include "corinzia/edit/NoteInput.hpp"
#include "corinzia/model/Commands.hpp"
#include "corinzia/rt/PlayheadTracker.hpp"
#include "corinzia/storage/ProjectStore.hpp"
#include "corinzia/ui/ArrangeView.hpp"
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
    // ---- note input state (one NOTIFY: the toolbar re-reads everything) ------
    Q_PROPERTY(bool inputMode READ inputMode WRITE setInputMode NOTIFY inputStateChanged)
    Q_PROPERTY(int duration READ duration WRITE setDuration NOTIFY inputStateChanged)     // 0=whole .. 6=64th
    Q_PROPERTY(int dots READ dots WRITE setDots NOTIFY inputStateChanged)
    Q_PROPERTY(bool triplet READ triplet WRITE setTriplet NOTIFY inputStateChanged)
    Q_PROPERTY(int accidental READ accidental WRITE setAccidental NOTIFY inputStateChanged)  // 9=key, -1 flat, 0 natural, 1 sharp
    Q_PROPERTY(bool chordMode READ chordMode WRITE setChordMode NOTIFY inputStateChanged)
    Q_PROPERTY(int voice READ voice WRITE setVoice NOTIFY inputStateChanged)               // 1..4
    Q_PROPERTY(QString caretLabel READ caretLabel NOTIFY inputStateChanged)
    // ---- DAW tab -------------------------------------------------------------
    Q_PROPERTY(QVariantList arrangeBars READ arrangeBars NOTIFY arrangeChanged)   // [{bar, quarter}]

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
    Q_INVOKABLE void attachArrange(corinzia::ui::ArrangeView* view);
    Q_INVOKABLE void locate(double quarter);      // move the transport (DAW ruler / timeline click)
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

    // ---- note input (Sibelius/Dorico-style; see docs/ARCHITECTURE.md §5.5) ---
    Q_INVOKABLE void enterNote(int step, bool addToChord = false);   // step 0=C..6=B
    Q_INVOKABLE void enterRest();
    Q_INVOKABLE void moveCaret(int direction);                      // ±1 by the current duration
    Q_INVOKABLE void moveCaretStaff(int direction);                 // ±1 staff
    Q_INVOKABLE void backspace();                                   // delete the event before the caret
    Q_INVOKABLE void deleteSelection();
    Q_INVOKABLE void toggleArticulation(int flag);                  // model::Articulation bit
    Q_INVOKABLE void setDynamic(int mark);                          // 0=ppp..7=fff, -1 clears
    Q_INVOKABLE void toggleTie();
    Q_INVOKABLE void cycleVoice() { setVoice(voice_ % 4 + 1); }
    void placeNoteAt(int page, QPointF staffSpacePos, Qt::KeyboardModifiers mods);   // mouse entry

    bool inputMode() const noexcept { return inputMode_; }
    void setInputMode(bool on);
    int duration() const noexcept { return int(spec_.base); }
    void setDuration(int d);
    int dots() const noexcept { return spec_.dots; }
    void setDots(int d);
    bool triplet() const noexcept { return spec_.triplet; }
    void setTriplet(bool t);
    int accidental() const noexcept { return accidental_; }
    void setAccidental(int a);
    bool chordMode() const noexcept { return chordMode_; }
    void setChordMode(bool c);
    int voice() const noexcept { return voice_; }
    void setVoice(int v);
    QString caretLabel() const;
    QVariantList arrangeBars() const;
    static QColor trackColor(std::size_t index);
    Fraction caretPosition() const noexcept { return caretPos_; }

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
    void inputStateChanged();
    void arrangeChanged();

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
    // Note input helpers (AppControllerInput.cpp).
    struct VoiceRef { model::Movement* movement; model::PlayerLane* lane; model::Voice* voice; };
    VoiceRef caretVoice(bool create);
    void commitVoiceEdit(const VoiceRef& ref, const edit::EditResult& r, const char* label, bool extendLength);
    void moveCaretTo(std::size_t lane, int voice, Fraction pos);
    void audition(const model::PlayerLane& lane, Fraction at, const std::vector<model::Pitch>& pitches);
    void syncCaret();
    void refreshArrange();

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
    QPointer<ui::ArrangeView> arrange_;
    std::unique_ptr<storage::PersistenceWorker> persistence_;

    QTimer pumpTimer_;
    rt::PlayheadTracker tracker_;
    std::shared_ptr<const std::vector<rt::Section>> sections_;   // tempo maps of the live snapshot
    std::uint64_t compileRequested_ = 0;
    double locatedQ_ = 0.0;                                       // transport start/return position

    // Note input state.
    bool inputMode_ = false, chordMode_ = false;
    edit::DurationSpec spec_;
    int accidental_ = 9;              // 9 = follow the key signature
    int voice_ = 1;
    std::size_t caretLane_ = 0;
    Fraction caretPos_{0};

    ElementId selection_;
    QString selectionLabel_, status_;
    bool playing_ = false;
    int engraveCount_ = 0;
};

}  // namespace corinzia::app
