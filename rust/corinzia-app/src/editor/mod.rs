//! The score editor: input state (caret, duration, accidental, voice),
//! selection (one element or a passage), clipboard, and undo/redo.
//!
//! Every user action
//! is one undo step: the project is snapshotted before the change (voices are
//! `Arc`-shared, so a snapshot copies little more than the movement headers)
//! and the edit itself goes through the pure `corinzia_edit` functions.
//! Nothing here knows about iced, so it is tested directly.

use corinzia_edit::{self as edit, DurationSpec, EditResult, Ids};
use corinzia_model::*;
use corinzia_score::score::{art, ids, span};
use corinzia_score::Frac;
use std::sync::Arc;

mod input;
mod selection;
mod spanners;
mod tracks;
mod mixer;
mod automation;
mod players;
mod structure;
mod filters;
mod clipboard;
mod page;

/// A passage: staves `lane_from..=lane_to`, time `[from, to)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Range {
    pub lane_from: usize,
    pub lane_to: usize,
    pub from: Frac,
    pub to: Frac,
}

/// A note in the clipboard: (staff offset, voice index, time from the start).
type ClipNote = (usize, u8, Frac);

#[derive(Clone, Debug)]
struct Clip {
    lanes: Vec<Vec<(u8, Vec<Event>, Vec<Tuplet>)>>, // per staff: (voice index, events and tuplets from 0)
    length: Frac,
    /// Slurs within the passage, and their sides.
    slurs: Vec<([ClipNote; 2], i8)>,
    /// The first staff's instrument (named over a cue).
    source: String,
}

const UNDO_LIMIT: usize = 500;

/// The master strip's id in mixer messages and effect chains.
pub const MASTER: Id = corinzia_model::MASTER;

/// Rhythmic grid choices (whole-note fractions) and their labels.
pub const GRIDS: [(i64, i64, &str); 5] = [(1, 4, "1/4"), (1, 8, "1/8"), (1, 16, "1/16"), (1, 32, "1/32"), (1, 12, "1/8T")];

pub struct Editor {
    pub project: Project,
    pub movement: usize,
    pub input_mode: bool,
    pub spec: DurationSpec,
    /// Explicit accidental for the next note; `None` = from the key.
    pub accidental: Option<i8>,
    pub chord_mode: bool,
    pub voice: u8,
    /// The verse lyrics are typed into.
    pub lyric_verse: u8,
    pub caret_lane: usize,
    pub caret: Frac,
    /// The score shows written pitches: letters typed are written pitches
    /// of the caret staff's instrument.
    pub transposed: bool,
    /// The rhythmic grid: Alt+arrows move music by it and the caret steps
    /// by it (an index into `GRIDS`).
    pub grid: usize,
    /// Tuplet input (actual, normal): notes entered go into a tuplet, a new
    /// one starting whenever the caret is outside one.
    pub tuplet_input: Option<(i64, i64)>,
    /// Grace note input: letters add grace notes (slashed if true) before
    /// the note at the caret.
    pub grace_input: Option<bool>,
    pub selection: Option<Id>,
    pub range: Option<Range>,
    /// Elements picked by a filter or Select similar (noteheads, rests,
    /// dynamics, lyrics, chord symbols, text): edits apply to each.
    pub picked: Vec<Id>,
    /// A slur selected (its first event's id); see `selected_slur`.
    slur: Option<Id>,
    /// The fixed end of a passage grown with Shift+arrows (the other end
    /// moves).
    range_anchor: Option<(usize, Frac)>,
    clip: Option<Clip>,
    undo: Vec<(Project, String)>,
    redo: Vec<(Project, String)>,
    /// Bumped on every model change.
    pub revision: u64,
    /// Bumped when the music changes (engraving and the playback plan);
    /// mixer edits leave it alone.
    pub music_revision: u64,
    /// The last commit's merge key: a slider drag is one undo step.
    merge: Option<(String, Id)>,
    /// Unsaved changes.
    pub modified: bool,
    pub status: String,
}

/// Where an element lives in the current movement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Locator {
    pub lane: usize,
    pub voice: usize, // index into lane.voices
}

