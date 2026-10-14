-- Corinzia project container (.project) — SQLite >= 3.45 (binary JSONB).
--
-- Design rules
--  * One file = one project. WAL mode: the PersistenceWorker writes on its own
--    connection while the UI thread never touches SQLite.
--  * Every row is keyed by the persistent ElementId used in memory, in the
--    engraved SVG (<g id="e-…">) and as CLAP note_id. No surrogate keys.
--  * Music is stored in *chunks* (voice_chunk / automation_chunk) of a fixed
--    musical span (4 whole notes = 16 crotchets). Editing one note rewrites one
--    small JSONB row, so auto-save cost is proportional to the edit, not the score.
--  * Large opaque blobs (CLAP plugin state, engraving cache) live in their own
--    tables, so metadata updates never rewrite megabytes of state.
--  * undo_journal persists the command history per movement → undo survives
--    reopen and can be scoped to a single movement.

PRAGMA application_id = 1129271898;   -- 'CORZ'
-- user_version (schema version) is managed by ProjectStore::open, which
-- creates/migrates: v1 -> v2 adds movement.notation.
PRAGMA journal_mode   = WAL;
PRAGMA foreign_keys   = ON;

-- ------------------------------------------------------------------ global --
CREATE TABLE IF NOT EXISTS project (
    id           INTEGER PRIMARY KEY CHECK (id = 1),
    title        TEXT    NOT NULL DEFAULT '',
    composer     TEXT    NOT NULL DEFAULT '',
    sample_rate  REAL    NOT NULL DEFAULT 48000,
    next_id      INTEGER NOT NULL,                    -- ElementId allocator high-water mark
    settings     BLOB,                                -- JSONB: engraving/house style, transport prefs
    created_at   TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    modified_at  TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

-- ------------------------------------------------------- routing / mixer --
CREATE TABLE IF NOT EXISTS audio_node (
    id              INTEGER PRIMARY KEY,
    kind            TEXT    NOT NULL CHECK (kind IN ('clap','bus','master')),
    name            TEXT    NOT NULL DEFAULT '',
    clap_bundle     TEXT,                             -- /usr/lib/clap/Foo.clap
    clap_plugin_id  TEXT,                             -- clap_plugin_descriptor.id
    bypassed        INTEGER NOT NULL DEFAULT 0
);

-- Routing matrix: one row per connection (sparse adjacency list).
CREATE TABLE IF NOT EXISTS audio_edge (
    from_node  INTEGER NOT NULL REFERENCES audio_node(id) ON DELETE CASCADE,
    from_port  INTEGER NOT NULL DEFAULT 0,
    to_node    INTEGER NOT NULL REFERENCES audio_node(id) ON DELETE CASCADE,
    to_port    INTEGER NOT NULL DEFAULT 0,
    gain       REAL    NOT NULL DEFAULT 1.0,
    PRIMARY KEY (from_node, from_port, to_node, to_port)
) WITHOUT ROWID;

-- CLAP state chunks (clap_plugin_state.save output), isolated for cheap metadata writes.
CREATE TABLE IF NOT EXISTS plugin_state (
    node_id     INTEGER PRIMARY KEY REFERENCES audio_node(id) ON DELETE CASCADE,
    state       BLOB    NOT NULL,
    state_hash  BLOB    NOT NULL,                     -- skip rewrite when unchanged
    saved_at    TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

CREATE TABLE IF NOT EXISTS track (
    id                  INTEGER PRIMARY KEY,
    ordinal             INTEGER NOT NULL,
    name                TEXT    NOT NULL,
    instrument_node_id  INTEGER REFERENCES audio_node(id) ON DELETE SET NULL,
    note_port           INTEGER NOT NULL DEFAULT 0,
    midi_channel        INTEGER NOT NULL DEFAULT 0 CHECK (midi_channel BETWEEN 0 AND 15),
    gain_db             REAL    NOT NULL DEFAULT 0,
    pan                 REAL    NOT NULL DEFAULT 0,
    mute                INTEGER NOT NULL DEFAULT 0,
    solo                INTEGER NOT NULL DEFAULT 0,
    output_bus_id       INTEGER REFERENCES audio_node(id) ON DELETE SET NULL
);

-- Named mixer/console layouts (strip order, widths, colours, visible sends).
CREATE TABLE IF NOT EXISTS mixer_layout (
    name    TEXT PRIMARY KEY,
    layout  BLOB NOT NULL                             -- JSONB
) WITHOUT ROWID;

-- ----------------------------------------------------- players / instruments --
CREATE TABLE IF NOT EXISTS player (
    id       INTEGER PRIMARY KEY,
    ordinal  INTEGER NOT NULL,
    name     TEXT    NOT NULL
);

CREATE TABLE IF NOT EXISTS instrument (
    id             INTEGER PRIMARY KEY,
    player_id      INTEGER NOT NULL REFERENCES player(id) ON DELETE CASCADE,
    ordinal        INTEGER NOT NULL,
    name           TEXT    NOT NULL,
    engraver_name  TEXT    NOT NULL DEFAULT '',
    transposition  INTEGER NOT NULL DEFAULT 0,
    clef           TEXT    NOT NULL DEFAULT 'treble',
    track_id       INTEGER REFERENCES track(id) ON DELETE SET NULL
);
CREATE INDEX IF NOT EXISTS instrument_by_player ON instrument(player_id, ordinal);

-- ---------------------------------------------------------------- movements --
CREATE TABLE IF NOT EXISTS movement (
    id          INTEGER PRIMARY KEY,
    ordinal     INTEGER NOT NULL,
    title       TEXT    NOT NULL DEFAULT '',
    length_num  INTEGER NOT NULL,
    length_den  INTEGER NOT NULL,
    revision    INTEGER NOT NULL DEFAULT 0,
    tempo       BLOB    NOT NULL,                     -- JSONB [{"at":[n,d],"bpm":120,"ramp":false}]
    meter       BLOB    NOT NULL,                     -- JSONB [{"at":[n,d],"num":4,"den":4}]
    keys        BLOB    NOT NULL,                     -- JSONB [{"at":[n,d],"fifths":0,"minor":false}]
    -- JSONB {"barlines":[{"id","at","k":"||"}], "marks":[{"id","at"}], "endings":[{"id","from","to","n":passes bitmask}],
    --        "clefs":[{"id","p":player,"at","c":"bass"}], "texts":[{"id","p","at","t":"dolce","k":"x"|"t"}]}
    notation    BLOB
);

CREATE TABLE IF NOT EXISTS player_lane (
    movement_id  INTEGER NOT NULL REFERENCES movement(id) ON DELETE CASCADE,
    player_id    INTEGER NOT NULL REFERENCES player(id)   ON DELETE CASCADE,
    ordinal      INTEGER NOT NULL,
    PRIMARY KEY (movement_id, player_id)
) WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS instrument_change (
    id             INTEGER PRIMARY KEY,
    movement_id    INTEGER NOT NULL,
    player_id      INTEGER NOT NULL,
    at_num         INTEGER NOT NULL,
    at_den         INTEGER NOT NULL,
    instrument_id  INTEGER NOT NULL REFERENCES instrument(id) ON DELETE CASCADE,
    FOREIGN KEY (movement_id, player_id) REFERENCES player_lane(movement_id, player_id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS change_by_lane ON instrument_change(movement_id, player_id);

CREATE TABLE IF NOT EXISTS voice (
    id           INTEGER PRIMARY KEY,
    movement_id  INTEGER NOT NULL,
    player_id    INTEGER NOT NULL,
    voice_index  INTEGER NOT NULL CHECK (voice_index BETWEEN 1 AND 4),
    FOREIGN KEY (movement_id, player_id) REFERENCES player_lane(movement_id, player_id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS voice_by_lane ON voice(movement_id, player_id);

-- Event payload, chunked by musical span: chunk_index = floor(start / 4 whole notes).
-- events: JSONB array of
--   {"id":17,"k":"n|r|c","s":[n,d],"d":[n,d],"a":bits,"dyn":5,
--    "h":[{"id":18,"p":[step,alter,octave],"t":0|1,"v":velocity}]}
CREATE TABLE IF NOT EXISTS voice_chunk (
    voice_id     INTEGER NOT NULL REFERENCES voice(id) ON DELETE CASCADE,
    chunk_index  INTEGER NOT NULL,
    events       BLOB    NOT NULL,
    PRIMARY KEY (voice_id, chunk_index)
) WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS automation_lane (
    id             INTEGER PRIMARY KEY,
    movement_id    INTEGER NOT NULL REFERENCES movement(id) ON DELETE CASCADE,
    name           TEXT    NOT NULL DEFAULT '',
    node_id        INTEGER NOT NULL REFERENCES audio_node(id) ON DELETE CASCADE,
    clap_param_id  INTEGER NOT NULL,
    mode           TEXT    NOT NULL CHECK (mode IN ('value','mod')),
    note_id        INTEGER NOT NULL DEFAULT -1
);

-- points: JSONB [{"id":40,"at":[n,d],"v":0.5,"c":"step|lin|exp|s"}]
CREATE TABLE IF NOT EXISTS automation_chunk (
    lane_id      INTEGER NOT NULL REFERENCES automation_lane(id) ON DELETE CASCADE,
    chunk_index  INTEGER NOT NULL,
    points       BLOB    NOT NULL,
    PRIMARY KEY (lane_id, chunk_index)
) WITHOUT ROWID;

-- ---------------------------------------------------------- history / cache --
-- Persistent, per-movement-scopable undo history (serialized Commands).
CREATE TABLE IF NOT EXISTS undo_journal (
    seq          INTEGER PRIMARY KEY AUTOINCREMENT,
    movement_id  INTEGER REFERENCES movement(id) ON DELETE CASCADE,   -- NULL = project-wide
    label        TEXT    NOT NULL,
    forward      BLOB    NOT NULL,                    -- JSONB command payload
    inverse      BLOB    NOT NULL,                    -- JSONB inverse payload
    undone       INTEGER NOT NULL DEFAULT 0,          -- 1 = on the redo side of the cursor
    created_at   TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE INDEX IF NOT EXISTS undo_by_movement ON undo_journal(movement_id, seq);

-- Last definitive engraving per movement: reopening a project shows real
-- engraving instantly while the background pass revalidates it.
CREATE TABLE IF NOT EXISTS engraving_cache (
    movement_id        INTEGER PRIMARY KEY REFERENCES movement(id) ON DELETE CASCADE,
    revision           INTEGER NOT NULL,              -- movement.revision it was built from
    engraver_version   TEXT    NOT NULL,              -- e.g. 'GNU LilyPond 2.26.0'
    svg                BLOB    NOT NULL               -- one or more pages, concatenated with \f
);
