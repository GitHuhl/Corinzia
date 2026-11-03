//! Navigation and selection: walking notes, bars and staves; passages; editing what is selected.

use super::*;

impl Editor {
    // --------------------------------------------------------- navigation --
    /// What arrow keys select for an event: a single note's head, else the
    /// event (a chord or a rest).
    pub(super) fn nav_id(e: &Event) -> Id {
        if e.heads.len() == 1 { e.heads[0].id } else { e.id }
    }

    /// The voice the arrows walk: the selection's, else the caret's.
    pub(super) fn nav_voice(&self) -> Option<(usize, usize)> {
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

    pub(super) fn set_range(&mut self, r: Range) {
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
    pub(super) fn edit_range_voices(&mut self, label: &str, mut f: impl FnMut(&[Event], u8, &mut Ids) -> Option<EditResult>) -> bool {
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
    pub(super) fn edit_selected(&mut self, label: &str, f: impl FnOnce(&[Event], Id, u8, &mut Ids) -> Option<EditResult>) -> Option<EditResult> {
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
    pub(super) fn change_durations(&mut self, label: &str, f: impl Fn(Frac) -> Frac + Copy) -> bool {
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
    pub(super) fn selection_scale(&self) -> Frac {
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
    pub(super) fn written_interval(&self, lane: usize, at: Frac) -> Interval {
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
}