/// A MIDI key spelled in a key: the scale's own note where the key has
/// one, else a sharp in sharp keys (and C major), a flat in flat keys.
pub fn spell_key(key: i32, fifths: i32) -> Pitch {
    const SEMIS: [i32; 7] = [0, 2, 4, 5, 7, 9, 11];
    let pc = key.rem_euclid(12);
    let octave = key.div_euclid(12) - 1;
    let pitch = |step: i32, alter: i32| {
        // B# and Cb cross the octave line.
        let o = octave + if SEMIS[step as usize] + alter >= 12 { -1 } else if SEMIS[step as usize] + alter < 0 { 1 } else { 0 };
        Pitch { step: step as i8, alter: alter as i8, octave: o as i8 }
    };
    for step in 0..7 {
        let alter = edit::key_alter(fifths, step);
        if (SEMIS[step as usize] + alter).rem_euclid(12) == pc {
            return pitch(step, alter);
        }
    }
    // Outside the scale: the key's own accidental, else natural, else the other.
    let first = if fifths < 0 { -1 } else { 1 };
    for alter in [first, 0, -first] {
        if let Some(step) = (0..7).find(|&s| (SEMIS[s as usize] + alter).rem_euclid(12) == pc) {
            return pitch(step, alter);
        }
    }
    pitch(0, 0)
}

fn key_fifths_at(m: &Movement, pos: Frac) -> i32 {
    m.keys.iter().filter(|k| k.at <= pos).last().map(|k| k.fifths).unwrap_or(0)
}

fn extend_to(m: &mut Movement, end: Frac) {
    if end > m.length {
        m.length = edit::round_up_to_bar(m, end);
    }
}

/// The voice with this index on a lane, created if asked.
fn voice_slot(m: &mut Movement, lane: usize, index: u8, create: bool, ids: &mut Ids) -> Option<usize> {
    let l = m.lanes.get_mut(lane)?;
    if let Some(i) = l.voices.iter().position(|v| v.index == index) {
        return Some(i);
    }
    if !create {
        return None;
    }
    l.voices.push(Voice { id: ids.next(), index, events: Arc::new(Vec::new()), tuplets: vec![] });
    l.voices.sort_by_key(|v| v.index);
    l.voices.iter().position(|v| v.index == index)
}

/// A voice's new events; its tuplets follow (one an event now crosses goes).
fn set_events(m: &mut Movement, lane: usize, voice: usize, r: &EditResult) {
    let v = &mut m.lanes[lane].voices[voice];
    v.events = Arc::new(r.events.clone());
    v.tuplets = edit::tuplets::sanitize(&v.events, &v.tuplets);
}

/// Note input's sounding duration at `caret`: the written value scaled by
/// the tuplets there. With tuplet input on and no tuplet at the caret, a
/// new one (of rests) starts there first.
fn input_duration(m: &mut Movement, lane: usize, vi: usize, caret: Frac, spec: DurationSpec, tuplet: Option<(i64, i64)>, ids: &mut Ids) -> Frac {
    let v = &mut m.lanes[lane].voices[vi];
    if let Some((actual, normal)) = tuplet {
        if edit::tuplets::tuplet_at(&v.tuplets, caret).is_none() {
            let unit = DurationSpec { dots: 0, ..spec }.value();
            if let Some((r, ts)) = edit::tuplets::create(&v.events, &v.tuplets, caret, unit, actual, normal, None, v.index == 1, ids) {
                v.events = Arc::new(r.events);
                v.tuplets = ts;
                extend_to(m, r.to);
            }
        }
    }
    spec.value() * edit::tuplets::sounding_scale(&m.lanes[lane].voices[vi].tuplets, caret)
}

/// After a rhythm edit, voice 1's rests around `[r.from, r.to]` join up
/// within each bar.
fn set_events_tidy(m: &mut Movement, lane: usize, voice: usize, r: &EditResult) {
    let mut evs = r.events.clone();
    if m.lanes[lane].voices[voice].index == 1 {
        // Never across a barline or a tuplet's edge.
        let mut edges = edit::bar_starts(m);
        edges.extend(m.lanes[lane].voices[voice].tuplets.iter().flat_map(|t| [t.start, t.end()]));
        edges.sort();
        edges.dedup();
        edit::merge_rests(&mut evs, r.from, r.to, &edges);
    }
    set_events(m, lane, voice, &EditResult { events: evs, ..r.clone() });
}

/// An instrument as the Players dialog and instrument changes give it.
#[derive(Clone, Debug, PartialEq)]
pub struct InstrumentSpec {
    pub name: String,
    pub abbr: String,
    /// "treble", "bass"…; "treble+bass" for a grand staff.
    pub clef: String,
    pub transposition: Interval,
}

