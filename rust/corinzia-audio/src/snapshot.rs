//! The render plan the audio thread plays: built on the main thread from the
//! project (through the performance compiler), immutable afterwards, handed
//! to the audio thread whole and handed back for dropping. Everything is in
//! absolute samples: movements are laid end to end, notes are resolved to
//! the node (track) that sounds them.

use crate::host::Plugin;
use corinzia_model::{Id, Project};
use corinzia_perform::tempo::{MeterPoint, TempoMap};
use corinzia_perform::Span;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScheduledNote {
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

/// A track's sound: its plugin and mixer settings.
pub struct Node {
    pub track: Id,
    pub plugin: Arc<Plugin>,
    pub gain: f32, // linear
    pub pan: f32,  // -1..1
    pub audible: bool,
}

pub struct Snapshot {
    pub generation: u64,
    pub sample_rate: f64,
    pub sections: Vec<Section>,
    pub notes: Vec<ScheduledNote>, // sorted by (at, offs first)
    pub nodes: Vec<Node>,
}

impl Snapshot {
    pub fn end(&self) -> i64 {
        self.sections.last().map(|s| s.end).unwrap_or(0)
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

pub fn mix(project: &Project) -> Vec<(Id, f32, f32, bool)> {
    let any_solo = project.tracks.iter().any(|t| t.solo);
    project
        .tracks
        .iter()
        // The fader's bottom is silence (−∞ dB).
        .map(|t| (t.id, if t.gain_db <= MIN_DB { 0.0 } else { 10f32.powf(t.gain_db as f32 / 20.0) }, t.pan.clamp(-1.0, 1.0) as f32, !t.mute && (!any_solo || t.solo)))
        .collect()
}

/// Builds the plan. `plugins(track)` gives the activated plugin of a track.
pub fn build(project: &Project, sample_rate: f64, generation: u64, movements: &[usize], mut plugins: impl FnMut(Id) -> Option<Arc<Plugin>>) -> Snapshot {
    const GAP_SECONDS: f64 = 2.0;
    let mut nodes: Vec<Node> = Vec::new();
    for (track, gain, pan, audible) in mix(project) {
        if let Some(plugin) = plugins(track) {
            nodes.push(Node { track, plugin, gain, pan, audible });
        }
    }
    let node_of = |track: Id| nodes.iter().position(|n| n.track == track).map(|i| i as u32);
    let mut sections = Vec::new();
    let mut notes = Vec::new();
    let mut start = 0i64;
    let mut note_id = 0i32;
    for &mi in movements {
        let Some(m) = project.movements.get(mi) else { continue };
        let perf = corinzia_perform::perform(m);
        let to_sample = |q: f64| start + (perf.tempo.seconds_at(q) * sample_rate).round() as i64;
        for n in &perf.notes {
            let Some(track) = project.instrument(n.instrument).and_then(|i| i.track) else { continue };
            let Some(node) = node_of(track) else { continue };
            let channel = project.track(track).map(|t| t.midi_channel as i16 & 15).unwrap_or(0);
            let port = project.track(track).map(|t| t.note_port as i16).unwrap_or(0);
            let on_sec = perf.tempo.seconds_at(n.on_q) + n.roll_sec;
            let off_sec = on_sec + (perf.tempo.seconds_at(n.full_off_q) - on_sec) * n.gate;
            let on = start + (on_sec * sample_rate).round() as i64;
            let off = (start + (off_sec * sample_rate).round() as i64).max(on + 1);
            let key = project.sounding_key(n.instrument, n.key).clamp(0, 127) as i16;
            let base = ScheduledNote { at: on, node, note_id, port, channel, key, on: true, velocity: n.velocity, element: n.element };
            notes.push(base);
            notes.push(ScheduledNote { at: off, on: false, ..base });
            note_id = note_id.wrapping_add(1) & 0x7fff_ffff;
        }
        let end = to_sample(perf.length_q);
        sections.push(Section { movement: m.id, start, end, sample_rate, tempo: perf.tempo.clone(), meters: perf.meter_points.clone(), spans: perf.spans.clone() });
        start = end + (GAP_SECONDS * sample_rate) as i64;
    }
    // Offs before ons at the same sample: a repeated key restarts cleanly.
    notes.sort_by_key(|n| (n.at, n.on));
    Snapshot { generation, sample_rate, sections, notes, nodes }
}
