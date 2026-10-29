//! The render plan the audio thread plays: built on the main thread from the
//! project (through the performance compiler), immutable afterwards, handed
//! to the audio thread whole and handed back for dropping. Everything is in
//! absolute samples: movements are laid end to end, notes are resolved to
//! the node (track) that sounds them.

use crate::host::Plugin;
use crate::media::AudioData;
use corinzia_model::{AutoTarget, Id, Project};
use corinzia_perform::tempo::{MeterPoint, TempoMap};
use corinzia_perform::Span;
use std::sync::Arc;

/// What a scheduled event does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Note,
    /// A MIDI controller to the node's instrument; the value is `velocity` (0..1).
    Cc(u8),
    /// A program change to the node's instrument (the program in `key`).
    Program,
    /// The node's fader (linear gain in `velocity`) or pan (-1..1).
    Gain,
    Pan,
    /// A plugin parameter, value in `velocity`: the node's instrument
    /// (`fx` = `u32::MAX`) or an effect (an index into `Snapshot::effects`).
    Param { fx: u32, id: u32 },
    /// A bus's fader or pan (`node` is the bus's index), the master's fader.
    BusGain,
    BusPan,
    MasterGain,
    /// The level (linear) of the node's send `index`.
    Send { index: u32 },
    /// Pitch bend to the node's instrument, -1..1 in `velocity`.
    PitchBend,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScheduledNote {
    pub kind: Kind,
    pub at: i64,
    pub node: u32,
    pub note_id: i32,
    pub port: i16,
    pub channel: i16,
    pub key: i16,
    pub on: bool,
    pub velocity: f32,
    pub element: Id, // the notehead (highlighting)
}

/// One movement on the timeline.
#[derive(Clone, Debug)]
pub struct Section {
    pub movement: Id,
    pub start: i64,
    pub end: i64,
    pub sample_rate: f64,
    pub tempo: TempoMap, // movement-local, performed quarters
    pub meters: Vec<MeterPoint>,
    pub spans: Vec<Span>,
}

impl Section {
    /// Performed quarters at a timeline sample.
    pub fn performed_quarter(&self, sample: i64) -> f64 {
        self.tempo.quarter_at((sample - self.start) as f64 / self.sample_rate)
    }
    /// Performed -> written quarter (the playhead in the score).
    pub fn written_quarter(&self, performed: f64) -> f64 {
        for s in self.spans.iter().rev() {
            if performed >= s.performed_q {
                return s.written_q + (performed - s.performed_q).min(s.length_q);
            }
        }
        performed
    }
    /// Written -> performed quarter, the first pass through that point.
    pub fn performed_from_written(&self, written: f64) -> f64 {
        for s in &self.spans {
            if written >= s.written_q && written < s.written_q + s.length_q {
                return s.performed_q + (written - s.written_q);
            }
        }
        self.spans.last().map(|s| s.performed_q + s.length_q).unwrap_or(written)
    }
    pub fn sample_at_written(&self, written: f64) -> i64 {
        self.start + (self.tempo.seconds_at(self.performed_from_written(written)) * self.sample_rate).round() as i64
    }
}

/// A clip of an audio track on the timeline: `len` frames of `data` from
/// frame `offset`, starting at timeline sample `start`.
#[derive(Clone, Debug)]
pub struct ScheduledClip {
    pub start: i64,
    pub offset: usize,
    pub len: usize,
    pub fade_in: usize,
    pub fade_out: usize,
    pub gain: f32,
    pub data: Arc<AudioData>,
}

impl ScheduledClip {
    pub fn end(&self) -> i64 {
        self.start + self.len as i64
    }
    /// The gain at frame `k` of the clip, fades included.
    pub fn gain_at(&self, k: usize) -> f32 {
        let mut g = self.gain;
        if k < self.fade_in {
            g *= k as f32 / self.fade_in as f32;
        }
        let left = self.len - k;
        if left < self.fade_out {
            g *= left as f32 / self.fade_out as f32;
        }
        g
    }
}

