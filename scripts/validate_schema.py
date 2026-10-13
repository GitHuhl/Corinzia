#!/usr/bin/env python3
"""Validate schema/project.sql against schema/example_project.json.

Creates a fresh database from the schema, inserts every example row (JSON
values into JSONB columns via jsonb()), checks foreign keys, and runs the same
json_each / ->> queries ProjectStore::load uses. Run inside `nix develop`:

    python3 -I scripts/validate_schema.py
"""
import json
import pathlib
import sqlite3
import sys
import tempfile

ROOT = pathlib.Path(__file__).resolve().parent.parent
JSONB_COLUMNS = {
    ("project", "settings"), ("mixer_layout", "layout"), ("movement", "tempo"), ("movement", "meter"),
    ("movement", "keys"), ("voice_chunk", "events"), ("automation_chunk", "points"),
    ("undo_journal", "forward"), ("undo_journal", "inverse"),
}
BINARY_COLUMNS = {("plugin_state", "state"), ("plugin_state", "state_hash"), ("engraving_cache", "svg")}
TABLE_ORDER = [
    "project", "audio_node", "audio_edge", "plugin_state", "track", "mixer_layout", "player", "instrument",
    "movement", "player_lane", "instrument_change", "voice", "voice_chunk", "automation_lane",
    "automation_chunk", "undo_journal", "engraving_cache",
]


def main() -> int:
    assert sqlite3.sqlite_version_info >= (3, 45), f"SQLite {sqlite3.sqlite_version} lacks JSONB"
    schema = (ROOT / "schema/project.sql").read_text()
    example = json.loads((ROOT / "schema/example_project.json").read_text())

    with tempfile.TemporaryDirectory() as tmp:
        db = sqlite3.connect(pathlib.Path(tmp) / "example.project")
        db.executescript(schema)
        for table in TABLE_ORDER:
            for row in example[table]:
                cols, vals = [], []
                for col, val in row.items():
                    cols.append(col)
                    if (table, col) in JSONB_COLUMNS:
                        vals.append(("jsonb(?)", json.dumps(val)))
                    elif (table, col) in BINARY_COLUMNS:
                        vals.append(("?", str(val).encode()))
                    else:
                        vals.append(("?", val))
                sql = f"INSERT INTO {table}({','.join(cols)}) VALUES ({','.join(v[0] for v in vals)})"
                db.execute(sql, [v[1] for v in vals])
        db.commit()

        problems = db.execute("PRAGMA foreign_key_check").fetchall()
        assert not problems, f"foreign key violations: {problems}"
        assert db.execute("PRAGMA application_id").fetchone()[0] == 1129271898
        assert db.execute("PRAGMA journal_mode").fetchone()[0] == "wal"

        # Same decoding path as ProjectStore::load (SQLite parses the JSONB).
        heads = db.execute(
            "SELECT e.value->>'id', h.value->>'id', h.value->>'$.p[0]' FROM voice_chunk c, json_each(c.events) e "
            "LEFT JOIN json_each(e.value, '$.h') h WHERE c.voice_id = 40 ORDER BY c.chunk_index, e.key, h.key"
        ).fetchall()
        assert heads[0] == (41, 42, 0) and len(heads) == 8, heads
        tempo = db.execute(
            "SELECT j.value->>'bpm', j.value->>'ramp' FROM movement m, json_each(m.tempo) j WHERE m.id = 30"
        ).fetchall()
        assert tempo == [(120, 1), (96, 0)], tempo

        # Cascades: deleting the movement removes its lanes, voices, chunks, automation, undo, cache.
        db.execute("PRAGMA foreign_keys = ON")
        db.execute("DELETE FROM movement WHERE id = 30")
        for t in ("player_lane", "voice", "voice_chunk", "automation_lane", "automation_chunk",
                  "undo_journal", "engraving_cache", "instrument_change"):
            assert db.execute(f"SELECT count(*) FROM {t}").fetchone()[0] == 0, t
        # ...but never the players, routing or plugin state.
        assert db.execute("SELECT count(*) FROM plugin_state").fetchone()[0] == 1
        db.close()

    print(f"schema OK (SQLite {sqlite3.sqlite_version}): {sum(len(example[t]) for t in TABLE_ORDER)} example rows")
    return 0


if __name__ == "__main__":
    sys.exit(main())
