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
    routing: String,     // the routing the plan was built for (see `snapshot::routing`)
    /// Bus of each bus strip of the plan.
    plan_buses: Vec<Id>,
    mixed: Option<u64>,  // revision whose mixer settings were sent
    generation: u64,
    previews: Vec<(Instant, u32, i16)>,
    /// Set when PipeWire is unavailable (the details go to stderr).
    pub problem: Option<String>,
    pub plugins: Vec<PluginInfo>,
    /// Clips whose files could not be read (shown in the Mix tab).
    pub clip_errors: Vec<String>,
    /// The cycle: a movement's written stretch (quarters), and whether it is on.
    pub cycle: Option<(Id, f64, f64)>,
    pub cycling: bool,
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
            routing: String::new(),
            plan_buses: Vec::new(),
            mixed: None,
            generation: 0,
            previews: Vec::new(),
            problem,
            plugins: Vec::new(),
            clip_errors: Vec::new(),
            cycle: None,
            cycling: false,
            scan_errors: Vec::new(),
            sample_rate: rate,
            search_paths: prefs.plugin_search_paths(),
        }
    }

    /// Keeps the plan in step: rebuild on music changes, else send the mixer.
    /// `folder`: the project file's, where clips' relative paths start.
    pub fn sync(&mut self, project: &Project, music_revision: u64, revision: u64, folder: Option<&std::path::Path>) {
        if self.pool.media.base.as_deref() != folder {
            self.pool.media.base = folder.map(Into::into);
            self.built = None; // the same names may now be other files
        }
        let routing = snapshot::routing(project);
        if self.built != Some(music_revision) || routing != self.routing {
            self.built = Some(music_revision);
            self.routing = routing;
            self.mixed = Some(revision);
            self.generation += 1;
            self.pool.retain_tracks(project);
            let movements: Vec<usize> = (0..project.movements.len()).collect();
            let snap = snapshot::build_from_pool(project, self.sample_rate, self.generation, &movements, &mut self.pool);
            self.node_tracks = snap.nodes.iter().map(|n| n.track).collect();
            self.plan_buses = snap.buses.iter().map(|b| b.id).collect();
            self.timeline = snap.timeline();
            self.clip_errors = snap.clip_errors.clone();
            self.handle.swap(snap);
            // Its samples move with the plan.
            self.send_cycle();
        } else if self.mixed != Some(revision) {
            self.mixed = Some(revision);
            for (track, gain, pan, audible, direct) in snapshot::mix(project) {
                if let Some(node) = self.node_tracks.iter().position(|t| *t == track) {
                    let _ = self.handle.send(Command::Mix { node: node as u32, gain, pan, audible, direct });
                }
            }
            self.send_levels(project);
        }
    }

    /// Buses, sends and the master: their levels, live.
    fn send_levels(&mut self, project: &Project) {
        for (bus, gain, pan, audible) in snapshot::bus_mix(project) {
            if let Some(i) = self.plan_buses.iter().position(|b| *b == bus) {
                let _ = self.handle.send(Command::MixBus { bus: i as u32, gain, pan, audible });
            }
        }
        for (track, index, gain) in snapshot::send_mix(project) {
            if let Some(node) = self.node_tracks.iter().position(|t| *t == track) {
                let _ = self.handle.send(Command::Send { node: node as u32, index: index as u32, gain });
            }
        }
        let _ = self.handle.send(Command::Master { gain: snapshot::db_to_gain(project.master.gain_db) });
    }

    /// A bus's, a send's or the master's level while a control moves.
    pub fn live_levels(&mut self, project: &Project) {
        self.send_levels(project);
    }

    /// A live mixer value while a slider moves (committed separately).
    pub fn live_mix(&mut self, project: &Project, track: Id) {
        if let (Some(node), Some((_, gain, pan, audible, direct))) = (self.node_tracks.iter().position(|t| *t == track), snapshot::mix(project).into_iter().find(|m| m.0 == track)) {
            let _ = self.handle.send(Command::Mix { node: node as u32, gain, pan, audible, direct });
        }
    }

    /// Sets the cycle (and turns it on), or turns it off (`None`).
    pub fn set_cycle(&mut self, cycle: Option<(Id, f64, f64)>) {
        if let Some((m, a, b)) = cycle {
            self.cycle = Some((m, a.min(b), a.max(b)));
        }
        self.cycling = cycle.is_some();
        self.send_cycle();
    }

    /// The cycle on or off (on: the one set last).
    pub fn toggle_cycle(&mut self) {
        self.cycling = !self.cycling && self.cycle.is_some();
        self.send_cycle();
    }

    fn send_cycle(&mut self) {
        let samples = self.cycle.filter(|_| self.cycling).and_then(|(m, a, b)| Some((self.timeline.sample_of(m, a)?, self.timeline.sample_of(m, b)?)));
        let _ = self.handle.send(Command::Loop(samples));
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

    /// Scans again from scratch (the cache forgotten): a plugin that timed
    /// out or crashed gets another try.
    pub fn rescan(&mut self) {
        if let Some(c) = crate::prefs::folder().map(|d| d.join("plugins.json")).filter(|_| !cfg!(test)) {
            let _ = std::fs::remove_file(c);
        }
        self.scan();
    }

    pub fn scan(&mut self) {
        // Each bundle is opened by a child process (a crashing plugin takes
        // only that down), and remembered until it changes.
        let (infos, errors) = if cfg!(test) {
            corinzia_audio::scan::scan_with(&self.search_paths, None, |b| host::Library::load(b).map(|l| l.plugins()))
        } else {
            let exe = std::env::current_exe().unwrap_or_else(|_| "corinzia".into());
            let cache = crate::prefs::folder().map(|d| d.join("plugins.json"));
            corinzia_audio::scan::scan_with(&self.search_paths, cache.as_deref(), |b| corinzia_audio::scan::run_child(&exe, b, std::time::Duration::from_secs(30)))
        };
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