/// A player in the Players dialog's list: one already in the score, or one
/// to add, with its instruments (the first, then doublings).
#[derive(Clone, Debug, PartialEq)]
pub enum PlayerSlot {
    Keep { id: Id, instruments: Vec<InstrumentSpec> },
    New { instruments: Vec<InstrumentSpec> },
}

/// Which end of a slur a drag or key moves.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SlurHandle {
    Start,
    End,
}

/// Single note selected: a span from it to the next note in its voice.
fn span_from_note(evs: &[Event], note: Id) -> Option<(Frac, Frac)> {
    let (e, _) = edit::find_event(evs, note)?;
    evs.iter().find(|n| n.start > e.start && !n.heads.is_empty()).map(|n| (e.start, n.end()))
}

/// A new instrument for `player` (not yet in its list), with a track of
/// its own on a free MIDI channel.
fn add_instrument(p: &mut Project, player: Id, spec: &InstrumentSpec) -> Id {
    let (track, inst) = (p.allocate_id(), p.allocate_id());
    let used: Vec<u8> = p.tracks.iter().map(|t| t.midi_channel).collect();
    // A drum set starts with its map, and plays on the drum channel (10).
    let drums = if spec.clef == "percussion" { default_drums(&spec.name) } else { vec![] };
    let midi_channel = if !drums.is_empty() { 9 } else { (0..16).filter(|c| *c != 9).find(|c| !used.contains(c)).unwrap_or((p.tracks.len() % 16) as u8) };
    p.tracks.push(Track { id: track, name: spec.name.clone(), instrument_node: None, note_port: 0, output_bus: None, midi_channel, gain_db: 0.0, pan: 0.0, mute: false, solo: false, inserts: vec![], sends: vec![], dynamics_cc: None, program: None, bank: None, techniques: vec![], color: None, folder: String::new(), audio: false, clips: vec![] });
    let clef = spec.clef.split('+').next().unwrap_or("treble").to_string();
    p.instruments.push(Instrument { id: inst, player, name: spec.name.clone(), engraver_name: spec.abbr.clone(), transposition: spec.transposition, clef, track: Some(track), drums, tuning: default_tuning(&spec.name) });
    inst
}

/// Brings a kept player's instruments in line with `specs` (see
/// [`Editor::set_players`]); returns how many were changed or added/removed.
fn reconcile_instruments(p: &mut Project, player: Id, specs: &[InstrumentSpec]) -> usize {
    let Some(held) = p.player(player).map(|pl| pl.instruments.clone()) else { return 0 };
    let mut free = held.clone();
    let mut result: Vec<Option<Id>> = vec![None; specs.len()];
    let name_of = |p: &Project, i: Id| p.instrument(i).map(|x| x.name.clone()).unwrap_or_default();
    let mut changed = 0;
    for (k, s) in specs.iter().enumerate() {
        if let Some(pos) = free.iter().position(|i| name_of(p, *i).eq_ignore_ascii_case(&s.name)) {
            let id = free.remove(pos);
            // The same instrument; its short name may be new.
            if let Some(i) = p.instruments.iter_mut().find(|i| i.id == id && i.engraver_name != s.abbr) {
                i.engraver_name = s.abbr.clone();
                changed += 1;
            }
            result[k] = Some(id);
        }
    }
    for (k, s) in specs.iter().enumerate() {
        if result[k].is_some() {
            continue;
        }
        changed += 1;
        if !free.is_empty() {
            // The instrument changes; its music and track stay.
            let id = free.remove(0);
            let clef = s.clef.split('+').next().unwrap_or("treble").to_string();
            let mut track = None;
            if let Some(i) = p.instruments.iter_mut().find(|i| i.id == id) {
                i.name = s.name.clone();
                i.engraver_name = s.abbr.clone();
                i.clef = clef;
                i.transposition = s.transposition;
                track = i.track;
            }
            if let Some(t) = p.tracks.iter_mut().find(|t| Some(t.id) == track) {
                t.name = s.name.clone();
            }
            result[k] = Some(id);
        } else {
            result[k] = Some(add_instrument(p, player, s));
        }
    }
    // Instruments no longer listed, and the changes to them.
    changed += free.len();
    p.instruments.retain(|i| !free.contains(&i.id));
    let ids: Vec<Id> = result.into_iter().flatten().collect();
    let first = ids[0];
    for m in p.movements.iter_mut() {
        for l in m.lanes.iter_mut().filter(|l| l.player == player) {
            l.changes.retain(|c| c.at == Frac::ZERO || !free.contains(&c.instrument));
            match l.changes.first_mut() {
                // The player starts on its first instrument.
                Some(c) if c.at == Frac::ZERO => c.instrument = first,
                _ => {}
            }
        }
    }
    if let Some(pl) = p.players.iter_mut().find(|pl| pl.id == player) {
        if changed == 0 && ids != pl.instruments {
            changed = 1; // reordered: another instrument starts
        }
        pl.name = specs[0].name.clone();
        pl.instruments = ids;
    }
    changed
}

