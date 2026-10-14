//! `.project` files: SQLite with the schema in schema/project.sql.

use crate::model::*;
use corinzia_score::Frac;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::Path;
use std::sync::Arc;

pub const SCHEMA: &str = include_str!("../../../schema/project.sql");
const SCHEMA_VERSION: i64 = 1;
/// Voice content is stored in chunks of this many whole notes.
const CHUNK_WHOLE_NOTES: i64 = 4;

#[derive(Debug)]
pub struct Error(pub String);
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for Error {}
impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self {
        Error(format!("sqlite: {e}"))
    }
}
impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error(format!("json: {e}"))
    }
}
pub type Result<T> = std::result::Result<T, Error>;

fn frac(v: &Value) -> Frac {
    match v.as_array() {
        Some(a) if a.len() == 2 => Frac::new(a[0].as_i64().unwrap_or(0), a[1].as_i64().unwrap_or(1).max(1)),
        _ => Frac::ZERO,
    }
}
fn jfrac(f: Frac) -> Value {
    json!([f.num, f.den])
}
fn chunk_of(start: Frac) -> i64 {
    start.num.div_euclid(start.den * CHUNK_WHOLE_NOTES)
}

/// Opens (creating or migrating as needed) a project database.
fn open_db(path: &Path) -> Result<Connection> {
    let db = Connection::open(path)?;
    db.execute_batch(SCHEMA)?;
    db.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    Ok(db)
}

