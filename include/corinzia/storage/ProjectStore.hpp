#pragma once
// SQLite persistence for the .project container (schema/project.sql).
//
// Threading: ProjectStore is used ONLY by the PersistenceWorker thread, which
// owns the write connection. The UI thread enqueues SaveBatches made of
// immutable snapshots (O(#voices) to build thanks to COW) and returns
// immediately; serialization to JSONB and the SQLite transaction happen on the
// worker. WAL mode means a crash mid-save leaves the previous commit intact.

#include "corinzia/model/Model.hpp"

#include <condition_variable>
#include <cstddef>
#include <deque>
#include <functional>
#include <memory>
#include <mutex>
#include <optional>
#include <string>
#include <thread>
#include <vector>

struct sqlite3;

namespace corinzia::storage {

inline constexpr std::int64_t kChunkSpanWholeNotes = 4;   // voice/automation chunk span

// Copyable header-level state (players, routing, mixer) captured on the UI thread.
struct ProjectHeader {
    model::ProjectMeta meta;
    std::uint64_t nextId = 1;
    struct PlayerRow { PlayerId id; std::string name; std::vector<model::Instrument> instruments; };
    std::vector<PlayerRow> players;
    std::vector<model::Track> tracks;
    model::AudioGraph graph;

    static ProjectHeader capture(const model::Project& p);
};

struct SaveBatch {
    std::optional<ProjectHeader> header;                 // routing/meta changed
    std::vector<model::MovementSnapshot> movements;      // movement rows + lanes + voices
    std::vector<VoiceId> dirtyVoices;                    // empty = all voices of `movements`
    std::vector<std::pair<NodeId, std::vector<std::byte>>> pluginStates;
    std::vector<MovementId> deletedMovements;
};

class ProjectStore {
public:
    static std::unique_ptr<ProjectStore> open(const std::string& path, std::string* error = nullptr);
    ~ProjectStore();

    bool save(const SaveBatch& batch, std::string* error = nullptr);   // one transaction
    bool load(model::Project& out, std::string* error = nullptr);

    bool appendUndo(std::optional<MovementId> movement, const std::string& label, const std::string& forwardJson,
                    const std::string& inverseJson);
    std::optional<std::vector<std::byte>> pluginState(NodeId node);

    // Introspection used by tests and the "file info" dialog.
    std::int64_t countRows(const char* table);
    sqlite3* handle() noexcept { return db_; }

private:
    explicit ProjectStore(sqlite3* db) : db_(db) {}
    bool exec(const char* sql, std::string* error = nullptr);
    bool saveHeader(const ProjectHeader& h);
    bool saveMovement(const model::Movement& m, const std::vector<VoiceId>& dirtyVoices);
    bool savePluginState(NodeId node, const std::vector<std::byte>& state);

    sqlite3* db_ = nullptr;
};

// Background writer with coalescing: if the user edits faster than SQLite
// commits, pending batches are merged so the queue never grows unbounded.
class PersistenceWorker {
public:
    explicit PersistenceWorker(std::unique_ptr<ProjectStore> store);
    ~PersistenceWorker();                                 // flushes, then joins

    void enqueue(SaveBatch batch);
    void flush();                                         // blocks until everything queued is committed
    void setErrorHandler(std::function<void(const std::string&)> h) { onError_ = std::move(h); }

private:
    void run();
    static void mergeInto(SaveBatch& dst, SaveBatch&& src);

    std::unique_ptr<ProjectStore> store_;
    std::mutex mutex_;
    std::condition_variable cv_, idleCv_;
    std::optional<SaveBatch> pending_;
    bool busy_ = false, stop_ = false;
    std::function<void(const std::string&)> onError_;
    std::thread thread_;
};

// Serialization helpers (exposed for tests and for the undo journal).
std::string eventsToJson(const model::EventList& events, std::size_t begin, std::size_t end);
std::int64_t chunkIndexOf(Fraction start) noexcept;

}  // namespace corinzia::storage