/// Drops tracks no instrument uses, and plugin nodes no track uses.
fn drop_unused_tracks(p: &mut Project) {
    let used: Vec<Id> = p.instruments.iter().filter_map(|i| i.track).collect();
    let gone_nodes: Vec<Id> = p.tracks.iter().filter(|t| !used.contains(&t.id)).filter_map(|t| t.instrument_node).collect();
    p.tracks.retain(|t| used.contains(&t.id));
    let still: Vec<Id> = p.tracks.iter().filter_map(|t| t.instrument_node).collect();
    for n in gone_nodes.into_iter().filter(|n| !still.contains(n)) {
        p.nodes.retain(|x| x.id != n);
        p.edges.retain(|e| e.from != n && e.to != n);
        p.plugin_state.remove(&n);
    }
}

impl Editor {
    pub fn new(project: Project) -> Editor {
        Editor {
            project,
            movement: 0,
            input_mode: false,
            spec: DurationSpec::default(),
            accidental: None,
            chord_mode: false,
            voice: 1,
            lyric_verse: 1,
            caret_lane: 0,
            caret: Frac::ZERO,
            selection: None,
            picked: Vec::new(),
            range: None,
            range_anchor: None,
            transposed: false,
            grid: 1,
            tuplet_input: None,
            grace_input: None,
            clip: None,
            undo: Vec::new(),
            redo: Vec::new(),
            revision: 0,
            music_revision: 0,
            merge: None,
            slur: None,
            modified: false,
            status: String::new(),
        }
    }

    pub fn movement(&self) -> Option<&Movement> {
        self.project.movements.get(self.movement)
    }

    // ------------------------------------------------------------ history --
    pub(super) fn snapshot(&self) -> Project {
        self.project.clone()
    }

    pub(super) fn commit(&mut self, before: Project, label: &str) {
        self.commit_with(before, label, true, None);
    }

    /// `merge`: consecutive commits with the same label and key fold into
    /// the first one's undo step.
    pub(super) fn commit_with(&mut self, before: Project, label: &str, music: bool, merge: Option<Id>) {
        let key = merge.map(|k| (label.to_string(), k));
        let merging = key.is_some() && key == self.merge && self.undo.last().is_some_and(|u| u.1 == label) && self.redo.is_empty();
        if !merging {
            self.undo.push((before, label.to_string()));
            if self.undo.len() > UNDO_LIMIT {
                self.undo.remove(0);
            }
        }
        self.merge = key;
        self.redo.clear();
        self.changed_with(music);
    }

    pub(super) fn changed(&mut self) {
        self.changed_with(true);
    }

    pub(super) fn changed_with(&mut self, music: bool) {
        self.revision += 1;
        if music {
            self.music_revision += 1;
        }
        self.modified = true;
        // Undo may take away the movement shown.
        self.movement = self.movement.min(self.project.movements.len().saturating_sub(1));
        // Slurs go with their notes.
        for m in self.project.movements.iter_mut() {
            m.tidy_slurs();
        }
        // Keep the selection valid.
        if let Some(id) = self.selection {
            if self.locate(id).is_none() && self.movement().and_then(|m| edit::describe_item(m, id)).is_none() && self.item_text(id).is_none() {
                self.selection = None;
            }
        }
        if let (Some(r), Some(m)) = (self.range, self.movement()) {
            if r.lane_to >= m.lanes.len() || r.from >= m.length {
                self.range = None;
            }
        }
    }

