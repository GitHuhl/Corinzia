//! The motif bank: passages of one staff kept, named and described (see
//! `corinzia_model::motif`). Every change is one undo step; the bank is
//! not music, so the engraving and the playback plan stay as they are.

use super::*;
use corinzia_model::motif::{self, Motif};

impl Editor {
    /// A motif from the passage selected on one staff, not yet in the bank:
    /// the user names it and says what it is first.
    pub fn motif_from_selection(&self) -> Result<Motif, String> {
        let m = self.movement().ok_or("No movement")?;
        let r = self.range.ok_or("Select the notes of the motif on one staff")?;
        if r.lane_from != r.lane_to {
            return Err("A motif is taken from one staff: select notes on one staff".into());
        }
        let bar = |at: Frac| edit::bar_number(m, at).0 as i64;
        let bars = (bar(r.from), bar(r.to - Frac::new(1, 4096)));
        motif::capture(&self.project, m, r.lane_from, r.from, r.to, bars, 0).ok_or_else(|| "There are no notes in the selection".into())
    }

    /// Puts a motif in the bank (a new one gets an id), or replaces the one
    /// with its id.
    pub fn save_motif(&mut self, mut m: Motif) -> Id {
        let before = self.snapshot();
        let label = if m.id == 0 { "Add motif" } else { "Edit motif" };
        if m.id == 0 || !self.project.motifs.iter().any(|x| x.id == m.id) {
            m.id = self.project.allocate_id();
        }
        let id = m.id;
        let name = m.name.clone();
        match self.project.motifs.iter_mut().find(|x| x.id == id) {
            Some(x) => *x = m,
            None => self.project.motifs.push(m),
        }
        self.commit_with(before, label, false, None);
        self.status = format!("{label}: {name}");
        id
    }

    pub fn delete_motif(&mut self, id: Id) {
        let Some(name) = self.project.motifs.iter().find(|m| m.id == id).map(|m| m.name.clone()) else { return };
        let before = self.snapshot();
        self.project.motifs.retain(|m| m.id != id);
        self.commit_with(before, "Delete motif", false, None);
        self.status = format!("Deleted motif: {name}");
    }

    /// Selects the passage a motif was taken from (switching movement if
    /// need be). False when it no longer exists.
    pub fn show_motif(&mut self, id: Id) -> bool {
        let Some(src) = self.project.motifs.iter().find(|m| m.id == id).and_then(|m| m.source.clone()) else { return false };
        let Some(mi) = self.project.movements.iter().position(|m| m.id == src.movement) else { return false };
        if mi != self.movement {
            self.switch_movement(mi);
        }
        let Some(m) = self.movement() else { return false };
        if src.lane >= m.lanes.len() {
            return false;
        }
        let (from, to) = (Frac::new(src.from.0, src.from.1.max(1)), Frac::new(src.to.0, src.to.1.max(1)));
        self.set_range(Range { lane_from: src.lane, lane_to: src.lane, from, to: to.min(m.length) });
        true
    }
}
