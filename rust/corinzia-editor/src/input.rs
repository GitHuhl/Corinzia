//! Input state and note entry: the caret, durations, accidentals, voices, notes, rests, grace notes and MIDI keys.

use super::*;

impl Editor {
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
    pub(super) fn enter_grace(&mut self, step: i32, slash: bool) {
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
}