    // ---------------------------------------------------------- movements --
    /// Shows movement `i`: the caret at its start, nothing selected.
    pub fn switch_movement(&mut self, i: usize) {
        if i >= self.project.movements.len() || i == self.movement {
            return;
        }
        self.movement = i;
        self.selection = None;
        self.range = None;
        self.caret = Frac::ZERO;
        self.caret_lane = 0;
        self.revision += 1;
        self.music_revision += 1;
        self.status = format!("Movement {}: {}", i + 1, self.project.movements[i].title);
    }

    /// A new movement after this one: the same staves and instruments
    /// (their opening ones), empty, in this movement's opening key, meter
    /// and tempo, as many bars as this one up to 32.
    pub fn add_movement(&mut self) {
        let Some(cur) = self.movement().cloned() else { return };
        let before = self.snapshot();
        let n = self.project.movements.len();
        let mut id = || self.project.allocate_id();
        let meter: Vec<Meter> = cur.meter.iter().filter(|x| !x.pickup).take(1).map(|x| Meter { at: Frac::ZERO, ..*x }).collect();
        let bar = meter.first().map(|x| Frac::new(x.num, x.den)).unwrap_or(Frac::int(1));
        let bars = edit::bar_starts(&cur).len().clamp(4, 32) as i64;
        let length = bar * Frac::int(bars);
        let lanes: Vec<Lane> = cur
            .lanes
            .iter()
            .map(|l| Lane {
                player: l.player,
                changes: l.changes.first().map(|c| InstrumentChange { id: id(), at: Frac::ZERO, instrument: c.instrument }).into_iter().collect(),
                // A staff starting in another clef than its instrument's keeps it.
                clefs: l.clefs.iter().filter(|c| c.at == Frac::ZERO).map(|c| ClefChange { id: id(), at: Frac::ZERO, clef: c.clef.clone() }).collect(),
                texts: vec![],
                chords: vec![],
                lines: vec![],
                voices: vec![Voice { id: id(), index: 1, events: Arc::new(Vec::new()), tuplets: vec![] }],
            })
            .collect();
        let new = Movement {
            id: id(),
            ordinal: 0,
            title: format!("Movement {}", n + 1),
            length,
            revision: 0,
            tempo: cur.tempo.iter().take(1).map(|t| Tempo { at: Frac::ZERO, ..t.clone() }).collect(),
            meter,
            keys: cur.keys.iter().take(1).map(|k| Key { at: Frac::ZERO, ..*k }).collect(),
            barlines: vec![],
            rehearsal: vec![],
            endings: vec![],
            navigation: vec![],
            slurs: vec![],
            breaks: vec![], cross: vec![], automation: vec![], tweaks: vec![],
            lanes,
        };
        let at = self.movement + 1;
        self.project.movements.insert(at, new);
        for (k, m) in self.project.movements.iter_mut().enumerate() {
            m.ordinal = k as i64;
        }
        self.commit(before, "New movement");
        self.switch_movement_force(at);
    }

    /// A percussion staff (its instrument's clef): no key; letters and
    /// clicks are staff positions.
    pub fn lane_is_percussion(&self, lane: usize) -> bool {
        self.movement().and_then(|m| m.lanes.get(lane)).and_then(|l| l.changes.first()).and_then(|c| self.project.instrument(c.instrument)).is_some_and(|i| i.clef == "percussion")
    }

    /// Tablature on or off for the selected staff's instrument (standard
    /// guitar tuning, or the bass guitar's on a bass staff).
    pub fn toggle_tab(&mut self) {
        let Some(m) = self.movement() else { return };
        let lane = self.selection.and_then(|s| self.event_of(s)).map(|(loc, _)| loc.lane).or(self.range.map(|r| r.lane_from)).unwrap_or(self.caret_lane);
        let Some(inst) = m.lanes.get(lane).and_then(|l| l.changes.first()).map(|c| c.instrument) else { return };
        let before = self.snapshot();
        let Some(i) = self.project.instruments.iter_mut().find(|i| i.id == inst) else { return };
        let on = i.tuning.is_empty();
        i.tuning = if !on { vec![] } else if i.clef.starts_with("bass") { BASS_TUNING.to_vec() } else { GUITAR_TUNING.to_vec() };
        self.status = format!("Tablature {} for {}", if on { "on" } else { "off" }, i.name);
        self.commit(before, "Tablature");
    }

    /// The drum sound of a note on a drum set's staff.
    pub fn drum_name(&self, lane: usize, at: Frac, p: Pitch) -> Option<String> {
        let m = self.movement()?;
        let inst = self.project.instrument(m.lanes.get(lane)?.instrument_at(at)?)?;
        inst.drum_at(p).map(|d| d.name.clone())
    }