pub fn load(path: &Path) -> Result<Project> {
    if !path.exists() {
        return Err(Error(format!("{} does not exist", path.display())));
    }
    let db = open_db(path)?;
    let mut p = Project { sample_rate: 48000.0, next_id: 1, ..Default::default() };
    if let Some((title, composer, sr, next)) = db
        .query_row("SELECT title, composer, sample_rate, next_id FROM project WHERE id = 1", [], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, f64>(2)?, r.get::<_, i64>(3)?))
        })
        .optional()?
    {
        p.title = title;
        p.composer = composer;
        p.sample_rate = sr;
        p.next_id = next as u64;
    }
    {
        let mut s = db.prepare("SELECT id, kind, name, clap_bundle, clap_plugin_id, bypassed FROM audio_node ORDER BY id")?;
        let rows = s.query_map([], |r| {
            Ok(AudioNode {
                id: r.get::<_, i64>(0)? as u64,
                kind: match r.get::<_, String>(1)?.as_str() {
                    "bus" => NodeKind::Bus,
                    "master" => NodeKind::Master,
                    _ => NodeKind::Clap,
                },
                name: r.get(2)?,
                bundle: r.get::<_, Option<String>>(3)?.unwrap_or_default(),
                plugin_id: r.get::<_, Option<String>>(4)?.unwrap_or_default(),
                bypassed: r.get::<_, i64>(5)? != 0,
            })
        })?;
        for n in rows {
            p.nodes.push(n?);
        }
        let mut s = db.prepare("SELECT from_node, from_port, to_node, to_port, gain FROM audio_edge ORDER BY from_node, to_node")?;
        let rows = s.query_map([], |r| {
            Ok(AudioEdge { from: r.get::<_, i64>(0)? as u64, from_port: r.get::<_, i64>(1)? as u32, to: r.get::<_, i64>(2)? as u64, to_port: r.get::<_, i64>(3)? as u32, gain: r.get(4)? })
        })?;
        for e in rows {
            p.edges.push(e?);
        }
        let mut s = db.prepare("SELECT node_id, state FROM plugin_state")?;
        let rows = s.query_map([], |r| Ok((r.get::<_, i64>(0)? as u64, r.get::<_, Vec<u8>>(1)?)))?;
        for row in rows {
            let (id, state) = row?;
            p.plugin_state.insert(id, Arc::new(state));
        }
    }
    {
        let mut s = db.prepare(
            "SELECT id, name, instrument_node_id, note_port, output_bus_id, midi_channel, gain_db, pan, mute, solo FROM track ORDER BY ordinal",
        )?;
        let rows = s.query_map([], |r| {
            Ok(Track {
                id: r.get::<_, i64>(0)? as u64,
                name: r.get(1)?,
                instrument_node: r.get::<_, Option<i64>>(2)?.map(|v| v as u64),
                note_port: r.get::<_, i64>(3)? as u16,
                output_bus: r.get::<_, Option<i64>>(4)?.map(|v| v as u64),
                midi_channel: r.get::<_, i64>(5)? as u8,
                gain_db: r.get(6)?,
                pan: r.get(7)?,
                mute: r.get::<_, i64>(8)? != 0,
                solo: r.get::<_, i64>(9)? != 0,
            })
        })?;
        for t in rows {
            p.tracks.push(t?);
        }
    }
    {
        let mut s = db.prepare("SELECT id, name FROM player ORDER BY ordinal")?;
        let rows = s.query_map([], |r| Ok((r.get::<_, i64>(0)? as u64, r.get::<_, String>(1)?)))?;
        for row in rows {
            let (id, name) = row?;
            p.players.push(Player { id, name, instruments: vec![] });
        }
        let mut s = db.prepare(
            "SELECT id, player_id, name, engraver_name, transposition, clef, track_id FROM instrument ORDER BY player_id, ordinal",
        )?;
        let rows = s.query_map([], |r| {
            Ok(Instrument {
                id: r.get::<_, i64>(0)? as u64,
                player: r.get::<_, i64>(1)? as u64,
                name: r.get(2)?,
                engraver_name: r.get(3)?,
                transposition: r.get::<_, i64>(4)? as i32,
                clef: r.get(5)?,
                track: r.get::<_, Option<i64>>(6)?.map(|v| v as u64),
            })
        })?;
        for i in rows {
            let i = i?;
            if let Some(pl) = p.players.iter_mut().find(|pl| pl.id == i.player) {
                pl.instruments.push(i.id);
            }
            p.instruments.push(i);
        }
    }

    let mut sm = db.prepare(
        "SELECT id, ordinal, title, length_num, length_den, revision, json(tempo), json(meter), json(keys), json(notation) \
         FROM movement ORDER BY ordinal",
    )?;
    let movements: Vec<(i64, i64, String, i64, i64, i64, String, String, String, Option<String>)> = sm
        .query_map([], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?, r.get(8)?, r.get(9)?))
        })?
        .collect::<std::result::Result<_, _>>()?;
    for (id, ordinal, title, ln, ld, rev, tempo, meter, keys, notation) in movements {
        let mut m = Movement {
            id: id as u64,
            ordinal,
            title,
            length: Frac::new(ln, ld.max(1)),
            revision: rev as u64,
            tempo: vec![],
            meter: vec![],
            keys: vec![],
            barlines: vec![],
            rehearsal: vec![],
            endings: vec![],
            lanes: vec![],
        };
        for t in serde_json::from_str::<Vec<Value>>(&tempo)? {
            m.tempo.push(Tempo {
                at: frac(&t["at"]),
                bpm: t["bpm"].as_f64().unwrap_or(120.0),
                ramp: t["ramp"].as_bool().unwrap_or(false),
                text: t["text"].as_str().unwrap_or("").to_string(),
            });
        }
        for t in serde_json::from_str::<Vec<Value>>(&meter)? {
            m.meter.push(Meter { at: frac(&t["at"]), num: t["num"].as_i64().unwrap_or(4), den: t["den"].as_i64().unwrap_or(4) });
        }
        for t in serde_json::from_str::<Vec<Value>>(&keys)? {
            m.keys.push(Key { at: frac(&t["at"]), fifths: t["fifths"].as_i64().unwrap_or(0) as i32, minor: t["minor"].as_bool().unwrap_or(false) });
        }
        // Lanes, instrument changes, voices.
        let mut sl = db.prepare("SELECT player_id FROM player_lane WHERE movement_id = ? ORDER BY ordinal")?;
        let players: Vec<i64> = sl.query_map([id], |r| r.get(0))?.collect::<std::result::Result<_, _>>()?;
        for pid in players {
            let mut lane = Lane { player: pid as u64, changes: vec![], clefs: vec![], texts: vec![], chords: vec![], voices: vec![] };
            let mut sc = db.prepare(
                "SELECT id, at_num, at_den, instrument_id FROM instrument_change WHERE movement_id = ? AND player_id = ? \
                 ORDER BY at_num * 1.0 / at_den",
            )?;
            for c in sc.query_map(params![id, pid], |r| {
                Ok(InstrumentChange {
                    id: r.get::<_, i64>(0)? as u64,
                    at: Frac::new(r.get(1)?, r.get::<_, i64>(2)?.max(1)),
                    instrument: r.get::<_, i64>(3)? as u64,
                })
            })? {
                lane.changes.push(c?);
            }
            let mut sv = db.prepare("SELECT id, voice_index FROM voice WHERE movement_id = ? AND player_id = ? ORDER BY voice_index")?;
            let voices: Vec<(i64, i64)> = sv.query_map(params![id, pid], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<std::result::Result<_, _>>()?;
            for (vid, index) in voices {
                let mut sch = db.prepare("SELECT json(events) FROM voice_chunk WHERE voice_id = ? ORDER BY chunk_index")?;
                let chunks: Vec<String> = sch.query_map([vid], |r| r.get(0))?.collect::<std::result::Result<_, _>>()?;
                let mut events = Vec::new();
                for c in chunks {
                    for e in serde_json::from_str::<Vec<StoredEvent>>(&c)? {
                        events.push(e.into_event());
                    }
                }
                lane.voices.push(Voice { id: vid as u64, index: index as u8, events: Arc::new(events) });
            }
            m.lanes.push(lane);
        }
        // Structure marks, clefs, texts, chord symbols.
        if let Some(n) = notation {
            let n: Value = serde_json::from_str(&n)?;
            for b in n["barlines"].as_array().into_iter().flatten() {
                m.barlines.push(Barline { id: b["id"].as_u64().unwrap_or(0), at: frac(&b["at"]), kind: BarKind::from_code(b["k"].as_str().unwrap_or("||")) });
            }
            for r in n["marks"].as_array().into_iter().flatten() {
                m.rehearsal.push(RehearsalMark { id: r["id"].as_u64().unwrap_or(0), at: frac(&r["at"]) });
            }
            for e in n["endings"].as_array().into_iter().flatten() {
                m.endings.push(Ending { id: e["id"].as_u64().unwrap_or(0), from: frac(&e["from"]), to: frac(&e["to"]), passes: e["n"].as_u64().unwrap_or(1) as u8 });
            }
            for c in n["clefs"].as_array().into_iter().flatten() {
                let pid = c["p"].as_u64().unwrap_or(0);
                if let Some(l) = m.lanes.iter_mut().find(|l| l.player == pid) {
                    l.clefs.push(ClefChange { id: c["id"].as_u64().unwrap_or(0), at: frac(&c["at"]), clef: c["c"].as_str().unwrap_or("treble").into() });
                }
            }
            for t in n["texts"].as_array().into_iter().flatten() {
                let pid = t["p"].as_u64().unwrap_or(0);
                if let Some(l) = m.lanes.iter_mut().find(|l| l.player == pid) {
                    l.texts.push(TextItem {
                        id: t["id"].as_u64().unwrap_or(0),
                        at: frac(&t["at"]),
                        text: t["t"].as_str().unwrap_or("").into(),
                        kind: if t["k"].as_str() == Some("t") { TextKind::Technique } else { TextKind::Expression },
                    });
                }
            }
            for c in n["chords"].as_array().into_iter().flatten() {
                let pid = c["p"].as_u64().unwrap_or(0);
                if let Some(l) = m.lanes.iter_mut().find(|l| l.player == pid) {
                    l.chords.push(ChordSymbol { id: c["id"].as_u64().unwrap_or(0), at: frac(&c["at"]), text: c["c"].as_str().unwrap_or("").into() });
                }
            }
        }
        p.movements.push(m);
    }
    Ok(p)
}

