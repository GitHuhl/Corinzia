//! Tracks: audio tracks and their clips, piano-roll tweaks, a track's settings, plugin and shared instrument.

use super::*;

impl Editor {
    // -------------------------------------------------------------- mixer --
    /// A mixer change on a track; `merge` folds a slider drag into one step.
    /// Ends a slider gesture: what follows is a new undo step.
    pub fn end_gesture(&mut self) {
        self.merge = None;
    }

    /// Adds an audio track (after the other tracks); its id.
    pub fn add_audio_track(&mut self, name: &str) -> Id {
        let before = self.snapshot();
        let id = self.project.allocate_id();
        self.project.tracks.push(Track { id, name: name.into(), instrument_node: None, note_port: 0, output_bus: None, midi_channel: 0, gain_db: 0.0, pan: 0.0, mute: false, solo: false, inserts: vec![], sends: vec![], dynamics_cc: None, program: None, bank: None, techniques: vec![], color: None, folder: String::new(), audio: true, clips: vec![] });
        self.commit_with(before, "Add audio track", true, None);
        id
    }

    /// Removes an audio track with its clips (instrument tracks go with
    /// their players).
    pub fn remove_audio_track(&mut self, track: Id) {
        if !self.project.track(track).is_some_and(|t| t.audio) {
            return;
        }
        let before = self.snapshot();
        self.project.tracks.retain(|t| t.id != track);
        // Its effects go too.
        let nodes: Vec<Id> = before.track(track).map(|t| t.inserts.clone()).unwrap_or_default();
        self.project.nodes.retain(|n| !nodes.contains(&n.id));
        for m in self.project.movements.iter_mut() {
            m.automation.retain(|l| !matches!(l.target, AutoTarget::Gain(t) | AutoTarget::Pan(t) if t == track) && !matches!(l.target, AutoTarget::Param { node, .. } if nodes.contains(&node)));
        }
        self.commit_with(before, "Remove audio track", true, None);
    }

    /// Puts a clip on an audio track (a new id is given); its id.
    pub fn add_clip(&mut self, track: Id, mut clip: AudioClip) -> Option<Id> {
        let before = self.snapshot();
        let id = self.project.allocate_id();
        let t = self.project.tracks.iter_mut().find(|t| t.id == track && t.audio)?;
        clip.id = id;
        t.clips.push(clip);
        self.commit_with(before, "Add audio", true, None);
        Some(id)
    }

    pub fn clip(&self, track: Id, clip: Id) -> Option<&AudioClip> {
        self.project.track(track)?.clips.iter().find(|c| c.id == clip)
    }

    /// Changes a clip. `live`: one step of a drag (merged into one undo
    /// step; playback catches up at [`Editor::music_changed`]).
    pub fn edit_clip(&mut self, track: Id, clip: Id, label: &str, live: bool, f: impl FnOnce(&mut AudioClip)) {
        let before = self.snapshot();
        let Some(c) = self.project.tracks.iter_mut().find(|t| t.id == track).and_then(|t| t.clips.iter_mut().find(|c| c.id == clip)) else { return };
        f(c);
        self.commit_with(before, label, !live, live.then_some(clip));
    }

    /// A copy of a clip, at the same place; its id.
    pub fn duplicate_clip(&mut self, track: Id, clip: Id) -> Option<Id> {
        let c = self.clip(track, clip)?.clone();
        self.add_clip(track, c)
    }

    pub fn delete_clip(&mut self, track: Id, clip: Id) {
        let before = self.snapshot();
        let Some(t) = self.project.tracks.iter_mut().find(|t| t.id == track) else { return };
        let n = t.clips.len();
        t.clips.retain(|c| c.id != clip);
        if t.clips.len() != n {
            self.commit_with(before, "Delete audio", true, None);
        }
    }

    /// A note's tweak in the movement edited (none: played as written).
    pub fn tweak_of(&self, head: Id) -> Option<Tweak> {
        self.movement()?.tweaks.iter().find(|t| t.head == head).copied()
    }

    /// Changes how notes play (the piano roll): `f` on each head's tweak
    /// (made if need be; dropped when it no longer changes anything).
    /// `live`: a drag step (one undo step; playback catches up at
    /// [`Editor::music_changed`]).
    pub fn tweak(&mut self, heads: &[Id], label: &str, live: bool, mut f: impl FnMut(&mut Tweak)) {
        let before = self.snapshot();
        let Some(m) = self.project.movements.get_mut(self.movement) else { return };
        for &h in heads {
            match m.tweaks.binary_search_by_key(&h, |t| t.head) {
                Ok(i) => f(&mut m.tweaks[i]),
                Err(i) => {
                    let mut t = Tweak::new(h);
                    f(&mut t);
                    m.tweaks.insert(i, t);
                }
            }
        }
        m.tweaks.retain(|t| !t.is_empty());
        let key = heads.first().copied().unwrap_or(0) ^ 0x7ee4;
        self.commit_with(before, label, !live, live.then_some(key));
    }

    /// The playback plan is out of date (after live edits).
    pub fn music_changed(&mut self) {
        self.changed_with(true);
    }

    pub fn set_track(&mut self, track: Id, label: &str, merge: bool, f: impl FnOnce(&mut Track)) {
        let before = self.snapshot();
        let Some(t) = self.project.tracks.iter_mut().find(|t| t.id == track) else { return };
        f(t);
        self.commit_with(before, label, false, merge.then_some(track));
    }

    /// Assigns a plugin (name, bundle, plugin id; CLAP or VST3) to a track, or the
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
                self.project.nodes.push(AudioNode { id: node, kind: NodeKind::Plugin, name: name.into(), bundle: bundle.into(), plugin_id: id.into(), bypassed: false });
                self.project.tracks[ti].instrument_node = Some(node);
                name.to_string()
            }
            None => "Corinzia Sine".to_string(),
        };
        self.status = format!("{} plays {name}", self.project.tracks[ti].name);
        self.commit_with(before, "Instrument plugin", true, None);
    }

    /// A track plays through another's plugin instance (a multi-timbral
    /// sampler), on the first MIDI channel no track sharing it uses.
    pub fn share_instrument(&mut self, track: Id, with: Id) -> bool {
        let Some(node) = self.project.track(with).and_then(|t| t.instrument_node) else {
            self.status = "Only a plugin instrument can be shared".into();
            return false;
        };
        let used: Vec<u8> = self.project.tracks.iter().filter(|t| t.id != track && t.instrument_node == Some(node)).map(|t| t.midi_channel & 15).collect();
        let Some(channel) = (0..16u8).find(|c| !used.contains(c)) else {
            self.status = "All 16 channels of that instrument are taken".into();
            return false;
        };
        let before = self.snapshot();
        let Some(ti) = self.project.tracks.iter().position(|t| t.id == track) else { return false };
        // Its own plugin goes, unless another track still plays it.
        if let Some(old) = self.project.tracks[ti].instrument_node.filter(|o| *o != node) {
            if !self.project.tracks.iter().enumerate().any(|(i, t)| i != ti && t.instrument_node == Some(old)) {
                self.project.nodes.retain(|n| n.id != old);
                self.project.edges.retain(|e| e.from != old && e.to != old);
                self.project.plugin_state.remove(&old);
            }
        }
        let t = &mut self.project.tracks[ti];
        (t.instrument_node, t.midi_channel) = (Some(node), channel);
        let (name, host) = (t.name.clone(), self.project.track(with).map(|t| t.name.clone()).unwrap_or_default());
        self.status = format!("{name} plays through {host}'s instrument, on channel {}", channel + 1);
        self.commit_with(before, "Share instrument", true, None);
        true
    }
}