    /// The movement holding element `id` (a note, rest, mark or text).
    pub fn movement_of(&self, id: Id) -> Option<usize> {
        let id = ids::as_dynamic(id).or(ids::as_lyric(id).map(|l| l.0)).unwrap_or(id);
        self.project.movements.iter().position(|m| {
            m.lanes.iter().any(|l| {
                l.voices.iter().any(|v| v.events.iter().any(|e| e.id == id || e.heads.iter().any(|h| h.id == id) || e.graces.iter().any(|g| g.id == id || g.heads.iter().any(|h| h.id == id))))
                    || l.texts.iter().any(|t| t.id == id)
                    || l.chords.iter().any(|c| c.id == id)
                    || l.clefs.iter().any(|c| c.id == id)
            }) || edit::describe_item(m, id).is_some()
                || m.slurs.iter().any(|s| s.id == id)
        })
    }

    pub(super) fn switch_movement_force(&mut self, i: usize) {
        self.movement = i.min(self.project.movements.len().saturating_sub(1));
        self.selection = None;
        self.range = None;
        self.caret = Frac::ZERO;
        self.caret_lane = 0;
        self.revision += 1;
        self.music_revision += 1;
    }

    /// Deletes the movement shown (not the last one left).
    pub fn delete_movement(&mut self) {
        if self.project.movements.len() < 2 {
            self.status = "A score needs at least one movement".into();
            return;
        }
        let before = self.snapshot();
        let gone = self.project.movements.remove(self.movement);
        for (k, m) in self.project.movements.iter_mut().enumerate() {
            m.ordinal = k as i64;
        }
        self.commit(before, &format!("Delete movement \"{}\"", gone.title));
        self.switch_movement_force(self.movement.saturating_sub(1));
    }

