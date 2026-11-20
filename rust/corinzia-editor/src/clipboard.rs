//! Copy, cut, paste, cues and repeats.

use super::*;

impl Editor {
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
}
