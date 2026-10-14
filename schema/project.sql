-- Corinzia project container (.project) — SQLite >= 3.45 (binary JSONB).
--
-- Read and written by rust/corinzia-model/src/store.rs.
--
-- Design rules
--  * One file = one project; autosave writes it from a worker thread (WAL).
--  * Every row is keyed by the persistent element id used in memory and in
--    the engraving. No surrogate keys.
--  * Music is stored in chunks (voice_chunk) of a fixed musical span (4 whole
--    notes = 16 crotchets), as JSONB.
--  * CLAP plugin state lives in its own table, so metadata writes never
--    rewrite large blobs.

PRAGMA application_id = 1129271898;   -- 'CORZ'
PRAGMA journal_mode   = WAL;
PRAGMA foreign_keys   = ON;

-- ------------------------------------------------------------------ global --
CREATE TABLE IF NOT EXISTS project (
    id           INTEGER PRIMARY KEY CHECK (id = 1),
    title        TEXT    NOT NULL DEFAULT '',
    composer     TEXT    NOT NULL DEFAULT '',
    sample_rate  REAL    NOT NULL DEFAULT 48000,
    next_id      INTEGER NOT NULL,                    -- element id allocator high-water mark
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
    --        "clefs":[{"id","p":player,"at","c":"bass"}], "texts":[{"id","p","at","t":"dolce","k":"x"|"t"}],
    --        "chords":[{"id","p","at","c":"F♯m7"}]}
    notation    BLOB    NOT NULL
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
--   {"id":17,"k":"n|r|c","s":[n,d],"d":[n,d],"a":bits,"dyn":5,"sp":span bits,
--    "h":[{"id":18,"p":[step,alter,octave],"t":0|1,"v":velocity}],
--    "ly":[{"v":verse,"t":"Glo","h":hyphen,"x":extender}]}
CREATE TABLE IF NOT EXISTS voice_chunk (
    voice_id     INTEGER NOT NULL REFERENCES voice(id) ON DELETE CASCADE,
    chunk_index  INTEGER NOT NULL,
    events       BLOB    NOT NULL,
    PRIMARY KEY (voice_id, chunk_index)
) WITHOUT ROWID;
