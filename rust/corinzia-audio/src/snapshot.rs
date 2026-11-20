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
    pub in_shape: corinzia_model::FadeShape,
    pub out_shape: corinzia_model::FadeShape,
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
            g *= self.in_shape.gain(k as f32 / self.fade_in as f32);
        }
        let left = self.len - k;
        if left < self.fade_out {
            g *= self.out_shape.gain(left as f32 / self.fade_out as f32);
        }
        g
    }
}

/// Where clips on one track overlap, the earlier fades out and the later
/// in over the overlap at equal power (a crossfade), unless their own
/// fades there are longer. `clips` sorted by start.
pub fn crossfade(clips: &mut [ScheduledClip]) {
    use corinzia_model::FadeShape;
    for i in 0..clips.len() {
        for j in i + 1..clips.len() {
            let overlap = clips[i].end() - clips[j].start;
            if overlap <= 0 {
                break;
            }
            // The later clip may lie wholly inside: then it is laid over, no fade.
            if clips[j].end() <= clips[i].end() {
                continue;
            }
            let n = overlap as usize;
            if clips[i].fade_out < n {
                (clips[i].fade_out, clips[i].out_shape) = (n.min(clips[i].len), FadeShape::EqualPower);
            }
            if clips[j].fade_in < n {
                (clips[j].fade_in, clips[j].in_shape) = (n.min(clips[j].len), FadeShape::EqualPower);
            }
        }
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
    /// A track sharing another's plugin instance (by its plugin node): the
    /// node (an index) whose instance plays its notes, on its `channel`.
    /// Its fader and pan become MIDI volume and pan on that channel.
    pub host: Option<u32>,
    pub channel: u8,
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
    /// At a timeline sample: the movement, the written quarter, the seconds
    /// since the movement began, and the tempo (quarters a minute).
    pub fn clock(&self, sample: i64) -> Option<(Id, f64, f64, f64)> {
        let s = self.sections.iter().find(|x| sample < x.end).or(self.sections.last())?;
        let at = sample.clamp(s.start, s.end);
        let performed = s.performed_quarter(at);
        Some((s.movement, s.written_quarter(performed), (at - s.start) as f64 / s.sample_rate, s.tempo.tempo_at(performed)))
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

/// A tone of a melody given note by note (a Byzantine chant): times in
/// beats, the key and the pitch bend (-1..1 of the instrument's range,
/// ±2 semitones by MIDI convention) that make its pitch.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tone {
    pub on: f64,
    pub off: f64,
    pub key: i16,
    pub bend: f32,
    pub channel: i16,
    pub velocity: f32,
    /// The element sounding (highlighting).
    pub element: Id,
}

/// A plan that plays `tones` on `track`'s instrument, through the
/// project's mixer, as one section (`section`: its id on the timeline,
/// `bpm`: the beat, `length`: in beats). Each tone's bend is set on its
/// channel just before it starts.
pub fn build_tones_from_pool(project: &Project, sample_rate: f64, generation: u64, section: Id, bpm: f64, length: f64, track: Id, tones: &[Tone], pool: &mut crate::pool::PluginPool) -> Snapshot {
    let mut snap = build_from_pool(project, sample_rate, generation, &[], pool);
    let tempo = TempoMap::new(&[corinzia_perform::tempo::TempoPoint { quarter: 0.0, bpm, ramp_to_next: false }], &[]);
    let to_sample = |beat: f64| (tempo.seconds_at(beat.max(0.0)) * sample_rate).round() as i64;
    if let Some(strip) = snap.nodes.iter().position(|n| n.track == track) {
        let node = snap.nodes[strip].host.unwrap_or(strip as u32);
        let port = project.track(track).map(|t| t.note_port as i16).unwrap_or(0);
        for (k, t) in tones.iter().enumerate() {
            let on = to_sample(t.on);
            let off = to_sample(t.off).max(on + 1);
            let base = ScheduledNote { kind: Kind::Note, at: on, node, note_id: k as i32, port, channel: t.channel, key: t.key, on: true, velocity: t.velocity, element: t.element };
            snap.notes.push(ScheduledNote { kind: Kind::PitchBend, at: (on - 1).max(0), note_id: -1, on: false, velocity: t.bend.clamp(-1.0, 1.0), ..base });
            snap.notes.push(base);
            snap.notes.push(ScheduledNote { at: off, on: false, ..base });
        }
    }
    snap.notes.sort_by_key(|n| (n.at, n.on));
    snap.sections = vec![Section { movement: section, start: 0, end: to_sample(length), sample_rate, tempo, meters: vec![], spans: vec![] }];
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

/// Whether a track plays the built-in instruments (no plugin, a bypassed
/// one, or Corinzia Ensemble itself).
pub fn plays_builtin(project: &Project, track: Id) -> bool {
    let Some(t) = project.track(track) else { return false };
    if t.audio {
        return false;
    }
    match t.instrument_node.and_then(|n| project.node(n)) {
        None => true,
        Some(n) => n.bypassed || n.bundle.is_empty() || crate::host::is_builtin(&n.bundle) && (n.plugin_id == corinzia_ensemble::PLUGIN_ID.to_str().unwrap_or_default() || n.plugin_id == corinzia_ensemble::OLD_PLUGIN_ID.to_str().unwrap_or_default()),
    }
}

/// Whether the score is for orchestra (its strings play as sections): it
/// has a double bass, or five string staves or more.
pub fn orchestral(project: &Project) -> bool {
    let names: Vec<String> = project.instruments.iter().map(|i| i.name.to_lowercase()).collect();
    let strings = names.iter().filter(|n| ["violin", "viola", "cello", "celli", "bass"].iter().any(|w| n.contains(w)) && !n.contains("guitar") && !n.contains("clarinet") && !n.contains("trombone")).count();
    names.iter().any(|n| n.contains("double bass") || n.contains("contrabass") || n.contains("basses")) || strings >= 5
}

/// The sound the built-in instruments give a track: its own program if it
/// has one, else the one its (first) instrument's name chooses.
pub fn builtin_program(project: &Project, track: Id) -> Option<(u8, u8, Option<u16>)> {
    let t = project.track(track)?;
    let ch = t.midi_channel & 15;
    if let Some(p) = t.program {
        return Some((ch, p, t.bank));
    }
    let inst = project.instruments.iter().find(|i| i.track == Some(track))?;
    let (program, bank) = corinzia_ensemble::patches::program_for(&inst.name, !inst.drums.is_empty(), orchestral(project));
    Some((ch, program, Some(bank)))
}

/// The articulation controller's value (CC9) for a note: its written
/// articulation, else a light, short stroke for a quick unslurred note;
/// its written bowing.
pub fn articulation_code(n: &corinzia_perform::PerformedNote, written_seconds: f64) -> u8 {
    use corinzia_ensemble::voice::articulation as a;
    use corinzia_score::score::art;
    let kind = if n.arts & art::STACCATISSIMO != 0 {
        a::STACCATISSIMO
    } else if n.arts & art::STACCATO != 0 {
        a::STACCATO
    } else if n.arts & art::MARCATO != 0 {
        a::MARCATO
    } else if n.arts & art::TENUTO != 0 {
        a::TENUTO
    } else if !n.slurred && written_seconds < 0.2 {
        a::SPICCATO
    } else {
        a::NORMAL
    };
    kind | if n.arts & art::DOWN_BOW != 0 { a::DOWN_BOW } else { 0 } | if n.arts & art::UP_BOW != 0 { a::UP_BOW } else { 0 }
}

/// The technique controller's value (CC3) for a technique.
pub fn technique_value(t: Option<&str>) -> u8 {
    use corinzia_ensemble::voice::tech;
    match t {
        Some("pizz.") => tech::PIZZ,
        Some("trem.") => tech::TREM,
        Some("con sord.") => tech::CON_SORD,
        Some("sul pont.") => tech::SUL_PONT,
        Some("sul tasto") => tech::SUL_TASTO,
        Some("col legno") => tech::COL_LEGNO,
        Some("harmonics") => tech::HARMONICS,
        Some("flutter") => tech::FLUTTER,
        _ => tech::ORD,
    }
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
            let program = if plays_builtin(project, track) { builtin_program(project, track) } else { t.and_then(|t| Some((t.midi_channel & 15, t.program?, t.bank))) };
            // An instance an earlier track plays already: this one is a guest.
            let host = plugin.as_ref().and_then(|p| nodes.iter().position(|n| n.host.is_none() && n.plugin.as_ref().is_some_and(|q| Arc::ptr_eq(p, q)))).map(|i| i as u32);
            let plugin = if host.is_some() { None } else { plugin };
            let channel = t.map(|t| t.midi_channel & 15).unwrap_or(0);
            nodes.push(Node { track, plugin, clips: vec![], gain, pan, audible, direct, host, channel, inserts, out, sends, program });
        }
    }
    let master_inserts = effects(&project.master.inserts, get);
    let master_gain = db_to_gain(project.master.gain_db);
    // The node a track's own strip is, and the node its notes go to (a
    // guest's host).
    let strip_of = |track: Id| nodes.iter().position(|n| n.track == track).map(|i| i as u32);
    let node_of = |track: Id| strip_of(track).map(|i| nodes[i as usize].host.unwrap_or(i));
    let mut sections = Vec::new();
    let mut notes = Vec::new();
    let mut start = 0i64;
    let mut note_id = 0i32;
    for &mi in movements {
        let Some(m) = project.movements.get(mi) else { continue };
        let perf = corinzia_perform::perform_with(m, corinzia_perform::Style::of(project));
        let to_sample = |q: f64| start + (perf.tempo.seconds_at(q) * sample_rate).round() as i64;
        // Expression maps: per track, its notes in time order.
        let mut by_track: std::collections::HashMap<Id, Vec<usize>> = Default::default();
        for (i, n) in perf.notes.iter().enumerate() {
            if let Some(t) = project.instrument(n.instrument).and_then(|i| i.track) {
                by_track.entry(t).or_default().push(i);
            }
        }
        let mut channels: Vec<Option<u8>> = vec![None; perf.notes.len()];
        let builtin: Vec<Id> = nodes.iter().filter(|n| n.plugin.is_some() || n.host.is_some()).map(|n| n.track).filter(|&t| plays_builtin(project, t)).collect();
        for (track, mut idx) in by_track {
            let (Some(t), Some(node)) = (project.track(track), node_of(track)) else { continue };
            if t.techniques.is_empty() {
                continue;
            }
            idx.sort_by(|a, b| perf.notes[*a].on_q.total_cmp(&perf.notes[*b].on_q));
            let list: Vec<&corinzia_perform::PerformedNote> = idx.iter().map(|&i| &perf.notes[i]).collect();
            for (k, e) in corinzia_perform::expression(t, &list).into_iter().enumerate() {
                channels[idx[k]] = e.channel;
                let on = start + (perf.notes[idx[k]].on_sec(&perf.tempo) * sample_rate).round() as i64;
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
        // A tremolo on the built-in instruments is one held note in the
        // tremolo technique (the bow's quick strokes), not the written-out
        // repeated notes other instruments get: its first note lasts to the
        // last one's end; the others are left out.
        let mut merged: std::collections::HashMap<usize, usize> = Default::default(); // first -> last
        let mut skip: std::collections::HashSet<usize> = Default::default();
        {
            let mut open: std::collections::HashMap<(Id, i32), usize> = Default::default();
            for (ni, n) in perf.notes.iter().enumerate() {
                if n.arts & corinzia_score::score::art::TREMOLO == 0 || !project.instrument(n.instrument).and_then(|i| i.track).is_some_and(|t| builtin.contains(&t)) {
                    continue;
                }
                match open.get(&(n.element, n.key)).copied() {
                    Some(first) if (perf.notes[merged.get(&first).copied().unwrap_or(first)].full_off_q - n.on_q).abs() < 1e-6 => {
                        merged.insert(first, ni);
                        skip.insert(ni);
                    }
                    _ => {
                        open.insert((n.element, n.key), ni);
                    }
                }
            }
        }
        // (A note and where it ends: a merged tremolo's last note's end.)
        let end_of = |ni: usize| -> &corinzia_perform::PerformedNote { &perf.notes[merged.get(&ni).copied().unwrap_or(ni)] };
        for (ni, n) in perf.notes.iter().enumerate() {
            if skip.contains(&ni) {
                continue;
            }
            let Some(track) = project.instrument(n.instrument).and_then(|i| i.track) else { continue };
            let Some(node) = node_of(track) else { continue };
            let channel = channels[ni].map(|c| c as i16).unwrap_or_else(|| project.track(track).map(|t| t.midi_channel as i16 & 15).unwrap_or(0));
            let port = project.track(track).map(|t| t.note_port as i16).unwrap_or(0);
            let on_sec = n.on_sec(&perf.tempo);
            let last = end_of(ni);
            let mut off_sec = if merged.contains_key(&ni) {
                let last_on = perf.tempo.seconds_at(last.on_q) + last.timing_sec;
                last_on + (perf.tempo.seconds_at(last.full_off_q) + last.timing_sec - last_on) * last.gate
            } else {
                n.off_sec(&perf.tempo)
            };
            // Slurred on the built-in instruments: held into the next note,
            // which joins it (CC68).
            if n.slurred && builtin.contains(&track) {
                off_sec += 0.012;
            }
            let on = start + (on_sec * sample_rate).round() as i64;
            let off = (start + (off_sec * sample_rate).round() as i64).max(on + 1);
            let key = project.sounding_key(n.instrument, n.key).clamp(0, 127) as i16;
            let base = ScheduledNote { kind: Kind::Note, at: on, node, note_id, port, channel, key, on: true, velocity: n.velocity, element: n.element };
            notes.push(base);
            notes.push(ScheduledNote { at: off, on: false, ..base });
            note_id = note_id.wrapping_add(1) & 0x7fff_ffff;
        }
        // The built-in instruments: the dynamic as CC1 (following hairpins
        // through held notes), the technique as CC3, slurs as CC68.
        for &track in &builtin {
            let Some(node) = node_of(track) else { continue };
            let Some(t) = project.track(track) else { continue };
            let channel = t.midi_channel as i16 & 15;
            let mut list: Vec<(usize, &corinzia_perform::PerformedNote)> = perf.notes.iter().enumerate().filter(|(i, n)| !skip.contains(i) && project.instrument(n.instrument).and_then(|i| i.track) == Some(track)).collect();
            list.sort_by(|a, b| a.1.on_q.total_cmp(&b.1.on_q).then(b.1.key.cmp(&a.1.key)));
            let at = |sec: f64| start + (sec * sample_rate).round() as i64;
            let cc = |notes: &mut Vec<ScheduledNote>, at: i64, cc: u8, v: f32| notes.push(ScheduledNote { kind: Kind::Cc(cc), at: at.max(0), node, note_id: -1, port: 0, channel, key: 0, on: false, velocity: v.clamp(0.0, 1.0), element: 0 });
            let (mut technique, mut legato, mut last_on, mut articulation) = (None::<u8>, false, f64::NAN, None::<u8>);
            let mut level: Option<f32> = None;
            for (ni, n) in list {
                let tremolo = merged.contains_key(&ni);
                let last = end_of(ni);
                let on_sec = n.on_sec(&perf.tempo);
                let on = at(on_sec);
                let chord = (n.on_q - last_on).abs() < 1e-9;
                last_on = n.on_q;
                // The dynamic is what the note settles to; an accent or an
                // sfz is in the velocity, the attack.
                let (d0, d1) = (n.sustain, last.end_velocity);
                if !chord {
                    let code = articulation_code(n, perf.tempo.seconds_at(last.full_off_q) - perf.tempo.seconds_at(n.on_q));
                    if articulation != Some(code) {
                        articulation = Some(code);
                        cc(&mut notes, on - 3, 9, code as f32 / 127.0);
                    }
                    if t.techniques.is_empty() {
                        let tv = if tremolo { corinzia_ensemble::voice::tech::TREM } else { technique_value(n.technique) };
                        if technique != Some(tv) {
                            technique = Some(tv);
                            cc(&mut notes, on - 4, 3, tv as f32 / 127.0);
                        }
                    }
                    if level.is_none_or(|l| (l - d0).abs() > 0.004) {
                        cc(&mut notes, on - 2, 1, d0);
                    }
                    level = Some(d0);
                    if (d1 - d0).abs() > 0.004 {
                        // A ramp through the note, every 25 ms.
                        let off_sec = last.off_sec(&perf.tempo);
                        let steps = (((off_sec - on_sec) / 0.025).ceil() as i64).clamp(1, 400);
                        for k in 1..=steps {
                            let x = k as f32 / steps as f32;
                            cc(&mut notes, at(on_sec + (off_sec - on_sec) * x as f64) - 1, 1, d0 + (d1 - d0) * x);
                        }
                        level = Some(d1);
                    }
                }
                if n.slurred != legato {
                    legato = n.slurred;
                    cc(&mut notes, on + 1, 68, if legato { 1.0 } else { 0.0 });
                }
            }
            if legato {
                cc(&mut notes, to_sample(perf.length_q), 68, 0.0);
            }
        }
        // Dynamics as a controller, where a track asks for it: at each onset.
        // By track: tracks sharing an instance each have their channel.
        let mut last_cc: std::collections::HashMap<Id, u8> = Default::default();
        for n in &perf.notes {
            let Some(t) = project.instrument(n.instrument).and_then(|i| i.track).and_then(|t| project.track(t)) else { continue };
            let (Some(cc), Some(node)) = (t.dynamics_cc, node_of(t.id)) else { continue };
            if builtin.contains(&t.id) && cc == 1 {
                continue; // the curve above has it
            }
            let value = (n.velocity.clamp(0.0, 1.0) * 127.0).round() as u8;
            if last_cc.insert(t.id, value) == Some(value) {
                continue;
            }
            let at = start + ((n.on_sec(&perf.tempo) * sample_rate).round() as i64 - 1).max(0);
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
                AutoTarget::Gain(t) if strip_of(t).is_none() => (bus_index(t), Kind::BusGain, |db| db_to_gain(db)),
                AutoTarget::Pan(t) if strip_of(t).is_none() => (bus_index(t), Kind::BusPan, |p| p.clamp(-1.0, 1.0) as f32),
                AutoTarget::Gain(t) => (strip_of(t), Kind::Gain, |db| db_to_gain(db)),
                AutoTarget::Pan(t) => (strip_of(t), Kind::Pan, |p| p.clamp(-1.0, 1.0) as f32),
                AutoTarget::Send { track, bus } => match project.track(track).and_then(|t| t.sends.iter().position(|s| s.bus == bus)) {
                    Some(i) => (strip_of(track), Kind::Send { index: i as u32 }, |db| db_to_gain(db)),
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
            node.clips.push(ScheduledClip { start: sec.start + frames(c.at) as i64, offset, len, fade_in, fade_out, in_shape: c.fade_in_shape, out_shape: c.fade_out_shape, gain: db_to_gain(c.gain_db), data });
        }
        node.clips.sort_by_key(|c| c.start);
        crossfade(&mut node.clips);
    }
    // Offs before ons at the same sample: a repeated key restarts cleanly.
    notes.sort_by_key(|n| (n.at, n.on));
    let effects = fx_nodes.into_iter().map(|x| x.1).collect();
    Snapshot { generation, sample_rate, sections, notes, nodes, buses, master_inserts, master_gain, effects, clip_errors }
}

#[cfg(test)]
mod crossfade_tests {
    use super::*;
    use corinzia_model::FadeShape;

    fn clip(start: i64, len: usize) -> ScheduledClip {
        let data = Arc::new(AudioData::from_channels(vec![vec![1.0; len]], 1000.0));
        ScheduledClip { start, offset: 0, len, fade_in: 0, fade_out: 0, in_shape: FadeShape::Linear, out_shape: FadeShape::Linear, gain: 1.0, data }
    }

    #[test]
    fn overlapping_clips_crossfade_at_equal_power() {
        let mut c = vec![clip(0, 1000), clip(800, 1000)];
        crossfade(&mut c);
        assert_eq!((c[0].fade_out, c[1].fade_in), (200, 200));
        // Through the overlap the two together keep their power.
        for t in (800..1000).step_by(10) {
            let (a, b) = (c[0].gain_at(t), c[1].gain_at((t - 800) as usize));
            assert!((a * a + b * b - 1.0).abs() < 0.02, "{t}: {a} {b}");
        }
        // Not touching: no fades; a clip inside another: laid over.
        let mut d = vec![clip(0, 100), clip(100, 100), clip(150, 10)];
        crossfade(&mut d);
        assert_eq!((d[0].fade_out, d[1].fade_in, d[2].fade_in), (0, 0, 0));
    }

    #[test]
    fn fades_follow_their_shapes() {
        for s in FadeShape::ALL {
            assert_eq!((s.gain(0.0), s.gain(1.0)), (0.0, 1.0), "{s:?}");
            assert_eq!(FadeShape::from_code(s.code()), s);
        }
        assert!(FadeShape::Fast.gain(0.5) > FadeShape::Linear.gain(0.5) && FadeShape::Slow.gain(0.5) < FadeShape::Linear.gain(0.5));
        assert!((FadeShape::SCurve.gain(0.5) - 0.5).abs() < 1e-6 && FadeShape::SCurve.gain(0.1) < 0.1);
        let mut c = clip(0, 100);
        (c.fade_in, c.in_shape) = (50, FadeShape::Slow);
        assert!((c.gain_at(25) - 0.25).abs() < 1e-6);
    }
}