/// A track's sound: its plugin (an instrument track) or its clips (an
/// audio track), and mixer settings.
pub struct Node {
    pub track: Id,
    pub plugin: Option<Arc<Plugin>>,
    /// An audio track's clips, in time order.
    pub clips: Vec<ScheduledClip>,
    pub gain: f32, // linear
    pub pan: f32,  // -1..1
    /// Sound leaves the track (not muted, not silenced by a solo).
    pub audible: bool,
    /// Its output is heard (false while only its sends feed a soloed bus).
    pub direct: bool,
    /// Effects after the instrument, in order.
    pub inserts: Vec<Arc<Plugin>>,
    /// The bus it goes to (an index into `Snapshot::buses`), else the master.
    pub out: Option<u32>,
    /// Sends: (bus index, linear gain, before the fader).
    pub sends: Vec<(u32, f32, bool)>,
    /// The MIDI program change (channel, program, bank) sent when playback starts.
    pub program: Option<(u8, u8, Option<u16>)>,
}

/// A bus: what tracks send or route to it, through its effects and fader,
/// to another bus or the master. In the plan, every bus comes before the
/// bus it goes into.
pub struct BusStrip {
    pub id: Id,
    /// The bus it goes into (a later index), else the master.
    pub out: Option<u32>,
    pub inserts: Vec<Arc<Plugin>>,
    pub gain: f32,
    pub pan: f32,
    pub audible: bool,
}

pub struct Snapshot {
    pub generation: u64,
    pub sample_rate: f64,
    pub sections: Vec<Section>,
    pub notes: Vec<ScheduledNote>, // sorted by (at, offs first)
    pub nodes: Vec<Node>,
    pub buses: Vec<BusStrip>,
    pub master_inserts: Vec<Arc<Plugin>>,
    pub master_gain: f32,
    /// Every effect of the plan (automation names them by index).
    pub effects: Vec<Arc<Plugin>>,
    /// Problems with clips' files (missing, unreadable).
    pub clip_errors: Vec<String>,
}

impl Snapshot {
    /// Where playback ends: the last movement's end, or the end of the
    /// last clip sounding after it.
    pub fn end(&self) -> i64 {
        let clips = self.nodes.iter().flat_map(|n| n.clips.iter().map(|c| c.end())).max().unwrap_or(0);
        self.sections.last().map(|s| s.end).unwrap_or(0).max(clips)
    }
    /// Every plugin the plan runs: instruments, then effects.
    pub fn plugins(&self) -> impl Iterator<Item = &Arc<Plugin>> {
        self.nodes
            .iter()
            .flat_map(|n| n.plugin.iter().chain(n.inserts.iter()))
            .chain(self.buses.iter().flat_map(|b| b.inserts.iter()))
            .chain(self.master_inserts.iter())
    }
    pub fn first_note_at_or_after(&self, s: i64) -> usize {
        self.notes.partition_point(|n| n.at < s)
    }
    pub fn section_at(&self, s: i64) -> Option<&Section> {
        self.sections.iter().find(|x| s < x.end).or(self.sections.last())
    }
    /// The parts the UI keeps to map the playhead (no plugins).
    pub fn timeline(&self) -> Timeline {
        Timeline { sections: self.sections.clone(), generation: self.generation }
    }
}

/// The timing half of a snapshot, kept by the UI.
#[derive(Clone, Debug, Default)]
pub struct Timeline {
    pub sections: Vec<Section>,
    pub generation: u64,
}
impl Timeline {
    /// (movement, written quarter) at a timeline sample.
    pub fn locate(&self, sample: i64) -> Option<(Id, f64)> {
        let s = self.sections.iter().find(|x| sample < x.end).or(self.sections.last())?;
        let q = s.written_quarter(s.performed_quarter(sample.clamp(s.start, s.end)));
        Some((s.movement, q))
    }
    /// The timeline sample of a written position in a movement.
    pub fn sample_of(&self, movement: Id, written_q: f64) -> Option<i64> {
        self.sections.iter().find(|s| s.movement == movement).map(|s| s.sample_at_written(written_q))
    }
    pub fn end(&self) -> i64 {
        self.sections.last().map(|s| s.end).unwrap_or(0)
    }
}

