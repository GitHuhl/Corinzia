//! Plugin instances per track, on the main thread: created from the
//! track's audio node (a CLAP bundle) or the built-in instrument, state
//! restored from the project, activated, and kept across snapshot rebuilds
//! so edits never reload a plugin.

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
    pub sample_rate: f64,
    pub max_frames: u32,
    /// Problems loading plugins (shown in the Mix tab).
    pub errors: Vec<String>,
}

impl PluginPool {
    pub fn new(sample_rate: f64, max_frames: u32) -> PluginPool {
        PluginPool { libs: HashMap::new(), tracks: HashMap::new(), sample_rate, max_frames, errors: Vec::new() }
    }

    fn key(project: &Project, track: Id) -> Key {
        let node = project.track(track).and_then(|t| t.instrument_node).and_then(|n| project.node(n)).filter(|n| n.kind == NodeKind::Clap && !n.bypassed && !n.bundle.is_empty());
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

    fn instantiate(&mut self, key: &Key) -> Result<Plugin, String> {
        let lib = self.library(&key.bundle)?;
        Plugin::create(&lib, &key.plugin_id)
    }

    /// Forgets tracks that are gone (their plugins are destroyed once the
    /// audio thread has retired the plans that use them).
    pub fn retain_tracks(&mut self, project: &Project) {
        self.tracks.retain(|t, _| project.track(*t).is_some());
    }

    /// Main-thread housekeeping for every instance.
    pub fn idle(&self) {
        for (_, p) in self.tracks.values() {
            p.idle();
        }
    }

    /// Writes the state of every plugin node into the project (before saving).
    pub fn store_states(&self, project: &mut Project) {
        for (key, p) in self.tracks.values() {
            if let (Some(node), Some(state)) = (key.node, p.save_state()) {
                project.plugin_state.insert(node, Arc::new(state));
            }
        }
    }

    pub fn plugin_of_track(&self, track: Id) -> Option<&Arc<Plugin>> {
        self.tracks.get(&track).map(|(_, p)| p)
    }
}
