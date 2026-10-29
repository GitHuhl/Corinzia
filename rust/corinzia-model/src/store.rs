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
    // Layouts and page texts.
    let setting = |key: &str| -> Result<Option<Value>> {
        let v: Option<String> = db.query_row("SELECT json(value) FROM setting WHERE key = ?", [key], |r| r.get(0)).optional()?;
        Ok(match v {
            Some(v) => Some(serde_json::from_str(&v)?),
            None => None,
        })
    };
    if let Some(v) = setting("layout")? {
        p.layout = layout_from(&v, Layout::default());
    }
    p.part_layout = match setting("part_layout")? {
        Some(v) => layout_from(&v, Layout::parts()),
        None => Layout::parts(),
    };
    if let Some(t) = setting("texts")? {
        let s = |k: &str| t[k].as_str().unwrap_or("").to_string();
        (p.subtitle, p.lyricist, p.arranger, p.copyright) = (s("subtitle"), s("lyricist"), s("arranger"), s("copyright"));
    }
    {
        let mut s = db.prepare("SELECT id, kind, name, clap_bundle, clap_plugin_id, bypassed FROM audio_node ORDER BY id")?;
        let rows = s.query_map([], |r| {
            Ok(AudioNode {
                id: r.get::<_, i64>(0)? as u64,
                kind: match r.get::<_, String>(1)?.as_str() {
                    "bus" => NodeKind::Bus,
                    "master" => NodeKind::Master,
                    _ => NodeKind::Plugin,
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
                inserts: vec![],
                sends: vec![],
                dynamics_cc: None,
                program: None,
                bank: None,
                techniques: vec![],
                color: None,
                folder: String::new(),
                audio: false,
                clips: vec![],
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
            "SELECT id, player_id, name, engraver_name, transposition_steps, transposition_semitones, clef, track_id FROM instrument ORDER BY player_id, ordinal",
        )?;
        let rows = s.query_map([], |r| {
            Ok(Instrument {
                id: r.get::<_, i64>(0)? as u64,
                player: r.get::<_, i64>(1)? as u64,
                name: r.get(2)?,
                engraver_name: r.get(3)?,
                transposition: Interval::new(r.get::<_, i64>(4)? as i32, r.get::<_, i64>(5)? as i32),
                clef: r.get(6)?,
                track: r.get::<_, Option<i64>>(7)?.map(|v| v as u64), drums: vec![], tuning: vec![],
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
    // Percussion maps, by instrument.
    if let Some(d) = setting("drums")? {
        for i in p.instruments.iter_mut() {
            for s in d[i.id.to_string()].as_array().into_iter().flatten() {
                i.drums.push(DrumSound {
                    name: s["name"].as_str().unwrap_or("").to_string(),
                    position: Pitch { step: s["step"].as_i64().unwrap_or(0) as i8, alter: 0, octave: s["octave"].as_i64().unwrap_or(5) as i8 },
                    key: s["key"].as_u64().unwrap_or(38).min(127) as u8,
                    head: Notehead::from_code(s["head"].as_str().unwrap_or("normal")),
                });
            }
        }
    }

    // The mixer: effects, sends, buses and the master.
    if let Some(m) = setting("mixer")? {
        let ids = |v: &Value| -> Vec<Id> { v.as_array().into_iter().flatten().filter_map(|x| x.as_u64()).collect() };
        for t in p.tracks.iter_mut() {
            let tv = &m["tracks"][t.id.to_string()];
            t.inserts = ids(&tv["inserts"]);
            t.techniques = tv["tech"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|x| {
                    let v = x["v"].as_u64().unwrap_or(0).min(127) as u8;
                    let action = match x["a"].as_str()? {
                        "key" => SwitchAction::Keyswitch(v),
                        "prog" => SwitchAction::Program(v),
                        "cc" => SwitchAction::Cc(v, x["w"].as_u64().unwrap_or(0).min(127) as u8),
                        "ch" => SwitchAction::Channel(v.min(15)),
                        _ => return None,
                    };
                    Some(TechniqueSwitch { technique: x["t"].as_str()?.to_string(), action })
                })
                .collect();
            t.dynamics_cc = tv["dyn_cc"].as_u64().map(|v| v.min(127) as u8);
            t.program = tv["program"].as_u64().map(|v| v.min(127) as u8);
            t.bank = tv["bank"].as_u64().map(|v| v.min(16383) as u16);
            t.sends = tv["sends"].as_array().into_iter().flatten().filter_map(|s| Some(SendLevel { bus: s["bus"].as_u64()?, gain_db: s["db"].as_f64().unwrap_or(0.0), pre: s["pre"].as_bool().unwrap_or(false) })).collect();
            t.audio = tv["audio"].as_bool().unwrap_or(false);
            t.color = tv["color"].as_u64().map(|c| c as u32 & 0xff_ffff);
            t.folder = tv["folder"].as_str().unwrap_or("").to_string();
            t.clips = tv["clips"].as_array().into_iter().flatten().filter_map(clip_from).collect();
        }
        p.buses = m["buses"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|b| {
                Some(Bus {
                    id: b["id"].as_u64()?,
                    name: b["name"].as_str().unwrap_or("Bus").to_string(),
                    inserts: ids(&b["inserts"]),
                    gain_db: b["db"].as_f64().unwrap_or(0.0),
                    pan: b["pan"].as_f64().unwrap_or(0.0),
                    mute: b["mute"].as_bool().unwrap_or(false),
                    solo: b["solo"].as_bool().unwrap_or(false),
                    output: b["out"].as_u64(),
                })
            })
            .collect();
        p.master = Master { inserts: ids(&m["master"]["inserts"]), gain_db: m["master"]["db"].as_f64().unwrap_or(0.0) };
    }
    // Fretted instruments' tunings, by instrument.
    if let Some(t) = setting("tunings")? {
        for i in p.instruments.iter_mut() {
            i.tuning = t[i.id.to_string()].as_array().into_iter().flatten().filter_map(|k| k.as_u64()).map(|k| k.min(127) as u8).collect();
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
            navigation: vec![],
            slurs: vec![],
            endings: vec![],
            lanes: vec![], breaks: vec![], cross: vec![], automation: vec![],
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
            m.meter.push(Meter { at: frac(&t["at"]), num: t["num"].as_i64().unwrap_or(4), den: t["den"].as_i64().unwrap_or(4), pickup: t["pickup"].as_bool().unwrap_or(false), beams: {
                let mut b = [0u8; 8];
                for (d, v) in b.iter_mut().zip(t["beams"].as_array().into_iter().flatten()) {
                    *d = v.as_u64().unwrap_or(0).min(64) as u8;
                }
                b
            } });
        }
        for t in serde_json::from_str::<Vec<Value>>(&keys)? {
            m.keys.push(Key { at: frac(&t["at"]), fifths: t["fifths"].as_i64().unwrap_or(0) as i32, minor: t["minor"].as_bool().unwrap_or(false) });
        }
        // Lanes, instrument changes, voices.
        let mut sl = db.prepare("SELECT lane_index, player_id FROM lane WHERE movement_id = ? ORDER BY lane_index")?;
        let lanes: Vec<(i64, i64)> = sl.query_map([id], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<std::result::Result<_, _>>()?;
        for (li, pid) in lanes {
            let mut lane = Lane { player: pid as u64, changes: vec![], clefs: vec![], texts: vec![], chords: vec![], lines: vec![], voices: vec![] };
            let mut sc = db.prepare(
                "SELECT id, at_num, at_den, instrument_id FROM instrument_change WHERE movement_id = ? AND lane_index = ? \
                 ORDER BY at_num * 1.0 / at_den",
            )?;
            for c in sc.query_map(params![id, li], |r| {
                Ok(InstrumentChange {
                    id: r.get::<_, i64>(0)? as u64,
                    at: Frac::new(r.get(1)?, r.get::<_, i64>(2)?.max(1)),
                    instrument: r.get::<_, i64>(3)? as u64,
                })
            })? {
                lane.changes.push(c?);
            }
            let mut sv = db.prepare("SELECT id, voice_index, json(tuplets) FROM voice WHERE movement_id = ? AND lane_index = ? ORDER BY voice_index")?;
            let voices: Vec<(i64, i64, String)> = sv.query_map(params![id, li], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<std::result::Result<_, _>>()?;
            for (vid, index, tuplets) in voices {
                let tuplets: Vec<Tuplet> = serde_json::from_str::<Vec<Value>>(&tuplets)?
                    .iter()
                    .map(|t| Tuplet { start: frac(&t["s"]), length: frac(&t["l"]), actual: t["a"].as_i64().unwrap_or(3), normal: t["n"].as_i64().unwrap_or(2) })
                    .collect();
                let mut sch = db.prepare("SELECT json(events) FROM voice_chunk WHERE voice_id = ? ORDER BY chunk_index")?;
                let chunks: Vec<String> = sch.query_map([vid], |r| r.get(0))?.collect::<std::result::Result<_, _>>()?;
                let mut events = Vec::new();
                for c in chunks {
                    for e in serde_json::from_str::<Vec<StoredEvent>>(&c)? {
                        events.push(e.into_event());
                    }
                }
                lane.voices.push(Voice { id: vid as u64, index: index as u8, events: Arc::new(events), tuplets });
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
            for r in n["nav"].as_array().into_iter().flatten() {
                if let Some(kind) = r["k"].as_str().and_then(Nav::from_code) {
                    m.navigation.push(NavMark { id: r["id"].as_u64().unwrap_or(0), at: frac(&r["at"]), kind });
                }
            }
            for s in n["slurs"].as_array().into_iter().flatten() {
                let id = |k: &str| s[k].as_u64().unwrap_or(0);
                m.slurs.push(Slur { id: id("id"), from: id("from"), to: id("to"), dir: s["dir"].as_i64().unwrap_or(0) as i8 });
            }
            for b in n["breaks"].as_array().into_iter().flatten() {
                m.breaks.push(Break { id: b["id"].as_u64().unwrap_or(0), at: frac(&b["at"]), page: b["page"].as_bool().unwrap_or(false), part: b["part"].as_u64() });
            }
            for a in n["automation"].as_array().into_iter().flatten() {
                let id = |k: &str| a[k].as_u64().unwrap_or(0);
                let target = match a["t"].as_str() {
                    Some("gain") => AutoTarget::Gain(id("track")),
                    Some("pan") => AutoTarget::Pan(id("track")),
                    _ => AutoTarget::Param { node: id("node"), param: id("param") as u32 },
                };
                let points = a["points"].as_array().into_iter().flatten().filter_map(|p| Some(AutoPoint { at: corinzia_score::Frac::new(p[0].as_i64()?, p[1].as_i64()?.max(1)), value: p[2].as_f64()? })).collect();
                m.automation.push(AutoLane { id: id("id"), target, points });
            }
            for c in n["cross"].as_array().into_iter().flatten() {
                m.cross.push((c["e"].as_u64().unwrap_or(0), c["d"].as_i64().unwrap_or(0) as i8));
            }
            for e in n["endings"].as_array().into_iter().flatten() {
                m.endings.push(Ending { id: e["id"].as_u64().unwrap_or(0), from: frac(&e["from"]), to: frac(&e["to"]), passes: e["n"].as_u64().unwrap_or(1) as u8 });
            }
            // Per-lane items name their lane by index (a player's staves
            // share the player id).
            fn lane_of<'a>(lanes: &'a mut [Lane], v: &Value) -> Option<&'a mut Lane> {
                match v["li"].as_u64() {
                    Some(i) => lanes.get_mut(i as usize),
                    None => {
                        let pid = v["p"].as_u64().unwrap_or(0);
                        lanes.iter_mut().find(|l| l.player == pid)
                    }
                }
            }
            for c in n["clefs"].as_array().into_iter().flatten() {
                if let Some(l) = lane_of(&mut m.lanes, c) {
                    l.clefs.push(ClefChange { id: c["id"].as_u64().unwrap_or(0), at: frac(&c["at"]), clef: c["c"].as_str().unwrap_or("treble").into() });
                }
            }
            for t in n["texts"].as_array().into_iter().flatten() {
                if let Some(l) = lane_of(&mut m.lanes, t) {
                    l.texts.push(TextItem {
                        id: t["id"].as_u64().unwrap_or(0),
                        at: frac(&t["at"]),
                        text: t["t"].as_str().unwrap_or("").into(),
                        kind: if t["k"].as_str() == Some("t") { TextKind::Technique } else { TextKind::Expression },
                        shown: match t["s"].as_str() {
                            Some("s") => Shown::Score,
                            Some("p") => Shown::Parts,
                            _ => Shown::Both,
                        },
                    });
                }
            }
            for c in n["chords"].as_array().into_iter().flatten() {
                if let Some(l) = lane_of(&mut m.lanes, c) {
                    l.chords.push(ChordSymbol { id: c["id"].as_u64().unwrap_or(0), at: frac(&c["at"]), text: c["c"].as_str().unwrap_or("").into() });
                }
            }
            for c in n["lines"].as_array().into_iter().flatten() {
                if let (Some(l), Some(kind)) = (lane_of(&mut m.lanes, c), c["k"].as_str().and_then(LineKind::from_code)) {
                    l.lines.push(LineSpan { id: c["id"].as_u64().unwrap_or(0), from: frac(&c["from"]), to: frac(&c["to"]), kind });
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
    #[serde(default)]
    g: Vec<StoredGrace>,
    #[serde(default)]
    fg: Vec<u8>,
}
#[derive(Deserialize)]
struct StoredGrace {
    id: u64,
    #[serde(default)]
    h: Vec<StoredHead>,
    #[serde(default)]
    b: u8,
    #[serde(default)]
    sl: bool,
}
impl StoredHead {
    fn head(&self) -> Head {
        Head { id: self.id, pitch: Pitch { step: self.p[0] as i8, alter: self.p[1] as i8, octave: self.p[2] as i8 }, tie: self.t != 0, velocity: self.v as u8 }
    }
}
fn head_json(h: &Head) -> Value {
    json!({"id": h.id, "p": [h.pitch.step, h.pitch.alter, h.pitch.octave], "t": h.tie as i32, "v": h.velocity})
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
            heads: self.h.iter().map(StoredHead::head).collect(),
            articulations: self.a,
            dynamic: self.dynamic,
            spans: self.sp,
            lyrics: self.ly.into_iter().map(|l| Lyric { verse: l.v.max(1), text: l.t, hyphen: l.h, extend: l.x }).collect(),
            fingering: self.fg,
            graces: self.g.iter().map(|g| Grace { id: g.id, heads: g.h.iter().map(StoredHead::head).collect(), base: if g.b == 0 { 8 } else { g.b }, slash: g.sl }).collect(),
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
        "h": e.heads.iter().map(head_json).collect::<Vec<_>>(),
    });
    if !e.fingering.is_empty() {
        o["fg"] = json!(e.fingering);
    }
    if !e.graces.is_empty() {
        o["g"] = Value::Array(e.graces.iter().map(|g| json!({"id": g.id, "h": g.heads.iter().map(head_json).collect::<Vec<_>>(), "b": g.base, "sl": g.slash})).collect());
    }
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

fn layout_from(v: &Value, d: Layout) -> Layout {
    let f = |k: &str, d: f64| v[k].as_f64().unwrap_or(d);
    Layout {
        page_width_mm: f("page_width_mm", d.page_width_mm),
        page_height_mm: f("page_height_mm", d.page_height_mm),
        landscape: v["landscape"].as_bool().unwrap_or(d.landscape),
        staff_mm: f("staff_mm", d.staff_mm),
        margin_y_mm: f("margin_y_mm", d.margin_y_mm),
        margin_x_mm: f("margin_x_mm", d.margin_x_mm),
        spacing: f("spacing", d.spacing),
        staff_distance: f("staff_distance", d.staff_distance),
        system_gap: f("system_gap", d.system_gap),
        bars_per_system: v["bars_per_system"].as_u64().unwrap_or(d.bars_per_system as u64) as u32,
        page_numbers: v["page_numbers"].as_bool().unwrap_or(d.page_numbers),
        condense: v["condense"].as_bool().unwrap_or(d.condense),
    }
}

fn layout_json(l: &Layout) -> Value {
    json!({
        "page_width_mm": l.page_width_mm, "page_height_mm": l.page_height_mm, "landscape": l.landscape,
        "staff_mm": l.staff_mm, "margin_y_mm": l.margin_y_mm, "margin_x_mm": l.margin_x_mm,
        "spacing": l.spacing, "staff_distance": l.staff_distance, "system_gap": l.system_gap,
        "bars_per_system": l.bars_per_system, "page_numbers": l.page_numbers, "condense": l.condense,
    })
}

/// Writes the whole project. Rows that no longer exist are removed.
fn clip_from(c: &Value) -> Option<AudioClip> {
    let f = |k: &str| c[k].as_f64().unwrap_or(0.0);
    Some(AudioClip {
        id: c["id"].as_u64()?,
        name: c["name"].as_str().unwrap_or("").to_string(),
        file: c["file"].as_str()?.to_string(),
        movement: c["movement"].as_u64()?,
        at: f("at"),
        offset: f("offset"),
        length: f("length"),
        fade_in: f("fade_in"),
        fade_out: f("fade_out"),
        gain_db: f("db"),
        mute: c["mute"].as_bool().unwrap_or(false),
        cue: c["cue"].as_u64(),
        cue_line: c["cue_line"].as_bool().unwrap_or(false),
    })
}

fn clip_json(c: &AudioClip) -> Value {
    json!({"id": c.id, "name": c.name, "file": c.file, "movement": c.movement, "at": c.at, "offset": c.offset, "length": c.length,
           "fade_in": c.fade_in, "fade_out": c.fade_out, "db": c.gain_db, "mute": c.mute, "cue": c.cue, "cue_line": c.cue_line})
}

pub fn save(project: &Project, path: &Path) -> Result<()> {
    let mut db = open_db(path)?;
    let tx = db.transaction()?;
    tx.execute(
        "INSERT INTO project(id, title, composer, sample_rate, next_id) VALUES (1, ?, ?, ?, ?) \
         ON CONFLICT(id) DO UPDATE SET title=excluded.title, composer=excluded.composer, \
         sample_rate=excluded.sample_rate, next_id=excluded.next_id",
        params![project.title, project.composer, project.sample_rate, project.next_id as i64],
    )?;
    let texts = json!({"subtitle": project.subtitle, "lyricist": project.lyricist, "arranger": project.arranger, "copyright": project.copyright});
    let drums: serde_json::Map<String, Value> = project
        .instruments
        .iter()
        .filter(|i| !i.drums.is_empty())
        .map(|i| (i.id.to_string(), json!(i.drums.iter().map(|d| json!({"name": d.name, "step": d.position.step, "octave": d.position.octave, "key": d.key, "head": d.head.code()})).collect::<Vec<_>>())))
        .collect();
    let mixer = json!({
        "tracks": project.tracks.iter().filter(|t| !t.inserts.is_empty() || !t.sends.is_empty() || t.program.is_some() || t.dynamics_cc.is_some() || !t.techniques.is_empty() || t.audio || t.color.is_some() || !t.folder.is_empty()).map(|t| (t.id.to_string(), json!({"tech": t.techniques.iter().map(|x| match x.action {
            SwitchAction::Keyswitch(k) => json!({"t": x.technique, "a": "key", "v": k}),
            SwitchAction::Program(p) => json!({"t": x.technique, "a": "prog", "v": p}),
            SwitchAction::Cc(c, v) => json!({"t": x.technique, "a": "cc", "v": c, "w": v}),
            SwitchAction::Channel(c) => json!({"t": x.technique, "a": "ch", "v": c}),
        }).collect::<Vec<_>>(), "dyn_cc": t.dynamics_cc, "inserts": t.inserts, "sends": t.sends.iter().map(|s| json!({"bus": s.bus, "db": s.gain_db, "pre": s.pre})).collect::<Vec<_>>(), "program": t.program, "bank": t.bank, "color": t.color, "folder": t.folder, "audio": t.audio, "clips": t.clips.iter().map(clip_json).collect::<Vec<_>>()}))).collect::<serde_json::Map<String, Value>>(),
        "buses": project.buses.iter().map(|b| json!({"id": b.id, "name": b.name, "inserts": b.inserts, "db": b.gain_db, "pan": b.pan, "mute": b.mute, "solo": b.solo, "out": b.output})).collect::<Vec<_>>(),
        "master": {"inserts": project.master.inserts, "db": project.master.gain_db},
    });
    let tunings: serde_json::Map<String, Value> = project.instruments.iter().filter(|i| !i.tuning.is_empty()).map(|i| (i.id.to_string(), json!(i.tuning))).collect();
    for (key, value) in [("layout", layout_json(&project.layout)), ("part_layout", layout_json(&project.part_layout)), ("texts", texts), ("drums", Value::Object(drums)), ("tunings", Value::Object(tunings)), ("mixer", mixer)] {
        tx.execute("INSERT INTO setting(key, value) VALUES (?, jsonb(?)) ON CONFLICT(key) DO UPDATE SET value = excluded.value", params![key, value.to_string()])?;
    }
    let ids = |v: Vec<u64>| serde_json::to_string(&v).unwrap();
    // The audio graph. Upserts, not REPLACE: replacing a node row would
    // delete it first and cascade to its state and edges.
    for n in &project.nodes {
        let kind = match n.kind {
            NodeKind::Plugin => "clap",
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
                "INSERT INTO instrument(id, player_id, ordinal, name, engraver_name, transposition_steps, transposition_semitones, clef, track_id) \
                 VALUES (?,?,?,?,?,?,?,?,?) \
                 ON CONFLICT(id) DO UPDATE SET player_id=excluded.player_id, ordinal=excluded.ordinal, name=excluded.name, \
                 engraver_name=excluded.engraver_name, transposition_steps=excluded.transposition_steps, \
                 transposition_semitones=excluded.transposition_semitones, clef=excluded.clef, track_id=excluded.track_id",
                params![
                    i.id as i64,
                    pl.id as i64,
                    iord as i64,
                    i.name,
                    i.engraver_name,
                    i.transposition.steps,
                    i.transposition.semitones,
                    i.clef,
                    i.track.map(|v| v as i64)
                ],
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
        let meter = arr(m
            .meter
            .iter()
            .map(|t| {
                let mut j = json!({"at": jfrac(t.at), "num": t.num, "den": t.den, "pickup": t.pickup});
                if t.beams[0] > 0 {
                    j["beams"] = json!(t.beams.iter().take_while(|b| **b > 0).collect::<Vec<_>>());
                }
                j
            })
            .collect());
        let keys = arr(m.keys.iter().map(|k| json!({"at": jfrac(k.at), "fifths": k.fifths, "minor": k.minor})).collect());
        let mut clefs = Vec::new();
        let mut texts = Vec::new();
        let mut chords = Vec::new();
        let mut lines = Vec::new();
        for (li, l) in m.lanes.iter().enumerate() {
            chords.extend(l.chords.iter().map(|c| json!({"id": c.id, "li": li, "at": jfrac(c.at), "c": c.text})));
            clefs.extend(l.clefs.iter().map(|c| json!({"id": c.id, "li": li, "at": jfrac(c.at), "c": c.clef})));
            texts.extend(l.texts.iter().map(|t| json!({
                "id": t.id, "li": li, "at": jfrac(t.at), "t": t.text,
                "k": if t.kind == TextKind::Technique { "t" } else { "x" },
                "s": match t.shown { Shown::Both => "", Shown::Score => "s", Shown::Parts => "p" }
            })));
            lines.extend(l.lines.iter().map(|x| json!({"id": x.id, "li": li, "from": jfrac(x.from), "to": jfrac(x.to), "k": x.kind.code()})));
        }
        let notation = json!({
            "barlines": m.barlines.iter().map(|b| json!({"id": b.id, "at": jfrac(b.at), "k": b.kind.code()})).collect::<Vec<_>>(),
            "marks": m.rehearsal.iter().map(|r| json!({"id": r.id, "at": jfrac(r.at)})).collect::<Vec<_>>(),
            "nav": m.navigation.iter().map(|r| json!({"id": r.id, "at": jfrac(r.at), "k": r.kind.code()})).collect::<Vec<_>>(),
            "endings": m.endings.iter().map(|e| json!({"id": e.id, "from": jfrac(e.from), "to": jfrac(e.to), "n": e.passes})).collect::<Vec<_>>(),
            "slurs": m.slurs.iter().map(|s| json!({"id": s.id, "from": s.from, "to": s.to, "dir": s.dir})).collect::<Vec<_>>(),
            "automation": m.automation.iter().map(|a| {
                let points: Vec<Value> = a.points.iter().map(|p| json!([p.at.num, p.at.den, p.value])).collect();
                match a.target {
                    AutoTarget::Gain(t) => json!({"id": a.id, "t": "gain", "track": t, "points": points}),
                    AutoTarget::Pan(t) => json!({"id": a.id, "t": "pan", "track": t, "points": points}),
                    AutoTarget::Param { node, param } => json!({"id": a.id, "t": "param", "node": node, "param": param, "points": points}),
                }
            }).collect::<Vec<_>>(),
            "cross": m.cross.iter().map(|c| json!({"e": c.0, "d": c.1})).collect::<Vec<_>>(),
            "breaks": m.breaks.iter().map(|b| json!({"id": b.id, "at": jfrac(b.at), "page": b.page, "part": b.part})).collect::<Vec<_>>(),
            "clefs": clefs,
            "texts": texts,
            "chords": chords,
            "lines": lines,
        })
        .to_string();
        tx.execute(
            "INSERT INTO movement(id, ordinal, title, length_num, length_den, revision, tempo, meter, keys, notation) \
             VALUES (?,?,?,?,?,?,jsonb(?),jsonb(?),jsonb(?),jsonb(?)) ON CONFLICT(id) DO UPDATE SET ordinal=excluded.ordinal, \
             title=excluded.title, length_num=excluded.length_num, length_den=excluded.length_den, revision=excluded.revision, \
             tempo=excluded.tempo, meter=excluded.meter, keys=excluded.keys, notation=excluded.notation",
            params![m.id as i64, m.ordinal, m.title, m.length.num, m.length.den, m.revision as i64, tempo, meter, keys, notation],
        )?;
        // Voices that are gone (or moved lanes) first, then the lanes past the end.
        let all_voices = ids(m.lanes.iter().flat_map(|l| l.voices.iter().map(|v| v.id)).collect());
        tx.execute("DELETE FROM voice WHERE movement_id = ? AND id NOT IN (SELECT value FROM json_each(?))", params![m.id as i64, all_voices])?;
        tx.execute("DELETE FROM lane WHERE movement_id = ? AND lane_index >= ?", params![m.id as i64, m.lanes.len() as i64])?;
        for (li, l) in m.lanes.iter().enumerate() {
            tx.execute(
                "INSERT INTO lane(movement_id, lane_index, player_id) VALUES (?,?,?) ON CONFLICT(movement_id, lane_index) DO UPDATE SET player_id=excluded.player_id",
                params![m.id as i64, li as i64, l.player as i64],
            )?;
            tx.execute("DELETE FROM instrument_change WHERE movement_id = ? AND lane_index = ?", params![m.id as i64, li as i64])?;
            for c in &l.changes {
                tx.execute(
                    "INSERT INTO instrument_change(id, movement_id, lane_index, at_num, at_den, instrument_id) VALUES (?,?,?,?,?,?)",
                    params![c.id as i64, m.id as i64, li as i64, c.at.num, c.at.den, c.instrument as i64],
                )?;
            }
            for v in &l.voices {
                tx.execute(
                    "INSERT INTO voice(id, movement_id, lane_index, voice_index, tuplets) VALUES (?,?,?,?,jsonb(?)) \
                     ON CONFLICT(id) DO UPDATE SET lane_index=excluded.lane_index, voice_index=excluded.voice_index, tuplets=excluded.tuplets",
                    params![
                        v.id as i64,
                        m.id as i64,
                        li as i64,
                        v.index,
                        Value::Array(v.tuplets.iter().map(|t| json!({"s": jfrac(t.start), "l": jfrac(t.length), "a": t.actual, "n": t.normal})).collect()).to_string()
                    ],
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
