//! Select only, Select similar and transposition.

use super::*;

impl Editor {
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

    pub(super) fn pick(&mut self, found: Vec<Id>, what: &edit::select::Pick, scope: &str) -> Result<(), String> {
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
    pub(super) fn picked_events<'m>(&self, m: &'m Movement) -> Vec<(usize, &'m Event, Vec<usize>)> {
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
    pub(super) fn modify_picked(&mut self, label: &str, mut f: impl FnMut(&mut Event, &[usize])) -> bool {
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
    pub(super) fn delete_picked(&mut self) {
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

    /// Diatonic steps (±7 = an octave); a selected head moves alone.
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
}
