//! The realtime render core. A backend (PipeWire, or the offline renderer)
//! owns the audio thread and calls [`Engine::process`] for every block.
//!
//! Realtime contract of `process`: no allocation, no locks, no blocking. The
//! UI talks to it only through wait-free SPSC queues ([`EngineHandle`]):
//! commands in (new snapshot, transport, mixer, preview notes), retired
//! snapshots out (dropped on the UI thread, which is where plugins may be
//! deactivated and destroyed), and atomics for the playhead.

use crate::snapshot::{ScheduledNote, Snapshot};
use clap_sys::events::*;
use clap_sys::fixedpoint::{CLAP_BEATTIME_FACTOR, CLAP_SECTIME_FACTOR};
use clap_sys::audio_buffer::clap_audio_buffer;
use clap_sys::process::clap_process;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::Arc;

pub const MAX_NODES: usize = 256;
pub const MAX_ACTIVE: usize = 4096;
const EVENTS_PER_NODE: usize = 1024;

pub enum Command {
    Swap(Box<Snapshot>),
    Play,
    Stop,
    Seek(i64),
    /// Live mixer change for a node: gain (linear), pan, audible.
    Mix { node: u32, gain: f32, pan: f32, audible: bool },
    /// Audition: velocity in 0..1; a negative velocity ends the note.
    Preview { node: u32, key: i16, channel: i16, velocity: f32 },
}

pub enum Retired {
    Snapshot(Box<Snapshot>),
}

/// What the UI reads every frame.
#[derive(Default)]
pub struct Shared {
    pub position: AtomicI64, // timeline sample of the next block
    pub playing: AtomicBool,
    pub generation: AtomicU64,
    pub dropped: AtomicU64, // events lost to full queues (diagnostics)
    pub blocks: AtomicU64,
}

/// The UI side.
pub struct EngineHandle {
    commands: rtrb::Producer<Command>,
    retired: rtrb::Consumer<Retired>,
    pub shared: Arc<Shared>,
    pub sample_rate: f64,
    pub max_frames: u32,
}

impl EngineHandle {
    /// Queues a command; gives it back if the queue is full.
    pub fn send(&mut self, c: Command) -> Result<(), Command> {
        self.commands.push(c).map_err(|rtrb::PushError::Full(c)| c)
    }
    pub fn play(&mut self) {
        let _ = self.send(Command::Play);
    }
    pub fn stop(&mut self) {
        let _ = self.send(Command::Stop);
    }
    pub fn seek(&mut self, sample: i64) {
        let _ = self.send(Command::Seek(sample));
    }
    pub fn swap(&mut self, s: Snapshot) {
        if let Err(Command::Swap(s)) = self.send(Command::Swap(Box::new(s))) {
            drop(s); // queue full: the audio thread is stalled; keep the old plan
        }
    }
    /// Drops retired snapshots (and with them, plugins no longer used).
    pub fn collect(&mut self) {
        while let Ok(r) = self.retired.pop() {
            match r {
                Retired::Snapshot(s) => drop(s),
            }
        }
    }
    pub fn position(&self) -> i64 {
        self.shared.position.load(Ordering::Acquire)
    }
    pub fn is_playing(&self) -> bool {
        self.shared.playing.load(Ordering::Acquire)
    }
}

/// A CLAP event slot: notes and MIDI share the list.
#[repr(C)]
#[derive(Clone, Copy)]
union EventSlot {
    header: clap_event_header,
    note: clap_event_note,
    midi: clap_event_midi,
}

struct InputEvents {
    list: clap_input_events,
    events: Vec<EventSlot>,
}

unsafe extern "C" fn in_size(list: *const clap_input_events) -> u32 {
    (*((*list).ctx as *const InputEvents)).events.len() as u32
}
unsafe extern "C" fn in_get(list: *const clap_input_events, i: u32) -> *const clap_event_header {
    let ev = &*((*list).ctx as *const InputEvents);
    ev.events.get(i as usize).map(|e| &e.header as *const _).unwrap_or(std::ptr::null())
}
unsafe extern "C" fn out_push(_list: *const clap_output_events, _e: *const clap_event_header) -> bool {
    true // note-end and parameter gestures are not used yet
}

struct NodeState {
    events: Box<InputEvents>,
    left: Vec<f32>,
    right: Vec<f32>,
    ptrs: [*mut f32; 2],
    gain: f32,
    pan: f32,
    audible: bool,
    midi: bool, // the plugin takes MIDI rather than CLAP notes
}

