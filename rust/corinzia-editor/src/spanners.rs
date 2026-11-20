//! Slurs, ties, hairpins and other spanners: making, selecting, dragging and deleting them.

use super::*;

impl Editor {
    // -------------------------------------------------------------- slurs --
    /// The slur selected: valid while nothing else is selected and the
    /// slur still exists.
    pub fn selected_slur(&self) -> Option<Id> {
        let id = self.slur.filter(|_| self.selection.is_none() && self.range.is_none())?;
        self.movement()?.slurs.iter().any(|s| s.id == id).then_some(id)
    }

    /// Selects the slur `id`; false if there is none.
    pub fn select_slur(&mut self, id: Id) -> bool {
        // Not a slur or hairpin: the selection stays as it was (a Shift+click
        // on a note extends it).
        let before = (self.slur, self.selection, self.range, self.range_anchor);
        let restore = |ed: &mut Self| (ed.slur, ed.selection, ed.range, ed.range_anchor) = before;
        self.slur = Some(id);
        self.selection = None;
        self.range = None;
        self.range_anchor = None;
        if ids::as_hairpin(id).is_some() {
            if self.selected_hairpin().is_none() {
                restore(self);
                return false;
            }
            self.status = "Hairpin · drag an end to another note · Shift+←→ end · Alt+←→ start · Delete removes it".into();
            return true;
        }
        if self.selected_slur().is_none() {
            restore(self);
            return false;
        }
        self.status = "Slur · drag an end to another note · Shift+←→ end · Alt+←→ start · X flips".into();
        true
    }

    /// Moves an end of the selected slur to another note of that end's
    /// voice: `to` picks one from the voice's notes and the end's place
    /// among them. The start stays before the end. `merge`: part of a drag
    /// (one undo step).
    pub(super) fn set_slur_end(&mut self, handle: SlurHandle, to: impl FnOnce(&[&Event], usize) -> usize, merge: bool) {
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

    // ----------------------------------------------------------- hairpins --
    /// The hairpin selected (the event it starts on): valid while nothing
    /// else is selected and the event still starts a hairpin. Selected
    /// through the slur's slot, by its display id.
    pub fn selected_hairpin(&self) -> Option<Id> {
        let event = ids::as_hairpin(self.slur.filter(|_| self.selection.is_none() && self.range.is_none())?)?;
        let (_, e) = self.event_of(event)?;
        (e.spans & (span::CRESC_START | span::DIM_START) != 0).then_some(event)
    }

    /// The events a hairpin starts and ends on (its lane and voice): the end
    /// is the next note or rest with an end mark or a dynamic, as the
    /// engraver reads it; none when it runs to its bar's end.
    fn hairpin_ends(&self, start: Id) -> Option<(Locator, usize, Option<usize>)> {
        let loc = self.locate(start)?;
        let evs = &self.movement()?.lanes[loc.lane].voices[loc.voice].events;
        let k = evs.iter().position(|e| e.id == start)?;
        let end = (k + 1..evs.len()).find(|&i| evs[i].dynamic.is_some() || evs[i].spans & span::HAIRPIN_END != 0);
        Some((loc, k, end))
    }

    /// Moves an end of the selected hairpin: its start to another note before
    /// its end, its end to another note or rest after its start. `pick`
    /// chooses the event index from the voice's events and the end's current
    /// index. `merge`: part of a drag (one undo step).
    fn set_hairpin_end(&mut self, handle: SlurHandle, pick: impl FnOnce(&[Event], usize) -> usize, merge: bool) {
        let Some(start) = self.selected_hairpin() else { return };
        let Some((loc, k, end)) = self.hairpin_ends(start) else { return };
        let evs: Vec<Event> = self.movement().unwrap().lanes[loc.lane].voices[loc.voice].events.to_vec();
        let current = if handle == SlurHandle::Start { k } else { end.unwrap_or(evs.len() - 1) };
        let want = pick(&evs, current).min(evs.len() - 1);
        // The nearest allowed event: a note before the end for the start;
        // after the start for the end.
        let ok = |i: usize| match handle {
            SlurHandle::Start => !evs[i].is_rest() && end.is_none_or(|e| i < e) && evs[i].spans & (span::CRESC_START | span::DIM_START) == 0 || i == k,
            SlurHandle::End => i > k,
        };
        let Some(to) = (0..evs.len()).filter(|&i| ok(i)).min_by_key(|&i| i.abs_diff(want)) else { return };
        if to == current {
            return;
        }
        let kind = evs[k].spans & (span::CRESC_START | span::DIM_START);
        let before = self.snapshot();
        let m = &mut self.project.movements[self.movement];
        let v = std::sync::Arc::make_mut(&mut m.lanes[loc.lane].voices[loc.voice].events);
        match handle {
            SlurHandle::Start => {
                v[k].spans &= !kind;
                v[to].spans |= kind;
                self.slur = Some(ids::hairpin(v[to].id));
            }
            SlurHandle::End => {
                if let Some(e) = end {
                    v[e].spans &= !span::HAIRPIN_END;
                }
                v[to].spans |= span::HAIRPIN_END;
            }
        }
        self.commit_with(before, "Move hairpin", true, merge.then_some(start));
        self.status = "Hairpin moved".into();
    }

    /// Moves an end of the selected hairpin by `by` notes (keyboard).
    pub fn step_hairpin(&mut self, handle: SlurHandle, by: i64) {
        self.set_hairpin_end(handle, |_, k| (k as i64 + by).max(0) as usize, false);
    }

    /// Drags an end of the selected hairpin to the event nearest `at`
    /// (written time, whole notes).
    pub fn drag_hairpin(&mut self, handle: SlurHandle, at: Frac) {
        self.set_hairpin_end(
            handle,
            |evs, _| {
                let d = |e: &Event| if e.start > at { e.start - at } else { at - e.start };
                (0..evs.len()).min_by(|&a, &b| d(&evs[a]).cmp(&d(&evs[b]))).unwrap_or(0)
            },
            true,
        );
    }

    /// Removes the selected hairpin (and its end mark).
    pub fn delete_hairpin(&mut self) {
        let Some(start) = self.selected_hairpin() else { return };
        let Some((loc, k, end)) = self.hairpin_ends(start) else { return };
        let before = self.snapshot();
        let m = &mut self.project.movements[self.movement];
        let v = std::sync::Arc::make_mut(&mut m.lanes[loc.lane].voices[loc.voice].events);
        v[k].spans &= !(span::CRESC_START | span::DIM_START);
        if let Some(e) = end {
            v[e].spans &= !span::HAIRPIN_END;
        }
        self.commit(before, "Delete hairpin");
        self.slur = None;
        self.status = "Hairpin deleted".into();
    }

    // ----------------------------------------------------------- spanners --
    pub(super) fn range_from_selected_note(&mut self) {
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
}