/// Mixer settings -> (linear gain, pan, audible) per track, solo aware.
/// The fader's lowest value: at or below it a track is silent.
pub const MIN_DB: f64 = -60.0;

pub fn db_to_gain(db: f64) -> f32 {
    if db <= MIN_DB {
        0.0
    } else {
        10f32.powf(db as f32 / 20.0)
    }
}

/// A bus and the buses it goes into, in order (a loop is cut).
pub fn bus_chain(project: &Project, bus: Id) -> Vec<Id> {
    let mut chain = vec![];
    let mut at = Some(bus);
    while let Some(b) = at.and_then(|b| project.bus(b)) {
        if chain.contains(&b.id) {
            break;
        }
        chain.push(b.id);
        at = b.output;
    }
    chain
}

/// What solo leaves heard. Soloing a track keeps the buses it reaches
/// (through its output or sends) heard; soloing a bus keeps what feeds it
/// and the buses after it heard. A track that only reaches a soloed bus
/// through a send is heard there and not on its own output.
/// (track, audible, direct) per track and (bus, heard) per bus.
type Heard = (Vec<(Id, bool, bool)>, Vec<(Id, bool)>);

fn solo(project: &Project) -> Heard {
    let soloed: Vec<Id> = project.buses.iter().filter(|b| b.solo).map(|b| b.id).collect();
    let any = !soloed.is_empty() || project.tracks.iter().any(|t| t.solo);
    let out_chain = |t: &corinzia_model::Track| t.output_bus.map(|b| bus_chain(project, b)).unwrap_or_default();
    let send_chains = |t: &corinzia_model::Track| t.sends.iter().flat_map(|s| bus_chain(project, s.bus)).collect::<Vec<_>>();
    let meets = |chain: &[Id]| chain.iter().any(|b| soloed.contains(b));
    let tracks = project
        .tracks
        .iter()
        .map(|t| {
            if !any {
                return (t.id, !t.mute, true);
            }
            let direct = t.solo || meets(&out_chain(t));
            let via_send = meets(&send_chains(t));
            (t.id, !t.mute && (direct || via_send), direct)
        })
        .collect();
    let buses = project
        .buses
        .iter()
        .map(|b| {
            if !any {
                return (b.id, !b.mute);
            }
            let fed_by_solo = project.tracks.iter().filter(|t| t.solo).any(|t| out_chain(t).contains(&b.id) || send_chains(t).contains(&b.id))
                || soloed.iter().any(|s| bus_chain(project, *s).contains(&b.id));
            (b.id, !b.mute && (meets(&bus_chain(project, b.id)) || fed_by_solo))
        })
        .collect();
    (tracks, buses)
}

/// Buses' levels: (bus, linear gain, pan, audible).
pub fn bus_mix(project: &Project) -> Vec<(Id, f32, f32, bool)> {
    let (_, heard) = solo(project);
    project.buses.iter().zip(heard).map(|(b, (_, h))| (b.id, db_to_gain(b.gain_db), b.pan.clamp(-1.0, 1.0) as f32, h)).collect()
}

/// Sends' levels by track: (track, send index, linear gain).
pub fn send_mix(project: &Project) -> Vec<(Id, usize, f32)> {
    project.tracks.iter().flat_map(|t| t.sends.iter().enumerate().map(move |(i, s)| (t.id, i, db_to_gain(s.gain_db)))).collect()
}