#[derive(Clone, Copy, Default)]
struct Active {
    node: u32,
    note_id: i32,
    port: i16,
    channel: i16,
    key: i16,
}

/// The audio side.
pub struct Engine {
    commands: rtrb::Consumer<Command>,
    retired: rtrb::Producer<Retired>,
    shared: Arc<Shared>,
    snap: Option<Box<Snapshot>>,
    nodes: Vec<NodeState>,
    active: Vec<Active>,
    sample_rate: f64,
    max_frames: u32,
    pos: i64,
    cursor: usize,
    playing: bool,
    steady: i64,
    out_events: clap_output_events,
}
unsafe impl Send for Engine {}

pub fn new(sample_rate: f64, max_frames: u32) -> (EngineHandle, Engine) {
    let (ctx, crx) = rtrb::RingBuffer::new(1024);
    let (rtx, rrx) = rtrb::RingBuffer::new(64);
    let shared = Arc::new(Shared::default());
    let nodes = (0..MAX_NODES)
        .map(|_| {
            let mut events = Box::new(InputEvents {
                list: clap_input_events { ctx: std::ptr::null_mut(), size: Some(in_size), get: Some(in_get) },
                events: Vec::with_capacity(EVENTS_PER_NODE),
            });
            events.list.ctx = &mut *events as *mut InputEvents as *mut c_void;
            NodeState { events, left: vec![0.0; max_frames as usize], right: vec![0.0; max_frames as usize], ptrs: [std::ptr::null_mut(); 2], gain: 1.0, pan: 0.0, audible: true, midi: false }
        })
        .collect();
    let engine = Engine {
        commands: crx,
        retired: rtx,
        shared: shared.clone(),
        snap: None,
        nodes,
        active: Vec::with_capacity(MAX_ACTIVE),
        sample_rate,
        max_frames,
        pos: 0,
        cursor: 0,
        playing: false,
        steady: 0,
        out_events: clap_output_events { ctx: std::ptr::null_mut(), try_push: Some(out_push) },
    };
    (EngineHandle { commands: ctx, retired: rrx, shared, sample_rate, max_frames }, engine)
}

fn note_event(n: &Active, on: bool, velocity: f32, time: u32, midi: bool) -> EventSlot {
    if midi {
        let status = if on { 0x90 } else { 0x80 } | (n.channel as u8 & 15);
        EventSlot {
            midi: clap_event_midi {
                header: clap_event_header { size: std::mem::size_of::<clap_event_midi>() as u32, time, space_id: CLAP_CORE_EVENT_SPACE_ID, type_: CLAP_EVENT_MIDI, flags: 0 },
                port_index: n.port as u16,
                data: [status, n.key as u8 & 127, if on { ((velocity * 127.0).round() as u8).clamp(1, 127) } else { 64 }],
            },
        }
    } else {
        EventSlot {
            note: clap_event_note {
                header: clap_event_header {
                    size: std::mem::size_of::<clap_event_note>() as u32,
                    time,
                    space_id: CLAP_CORE_EVENT_SPACE_ID,
                    type_: if on { CLAP_EVENT_NOTE_ON } else { CLAP_EVENT_NOTE_OFF },
                    flags: 0,
                },
                note_id: n.note_id,
                port_index: n.port,
                channel: n.channel,
                key: n.key,
                velocity: velocity as f64,
            },
        }
    }
}

