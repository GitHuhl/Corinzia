//! Automation lanes and their points.

use super::*;

impl Editor {
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
        lane.points.insert(i, AutoPoint { at, value, hold: false });
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

    /// A point holds its value up to the next (a step), or slides to it.
    pub fn toggle_auto_hold(&mut self, target: AutoTarget, index: usize) {
        let before = self.snapshot();
        let Some(p) = self.project.movements.get_mut(self.movement).and_then(|m| m.automation.iter_mut().find(|l| l.target == target)).and_then(|l| l.points.get_mut(index)) else { return };
        p.hold = !p.hold;
        self.commit_with(before, "Automation step", false, None);
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
}