    pub fn rename_movement(&mut self, title: &str) {
        let title = title.trim().to_string();
        if self.movement().is_none_or(|m| m.title == title) {
            return;
        }
        let before = self.snapshot();
        let i = self.movement;
        self.project.movements[i].title = title;
        self.commit(before, "Rename movement");
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    pub fn undo_label(&self) -> Option<&str> {
        self.undo.last().map(|u| u.1.as_str())
    }

    pub fn undo(&mut self) {
        self.merge = None;
        if let Some((p, label)) = self.undo.pop() {
            let now = std::mem::replace(&mut self.project, p);
            self.redo.push((now, label.clone()));
            self.status = format!("Undo {label}");
            self.changed();
        }
    }

    pub fn redo(&mut self) {
        self.merge = None;
        if let Some((p, label)) = self.redo.pop() {
            let now = std::mem::replace(&mut self.project, p);
            self.undo.push((now, label.clone()));
            self.status = format!("Redo {label}");
            self.changed();
        }
    }

    /// Runs `f` on the current movement with an id allocator; commits one
    /// undo step when it returns `true`, otherwise restores the project.
    pub(super) fn transact(&mut self, label: &str, f: impl FnOnce(&mut Movement, &mut Ids) -> bool) -> bool {
        let before = self.snapshot();
        let mut ids = Ids(self.project.next_id);
        let Some(m) = self.project.movements.get_mut(self.movement) else { return false };
        let ok = f(m, &mut ids);
        self.project.next_id = ids.0;
        if ok {
            self.commit(before, label);
        } else {
            self.project = before;
        }
        ok
    }

    // ------------------------------------------------------------ lookups --
    pub fn locate(&self, id: Id) -> Option<Locator> {
        let m = self.movement()?;
        for (li, l) in m.lanes.iter().enumerate() {
            for (vi, v) in l.voices.iter().enumerate() {
                if edit::find_event(&v.events, id).is_some() {
                    return Some(Locator { lane: li, voice: vi });
                }
            }
        }
        None
    }

    pub(super) fn event_of(&self, id: Id) -> Option<(Locator, &Event)> {
        let loc = self.locate(id)?;
        let v = &self.movement()?.lanes[loc.lane].voices[loc.voice];
        edit::find_event(&v.events, id).map(|(e, _)| (loc, e))
    }

    // ------------------------------------------------------------- labels --
    /// "Bar 3, beat 2 · Flute · voice 1"
    pub fn caret_label(&self) -> String {
        let Some(m) = self.movement() else { return String::new() };
        let (bar, offset) = edit::bar_number(m, self.caret);
        let den = m.meter.iter().filter(|x| x.at <= self.caret).last().map(|x| x.den).unwrap_or(4);
        let beat = offset * Frac::int(den);
        let beat = beat.num as f64 / beat.den as f64 + 1.0;
        let beat = if (beat - beat.round()).abs() < 1e-6 { format!("{}", beat.round() as i64) } else { format!("{beat:.2}") };
        let who = m.lanes.get(self.caret_lane).and_then(|l| self.project.player(l.player)).map(|p| p.name.clone()).unwrap_or_default();
        format!("Bar {bar}, beat {beat} · {who} · voice {}", self.voice)
    }

    /// The selection in words: "E5 · Flute", "Bars 2–4 · 3 staves".
    pub fn selection_label(&self) -> String {
        let Some(m) = self.movement() else { return String::new() };
        if !self.picked.is_empty() {
            return format!("{} picked", self.picked.len());
        }
        if let Some(r) = self.range {
            let b0 = edit::bar_number(m, r.from).0;
            let b1 = edit::bar_number(m, (r.to - Frac::new(1, 1024)).max(r.from)).0;
            let staves = r.lane_to - r.lane_from + 1;
            let bars = if b0 == b1 { format!("Bar {b0}") } else { format!("Bars {b0}–{b1}") };
            return format!("{bars} · {staves} {}", if staves == 1 { "staff" } else { "staves" });
        }
        let Some(id) = self.selection else { return String::new() };
        if let Some(d) = self.describe_page_item(id) {
            return d;
        }
        if let Some(d) = edit::describe_item(m, id) {
            return d;
        }
        let Some((loc, e)) = self.event_of(id) else { return String::new() };
        let lane = &m.lanes[loc.lane];
        let inst = lane.instrument_at(e.start).and_then(|i| self.project.instrument(i)).map(|i| i.name.clone()).unwrap_or_default();
        let bar = edit::bar_number(m, e.start).0;
        match e.heads.iter().find(|h| h.id == id).or(e.heads.first()) {
            Some(h) if !e.is_rest() => format!("{} · {inst} · bar {bar}", self.drum_name(loc.lane, e.start, h.pitch).unwrap_or_else(|| pitch_name(h.pitch))),
            _ => format!("Rest · {inst} · bar {bar}"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PopoverKind {
    Key,
    Time,
    Clef,
    Barline,
    Tempo,
    Expression,
    Technique,
    Ending,
    Rehearsal,
    Ornament,
    Lyrics,
    Chord,
    Tuplet,
    MovementTitle,
    Fingering,
    Dynamic,
    Title,
    Composer,
    Instrument,
    Transpose,
    Filter,
}
impl PopoverKind {
    pub fn title(self) -> &'static str {
        match self {
            PopoverKind::Key => "Key signature",
            PopoverKind::Time => "Time signature",
            PopoverKind::Clef => "Clef",
            PopoverKind::Barline => "Barline",
            PopoverKind::Tempo => "Tempo",
            PopoverKind::Expression => "Expression text",
            PopoverKind::Technique => "Technique text",
            PopoverKind::Ending => "Ending or jump",
            PopoverKind::Rehearsal => "Rehearsal mark",
            PopoverKind::Ornament => "Ornament",
            PopoverKind::Lyrics => "Lyrics",
            PopoverKind::Chord => "Chord symbol",
            PopoverKind::Tuplet => "Tuplet",
            PopoverKind::MovementTitle => "Movement title",
            PopoverKind::Fingering => "Fingering",
            PopoverKind::Dynamic => "Dynamic",
            PopoverKind::Title => "Title",
            PopoverKind::Composer => "Composer",
            PopoverKind::Instrument => "Instrument change",
            PopoverKind::Transpose => "Transpose",
            PopoverKind::Filter => "Select only",
        }
    }
    pub fn placeholder(self) -> &'static str {
        match self {
            PopoverKind::Key => "C, Eb, f#, a minor",
            PopoverKind::Time => "3/4, 6/8, C, C|, 7/8 2+2+3, pickup 1/4",
            PopoverKind::Clef => "treble, bass, alto, tenor; 8va, 8vb, 15ma, loco",
            PopoverKind::Barline => "||  |.  |:  :|  :|:",
            PopoverKind::Tempo => "Allegro, Andante 84, q=96",
            PopoverKind::Expression => "dolce, espress.",
            PopoverKind::Technique => "pizz., arco, con sord.; ped, *; up bow, down bow",
            PopoverKind::Ending => "1, 2, 1-3; segno, coda, to coda, fine, D.C. al Fine, D.S. al Coda",
            PopoverKind::Rehearsal => "",
            PopoverKind::Ornament => "tr, mordent, turn, trem, arp, fermata",
            PopoverKind::Lyrics => "Syllable; space = next note, - = hyphen, _ = extender",
            PopoverKind::Chord => "C, F#m7, Bbmaj7, G7/B; space = next beat",
            PopoverKind::Tuplet => "3, 5, 6, 7, 2 (duplet), 5:4, 7:8",
            PopoverKind::MovementTitle => "II. Adagio",
            PopoverKind::Fingering => "1, 1 3 5 (a chord, bottom up), empty clears",
            PopoverKind::Dynamic => "pp, mf, ff, sfz, fp, rfz…; empty clears",
            PopoverKind::Title => "Serenade",
            PopoverKind::Composer => "J. S. Bach",
            PopoverKind::Instrument => "Piccolo, Clarinet in A, Flute…; empty removes",
            PopoverKind::Transpose => "up M2, down P5, -m3, octave; to Eb",
            PopoverKind::Filter => "top, bottom, voice 2, above C5, F#, staccato, tied, rests, dynamics mf, lyrics",
        }
    }
}

/// "up a major 2nd", "down a perfect 5th".
pub fn interval_name(i: Interval) -> String {
    let (dir, i) = if i.semitones < 0 || (i.semitones == 0 && i.steps < 0) { ("down", i.inverse()) } else { ("up", i) };
    const BASE: [i32; 7] = [0, 2, 4, 5, 7, 9, 11];
    let class = i.steps.rem_euclid(7);
    let diff = i.semitones - BASE[class as usize] - 12 * i.steps.div_euclid(7);
    let perfect = matches!(class, 0 | 3 | 4);
    let quality = match (perfect, diff) {
        (true, 0) => "perfect",
        (false, 0) => "major",
        (false, -1) => "minor",
        (_, d) if d > 0 => "augmented",
        _ => "diminished",
    };
    let n = i.steps + 1;
    let suffix = match (n % 10, n % 100) {
        (1, x) if x != 11 => "st",
        (2, x) if x != 12 => "nd",
        (3, x) if x != 13 => "rd",
        _ => "th",
    };
    let article = if quality.starts_with(['a', 'e', 'i', 'o', 'u']) { "an" } else { "a" };
    match (quality, n) {
        ("perfect", 8) => format!("{dir} an octave"),
        _ => format!("{dir} {article} {quality} {n}{suffix}"),
    }
}

/// A dynamic typed into its popover: `None` (empty) clears.
pub fn parse_dynamic(text: &str) -> Result<Option<u8>, String> {
    let t = text.trim().to_lowercase();
    if t.is_empty() {
        return Ok(None);
    }
    dynamic_index(&t).map(Some).ok_or_else(|| format!("Dynamic not understood: {}", DYNAMICS.join(" ")))
}

/// A line typed into the clef popover (octave lines; `Some(None)` = loco)
/// or the technique popover (pedal; `Some(None)` = release).
fn line_text(kind: PopoverKind, text: &str) -> Option<Option<LineKind>> {
    match kind {
        PopoverKind::Clef => edit::parse_octave_line(text),
        PopoverKind::Technique => edit::parse_pedal(text).map(|on| on.then_some(LineKind::Pedal)),
        _ => None,
    }
}

pub fn pitch_name(p: Pitch) -> String {
    const NAMES: [&str; 7] = ["C", "D", "E", "F", "G", "A", "B"];
    let acc = match p.alter {
        -2 => "𝄫",
        -1 => "♭",
        1 => "♯",
        2 => "𝄪",
        _ => "",
    };
    format!("{}{}{}", NAMES[p.step.clamp(0, 6) as usize], acc, p.octave)
}

fn quarters(f: Frac) -> String {
    let q = f.num as f64 * 4.0 / f.den as f64;
    if (q - q.round()).abs() < 1e-9 {
        format!("{}", q.round() as i64)
    } else {
        format!("{q:.2}")
    }
}

#[cfg(test)]
mod tests;