impl Engine {
    fn push_event(&mut self, node: u32, a: Active, on: bool, velocity: f32, time: u32) {
        let Some(ns) = self.nodes.get_mut(node as usize) else { return };
        let midi = ns.midi;
        if ns.events.events.len() < EVENTS_PER_NODE {
            ns.events.events.push(note_event(&a, on, velocity, time, midi));
        } else {
            self.shared.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn release_all(&mut self, time: u32) {
        let active = std::mem::take(&mut self.active);
        for a in &active {
            self.push_event(a.node, *a, false, 0.0, time);
        }
        self.active = active;
        self.active.clear();
    }

    fn reset_cursor(&mut self) {
        self.cursor = self.snap.as_ref().map(|s| s.first_note_at_or_after(self.pos)).unwrap_or(0);
    }

    fn handle_commands(&mut self) {
        while let Ok(c) = self.commands.pop() {
            match c {
                Command::Swap(next) => {
                    // Notes of the old plan end now; plugins that leave stop processing here.
                    self.release_all(0);
                    self.flush_releases_to_old_plugins();
                    if let Some(old) = self.snap.take() {
                        for n in &old.nodes {
                            if !next.nodes.iter().any(|m| Arc::ptr_eq(&m.plugin, &n.plugin)) {
                                n.plugin.rt_stop();
                            }
                        }
                        if let Err(rtrb::PushError::Full(Retired::Snapshot(s))) = self.retired.push(Retired::Snapshot(old)) {
                            std::mem::forget(s); // never free on the audio thread
                        }
                    }
                    for (i, n) in next.nodes.iter().enumerate().take(MAX_NODES) {
                        self.nodes[i].gain = n.gain;
                        self.nodes[i].pan = n.pan;
                        self.nodes[i].audible = n.audible;
                        self.nodes[i].midi = n.plugin.midi_dialect;
                    }
                    self.shared.generation.store(next.generation, Ordering::Release);
                    self.snap = Some(next);
                    self.reset_cursor();
                }
                Command::Play => {
                    self.playing = true;
                    self.reset_cursor();
                }
                Command::Stop => {
                    self.playing = false;
                    self.release_all(0);
                }
                Command::Seek(s) => {
                    self.release_all(0);
                    self.pos = s.max(0);
                    self.reset_cursor();
                }
                Command::Mix { node, gain, pan, audible } => {
                    if let Some(n) = self.nodes.get_mut(node as usize) {
                        n.gain = gain;
                        n.pan = pan;
                        n.audible = audible;
                    }
                }
                Command::Preview { node, key, channel, velocity } => {
                    let a = Active { node, note_id: -1, port: 0, channel, key };
                    self.push_event(node, a, velocity >= 0.0, velocity.max(0.0), 0);
                }
            }
        }
    }

    /// Note-offs queued for plugins that are about to leave with the old
    /// plan: deliver them with an empty render so voices end cleanly.
    fn flush_releases_to_old_plugins(&mut self) {
        let Some(snap) = self.snap.as_ref() else { return };
        for (i, n) in snap.nodes.iter().enumerate().take(MAX_NODES) {
            if self.nodes[i].events.events.is_empty() || !n.plugin.rt_start() {
                continue;
            }
            let ns = &mut self.nodes[i];
            let frames = 1u32;
            ns.ptrs = [ns.left.as_mut_ptr(), ns.right.as_mut_ptr()];
            let mut out = clap_audio_buffer { data32: ns.ptrs.as_mut_ptr(), data64: std::ptr::null_mut(), channel_count: 2, latency: 0, constant_mask: 0 };
            let p = clap_process {
                steady_time: -1,
                frames_count: frames,
                transport: std::ptr::null(),
                audio_inputs: std::ptr::null(),
                audio_outputs: &mut out,
                audio_inputs_count: 0,
                audio_outputs_count: 1,
                in_events: &ns.events.list,
                out_events: &self.out_events,
            };
            unsafe { n.plugin.rt_process(&p) };
            ns.events.events.clear();
        }
    }

    /// Fills `left`/`right` (same length, at most `max_frames`).
    pub fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        let frames = left.len().min(right.len()).min(self.max_frames as usize);
        left.fill(0.0);
        right.fill(0.0);
        for n in self.nodes.iter_mut() {
            n.events.events.clear();
        }
        self.handle_commands();
        let block_start = self.pos;
        let block_end = self.pos + frames as i64;
        // Schedule the plan's notes inside this block, sample-accurately.
        if self.playing {
            if let Some(snap) = self.snap.take() {
                while let Some(n) = snap.notes.get(self.cursor) {
                    if n.at >= block_end {
                        break;
                    }
                    self.cursor += 1;
                    if n.at < block_start {
                        continue;
                    }
                    self.schedule(n, (n.at - block_start) as u32);
                }
                let end = snap.end();
                self.snap = Some(snap);
                if block_end >= end && self.active.is_empty() {
                    // The end: the playhead rests on it.
                    self.playing = false;
                    self.pos = end.max(block_start);
                }
            }
        }
        self.render(left, right, frames, block_start);
        if self.playing {
            self.pos = block_end;
        }
        self.steady += frames as i64;
        self.shared.position.store(self.pos, Ordering::Release);
        self.shared.playing.store(self.playing, Ordering::Release);
        self.shared.blocks.fetch_add(1, Ordering::Relaxed);
    }

    fn schedule(&mut self, n: &ScheduledNote, time: u32) {
        let a = Active { node: n.node, note_id: n.note_id, port: n.port, channel: n.channel, key: n.key };
        if n.on {
            if self.active.len() < MAX_ACTIVE {
                self.active.push(a);
            }
        } else if let Some(i) = self.active.iter().position(|x| x.note_id == n.note_id && x.node == n.node) {
            self.active.swap_remove(i);
        }
        self.push_event(n.node, a, n.on, n.velocity, time);
    }

    fn transport(&self, at: i64) -> clap_event_transport {
        let mut t = clap_event_transport {
            header: clap_event_header { size: std::mem::size_of::<clap_event_transport>() as u32, time: 0, space_id: CLAP_CORE_EVENT_SPACE_ID, type_: CLAP_EVENT_TRANSPORT, flags: 0 },
            flags: CLAP_TRANSPORT_HAS_TEMPO | CLAP_TRANSPORT_HAS_BEATS_TIMELINE | CLAP_TRANSPORT_HAS_SECONDS_TIMELINE | CLAP_TRANSPORT_HAS_TIME_SIGNATURE,
            song_pos_beats: 0,
            song_pos_seconds: 0,
            tempo: 120.0,
            tempo_inc: 0.0,
            loop_start_beats: 0,
            loop_end_beats: 0,
            loop_start_seconds: 0,
            loop_end_seconds: 0,
            bar_start: 0,
            bar_number: 0,
            tsig_num: 4,
            tsig_denom: 4,
        };
        if self.playing {
            t.flags |= CLAP_TRANSPORT_IS_PLAYING;
        }
        if let Some(sec) = self.snap.as_ref().and_then(|s| s.section_at(at)) {
            let q = sec.performed_quarter(at.max(sec.start));
            let bar = sec.tempo.bar_at(q);
            t.song_pos_beats = (q * CLAP_BEATTIME_FACTOR as f64) as i64;
            t.song_pos_seconds = ((at - sec.start) as f64 / self.sample_rate * CLAP_SECTIME_FACTOR as f64) as i64;
            t.tempo = sec.tempo.tempo_at(q);
            t.bar_start = (bar.start_quarter * CLAP_BEATTIME_FACTOR as f64) as i64;
            t.bar_number = (bar.bar - 1) as i32;
            t.tsig_num = bar.num as u16;
            t.tsig_denom = bar.den as u16;
        }
        t
    }

    fn render(&mut self, left: &mut [f32], right: &mut [f32], frames: usize, at: i64) {
        let Some(snap) = self.snap.take() else { return };
        let transport = self.transport(at);
        for (i, node) in snap.nodes.iter().enumerate().take(MAX_NODES) {
            if !node.plugin.rt_start() {
                continue;
            }
            let ns = &mut self.nodes[i];
            ns.ptrs = [ns.left.as_mut_ptr(), ns.right.as_mut_ptr()];
            let mut out = clap_audio_buffer { data32: ns.ptrs.as_mut_ptr(), data64: std::ptr::null_mut(), channel_count: 2, latency: 0, constant_mask: 0 };
            let p = clap_process {
                steady_time: self.steady,
                frames_count: frames as u32,
                transport: &transport,
                audio_inputs: std::ptr::null(),
                audio_outputs: &mut out,
                audio_inputs_count: 0,
                audio_outputs_count: 1,
                in_events: &ns.events.list,
                out_events: &self.out_events,
            };
            unsafe { node.plugin.rt_process(&p) };
            if !ns.audible {
                continue;
            }
            // Equal-power pan.
            let angle = (ns.pan + 1.0) * std::f32::consts::FRAC_PI_4;
            let (gl, gr) = (ns.gain * angle.cos() * std::f32::consts::SQRT_2, ns.gain * angle.sin() * std::f32::consts::SQRT_2);
            for f in 0..frames {
                left[f] += ns.left[f] * gl;
                right[f] += ns.right[f] * gr;
            }
        }
        self.snap = Some(snap);
    }

    /// Offline rendering (tests, audio export): `seconds` of output.
    pub fn render_offline(&mut self, frames: usize) -> (Vec<f32>, Vec<f32>) {
        let (mut l, mut r) = (vec![0.0; frames], vec![0.0; frames]);
        let block = self.max_frames as usize;
        let mut at = 0;
        while at < frames {
            let n = block.min(frames - at);
            self.process(&mut l[at..at + n], &mut r[at..at + n]);
            at += n;
        }
        (l, r)
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        // The audio thread is gone: stop the plugins it was processing.
        if let Some(s) = self.snap.as_ref() {
            for n in &s.nodes {
                n.plugin.rt_stop();
            }
        }
    }
}
