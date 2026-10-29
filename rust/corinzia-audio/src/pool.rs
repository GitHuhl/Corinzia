//! Plugin instances per track, on the main thread: created from the
//! track's audio node (a CLAP or VST3 bundle) or the built-in instrument,
//! state restored from the project, activated, and kept across snapshot
//! rebuilds so edits never reload a plugin. Their editors open from here.

use crate::host::{Library, Plugin, BUILTIN};
use corinzia_model::{Id, NodeKind, Project};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Key {
    bundle: String,
    plugin_id: String,
    node: Option<Id>,
}

pub struct PluginPool {
    libs: HashMap<PathBuf, Arc<Library>>,
    tracks: HashMap<Id, (Key, Arc<Plugin>)>,
    /// Effect instances by audio node.
    effects: HashMap<Id, (Key, Arc<Plugin>)>,
    pub sample_rate: f64,
    pub max_frames: u32,
    /// Problems loading plugins (shown in the Mix tab).
    pub errors: Vec<String>,
    /// Sound files of audio tracks' clips.
    pub media: crate::media::Media,
}

impl PluginPool {
    pub fn new(sample_rate: f64, max_frames: u32) -> PluginPool {
        PluginPool { libs: HashMap::new(), tracks: HashMap::new(), effects: HashMap::new(), sample_rate, max_frames, errors: Vec::new(), media: crate::media::Media::new(sample_rate) }
    }

    fn key(project: &Project, track: Id) -> Key {
        let node = project.track(track).and_then(|t| t.instrument_node).and_then(|n| project.node(n)).filter(|n| n.kind == NodeKind::Plugin && !n.bypassed && !n.bundle.is_empty());
        match node {
            Some(n) => Key { bundle: n.bundle.clone(), plugin_id: n.plugin_id.clone(), node: Some(n.id) },
            None => Key { bundle: BUILTIN.into(), plugin_id: corinzia_sine::PLUGIN_ID.to_string_lossy().into_owned(), node: None },
        }
    }

    fn library(&mut self, bundle: &str) -> Result<Arc<Library>, String> {
        let path = PathBuf::from(bundle);
        if let Some(l) = self.libs.get(&path) {
            return Ok(l.clone());
        }
        let lib = Library::load(&path)?;
        self.libs.insert(path, lib.clone());
        Ok(lib)
    }

    /// The activated plugin of a track; a plugin that fails to load falls
    /// back to the built-in instrument (and the error is recorded).
    pub fn plugin_for(&mut self, project: &Project, track: Id) -> Option<Arc<Plugin>> {
        let key = Self::key(project, track);
        if let Some((k, p)) = self.tracks.get(&track) {
            if *k == key {
                return Some(p.clone());
            }
        }
        // The plugin node another track plays already: one instance for
        // both (a multi-timbral sampler, each track on its channel).
        if key.node.is_some() {
            if let Some(p) = self.tracks.values().find(|(k, _)| *k == key).map(|(_, p)| p.clone()) {
                self.tracks.insert(track, (key, p.clone()));
                return Some(p);
            }
        }
        // The track changes plugin: the old one's editor goes now (the
        // instance itself when the audio thread lets go of it), unless
        // another track still plays it.
        if let Some((_, old)) = self.tracks.get(&track) {
            if self.tracks.values().filter(|(_, p)| Arc::ptr_eq(p, old)).count() == 1 {
                old.close_editor();
            }
        }
        let made = self.instantiate(&key).and_then(|p| {
            if let Some(state) = key.node.and_then(|n| project.plugin_state.get(&n)) {
                if !p.load_state(state) {
                    self.errors.push(format!("{}: could not restore its state", p.info.name));
                }
            }
            p.activate(self.sample_rate, self.max_frames)?;
            Ok(p)
        });
        let plugin = match made {
            Ok(p) => Arc::new(p),
            Err(e) => {
                self.errors.push(e);
                let fallback = Key { bundle: BUILTIN.into(), plugin_id: corinzia_sine::PLUGIN_ID.to_string_lossy().into_owned(), node: None };
                let p = self.instantiate(&fallback).ok()?;
                p.activate(self.sample_rate, self.max_frames).ok()?;
                Arc::new(p)
            }
        };
        self.tracks.insert(track, (key, plugin.clone()));
        Some(plugin)
    }

