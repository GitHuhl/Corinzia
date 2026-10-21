//! Playback for the app: the engine handle, the audio output, plugin
//! instances, and the plan kept in step with the editor (rebuilt when the
//! music changes, mixer settings sent live otherwise).

use corinzia_audio::engine::{self, Command, Engine, EngineHandle};
use corinzia_audio::host::{self, PluginInfo};
use corinzia_audio::pool::PluginPool;
use corinzia_audio::snapshot::{self, Timeline};
use corinzia_model::{Id, Project};
use std::time::{Duration, Instant};


pub struct Playback {
    handle: EngineHandle,
    #[cfg(feature = "audio-out")]
    _output: Option<corinzia_audio::backend::Output>,
    /// Without an output (tests, no PipeWire), the engine stays here.
    #[cfg_attr(not(test), allow(dead_code))]
    pub offline: Option<Engine>,
    pub pool: PluginPool,
    pub timeline: Timeline,
    /// Track of each node of the current plan.
    node_tracks: Vec<Id>,
    built: Option<u64>,  // music revision of the plan
    mixed: Option<u64>,  // revision whose mixer settings were sent
    generation: u64,
    previews: Vec<(Instant, u32, i16)>,
    /// Set when PipeWire is unavailable (the details go to stderr).
    pub problem: Option<String>,
    pub plugins: Vec<PluginInfo>,
    pub scan_errors: Vec<String>,
    pub sample_rate: f64,
    /// Where plugins are looked for (from the preferences).
    pub search_paths: Vec<std::path::PathBuf>,
}

impl Playback {
    /// Starts audio output (PipeWire, or WASAPI on Windows) unless `live`
    /// is false, at the preferences' rate and buffer, on their device.
    pub fn new(live: bool, prefs: &crate::prefs::Prefs) -> Playback {
        let (rate, frames) = (prefs.sample_rate as f64, prefs.buffer);
        let (handle, eng) = engine::new(rate, frames);
        let mut offline = Some(eng);
        let mut problem = None;
        #[cfg(feature = "audio-out")]
        let mut output = None;
        if live {
            #[cfg(feature = "audio-out")]
            match corinzia_audio::backend::start(offline.take().unwrap(), prefs.sample_rate, frames, &prefs.output_device) {
                Ok(o) => output = Some(o),
                Err(e) => {
                    eprintln!("corinzia: no audio output: {e}");
                    problem = Some(if cfg!(windows) { "No audio output (no audio device)" } else { "No audio output (PipeWire unavailable)" }.into());
                }
            }
            #[cfg(not(feature = "audio-out"))]
            {
                problem = Some("Built without audio output".into());
            }
        }
        Playback {
            handle,
            #[cfg(feature = "audio-out")]
            _output: output,
            offline,
            pool: PluginPool::new(rate, frames),
            timeline: Timeline::default(),
            node_tracks: Vec::new(),
            built: None,
            mixed: None,
            generation: 0,
            previews: Vec::new(),
            problem,
            plugins: Vec::new(),
            scan_errors: Vec::new(),
            sample_rate: rate,
            search_paths: prefs.plugin_search_paths(),
        }
    }

    /// Keeps the plan in step: rebuild on music changes, else send the mixer.
    pub fn sync(&mut self, project: &Project, music_revision: u64, revision: u64) {
        if self.built != Some(music_revision) {
            self.built = Some(music_revision);
            self.mixed = Some(revision);
            self.generation += 1;
            self.pool.retain_tracks(project);
            let movements: Vec<usize> = (0..project.movements.len()).collect();
            let pool = &mut self.pool;
            let snap = snapshot::build(project, self.sample_rate, self.generation, &movements, |t| pool.plugin_for(project, t));
            self.node_tracks = snap.nodes.iter().map(|n| n.track).collect();
            self.timeline = snap.timeline();
            self.handle.swap(snap);
        } else if self.mixed != Some(revision) {
            self.mixed = Some(revision);
            for (track, gain, pan, audible) in snapshot::mix(project) {
                if let Some(node) = self.node_tracks.iter().position(|t| *t == track) {
                    let _ = self.handle.send(Command::Mix { node: node as u32, gain, pan, audible });
                }
            }
        }
    }

    /// A live mixer value while a slider moves (committed separately).
    pub fn live_mix(&mut self, project: &Project, track: Id) {
        if let (Some(node), Some((_, gain, pan, audible))) = (self.node_tracks.iter().position(|t| *t == track), snapshot::mix(project).into_iter().find(|m| m.0 == track)) {
            let _ = self.handle.send(Command::Mix { node: node as u32, gain, pan, audible });
        }
    }

    pub fn is_playing(&self) -> bool {
        self.handle.is_playing()
    }

    /// Plays from a written position of a movement.
    pub fn play_from(&mut self, movement: Id, written_quarter: f64) {
        if let Some(s) = self.timeline.sample_of(movement, written_quarter) {
            self.handle.seek(s);
        }
        self.handle.play();
    }

    pub fn stop(&mut self) {
        self.handle.stop();
    }

    /// (movement, written quarter) under the playhead while playing.
    pub fn playhead(&self) -> Option<(Id, f64)> {
        self.is_playing().then(|| self.timeline.locate(self.handle.position())).flatten()
    }

    /// Auditions keys on a track's instrument for a moment.
    pub fn preview(&mut self, track: Id, channel: i16, keys: &[i32]) {
        let Some(node) = self.node_tracks.iter().position(|t| *t == track) else { return };
        for &k in keys {
            let key = k.clamp(0, 127) as i16;
            if self.handle.send(Command::Preview { node: node as u32, key, channel, velocity: 0.7 }).is_ok() {
                self.previews.push((Instant::now() + Duration::from_millis(400), node as u32, key));
            }
        }
    }

    /// Per frame (UI thread): end previews, drop retired plans, plugin idle.
    pub fn tick(&mut self) {
        let now = Instant::now();
        let mut i = 0;
        while i < self.previews.len() {
            if self.previews[i].0 <= now {
                let (_, node, key) = self.previews.swap_remove(i);
                let _ = self.handle.send(Command::Preview { node, key, channel: -1, velocity: -1.0 });
            } else {
                i += 1;
            }
        }
        self.handle.collect();
        self.pool.idle();
    }

    /// Plugin editors are open, or plugins have timers or descriptors to
    /// look after: the app keeps ticking.
    pub fn editors_busy(&self) -> bool {
        self.pool.any_editor_open() || corinzia_audio::gui::has_work()
    }

    /// Needs per-frame ticks (playing, or previews pending).
    pub fn busy(&self) -> bool {
        self.is_playing() || !self.previews.is_empty()
    }

    pub fn scan(&mut self) {
        let (infos, errors) = host::scan(&self.search_paths);
        self.plugins = infos;
        self.scan_errors = errors;
    }

    /// Output devices to choose from: (id, label).
    pub fn devices() -> Vec<(String, String)> {
        #[cfg(feature = "audio-out")]
        return corinzia_audio::backend::devices();
        #[cfg(not(feature = "audio-out"))]
        Vec::new()
    }

    /// The plugin name a track plays through.
    pub fn plugin_name(&self, track: Id) -> String {
        self.pool.plugin_of_track(track).map(|p| p.info.name.clone()).unwrap_or_else(|| "—".into())
    }
}
