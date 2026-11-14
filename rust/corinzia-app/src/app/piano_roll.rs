//! The piano roll: opening it and its edits.

use super::*;

/// The piano roll's messages.
#[derive(Debug, Clone)]
pub(crate) enum RollMsg {
    Event(roll::RollEvent),
    /// Back to the arrange view.
    Close,
    /// Play the selected notes as written again.
    Reset,
}

impl App {
    /// Opens a track's piano roll on the movement edited.
    pub(crate) fn open_roll(&mut self, track: corinzia_model::Id) {
        let content = roll::content(&self.editor.project, self.editor.movement, track);
        let top_key = (content.high + 2).min(127);
        self.roll = Some(RollState { content, built: self.editor.music_revision, movement: self.editor.movement, selected: vec![], ppq: self.arrange_ppq.max(24.0), scroll_x: self.arrange_scroll, top_key, origin: vec![], dragged: false });
    }

    /// The piano roll's notes rebuilt after the music changed.
    pub(crate) fn sync_roll(&mut self) {
        let (rev, movement) = (self.editor.music_revision, self.editor.movement);
        let Some(r) = self.roll.as_mut() else { return };
        if r.built == rev && r.movement == movement {
            return;
        }
        if self.editor.project.track(r.content.track).is_none() {
            self.roll = None;
            return;
        }
        r.content = roll::content(&self.editor.project, movement, r.content.track);
        (r.built, r.movement) = (rev, movement);
        r.selected.retain(|h| r.content.notes.iter().any(|n| n.head == *h));
    }

    pub(crate) fn roll_event(&mut self, e: roll::RollEvent) {
        let Some(r) = self.roll.as_mut() else { return };
        match e {
            roll::RollEvent::Select { head: None, .. } => r.selected.clear(),
            roll::RollEvent::Select { head: Some(h), add } => {
                if !add {
                    r.selected.clear();
                }
                if !r.selected.contains(&h) {
                    r.selected.push(h);
                }
                // The note on its instrument, as when clicked in the score.
                self.editor.select(Some(h));
                self.audition();
            }
            roll::RollEvent::Drag { drag, dq } => {
                if r.origin.is_empty() {
                    r.origin = r.selected.iter().map(|&h| self.editor.tweak_of(h).unwrap_or(corinzia_model::Tweak::new(h))).collect();
                }
                r.dragged = true;
                let (origin, notes) = (r.origin.clone(), r.content.notes.clone());
                let heads: Vec<_> = origin.iter().map(|t| t.head).collect();
                let label = if drag == roll::Drag::Shift { "Note timing" } else { "Note length" };
                self.editor.tweak(&heads, label, true, |t| {
                    let Some(o) = origin.iter().find(|o| o.head == t.head) else { return };
                    match drag {
                        roll::Drag::Shift => t.shift = o.shift + dq,
                        roll::Drag::Length => {
                            let base = o.length.or_else(|| notes.iter().find(|n| n.head == t.head).map(|n| n.length)).unwrap_or(1.0);
                            t.length = Some((base + dq).max(1.0 / 64.0));
                        }
                    }
                });
            }
            roll::RollEvent::Velocity { head, velocity } => {
                r.dragged = true;
                self.editor.tweak(&[head], "Note velocity", true, |t| t.velocity = Some((velocity * 127.0).round() / 127.0));
            }
            roll::RollEvent::Release => {
                r.origin.clear();
                self.editor.end_gesture();
                if std::mem::take(&mut r.dragged) {
                    self.editor.music_changed();
                }
            }
            roll::RollEvent::Scroll { dx, dy } => {
                r.scroll_x = (r.scroll_x + dx).clamp(0.0, (r.content.length_q as f32 * r.ppq - 100.0).max(0.0));
                r.top_key = (r.top_key + (dy / roll::ROW).round() as i32).clamp(20, 127);
            }
            roll::RollEvent::Zoom(k) => r.ppq = (r.ppq * k).clamp(8.0, 400.0),
        }
    }

    /// Plays the selected notes (or all of them) as written again.
    pub(crate) fn roll_reset(&mut self) {
        if let Some(r) = &self.roll {
            let heads = r.selected.clone();
            let all = if heads.is_empty() { r.content.notes.iter().map(|n| n.head).collect() } else { heads };
            self.editor.tweak(&all, "Reset playing", false, |t| *t = corinzia_model::Tweak::new(t.head));
        }
    }
}
