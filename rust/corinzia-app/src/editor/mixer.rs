//! The mixer: effect chains, buses, outputs, sends and the master.

use super::*;

impl Editor {
    // -------------------------------------------------------------- mixer --
    /// The effect chain of a strip: a track, a bus, or the master ([`MASTER`]).
    pub(super) fn inserts_mut(p: &mut Project, owner: Id) -> Option<&mut Vec<Id>> {
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
        self.project.buses.push(Bus { id, name: name.clone(), inserts: vec![], gain_db: 0.0, pan: 0.0, mute: false, solo: false, output: None });
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
        for x in self.project.buses.iter_mut().filter(|x| x.output == Some(bus)) {
            x.output = None;
        }
        // Its automation, and its sends'.
        for m in self.project.movements.iter_mut() {
            m.automation.retain(|l| !matches!(l.target, AutoTarget::Gain(b) | AutoTarget::Pan(b) | AutoTarget::Send { bus: b, .. } if b == bus));
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
                t.sends.push(SendLevel { bus, gain_db: -6.0, pre: false });
            }
        });
    }

    /// A send taken before the fader, or after it.
    pub fn toggle_send_pre(&mut self, track: Id, index: usize) {
        self.set_track(track, "Pre-fader send", false, |t| {
            if let Some(s) = t.sends.get_mut(index) {
                s.pre = !s.pre;
            }
        });
    }

    /// Where a bus goes: another bus, or the master (`None`). A bus cannot
    /// go into itself or a bus that comes back to it.
    pub fn set_bus_output(&mut self, bus: Id, to: Option<Id>) -> bool {
        if let Some(to) = to {
            if corinzia_audio::snapshot::bus_chain(&self.project, to).contains(&bus) {
                let name = self.project.bus(bus).map(|b| b.name.clone()).unwrap_or_default();
                self.status = format!("{name} cannot go into a bus that goes back into it");
                return false;
            }
        }
        self.set_bus(bus, "Bus output", false, |b| b.output = to);
        true
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
}