#[derive(Deserialize)]
struct StoredHead {
    id: u64,
    p: [i64; 3],
    #[serde(default)]
    t: i64,
    #[serde(default)]
    v: i64,
}
#[derive(Deserialize)]
struct StoredEvent {
    id: u64,
    k: String,
    s: [i64; 2],
    d: [i64; 2],
    #[serde(default)]
    a: u16,
    #[serde(default, rename = "dyn")]
    dynamic: Option<u8>,
    #[serde(default)]
    sp: u8,
    #[serde(default)]
    h: Vec<StoredHead>,
    #[serde(default)]
    ly: Vec<StoredLyric>,
}
#[derive(Deserialize)]
struct StoredLyric {
    #[serde(default)]
    v: u8,
    t: String,
    #[serde(default)]
    h: bool,
    #[serde(default)]
    x: bool,
}
impl StoredEvent {
    fn into_event(self) -> Event {
        Event {
            id: self.id,
            kind: match self.k.as_str() {
                "r" => EventKind::Rest,
                "c" => EventKind::Chord,
                _ => EventKind::Note,
            },
            start: Frac::new(self.s[0], self.s[1].max(1)),
            duration: Frac::new(self.d[0], self.d[1].max(1)),
            heads: self
                .h
                .into_iter()
                .map(|h| Head {
                    id: h.id,
                    pitch: Pitch { step: h.p[0] as i8, alter: h.p[1] as i8, octave: h.p[2] as i8 },
                    tie: h.t != 0,
                    velocity: h.v as u8,
                })
                .collect(),
            articulations: self.a,
            dynamic: self.dynamic,
            spans: self.sp,
            lyrics: self.ly.into_iter().map(|l| Lyric { verse: l.v.max(1), text: l.t, hyphen: l.h, extend: l.x }).collect(),
        }
    }
}