    /// The activated effect of an audio node (an insert), or None when it
    /// is bypassed, missing or fails to load (the error is recorded once).
    pub fn effect_for(&mut self, project: &Project, node: Id) -> Option<Arc<Plugin>> {
        let n = project.node(node).filter(|n| n.kind == NodeKind::Plugin && !n.bundle.is_empty())?;
        if n.bypassed {
            return None;
        }
        let key = Key { bundle: n.bundle.clone(), plugin_id: n.plugin_id.clone(), node: Some(node) };
        if let Some((k, p)) = self.effects.get(&node) {
            if *k == key {
                return Some(p.clone());
            }
            p.close_editor();
        }
        let made = self.instantiate(&key).and_then(|p| {
            if let Some(state) = project.plugin_state.get(&node) {
                if !p.load_state(state) {
                    self.errors.push(format!("{}: could not restore its state", p.info.name));
                }
            }
            p.activate(self.sample_rate, self.max_frames)?;
            Ok(p)
        });
        match made {
            Ok(p) => {
                let p = Arc::new(p);
                self.effects.insert(node, (key, p.clone()));
                Some(p)
            }
            Err(e) => {
                if !self.errors.contains(&e) {
                    self.errors.push(e);
                }
                None
            }
        }
    }

    /// The effect instance of a node, if loaded.
    pub fn effect(&self, node: Id) -> Option<&Arc<Plugin>> {
        self.effects.get(&node).map(|(_, p)| p)
    }

    fn instantiate(&mut self, key: &Key) -> Result<Plugin, String> {
        let lib = self.library(&key.bundle)?;
        Plugin::create(&lib, &key.plugin_id)
    }

    /// Forgets tracks that are gone (their plugins are destroyed once the
    /// audio thread has retired the plans that use them).
    pub fn retain_tracks(&mut self, project: &Project) {
        self.tracks.retain(|t, (_, p)| {
            let keep = project.track(*t).is_some();
            if !keep {
                p.close_editor();
            }
            keep
        });
        // Effects no longer inserted anywhere go too.
        let used = |n: Id| project.tracks.iter().any(|t| t.inserts.contains(&n)) || project.buses.iter().any(|b| b.inserts.contains(&n)) || project.master.inserts.contains(&n);
        self.effects.retain(|n, (_, p)| {
            let keep = used(*n);
            if !keep {
                p.close_editor();
            }
            keep
        });
    }

    /// Shows the editor of a track's plugin (brings it to the front when
    /// it is open already).
    pub fn open_editor(&self, track: Id, title: &str) -> Result<(), String> {
        let p = self.plugin_of_track(track).ok_or("The track has no instrument loaded yet")?;
        if !p.has_editor() {
            return Err(format!("{} has no editor of its own", p.info.name));
        }
        p.open_editor(title)
    }

    pub fn close_editor(&self, track: Id) {
        if let Some(p) = self.plugin_of_track(track) {
            p.close_editor();
        }
    }

    pub fn editor_is_open(&self, track: Id) -> bool {
        self.plugin_of_track(track).is_some_and(|p| p.editor_is_open())
    }

    /// Whether any editor is open (the app then pumps the plugin run loop).
    pub fn any_editor_open(&self) -> bool {
        self.tracks.values().chain(self.effects.values()).any(|(_, p)| p.editor_is_open())
    }

    pub fn close_editors(&self) {
        for (_, p) in self.tracks.values().chain(self.effects.values()) {
            p.close_editor();
        }
    }

    /// Main-thread housekeeping for every instance: plugin timers and
    /// descriptors, deferred work, editor windows.
    pub fn idle(&self) {
        crate::gui::pump();
        for (_, p) in self.tracks.values().chain(self.effects.values()) {
            p.idle();
        }
    }

    /// Writes the state of every plugin node into the project (before saving).
    pub fn store_states(&self, project: &mut Project) {
        for (key, p) in self.tracks.values().chain(self.effects.values()) {
            if let (Some(node), Some(state)) = (key.node, p.save_state()) {
                project.plugin_state.insert(node, Arc::new(state));
            }
        }
    }

    pub fn plugin_of_track(&self, track: Id) -> Option<&Arc<Plugin>> {
        self.tracks.get(&track).map(|(_, p)| p)
    }
}
