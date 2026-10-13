#pragma once
// Command pattern = the only mutation path into the Project.
//
// Every executed command yields a ChangeSet. The UI thread fans that out to:
//   1. the canvas (instant-feedback pass for the touched measures),
//   2. the EngravingService (debounced definitive pass, per dirty movement),
//   3. the SnapshotCompiler (new PlaybackSnapshot -> audio thread),
//   4. the PersistenceWorker (dirty rows / chunks -> SQLite, one transaction).
// Undo/redo replays commands; it never diffs whole documents.

#include "corinzia/model/Model.hpp"

#include <memory>
#include <string>
#include <vector>

namespace corinzia::model {

struct DirtyRange { MovementId movement; Fraction from, to; };

struct ChangeSet {
    std::vector<MovementId> movements;      // musical content changed
    std::vector<DirtyRange> ranges;         // finer: which bars to re-layout instantly
    std::vector<VoiceId> voices;            // chunk-level persistence targets
    std::vector<LaneId> lanes;
    bool routingChanged = false;            // tracks / graph edges / nodes
    bool metaChanged = false;

    void merge(const ChangeSet& o);
    bool empty() const noexcept;
};

class Command {
public:
    virtual ~Command() = default;
    virtual std::string label() const = 0;
    virtual ChangeSet apply(Project& p) = 0;
    virtual ChangeSet revert(Project& p) = 0;
    // Coalesce rapid repeats (e.g. dragging a note up 5 semitones = 1 undo step).
    virtual bool mergeWith(const Command&) { return false; }
};

// Replaces a voice's event list wholesale. Because EventList is COW, keeping
// "before" and "after" costs two shared_ptrs, not two copies of the voice.
class ReplaceVoiceEvents final : public Command {
public:
    // `mergeable`: consecutive commands with the same label on the same voice
    // collapse into one undo step (dragging a pitch); note entry passes false.
    ReplaceVoiceEvents(MovementId m, VoiceId v, std::shared_ptr<const EventList> after,
                       DirtyRange range, std::string label, bool mergeable = true);
    std::string label() const override { return label_; }
    ChangeSet apply(Project& p) override;
    ChangeSet revert(Project& p) override;
    bool mergeWith(const Command& next) override;

private:
    ChangeSet swapIn(Project& p, std::shared_ptr<const EventList> list);
    MovementId movement_;
    VoiceId voice_;
    std::shared_ptr<const EventList> before_, after_;
    DirtyRange range_;
    std::string label_;
    bool mergeable_ = true;
};

// Adds an (empty) voice to a player lane, e.g. when input switches to voice 2.
class AddVoice final : public Command {
public:
    AddVoice(MovementId m, PlayerId player, VoiceId voice, std::uint8_t index)
        : movement_(m), player_(player), voice_(voice), index_(index) {}
    std::string label() const override { return "Add voice"; }
    ChangeSet apply(Project& p) override;
    ChangeSet revert(Project& p) override;

private:
    MovementId movement_;
    PlayerId player_;
    VoiceId voice_;
    std::uint8_t index_;
};

// Changes a movement's length (input past the final barline extends it).
class SetMovementLength final : public Command {
public:
    SetMovementLength(MovementId m, Fraction length) : movement_(m), after_(length) {}
    std::string label() const override { return "Movement length"; }
    ChangeSet apply(Project& p) override;
    ChangeSet revert(Project& p) override;

private:
    ChangeSet set(Project& p, Fraction len);
    MovementId movement_;
    Fraction before_{0}, after_;
    bool captured_ = false;
};

// Mixer edit: one DAW track's gain. Consecutive moves of the same fader merge.
class SetTrackGain final : public Command {
public:
    SetTrackGain(TrackId track, float gainDb) : track_(track), after_(gainDb) {}
    std::string label() const override { return "Track gain"; }
    ChangeSet apply(Project& p) override;
    ChangeSet revert(Project& p) override;
    bool mergeWith(const Command& next) override;

private:
    ChangeSet set(Project& p, float db);
    TrackId track_;
    float before_ = 0.f, after_;
    bool captured_ = false;
};

class UndoStack {
public:
    explicit UndoStack(std::size_t limit = 512) : limit_(limit) {}

    ChangeSet push(std::unique_ptr<Command> cmd, Project& p);  // executes, may merge
    ChangeSet undo(Project& p);
    ChangeSet redo(Project& p);

    // Group several commands into one undo step (e.g. "paste 32 bars").
    void beginMacro(std::string label);
    void endMacro();

    bool canUndo() const noexcept { return cursor_ > 0; }
    bool canRedo() const noexcept { return cursor_ < entries_.size(); }
    std::uint64_t cleanIndex() const noexcept { return cleanIndex_; }
    void setClean() noexcept { cleanIndex_ = cursor_; }

private:
    struct Entry { std::string label; std::vector<std::unique_ptr<Command>> cmds; };
    std::vector<Entry> entries_;
    std::size_t cursor_ = 0;
    std::size_t limit_;
    std::uint64_t cleanIndex_ = 0;
    int macroDepth_ = 0;
};

}  // namespace corinzia::model