fn event_json(e: &Event) -> Value {
    let mut o = json!({
        "id": e.id,
        "k": match e.kind { EventKind::Rest => "r", EventKind::Chord => "c", EventKind::Note => "n" },
        "s": jfrac(e.start),
        "d": jfrac(e.duration),
        "a": e.articulations,
        "h": e.heads.iter().map(|h| json!({
            "id": h.id, "p": [h.pitch.step, h.pitch.alter, h.pitch.octave], "t": h.tie as i32, "v": h.velocity
        })).collect::<Vec<_>>(),
    });
    if let Some(d) = e.dynamic {
        o["dyn"] = json!(d);
    }
    if e.spans != 0 {
        o["sp"] = json!(e.spans);
    }
    if !e.lyrics.is_empty() {
        o["ly"] = Value::Array(
            e.lyrics
                .iter()
                .map(|l| {
                    let mut x = json!({"v": l.verse, "t": l.text});
                    if l.hyphen {
                        x["h"] = json!(true);
                    }
                    if l.extend {
                        x["x"] = json!(true);
                    }
                    x
                })
                .collect(),
        );
    }
    o
}

/// Writes the whole project. Rows that no longer exist are removed.
pub fn save(project: &Project, path: &Path) -> Result<()> {
    let mut db = open_db(path)?;
    let tx = db.transaction()?;
    tx.execute(
        "INSERT INTO project(id, title, composer, sample_rate, next_id) VALUES (1, ?, ?, ?, ?) \
         ON CONFLICT(id) DO UPDATE SET title=excluded.title, composer=excluded.composer, \
         sample_rate=excluded.sample_rate, next_id=excluded.next_id",
        params![project.title, project.composer, project.sample_rate, project.next_id as i64],
    )?;
    let ids = |v: Vec<u64>| serde_json::to_string(&v).unwrap();
    // The audio graph. Upserts, not REPLACE: replacing a node row would
    // delete it first and cascade to its state and edges.
    for n in &project.nodes {
        let kind = match n.kind {
            NodeKind::Clap => "clap",
            NodeKind::Bus => "bus",
            NodeKind::Master => "master",
        };
        let opt = |s: &str| (!s.is_empty()).then(|| s.to_string());
        tx.execute(
            "INSERT INTO audio_node(id, kind, name, clap_bundle, clap_plugin_id, bypassed) VALUES (?,?,?,?,?,?) \
             ON CONFLICT(id) DO UPDATE SET kind=excluded.kind, name=excluded.name, clap_bundle=excluded.clap_bundle, \
             clap_plugin_id=excluded.clap_plugin_id, bypassed=excluded.bypassed",
            params![n.id as i64, kind, n.name, opt(&n.bundle), opt(&n.plugin_id), n.bypassed as i64],
        )?;
    }
    tx.execute("DELETE FROM audio_node WHERE id NOT IN (SELECT value FROM json_each(?))", [ids(project.nodes.iter().map(|n| n.id).collect())])?;
    tx.execute("DELETE FROM audio_edge", [])?;
    for e in &project.edges {
        tx.execute(
            "INSERT OR IGNORE INTO audio_edge(from_node, from_port, to_node, to_port, gain) VALUES (?,?,?,?,?)",
            params![e.from as i64, e.from_port, e.to as i64, e.to_port, e.gain],
        )?;
    }
    for (id, state) in &project.plugin_state {
        if project.node(*id).is_none() {
            continue;
        }
        let hash = fnv1a(state).to_le_bytes().to_vec();
        tx.execute(
            "INSERT INTO plugin_state(node_id, state, state_hash) VALUES (?,?,?) ON CONFLICT(node_id) DO UPDATE SET \
             state=excluded.state, state_hash=excluded.state_hash, saved_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') \
             WHERE plugin_state.state_hash IS NOT excluded.state_hash",
            params![*id as i64, state.as_slice(), hash],
        )?;
    }
    tx.execute("DELETE FROM plugin_state WHERE node_id NOT IN (SELECT value FROM json_each(?))", [ids(project.plugin_state.keys().copied().collect())])?;
    for (ord, t) in project.tracks.iter().enumerate() {
        let node = t.instrument_node.filter(|n| node_exists(&tx, *n));
        let bus = t.output_bus.filter(|n| node_exists(&tx, *n));
        tx.execute(
            "INSERT INTO track(id, ordinal, name, instrument_node_id, note_port, midi_channel, gain_db, pan, mute, solo, output_bus_id) \
             VALUES (?,?,?,?,?,?,?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET ordinal=excluded.ordinal, name=excluded.name, \
             instrument_node_id=excluded.instrument_node_id, note_port=excluded.note_port, midi_channel=excluded.midi_channel, \
             gain_db=excluded.gain_db, pan=excluded.pan, mute=excluded.mute, solo=excluded.solo, output_bus_id=excluded.output_bus_id",
            params![t.id as i64, ord as i64, t.name, node.map(|v| v as i64), t.note_port, t.midi_channel, t.gain_db, t.pan, t.mute as i64, t.solo as i64, bus.map(|v| v as i64)],
        )?;
    }
    tx.execute("DELETE FROM track WHERE id NOT IN (SELECT value FROM json_each(?))", [ids(project.tracks.iter().map(|t| t.id).collect())])?;
    for (ord, pl) in project.players.iter().enumerate() {
        tx.execute(
            "INSERT INTO player(id, ordinal, name) VALUES (?,?,?) ON CONFLICT(id) DO UPDATE SET ordinal=excluded.ordinal, name=excluded.name",
            params![pl.id as i64, ord as i64, pl.name],
        )?;
        for (iord, iid) in pl.instruments.iter().enumerate() {
            let Some(i) = project.instrument(*iid) else { continue };
            tx.execute(
                "INSERT INTO instrument(id, player_id, ordinal, name, engraver_name, transposition, clef, track_id) VALUES (?,?,?,?,?,?,?,?) \
                 ON CONFLICT(id) DO UPDATE SET player_id=excluded.player_id, ordinal=excluded.ordinal, name=excluded.name, \
                 engraver_name=excluded.engraver_name, transposition=excluded.transposition, clef=excluded.clef, track_id=excluded.track_id",
                params![i.id as i64, pl.id as i64, iord as i64, i.name, i.engraver_name, i.transposition, i.clef, i.track.map(|v| v as i64)],
            )?;
        }
    }
    tx.execute("DELETE FROM instrument WHERE id NOT IN (SELECT value FROM json_each(?))", [ids(project.instruments.iter().map(|i| i.id).collect())])?;
    tx.execute("DELETE FROM player WHERE id NOT IN (SELECT value FROM json_each(?))", [ids(project.players.iter().map(|p| p.id).collect())])?;

    for m in &project.movements {
        let arr = |items: Vec<Value>| Value::Array(items).to_string();
        let tempo = arr(m.tempo.iter().map(|t| {
            let mut o = json!({"at": jfrac(t.at), "bpm": t.bpm, "ramp": t.ramp});
            if !t.text.is_empty() {
                o["text"] = json!(t.text);
            }
            o
        }).collect());
        let meter = arr(m.meter.iter().map(|t| json!({"at": jfrac(t.at), "num": t.num, "den": t.den})).collect());
        let keys = arr(m.keys.iter().map(|k| json!({"at": jfrac(k.at), "fifths": k.fifths, "minor": k.minor})).collect());
        let mut clefs = Vec::new();
        let mut texts = Vec::new();
        let mut chords = Vec::new();
        for l in &m.lanes {
            chords.extend(l.chords.iter().map(|c| json!({"id": c.id, "p": l.player, "at": jfrac(c.at), "c": c.text})));
            clefs.extend(l.clefs.iter().map(|c| json!({"id": c.id, "p": l.player, "at": jfrac(c.at), "c": c.clef})));
            texts.extend(l.texts.iter().map(|t| json!({
                "id": t.id, "p": l.player, "at": jfrac(t.at), "t": t.text,
                "k": if t.kind == TextKind::Technique { "t" } else { "x" }
            })));
        }
        let notation = json!({
            "barlines": m.barlines.iter().map(|b| json!({"id": b.id, "at": jfrac(b.at), "k": b.kind.code()})).collect::<Vec<_>>(),
            "marks": m.rehearsal.iter().map(|r| json!({"id": r.id, "at": jfrac(r.at)})).collect::<Vec<_>>(),
            "endings": m.endings.iter().map(|e| json!({"id": e.id, "from": jfrac(e.from), "to": jfrac(e.to), "n": e.passes})).collect::<Vec<_>>(),
            "clefs": clefs,
            "texts": texts,
            "chords": chords,
        })
        .to_string();
        tx.execute(
            "INSERT INTO movement(id, ordinal, title, length_num, length_den, revision, tempo, meter, keys, notation) \
             VALUES (?,?,?,?,?,?,jsonb(?),jsonb(?),jsonb(?),jsonb(?)) ON CONFLICT(id) DO UPDATE SET ordinal=excluded.ordinal, \
             title=excluded.title, length_num=excluded.length_num, length_den=excluded.length_den, revision=excluded.revision, \
             tempo=excluded.tempo, meter=excluded.meter, keys=excluded.keys, notation=excluded.notation",
            params![m.id as i64, m.ordinal, m.title, m.length.num, m.length.den, m.revision as i64, tempo, meter, keys, notation],
        )?;
        tx.execute("DELETE FROM player_lane WHERE movement_id = ? AND player_id NOT IN (SELECT value FROM json_each(?))", params![m.id as i64, ids(m.lanes.iter().map(|l| l.player).collect())])?;
        for (ord, l) in m.lanes.iter().enumerate() {
            tx.execute(
                "INSERT INTO player_lane(movement_id, player_id, ordinal) VALUES (?,?,?) ON CONFLICT(movement_id, player_id) DO UPDATE SET ordinal=excluded.ordinal",
                params![m.id as i64, l.player as i64, ord as i64],
            )?;
            tx.execute("DELETE FROM instrument_change WHERE movement_id = ? AND player_id = ?", params![m.id as i64, l.player as i64])?;
            for c in &l.changes {
                tx.execute(
                    "INSERT INTO instrument_change(id, movement_id, player_id, at_num, at_den, instrument_id) VALUES (?,?,?,?,?,?)",
                    params![c.id as i64, m.id as i64, l.player as i64, c.at.num, c.at.den, c.instrument as i64],
                )?;
            }
            tx.execute(
                "DELETE FROM voice WHERE movement_id = ? AND player_id = ? AND id NOT IN (SELECT value FROM json_each(?))",
                params![m.id as i64, l.player as i64, ids(l.voices.iter().map(|v| v.id).collect())],
            )?;
            for v in &l.voices {
                tx.execute(
                    "INSERT INTO voice(id, movement_id, player_id, voice_index) VALUES (?,?,?,?) ON CONFLICT(id) DO UPDATE SET voice_index=excluded.voice_index",
                    params![v.id as i64, m.id as i64, l.player as i64, v.index],
                )?;
                tx.execute("DELETE FROM voice_chunk WHERE voice_id = ?", [v.id as i64])?;
                let mut i = 0;
                while i < v.events.len() {
                    let ci = chunk_of(v.events[i].start);
                    let mut j = i;
                    while j < v.events.len() && chunk_of(v.events[j].start) == ci {
                        j += 1;
                    }
                    let chunk = Value::Array(v.events[i..j].iter().map(event_json).collect()).to_string();
                    tx.execute("INSERT INTO voice_chunk(voice_id, chunk_index, events) VALUES (?,?,jsonb(?))", params![v.id as i64, ci, chunk])?;
                    i = j;
                }
            }
        }
    }
    tx.execute("DELETE FROM movement WHERE id NOT IN (SELECT value FROM json_each(?))", [ids(project.movements.iter().map(|m| m.id).collect())])?;
    tx.commit()?;
    Ok(())
}

/// 64-bit FNV-1a, for plugin_state.state_hash (skip rewriting unchanged state).
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ *b as u64).wrapping_mul(0x0100_0000_01b3))
}

fn node_exists(tx: &rusqlite::Transaction, id: u64) -> bool {
    tx.query_row("SELECT 1 FROM audio_node WHERE id = ?", [id as i64], |_| Ok(())).optional().ok().flatten().is_some()
}