/// What the plan's shape depends on (inserts, routing, bypass): a change
/// needs a new plan, level changes do not.
pub fn routing(project: &Project) -> String {
    let fx = |ids: &[Id]| ids.iter().map(|n| format!("{n}{}", if project.node(*n).is_some_and(|x| x.bypassed) { "b" } else { "" })).collect::<Vec<_>>().join(",");
    let mut s = String::new();
    for t in &project.tracks {
        s += &format!("t{}[{}]>{:?}{:?}p{:?}{:?};", t.id, fx(&t.inserts), t.output_bus, t.sends.iter().map(|x| (x.bus, x.pre)).collect::<Vec<_>>(), t.program, t.bank);
    }
    for b in &project.buses {
        s += &format!("b{}[{}]>{:?};", b.id, fx(&b.inserts), b.output);
    }
    // Automation and dynamics controllers are in the plan too.
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for m in &project.movements {
        format!("{:?}", m.automation).hash(&mut h);
    }
    for t in &project.tracks {
        t.dynamics_cc.hash(&mut h);
        format!("{:?}", t.techniques).hash(&mut h);
    }
    s + &format!("m[{}]a{:x}", fx(&project.master.inserts), h.finish())
}

/// A plugin the plan needs: a track's instrument, or an effect node.
pub enum Want {
    Instrument(Id),
    Effect(Id),
}

/// Builds the plan with instruments and effects from the pool.
pub fn build_from_pool(project: &Project, sample_rate: f64, generation: u64, movements: &[usize], pool: &mut crate::pool::PluginPool) -> Snapshot {
    let used: Vec<String> = project.tracks.iter().flat_map(|t| t.clips.iter().map(|c| c.file.clone())).collect();
    // The media leaves the pool while the plugins are fetched from it.
    let mut media = std::mem::replace(&mut pool.media, crate::media::Media::new(sample_rate));
    media.retain(&used);
    let snap = build_with(project, sample_rate, generation, movements, &mut |w| match w {
        Want::Instrument(t) => pool.plugin_for(project, t),
        Want::Effect(n) => pool.effect_for(project, n),
    }, &mut |file| media.get(file));
    pool.media = media;
    snap
}

/// Tracks' levels: (track, linear gain, pan, audible, direct) (see `solo`).
pub fn mix(project: &Project) -> Vec<(Id, f32, f32, bool, bool)> {
    let (heard, _) = solo(project);
    project
        .tracks
        .iter()
        .zip(heard)
        // The fader's bottom is silence (−∞ dB).
        .map(|(t, (_, audible, direct))| (t.id, if t.gain_db <= MIN_DB { 0.0 } else { 10f32.powf(t.gain_db as f32 / 20.0) }, t.pan.clamp(-1.0, 1.0) as f32, audible, direct))
        .collect()
}

/// Builds the plan. `plugins(track)` gives the activated plugin of a track.
pub fn build(project: &Project, sample_rate: f64, generation: u64, movements: &[usize], mut plugins: impl FnMut(Id) -> Option<Arc<Plugin>>) -> Snapshot {
    build_with(project, sample_rate, generation, movements, &mut |w| match w {
        Want::Instrument(t) => plugins(t),
        Want::Effect(_) => None,
    }, &mut |_| Err("no media".into()))
}

