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
use corinzia_score::score::span;
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

#[derive(Clone, Debug)]
struct Clip {
    lanes: Vec<Vec<(u8, Vec<Event>)>>, // per staff: (voice index, events from 0)
    length: Frac,
}

const UNDO_LIMIT: usize = 500;

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
    pub selection: Option<Id>,
    pub range: Option<Range>,
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
    l.voices.push(Voice { id: ids.next(), index, events: Arc::new(Vec::new()) });
    l.voices.sort_by_key(|v| v.index);
    l.voices.iter().position(|v| v.index == index)
}

fn set_events(m: &mut Movement, lane: usize, voice: usize, r: &EditResult) {
    m.lanes[lane].voices[voice].events = Arc::new(r.events.clone());
}

/// Single note selected: a span from it to the next note in its voice.
fn span_from_note(evs: &[Event], note: Id) -> Option<(Frac, Frac)> {
    let (e, _) = edit::find_event(evs, note)?;
    evs.iter().find(|n| n.start > e.start && !n.heads.is_empty()).map(|n| (e.start, n.end()))
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
            range: None,
            range_anchor: None,
            clip: None,
            undo: Vec::new(),
            redo: Vec::new(),
            revision: 0,
            music_revision: 0,
            merge: None,
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
        // Keep the selection valid.
        if let Some(id) = self.selection {
            if self.locate(id).is_none() && self.movement().and_then(|m| edit::describe_item(m, id)).is_none() {
                self.selection = None;
            }
        }
        if let (Some(r), Some(m)) = (self.range, self.movement()) {
            if r.lane_to >= m.lanes.len() || r.from >= m.length {
                self.range = None;
            }
        }
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

    // -------------------------------------------------------------- mixer --
    /// A mixer change on a track; `merge` folds a slider drag into one step.
    pub fn set_track(&mut self, track: Id, label: &str, merge: bool, f: impl FnOnce(&mut Track)) {
        let before = self.snapshot();
        let Some(t) = self.project.tracks.iter_mut().find(|t| t.id == track) else { return };
        f(t);
        self.commit_with(before, label, false, merge.then_some(track));
    }

    /// Assigns a CLAP plugin (name, bundle, plugin id) to a track, or the
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
                self.project.nodes.push(AudioNode { id: node, kind: NodeKind::Clap, name: name.into(), bundle: bundle.into(), plugin_id: id.into(), bypassed: false });
                self.project.tracks[ti].instrument_node = Some(node);
                name.to_string()
            }
            None => "Corinzia Sine".to_string(),
        };
        self.status = format!("{} plays {name}", self.project.tracks[ti].name);
        self.commit_with(before, "Instrument plugin", true, None);
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
        let step = step.rem_euclid(7);
        let (lane, voice, caret, spec, alter, chord) = (self.caret_lane, self.voice, self.caret, self.spec, self.accidental.map(i32::from), add_to_chord || self.chord_mode);
        let mut out: Option<EditResult> = None;
        let mut added_to_chord = false;
        self.transact("Note input", |m, ids| {
            if m.lanes.is_empty() {
                return false;
            }
            let lane = lane.min(m.lanes.len() - 1);
            let Some(vi) = voice_slot(m, lane, voice, true, ids) else { return false };
            let evs = m.lanes[lane].voices[vi].events.clone();
            let fifths = key_fifths_at(m, caret);
            // Chord: add above the event just entered (the one ending at the caret).
            let last = evs.iter().filter(|e| e.start < caret && e.end() == caret && !e.heads.is_empty()).last();
            let r = if let (true, Some(last)) = (chord, last) {
                let top = last.heads.last().unwrap().pitch;
                let mut p = edit::nearest_pitch(step, alter, fifths, top);
                if edit::diatonic(p) <= edit::diatonic(top) {
                    p = edit::from_diatonic(edit::diatonic(p) + 7, p.alter as i32);
                }
                added_to_chord = true;
                match edit::add_to_chord(&evs, last.start, p, ids) {
                    Some(r) => r,
                    None => return false,
                }
            } else {
                let reference = edit::previous_pitch(&evs, caret).unwrap_or(Pitch { step: 6, alter: 0, octave: 4 });
                let p = edit::nearest_pitch(step, alter, fifths, reference);
                let r = edit::insert_event(&evs, edit::make_event(ids, caret, spec.value(), &[p], 0), voice == 1, ids);
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

    pub fn enter_rest(&mut self) {
        let (lane, voice, caret, spec) = (self.caret_lane, self.voice, self.caret, self.spec);
        let mut out = None;
        self.transact("Rest input", |m, ids| {
            if m.lanes.is_empty() {
                return false;
            }
            let lane = lane.min(m.lanes.len() - 1);
            let Some(vi) = voice_slot(m, lane, voice, true, ids) else { return false };
            let evs = m.lanes[lane].voices[vi].events.clone();
            let r = edit::insert_event(&evs, edit::make_event(ids, caret, spec.value(), &[], 0), voice == 1, ids);
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
        // Snap to the grid of the current (undotted) duration, biased left so
        // a click just after a beat still lands on it.
        let grid = DurationSpec { dots: 0, ..self.spec }.value();
        let grid_q = grid.num as f64 / grid.den as f64 * 4.0;
        let k = (quarter / grid_q + 0.35).floor().max(0.0) as i64;
        self.caret_lane = lane;
        self.caret = grid * Frac::int(k);
        let (voice, caret, spec, accidental, chord) = (self.voice, self.caret, self.spec, self.accidental, add_to_chord || self.chord_mode);
        let step = diatonic.rem_euclid(7);
        let mut out = None;
        self.transact("Note input", |m, ids| {
            let Some(vi) = voice_slot(m, lane, voice, true, ids) else { return false };
            let evs = m.lanes[lane].voices[vi].events.clone();
            let alter = accidental.map(i32::from).unwrap_or_else(|| edit::key_alter(key_fifths_at(m, caret), step));
            let p = edit::from_diatonic(diatonic, alter);
            let onset = edit::event_at(&evs, caret).filter(|e| e.start == caret && !e.heads.is_empty());
            let r = if chord && onset.is_some() {
                match edit::add_to_chord(&evs, caret, p, ids) {
                    Some(r) => r,
                    None => return false,
                }
            } else {
                let r = edit::insert_event(&evs, edit::make_event(ids, caret, spec.value(), &[p], 0), voice == 1, ids);
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
        let next = self.caret + self.spec.value() * Frac::int(direction);
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
        self.range_anchor = None;
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
        self.caret_lane = r.lane_from;
        self.caret = r.from;
        self.range = Some(r);
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
        if let Some(r) = self.range {
            self.edit_range_voices("Delete", |evs, index, ids| Some(edit::erase_range(evs, r.from, r.to, index == 1, ids)));
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
        let mark = mark.map(|m| m.min(7));
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

    /// Diatonic steps (±7 = an octave); a selected head moves alone.
    pub fn transpose_selection(&mut self, steps: i32) {
        if let Some(r) = self.range {
            self.edit_range_voices("Transpose", |evs, _, _| edit::transpose_range(evs, r.from, r.to, steps));
            return;
        }
        self.edit_selected("Transpose", |evs, id, _, _| {
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
        let lanes: Vec<Vec<(u8, Vec<Event>)>> = (r.lane_from..=r.lane_to)
            .filter_map(|li| m.lanes.get(li))
            .map(|l| l.voices.iter().map(|v| (v.index, edit::slice_range(&v.events, r.from, r.to))).collect())
            .collect();
        let length = r.to - r.from;
        self.status = format!("Copied {} {}, {} beats", lanes.len(), if lanes.len() == 1 { "staff" } else { "staves" }, quarters(length));
        self.clip = Some(Clip { lanes, length });
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
                for (index, events) in lane_clip {
                    let Some(vi) = voice_slot(m, li, *index, !events.is_empty(), ids) else { continue };
                    let evs = m.lanes[li].voices[vi].events.clone();
                    let r = edit::paste_events(&evs, events, at, clip.length, *index == 1, ids);
                    set_events(m, li, vi, &r);
                }
            }
            true
        });
        self.set_range(Range { lane_from: lane0, lane_to: last_lane, from: at, to: end });
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

    pub fn add_slur(&mut self) {
        self.range_from_selected_note();
        let Some(r) = self.range else { return };
        if !self.edit_range_voices("Slur", |evs, _, _| edit::mark_span(evs, r.from, r.to, span::SLUR_START, span::SLUR_END)) {
            self.status = "A slur needs at least two notes".into();
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
            PopoverKind::Time => {
                let (n, d) = edit::parse_time(&text).ok_or("Time signature not understood: try 3/4, 6/8, C or C|")?;
                at = bar;
                label = "Time signature".into();
                Box::new(move |m, _| {
                    edit::set_meter(m, bar, n, d);
                    // Keep the movement a whole number of bars in the new meter.
                    let last = edit::bar_start_at_or_before(m, m.length);
                    if last != m.length {
                        m.length = last + edit::bar_length(m, last);
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
            PopoverKind::Ending => {
                let passes = edit::parse_ending(&text).ok_or("Ending not understood: 1, 2, \"1, 2\" or 1-3")?;
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
                return self.enter_lyric(verse, &text, false, false).map(|_| ());
            }
            PopoverKind::Ornament => unreachable!(),
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
        let Some(what) = self.movement().and_then(|m| edit::describe_item(m, id)) else { return false };
        self.transact(&format!("Delete {what}"), |m, _| edit::remove_item(m, id).is_some());
        self.selection = None;
        self.status = format!("Deleted {what}");
        true
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
        if let Some(r) = self.range {
            let b0 = edit::bar_number(m, r.from).0;
            let b1 = edit::bar_number(m, (r.to - Frac::new(1, 1024)).max(r.from)).0;
            let staves = r.lane_to - r.lane_from + 1;
            let bars = if b0 == b1 { format!("Bar {b0}") } else { format!("Bars {b0}–{b1}") };
            return format!("{bars} · {staves} {}", if staves == 1 { "staff" } else { "staves" });
        }
        let Some(id) = self.selection else { return String::new() };
        if let Some(d) = edit::describe_item(m, id) {
            return d;
        }
        let Some((loc, e)) = self.event_of(id) else { return String::new() };
        let lane = &m.lanes[loc.lane];
        let inst = lane.instrument_at(e.start).and_then(|i| self.project.instrument(i)).map(|i| i.name.clone()).unwrap_or_default();
        let bar = edit::bar_number(m, e.start).0;
        match e.heads.iter().find(|h| h.id == id).or(e.heads.first()) {
            Some(h) if !e.is_rest() => format!("{} · {inst} · bar {bar}", pitch_name(h.pitch)),
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
            PopoverKind::Ending => "Ending",
            PopoverKind::Rehearsal => "Rehearsal mark",
            PopoverKind::Ornament => "Ornament",
            PopoverKind::Lyrics => "Lyrics",
            PopoverKind::Chord => "Chord symbol",
        }
    }
    pub fn placeholder(self) -> &'static str {
        match self {
            PopoverKind::Key => "C, Eb, f#, a minor",
            PopoverKind::Time => "3/4, 6/8, C, C|",
            PopoverKind::Clef => "treble, bass, alto, tenor",
            PopoverKind::Barline => "||  |.  |:  :|  :|:",
            PopoverKind::Tempo => "Allegro, Andante 84, q=96",
            PopoverKind::Expression => "dolce, espress.",
            PopoverKind::Technique => "pizz., arco, con sord.",
            PopoverKind::Ending => "1, 2, 1-3",
            PopoverKind::Rehearsal => "",
            PopoverKind::Ornament => "tr, mordent, turn, trem, arp, fermata",
            PopoverKind::Lyrics => "Syllable; space = next note, - = hyphen, _ = extender",
            PopoverKind::Chord => "C, F#m7, Bbmaj7, G7/B; space = next beat",
        }
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
    fn slurs_hairpins_ties_and_transpose() {
        let mut e = Editor::new(demo::project());
        let cello = events(&e, 1, 0);
        e.select(Some(cello[0].heads[0].id));
        e.add_slur();
        let ev = events(&e, 1, 0);
        assert!(ev[0].spans & span::SLUR_START != 0 && ev[1].spans & span::SLUR_END != 0);
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
}
