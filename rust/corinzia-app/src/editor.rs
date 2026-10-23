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
pub const MASTER: Id = Id::MAX;

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
    p.tracks.push(Track { id: track, name: spec.name.clone(), instrument_node: None, note_port: 0, output_bus: None, midi_channel, gain_db: 0.0, pan: 0.0, mute: false, solo: false, inserts: vec![], sends: vec![], dynamics_cc: None, program: None, bank: None, techniques: vec![] });
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
    fn snapshot(&self) -> Project {
        self.project.clone()
    }

    fn commit(&mut self, before: Project, label: &str) {
        self.commit_with(before, label, true, None);
    }

    /// `merge`: consecutive commits with the same label and key fold into
    /// the first one's undo step.
    fn commit_with(&mut self, before: Project, label: &str, music: bool, merge: Option<Id>) {
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

    fn changed(&mut self) {
        self.changed_with(true);
    }

    fn changed_with(&mut self, music: bool) {
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
            breaks: vec![], cross: vec![], automation: vec![],
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

    fn switch_movement_force(&mut self, i: usize) {
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
    fn transact(&mut self, label: &str, f: impl FnOnce(&mut Movement, &mut Ids) -> bool) -> bool {
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

    // -------------------------------------------------------------- slurs --
    /// The slur selected: valid while nothing else is selected and the
    /// slur still exists.
    pub fn selected_slur(&self) -> Option<Id> {
        let id = self.slur.filter(|_| self.selection.is_none() && self.range.is_none())?;
        self.movement()?.slurs.iter().any(|s| s.id == id).then_some(id)
    }

    /// Selects the slur `id`; false if there is none.
    pub fn select_slur(&mut self, id: Id) -> bool {
        self.slur = Some(id);
        self.selection = None;
        self.range = None;
        self.range_anchor = None;
        if self.selected_slur().is_none() {
            self.slur = None;
            return false;
        }
        self.status = "Slur · drag an end to another note · Shift+←→ end · Alt+←→ start · X flips".into();
        true
    }

    /// Moves an end of the selected slur to another note of that end's
    /// voice: `to` picks one from the voice's notes and the end's place
    /// among them. The start stays before the end. `merge`: part of a drag
    /// (one undo step).
    fn set_slur_end(&mut self, handle: SlurHandle, to: impl FnOnce(&[&Event], usize) -> usize, merge: bool) {
        let Some(id) = self.selected_slur() else { return };
        let m = self.movement().unwrap();
        let Some(sl) = m.slurs.iter().find(|s| s.id == id).copied() else { return };
        let (moving, other) = if handle == SlurHandle::Start { (sl.from, sl.to) } else { (sl.to, sl.from) };
        let (Some(loc), Some((_, o))) = (self.locate(moving), self.event_of(other)) else { return };
        let limit = o.start;
        let notes: Vec<&Event> = m.lanes[loc.lane].voices[loc.voice].events.iter().filter(|e| !e.is_rest()).collect();
        let Some(k) = notes.iter().position(|e| e.id == moving) else { return };
        let target = to(&notes, k).min(notes.len() - 1);
        // The nearest allowed note to the one asked for.
        let ok = |i: usize| if handle == SlurHandle::Start { notes[i].start < limit } else { notes[i].start > limit };
        let Some(pick) = (0..notes.len()).filter(|&i| ok(i)).min_by_key(|&i| i.abs_diff(target)) else { return };
        if pick == k {
            return;
        }
        let new = notes[pick].id;
        let before = self.snapshot();
        let m = &mut self.project.movements[self.movement];
        let Some(sl) = m.slurs.iter_mut().find(|s| s.id == id) else { return };
        if handle == SlurHandle::Start {
            sl.from = new;
        } else {
            sl.to = new;
        }
        self.commit_with(before, "Move slur", true, merge.then_some(id));
        self.status = "Slur moved".into();
    }

    /// Moves an end of the selected slur by `by` notes (keyboard).
    pub fn step_slur(&mut self, handle: SlurHandle, by: i64) {
        self.set_slur_end(handle, |_, k| (k as i64 + by).max(0) as usize, false);
    }

    /// Drags an end of the selected slur to the note of its voice nearest
    /// `at` (written time, whole notes).
    pub fn drag_slur(&mut self, handle: SlurHandle, at: Frac) {
        self.set_slur_end(
            handle,
            |notes, _| {
                let d = |e: &Event| if e.start > at { e.start - at } else { at - e.start };
                (0..notes.len()).min_by(|&a, &b| d(notes[a]).cmp(&d(notes[b]))).unwrap_or(0)
            },
            true,
        );
    }

    /// Flips the selected slur to the other side (`above`: where it is now).
    pub fn flip_slur(&mut self, above: bool) {
        let Some(id) = self.selected_slur() else { return };
        self.transact("Flip slur", |m, _| {
            let Some(s) = m.slurs.iter_mut().find(|s| s.id == id) else { return false };
            s.dir = if above { -1 } else { 1 };
            true
        });
        self.status = format!("Slur {}", if above { "below" } else { "above" });
    }

    /// Removes the selected slur.
    pub fn delete_slur(&mut self) {
        let Some(id) = self.selected_slur() else { return };
        self.transact("Delete slur", |m, _| {
            let n = m.slurs.len();
            m.slurs.retain(|s| s.id != id);
            m.slurs.len() != n
        });
        self.slur = None;
        self.status = "Slur deleted".into();
    }

    // -------------------------------------------------------------- mixer --
    /// A mixer change on a track; `merge` folds a slider drag into one step.
    /// Ends a slider gesture: what follows is a new undo step.
    pub fn end_gesture(&mut self) {
        self.merge = None;
    }

    pub fn set_track(&mut self, track: Id, label: &str, merge: bool, f: impl FnOnce(&mut Track)) {
        let before = self.snapshot();
        let Some(t) = self.project.tracks.iter_mut().find(|t| t.id == track) else { return };
        f(t);
        self.commit_with(before, label, false, merge.then_some(track));
    }

    /// Assigns a plugin (name, bundle, plugin id; CLAP or VST3) to a track, or the
    /// built-in instrument (`None`). The track's previous plugin node goes.
    pub fn set_track_plugin(&mut self, track: Id, plugin: Option<(&str, &str, &str)>) {
        let before = self.snapshot();
        let Some(ti) = self.project.tracks.iter().position(|t| t.id == track) else { return };
        if let Some(old) = self.project.tracks[ti].instrument_node.take() {
            let shared = self.project.tracks.iter().any(|t| t.instrument_node == Some(old));
            if !shared {
                self.project.nodes.retain(|n| n.id != old);
                self.project.edges.retain(|e| e.from != old && e.to != old);
                self.project.plugin_state.remove(&old);
            }
        }
        let name = match plugin {
            Some((name, bundle, id)) => {
                let node = self.project.allocate_id();
                self.project.nodes.push(AudioNode { id: node, kind: NodeKind::Plugin, name: name.into(), bundle: bundle.into(), plugin_id: id.into(), bypassed: false });
                self.project.tracks[ti].instrument_node = Some(node);
                name.to_string()
            }
            None => "Corinzia Sine".to_string(),
        };
        self.status = format!("{} plays {name}", self.project.tracks[ti].name);
        self.commit_with(before, "Instrument plugin", true, None);
    }

    // -------------------------------------------------------------- mixer --
    /// The effect chain of a strip: a track, a bus, or the master ([`MASTER`]).
    fn inserts_mut(p: &mut Project, owner: Id) -> Option<&mut Vec<Id>> {
        if owner == MASTER {
            return Some(&mut p.master.inserts);
        }
        if let Some(t) = p.tracks.iter_mut().find(|t| t.id == owner) {
            return Some(&mut t.inserts);
        }
        p.buses.iter_mut().find(|b| b.id == owner).map(|b| &mut b.inserts)
    }

    /// A strip's name: the track's, the bus's, or "Master".
    pub fn strip_name(&self, owner: Id) -> String {
        if owner == MASTER {
            return "Master".into();
        }
        self.project.track(owner).map(|t| t.name.clone()).or_else(|| self.project.bus(owner).map(|b| b.name.clone())).unwrap_or_default()
    }

    /// Adds an effect (name, bundle, plugin id) at the end of a strip's chain.
    pub fn add_insert(&mut self, owner: Id, name: &str, bundle: &str, plugin_id: &str) -> Option<Id> {
        let before = self.snapshot();
        let node = self.project.allocate_id();
        Self::inserts_mut(&mut self.project, owner)?.push(node);
        self.project.nodes.push(AudioNode { id: node, kind: NodeKind::Plugin, name: name.into(), bundle: bundle.into(), plugin_id: plugin_id.into(), bypassed: false });
        self.status = format!("{name} on {}", self.strip_name(owner));
        self.commit_with(before, "Add effect", false, None);
        Some(node)
    }

    /// Takes an effect out of its strip (its node and state go).
    pub fn remove_insert(&mut self, owner: Id, node: Id) {
        let before = self.snapshot();
        let Some(list) = Self::inserts_mut(&mut self.project, owner) else { return };
        list.retain(|n| *n != node);
        let name = self.project.node(node).map(|n| n.name.clone()).unwrap_or_default();
        self.project.nodes.retain(|n| n.id != node);
        self.project.plugin_state.remove(&node);
        self.status = format!("{name} removed");
        self.commit_with(before, "Remove effect", false, None);
    }

    /// Moves an effect earlier (-1) or later (+1) in its chain.
    pub fn move_insert(&mut self, owner: Id, node: Id, by: i32) {
        let before = self.snapshot();
        let Some(list) = Self::inserts_mut(&mut self.project, owner) else { return };
        let Some(i) = list.iter().position(|n| *n == node) else { return };
        let j = (i as i32 + by).clamp(0, list.len() as i32 - 1) as usize;
        if i == j {
            return;
        }
        list.swap(i, j);
        self.commit_with(before, "Move effect", false, None);
    }

    /// Bypasses an effect, or brings it back.
    pub fn toggle_bypass(&mut self, node: Id) {
        let before = self.snapshot();
        let Some(n) = self.project.nodes.iter_mut().find(|n| n.id == node) else { return };
        n.bypassed = !n.bypassed;
        self.status = format!("{} {}", n.name, if n.bypassed { "bypassed" } else { "back on" });
        self.commit_with(before, "Bypass", false, None);
    }

    /// A new bus ("Bus 1", "Reverb"…), into the master.
    pub fn add_bus(&mut self, name: &str) -> Id {
        let before = self.snapshot();
        let id = self.project.allocate_id();
        let name = if name.is_empty() { format!("Bus {}", self.project.buses.len() + 1) } else { name.to_string() };
        self.project.buses.push(Bus { id, name: name.clone(), inserts: vec![], gain_db: 0.0, pan: 0.0, mute: false });
        self.status = format!("{name} added");
        self.commit_with(before, "Add bus", false, None);
        id
    }

    /// Removes a bus: tracks routed to it go to the master, sends to it go.
    pub fn remove_bus(&mut self, bus: Id) {
        let before = self.snapshot();
        let Some(i) = self.project.buses.iter().position(|b| b.id == bus) else { return };
        let b = self.project.buses.remove(i);
        for n in &b.inserts {
            self.project.nodes.retain(|x| x.id != *n);
            self.project.plugin_state.remove(n);
        }
        for t in self.project.tracks.iter_mut() {
            if t.output_bus == Some(bus) {
                t.output_bus = None;
            }
            t.sends.retain(|s| s.bus != bus);
        }
        self.status = format!("{} removed", b.name);
        self.commit_with(before, "Remove bus", false, None);
    }

    /// Where a track goes: a bus, or the master (`None`).
    pub fn set_output(&mut self, track: Id, bus: Option<Id>) {
        self.set_track(track, "Output", false, |t| t.output_bus = bus);
    }

    /// A send from a track to a bus (at -6 dB), or none when there is one.
    pub fn toggle_send(&mut self, track: Id, bus: Id) {
        self.set_track(track, "Send", false, |t| {
            if t.sends.iter().any(|s| s.bus == bus) {
                t.sends.retain(|s| s.bus != bus);
            } else {
                t.sends.push(SendLevel { bus, gain_db: -6.0 });
            }
        });
    }

    pub fn set_send(&mut self, track: Id, index: usize, db: f64) {
        self.set_track(track, "Send level", true, |t| {
            if let Some(s) = t.sends.get_mut(index) {
                s.gain_db = db;
            }
        });
    }

    pub fn set_bus(&mut self, bus: Id, label: &str, merge: bool, f: impl FnOnce(&mut Bus)) {
        let before = self.snapshot();
        let Some(b) = self.project.buses.iter_mut().find(|b| b.id == bus) else { return };
        f(b);
        self.commit_with(before, label, false, merge.then_some(bus));
    }

    pub fn set_master_gain(&mut self, db: f64) {
        let before = self.snapshot();
        self.project.master.gain_db = db;
        self.commit_with(before, "Master", false, Some(MASTER));
    }

    // --------------------------------------------------------- automation --
    /// The current movement's lane for a target, if any.
    pub fn auto_lane(&self, target: AutoTarget) -> Option<&AutoLane> {
        self.movement()?.automation.iter().find(|l| l.target == target)
    }

    /// Adds a point (a lane is made for the target if need be); returns its index.
    pub fn add_auto_point(&mut self, target: AutoTarget, at: Frac, value: f64) -> Option<usize> {
        let before = self.snapshot();
        let id = self.project.allocate_id();
        let m = self.project.movements.get_mut(self.movement)?;
        let lane = match m.automation.iter().position(|l| l.target == target) {
            Some(i) => &mut m.automation[i],
            None => {
                m.automation.push(AutoLane { id, target, points: vec![] });
                m.automation.last_mut().unwrap()
            }
        };
        let i = lane.points.partition_point(|p| p.at <= at);
        lane.points.insert(i, AutoPoint { at, value });
        self.commit_with(before, "Automation point", false, None);
        Some(i)
    }

    /// Moves a point (kept between its neighbours); a drag is one undo step.
    pub fn move_auto_point(&mut self, target: AutoTarget, index: usize, at: Frac, value: f64) {
        let before = self.snapshot();
        let Some(m) = self.project.movements.get_mut(self.movement) else { return };
        let Some(lane) = m.automation.iter_mut().find(|l| l.target == target) else { return };
        let lo = index.checked_sub(1).and_then(|i| lane.points.get(i)).map_or(Frac::ZERO, |p| p.at);
        let hi = lane.points.get(index + 1).map_or(m.length, |p| p.at);
        let Some(p) = lane.points.get_mut(index) else { return };
        let at = at.max(lo).min(hi);
        if (p.at, p.value) == (at, value) {
            return;
        }
        (p.at, p.value) = (at, value);
        let key = lane.id;
        self.commit_with(before, "Move automation", false, Some(key));
    }

    /// Deletes a point; an empty lane goes.
    pub fn delete_auto_point(&mut self, target: AutoTarget, index: usize) {
        let before = self.snapshot();
        let Some(m) = self.project.movements.get_mut(self.movement) else { return };
        let Some(li) = m.automation.iter().position(|l| l.target == target) else { return };
        if index >= m.automation[li].points.len() {
            return;
        }
        m.automation[li].points.remove(index);
        if m.automation[li].points.is_empty() {
            m.automation.remove(li);
        }
        self.commit_with(before, "Delete automation", false, None);
    }

    // ------------------------------------------------------------ players --
    /// Sets the instrumentation: the players in `list`, in its order, each
    /// with its instruments (the first, then doublings). Players left out go
    /// with their staves (in every movement), instruments and tracks; new
    /// ones get staves of whole-bar rests in every movement. A kept player's
    /// instruments are matched by name: a name still listed keeps its
    /// instrument; a new name takes the place of one no longer listed (the
    /// instrument changes, its music and track stay); any others are added
    /// as doublings with tracks of their own, and instruments left over go.
    /// One undo step.
    pub fn set_players(&mut self, list: &[PlayerSlot]) -> Result<(), String> {
        if list.is_empty() {
            return Err("A score needs at least one player".into());
        }
        if list.iter().any(|s| match s {
            PlayerSlot::Keep { instruments, .. } | PlayerSlot::New { instruments } => instruments.is_empty(),
        }) {
            return Err("Each player needs an instrument".into());
        }
        let before = self.snapshot();
        let p = &mut self.project;
        let kept: Vec<Id> = list.iter().filter_map(|s| if let PlayerSlot::Keep { id, .. } = s { Some(*id) } else { None }).collect();
        // Removed players and their instruments.
        let gone: Vec<Id> = p.players.iter().map(|pl| pl.id).filter(|id| !kept.contains(id)).collect();
        p.instruments.retain(|i| !gone.contains(&i.player));
        p.players.retain(|pl| !gone.contains(&pl.id));
        for m in p.movements.iter_mut() {
            m.lanes.retain(|l| !gone.contains(&l.player));
        }
        // Kept players' instruments.
        let mut changed = 0;
        for slot in list {
            if let PlayerSlot::Keep { id, instruments } = slot {
                changed += reconcile_instruments(p, *id, instruments);
            }
        }
        // New players.
        let mut order = Vec::new();
        for slot in list {
            let id = match slot {
                PlayerSlot::Keep { id, .. } => *id,
                PlayerSlot::New { instruments } => {
                    let player = p.allocate_id();
                    let ids: Vec<Id> = instruments.iter().map(|spec| add_instrument(p, player, spec)).collect();
                    p.players.push(Player { id: player, name: instruments[0].name.clone(), instruments: ids.clone() });
                    let clefs: Vec<String> = instruments[0].clef.split('+').map(str::to_string).collect();
                    for mi in 0..p.movements.len() {
                        let bars: Vec<(Frac, Frac)> = {
                            let m = &p.movements[mi];
                            edit::bar_starts(m).into_iter().map(|b| (b, edit::bar_length(m, b).min(m.length - b))).collect()
                        };
                        for (si, c) in clefs.iter().enumerate() {
                            let rests: Vec<Event> = bars
                                .iter()
                                .map(|&(start, duration)| Event { id: p.allocate_id(), kind: EventKind::Rest, start, duration, heads: vec![], articulations: 0, dynamic: None, spans: 0, lyrics: vec![], graces: vec![], fingering: vec![] })
                                .collect();
                            let (change, voice) = (p.allocate_id(), p.allocate_id());
                            let clef_changes = if si == 0 { vec![] } else { vec![ClefChange { id: p.allocate_id(), at: Frac::ZERO, clef: c.clone() }] };
                            p.movements[mi].lanes.push(Lane {
                                player,
                                changes: vec![InstrumentChange { id: change, at: Frac::ZERO, instrument: ids[0] }],
                                clefs: clef_changes,
                                texts: vec![],
                                chords: vec![],
                                lines: vec![],
                                voices: vec![Voice { id: voice, index: 1, events: Arc::new(rests), tuplets: vec![] }],
                            });
                        }
                    }
                    player
                }
            };
            order.push(id);
        }
        drop_unused_tracks(p);
        // The list's order: players, their staves (a player's own staves
        // keep theirs), and the mixer's tracks.
        let rank = |player: Id| order.iter().position(|x| *x == player).unwrap_or(usize::MAX);
        p.players.sort_by_key(|pl| rank(pl.id));
        for m in p.movements.iter_mut() {
            m.lanes.sort_by_key(|l| rank(l.player)); // stable
        }
        let mut track_rank: std::collections::HashMap<Id, (usize, usize)> = std::collections::HashMap::new();
        for pl in &p.players {
            for (k, i) in pl.instruments.iter().enumerate() {
                if let Some(t) = p.instruments.iter().find(|x| x.id == *i).and_then(|x| x.track) {
                    let r = (rank(pl.id), k);
                    let e = track_rank.entry(t).or_insert(r);
                    *e = (*e).min(r);
                }
            }
        }
        p.tracks.sort_by_key(|t| track_rank.get(&t.id).copied().unwrap_or((usize::MAX, 0)));
        let (added, removed) = (list.len() - kept.len(), gone.len());
        let same_order = before.players.iter().map(|pl| pl.id).eq(self.project.players.iter().map(|pl| pl.id));
        if added == 0 && removed == 0 && changed == 0 && same_order {
            self.project = before;
            self.status = "No changes to the players".into();
            return Ok(());
        }
        // Nothing to point at in staves that may be gone.
        self.selection = None;
        self.range = None;
        self.caret = Frac::ZERO;
        self.caret_lane = 0;
        let plural = |n: usize| if n == 1 { "" } else { "s" };
        let mut parts = Vec::new();
        if added > 0 {
            parts.push(format!("{added} player{} added", plural(added)));
        }
        if removed > 0 {
            parts.push(format!("{removed} player{} removed", plural(removed)));
        }
        if changed > 0 {
            parts.push(format!("{changed} instrument{} changed", plural(changed)));
        }
        self.status = if parts.is_empty() { "Players updated".into() } else { parts.join(", ") };
        self.commit(before, "Players");
        Ok(())
    }

    /// An instrument change at the caret, the selection's start or the
    /// selected bar, on its player's staves: to an instrument the player
    /// holds, or one added as a doubling (with a track). The clef follows
    /// the instrument. Empty text removes the change there.
    pub fn change_instrument(&mut self, text: &str) -> Result<(), String> {
        let (lane, target) = self.structure_target().ok_or("No movement")?;
        let m = self.movement().ok_or("No movement")?;
        let player = m.lanes.get(lane).ok_or("No staves")?.player;
        let mi = self.movement;
        let before = self.snapshot();
        if text.trim().is_empty() {
            let mut removed = false;
            for l in self.project.movements[mi].lanes.iter_mut().filter(|l| l.player == player) {
                let n = l.changes.len();
                l.changes.retain(|c| c.at == Frac::ZERO || c.at != target);
                removed |= l.changes.len() != n;
            }
            if !removed {
                return Err("No instrument change here".into());
            }
            self.commit(before, "Remove instrument change");
            self.status = "Instrument change removed".into();
            return Ok(());
        }
        let spec = crate::files::instrument_spec(text).ok_or("Name an instrument")?;
        let p = &mut self.project;
        let held = p.player(player).map(|pl| pl.instruments.clone()).unwrap_or_default();
        let inst = match held.iter().copied().find(|i| p.instrument(*i).is_some_and(|x| x.name.eq_ignore_ascii_case(&spec.name))) {
            Some(i) => i,
            None => {
                let i = add_instrument(p, player, &spec);
                if let Some(pl) = p.players.iter_mut().find(|pl| pl.id == player) {
                    pl.instruments.push(i);
                }
                i
            }
        };
        let name = p.instrument(inst).map(|i| i.name.clone()).unwrap_or_default();
        let new_clef = p.instrument(inst).map(|i| i.clef.split('+').next().unwrap_or("treble").to_string()).unwrap_or_default();
        let first_clef = |p: &Project, l: &Lane| l.changes.first().and_then(|c| p.instrument(c.instrument)).map(|i| i.clef.split('+').next().unwrap_or("treble").to_string()).unwrap_or_default();
        let lanes: Vec<usize> = p.movements[mi].lanes.iter().enumerate().filter(|(_, l)| l.player == player).map(|(i, _)| i).collect();
        // The player's first staff: the clef in force here, before the change.
        let first_lane = lanes[0];
        let clef_before = {
            let l = &p.movements[mi].lanes[first_lane];
            l.clefs.iter().rfind(|c| c.at <= target).map(|c| c.clef.clone()).unwrap_or_else(|| first_clef(p, l))
        };
        let mut ids = Ids(p.next_id);
        for &li in &lanes {
            let l = &mut p.movements[mi].lanes[li];
            if target == Frac::ZERO {
                match l.changes.first_mut() {
                    Some(c) => c.instrument = inst,
                    None => l.changes.push(InstrumentChange { id: ids.next(), at: Frac::ZERO, instrument: inst }),
                }
            } else {
                l.changes.retain(|c| c.at != target);
                l.changes.push(InstrumentChange { id: ids.next(), at: target, instrument: inst });
                l.changes.sort_by_key(|c| c.at);
            }
        }
        if target > Frac::ZERO && new_clef != clef_before && !new_clef.is_empty() {
            let l = &mut p.movements[mi].lanes[first_lane];
            l.clefs.retain(|c| c.at != target);
            l.clefs.push(ClefChange { id: ids.next(), at: target, clef: new_clef });
            l.clefs.sort_by_key(|c| c.at);
        }
        p.next_id = ids.0;
        self.commit(before, "Instrument change");
        let m = self.movement().unwrap();
        self.status = format!("{name} · bar {}", edit::bar_number(m, target.min(m.length)).0);
        Ok(())
    }

    /// Marks the project saved (the app calls this after writing the file).
    pub fn saved(&mut self) {
        self.modified = false;
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

    fn event_of(&self, id: Id) -> Option<(Locator, &Event)> {
        let loc = self.locate(id)?;
        let v = &self.movement()?.lanes[loc.lane].voices[loc.voice];
        edit::find_event(&v.events, id).map(|(e, _)| (loc, e))
    }

    // -------------------------------------------------------- input state --
    pub fn set_input_mode(&mut self, on: bool) {
        if on == self.input_mode {
            return;
        }
        self.input_mode = on;
        if !on {
            self.grace_input = None;
            self.tuplet_input = None;
        }
        // Entering input mode starts at the selected item.
        if on {
            if let Some((loc, e)) = self.selection.and_then(|id| self.event_of(id)) {
                let (start, index) = (e.start, self.movement().unwrap().lanes[loc.lane].voices[loc.voice].index);
                self.caret_lane = loc.lane;
                self.voice = index;
                self.caret = start;
            } else if let Some(r) = self.range {
                self.caret_lane = r.lane_from;
                self.caret = r.from;
            }
        }
    }

    pub fn set_voice(&mut self, v: u8) {
        self.voice = v.clamp(1, 4);
    }
    pub fn cycle_voice(&mut self) {
        self.voice = self.voice % 4 + 1;
    }
    /// Toggles an explicit accidental (pressing the same one again clears it).
    pub fn toggle_accidental(&mut self, a: i8) {
        self.accidental = if self.accidental == Some(a) { None } else { Some(a) };
    }
    pub fn cycle_dots(&mut self) {
        self.spec.dots = (self.spec.dots + 1) % 3;
    }

    // ------------------------------------------------------------- entry --
    pub fn enter_note(&mut self, step: i32, add_to_chord: bool) {
        if let Some(slash) = self.grace_input {
            self.enter_grace(step, slash);
            return;
        }
        let step = step.rem_euclid(7);
        let (lane, voice, caret, spec, alter, chord) = (self.caret_lane, self.voice, self.caret, self.spec, self.accidental.map(i32::from), add_to_chord || self.chord_mode);
        let perc = self.lane_is_percussion(lane); // no key: letters are staff positions
        let tuplet = self.tuplet_input;
        let transposed = self.transposed;
        let instruments: std::collections::HashMap<Id, Interval> = self.project.instruments.iter().map(|i| (i.id, i.transposition)).collect();
        let mut out: Option<EditResult> = None;
        let mut added_to_chord = false;
        self.transact("Note input", |m, ids| {
            if m.lanes.is_empty() {
                return false;
            }
            let lane = lane.min(m.lanes.len() - 1);
            let Some(vi) = voice_slot(m, lane, voice, true, ids) else { return false };
            let evs = m.lanes[lane].voices[vi].events.clone();
            // In a transposed score the letter is a written pitch: choose it in
            // the written key next to the written previous note, then store it
            // at concert pitch.
            let by = if transposed { m.lanes[lane].instrument_at(caret).and_then(|i| instruments.get(&i).copied()).unwrap_or_default() } else { Interval::default() };
            let fifths = if perc { 0 } else { corinzia_model::shift_key(key_fifths_at(m, caret), by.fifths()) };
            let concert = |p: Pitch| p.transposed(by.inverse());
            // Chord: add above the event just entered (the one ending at the caret).
            let last = evs.iter().filter(|e| e.start < caret && e.end() == caret && !e.heads.is_empty()).last();
            let r = if let (true, Some(last)) = (chord, last) {
                let top = last.heads.last().unwrap().pitch.transposed(by);
                let mut p = edit::nearest_pitch(step, alter, fifths, top);
                if edit::diatonic(p) <= edit::diatonic(top) {
                    p = edit::from_diatonic(edit::diatonic(p) + 7, p.alter as i32);
                }
                let p = concert(p);
                added_to_chord = true;
                match edit::add_to_chord(&evs, last.start, p, ids) {
                    Some(r) => r,
                    None => return false,
                }
            } else {
                let reference = edit::previous_pitch(&evs, caret).map(|p| p.transposed(by)).unwrap_or(Pitch { step: 6, alter: 0, octave: 4 });
                let p = concert(edit::nearest_pitch(step, alter, fifths, reference));
                let dur = input_duration(m, lane, vi, caret, spec, tuplet, ids);
                let evs = m.lanes[lane].voices[vi].events.clone();
                let r = edit::insert_event(&evs, edit::make_event(ids, caret, dur, &[p], 0), voice == 1, ids);
                extend_to(m, r.to);
                r
            };
            set_events(m, lane, vi, &r);
            out = Some(r);
            true
        });
        if let Some(r) = out {
            self.selection = r.focus;
            self.range = None;
            if !added_to_chord {
                self.caret = r.caret;
            }
        }
        self.accidental = None; // accidentals apply to one note
    }

    /// Note input from a MIDI keyboard: the key played (at concert pitch),
    /// spelled in the key signature; on a drum set, the drum on that key.
    /// `add_to_chord`: above the note just entered (keys held together).
    pub fn enter_midi_key(&mut self, key: u8, add_to_chord: bool) {
        let (lane, voice, caret, spec) = (self.caret_lane, self.voice, self.caret, self.spec);
        let tuplet = self.tuplet_input;
        let drum = self.movement().and_then(|m| m.lanes.get(lane)).and_then(|l| l.instrument_at(caret)).and_then(|i| self.project.instrument(i)).and_then(|i| i.drums.iter().find(|d| d.key == key).map(|d| d.position));
        if self.lane_is_percussion(lane) && drum.is_none() {
            self.status = format!("No drum on key {key}");
            return;
        }
        let mut out: Option<EditResult> = None;
        let mut chorded = false;
        self.transact("Note input", |m, ids| {
            if m.lanes.is_empty() {
                return false;
            }
            let lane = lane.min(m.lanes.len() - 1);
            let Some(vi) = voice_slot(m, lane, voice, true, ids) else { return false };
            let p = drum.unwrap_or_else(|| spell_key(key as i32, key_fifths_at(m, caret)));
            let evs = m.lanes[lane].voices[vi].events.clone();
            let last = evs.iter().filter(|e| e.start < caret && e.end() == caret && !e.heads.is_empty()).last();
            let r = match (add_to_chord, last) {
                (true, Some(last)) => {
                    chorded = true;
                    match edit::add_to_chord(&evs, last.start, p, ids) {
                        Some(r) => r,
                        None => return false,
                    }
                }
                _ => {
                    let dur = input_duration(m, lane, vi, caret, spec, tuplet, ids);
                    let evs = m.lanes[lane].voices[vi].events.clone();
                    let r = edit::insert_event(&evs, edit::make_event(ids, caret, dur, &[p], 0), voice == 1, ids);
                    extend_to(m, r.to);
                    r
                }
            };
            set_events(m, lane, vi, &r);
            out = Some(r);
            true
        });
        if let Some(r) = out {
            self.selection = r.focus;
            self.range = None;
            if !chorded {
                self.caret = r.caret;
            }
        }
    }

    /// A grace note before the note at the caret (a rest is put there
    /// first if the voice has nothing starting at the caret). The caret
    /// stays, so the main note can follow.
    fn enter_grace(&mut self, step: i32, slash: bool) {
        let (lane, voice, caret, spec, alter) = (self.caret_lane, self.voice, self.caret, self.spec, self.accidental.map(i32::from));
        let by = self.written_interval(lane, caret);
        let perc = self.lane_is_percussion(lane);
        let mut focus = None;
        self.transact("Grace note", |m, ids| {
            if m.lanes.is_empty() {
                return false;
            }
            let lane = lane.min(m.lanes.len() - 1);
            let Some(vi) = voice_slot(m, lane, voice, true, ids) else { return false };
            let mut evs = m.lanes[lane].voices[vi].events.to_vec();
            if !evs.iter().any(|e| e.start == caret) {
                let r = edit::insert_event(&evs, edit::make_event(ids, caret, spec.value(), &[], 0), voice == 1, ids);
                extend_to(m, r.to);
                evs = r.events;
            }
            let fifths = if perc { 0 } else { corinzia_model::shift_key(key_fifths_at(m, caret), by.fifths()) };
            // Next to the last grace typed here, else the previous note.
            let reference = evs
                .iter()
                .find(|e| e.start == caret)
                .and_then(|e| e.graces.last())
                .and_then(|g| g.heads.first())
                .map(|h| h.pitch)
                .or_else(|| edit::previous_pitch(&evs, caret))
                .unwrap_or(Pitch { step: 6, alter: 0, octave: 4 })
                .transposed(by);
            let p = edit::nearest_pitch(step.rem_euclid(7), alter, fifths, reference).transposed(by.inverse());
            let base = 1u8 << (spec.base as u8).clamp(3, 6);
            let Some(r) = edit::add_grace(&evs, caret, &[p], base, slash, ids) else { return false };
            set_events(m, lane, vi, &r);
            focus = r.focus;
            true
        });
        if focus.is_some() {
            self.selection = focus;
            self.range = None;
        }
        self.accidental = None;
    }

    /// `/` (slashed) or `?` (plain) in note input: grace note input on or off.
    pub fn toggle_grace_input(&mut self, slash: bool) {
        self.grace_input = if self.grace_input == Some(slash) { None } else { Some(slash) };
        self.status = match self.grace_input {
            Some(true) => "Grace notes (slashed): letters add them before the next note · / again ends",
            Some(false) => "Grace notes: letters add them before the next note · ? again ends",
            None => "Grace notes off",
        }
        .into();
    }

    pub fn enter_rest(&mut self) {
        let (lane, voice, caret, spec, tuplet) = (self.caret_lane, self.voice, self.caret, self.spec, self.tuplet_input);
        let mut out = None;
        self.transact("Rest input", |m, ids| {
            if m.lanes.is_empty() {
                return false;
            }
            let lane = lane.min(m.lanes.len() - 1);
            let Some(vi) = voice_slot(m, lane, voice, true, ids) else { return false };
            let dur = input_duration(m, lane, vi, caret, spec, tuplet, ids);
            let evs = m.lanes[lane].voices[vi].events.clone();
            let r = edit::insert_event(&evs, edit::make_event(ids, caret, dur, &[], 0), voice == 1, ids);
            extend_to(m, r.to);
            set_events(m, lane, vi, &r);
            out = Some(r);
            true
        });
        if let Some(r) = out {
            self.selection = r.focus;
            self.range = None;
            self.caret = r.caret;
        }
    }

    /// Mouse input: a click on staff `lane` at written position `quarter`
    /// and staff step `diatonic` (C4 = 28).
    pub fn place_note_at(&mut self, lane: usize, quarter: f64, diatonic: i32, add_to_chord: bool) {
        let perc = self.lane_is_percussion(lane);
        // Snap to the grid of the current (undotted) duration, biased left so
        // a click just after a beat still lands on it.
        let grid = DurationSpec { dots: 0, ..self.spec }.value();
        let grid_q = grid.num as f64 / grid.den as f64 * 4.0;
        let k = (quarter / grid_q + 0.35).floor().max(0.0) as i64;
        self.caret_lane = lane;
        self.caret = grid * Frac::int(k);
        // Inside a tuplet, snap to its notes instead.
        if let Some(t) = self.movement().and_then(|m| m.lanes.get(lane)).and_then(|l| l.voices.iter().find(|v| v.index == self.voice)).and_then(|v| edit::tuplets::tuplet_at(&v.tuplets, self.caret)) {
            let step = t.length * Frac::new(1, t.actual);
            let q = |f: Frac| f.num as f64 / f.den as f64 * 4.0;
            let k = ((quarter - q(t.start)) / q(step) + 0.35).floor().clamp(0.0, (t.actual - 1) as f64) as i64;
            self.caret = t.start + step * Frac::int(k);
        }
        let (voice, caret, spec, accidental, chord, tuplet) = (self.voice, self.caret, self.spec, self.accidental, add_to_chord || self.chord_mode, self.tuplet_input);
        let step = diatonic.rem_euclid(7);
        let mut out = None;
        self.transact("Note input", |m, ids| {
            let Some(vi) = voice_slot(m, lane, voice, true, ids) else { return false };
            let evs = m.lanes[lane].voices[vi].events.clone();
            let alter = accidental.map(i32::from).unwrap_or_else(|| edit::key_alter(if perc { 0 } else { key_fifths_at(m, caret) }, step));
            let p = edit::from_diatonic(diatonic, alter);
            let onset = edit::event_at(&evs, caret).filter(|e| e.start == caret && !e.heads.is_empty());
            let r = if chord && onset.is_some() {
                match edit::add_to_chord(&evs, caret, p, ids) {
                    Some(r) => r,
                    None => return false,
                }
            } else {
                let dur = input_duration(m, lane, vi, caret, spec, tuplet, ids);
                let evs = m.lanes[lane].voices[vi].events.clone();
                let r = edit::insert_event(&evs, edit::make_event(ids, caret, dur, &[p], 0), voice == 1, ids);
                extend_to(m, r.to);
                r
            };
            set_events(m, lane, vi, &r);
            out = Some(r);
            true
        });
        if let Some(r) = out {
            self.selection = r.focus;
            self.range = None;
            self.caret = r.caret;
        }
        self.accidental = None;
    }

    // ------------------------------------------------------------- caret --
    pub fn move_caret(&mut self, direction: i64) {
        let len = self.movement().map(|m| m.length).unwrap_or(Frac::ZERO);
        let next = self.caret + self.grid_value() * Frac::int(direction);
        self.caret = next.max(Frac::ZERO).min(len);
    }

    pub fn move_caret_staff(&mut self, direction: i64) {
        let Some(n) = self.movement().map(|m| m.lanes.len()) else { return };
        if n == 0 {
            return;
        }
        self.caret_lane = (self.caret_lane as i64 + direction).clamp(0, n as i64 - 1) as usize;
        self.voice = 1;
    }

    /// Input mode: deletes the event before the caret and steps back.
    pub fn backspace(&mut self) {
        let (lane, voice, caret) = (self.caret_lane, self.voice, self.caret);
        let Some(m) = self.movement() else { return };
        let Some(v) = m.lanes.get(lane).and_then(|l| l.voices.iter().find(|v| v.index == voice)) else { return };
        // Grace notes just typed at the caret go first, last one first.
        if let Some(g) = v.events.iter().find(|e| e.start == caret).and_then(|e| e.graces.last()).map(|g| g.id) {
            self.transact("Delete grace note", |m, ids| {
                let Some(vi) = voice_slot(m, lane, voice, false, ids) else { return false };
                let evs = m.lanes[lane].voices[vi].events.clone();
                match edit::delete_element(&evs, g, voice == 1, ids) {
                    Some(r) => {
                        set_events(m, lane, vi, &r);
                        true
                    }
                    None => false,
                }
            });
            return;
        }
        let Some(prev) = v.events.iter().filter(|e| e.start < caret && e.end() <= caret).last().cloned() else { return };
        // A voice-1 rest is already "empty": just step over it.
        if !(prev.kind == EventKind::Rest && voice == 1) {
            self.transact("Delete", |m, ids| {
                let Some(vi) = voice_slot(m, lane, voice, false, ids) else { return false };
                let evs = m.lanes[lane].voices[vi].events.clone();
                match edit::delete_element(&evs, prev.id, voice == 1, ids) {
                    Some(r) => {
                        set_events(m, lane, vi, &r);
                        true
                    }
                    None => false,
                }
            });
        }
        self.caret = prev.start;
    }

    // --------------------------------------------------------- navigation --
    /// What arrow keys select for an event: a single note's head, else the
    /// event (a chord or a rest).
    fn nav_id(e: &Event) -> Id {
        if e.heads.len() == 1 { e.heads[0].id } else { e.id }
    }

    /// The voice the arrows walk: the selection's, else the caret's.
    fn nav_voice(&self) -> Option<(usize, usize)> {
        if let Some(loc) = self.selection.and_then(|id| self.locate(id)) {
            return Some((loc.lane, loc.voice));
        }
        let m = self.movement()?;
        let lane = m.lanes.get(self.caret_lane)?;
        let vi = lane.voices.iter().position(|v| v.index == self.voice).or_else(|| (!lane.voices.is_empty()).then_some(0))?;
        Some((self.caret_lane, vi))
    }

    /// Left/Right: the previous or next note or rest in the selected voice;
    /// with nothing selected, the one at the caret (or a passage's edge).
    /// `bar` steps to the first event of the previous or next bar.
    pub fn select_step(&mut self, dir: i64, bar: bool) {
        if let Some(r) = self.range.filter(|_| self.selection.is_none()) {
            // Collapse a passage to its first or last event.
            self.caret_lane = r.lane_from;
            self.caret = if dir < 0 { r.from } else { r.to };
            self.range = None;
            self.range_anchor = None;
        }
        let Some((lane, vi)) = self.nav_voice() else { return };
        let Some(m) = self.movement() else { return };
        let evs = &m.lanes[lane].voices[vi].events;
        if evs.is_empty() {
            return;
        }
        let current = self.selection.and_then(|id| evs.iter().position(|e| e.id == id || e.heads.iter().any(|h| h.id == id)));
        let target = match (current, bar) {
            (Some(i), false) => (i as i64 + dir).clamp(0, evs.len() as i64 - 1) as usize,
            (_, true) => {
                let at = current.map(|i| evs[i].start).unwrap_or(self.caret);
                let bars = edit::bar_starts(m);
                let goal = if dir < 0 {
                    // The start of this bar, or of the previous one when already there.
                    bars.iter().copied().rev().find(|&b| b < at).unwrap_or(Frac::ZERO)
                } else {
                    bars.iter().copied().find(|&b| b > at).unwrap_or(m.length)
                };
                evs.iter().position(|e| e.start >= goal).unwrap_or(evs.len() - 1)
            }
            (None, false) => {
                let at = self.caret;
                evs.iter().position(|e| e.start <= at && at < e.end()).or_else(|| evs.iter().rposition(|e| e.start < at)).unwrap_or(0)
            }
        };
        let id = Self::nav_id(&evs[target]);
        self.range_anchor = None;
        self.select(Some(id));
    }

    /// Home/End: the first or last event of the selected (or caret's) voice.
    pub fn select_edge(&mut self, last: bool) {
        let Some((lane, vi)) = self.nav_voice() else { return };
        let Some(m) = self.movement() else { return };
        let evs = &m.lanes[lane].voices[vi].events;
        let Some(e) = (if last { evs.last() } else { evs.first() }) else { return };
        let id = Self::nav_id(e);
        self.range_anchor = None;
        self.select(Some(id));
    }

    /// Alt+Up/Down: the event sounding at the same time on the staff above
    /// or below (the caret staff when nothing is selected).
    pub fn select_staff(&mut self, dir: i64) {
        let Some((lane, at)) = self.selection.and_then(|id| self.event_of(id)).map(|(loc, e)| (loc.lane, e.start)) else {
            self.move_caret_staff(dir);
            return;
        };
        let Some(m) = self.movement() else { return };
        let to = lane as i64 + dir;
        if to < 0 || to as usize >= m.lanes.len() {
            return;
        }
        let l = &m.lanes[to as usize];
        let Some(v) = l.voices.iter().find(|v| v.index == 1).or(l.voices.first()) else { return };
        let Some(e) = v.events.iter().find(|e| e.start <= at && at < e.end()).or(v.events.last()) else { return };
        let id = Self::nav_id(e);
        self.select(Some(id));
    }

    /// Shift+Left/Right: grows or shrinks a passage by one event on the
    /// caret's staff, from the selection (or the caret) as anchor.
    pub fn extend_step(&mut self, dir: i64) {
        let (lane, anchor, mut focus) = match (self.range, self.range_anchor) {
            (Some(r), Some((lane, a))) => (lane, a, if a == r.from { r.to } else { r.from }),
            _ => match self.selection.and_then(|id| self.event_of(id)).map(|(loc, e)| (loc.lane, e.start, e.end())) {
                Some((lane, a, b)) => (lane, if dir > 0 { a } else { b }, if dir > 0 { b } else { a }),
                None => (self.caret_lane, self.caret, self.caret),
            },
        };
        let Some(m) = self.movement() else { return };
        let Some(l) = m.lanes.get(lane) else { return };
        // Event boundaries on the staff (all voices).
        let mut edges: Vec<Frac> = l.voices.iter().flat_map(|v| v.events.iter().flat_map(|e| [e.start, e.end()])).collect();
        edges.sort();
        edges.dedup();
        let next = if dir > 0 { edges.iter().copied().find(|&t| t > focus) } else { edges.iter().copied().rev().find(|&t| t < focus) };
        let Some(next) = next else { return };
        focus = next;
        if focus == anchor {
            // Never collapse to nothing: step past the anchor.
            let past = if dir > 0 { edges.iter().copied().find(|&t| t > anchor) } else { edges.iter().copied().rev().find(|&t| t < anchor) };
            let Some(p) = past else { return };
            focus = p;
        }
        let (from, to) = (anchor.min(focus), anchor.max(focus));
        self.set_range(Range { lane_from: lane, lane_to: lane, from, to });
        self.range_anchor = Some((lane, anchor));
        self.caret = focus;
    }

    // ---------------------------------------------------------- selection --
    pub fn select(&mut self, id: Option<Id>) {
        self.selection = id;
        self.range = None;
        self.picked.clear();
        self.slur = None;
        if let Some((lane, start)) = id.and_then(|id| self.event_of(id)).map(|(loc, e)| (loc.lane, e.start)) {
            if !self.input_mode {
                self.caret_lane = lane;
                self.caret = start;
            }
        }
    }

    pub fn clear_selection(&mut self) {
        self.selection = None;
        self.range = None;
        self.picked.clear();
        self.range_anchor = None;
        self.slur = None;
    }

    pub fn select_all(&mut self) {
        let Some(m) = self.movement() else { return };
        if m.lanes.is_empty() {
            return;
        }
        let r = Range { lane_from: 0, lane_to: m.lanes.len() - 1, from: Frac::ZERO, to: m.length };
        self.set_range(r);
    }

    fn set_range(&mut self, r: Range) {
        self.selection = None;
        self.picked.clear();
        self.caret_lane = r.lane_from;
        self.caret = r.from;
        self.range = Some(r);
    }

    /// The bar at `at` on `lane` as a passage (a click on its empty space);
    /// `extend` adds it to the passage selected.
    pub fn select_bar(&mut self, lane: usize, at: Frac, extend: bool) {
        let Some(m) = self.movement() else { return };
        if lane >= m.lanes.len() || at < Frac::ZERO || at >= m.length {
            self.clear_selection();
            return;
        }
        let from = edit::bar_start_at_or_before(m, at);
        let to = (from + edit::bar_length(m, from)).min(m.length);
        let r = match (extend, self.range) {
            (true, Some(r)) => Range { lane_from: r.lane_from.min(lane), lane_to: r.lane_to.max(lane), from: r.from.min(from), to: r.to.max(to) },
            _ => Range { lane_from: lane, lane_to: lane, from, to },
        };
        self.set_range(r);
        self.range_anchor = None;
        let (b0, b1) = (edit::bar_number(self.movement().unwrap(), r.from).0, edit::bar_number(self.movement().unwrap(), r.to - Frac::new(1, 1024)).0);
        self.status = if b0 == b1 { format!("Bar {b0}") } else { format!("Bars {b0}–{b1}") };
    }

    /// The union of the staves and time spans of these notes and rests (plus
    /// the current selection when extending): marquee and shift-click.
    pub fn select_elements(&mut self, ids: &[Id], extend: bool) {
        let mut acc: Option<Range> = None;
        let mut include = |lane: usize, a: Frac, b: Frac| {
            acc = Some(match acc {
                None => Range { lane_from: lane, lane_to: lane, from: a, to: b },
                Some(r) => Range { lane_from: r.lane_from.min(lane), lane_to: r.lane_to.max(lane), from: r.from.min(a), to: r.to.max(b) },
            });
        };
        let mut all = ids.to_vec();
        if extend {
            if let Some(s) = self.selection {
                all.push(s);
            }
            if let Some(r) = self.range {
                include(r.lane_from, r.from, r.to);
                include(r.lane_to, r.from, r.to);
            }
        }
        for id in all {
            if let Some((loc, e)) = self.event_of(id) {
                include(loc.lane, e.start, e.end());
            }
        }
        match acc {
            Some(r) => self.set_range(r),
            None => self.clear_selection(),
        }
    }

    /// Applies `f` to every voice in the passage: one undo step.
    fn edit_range_voices(&mut self, label: &str, mut f: impl FnMut(&[Event], u8, &mut Ids) -> Option<EditResult>) -> bool {
        let Some(r) = self.range else { return false };
        self.transact(label, |m, ids| {
            let mut any = false;
            for li in r.lane_from..=r.lane_to.min(m.lanes.len().saturating_sub(1)) {
                for vi in 0..m.lanes[li].voices.len() {
                    let v = &m.lanes[li].voices[vi];
                    if let Some(res) = f(&v.events.clone(), v.index, ids) {
                        set_events(m, li, vi, &res);
                        any = true;
                    }
                }
            }
            any
        })
    }

    /// Edits the voice holding the selected element.
    fn edit_selected(&mut self, label: &str, f: impl FnOnce(&[Event], Id, u8, &mut Ids) -> Option<EditResult>) -> Option<EditResult> {
        let id = self.selection?;
        let loc = self.locate(id)?;
        let mut out = None;
        self.transact(label, |m, ids| {
            let v = &m.lanes[loc.lane].voices[loc.voice];
            match f(&v.events.clone(), id, v.index, ids) {
                Some(r) => {
                    set_events(m, loc.lane, loc.voice, &r);
                    out = Some(r);
                    true
                }
                None => false,
            }
        });
        out
    }

    pub fn delete_selection(&mut self) {
        if !self.picked.is_empty() {
            self.delete_picked();
            return;
        }
        if let Some(r) = self.range {
            self.edit_range_voices("Delete", |evs, index, ids| Some(edit::erase_range(evs, r.from, r.to, index == 1, ids)));
            // Tuplets inside the passage go with it.
            if let Some(m) = self.project.movements.get_mut(self.movement) {
                for li in r.lane_from..=r.lane_to.min(m.lanes.len().saturating_sub(1)) {
                    for v in m.lanes[li].voices.iter_mut() {
                        v.tuplets = edit::tuplets::remove_within(&v.tuplets, r.from, r.to);
                    }
                }
            }
            return;
        }
        if self.delete_structure_item() {
            return;
        }
        if let Some(r) = self.edit_selected("Delete", |evs, id, index, ids| edit::delete_element(evs, id, index == 1, ids)) {
            self.selection = r.focus;
        }
    }

    pub fn toggle_articulation(&mut self, flag: u16) {
        if !self.picked.is_empty() {
            let Some(m) = self.movement() else { return };
            let set = !self.picked_events(m).iter().all(|(_, e, _)| e.articulations & flag != 0);
            self.modify_picked("Articulation", |e, _| e.articulations = if set { e.articulations | flag } else { e.articulations & !flag });
            return;
        }
        if let Some(r) = self.range {
            // If every note in the passage already has it, remove it; else add it.
            let Some(m) = self.movement() else { return };
            let notes: Vec<bool> = (r.lane_from..=r.lane_to)
                .filter_map(|li| m.lanes.get(li))
                .flat_map(|l| l.voices.iter())
                .flat_map(|v| v.events.iter())
                .filter(|e| e.start >= r.from && e.start < r.to && !e.heads.is_empty())
                .map(|e| e.articulations & flag != 0)
                .collect();
            if notes.is_empty() {
                return;
            }
            let set = !notes.iter().all(|&b| b);
            self.edit_range_voices("Articulation", |evs, _, _| {
                edit::apply_to_range(evs, r.from, r.to, |e| {
                    if !e.heads.is_empty() {
                        e.articulations = if set { e.articulations | flag } else { e.articulations & !flag };
                    }
                })
            });
            return;
        }
        self.edit_selected("Articulation", |evs, id, _, _| edit::modify_event(evs, id, |e, _| e.articulations ^= flag));
    }

    /// `None` clears the dynamic.
    pub fn set_dynamic(&mut self, mark: Option<u8>) {
        let mark = mark.filter(|&m| (m as usize) < DYNAMICS.len());
        if !self.picked.is_empty() {
            self.modify_picked("Dynamic", |e, _| e.dynamic = mark);
            return;
        }
        // A selected dynamic: that one.
        if let Some(ev) = self.selection.and_then(ids::as_dynamic) {
            self.set_dynamic_of(ev, mark);
            return;
        }
        if let Some(r) = self.range {
            // On the first note of the passage on each staff.
            self.edit_range_voices("Dynamic", |evs, index, _| {
                if index != 1 {
                    return None;
                }
                let first = evs.iter().find(|e| e.start >= r.from && e.start < r.to && !e.heads.is_empty())?;
                edit::modify_event(evs, first.id, |e, _| e.dynamic = mark)
            });
            return;
        }
        self.edit_selected("Dynamic", |evs, id, _, _| edit::modify_event(evs, id, |e, _| e.dynamic = mark));
    }

    /// Beams by hand on the selected notes: `span::BEAM_BREAK` (a new beam
    /// starts here), `BEAM_JOIN` (the beam goes on into it), `NO_BEAM`, or
    /// 0 (automatic).
    pub fn set_beam(&mut self, mode: u8) {
        let set = |e: &mut Event| {
            if !e.is_rest() {
                e.spans = (e.spans & !span::BEAMS) | mode;
            }
        };
        let label = match mode {
            span::BEAM_BREAK => "Break beam",
            span::BEAM_JOIN => "Join beam",
            span::NO_BEAM => "No beam",
            _ => "Automatic beams",
        };
        if !self.picked.is_empty() {
            self.modify_picked(label, |e, _| set(e));
        } else if let Some(r) = self.range {
            self.edit_range_voices(label, |evs, _, _| edit::apply_to_range(evs, r.from, r.to, set));
        } else {
            self.edit_selected(label, |evs, id, _, _| edit::modify_event(evs, id, |e, _| set(e)));
        }
        self.status = label.into();
    }

    /// Ctrl+Shift+↑/↓: the selected notes onto the player's staff above or
    /// below (beams follow them), or back home.
    pub fn cross_staff(&mut self, dir: i8) {
        let Some(m) = self.movement() else { return };
        let targets: Vec<(usize, Id)> = if !self.picked.is_empty() {
            self.picked_events(m).iter().filter(|(_, e, _)| !e.is_rest()).map(|(li, e, _)| (*li, e.id)).collect()
        } else if let Some(r) = self.range {
            (r.lane_from..=r.lane_to.min(m.lanes.len().saturating_sub(1)))
                .flat_map(|li| m.lanes[li].voices.iter().flat_map(move |v| v.events.iter().map(move |e| (li, e))))
                .filter(|(_, e)| !e.is_rest() && e.start >= r.from && e.start < r.to)
                .map(|(li, e)| (li, e.id))
                .collect()
        } else {
            self.selection.and_then(|s| self.event_of(s)).filter(|(_, e)| !e.is_rest()).map(|(loc, e)| vec![(loc.lane, e.id)]).unwrap_or_default()
        };
        if targets.is_empty() {
            self.status = "Select some notes to move across staves".into();
            return;
        }
        let mut moved = 0;
        self.transact("Cross staff", |m, _| {
            for &(li, id) in &targets {
                let d = m.cross.iter().find(|c| c.0 == id).map_or(0, |c| c.1) as i64 + dir as i64;
                let to = li as i64 + d;
                if to < 0 || to as usize >= m.lanes.len() || m.lanes[to as usize].player != m.lanes[li].player {
                    continue;
                }
                m.cross.retain(|c| c.0 != id);
                if d != 0 {
                    m.cross.push((id, d as i8));
                }
                moved += 1;
            }
            moved > 0
        });
        self.status = if moved == 0 { "No staff of this player there".into() } else { format!("Cross staff: {moved} {}", if moved == 1 { "note" } else { "notes" }) };
    }

    pub fn toggle_tie(&mut self) {
        if let Some(r) = self.range {
            self.edit_range_voices("Tie", |evs, _, _| {
                edit::apply_to_range(evs, r.from, r.to, |e| {
                    let on = !e.heads.iter().all(|h| h.tie);
                    e.heads.iter_mut().for_each(|h| h.tie = on);
                })
            });
            return;
        }
        self.edit_selected("Tie", |evs, id, _, _| {
            edit::modify_event(evs, id, |e, head| match head {
                Some(i) => e.heads[i].tie = !e.heads[i].tie,
                None => e.heads.iter_mut().for_each(|h| h.tie = !h.tie),
            })
        });
    }

    /// True when a note, rest or passage is selected.
    pub fn has_selection(&self) -> bool {
        !self.picked.is_empty() || self.range.is_some() || self.selection.is_some_and(|id| self.event_of(id).is_some())
    }

    /// Changes the rhythm of the selection (a note or rest, or every event
    /// in a passage) with `f(old duration)`: see `edit::set_durations`.
    fn change_durations(&mut self, label: &str, f: impl Fn(Frac) -> Frac + Copy) -> bool {
        let (lanes, voice, from, to) = match (self.range, self.selection.and_then(|id| self.event_of(id))) {
            (Some(r), _) => ((r.lane_from, r.lane_to), None, r.from, r.to),
            (None, Some((loc, e))) => ((loc.lane, loc.lane), Some(loc.voice), e.start, e.end()),
            _ => return false,
        };
        let mut end = to;
        let done = self.transact(label, |m, ids| {
            let mut any = false;
            for li in lanes.0..=lanes.1.min(m.lanes.len().saturating_sub(1)) {
                for vi in 0..m.lanes[li].voices.len() {
                    if voice.is_some_and(|v| v != vi) {
                        continue;
                    }
                    let v = &m.lanes[li].voices[vi];
                    if let Some(r) = edit::set_durations(&v.events.clone(), from, to, f, v.index == 1, ids) {
                        extend_to(m, r.to);
                        set_events_tidy(m, li, vi, &r);
                        end = if any { end.max(r.caret) } else { r.caret };
                        any = true;
                    }
                }
            }
            any
        });
        if done {
            if let Some(r) = self.range {
                self.range = Some(Range { to: end, ..r });
            }
        }
        done
    }

    pub fn grid_value(&self) -> Frac {
        let (n, d, _) = GRIDS[self.grid.min(GRIDS.len() - 1)];
        Frac::new(n, d)
    }

    /// Alt+[ / Alt+]: a coarser or finer grid (the triplet grid is last).
    pub fn step_grid(&mut self, dir: i64) {
        self.grid = (self.grid as i64 + dir).clamp(0, GRIDS.len() as i64 - 1) as usize;
        self.status = format!("Rhythmic grid: {}", GRIDS[self.grid].2);
    }

    /// Alt+Left/Right: the selected note, rest or passage moves by the
    /// rhythmic grid, overwriting what it lands on.
    pub fn nudge(&mut self, dir: i64) {
        let by = self.grid_value() * Frac::int(dir);
        let (lanes, voice, from, to) = match (self.range, self.selection.and_then(|id| self.event_of(id))) {
            (Some(r), _) => ((r.lane_from, r.lane_to), None, r.from, r.to),
            (None, Some((loc, e))) => ((loc.lane, loc.lane), Some(loc.voice), e.start, e.end()),
            _ => return,
        };
        let done = self.transact("Move", |m, ids| {
            let mut any = false;
            for li in lanes.0..=lanes.1.min(m.lanes.len().saturating_sub(1)) {
                for vi in 0..m.lanes[li].voices.len() {
                    if voice.is_some_and(|v| v != vi) {
                        continue;
                    }
                    let v = &m.lanes[li].voices[vi];
                    if let Some(r) = edit::move_events(&v.events.clone(), from, to, by, v.index == 1, ids) {
                        extend_to(m, r.to);
                        let v = &mut m.lanes[li].voices[vi];
                        v.tuplets = edit::tuplets::moved(&v.tuplets, from, to, by);
                        set_events_tidy(m, li, vi, &r);
                        any = true;
                    }
                }
            }
            any
        });
        if done {
            if let Some(r) = self.range {
                self.range = Some(Range { from: r.from + by, to: r.to + by, ..r });
                self.caret = r.from + by;
            }
        } else {
            self.status = "Nothing to move there".into();
        }
    }

    /// A duration key or button with music selected changes its rhythm.
    pub fn set_selection_duration(&mut self) {
        let d = edit::DurationSpec { dots: 0, ..self.spec }.value() * self.selection_scale();
        if self.change_durations("Duration", |_| d) {
            self.status = "Duration changed".into();
        }
    }

    /// The dot key with music selected: plain → dotted → double-dotted.
    pub fn cycle_selection_dots(&mut self) {
        let scale = self.selection_scale();
        self.change_durations("Dots", move |d| edit::cycle_dots(d / scale) * scale);
    }

    /// Sounding per written time where the selection starts (tuplets).
    fn selection_scale(&self) -> Frac {
        let at = |lane: usize, voice: Option<usize>, t: Frac| -> Frac {
            let Some(l) = self.movement().and_then(|m| m.lanes.get(lane)) else { return Frac::int(1) };
            let v = voice.and_then(|v| l.voices.get(v)).or(l.voices.first());
            v.map(|v| edit::tuplets::sounding_scale(&v.tuplets, t)).unwrap_or(Frac::int(1))
        };
        match (self.range, self.selection.and_then(|id| self.event_of(id))) {
            (Some(r), _) => at(r.lane_from, None, r.from),
            (None, Some((loc, e))) => at(loc.lane, Some(loc.voice), e.start),
            _ => Frac::int(1),
        }
    }

    /// The tuplet popover: in note input, tuplet input of `actual:normal`
    /// (from the caret on); otherwise the selected note becomes the first
    /// note of a tuplet written in its value.
    pub fn apply_tuplet(&mut self, actual: i64, normal: i64) {
        if self.input_mode {
            self.tuplet_input = Some((actual, normal));
            self.status = format!("Tuplet input {actual}:{normal} · : ends it");
            return;
        }
        let Some((loc, e)) = self.selection.and_then(|id| self.event_of(id)) else {
            self.status = "Select a note, or start note input, to make a tuplet".into();
            return;
        };
        let (first, at) = (e.clone(), e.start);
        let scale = self.selection_scale();
        // The note's written value, undotted.
        let written = edit::plain_value(first.duration / scale);
        let mut focus = None;
        let ok = self.transact("Tuplet", |m, ids| {
            let v = &m.lanes[loc.lane].voices[loc.voice];
            let fill = v.index == 1;
            match edit::tuplets::create(&v.events, &v.tuplets, at, written, actual, normal, Some(&first), fill, ids) {
                Some((r, ts)) => {
                    extend_to(m, r.to);
                    m.lanes[loc.lane].voices[loc.voice].tuplets = ts;
                    set_events(m, loc.lane, loc.voice, &r);
                    focus = r.focus;
                    true
                }
                None => false,
            }
        });
        if ok {
            self.selection = focus;
            self.status = format!("Tuplet {actual}:{normal}");
        } else {
            self.status = "The tuplet does not fit there".into();
        }
    }

    /// Toggles triplet input (the panel's 3 button).
    pub fn toggle_triplets(&mut self) {
        self.tuplet_input = if self.tuplet_input.is_some() { None } else { Some((3, 2)) };
    }

    /// The written interval of the instrument playing `lane` at `at`, in a
    /// transposed score (else none).
    fn written_interval(&self, lane: usize, at: Frac) -> Interval {
        if !self.transposed {
            return Interval::default();
        }
        let Some(m) = self.movement() else { return Interval::default() };
        m.lanes.get(lane).and_then(|l| l.instrument_at(at)).and_then(|i| self.project.instrument(i)).map(|i| i.transposition).unwrap_or_default()
    }

    /// A letter with a note or rest selected (outside note input): the note
    /// takes that pitch nearest its old one; a rest becomes that note.
    pub fn repitch_selection(&mut self, step: i32) {
        let Some(id) = self.selection else { return };
        let Some((loc, e)) = self.event_of(id) else { return };
        let (start, lane, voice) = (e.start, loc.lane, loc.voice);
        let by = self.written_interval(lane, start);
        let perc = self.lane_is_percussion(lane);
        let Some(m) = self.movement() else { return };
        let fifths = if perc { 0 } else { corinzia_model::shift_key(key_fifths_at(m, start), by.fifths()) };
        let evs = m.lanes[lane].voices[voice].events.clone();
        let reference = match edit::find_event(&evs, id) {
            Some((e, Some(h))) => e.heads[h].pitch,
            Some((e, None)) if !e.heads.is_empty() => e.heads[0].pitch,
            _ => edit::previous_pitch(&evs, start).unwrap_or(Pitch { step: 6, alter: 0, octave: 4 }),
        }
        .transposed(by);
        let p = edit::nearest_pitch(step.rem_euclid(7), None, fifths, reference).transposed(by.inverse());
        let mut focus = id;
        self.edit_selected("Pitch", |evs, id, _, ids| {
            edit::modify_event(evs, id, |e, head| {
                match head {
                    Some(i) => e.heads[i].pitch = p,
                    None if e.heads.is_empty() => {
                        focus = ids.next();
                        e.heads.push(Head { id: focus, pitch: p, tie: false, velocity: 0 });
                        e.kind = corinzia_model::EventKind::Note;
                    }
                    None => {
                        // A whole chord moves by its lowest note.
                        let shift = edit::diatonic(p) - edit::diatonic(e.heads[0].pitch);
                        let lowest = e.heads[0].id;
                        for h in e.heads.iter_mut() {
                            h.pitch = if h.id == lowest { p } else { edit::from_diatonic(edit::diatonic(h.pitch) + shift, h.pitch.alter as i32) };
                        }
                    }
                }
                e.heads.sort_by_key(|h| h.pitch.midi_key());
            })
        });
        self.selection = Some(focus);
    }

    /// An accidental key with music selected (outside note input): the
    /// selected heads take it; pressing it again returns to the key's.
    pub fn set_selection_accidental(&mut self, alter: i8) {
        if !self.picked.is_empty() {
            self.modify_picked("Accidental", |e, heads| {
                for &k in heads {
                    let h = &mut e.heads[k];
                    h.pitch.alter = if h.pitch.alter == alter { 0 } else { alter };
                }
                e.heads.sort_by_key(|h| h.pitch.midi_key());
            });
            return;
        }
        let Some(m) = self.movement() else { return };
        let keys: Vec<(Frac, i32)> = m.keys.iter().map(|k| (k.at, k.fifths)).collect();
        let key_at = move |at: Frac| keys.iter().filter(|k| k.0 <= at).last().map(|k| k.1).unwrap_or(0);
        let respell = move |h: &mut Head, by: Interval, at: Frac| {
            let w = h.pitch.transposed(by);
            let fifths = corinzia_model::shift_key(key_at(at), by.fifths());
            let a = if w.alter == alter { edit::key_alter(fifths, w.step as i32) as i8 } else { alter };
            h.pitch = Pitch { alter: a, ..w }.transposed(by.inverse());
        };
        if let Some(r) = self.range {
            let intervals: Vec<Interval> = (r.lane_from..=r.lane_to).map(|li| self.written_interval(li, r.from)).collect();
            self.transact("Accidental", |m, _| {
                let mut any = false;
                for (k, lane) in (r.lane_from..=r.lane_to.min(m.lanes.len().saturating_sub(1))).enumerate() {
                    for v in m.lanes[lane].voices.iter_mut() {
                        let evs = std::sync::Arc::make_mut(&mut v.events);
                        for e in evs.iter_mut().filter(|e| e.start >= r.from && e.start < r.to) {
                            let at = e.start;
                            for h in e.heads.iter_mut() {
                                respell(h, intervals[k], at);
                                any = true;
                            }
                        }
                    }
                }
                any
            });
            return;
        }
        let Some(id) = self.selection else { return };
        let Some((loc, e)) = self.event_of(id) else { return };
        let interval = self.written_interval(loc.lane, e.start);
        self.edit_selected("Accidental", |evs, id, _, _| {
            edit::modify_event(evs, id, |e, head| {
                let at = e.start;
                match head {
                    Some(i) => respell(&mut e.heads[i], interval, at),
                    None => e.heads.iter_mut().for_each(|h| respell(h, interval, at)),
                }
                e.heads.sort_by_key(|h| h.pitch.midi_key());
            })
        });
    }

    /// Diatonic steps (±7 = an octave); a selected head moves alone.
    // ------------------------------------------------------------- layout --
    /// The score's and the parts' layouts and the page texts (subtitle,
    /// lyricist, arranger, copyright), as one undo step.
    pub fn set_layout(&mut self, layout: Layout, parts: Layout, texts: [String; 4]) {
        let p = &self.project;
        let unchanged = p.layout == layout && p.part_layout == parts && [&p.subtitle, &p.lyricist, &p.arranger, &p.copyright].iter().zip(&texts).all(|(a, b)| *a == b);
        if unchanged {
            self.status = "No changes to the layout".into();
            return;
        }
        let before = self.snapshot();
        let p = &mut self.project;
        p.layout = layout;
        p.part_layout = parts;
        let [subtitle, lyricist, arranger, copyright] = texts;
        (p.subtitle, p.lyricist, p.arranger, p.copyright) = (subtitle, lyricist, arranger, copyright);
        self.commit(before, "Layout");
        self.status = "Layout changed".into();
    }

    /// A system break (or page break) before the selected bar, or the
    /// caret's; in `part`'s part, or in the full score. Again removes it;
    /// the other kind there turns it into this one.
    pub fn toggle_break(&mut self, page: bool, part: Option<Id>) -> Result<(), String> {
        let m = self.movement().ok_or("No movement")?;
        let t = match (self.range, self.selection.and_then(|id| self.event_of(id))) {
            (Some(r), _) => r.from,
            (None, Some((_, e))) => e.start,
            _ => self.caret,
        };
        let bar = edit::bar_start_at_or_before(m, t.min(m.length));
        if bar == Frac::ZERO {
            return Err("A break goes before a bar after the first: select a later bar".into());
        }
        let n = edit::bar_number(m, bar).0;
        let what = if page { "Page break" } else { "System break" };
        let before = self.snapshot();
        let id = self.project.allocate_id();
        let m = &mut self.project.movements[self.movement];
        let label = match m.breaks.iter().position(|b| b.at == bar && b.part == part) {
            Some(i) if m.breaks[i].page == page => {
                m.breaks.remove(i);
                format!("{what} removed")
            }
            Some(i) => {
                m.breaks[i].page = page;
                what.to_string()
            }
            None => {
                m.breaks.push(Break { id, at: bar, page, part });
                m.breaks.sort_by_key(|b| b.at);
                what.to_string()
            }
        };
        self.commit(before, &label);
        let scope = if part.is_some() { " in the part" } else { "" };
        self.status = format!("{label} before bar {n}{scope}");
        Ok(())
    }

    // ------------------------------------------------------ explode/reduce --
    /// The passage's staves, its voice-1 index on each, checked for
    /// explode/reduce: two staves or more, no tuplets.
    fn staves_passage(&self, what: &str) -> Result<(Range, Vec<usize>), String> {
        let r = self.range.filter(|r| r.lane_to > r.lane_from).ok_or_else(|| format!("{what}: select a passage across two or more staves"))?;
        let m = self.movement().ok_or("No movement")?;
        let mut voice1 = Vec::new();
        for li in r.lane_from..=r.lane_to {
            let l = m.lanes.get(li).ok_or("No such staff")?;
            if l.voices.iter().any(|v| v.tuplets.iter().any(|t| t.start < r.to && t.end() > r.from)) {
                return Err(format!("{what} does not handle tuplets yet"));
            }
            voice1.push(l.voices.iter().position(|v| v.index == 1).ok_or("A staff has no voice 1")?);
        }
        Ok((r, voice1))
    }

    /// Spreads the chords of a staff in the passage over all its staves,
    /// top note to the top staff. One undo step.
    pub fn explode(&mut self) -> Result<(), String> {
        let (r, voice1) = self.staves_passage("Explode")?;
        let m = self.movement().unwrap();
        let has_chords = |li: usize| m.lanes[li].voices[voice1[li - r.lane_from]].events.iter().any(|e| e.start >= r.from && e.start < r.to && e.heads.len() > 1);
        let source = (r.lane_from..=r.lane_to).find(|&li| has_chords(li)).ok_or("Explode: no chords in the passage")?;
        let src = m.lanes[source].voices[voice1[source - r.lane_from]].events.clone();
        let parts = edit::staves::explode(&src, r.from, r.to, r.lane_to - r.lane_from + 1);
        self.transact("Explode", |m, ids| {
            for (j, clip) in parts.iter().enumerate() {
                let (li, vi) = (r.lane_from + j, voice1[j]);
                let res = edit::paste_events(&m.lanes[li].voices[vi].events, clip, r.from, r.to - r.from, true, ids);
                set_events(m, li, vi, &res);
            }
            true
        });
        self.status = format!("Exploded onto {} staves", parts.len());
        Ok(())
    }

    /// Combines the passage's staves into chords on one: an empty staff of
    /// the passage if there is one, else the top one. One undo step.
    pub fn reduce(&mut self) -> Result<(), String> {
        let (r, voice1) = self.staves_passage("Reduce")?;
        let m = self.movement().unwrap();
        let empty = |li: usize| m.lanes[li].voices.iter().all(|v| v.events.iter().all(|e| e.start < r.from || e.start >= r.to || e.is_rest()));
        let dest = (r.lane_from..=r.lane_to).find(|&li| empty(li)).unwrap_or(r.lane_from);
        let sources: Vec<std::sync::Arc<Vec<Event>>> = (r.lane_from..=r.lane_to).filter(|&li| li != dest || !empty(li)).flat_map(|li| m.lanes[li].voices.iter().map(|v| v.events.clone())).collect();
        let lists: Vec<&[Event]> = sources.iter().map(|v| v.as_slice()).collect();
        let clip = edit::staves::reduce(&lists, r.from, r.to);
        if clip.iter().all(|e| e.is_rest()) {
            return Err("Reduce: no notes in the passage".into());
        }
        let vi = voice1[dest - r.lane_from];
        self.transact("Reduce", |m, ids| {
            let res = edit::paste_events(&m.lanes[dest].voices[vi].events, &clip, r.from, r.to - r.from, true, ids);
            set_events(m, dest, vi, &res);
            true
        });
        let name = self.movement().and_then(|m| m.lanes[dest].changes.first()).and_then(|c| self.project.instrument(c.instrument)).map(|i| i.name.clone()).unwrap_or_default();
        self.status = format!("Reduced onto {name}");
        Ok(())
    }

    // ------------------------------------------------------------ filters --
    /// Picks what a filter query finds ("top", "voice 2", "above C5", "F#",
    /// "staccato", "rests", "dynamics mf"…) in the selected passage, among
    /// the elements already picked, or else in the whole movement.
    pub fn filter(&mut self, text: &str) -> Result<(), String> {
        let q = edit::select::parse_query(text)?;
        let m = self.movement().ok_or("No movement")?;
        if m.lanes.is_empty() {
            return Err("No staves".into());
        }
        let (lanes, from, to, scope) = match self.range {
            Some(r) => (r.lane_from..=r.lane_to, r.from, r.to, "in the passage"),
            None => (0..=m.lanes.len() - 1, Frac::ZERO, m.length, "in the movement"),
        };
        let mut found = edit::select::find(m, lanes, from, to, &q);
        if self.range.is_none() && !self.picked.is_empty() {
            found.retain(|id| self.picked.contains(id));
        }
        self.pick(found, &q.pick, scope)
    }

    /// Picks what is like the selected element in the movement (or the
    /// passage): notes on its staff, rests, the same dynamic, the same
    /// verse's lyrics, chord symbols, the same text.
    pub fn select_similar(&mut self) -> Result<(), String> {
        let id = self.selection.or(self.picked.first().copied()).ok_or("Select something first")?;
        let m = self.movement().ok_or("No movement")?;
        let all = 0..=m.lanes.len().saturating_sub(1);
        let (q, lanes) = if let Some(ev) = ids::as_dynamic(id) {
            let mark = self.event_of(ev).and_then(|(_, e)| e.dynamic);
            (edit::select::Query::of(edit::select::Pick::Dynamics(mark)), all)
        } else if let Some((_, verse)) = ids::as_lyric(id) {
            (edit::select::Query::of(edit::select::Pick::Lyrics(Some(verse))), all)
        } else if m.lanes.iter().any(|l| l.chords.iter().any(|c| c.id == id)) {
            (edit::select::Query::of(edit::select::Pick::ChordSymbols), all)
        } else if let Some(t) = m.lanes.iter().flat_map(|l| l.texts.iter()).find(|t| t.id == id) {
            (edit::select::Query::of(edit::select::Pick::Texts(Some(t.text.to_lowercase()))), all)
        } else if let Some((loc, e)) = self.event_of(id) {
            if e.is_rest() {
                (edit::select::Query::of(edit::select::Pick::Rests), all)
            } else {
                (edit::select::Query::notes(), loc.lane..=loc.lane)
            }
        } else {
            return Err("Nothing like it to select".into());
        };
        let (lanes, from, to, scope) = match self.range {
            Some(r) => (r.lane_from..=r.lane_to, r.from, r.to, "in the passage"),
            None if q.pick == edit::select::Pick::Notes => (lanes, Frac::ZERO, m.length, "on the staff"),
            None => (lanes, Frac::ZERO, m.length, "in the movement"),
        };
        let found = edit::select::find(m, lanes, from, to, &q);
        self.pick(found, &q.pick, scope)
    }

    fn pick(&mut self, found: Vec<Id>, what: &edit::select::Pick, scope: &str) -> Result<(), String> {
        if found.is_empty() {
            return Err(format!("No {} {scope}", edit::select::describe(what, 2).trim_start_matches("2 ")));
        }
        self.status = format!("{} picked {scope}", edit::select::describe(what, found.len()));
        self.selection = None;
        self.range = None;
        self.slur = None;
        self.picked = found;
        self.revision += 1; // shown, not changed
        Ok(())
    }

    /// The events holding picked noteheads (or picked dynamics, whose
    /// events they are): (lane, event, picked head indices).
    fn picked_events<'m>(&self, m: &'m Movement) -> Vec<(usize, &'m Event, Vec<usize>)> {
        let mut out = Vec::new();
        for (li, l) in m.lanes.iter().enumerate() {
            for v in &l.voices {
                for e in v.events.iter() {
                    let heads: Vec<usize> = e.heads.iter().enumerate().filter(|(_, h)| self.picked.contains(&h.id)).map(|(k, _)| k).collect();
                    if !heads.is_empty() || self.picked.contains(&ids::dynamic(e.id)) && !e.is_rest() {
                        out.push((li, e, heads));
                    }
                }
            }
        }
        out
    }

    /// Changes every event holding a picked note (given the picked heads'
    /// indices; all of them for a picked dynamic). One undo step.
    fn modify_picked(&mut self, label: &str, mut f: impl FnMut(&mut Event, &[usize])) -> bool {
        let before = self.snapshot();
        let picked = self.picked.clone();
        let Some(m) = self.project.movements.get_mut(self.movement) else { return false };
        let mut any = false;
        for l in m.lanes.iter_mut() {
            for v in l.voices.iter_mut() {
                let touched = v.events.iter().any(|e| e.heads.iter().any(|h| picked.contains(&h.id)) || (!e.is_rest() && picked.contains(&ids::dynamic(e.id))));
                if !touched {
                    continue;
                }
                let mut evs = (*v.events).clone();
                for e in evs.iter_mut() {
                    let mut heads: Vec<usize> = e.heads.iter().enumerate().filter(|(_, h)| picked.contains(&h.id)).map(|(k, _)| k).collect();
                    if heads.is_empty() && !e.is_rest() && picked.contains(&ids::dynamic(e.id)) {
                        heads = (0..e.heads.len()).collect();
                    }
                    if !heads.is_empty() {
                        f(e, &heads);
                        any = true;
                    }
                }
                v.events = Arc::new(evs);
            }
        }
        if any {
            self.commit(before, label);
        } else {
            self.project = before;
        }
        any
    }

    /// Deletes what is picked: notes (rests in voice 1), dynamics, lyrics,
    /// chord symbols and text. One undo step.
    fn delete_picked(&mut self) {
        let before = self.snapshot();
        let picked = std::mem::take(&mut self.picked);
        let mut ids = Ids(self.project.next_id);
        let Some(m) = self.project.movements.get_mut(self.movement) else { return };
        let mut n = 0;
        for id in &picked {
            if edit::remove_item(m, *id).is_some() {
                n += 1;
            }
        }
        for l in m.lanes.iter_mut() {
            for v in l.voices.iter_mut() {
                let mut evs = (*v.events).clone();
                let mut changed = false;
                for e in evs.iter_mut() {
                    if picked.contains(&ids::dynamic(e.id)) && e.dynamic.is_some() {
                        e.dynamic = None;
                        changed = true;
                        n += 1;
                    }
                    let before = e.lyrics.len();
                    e.lyrics.retain(|ly| !picked.contains(&ids::lyric(e.id, ly.verse)));
                    if e.lyrics.len() != before {
                        changed = true;
                        n += before - e.lyrics.len();
                    }
                }
                // Notes, one by one (a chord loses heads, the last leaves a rest).
                let heads: Vec<Id> = evs.iter().flat_map(|e| e.heads.iter().map(|h| h.id)).filter(|h| picked.contains(h)).collect();
                for h in heads {
                    if let Some(r) = edit::delete_element(&evs, h, v.index == 1, &mut ids) {
                        evs = r.events;
                        changed = true;
                        n += 1;
                    }
                }
                if changed {
                    v.events = Arc::new(evs);
                }
            }
        }
        self.project.next_id = ids.0;
        if n > 0 {
            self.commit(before, "Delete");
            self.status = format!("{n} deleted");
        } else {
            self.project = before;
        }
    }

    /// Transposes by an interval ("up M2", "down P5") or to a key ("to
    /// Eb", by the nearer way from the key in force): the selected passage
    /// (notes and chord symbols), or with nothing selected the whole score,
    /// key signatures included. One undo step.
    pub fn transpose(&mut self, text: &str) -> Result<(), String> {
        let spec = edit::parse_transpose(text).ok_or("Not understood: an interval (up M2, down P5, -m3, octave) or a key (to Eb)")?;
        let range = self.range;
        let m = self.movement().ok_or("No movement")?;
        let by = match spec {
            edit::TransposeSpec::By(i) => i,
            edit::TransposeSpec::To { step, alter } => {
                let at = range.map(|r| r.from).unwrap_or(Frac::ZERO);
                let k = m.keys.iter().rfind(|k| k.at <= at).copied().unwrap_or(Key { at: Frac::ZERO, fifths: 0, minor: false });
                edit::interval_between(edit::key_tonic(k.fifths, k.minor), (step, alter))
            }
        };
        if by.is_zero() {
            return Err("That is the key it is in".into());
        }
        if !self.picked.is_empty() {
            self.modify_picked("Transpose", |e, heads| {
                for &k in heads {
                    e.heads[k].pitch = e.heads[k].pitch.transposed(by);
                }
                e.heads.sort_by_key(|h| h.pitch.midi_key());
            });
            self.status = format!("{} notes transposed {}", self.picked.len(), interval_name(by));
            return Ok(());
        }
        let before = self.snapshot();
        let chord = |c: &mut ChordSymbol| c.text = notation::transpose_chord_text(&c.text, by);
        match range {
            Some(r) => {
                let m = &mut self.project.movements[self.movement];
                for li in r.lane_from..=r.lane_to.min(m.lanes.len().saturating_sub(1)) {
                    let lane = &mut m.lanes[li];
                    for v in lane.voices.iter_mut() {
                        if let Some(res) = edit::transpose_by(&v.events, r.from, r.to, by) {
                            v.events = Arc::new(res.events);
                        }
                    }
                    lane.chords.iter_mut().filter(|c| c.at >= r.from && c.at < r.to).for_each(chord);
                }
            }
            None => {
                for m in self.project.movements.iter_mut() {
                    for k in m.keys.iter_mut() {
                        k.fifths = shift_key(k.fifths, by.fifths());
                    }
                    for lane in m.lanes.iter_mut() {
                        for v in lane.voices.iter_mut() {
                            let mut evs = (*v.events).clone();
                            evs.iter_mut().for_each(|e| edit::transpose_event(e, by));
                            v.events = Arc::new(evs);
                        }
                        lane.chords.iter_mut().for_each(chord);
                    }
                }
            }
        }
        self.commit(before, "Transpose");
        let what = if range.is_some() { "The passage" } else { "The score" };
        self.status = format!("{what} transposed {}", interval_name(by));
        Ok(())
    }

    pub fn transpose_selection(&mut self, steps: i32) {
        if !self.picked.is_empty() {
            self.modify_picked("Transpose", |e, heads| {
                for &k in heads {
                    let h = &mut e.heads[k];
                    h.pitch = edit::from_diatonic(edit::diatonic(h.pitch) + steps, h.pitch.alter as i32);
                }
                e.heads.sort_by_key(|h| h.pitch.midi_key());
            });
            return;
        }
        if let Some(r) = self.range {
            self.edit_range_voices("Transpose", |evs, _, _| edit::transpose_range(evs, r.from, r.to, steps));
            return;
        }
        self.edit_selected("Transpose", |evs, id, _, _| {
            if edit::find_grace(evs, id).is_some() {
                return edit::modify_grace(evs, id, |h| h.pitch = edit::from_diatonic(edit::diatonic(h.pitch) + steps, h.pitch.alter as i32));
            }
            edit::modify_event(evs, id, |e, head| {
                let mv = |h: &mut Head| h.pitch = edit::from_diatonic(edit::diatonic(h.pitch) + steps, h.pitch.alter as i32);
                match head {
                    Some(i) => mv(&mut e.heads[i]),
                    None => e.heads.iter_mut().for_each(mv),
                }
                e.heads.sort_by_key(|h| h.pitch.midi_key());
            })
        });
    }

    // ---------------------------------------------------------- clipboard --
    pub fn has_clip(&self) -> bool {
        self.clip.is_some()
    }

    pub fn copy(&mut self) {
        if self.range.is_none() {
            if let Some(s) = self.selection {
                self.select_elements(&[s], false); // a single note copies as a passage
            }
        }
        let (Some(r), Some(m)) = (self.range, self.movement()) else { return };
        let lanes: Vec<Vec<(u8, Vec<Event>, Vec<Tuplet>)>> = (r.lane_from..=r.lane_to)
            .filter_map(|li| m.lanes.get(li))
            .map(|l| l.voices.iter().map(|v| (v.index, edit::slice_range(&v.events, r.from, r.to), edit::tuplets::slice(&v.tuplets, r.from, r.to))).collect())
            .collect();
        let length = r.to - r.from;
        let places = m.note_places();
        let inside = |id: &Id| {
            let &(li, vi, at) = places.get(id)?;
            (li >= r.lane_from && li <= r.lane_to && at >= r.from && at < r.to).then(|| (li - r.lane_from, m.lanes[li].voices[vi].index, at - r.from))
        };
        let slurs = m.slurs.iter().filter_map(|s| Some(([inside(&s.from)?, inside(&s.to)?], s.dir))).collect();
        let source = m.lanes.get(r.lane_from).and_then(|l| l.instrument_at(r.from)).and_then(|i| self.project.instrument(i)).map(|i| i.name.clone()).unwrap_or_default();
        self.status = format!("Copied {} {}, {} beats", lanes.len(), if lanes.len() == 1 { "staff" } else { "staves" }, quarters(length));
        self.clip = Some(Clip { lanes, length, slurs, source });
    }

    pub fn cut(&mut self) {
        self.copy();
        self.delete_selection();
    }

    pub fn paste(&mut self) {
        let Some(clip) = self.clip.clone() else { return };
        let Some(m) = self.movement() else { return };
        if m.lanes.is_empty() {
            return;
        }
        // Target: the passage, else the selected note, else the caret.
        let (mut lane0, mut at) = match self.range {
            Some(r) => (r.lane_from, r.from),
            None => (self.caret_lane.min(m.lanes.len() - 1), self.caret),
        };
        if self.range.is_none() {
            if let Some((loc, e)) = self.selection.and_then(|s| self.event_of(s)) {
                lane0 = loc.lane;
                at = e.start;
            }
        }
        let end = at + clip.length;
        let mut last_lane = lane0;
        self.transact("Paste", |m, ids| {
            extend_to(m, end);
            for (i, lane_clip) in clip.lanes.iter().enumerate() {
                let li = lane0 + i;
                if li >= m.lanes.len() {
                    break;
                }
                last_lane = li;
                for (index, events, tuplets) in lane_clip {
                    let Some(vi) = voice_slot(m, li, *index, !events.is_empty(), ids) else { continue };
                    let evs = m.lanes[li].voices[vi].events.clone();
                    let r = edit::paste_events(&evs, events, at, clip.length, *index == 1, ids);
                    let v = &mut m.lanes[li].voices[vi];
                    v.tuplets = edit::tuplets::paste(&v.tuplets, tuplets, at, clip.length);
                    set_events(m, li, vi, &r);
                }
            }
            // The passage's slurs, onto the pasted notes.
            for (ends, dir) in &clip.slurs {
                let note = |m: &Movement, (off, index, t): (usize, u8, Frac)| {
                    let v = m.lanes.get(lane0 + off)?.voices.iter().find(|v| v.index == index)?;
                    v.events.iter().find(|e| !e.is_rest() && e.start == at + t).map(|e| e.id)
                };
                if let (Some(from), Some(to)) = (note(m, ends[0]), note(m, ends[1])) {
                    m.slurs.retain(|s| (s.from, s.to) != (from, to));
                    m.slurs.push(Slur { id: ids.next(), from, to, dir: *dir });
                }
            }
            true
        });
        self.set_range(Range { lane_from: lane0, lane_to: last_lane, from: at, to: end });
    }

    /// Ctrl+Shift+V: the clipboard's first staff as a cue: small notes in
    /// voice 2 of the target staff, silent, under the source instrument's name.
    pub fn paste_cue(&mut self) {
        let Some(clip) = self.clip.clone() else { return };
        let Some(m) = self.movement() else { return };
        if m.lanes.is_empty() {
            return;
        }
        let (lane, at) = match (self.range, self.selection.and_then(|s| self.event_of(s))) {
            (Some(r), _) => (r.lane_from, r.from),
            (None, Some((loc, e))) => (loc.lane, e.start),
            _ => (self.caret_lane.min(m.lanes.len() - 1), self.caret),
        };
        let Some(voice) = clip.lanes.first().and_then(|l| l.iter().find(|(_, evs, _)| evs.iter().any(|e| !e.is_rest()))).cloned() else {
            self.status = "Nothing to cue: copy some notes first".into();
            return;
        };
        let end = at + clip.length;
        let player = m.lanes[lane].player;
        let source = clip.source.clone();
        self.transact("Cue", |m, ids| {
            extend_to(m, end);
            let cue: Vec<Event> = voice
                .1
                .iter()
                .map(|e| Event { articulations: e.articulations | art::CUE, dynamic: None, lyrics: vec![], graces: vec![], ..e.clone() })
                .collect();
            let Some(vi) = voice_slot(m, lane, 2, true, ids) else { return false };
            let evs = m.lanes[lane].voices[vi].events.clone();
            let r = edit::paste_events(&evs, &cue, at, clip.length, false, ids);
            let v = &mut m.lanes[lane].voices[vi];
            v.tuplets = edit::tuplets::paste(&v.tuplets, &voice.2, at, clip.length);
            set_events(m, lane, vi, &r);
            if !source.is_empty() {
                edit::add_text(m, player, at, &source, TextKind::Technique, ids);
            }
            true
        });
        self.set_range(Range { lane_from: lane, lane_to: lane, from: at, to: end });
        self.status = format!("Cue from {}", if source.is_empty() { "the clipboard" } else { &source });
    }

    /// R: the selection again, right after itself; the clipboard is kept.
    pub fn repeat_selection(&mut self) {
        if self.range.is_none() {
            if let Some(s) = self.selection {
                self.select_elements(&[s], false);
            }
        }
        let Some(r) = self.range else { return };
        let saved = self.clip.take();
        let status = self.status.clone();
        self.copy();
        self.range = Some(Range { from: r.to, to: r.to + (r.to - r.from), ..r });
        self.paste();
        if saved.is_some() {
            self.clip = saved;
        }
        self.status = status;
    }

    // ----------------------------------------------------------- spanners --
    fn range_from_selected_note(&mut self) {
        if self.range.is_some() {
            return;
        }
        let Some(id) = self.selection else { return };
        let Some(loc) = self.locate(id) else { return };
        let m = self.movement().unwrap();
        if let Some((a, b)) = span_from_note(&m.lanes[loc.lane].voices[loc.voice].events, id) {
            self.set_range(Range { lane_from: loc.lane, lane_to: loc.lane, from: a, to: b });
        }
    }

    /// S: a slur over the passage in each of its voices (from its first
    /// note to its last), inside or around slurs already there; the same
    /// slur again takes it away.
    pub fn add_slur(&mut self) {
        self.range_from_selected_note();
        let Some(r) = self.range else { return };
        let mut made = None;
        let ok = self.transact("Slur", |m, ids| {
            let mut changed = false;
            for li in r.lane_from..=r.lane_to.min(m.lanes.len().saturating_sub(1)) {
                for vi in 0..m.lanes[li].voices.len() {
                    let notes: Vec<Id> = m.lanes[li].voices[vi].events.iter().filter(|e| !e.is_rest() && e.start >= r.from && e.start < r.to).map(|e| e.id).collect();
                    let (Some(&from), Some(&to)) = (notes.first(), notes.last()) else { continue };
                    if from == to {
                        continue;
                    }
                    let n = m.slurs.len();
                    m.slurs.retain(|s| (s.from, s.to) != (from, to));
                    if m.slurs.len() == n {
                        let id = ids.next();
                        m.slurs.push(Slur { id, from, to, dir: 0 });
                        made = Some(id);
                    }
                    changed = true;
                }
            }
            changed
        });
        if !ok {
            self.status = "A slur needs at least two notes".into();
        }
        // A single new slur is selected, ready to have its ends moved.
        let single = r.lane_from == r.lane_to && self.movement().is_some_and(|m| m.lanes[r.lane_from].voices.iter().filter(|v| v.events.iter().filter(|e| !e.is_rest() && e.start >= r.from && e.start < r.to).count() > 1).count() == 1);
        if let (Some(id), true) = (made, single) {
            self.select_slur(id);
        }
    }

    pub fn add_hairpin(&mut self, crescendo: bool) {
        self.range_from_selected_note();
        let Some(r) = self.range else { return };
        let start = if crescendo { span::CRESC_START } else { span::DIM_START };
        // One hairpin per staff, on voice 1.
        let label = if crescendo { "Crescendo" } else { "Diminuendo" };
        if !self.edit_range_voices(label, |evs, index, _| if index == 1 { edit::mark_span(evs, r.from, r.to, start, span::HAIRPIN_END) } else { None }) {
            self.status = "A hairpin needs at least two notes in voice 1".into();
        }
    }

    // ---------------------------------------------------------- structure --
    /// Where a popover acts: the passage start, else the selected element,
    /// else the caret. (lane, position)
    pub fn structure_target(&self) -> Option<(usize, Frac)> {
        let m = self.movement()?;
        if let Some(r) = self.range {
            return Some((r.lane_from, r.from));
        }
        if let Some((loc, e)) = self.selection.and_then(|s| self.event_of(s)) {
            return Some((loc.lane, e.start));
        }
        Some((self.caret_lane.min(m.lanes.len().saturating_sub(1)), self.caret.min(m.length)))
    }

    /// Points the popovers at a clef, key or time signature or barline
    /// right-clicked at `pos` on a lane's staff: the caret moves to the
    /// change drawn there (or the bar it opens), and nothing stays selected.
    pub fn target_mark(&mut self, lane: usize, pos: Frac, kind: PopoverKind) {
        let Some(m) = self.movement() else { return };
        if lane >= m.lanes.len() {
            return;
        }
        // Signatures are drawn just before the bar they open.
        let ahead = (pos + Frac::new(1, 16)).min(m.length);
        let at = match kind {
            // A clef change stands just before its note; a clef at a
            // system's start belongs to the bar there.
            PopoverKind::Clef => m.lanes[lane].clefs.iter().map(|c| c.at).filter(|&at| at >= pos - Frac::new(1, 16) && at <= pos + Frac::new(1, 2)).min().unwrap_or_else(|| edit::bar_start_at_or_before(m, ahead)),
            PopoverKind::Barline => edit::nearest_barline(m, pos),
            _ => edit::bar_start_at_or_before(m, ahead),
        };
        self.clear_selection();
        self.caret_lane = lane;
        self.caret = at;
    }

    /// "Bar 5" for the popover title.
    pub fn popover_position(&self) -> String {
        match (self.structure_target(), self.movement()) {
            (Some((_, at)), Some(m)) => format!("Bar {}", edit::bar_number(m, at).0),
            _ => String::new(),
        }
    }

    // ------------------------------------------------------ lyrics, chords --
    /// The note lyrics go on: the selected note, else the first note at the
    /// popover target in voice 1 of its staff.
    pub fn lyric_target(&self) -> Option<Id> {
        if let Some((_, e)) = self.selection.and_then(|s| self.event_of(s)) {
            if !e.is_rest() {
                return self.selection;
            }
        }
        let (lane, at) = self.structure_target()?;
        let v = self.movement()?.lanes.get(lane)?.voices.iter().find(|v| v.index == 1)?;
        v.events.iter().find(|e| e.start >= at && !e.is_rest()).map(|e| e.heads[0].id)
    }

    /// Writes a syllable on the target note and moves to the next note.
    /// Empty text just moves on (a melisma). Returns the next note.
    pub fn enter_lyric(&mut self, verse: u8, text: &str, hyphen: bool, extend: bool) -> Result<Option<Id>, String> {
        let target = self.lyric_target().ok_or("Select a note for the lyrics")?;
        let loc = self.locate(target).ok_or("Select a note for the lyrics")?;
        if !text.is_empty() {
            let label = format!("Lyric \"{text}\"");
            self.transact(&label, |m, _| match edit::set_lyric(&m.lanes[loc.lane].voices[loc.voice].events.clone(), target, verse, text, hyphen, extend) {
                Some(r) => {
                    set_events(m, loc.lane, loc.voice, &r);
                    true
                }
                None => false,
            });
        }
        let m = self.movement().ok_or("No movement")?;
        let next = edit::next_note(&m.lanes[loc.lane].voices[loc.voice].events, target).map(|e| e.heads[0].id);
        self.selection = next.or(Some(target));
        self.range = None;
        Ok(next)
    }

    /// Moves the popover target to the next event (note or rest) on the staff.
    pub fn advance_target(&mut self) {
        let Some((lane, at)) = self.structure_target() else { return };
        let Some(v) = self.movement().and_then(|m| m.lanes.get(lane)).and_then(|l| l.voices.iter().find(|v| v.index == 1)) else { return };
        if let Some(e) = v.events.iter().find(|e| e.start > at) {
            let id = e.heads.first().map(|h| h.id).unwrap_or(e.id);
            self.selection = Some(id);
            self.range = None;
        }
    }

    /// Whole bars covering the passage, else the bar at `at`.
    fn selected_bars(&self, m: &Movement, at: Frac) -> (Frac, Frac) {
        let from = edit::bar_start_at_or_before(m, self.range.map(|r| r.from).unwrap_or(at));
        let mut to = from + edit::bar_length(m, from);
        if let Some(r) = self.range {
            if r.to > to {
                to = edit::bar_start_at_or_before(m, r.to);
                if to < r.to {
                    to = to + edit::bar_length(m, to);
                }
            }
        }
        (from, to.min(m.length))
    }

    /// Applies a popover; `Err` carries the hint to show.
    pub fn apply_popover(&mut self, kind: PopoverKind, input: &str) -> Result<(), String> {
        let text = input.trim().to_string();
        if kind == PopoverKind::Fingering {
            let fingers = edit::parse_fingering(&text).ok_or("Fingering not understood: 1, 2… or \"1 3 5\" for a chord")?;
            if self.range.is_none() && self.selection.is_none() {
                return Err("Select a note first".into());
            }
            self.edit_selected("Fingering", |evs, id, _, _| edit::modify_event(evs, id, |e, _| e.fingering = fingers.clone()));
            return Ok(());
        }
        // Bowings in the technique popover are marks on the notes.
        if kind == PopoverKind::Technique {
            if let Some(bit) = edit::parse_bowing(&text) {
                if self.range.is_none() && self.selection.is_none() {
                    return Err("Select notes first".into());
                }
                self.toggle_articulation(bit);
                return Ok(());
            }
        }
        if kind == PopoverKind::Instrument {
            return self.change_instrument(&text);
        }
        if kind == PopoverKind::Transpose {
            return self.transpose(&text);
        }
        if kind == PopoverKind::Filter {
            return self.filter(&text);
        }
        if kind == PopoverKind::MovementTitle {
            self.rename_movement(&text);
            return Ok(());
        }
        if kind == PopoverKind::Title || kind == PopoverKind::Composer {
            self.set_header(kind, &text);
            return Ok(());
        }
        if kind == PopoverKind::Dynamic {
            let mark = parse_dynamic(&text)?;
            if self.range.is_none() && self.selection.is_none() {
                return Err("Select a note first".into());
            }
            self.set_dynamic(mark);
            return Ok(());
        }
        if kind == PopoverKind::Tuplet {
            let (a, n) = edit::tuplets::parse_tuplet(&text).ok_or("Tuplet not understood: try 3, 5, 5:4 or 7:8")?;
            self.apply_tuplet(a, n);
            return Ok(());
        }
        if kind == PopoverKind::Ornament {
            let bit = edit::parse_ornament(&text).ok_or("Ornament not understood: tr, mordent, turn, trem, arp, breath, caesura, fermata")?;
            if self.range.is_none() && self.selection.is_none() {
                return Err("Select notes first".into());
            }
            self.toggle_articulation(bit);
            return Ok(());
        }
        let (lane, target) = self.structure_target().ok_or("No movement")?;
        let m = self.movement().ok_or("No movement")?;
        if m.lanes.is_empty() {
            return Err("No staves".into());
        }
        let bar = edit::bar_start_at_or_before(m, target);
        let player = m.lanes[lane].player;
        let bars = self.selected_bars(m, target);
        let mut at = target;
        let label: String;
        let change: Box<dyn FnOnce(&mut Movement, &mut Ids)> = match kind {
            PopoverKind::Key => {
                let k = edit::parse_key(&text).ok_or("Key not understood: try C, Eb, f# (minor) or \"a minor\"")?;
                at = bar;
                label = "Key signature".into();
                Box::new(move |m, _| edit::set_key(m, bar, k))
            }
            PopoverKind::Time if edit::parse_pickup(&text, Frac::new(1, 4)).is_some() => {
                let beat = m.meter.iter().find(|x| !x.pickup).map(|x| Frac::new(1, x.den)).unwrap_or(Frac::new(1, 4));
                let len = edit::parse_pickup(&text, beat).flatten();
                at = Frac::ZERO;
                label = if len.is_some() { "Pickup bar".into() } else { "Remove pickup".into() };
                Box::new(move |m, _| edit::set_pickup(m, len))
            }
            PopoverKind::Time => {
                let (n, d, beams) = edit::parse_time_grouped(&text).ok_or("Time signature not understood: try 3/4, 6/8, C, C|, \"7/8 2+2+3\" or \"pickup 1/4\"")?;
                at = bar;
                label = "Time signature".into();
                Box::new(move |m, _| {
                    edit::set_meter_grouped(m, bar, n, d, beams);
                    // Keep the movement a whole number of bars in the new meter.
                    let last = edit::bar_start_at_or_before(m, m.length);
                    if last != m.length {
                        m.length = last + edit::bar_length(m, last);
                    }
                })
            }
            PopoverKind::Clef | PopoverKind::Technique if line_text(kind, &text).is_some() => {
                // Octave lines and pedal over the passage, the selected note, or the bar.
                let (l0, l1, from, to) = match (self.range, self.selection.and_then(|id| self.event_of(id))) {
                    (Some(r), _) => (r.lane_from, r.lane_to, r.from, r.to),
                    (None, Some((loc, e))) => (loc.lane, loc.lane, e.start, e.end()),
                    _ => (lane, lane, bars.0, bars.1),
                };
                at = from;
                let line = line_text(kind, &text).unwrap();
                label = match line {
                    Some(LineKind::Pedal) => "Pedal".into(),
                    Some(k) => format!("{} line", k.code()),
                    None if kind == PopoverKind::Technique => "Pedal off".into(),
                    None => "Loco".into(),
                };
                Box::new(move |m, ids| {
                    for li in l0..=l1.min(m.lanes.len().saturating_sub(1)) {
                        match line {
                            Some(k) => edit::set_line(m, li, from, to, k, ids),
                            None => edit::remove_lines(m, li, from, to, kind == PopoverKind::Technique, ids),
                        }
                    }
                })
            }
            PopoverKind::Clef => {
                let c = edit::parse_clef(&text).ok_or("Clef not understood: treble, bass, alto, tenor, treble_8, bass_8")?;
                label = "Clef".into();
                Box::new(move |m, ids| edit::set_clef(m, player, target, c, ids))
            }
            PopoverKind::Barline => {
                let b = edit::parse_barline(&text).ok_or("Barline not understood: ||  |.  |:  :|  :|:")?;
                at = edit::nearest_barline(m, target);
                if at == Frac::ZERO && b != BarKind::RepeatStart {
                    at = (bar + edit::bar_length(m, bar)).min(m.length);
                }
                label = "Barline".into();
                let at = at;
                Box::new(move |m, ids| edit::set_barline(m, at, b, ids))
            }
            PopoverKind::Tempo => {
                let t = edit::parse_tempo(&text).ok_or("Tempo not understood: try \"Allegro\", \"Andante 84\" or \"q=96\"")?;
                label = "Tempo".into();
                Box::new(move |m, _| edit::set_tempo(m, target, &t))
            }
            PopoverKind::Expression | PopoverKind::Technique => {
                if text.is_empty() {
                    return Err("Type some text first".into());
                }
                let k = if kind == PopoverKind::Expression { TextKind::Expression } else { TextKind::Technique };
                label = if k == TextKind::Expression { "Expression text" } else { "Technique text" }.into();
                Box::new(move |m, ids| edit::add_text(m, player, target, &text, k, ids))
            }
            PopoverKind::Ending if edit::parse_nav(&text).is_some() => {
                let kind = edit::parse_nav(&text).unwrap();
                // Signs open their bar; words close it.
                let pos = if kind.at_bar_start() { bars.0 } else { bars.1 };
                at = pos;
                let k = kind;
                label = match kind {
                    Nav::Segno => "Segno".into(),
                    Nav::Coda => "Coda".into(),
                    k => k.text().to_string(),
                };
                Box::new(move |m, ids| edit::toggle_nav(m, pos, k, ids))
            }
            PopoverKind::Ending => {
                let passes = edit::parse_ending(&text).ok_or("Not understood: endings 1, 2, 1-3; or segno, coda, to coda, fine, D.C. al Fine, D.S. al Coda")?;
                at = bars.0;
                label = format!("Ending {}", edit::ending_label(passes));
                Box::new(move |m, ids| edit::set_ending(m, bars.0, bars.1, passes, ids))
            }
            PopoverKind::Rehearsal => {
                at = bar;
                label = if m.rehearsal.iter().any(|r| r.at == bar) { "Remove rehearsal mark" } else { "Rehearsal mark" }.into();
                Box::new(move |m, ids| edit::toggle_rehearsal_mark(m, bar, ids))
            }
            PopoverKind::Chord => {
                let c = edit::parse_chord(&text).ok_or("Chord symbol not understood: try C, F#m7, Bbmaj7, Dsus4, G7/B")?;
                label = format!("Chord symbol {c}");
                Box::new(move |m, ids| edit::set_chord(m, player, target, &c, ids))
            }
            PopoverKind::Lyrics => {
                let verse = self.lyric_verse;
                let (word, hyphen, extend) = match text.strip_suffix('-').map(|w| (w, true, false)).or(text.strip_suffix('_').map(|w| (w, false, true))) {
                    Some(x) => x,
                    None => (text.as_str(), false, false),
                };
                return self.enter_lyric(verse, word.trim_end(), hyphen, extend).map(|_| ());
            }
            PopoverKind::Ornament | PopoverKind::Tuplet | PopoverKind::MovementTitle | PopoverKind::Fingering | PopoverKind::Dynamic | PopoverKind::Title | PopoverKind::Composer | PopoverKind::Instrument | PopoverKind::Transpose | PopoverKind::Filter => unreachable!(),
        };
        self.transact(&label, |m, ids| {
            change(m, ids);
            true
        });
        let m = self.movement().unwrap();
        self.status = format!("{label} · bar {}", edit::bar_number(m, at.min(m.length)).0);
        Ok(())
    }

    pub fn add_first_second_endings(&mut self) {
        let Some((_, at)) = self.structure_target() else { return };
        let m = self.movement().unwrap();
        let (from, to) = self.selected_bars(m, at);
        // 1st ending = the selected bars, closed by an end repeat; 2nd = as
        // many bars after it, as far as the movement goes.
        let to2 = (to + (to - from)).min(m.length);
        if to >= to2 {
            self.status = "No room for a 2nd ending after the selection".into();
            return;
        }
        let (b0, b1) = (edit::bar_number(m, from).0, edit::bar_number(m, to2 - Frac::new(1, 1024)).0);
        self.transact("1st and 2nd endings", |m, ids| {
            edit::set_ending(m, from, to, 1, ids);
            edit::set_ending(m, to, to2, 2, ids);
            edit::set_barline(m, to, BarKind::RepeatEnd, ids);
            true
        });
        self.status = format!("1st and 2nd endings · bars {b0}–{b1}");
    }

    pub fn insert_bars(&mut self, count: i64) {
        let Some((_, target)) = self.structure_target() else { return };
        if count < 1 {
            return;
        }
        let m = self.movement().unwrap();
        let at = edit::bar_start_at_or_before(m, target);
        let len = edit::bar_length(m, at) * Frac::int(count);
        let bar = edit::bar_number(m, at).0;
        self.transact("Insert bars", |m, _| {
            for l in m.lanes.iter_mut() {
                for v in l.voices.iter_mut() {
                    v.events = Arc::new(edit::insert_span_events(&v.events, at, len));
                    v.tuplets = edit::tuplets::shift(&v.tuplets, at, len);
                }
            }
            edit::insert_span(m, at, len);
            true
        });
        self.status = format!("Inserted {count} bar{} before bar {bar}", if count == 1 { "" } else { "s" });
    }

    pub fn delete_bars(&mut self) {
        let Some((_, target)) = self.structure_target() else { return };
        let m = self.movement().unwrap();
        let (from, to) = self.selected_bars(m, target);
        if from >= to || to - from >= m.length {
            self.status = "A movement needs at least one bar".into();
            return;
        }
        let (b0, b1) = (edit::bar_number(m, from).0, edit::bar_number(m, to - Frac::new(1, 1024)).0);
        self.transact("Delete bars", |m, _| {
            for l in m.lanes.iter_mut() {
                for v in l.voices.iter_mut() {
                    v.events = Arc::new(edit::delete_span_events(&v.events, from, to));
                    v.tuplets = edit::tuplets::cut(&v.tuplets, from, to);
                }
            }
            edit::delete_span(m, from, to);
            true
        });
        self.range = None;
        self.selection = None;
        let len = self.movement().unwrap().length;
        self.caret = from.min(len);
        self.status = if b0 == b1 { format!("Deleted bar {b0}") } else { format!("Deleted bars {b0}–{b1}") };
    }

    /// A selected barline, mark, ending, clef change or text: removed.
    pub fn delete_structure_item(&mut self) -> bool {
        let Some(id) = self.selection else { return false };
        if self.delete_page_item(id) {
            return true;
        }
        let Some(what) = self.movement().and_then(|m| edit::describe_item(m, id)) else { return false };
        self.transact(&format!("Delete {what}"), |m, _| edit::remove_item(m, id).is_some());
        self.selection = None;
        self.status = format!("Deleted {what}");
        true
    }

    // --------------------------------------------------------- page items --
    // Titles, tempo marks, texts, chord symbols, dynamics and lyrics: the
    // things on the page that are words. A click selects them; a
    // double-click opens their popover with their text to retype.

    /// What editing `id` opens, and the text it starts with.
    pub fn item_text(&self, id: Id) -> Option<(PopoverKind, String)> {
        let m = self.movement()?;
        match id {
            ids::PROJECT_TITLE => return Some((PopoverKind::Title, self.project.title.clone())),
            ids::MOVEMENT_TITLE => return Some((PopoverKind::MovementTitle, m.title.clone())),
            ids::COMPOSER => return Some((PopoverKind::Composer, self.project.composer.clone())),
            _ => {}
        }
        if let Some(i) = ids::as_tempo(id) {
            let t = m.tempo.get(i)?;
            let bpm = t.bpm.round() as i64;
            return Some((PopoverKind::Tempo, if t.text.is_empty() { format!("q={bpm}") } else { format!("{} {bpm}", t.text) }));
        }
        if let Some(ev) = ids::as_dynamic(id) {
            let (_, e) = self.event_of(ev)?;
            return Some((PopoverKind::Dynamic, e.dynamic.and_then(|d| DYNAMICS.get(d as usize)).map(|d| d.to_string()).unwrap_or_default()));
        }
        if let Some((ev, verse)) = ids::as_lyric(id) {
            let (_, e) = self.event_of(ev)?;
            let l = e.lyrics.iter().find(|l| l.verse == verse)?;
            return Some((PopoverKind::Lyrics, format!("{}{}", l.text, if l.hyphen { "-" } else if l.extend { "_" } else { "" })));
        }
        for l in &m.lanes {
            if let Some(t) = l.texts.iter().find(|t| t.id == id) {
                return Some((if t.kind == TextKind::Expression { PopoverKind::Expression } else { PopoverKind::Technique }, edit::shown_text(t)));
            }
            if let Some(c) = l.chords.iter().find(|c| c.id == id) {
                return Some((PopoverKind::Chord, c.text.clone()));
            }
        }
        None
    }

    /// Replaces the text of the item `id` (see `item_text`); empty text
    /// removes a tempo mark, text, chord symbol, dynamic or lyric.
    pub fn edit_item(&mut self, id: Id, input: &str) -> Result<(), String> {
        let text = input.trim().to_string();
        let Some((kind, _)) = self.item_text(id) else { return Err("It is no longer there".into()) };
        match kind {
            PopoverKind::Title | PopoverKind::Composer => self.set_header(kind, &text),
            PopoverKind::MovementTitle => self.rename_movement(&text),
            PopoverKind::Dynamic => {
                let mark = parse_dynamic(&text)?;
                self.set_dynamic_of(ids::as_dynamic(id).unwrap(), mark);
            }
            PopoverKind::Lyrics => {
                let (ev, verse) = ids::as_lyric(id).unwrap();
                self.select(Some(ev));
                self.lyric_verse = verse;
                if text.is_empty() {
                    self.edit_selected("Lyric", |evs, id, _, _| edit::modify_event(evs, id, |e, _| e.lyrics.retain(|l| l.verse != verse)));
                } else {
                    self.apply_popover(PopoverKind::Lyrics, &text)?;
                }
            }
            PopoverKind::Tempo => {
                let i = ids::as_tempo(id).unwrap();
                if text.is_empty() {
                    self.delete_page_item(id);
                    return Ok(());
                }
                let t = edit::parse_tempo(&text).ok_or("Tempo not understood: try \"Allegro\", \"Andante 84\" or \"q=96\"")?;
                self.transact("Tempo", |m, _| {
                    let Some(old) = m.tempo.get_mut(i) else { return false };
                    old.text = t.text.clone();
                    old.bpm = t.bpm;
                    true
                });
            }
            PopoverKind::Expression | PopoverKind::Technique | PopoverKind::Chord => {
                if text.is_empty() {
                    self.selection = Some(id);
                    self.delete_structure_item();
                    return Ok(());
                }
                let chord = if kind == PopoverKind::Chord {
                    Some(edit::parse_chord(&text).ok_or("Chord symbol not understood: try C, F#m7, Bbmaj7, Dsus4, G7/B")?)
                } else {
                    None
                };
                let label = if chord.is_some() { "Chord symbol" } else { "Text" };
                self.transact(label, |m, _| {
                    let mut found = false;
                    for l in m.lanes.iter_mut() {
                        for t in l.texts.iter_mut().filter(|t| t.id == id) {
                            t.text = text.clone();
                            found = true;
                        }
                        for c in l.chords.iter_mut().filter(|c| c.id == id) {
                            c.text = chord.clone().unwrap_or_default();
                            found = true;
                        }
                    }
                    found
                });
            }
            _ => return Err("Not editable".into()),
        }
        self.status = format!("{} changed", kind.title());
        Ok(())
    }

    /// The project's title or composer.
    pub fn set_header(&mut self, kind: PopoverKind, text: &str) {
        let text = text.trim().to_string();
        fn field(p: &mut Project, kind: PopoverKind) -> &mut String {
            if kind == PopoverKind::Title { &mut p.title } else { &mut p.composer }
        }
        if *field(&mut self.project, kind) == text {
            return;
        }
        let before = self.snapshot();
        *field(&mut self.project, kind) = text;
        self.commit(before, kind.title());
    }

    fn set_dynamic_of(&mut self, event: Id, mark: Option<u8>) {
        let Some((loc, _)) = self.event_of(event) else { return };
        let before = self.snapshot();
        let m = &mut self.project.movements[self.movement];
        let v = &mut m.lanes[loc.lane].voices[loc.voice];
        let mut evs = (*v.events).clone();
        let Some(e) = evs.iter_mut().find(|e| e.id == event) else { return };
        if e.dynamic == mark {
            return;
        }
        e.dynamic = mark;
        v.events = Arc::new(evs);
        self.commit(before, if mark.is_some() { "Dynamic" } else { "Remove dynamic" });
        if mark.is_none() && self.selection == Some(ids::dynamic(event)) {
            self.selection = None;
        }
    }

    /// Deletes a dynamic, tempo mark, lyric or the title or composer.
    fn delete_page_item(&mut self, id: Id) -> bool {
        if let Some(ev) = ids::as_dynamic(id) {
            self.set_dynamic_of(ev, None);
            self.status = "Deleted dynamic".into();
            return true;
        }
        if let Some((ev, verse)) = ids::as_lyric(id) {
            self.select(Some(ev));
            self.edit_selected("Delete lyric", |evs, id, _, _| edit::modify_event(evs, id, |e, _| e.lyrics.retain(|l| l.verse != verse)));
            self.status = "Deleted lyric".into();
            return true;
        }
        if let Some(i) = ids::as_tempo(id) {
            // The first tempo stays (playback needs one); it loses its words.
            self.transact("Delete tempo mark", |m, _| {
                if i >= m.tempo.len() {
                    return false;
                }
                if m.tempo[i].at == Frac::ZERO {
                    m.tempo[i].text.clear();
                } else {
                    m.tempo.remove(i);
                }
                true
            });
            self.selection = None;
            self.status = "Deleted tempo mark".into();
            return true;
        }
        match id {
            ids::PROJECT_TITLE => self.set_header(PopoverKind::Title, ""),
            ids::COMPOSER => self.set_header(PopoverKind::Composer, ""),
            ids::MOVEMENT_TITLE => self.rename_movement(""),
            _ => return false,
        }
        self.selection = None;
        true
    }

    fn describe_page_item(&self, id: Id) -> Option<String> {
        let (kind, text) = self.item_text(id)?;
        let what = match kind {
            PopoverKind::Expression | PopoverKind::Technique | PopoverKind::Chord => return None, // edit::describe_item
            PopoverKind::Lyrics => format!("Lyric \"{text}\" · verse {}", ids::as_lyric(id)?.1),
            PopoverKind::Tempo => format!("Tempo {text}"),
            k => format!("{} {text}", k.title()),
        };
        Some(format!("{what} · double-click to edit"))
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
mod tests {
    use super::*;
    use crate::demo;
    use corinzia_score::score::art;

    fn f(n: i64, d: i64) -> Frac {
        Frac::new(n, d)
    }
    fn blank() -> Editor {
        Editor::new(demo::blank("Test", &[("Flute", "Fl.", "treble"), ("Cello", "Vc.", "bass")], 2))
    }
    fn events(e: &Editor, lane: usize, voice: usize) -> Vec<Event> {
        e.movement().unwrap().lanes[lane].voices[voice].events.to_vec()
    }
    fn notes(e: &Editor, lane: usize) -> Vec<String> {
        events(e, lane, 0).iter().map(|e| if e.is_rest() { "r".into() } else { e.heads.iter().map(|h| pitch_name(h.pitch)).collect::<Vec<_>>().join("+") }).collect()
    }

    #[test]
    fn transposed_input_types_written_pitches() {
        let mut e = Editor::new(crate::files::new_project("T", "Clarinet", 2).unwrap());
        e.transposed = true;
        e.set_input_mode(true);
        // Concert C major is D major for the clarinet: a typed C follows the
        // written key (C#5) and sounds a major second lower (B4).
        e.enter_note(0, false);
        e.enter_note(1, false); // written D5 sounds C5
        assert_eq!(notes(&e, 0)[..2], ["B4", "C5"]);
        e.transposed = false;
        e.enter_note(0, false); // concert C, next to C5
        assert_eq!(notes(&e, 0)[2], "C5");
    }

    /// Durations as quarters, rests as "r": "C5:1 r:1/2".
    fn rhythm(e: &Editor, lane: usize) -> Vec<String> {
        events(e, lane, 0)
            .iter()
            .map(|ev| {
                let q = ev.duration * Frac::int(4);
                let name = if ev.is_rest() { "r".to_string() } else { pitch_name(ev.heads[0].pitch) };
                if q.den == 1 { format!("{name}:{}", q.num) } else { format!("{name}:{}/{}", q.num, q.den) }
            })
            .collect()
    }

    fn quarters_cdef(e: &mut Editor) {
        e.set_input_mode(true);
        for step in [0, 1, 2, 3] {
            e.enter_note(step, false);
        }
        e.set_input_mode(false);
    }

    fn select_nth(e: &mut Editor, n: usize) {
        let ev = &events(e, 0, 0)[n];
        let id = ev.heads.first().map(|h| h.id).unwrap_or(ev.id);
        e.select(Some(id));
    }

    #[test]
    fn changing_a_selected_notes_duration() {
        let mut e = blank();
        quarters_cdef(&mut e);
        assert_eq!(rhythm(&e, 0)[..4], ["C5:1", "D5:1", "E5:1", "F5:1"]);
        // Shorter: D becomes an eighth and an eighth rest takes the rest of its beat.
        select_nth(&mut e, 1);
        e.spec.base = edit::BaseDuration::Eighth;
        e.set_selection_duration();
        assert_eq!(rhythm(&e, 0)[..5], ["C5:1", "D5:1/2", "r:1/2", "E5:1", "F5:1"]);
        assert_eq!(e.event_of(e.selection.unwrap()).unwrap().1.duration, f(1, 8), "D stays selected");
        // Longer: C becomes a half note and overwrites D and the rest.
        select_nth(&mut e, 0);
        e.spec.base = edit::BaseDuration::Half;
        e.set_selection_duration();
        assert_eq!(rhythm(&e, 0)[..3], ["C5:2", "E5:1", "F5:1"]);
        // Longer into the middle of a note: E's tail stays as a rest.
        select_nth(&mut e, 0);
        e.spec.base = edit::BaseDuration::Half;
        e.cycle_selection_dots(); // a dotted half: overwrites E, half of nothing else
        assert_eq!(rhythm(&e, 0)[..2], ["C5:3", "F5:1"]);
        // Dots cycle: dotted → double-dotted cuts into F, then back to plain.
        e.cycle_selection_dots();
        assert_eq!(rhythm(&e, 0)[..2], ["C5:7/2", "r:1/2"]);
        e.cycle_selection_dots();
        // The space left is one rest per bar (drawn as the meter wants).
        assert_eq!(rhythm(&e, 0)[..3], ["C5:2", "r:2", "r:4"]);
        // Undo walks back each step.
        e.undo();
        assert_eq!(rhythm(&e, 0)[..2], ["C5:7/2", "r:1/2"]);
        // The total stays two bars.
        let total: Frac = events(&e, 0, 0).iter().fold(Frac::ZERO, |a, ev| a + ev.duration);
        assert_eq!(total, Frac::int(2));
    }

    #[test]
    fn changing_durations_of_a_passage_and_at_the_end() {
        let mut e = blank();
        quarters_cdef(&mut e);
        // A passage of four quarters becomes four eighths and a half rest.
        e.select(None);
        e.set_range(Range { lane_from: 0, lane_to: 0, from: Frac::ZERO, to: Frac::int(1) });
        e.spec.base = edit::BaseDuration::Eighth;
        e.set_selection_duration();
        assert_eq!(rhythm(&e, 0)[..5], ["C5:1/2", "D5:1/2", "E5:1/2", "F5:1/2", "r:2"]);
        assert_eq!(e.range.unwrap().to, f(1, 2), "the passage follows the new rhythm");
        // A note on the last beat made a whole: the movement grows by a bar.
        e.set_input_mode(true);
        e.caret = f(7, 4);
        e.spec.base = edit::BaseDuration::Quarter;
        e.enter_note(4, false);
        e.set_input_mode(false);
        let n = events(&e, 0, 0).len();
        select_nth(&mut e, n - 1);
        e.spec.base = edit::BaseDuration::Whole;
        e.set_selection_duration();
        assert_eq!(e.movement().unwrap().length, Frac::int(3));
        assert_eq!(rhythm(&e, 0).last().unwrap(), "G5:4");
        // In voice 1 the rests between stay one per bar.
        assert_eq!(rhythm(&e, 0)[4..6], ["r:2".to_string(), "r:3".to_string()]);
    }

    #[test]
    fn letters_and_accidentals_change_the_selected_note() {
        let mut e = blank();
        quarters_cdef(&mut e);
        select_nth(&mut e, 1); // D5
        e.repitch_selection(6); // B nearest D5 is B4
        assert_eq!(rhythm(&e, 0)[1], "B4:1");
        e.set_selection_accidental(-1);
        assert_eq!(rhythm(&e, 0)[1], "B♭4:1");
        e.set_selection_accidental(-1); // again: back to the key's
        assert_eq!(rhythm(&e, 0)[1], "B4:1");
        // A rest becomes a note.
        let n = events(&e, 0, 0).len();
        select_nth(&mut e, n - 1);
        assert!(events(&e, 0, 0)[n - 1].is_rest());
        e.repitch_selection(4);
        assert!(!events(&e, 0, 0)[n - 1].is_rest());
        assert_eq!(e.event_of(e.selection.unwrap()).unwrap().1.heads.len(), 1);
    }

    #[test]
    fn alt_arrows_move_music_by_the_rhythmic_grid() {
        let mut e = blank();
        quarters_cdef(&mut e);
        assert_eq!(e.grid_value(), f(1, 8), "an eighth grid by default");
        // D moves an eighth later: an eighth rest opens before it, E loses its first half.
        select_nth(&mut e, 1);
        let id = e.selection.unwrap();
        e.nudge(1);
        assert_eq!(rhythm(&e, 0)[..5], ["C5:1", "r:1/2", "D5:1", "r:1/2", "F5:1"]);
        assert_eq!(e.selection, Some(id), "the note stays selected");
        assert_eq!(e.event_of(id).unwrap().1.start, f(3, 8));
        // And back: the rests join up again around it.
        e.nudge(-1);
        assert_eq!(rhythm(&e, 0)[..4], ["C5:1", "D5:1", "r:1", "F5:1"]);
        // Not before the start.
        select_nth(&mut e, 0);
        e.nudge(-1);
        assert_eq!(rhythm(&e, 0)[0], "C5:1");
        assert_eq!(e.status, "Nothing to move there");
        // A passage moves by a quarter grid.
        e.grid = 0;
        e.set_range(Range { lane_from: 0, lane_to: 0, from: Frac::ZERO, to: f(1, 2) });
        e.nudge(1);
        assert_eq!(rhythm(&e, 0)[..4], ["r:1", "C5:1", "D5:1", "F5:1"]);
        assert_eq!(e.range.map(|r| (r.from, r.to)), Some((f(1, 4), f(3, 4))));
        // The caret steps by the grid; Alt+] makes it finer.
        e.step_grid(1);
        e.caret = Frac::ZERO;
        e.move_caret(1);
        assert_eq!(e.caret, f(1, 8));
    }

    fn tuplets(e: &Editor) -> Vec<Tuplet> {
        e.movement().unwrap().lanes[0].voices[0].tuplets.clone()
    }

    #[test]
    fn tuplet_input_fills_tuplets_one_after_another() {
        let mut e = blank();
        e.set_input_mode(true);
        e.spec.base = edit::BaseDuration::Eighth;
        e.apply_popover(PopoverKind::Tuplet, "3").unwrap();
        for step in [0, 1, 2, 3, 4, 5] {
            e.enter_note(step, false);
        }
        // Two triplets of eighths, one per beat.
        let ts = tuplets(&e);
        assert_eq!(ts.len(), 2);
        assert_eq!((ts[0].start, ts[0].length, ts[1].start), (Frac::ZERO, f(1, 4), f(1, 4)));
        assert_eq!(rhythm(&e, 0)[..6].iter().map(|r| r.split(':').nth(1).unwrap()).collect::<Vec<_>>(), ["1/3"; 6]);
        assert_eq!(e.caret, f(1, 2));
        // ':' ends tuplet input: the next eighth is plain.
        e.tuplet_input = None;
        e.enter_note(6, false);
        assert_eq!(rhythm(&e, 0)[6], "B5:1/2");
        assert_eq!(tuplets(&e).len(), 2);
        // A quintuplet of sixteenths over the next beat.
        e.caret = f(3, 4);
        e.spec.base = edit::BaseDuration::Sixteenth;
        e.apply_popover(PopoverKind::Tuplet, "5").unwrap();
        for step in [0, 1, 2, 3, 4] {
            e.enter_note(step, false);
        }
        let q = tuplets(&e).into_iter().find(|t| t.actual == 5).unwrap();
        assert_eq!((q.start, q.length, q.normal), (f(3, 4), f(1, 4), 4));
        assert_eq!(e.caret, f(1, 1));
    }

    #[test]
    fn a_selected_note_becomes_a_tuplet_and_edits_keep_it_sound() {
        let mut e = blank();
        quarters_cdef(&mut e);
        // D (a quarter) starts a quarter-note triplet over two beats.
        select_nth(&mut e, 1);
        e.apply_popover(PopoverKind::Tuplet, "3").unwrap();
        assert_eq!(tuplets(&e), vec![Tuplet { start: f(1, 4), length: f(1, 2), actual: 3, normal: 2 }]);
        assert_eq!(rhythm(&e, 0)[..4], ["C5:1", "D5:2/3", "r:4/3", "F5:1"]);
        // Duration keys inside it are written values: an eighth is 1/12.
        e.spec.base = edit::BaseDuration::Eighth;
        e.set_selection_duration();
        assert_eq!(rhythm(&e, 0)[1], "D5:1/3");
        assert_eq!(tuplets(&e).len(), 1, "still inside");
        // A note now crossing its end removes it.
        e.spec.base = edit::BaseDuration::Whole;
        e.set_selection_duration();
        assert!(tuplets(&e).is_empty());
        e.undo();
        e.undo();
        assert_eq!(tuplets(&e).len(), 1);
        // Copy the triplet's beats, paste them on beat 1 of bar 2: it comes along.
        e.set_range(Range { lane_from: 0, lane_to: 0, from: f(1, 4), to: f(3, 4) });
        e.copy();
        e.set_range(Range { lane_from: 0, lane_to: 0, from: f(1, 1), to: f(1, 1) });
        e.paste();
        assert_eq!(tuplets(&e).iter().map(|t| t.start).collect::<Vec<_>>(), [f(1, 4), f(1, 1)]);
        // Deleting a passage holding it removes it; inserting a bar before moves it.
        e.set_range(Range { lane_from: 0, lane_to: 0, from: f(1, 1), to: f(3, 2) });
        e.delete_selection();
        assert_eq!(tuplets(&e).len(), 1);
        e.clear_selection();
        e.caret = Frac::ZERO;
        e.insert_bars(1);
        assert_eq!(tuplets(&e)[0].start, f(5, 4));
    }

    #[test]
    fn grace_notes_are_typed_before_their_note() {
        let mut e = blank();
        e.set_input_mode(true);
        e.enter_note(0, false); // C5
        e.spec.base = edit::BaseDuration::Sixteenth;
        e.toggle_grace_input(true);
        e.enter_note(4, false); // grace G
        e.enter_note(5, false); // grace A
        assert_eq!(e.caret, f(1, 4), "graces leave the caret where it is");
        e.toggle_grace_input(true);
        e.spec.base = edit::BaseDuration::Quarter;
        e.enter_note(6, false); // the main note B, after its graces
        let ev = &events(&e, 0, 0)[1];
        assert_eq!(pitch_name(ev.heads[0].pitch), "B4");
        let g: Vec<String> = ev.graces.iter().map(|g| pitch_name(g.heads[0].pitch)).collect();
        assert_eq!(g, ["G4", "A4"]);
        assert!(ev.graces.iter().all(|g| g.slash && g.base == 16));
        // A grace selected transposes alone; Backspace on the caret removes the last grace.
        let a = ev.graces[1].heads[0].id;
        e.set_input_mode(false);
        e.select(Some(a));
        e.transpose_selection(1);
        assert_eq!(pitch_name(events(&e, 0, 0)[1].graces[1].heads[0].pitch), "B4");
        e.set_input_mode(true);
        e.caret = f(1, 4);
        e.backspace();
        assert_eq!(events(&e, 0, 0)[1].graces.len(), 1);
        // Delete with the remaining grace selected removes it, not its note.
        e.set_input_mode(false);
        let g = events(&e, 0, 0)[1].graces[0].id;
        e.select(Some(g));
        e.delete_selection();
        assert!(events(&e, 0, 0)[1].graces.is_empty());
        assert!(!events(&e, 0, 0)[1].is_rest());
    }

    #[test]
    fn movements_are_added_switched_renamed_and_deleted() {
        let mut e = blank();
        quarters_cdef(&mut e);
        e.apply_popover(PopoverKind::Key, "D").unwrap();
        e.add_movement();
        assert_eq!(e.project.movements.len(), 2);
        assert_eq!(e.movement, 1);
        let m = e.movement().unwrap();
        // The same staves and key, empty.
        assert_eq!(m.lanes.len(), 2);
        assert_eq!(m.keys[0].fifths, 2);
        assert!(m.lanes.iter().all(|l| l.voices[0].events.is_empty()));
        assert_eq!(m.title, "Movement 2");
        e.apply_popover(PopoverKind::MovementTitle, "II. Scherzo").unwrap();
        assert_eq!(e.movement().unwrap().title, "II. Scherzo");
        // Notes typed here stay here.
        e.set_input_mode(true);
        e.enter_note(4, false);
        e.set_input_mode(false);
        e.switch_movement(0);
        assert_eq!(notes(&e, 0)[..4], ["C5", "D5", "E5", "F5"]);
        e.switch_movement(1);
        assert_eq!(notes(&e, 0)[0], "G4");
        // Delete it; undo brings it back.
        e.delete_movement();
        assert_eq!((e.project.movements.len(), e.movement), (1, 0));
        e.undo();
        assert_eq!(e.project.movements.len(), 2);
        assert_eq!(e.project.movements[1].title, "II. Scherzo");
        // The last movement cannot go.
        e.switch_movement(0);
        e.delete_movement();
        e.delete_movement();
        assert_eq!(e.project.movements.len(), 1);
        assert_eq!(e.status, "A score needs at least one movement");
        // The PDF has every movement, numbered on.
        e.undo();
        let pages = crate::files::work_pages(&e.project, false);
        assert!(pages.len() >= 2);
    }

    #[test]
    fn keys_reach_the_selection_outside_note_input() {
        use crate::keys::{map, Action};
        use iced::keyboard::{Key, Modifiers};
        assert_eq!(map(&Key::Character("e".into()), Modifiers::empty(), false), Some(Action::Pitch(2)));
        assert_eq!(map(&Key::Character("e".into()), Modifiers::empty(), true), Some(Action::Note { step: 2, chord: false }));
    }

    #[test]
    fn arrow_keys_walk_notes_bars_staves_and_passages() {
        let mut e = blank();
        e.set_input_mode(true);
        for step in [0, 1, 2, 3, 4, 5, 6, 0] {
            e.enter_note(step, false); // two bars of quarters: C D E F | G A B C
        }
        e.set_input_mode(false);
        e.clear_selection();
        e.caret = Frac::ZERO;
        let name = |e: &Editor| {
            let id = e.selection.unwrap();
            let (_, ev) = e.event_of(id).unwrap();
            pitch_name(ev.heads[0].pitch)
        };
        e.select_step(1, false); // nothing selected: the note at the caret
        assert_eq!(name(&e), "C5");
        e.select_step(1, false);
        e.select_step(1, false);
        assert_eq!(name(&e), "E5");
        e.select_step(-1, false);
        assert_eq!(name(&e), "D5");
        e.select_step(1, true); // next bar's first note
        assert_eq!(name(&e), "G5");
        e.select_step(-1, true); // back to this bar's start... already there: the previous bar's
        assert_eq!(name(&e), "C5");
        e.select_edge(true);
        assert_eq!(e.event_of(e.selection.unwrap()).unwrap().1.start, events(&e, 0, 0).last().unwrap().start);
        e.select_edge(false);
        assert_eq!(name(&e), "C5");
        // Shift+Right grows a passage by notes, Shift+Left shrinks it.
        e.extend_step(1);
        e.extend_step(1);
        let r = e.range.unwrap();
        assert_eq!((r.from, r.to), (Frac::ZERO, f(3, 4)));
        e.extend_step(-1);
        assert_eq!(e.range.unwrap().to, f(1, 2));
        // Right collapses the passage to its end.
        e.select_step(1, false);
        assert!(e.range.is_none() && e.selection.is_some());
        // Alt+Down: the cello's event at the same time (its whole-bar rest);
        // Alt+Up from there: the flute's note where that rest starts.
        e.select(Some(events(&e, 0, 0)[1].heads[0].id));
        e.select_staff(1);
        assert_eq!(e.locate(e.selection.unwrap()).unwrap().lane, 1);
        e.select_staff(-1);
        assert_eq!(name(&e), "C5");
    }

    #[test]
    fn typing_notes_follows_the_caret_and_octaves() {
        let mut e = blank();
        e.set_input_mode(true);
        for step in [0, 4, 2, 0] {
            e.enter_note(step, false); // C (near B4 -> C5), G, E, C
        }
        assert_eq!(notes(&e, 0)[..4], ["C5", "G4", "E4", "C4"]);
        assert_eq!(e.caret, f(1, 1));
        assert_eq!(e.caret_label(), "Bar 2, beat 1 · Flute · voice 1");
        // Voice 1 stays contiguous: the rest of the piece is still rests.
        let ev = events(&e, 0, 0);
        assert_eq!(ev.last().unwrap().end(), f(2, 1));
        assert!(ev.windows(2).all(|w| w[0].end() == w[1].start));
        // One undo step per note.
        e.undo();
        assert_eq!(notes(&e, 0)[3], "r");
        e.redo();
        assert_eq!(notes(&e, 0)[3], "C4");
    }

    #[test]
    fn chords_accidentals_rests_and_durations() {
        let mut e = blank();
        e.set_input_mode(true);
        e.spec.base = edit::BaseDuration::Half;
        e.enter_note(0, false); // C5
        e.enter_note(2, true); // E5 above
        e.toggle_accidental(1);
        e.enter_note(4, true); // G#5 above
        assert_eq!(notes(&e, 0)[0], "C5+E5+G♯5");
        assert_eq!(e.caret, f(1, 2));
        assert_eq!(e.accidental, None, "an accidental lasts one note");
        e.cycle_dots();
        e.spec.base = edit::BaseDuration::Quarter;
        e.enter_rest(); // dotted quarter rest
        assert_eq!(e.caret, f(7, 8));
        e.backspace();
        assert_eq!(e.caret, f(1, 2), "backspace steps over a voice-1 rest");
    }

    #[test]
    fn voice_two_is_created_with_holes() {
        let mut e = blank();
        e.set_input_mode(true);
        e.voice = 2;
        e.caret = f(1, 2);
        e.enter_note(6, false);
        let m = e.movement().unwrap();
        assert_eq!(m.lanes[0].voices.len(), 2);
        assert_eq!(m.lanes[0].voices[1].index, 2);
        assert_eq!(m.lanes[0].voices[1].events.len(), 1, "no filler rests in voice 2");
    }

    #[test]
    fn entering_past_the_end_adds_bars() {
        let mut e = blank();
        e.set_input_mode(true);
        e.caret = f(7, 4);
        e.spec.base = edit::BaseDuration::Half;
        e.enter_note(0, false);
        assert_eq!(e.movement().unwrap().length, f(3, 1));
    }

    #[test]
    fn mouse_input_snaps_to_the_grid() {
        let mut e = blank();
        e.spec.base = edit::BaseDuration::Eighth;
        // Quarter 1.4 -> eighth grid 2.8 + 0.35 -> 3rd eighth = beat 2.5 (3/8)
        e.place_note_at(1, 1.4, 19, false); // A2 on the cello (diatonic 5 + 7*2)
        let ev = events(&e, 1, 0);
        let n = ev.iter().find(|x| !x.is_rest()).unwrap();
        assert_eq!(n.start, f(3, 8));
        assert_eq!(pitch_name(n.heads[0].pitch), "A2");
        e.place_note_at(1, 1.45, 21, true); // shift: add C3 to it
        assert_eq!(events(&e, 1, 0).iter().find(|x| !x.is_rest()).unwrap().heads.len(), 2);
    }

    #[test]
    fn range_selection_copy_paste_and_repeat() {
        let mut e = Editor::new(demo::project());
        let flute: Vec<Id> = events(&e, 0, 0).iter().take(2).map(|x| x.heads[0].id).collect();
        e.select(Some(flute[0]));
        e.select_elements(&[flute[1]], true);
        assert_eq!(e.range, Some(Range { lane_from: 0, lane_to: 0, from: Frac::ZERO, to: f(3, 8) }));
        assert_eq!(e.selection_label(), "Bar 1 · 1 staff");
        e.copy();
        e.repeat_selection();
        let r = e.range.unwrap();
        assert_eq!((r.from, r.to), (f(3, 8), f(3, 4)));
        let after = events(&e, 0, 0);
        let at = |t| edit::event_at(&after, t).unwrap();
        assert_eq!(pitch_name(at(f(3, 8)).heads[0].pitch), "E5");
        assert_eq!(pitch_name(at(f(5, 8)).heads[0].pitch), "D5");
        // Paste at a note in the cello.
        let cello = events(&e, 1, 0)[2].heads[0].id;
        e.select(Some(cello));
        e.paste();
        assert_eq!(e.range.unwrap().lane_from, 1);
        assert_eq!(pitch_name(edit::event_at(&events(&e, 1, 0), f(1, 1)).unwrap().heads[0].pitch), "E5");
    }

    #[test]
    fn select_all_delete_and_articulations() {
        let mut e = Editor::new(demo::project());
        let first = events(&e, 0, 0)[0].heads[0].id;
        e.select(Some(first));
        e.toggle_articulation(art::STACCATO);
        assert!(events(&e, 0, 0)[0].articulations & art::STACCATO != 0);
        e.select_all();
        e.toggle_articulation(art::ACCENT);
        assert!(events(&e, 1, 0).iter().filter(|x| !x.is_rest()).all(|x| x.articulations & art::ACCENT != 0));
        e.toggle_articulation(art::ACCENT); // all have it: removed
        assert!(events(&e, 1, 0).iter().all(|x| x.articulations & art::ACCENT == 0));
        e.set_dynamic(Some(5));
        assert_eq!(events(&e, 0, 0)[0].dynamic, Some(5));
        e.delete_selection();
        assert!(events(&e, 0, 0).iter().all(|x| x.is_rest()));
        assert!(e.can_undo());
        for _ in 0..4 {
            e.undo();
        }
        assert_eq!(events(&e, 0, 0)[0].articulations, art::STACCATO);
    }

    #[test]
    fn slurs_nest_overlap_and_follow_their_notes() {
        let mut e = blank();
        quarters_cdef(&mut e);
        let n = events(&e, 0, 0);
        let slurs = |e: &Editor| {
            let n = events(e, 0, 0);
            let at = |id| n.iter().position(|x| x.id == id).unwrap();
            let mut v: Vec<(usize, usize)> = e.movement().unwrap().slurs.iter().map(|s| (at(s.from), at(s.to))).collect();
            v.sort();
            v
        };
        // A phrase over all four, a slur inside it, one overlapping its end.
        e.set_range(Range { lane_from: 0, lane_to: 0, from: Frac::ZERO, to: f(1, 1) });
        e.add_slur();
        e.set_range(Range { lane_from: 0, lane_to: 0, from: f(1, 4), to: f(3, 4) });
        e.add_slur();
        assert_eq!(slurs(&e), [(0, 3), (1, 2)], "nested");
        // Moving the inner slur's end past the phrase's end overlaps it.
        let inner = e.movement().unwrap().slurs.iter().find(|s| s.from == n[1].id).unwrap().id;
        assert!(e.select_slur(inner));
        e.step_slur(SlurHandle::End, 1);
        assert_eq!(slurs(&e), [(0, 3), (1, 3)]);
        // A start can't pass the end.
        e.step_slur(SlurHandle::Start, 5);
        assert_eq!(slurs(&e), [(0, 3), (2, 3)]);
        // S on the same notes again takes that slur away.
        e.set_range(Range { lane_from: 0, lane_to: 0, from: f(2, 4), to: f(1, 1) });
        e.add_slur();
        assert_eq!(slurs(&e), [(0, 3)]);
        // Copy and paste carries the slur; deleting an end note drops it.
        e.set_range(Range { lane_from: 0, lane_to: 0, from: Frac::ZERO, to: f(1, 1) });
        e.copy();
        e.set_range(Range { lane_from: 0, lane_to: 0, from: f(1, 1), to: f(2, 1) });
        e.paste();
        assert_eq!(e.movement().unwrap().slurs.len(), 2);
        let last = events(&e, 0, 0).iter().filter(|x| !x.is_rest()).last().unwrap().heads[0].id;
        e.select(Some(last));
        e.delete_selection();
        assert_eq!(e.movement().unwrap().slurs.len(), 1, "a slur goes with its note");
        e.undo();
        assert_eq!(e.movement().unwrap().slurs.len(), 2);
    }

    #[test]
    fn slurs_hairpins_ties_and_transpose() {
        let mut e = Editor::new(demo::project());
        let cello = events(&e, 1, 0);
        e.select(Some(cello[0].heads[0].id));
        e.add_slur();
        let ev = events(&e, 1, 0);
        let slurs = |e: &Editor| e.movement().unwrap().slurs.iter().map(|s| (s.from, s.to)).collect::<Vec<_>>();
        assert!(slurs(&e).contains(&(ev[0].id, ev[1].id)));
        assert!(e.selected_slur().is_some(), "the new slur is selected");
        e.select(Some(cello[0].heads[0].id));
        e.add_hairpin(true);
        assert!(events(&e, 1, 0)[0].spans & span::CRESC_START != 0);
        e.select(Some(cello[0].heads[0].id));
        e.toggle_tie();
        assert!(events(&e, 1, 0)[0].heads[0].tie);
        e.transpose_selection(7);
        assert_eq!(pitch_name(events(&e, 1, 0)[0].heads[0].pitch), "C4");
    }

    #[test]
    fn popovers_and_bars() {
        let mut e = Editor::new(demo::project());
        let note = events(&e, 0, 0)[3].heads[0].id; // bar 1
        e.select(Some(note));
        assert_eq!(e.popover_position(), "Bar 1");
        assert!(e.apply_popover(PopoverKind::Key, "H").unwrap_err().starts_with("Key not understood"));
        e.apply_popover(PopoverKind::Key, "Eb").unwrap();
        assert_eq!(e.movement().unwrap().keys[0].fifths, -3);
        assert_eq!(e.status, "Key signature · bar 1");
        e.apply_popover(PopoverKind::Time, "3/4").unwrap();
        assert_eq!(e.movement().unwrap().meter[0].num, 3);
        assert_eq!(e.movement().unwrap().length, f(33, 4), "rounded up to whole 3/4 bars");
        e.undo();
        e.apply_popover(PopoverKind::Tempo, "Allegro").unwrap();
        let tempo = &e.movement().unwrap().tempo;
        assert_eq!((tempo.len(), tempo[1].at, tempo[1].bpm), (2, f(1, 2), 132.0), "at the selected note");
        e.apply_popover(PopoverKind::Expression, "espress.").unwrap();
        assert_eq!(e.movement().unwrap().lanes[0].texts.len(), 2);
        e.apply_popover(PopoverKind::Ornament, "tr").unwrap();
        assert!(events(&e, 0, 0)[3].articulations & art::TRILL != 0);
        e.insert_bars(2);
        assert_eq!(e.movement().unwrap().length, f(10, 1));
        assert_eq!(e.status, "Inserted 2 bars before bar 1");
        e.delete_bars();
        assert_eq!(e.movement().unwrap().length, f(9, 1));
        // Structure items by id: the demo's rehearsal mark.
        e.select(Some(71));
        assert_eq!(e.selection_label(), "Rehearsal mark");
        e.delete_selection();
        assert!(e.movement().unwrap().rehearsal.is_empty());
        e.clear_selection();
        e.caret = f(1, 1);
        e.add_first_second_endings();
        assert_eq!(e.movement().unwrap().endings.len(), 2);
    }

    #[test]
    fn drum_sets_get_a_map_and_channel_ten() {
        let e = Editor::new(crate::files::new_project("D", "Flute, Drum set", 2).unwrap());
        let drums = e.project.instruments.iter().find(|i| i.name == "Drum set").unwrap().clone();
        assert!(!drums.drums.is_empty());
        let track = e.project.tracks.iter().find(|t| Some(t.id) == drums.track).unwrap();
        assert_eq!(track.midi_channel, 9);
        assert!(e.project.tracks.iter().filter(|t| Some(t.id) != drums.track).all(|t| t.midi_channel != 9));
        assert!(e.lane_is_percussion(1) && !e.lane_is_percussion(0));
        // The snare sits on C5 and sounds key 38.
        let snare = Pitch { step: 0, alter: 0, octave: 5 };
        assert_eq!(e.drum_name(1, Frac::ZERO, snare).as_deref(), Some("Snare drum"));
        assert_eq!(e.project.sounding_key(drums.id, snare.midi_key()), 38);
    }

    #[test]
    fn cues_and_part_only_texts() {
        let mut e = blank();
        quarters_cdef(&mut e);
        e.set_range(Range { lane_from: 0, lane_to: 0, from: Frac::ZERO, to: f(1, 1) });
        e.copy();
        e.select(None);
        e.set_range(Range { lane_from: 1, lane_to: 1, from: f(1, 1), to: f(2, 1) });
        e.paste_cue();
        let l = &e.movement().unwrap().lanes[1];
        let cue = l.voices.iter().find(|v| v.index == 2).unwrap();
        let notes: Vec<&Event> = cue.events.iter().filter(|x| !x.is_rest()).collect();
        assert_eq!(notes.len(), 4);
        assert!(notes.iter().all(|x| x.articulations & art::CUE != 0 && x.start >= f(1, 1)));
        assert_eq!((l.texts[0].text.as_str(), l.texts[0].at), ("Flute", f(1, 1)));
        assert_eq!(e.status, "Cue from Flute");
        // Cues are silent.
        assert_eq!(corinzia_perform::perform(e.movement().unwrap()).notes.len(), 4, "the flute's four notes only");
        // A text for the parts only: not in the score, in the part.
        e.set_range(Range { lane_from: 0, lane_to: 0, from: Frac::ZERO, to: f(1, 4) });
        e.apply_popover(PopoverKind::Expression, "parts: solo").unwrap();
        let t = e.movement().unwrap().lanes[0].texts.iter().find(|t| t.text == "solo").unwrap().clone();
        assert_eq!(t.shown, corinzia_model::Shown::Parts);
        assert_eq!(edit::shown_text(&t), "parts: solo");
        let m = e.movement().unwrap();
        let has = |s: &corinzia_score::score::Score| s.parts.iter().any(|p| p.texts.iter().any(|x| x.text == "solo"));
        assert!(!has(&corinzia_model::notation::score(&e.project, m)));
        assert!(has(&corinzia_model::notation::part_score(&e.project, m, m.lanes[0].player)));
    }

    #[test]
    fn cross_staff_notes() {
        let mut e = Editor::new(crate::files::new_project("P", "Piano", 2).unwrap());
        assert_eq!(e.movement().unwrap().lanes.len(), 2, "a grand staff");
        quarters_cdef(&mut e);
        select_nth(&mut e, 1);
        e.cross_staff(1);
        let id = events(&e, 0, 0)[1].id;
        assert_eq!(e.movement().unwrap().cross, [(id, 1)]);
        assert_eq!(e.status, "Cross staff: 1 note");
        let s = corinzia_model::notation::score(&e.project, e.movement().unwrap());
        assert_eq!(s.parts[0].cross, [(id, 1)]);
        // No staff below the bass staff; back up goes home.
        e.cross_staff(1);
        assert_eq!(e.movement().unwrap().cross, [(id, 1)]);
        assert_eq!(e.status, "No staff of this player there");
        e.cross_staff(-1);
        assert!(e.movement().unwrap().cross.is_empty());
    }

    #[test]
    fn guitar_tablature() {
        let mut e = Editor::new(crate::files::new_project("G", "Electric guitar, Guitar, Bass guitar", 1).unwrap());
        let tunings: Vec<usize> = e.project.instruments.iter().map(|i| i.tuning.len()).collect();
        assert_eq!(tunings, [6, 0, 4]);
        e.caret_lane = 1;
        e.toggle_tab();
        assert_eq!(e.project.instruments[1].tuning, GUITAR_TUNING);
        assert_eq!(e.status, "Tablature on for Guitar");
        let s = corinzia_model::notation::score(&e.project, e.movement().unwrap());
        assert_eq!(s.parts[1].tab, GUITAR_TUNING);
        e.undo();
        assert!(e.project.instruments[1].tuning.is_empty());
    }

    #[test]
    fn musicxml_round_trip_keeps_the_newer_notation() {
        let mut p = crate::files::new_project("RT", "Flute, Piano, Drum set, Electric guitar", 4).unwrap();
        p.subtitle = "for four".into();
        p.lyricist = "Words: A".into();
        p.arranger = "arr. B".into();
        p.copyright = "© C".into();
        let mut e = Editor::new(p);
        let fill = |e: &mut Editor, lane: usize, from: Frac, steps: &[i32], eighths: bool| {
            e.select(None);
            e.set_input_mode(true);
            e.caret_lane = lane;
            e.caret = from;
            e.spec.base = if eighths { edit::BaseDuration::Eighth } else { edit::BaseDuration::Quarter };
            for &s in steps {
                e.enter_note(s, false);
            }
            e.set_input_mode(false);
        };
        // Flute: eighths in bar 1 (a beam broken by hand, one flagged), quarters in bar 3.
        fill(&mut e, 0, Frac::ZERO, &[0, 1, 2, 3, 4, 3, 2, 1], true);
        fill(&mut e, 0, f(2, 1), &[4, 4, 4, 4], false);
        // Piano: quarters in bar 1, the second written on the left hand's staff.
        fill(&mut e, 1, Frac::ZERO, &[0, 1, 2, 3], false);
        // Drums: snare (C5) and kick (F4); guitar: a cue of the flute in bar 2.
        fill(&mut e, 3, Frac::ZERO, &[0, 3], false);
        fill(&mut e, 4, f(1, 1), &[4, 5, 6, 0], false);
        let piccolo_at = f(2, 1);
        {
            let p = &mut e.project;
            let flute_player = p.players[0].id;
            let picc = p.allocate_id();
            let track = p.instruments[0].track;
            p.instruments.push(Instrument { id: picc, player: flute_player, name: "Piccolo".into(), engraver_name: "Picc.".into(), transposition: Default::default(), clef: "treble".into(), track, drums: vec![], tuning: vec![] });
            p.players[0].instruments.push(picc);
            let m = &mut p.movements[0];
            m.lanes[0].changes.push(InstrumentChange { id: 9001, at: piccolo_at, instrument: picc });
            let mut flute = (*m.lanes[0].voices[0].events).clone();
            flute[2].spans |= span::BEAM_BREAK;
            flute[7].spans |= span::NO_BEAM;
            m.lanes[0].voices[0].events = Arc::new(flute);
            let second = m.lanes[1].voices[0].events[1].id;
            m.cross.push((second, 1));
            let mut gtr = (*m.lanes[4].voices[0].events).clone();
            for ev in gtr.iter_mut().filter(|x| !x.is_rest()) {
                ev.articulations |= art::CUE;
            }
            m.lanes[4].voices[0].events = Arc::new(gtr);
            m.breaks = vec![Break { id: 9002, at: f(1, 1), page: false, part: None }, Break { id: 9003, at: f(2, 1), page: true, part: None }];
            m.meter.push(Meter { at: f(3, 1), num: 7, den: 8, pickup: false, beams: [2, 2, 3, 0, 0, 0, 0, 0] });
        }
        let p = &e.project;
        let xml = corinzia_perform::musicxml(p, &p.movements[0]).unwrap();
        assert!(xml.contains("<staves>2</staves>"), "the piano is one part");
        let q = corinzia_import::musicxml(xml.as_bytes()).unwrap();
        let (a, b) = (&p.movements[0], &q.movements[0]);
        assert_eq!((q.subtitle.as_str(), q.lyricist.as_str(), q.arranger.as_str(), q.copyright.as_str()), ("for four", "Words: A", "arr. B", "© C"));
        assert_eq!(q.players.len(), 4, "{:?}", q.players.iter().map(|p| &p.name).collect::<Vec<_>>());
        assert_eq!(b.lanes.len(), 5);
        let clef_of = |m: &Movement, li: usize| m.lanes[li].clefs.iter().find(|c| c.at.is_zero()).map(|c| c.clef.clone()).unwrap_or_default();
        assert_eq!(clef_of(b, 2), "bass", "the left hand");
        // Beams by hand.
        let spans = |m: &Movement| m.lanes[0].voices[0].events.iter().map(|x| x.spans & span::BEAMS).collect::<Vec<_>>();
        assert_eq!((spans(a)[2], spans(a)[7]), (span::BEAM_BREAK, span::NO_BEAM));
        assert_eq!(spans(b)[..8], spans(a)[..8]);
        if let Ok(path) = std::env::var("RT_XML") {
            std::fs::write(path, &xml).unwrap();
        }
        // The doubling.
        let change = b.lanes[0].changes.iter().find(|c| c.at == piccolo_at).expect("a change to the piccolo");
        assert_eq!(q.instrument(change.instrument).unwrap().name, "Piccolo");
        assert!(!b.lanes[0].texts.iter().any(|t| t.text == "Piccolo"), "the change is not also a text");
        // Cross-staff.
        let second = b.lanes[1].voices[0].events[1].id;
        assert_eq!(b.cross, [(second, 1)]);
        // Drums: their map and noteheads.
        let kit = q.instrument(b.lanes[3].changes[0].instrument).unwrap();
        let snare = kit.drums.iter().find(|d| d.position == Pitch { step: 0, alter: 0, octave: 5 }).unwrap();
        assert_eq!((snare.name.as_str(), snare.key), ("Snare drum", 38));
        assert_eq!(kit.clef, "percussion");
        // Cue notes, the guitar's tuning, breaks and the grouped meter.
        assert!(b.lanes[4].voices[0].events.iter().filter(|x| !x.is_rest()).all(|x| x.articulations & art::CUE != 0));
        assert_eq!(b.lanes[4].voices[0].events.iter().filter(|x| !x.is_rest()).count(), 4);
        assert_eq!(q.instrument(b.lanes[4].changes[0].instrument).unwrap().tuning, GUITAR_TUNING);
        assert_eq!(b.breaks.iter().map(|x| (x.at, x.page)).collect::<Vec<_>>(), [(f(1, 1), false), (f(2, 1), true)]);
        assert_eq!(b.meter.iter().find(|m| m.at == f(3, 1)).map(|m| m.beams), Some([2, 2, 3, 0, 0, 0, 0, 0]));
    }
}