fn build_with(project: &Project, sample_rate: f64, generation: u64, movements: &[usize], get: &mut dyn FnMut(Want) -> Option<Arc<Plugin>>, sound: &mut dyn FnMut(&str) -> Result<Arc<AudioData>, String>) -> Snapshot {
    const GAP_SECONDS: f64 = 2.0;
    let mut fx_nodes: Vec<(Id, Arc<Plugin>)> = Vec::new();
    let mut effects = |ids: &[Id], get: &mut dyn FnMut(Want) -> Option<Arc<Plugin>>| -> Vec<Arc<Plugin>> {
        ids.iter()
            .filter_map(|&n| Some((n, get(Want::Effect(n))?)))
            .filter(|(_, p)| p.input_channels > 0)
            .map(|(n, p)| {
                fx_nodes.push((n, p.clone()));
                p
            })
            .collect()
    };
    // Buses into others come before them: deepest chains first.
    let mut levels = bus_mix(project);
    levels.sort_by_key(|l| std::cmp::Reverse(bus_chain(project, l.0).len()));
    let order: Vec<Id> = levels.iter().map(|l| l.0).collect();
    let buses: Vec<BusStrip> = levels
        .into_iter()
        .enumerate()
        .map(|(i, (id, gain, pan, audible))| {
            let out = project.bus(id).and_then(|b| b.output).and_then(|o| order.iter().position(|x| *x == o)).filter(|&o| o > i).map(|o| o as u32);
            BusStrip { id, out, inserts: effects(&project.bus(id).map(|b| b.inserts.clone()).unwrap_or_default(), get), gain, pan, audible }
        })
        .collect();
    let bus_index = |id: Id| buses.iter().position(|b| b.id == id).map(|i| i as u32);
    let mut nodes: Vec<Node> = Vec::new();
    for (track, gain, pan, audible, direct) in mix(project) {
        let audio = project.track(track).is_some_and(|t| t.audio);
        let plugin = if audio { None } else { get(Want::Instrument(track)) };
        if audio || plugin.is_some() {
            let t = project.track(track);
            let inserts = effects(&t.map(|t| t.inserts.clone()).unwrap_or_default(), get);
            let out = t.and_then(|t| t.output_bus).and_then(bus_index);
            // Indices follow the track's sends (a send to a missing bus goes nowhere).
            let sends = t.map(|t| t.sends.iter().map(|s| (bus_index(s.bus).unwrap_or(u32::MAX), db_to_gain(s.gain_db), s.pre)).collect()).unwrap_or_default();
            let program = t.and_then(|t| Some((t.midi_channel & 15, t.program?, t.bank)));
            nodes.push(Node { track, plugin, clips: vec![], gain, pan, audible, direct, inserts, out, sends, program });
        }
    }
    let master_inserts = effects(&project.master.inserts, get);
    let master_gain = db_to_gain(project.master.gain_db);
    let node_of = |track: Id| nodes.iter().position(|n| n.track == track).map(|i| i as u32);
    let mut sections = Vec::new();
    let mut notes = Vec::new();
    let mut start = 0i64;
    let mut note_id = 0i32;
    for &mi in movements {
        let Some(m) = project.movements.get(mi) else { continue };
        let perf = corinzia_perform::perform(m);
        let to_sample = |q: f64| start + (perf.tempo.seconds_at(q) * sample_rate).round() as i64;
        // Expression maps: per track, its notes in time order.
        let mut by_track: std::collections::HashMap<Id, Vec<usize>> = Default::default();
        for (i, n) in perf.notes.iter().enumerate() {
            if let Some(t) = project.instrument(n.instrument).and_then(|i| i.track) {
                by_track.entry(t).or_default().push(i);
            }
        }
        let mut channels: Vec<Option<u8>> = vec![None; perf.notes.len()];
        for (track, mut idx) in by_track {
            let (Some(t), Some(node)) = (project.track(track), node_of(track)) else { continue };
            if t.techniques.is_empty() {
                continue;
            }
            idx.sort_by(|a, b| perf.notes[*a].on_q.total_cmp(&perf.notes[*b].on_q));
            let list: Vec<&corinzia_perform::PerformedNote> = idx.iter().map(|&i| &perf.notes[i]).collect();
            for (k, e) in corinzia_perform::expression(t, &list).into_iter().enumerate() {
                channels[idx[k]] = e.channel;
                let on = start + (perf.tempo.seconds_at(perf.notes[idx[k]].on_q) * sample_rate).round() as i64;
                let channel = t.midi_channel as i16 & 15;
                let ev = |kind: Kind, at: i64, key: i16, on: bool, velocity: f32| ScheduledNote { kind, at: at.max(0), node, note_id: -1, port: 0, channel, key, on, velocity, element: 0 };
                for a in e.before {
                    match a {
                        // Pressed and let go just before the note.
                        corinzia_model::SwitchAction::Keyswitch(key) => {
                            notes.push(ScheduledNote { note_id: -2, ..ev(Kind::Note, on - 3, key as i16, true, 0.8) });
                            notes.push(ScheduledNote { note_id: -2, ..ev(Kind::Note, (on - 2).max((on - 3).max(0) + 1), key as i16, false, 0.0) });
                        }
                        corinzia_model::SwitchAction::Program(p) => notes.push(ev(Kind::Program, on - 3, p as i16, false, 0.0)),
                        corinzia_model::SwitchAction::Cc(cc, v) => notes.push(ev(Kind::Cc(cc), on - 3, 0, false, v as f32 / 127.0)),
                        corinzia_model::SwitchAction::Channel(_) => {}
                    }
                }
            }
        }
        for (ni, n) in perf.notes.iter().enumerate() {
            let Some(track) = project.instrument(n.instrument).and_then(|i| i.track) else { continue };
            let Some(node) = node_of(track) else { continue };
            let channel = channels[ni].map(|c| c as i16).unwrap_or_else(|| project.track(track).map(|t| t.midi_channel as i16 & 15).unwrap_or(0));
            let port = project.track(track).map(|t| t.note_port as i16).unwrap_or(0);
            let on_sec = perf.tempo.seconds_at(n.on_q) + n.roll_sec;
            let off_sec = on_sec + (perf.tempo.seconds_at(n.full_off_q) - on_sec) * n.gate;
            let on = start + (on_sec * sample_rate).round() as i64;
            let off = (start + (off_sec * sample_rate).round() as i64).max(on + 1);
            let key = project.sounding_key(n.instrument, n.key).clamp(0, 127) as i16;
            let base = ScheduledNote { kind: Kind::Note, at: on, node, note_id, port, channel, key, on: true, velocity: n.velocity, element: n.element };
            notes.push(base);
            notes.push(ScheduledNote { at: off, on: false, ..base });
            note_id = note_id.wrapping_add(1) & 0x7fff_ffff;
        }
        // Dynamics as a controller, where a track asks for it: at each onset.
        let mut last_cc: Vec<Option<u8>> = vec![None; nodes.len()];
        for n in &perf.notes {
            let Some(t) = project.instrument(n.instrument).and_then(|i| i.track).and_then(|t| project.track(t)) else { continue };
            let (Some(cc), Some(node)) = (t.dynamics_cc, node_of(t.id)) else { continue };
            let value = (n.velocity.clamp(0.0, 1.0) * 127.0).round() as u8;
            if last_cc[node as usize] == Some(value) {
                continue;
            }
            last_cc[node as usize] = Some(value);
            let at = start + ((perf.tempo.seconds_at(n.on_q) * sample_rate).round() as i64 - 1).max(0);
            let channel = t.midi_channel as i16 & 15;
            notes.push(ScheduledNote { kind: Kind::Cc(cc), at, node, note_id: -1, port: 0, channel, key: 0, on: false, velocity: value as f32 / 127.0, element: 0 });
        }
        // Automation: through the playback order (repeats too), every
        // sixteenth of a quarter where the value changes.
        for lane in &m.automation {
            let channel_of = |t: Id| project.track(t).map(|t| t.midi_channel as i16 & 15).unwrap_or(0);
            let mut channel = 0;
            let (node, kind, scale): (Option<u32>, Kind, fn(f64) -> f32) = match lane.target {
                AutoTarget::Gain(corinzia_model::MASTER) => (Some(0), Kind::MasterGain, |db| db_to_gain(db)),
                AutoTarget::Gain(t) if node_of(t).is_none() => (bus_index(t), Kind::BusGain, |db| db_to_gain(db)),
                AutoTarget::Pan(t) if node_of(t).is_none() => (bus_index(t), Kind::BusPan, |p| p.clamp(-1.0, 1.0) as f32),
                AutoTarget::Gain(t) => (node_of(t), Kind::Gain, |db| db_to_gain(db)),
                AutoTarget::Pan(t) => (node_of(t), Kind::Pan, |p| p.clamp(-1.0, 1.0) as f32),
                AutoTarget::Send { track, bus } => match project.track(track).and_then(|t| t.sends.iter().position(|s| s.bus == bus)) {
                    Some(i) => (node_of(track), Kind::Send { index: i as u32 }, |db| db_to_gain(db)),
                    None => continue,
                },
                AutoTarget::Cc { track, cc } => {
                    channel = channel_of(track);
                    (node_of(track), Kind::Cc(cc), |v| (v.clamp(0.0, 127.0) / 127.0) as f32)
                }
                AutoTarget::PitchBend(track) => {
                    channel = channel_of(track);
                    (node_of(track), Kind::PitchBend, |v| v.clamp(-1.0, 1.0) as f32)
                }
                AutoTarget::Param { node: n, param } => match fx_nodes.iter().position(|x| x.0 == n) {
                    Some(k) => (Some(0), Kind::Param { fx: k as u32, id: param }, |v| v as f32),
                    None => (project.tracks.iter().find(|t| t.instrument_node == Some(n)).and_then(|t| node_of(t.id)), Kind::Param { fx: u32::MAX, id: param }, |v| v as f32),
                },
            };
            let Some(node) = node else { continue };
            let mut last: Option<f32> = None;
            for sp in &perf.spans {
                let steps = (sp.length_q * 16.0).ceil() as usize;
                for k in 0..=steps {
                    let dq = (k as f64 / 16.0).min(sp.length_q);
                    let Some(v) = lane.value_at(sp.written_q + dq) else { continue };
                    let v = scale(v);
                    if last == Some(v) {
                        continue;
                    }
                    last = Some(v);
                    let at = start + (perf.tempo.seconds_at(sp.performed_q + dq) * sample_rate).round() as i64;
                    notes.push(ScheduledNote { kind, at, node, note_id: -1, port: 0, channel, key: 0, on: false, velocity: v, element: 0 });
                }
            }
        }
        let end = to_sample(perf.length_q);
        sections.push(Section { movement: m.id, start, end, sample_rate, tempo: perf.tempo.clone(), meters: perf.meter_points.clone(), spans: perf.spans.clone() });
        start = end + (GAP_SECONDS * sample_rate) as i64;
    }
    // Audio tracks' clips, at fixed times after their movements' starts.
    let mut clip_errors = Vec::new();
    for node in nodes.iter_mut().filter(|n| n.plugin.is_none()) {
        let Some(t) = project.track(node.track) else { continue };
        for c in t.clips.iter().filter(|c| !c.mute) {
            let Some(sec) = sections.iter().find(|s| s.movement == c.movement) else { continue };
            let data = match sound(&c.file) {
                Ok(d) => d,
                Err(e) => {
                    if !clip_errors.contains(&e) {
                        clip_errors.push(e);
                    }
                    continue;
                }
            };
            let frames = |s: f64| (s.max(0.0) * sample_rate).round() as usize;
            let offset = frames(c.offset).min(data.frames());
            let len = (data.frames() - offset).min(if c.length > 0.0 { frames(c.length) } else { usize::MAX });
            if len == 0 {
                continue;
            }
            let (fade_in, fade_out) = (frames(c.fade_in).min(len), frames(c.fade_out).min(len));
            node.clips.push(ScheduledClip { start: sec.start + frames(c.at) as i64, offset, len, fade_in, fade_out, gain: db_to_gain(c.gain_db), data });
        }
        node.clips.sort_by_key(|c| c.start);
    }
    // Offs before ons at the same sample: a repeated key restarts cleanly.
    notes.sort_by_key(|n| (n.at, n.on));
    let effects = fx_nodes.into_iter().map(|x| x.1).collect();
    Snapshot { generation, sample_rate, sections, notes, nodes, buses, master_inserts, master_gain, effects, clip_errors }
}
