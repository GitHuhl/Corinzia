//! The realtime render core. A backend (PipeWire, or the offline renderer)
//! owns the audio thread and calls [`Engine::process`] for every block.
//!
//! Realtime contract of `process`: no allocation, no locks, no blocking. The
//! UI talks to it only through wait-free SPSC queues ([`EngineHandle`]):
//! commands in (new snapshot, transport, mixer, preview notes), retired
//! snapshots out (dropped on the UI thread, which is where plugins may be
//! deactivated and destroyed), and atomics for the playhead.

use crate::snapshot::{Kind, ScheduledNote, Snapshot};
use clap_sys::events::*;
use clap_sys::fixedpoint::{CLAP_BEATTIME_FACTOR, CLAP_SECTIME_FACTOR};
use clap_sys::audio_buffer::clap_audio_buffer;
use clap_sys::process::clap_process;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::Arc;
use crate::workers::Workers;

pub const MAX_NODES: usize = 256;
pub const MAX_BUSES: usize = 64;
pub const MAX_SENDS: usize = 8;
pub const MAX_ACTIVE: usize = 4096;
const EVENTS_PER_NODE: usize = 1024;

pub enum Command {
    Swap(Box<Snapshot>),
    Play,
    Stop,
    Seek(i64),
    /// Live mixer change for a node: gain (linear), pan, audible, and
    /// whether its own output is heard (see `snapshot::mix`).
    Mix { node: u32, gain: f32, pan: f32, audible: bool, direct: bool },
    /// Live change of a bus: gain (linear), pan, audible.
    MixBus { bus: u32, gain: f32, pan: f32, audible: bool },
    /// Live change of a node's send (by its index in the node's sends).
    Send { node: u32, index: u32, gain: f32 },
    /// Live change of the master fader (linear).
    Master { gain: f32 },
    /// The cycle: playback reaching its end goes back to its start
    /// (timeline samples, start before end); none: no cycle.
    Loop(Option<(i64, i64)>),
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
    param: clap_event_param_value,
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
    direct: bool,
    midi: bool, // the plugin takes MIDI rather than CLAP notes
    /// The left/right gains applied at the end of the last block: changes
    /// ramp from them across the next one (no zipper noise, no clicks).
    applied: Option<(f32, f32)>,
    /// The same for its own output (silent while only its sends are heard).
    main_applied: Option<(f32, f32)>,
    /// Send levels (linear), and the levels reached last block.
    sends: [f32; MAX_SENDS],
    send_applied: [Option<f32>; MAX_SENDS],
    /// Rendered this block (its plugin is processing).
    ran: bool,
}

/// What a render thread needs to run effects: a scratch pair for their
/// output and their event list. One per thread.
struct Scratch {
    tmp: [Vec<f32>; 2],
    fx_events: Box<InputEvents>,
}

fn input_events() -> Box<InputEvents> {
    let mut events = Box::new(InputEvents {
        list: clap_input_events { ctx: std::ptr::null_mut(), size: Some(in_size), get: Some(in_get) },
        events: Vec::with_capacity(EVENTS_PER_NODE),
    });
    events.list.ctx = &mut *events as *mut InputEvents as *mut c_void;
    events
}

/// One block's instrument jobs (see [`render_node`]); pointers into the
/// engine, valid while [`Workers::run`] runs.
struct NodeJobs {
    nodes: *mut NodeState,
    plan: *const crate::snapshot::Node,
    scratch: *mut Scratch,
    transport: *const clap_event_transport,
    out_events: *const clap_output_events,
    fx_auto: *const FxAuto,
    frames: usize,
    steady: i64,
    /// The block's first timeline sample, and whether the transport runs
    /// (clips sound only then).
    at: i64,
    playing: bool,
}

/// Renders one instrument and its inserts into the node's own buffers.
/// Jobs touch only their node and their thread's scratch, so they run
/// concurrently; mixing waits for all of them.
unsafe fn render_node(ctx: *const (), i: usize, worker: usize) {
    let j = &*(ctx as *const NodeJobs);
    let ns = &mut *j.nodes.add(i);
    let node = &*j.plan.add(i);
    let frames = j.frames;
    let Some(plugin) = &node.plugin else {
        // An audio track: its clips under the block.
        ns.ran = true;
        ns.left[..frames].fill(0.0);
        ns.right[..frames].fill(0.0);
        if j.playing {
            play_clips(&node.clips, j.at, &mut ns.left[..frames], &mut ns.right[..frames]);
        }
        run_inserts(node, ns, j, worker);
        return;
    };
    ns.ran = plugin.rt_start();
    if !ns.ran {
        return;
    }
    ns.ptrs = [ns.left.as_mut_ptr(), ns.right.as_mut_ptr()];
    let mut out = clap_audio_buffer { data32: ns.ptrs.as_mut_ptr(), data64: std::ptr::null_mut(), channel_count: 2, latency: 0, constant_mask: 0 };
    let p = clap_process {
        steady_time: j.steady,
        frames_count: frames as u32,
        transport: j.transport,
        audio_inputs: std::ptr::null(),
        audio_outputs: &mut out,
        audio_inputs_count: 0,
        audio_outputs_count: 1,
        in_events: &ns.events.list,
        out_events: j.out_events,
    };
    plugin.rt_process(&p);
    run_inserts(node, ns, j, worker);
}

/// The track's effects, in order, on its buffers.
unsafe fn run_inserts(node: &crate::snapshot::Node, ns: &mut NodeState, j: &NodeJobs, worker: usize) {
    let frames = j.frames;
    let sc = &mut *j.scratch.add(worker);
    for fx in &node.inserts {
        run_effect(fx, &mut ns.left[..frames], &mut ns.right[..frames], &mut sc.tmp, frames, j.steady, &*j.transport, &mut sc.fx_events, &*j.out_events, &*j.fx_auto);
    }
}

/// Adds the clips sounding in the block starting at timeline sample `at`
/// (mono clips to both sides).
fn play_clips(clips: &[crate::snapshot::ScheduledClip], at: i64, left: &mut [f32], right: &mut [f32]) {
    let end = at + left.len() as i64;
    for c in clips {
        if c.start >= end {
            break; // in time order
        }
        let (from, to) = (c.start.max(at), c.end().min(end));
        if from >= to {
            continue;
        }
        let l = &c.data.channels[0];
        let r = c.data.channels.get(1).unwrap_or(l);
        for t in from..to {
            let k = (t - c.start) as usize;
            let g = c.gain_at(k);
            let f = (t - at) as usize;
            left[f] += l[c.offset + k] * g;
            right[f] += r[c.offset + k] * g;
        }
    }
}

/// Effect automation for the block: the plan's effects (as pointers, to
/// match the one running) and (effect, parameter, value, frame) to deliver.
struct FxAuto {
    effects: Vec<*const crate::host::Plugin>,
    queue: Vec<(u32, u32, f64, u32)>,
}

/// A bus's mix buffer and fader.
struct BusState {
    left: Vec<f32>,
    right: Vec<f32>,
    gain: f32,
    pan: f32,
    audible: bool,
    applied: Option<(f32, f32)>,
}

/// Equal-power pan of a gain: (left, right); silent when not audible.
fn pan_gains(gain: f32, pan: f32, audible: bool) -> (f32, f32) {
    let angle = (pan + 1.0) * std::f32::consts::FRAC_PI_4;
    let g = if audible { gain } else { 0.0 };
    (g * angle.cos() * std::f32::consts::SQRT_2, g * angle.sin() * std::f32::consts::SQRT_2)
}

/// Adds `src` scaled by a gain ramping from `from` to `to` over the block.
fn mix_into(dst: &mut [f32], src: &[f32], from: f32, to: f32) {
    if from == 0.0 && to == 0.0 {
        return;
    }
    let n = dst.len().max(1) as f32;
    for (f, (d, s)) in dst.iter_mut().zip(src).enumerate() {
        *d += s * (from + (to - from) * (f + 1) as f32 / n);
    }
}

/// Runs an effect in place on `left`/`right` (through the scratch pair).
///
/// # Safety
/// Audio thread; the buffers hold at least `frames` samples.
#[allow(clippy::too_many_arguments)]
unsafe fn run_effect(plugin: &Arc<crate::host::Plugin>, left: &mut [f32], right: &mut [f32], tmp: &mut [Vec<f32>; 2], frames: usize, steady: i64, transport: &clap_event_transport, fx: &mut InputEvents, out_events: &clap_output_events, auto: &FxAuto) {
    if !plugin.rt_start() {
        return;
    }
    fx.events.clear();
    plugin.rt_take_params(|id, v| {
        if fx.events.len() < fx.events.capacity() {
            fx.events.push(EventSlot { param: crate::host::param_event(id, v, 0) });
        }
    });
    // Its automation in this block, in time order.
    if let Some(k) = auto.effects.iter().position(|e| std::ptr::eq(*e, Arc::as_ptr(plugin))) {
        for &(f, id, v, time) in &auto.queue {
            if f as usize == k && fx.events.len() < fx.events.capacity() {
                fx.events.push(EventSlot { param: crate::host::param_event(id, v, time) });
            }
        }
    }
    let events = &fx.list;
    let mut ins = [left.as_mut_ptr(), right.as_mut_ptr()];
    let mut outs = [tmp[0].as_mut_ptr(), tmp[1].as_mut_ptr()];
    let input = clap_audio_buffer { data32: ins.as_mut_ptr(), data64: std::ptr::null_mut(), channel_count: 2, latency: 0, constant_mask: 0 };
    let mut output = clap_audio_buffer { data32: outs.as_mut_ptr(), data64: std::ptr::null_mut(), channel_count: 2, latency: 0, constant_mask: 0 };
    let p = clap_process {
        steady_time: steady,
        frames_count: frames as u32,
        transport,
        audio_inputs: &input,
        audio_outputs: &mut output,
        audio_inputs_count: 1,
        audio_outputs_count: 1,
        in_events: events,
        out_events,
    };
    plugin.rt_process(&p);
    left[..frames].copy_from_slice(&tmp[0][..frames]);
    right[..frames].copy_from_slice(&tmp[1][..frames]);
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
    cycle: Option<(i64, i64)>,
    steady: i64,
    out_events: clap_output_events,
    buses: Vec<BusState>,
    master_gain: f32,
    master_applied: Option<f32>,
    /// Effect scratch per render thread (0: this one, for buses and master too).
    scratch: Vec<Scratch>,
    workers: Workers,
    fx_auto: FxAuto,
}
unsafe impl Send for Engine {}

/// The engine, rendering instruments on a worker per spare core.
pub fn new(sample_rate: f64, max_frames: u32) -> (EngineHandle, Engine) {
    with_workers(sample_rate, max_frames, Workers::for_this_machine())
}

pub fn with_workers(sample_rate: f64, max_frames: u32, workers: Workers) -> (EngineHandle, Engine) {
    let (ctx, crx) = rtrb::RingBuffer::new(1024);
    let (rtx, rrx) = rtrb::RingBuffer::new(64);
    let shared = Arc::new(Shared::default());
    let nodes = (0..MAX_NODES)
        .map(|_| {
            NodeState { events: input_events(), left: vec![0.0; max_frames as usize], right: vec![0.0; max_frames as usize], ptrs: [std::ptr::null_mut(); 2], gain: 1.0, pan: 0.0, audible: true, direct: true, midi: false, applied: None, main_applied: None, sends: [0.0; MAX_SENDS], send_applied: [None; MAX_SENDS], ran: false }
        })
        .collect();
    let buses = (0..MAX_BUSES).map(|_| BusState { left: vec![0.0; max_frames as usize], right: vec![0.0; max_frames as usize], gain: 1.0, pan: 0.0, audible: true, applied: None }).collect();
    let scratch = (0..workers.threads()).map(|_| Scratch { tmp: [vec![0.0; max_frames as usize], vec![0.0; max_frames as usize]], fx_events: input_events() }).collect();
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
        cycle: None,
        steady: 0,
        out_events: clap_output_events { ctx: std::ptr::null_mut(), try_push: Some(out_push) },
        buses,
        master_gain: 1.0,
        master_applied: None,
        scratch,
        workers,
        fx_auto: FxAuto { effects: Vec::with_capacity(MAX_NODES * 4), queue: Vec::with_capacity(EVENTS_PER_NODE) },
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
                        for p in old.plugins() {
                            if !next.plugins().any(|q| Arc::ptr_eq(q, p)) {
                                p.rt_stop();
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
                        self.nodes[i].direct = n.direct;
                        self.nodes[i].midi = n.plugin.as_ref().is_some_and(|p| p.midi_dialect);
                        for (k, s) in n.sends.iter().enumerate().take(MAX_SENDS) {
                            self.nodes[i].sends[k] = s.1;
                        }
                    }
                    for (i, b) in next.buses.iter().enumerate().take(MAX_BUSES) {
                        let bs = &mut self.buses[i];
                        (bs.gain, bs.pan, bs.audible) = (b.gain, b.pan, b.audible);
                    }
                    self.master_gain = next.master_gain;
                    self.shared.generation.store(next.generation, Ordering::Release);
                    self.fx_auto.effects.clear();
                    for e in next.effects.iter().take(self.fx_auto.effects.capacity()) {
                        self.fx_auto.effects.push(Arc::as_ptr(e));
                    }
                    self.snap = Some(next);
                    self.reset_cursor();
                    self.chase();
                }
                Command::Play => {
                    self.playing = true;
                    self.reset_cursor();
                    self.send_programs();
                    self.chase();
                }
                Command::Stop => {
                    self.playing = false;
                    self.release_all(0);
                }
                Command::Seek(s) => {
                    self.release_all(0);
                    self.pos = s.max(0);
                    self.reset_cursor();
                    self.chase();
                }
                Command::Mix { node, gain, pan, audible, direct } => {
                    if let Some(n) = self.nodes.get_mut(node as usize) {
                        n.gain = gain;
                        n.pan = pan;
                        n.audible = audible;
                        n.direct = direct;
                    }
                }
                Command::MixBus { bus, gain, pan, audible } => {
                    if let Some(b) = self.buses.get_mut(bus as usize) {
                        (b.gain, b.pan, b.audible) = (gain, pan, audible);
                    }
                }
                Command::Send { node, index, gain } => {
                    if let Some(s) = self.nodes.get_mut(node as usize).and_then(|n| n.sends.get_mut(index as usize)) {
                        *s = gain;
                    }
                }
                Command::Master { gain } => self.master_gain = gain,
                Command::Loop(c) => self.cycle = c.filter(|(a, b)| a < b),
                Command::Preview { node, key, channel, velocity } => {
                    let a = Active { node, note_id: -1, port: 0, channel, key };
                    self.push_event(node, a, velocity >= 0.0, velocity.max(0.0), 0);
                }
            }
        }
    }

    /// Automation before the playhead: each target takes its latest value
    /// (faders and pans here; parameters and controllers with the block).
    fn chase(&mut self) {
        let Some(snap) = self.snap.take() else { return };
        let upto = self.cursor.min(snap.notes.len());
        // Latest first: a target already set is skipped.
        let mut done: [u64; 64] = [u64::MAX; 64];
        let mut n_done = 0;
        for n in snap.notes[..upto].iter().rev() {
            let key = match n.kind {
                Kind::Note => continue,
                Kind::Cc(cc) => (1u64 << 60) | ((n.node as u64) << 8) | cc as u64,
                Kind::Program => (5u64 << 60) | n.node as u64,
                Kind::Gain => (2u64 << 60) | n.node as u64,
                Kind::Pan => (3u64 << 60) | n.node as u64,
                Kind::Param { fx, id } => (4u64 << 60) | ((fx as u64) << 32) | id as u64 ^ ((n.node as u64) << 20),
                Kind::BusGain => (6u64 << 60) | n.node as u64,
                Kind::BusPan => (7u64 << 60) | n.node as u64,
                Kind::MasterGain => 8u64 << 60,
                Kind::Send { index } => (9u64 << 60) | ((n.node as u64) << 8) | index as u64,
                Kind::PitchBend => (10u64 << 60) | n.node as u64,
            };
            if done[..n_done].contains(&key) {
                continue;
            }
            if n_done == done.len() {
                break;
            }
            done[n_done] = key;
            n_done += 1;
            self.apply(n, 0);
        }
        self.snap = Some(snap);
    }

    /// An automation or controller event at a frame of the block.
    fn apply(&mut self, n: &ScheduledNote, time: u32) {
        // Buses and the master.
        match n.kind {
            Kind::BusGain | Kind::BusPan => {
                if let Some(b) = self.buses.get_mut(n.node as usize) {
                    if n.kind == Kind::BusGain {
                        b.gain = n.velocity;
                    } else {
                        b.pan = n.velocity;
                    }
                }
                return;
            }
            Kind::MasterGain => {
                self.master_gain = n.velocity;
                return;
            }
            _ => {}
        }
        let Some(ns) = self.nodes.get_mut(n.node as usize) else { return };
        match n.kind {
            Kind::Note | Kind::BusGain | Kind::BusPan | Kind::MasterGain => {}
            Kind::Send { index } => {
                if let Some(s) = ns.sends.get_mut(index as usize) {
                    *s = n.velocity;
                }
            }
            Kind::PitchBend => {
                if ns.events.events.len() < EVENTS_PER_NODE {
                    let v = ((n.velocity.clamp(-1.0, 1.0) + 1.0) * 8191.5).round().min(16383.0) as u16;
                    let data = [0xe0 | (n.channel as u8 & 15), (v & 0x7f) as u8, (v >> 7) as u8];
                    ns.events.events.push(EventSlot { midi: clap_event_midi { header: clap_event_header { size: std::mem::size_of::<clap_event_midi>() as u32, time, space_id: CLAP_CORE_EVENT_SPACE_ID, type_: CLAP_EVENT_MIDI, flags: 0 }, port_index: n.port as u16, data } });
                }
            }
            Kind::Gain => ns.gain = n.velocity,
            Kind::Pan => ns.pan = n.velocity,
            Kind::Cc(cc) => {
                if ns.events.events.len() < EVENTS_PER_NODE {
                    let data = [0xb0 | (n.channel as u8 & 15), cc & 0x7f, (n.velocity.clamp(0.0, 1.0) * 127.0).round() as u8];
                    ns.events.events.push(EventSlot { midi: clap_event_midi { header: clap_event_header { size: std::mem::size_of::<clap_event_midi>() as u32, time, space_id: CLAP_CORE_EVENT_SPACE_ID, type_: CLAP_EVENT_MIDI, flags: 0 }, port_index: n.port as u16, data } });
                }
            }
            Kind::Program => {
                if ns.events.events.len() < EVENTS_PER_NODE {
                    let data = [0xc0 | (n.channel as u8 & 15), n.key as u8 & 0x7f, 0];
                    ns.events.events.push(EventSlot { midi: clap_event_midi { header: clap_event_header { size: std::mem::size_of::<clap_event_midi>() as u32, time, space_id: CLAP_CORE_EVENT_SPACE_ID, type_: CLAP_EVENT_MIDI, flags: 0 }, port_index: n.port as u16, data } });
                }
            }
            Kind::Param { fx: u32::MAX, id } => {
                if ns.events.events.len() < EVENTS_PER_NODE {
                    ns.events.events.push(EventSlot { param: crate::host::param_event(id, n.velocity as f64, time) });
                }
            }
            Kind::Param { fx, id } => {
                if self.fx_auto.queue.len() < self.fx_auto.queue.capacity() {
                    self.fx_auto.queue.push((fx, id, n.velocity as f64, time));
                }
            }
        }
    }

    /// Each instrument's program (bank select first), at the block's start.
    fn send_programs(&mut self) {
        let Some(snap) = self.snap.as_ref() else { return };
        for (i, node) in snap.nodes.iter().enumerate().take(MAX_NODES) {
            let Some((ch, program, bank)) = node.program else { continue };
            let events = &mut self.nodes[i].events.events;
            let mut midi = |data: [u8; 3]| {
                if events.len() < EVENTS_PER_NODE {
                    events.push(EventSlot { midi: clap_event_midi { header: clap_event_header { size: std::mem::size_of::<clap_event_midi>() as u32, time: 0, space_id: CLAP_CORE_EVENT_SPACE_ID, type_: CLAP_EVENT_MIDI, flags: 0 }, port_index: 0, data } });
                }
            };
            if let Some(b) = bank {
                midi([0xb0 | ch, 0, (b >> 7) as u8 & 0x7f]);
                midi([0xb0 | ch, 32, b as u8 & 0x7f]);
            }
            midi([0xc0 | ch, program & 0x7f, 0]);
        }
    }

    /// Note-offs queued for plugins that are about to leave with the old
    /// plan: deliver them with an empty render so voices end cleanly.
    fn flush_releases_to_old_plugins(&mut self) {
        let Some(snap) = self.snap.as_ref() else { return };
        for (i, n) in snap.nodes.iter().enumerate().take(MAX_NODES) {
            let Some(plugin) = &n.plugin else { continue };
            if self.nodes[i].events.events.is_empty() || !plugin.rt_start() {
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
            unsafe { plugin.rt_process(&p) };
            ns.events.events.clear();
        }
    }

    /// Fills `left`/`right` (same length, at most `max_frames`).
    pub fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        let frames = left.len().min(right.len()).min(self.max_frames as usize);
        // In parts where the cycle's end falls inside the block: after each,
        // back to the cycle's start (notes released, automation caught up).
        let mut at = 0;
        let mut first = true;
        while at < frames || first {
            let ahead = self.cycle.filter(|&(_, end)| self.playing && self.pos < end);
            let n = match ahead {
                Some((_, end)) => ((end - self.pos) as usize).min(frames - at),
                None => frames - at,
            };
            self.process_part(&mut left[at..at + n], &mut right[at..at + n], first);
            first = false;
            at += n;
            if let Some((start, _)) = ahead.filter(|&(_, end)| self.playing && self.pos >= end) {
                self.release_all(0);
                self.pos = start;
                self.reset_cursor();
                self.chase();
            }
        }
        self.shared.position.store(self.pos, Ordering::Release);
        self.shared.playing.store(self.playing, Ordering::Release);
        self.shared.blocks.fetch_add(1, Ordering::Relaxed);
    }

    /// Renders a stretch of the block (the commands go with the first).
    fn process_part(&mut self, left: &mut [f32], right: &mut [f32], first: bool) {
        let frames = left.len().min(right.len());
        left.fill(0.0);
        right.fill(0.0);
        // Parameter changes from the main thread lead the block.
        if let Some(snap) = self.snap.as_ref() {
            for (i, node) in snap.nodes.iter().enumerate().take(MAX_NODES) {
                let events = &mut self.nodes[i].events.events;
                let Some(plugin) = &node.plugin else { continue };
                plugin.rt_take_params(|id, v| {
                    if events.len() < EVENTS_PER_NODE {
                        events.push(EventSlot { param: crate::host::param_event(id, v, 0) });
                    }
                });
            }
        }
        if first {
            self.handle_commands();
        }
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
                let cycling = self.cycle.is_some_and(|(_, e)| block_start < e);
                if block_end >= end && self.active.is_empty() && !cycling {
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
        // The events were delivered: the next part's (a cycle's note-offs,
        // say) are added from here.
        for n in self.nodes.iter_mut() {
            n.events.events.clear();
        }
        self.fx_auto.queue.clear();
    }

    fn schedule(&mut self, n: &ScheduledNote, time: u32) {
        if n.kind != Kind::Note {
            self.apply(n, time);
            return;
        }
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
        let nb = snap.buses.len().min(MAX_BUSES);
        for b in self.buses.iter_mut().take(nb) {
            b.left[..frames].fill(0.0);
            b.right[..frames].fill(0.0);
        }
        // Instruments in parallel, each into its own buffers.
        let count = snap.nodes.len().min(MAX_NODES);
        let jobs = NodeJobs {
            nodes: self.nodes.as_mut_ptr(),
            plan: snap.nodes.as_ptr(),
            scratch: self.scratch.as_mut_ptr(),
            transport: &transport,
            out_events: &self.out_events,
            fx_auto: &self.fx_auto,
            frames,
            steady: self.steady,
            at,
            playing: self.playing,
        };
        unsafe { self.workers.run(count, render_node, &jobs as *const NodeJobs as *const ()) };
        // Then the mix, in order.
        for (i, node) in snap.nodes.iter().enumerate().take(count) {
            let ns = &mut self.nodes[i];
            if !ns.ran {
                continue;
            }
            // Equal-power pan; silent when muted (or another track soloed).
            let (gl, gr) = pan_gains(ns.gain, ns.pan, ns.audible);
            let (l0, r0) = ns.applied.unwrap_or((gl, gr));
            ns.applied = Some((gl, gr));
            // Its own output: silent while only its sends feed a soloed bus.
            let (ml, mr) = if ns.direct { (gl, gr) } else { (0.0, 0.0) };
            let (ml0, mr0) = ns.main_applied.unwrap_or((ml, mr));
            ns.main_applied = Some((ml, mr));
            // Into its bus or the master; sends to buses.
            let (dl, dr): (&mut [f32], &mut [f32]) = match node.out.filter(|&b| (b as usize) < nb) {
                Some(b) => {
                    let bs = &mut self.buses[b as usize];
                    (&mut bs.left[..frames], &mut bs.right[..frames])
                }
                None => (&mut left[..frames], &mut right[..frames]),
            };
            mix_into(dl, &ns.left[..frames], ml0, ml);
            mix_into(dr, &ns.right[..frames], mr0, mr);
            for (k, &(bus, _, pre)) in node.sends.iter().enumerate().take(MAX_SENDS) {
                if (bus as usize) >= nb {
                    continue;
                }
                let level = if ns.audible { ns.sends[k] } else { 0.0 };
                let from = ns.send_applied[k].unwrap_or(level);
                ns.send_applied[k] = Some(level);
                let bs = &mut self.buses[bus as usize];
                if pre {
                    // Before the fader and pan.
                    mix_into(&mut bs.left[..frames], &ns.left[..frames], from, level);
                    mix_into(&mut bs.right[..frames], &ns.right[..frames], from, level);
                } else {
                    // After the fader, panned as the track.
                    mix_into(&mut bs.left[..frames], &ns.left[..frames], from * l0, level * gl);
                    mix_into(&mut bs.right[..frames], &ns.right[..frames], from * r0, level * gr);
                }
            }
        }
        // Buses in the plan's order: each into a later bus, or the master.
        for (b, strip) in snap.buses.iter().enumerate().take(nb) {
            let (head, rest) = self.buses.split_at_mut(b + 1);
            let bs = &mut head[b];
            let sc = &mut self.scratch[0];
            for fx in &strip.inserts {
                unsafe { run_effect(fx, &mut bs.left[..frames], &mut bs.right[..frames], &mut sc.tmp, frames, self.steady, &transport, &mut sc.fx_events, &self.out_events, &self.fx_auto) };
            }
            let (gl, gr) = pan_gains(bs.gain, bs.pan, bs.audible);
            let (l0, r0) = bs.applied.unwrap_or((gl, gr));
            bs.applied = Some((gl, gr));
            let (dl, dr): (&mut [f32], &mut [f32]) = match strip.out.map(|o| o as usize).filter(|&o| o > b && o < nb) {
                Some(o) => {
                    let d = &mut rest[o - b - 1];
                    (&mut d.left[..frames], &mut d.right[..frames])
                }
                None => (&mut left[..frames], &mut right[..frames]),
            };
            mix_into(dl, &bs.left[..frames], l0, gl);
            mix_into(dr, &bs.right[..frames], r0, gr);
        }
        let sc = &mut self.scratch[0];
        for fx in &snap.master_inserts {
            unsafe { run_effect(fx, &mut left[..frames], &mut right[..frames], &mut sc.tmp, frames, self.steady, &transport, &mut sc.fx_events, &self.out_events, &self.fx_auto) };
        }
        let g = self.master_gain;
        let g0 = self.master_applied.unwrap_or(g);
        self.master_applied = Some(g);
        if g0 != 1.0 || g != 1.0 {
            let n = frames.max(1) as f32;
            for f in 0..frames {
                let k = g0 + (g - g0) * (f + 1) as f32 / n;
                left[f] *= k;
                right[f] *= k;
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
            for p in s.plugins() {
                p.rt_stop();
            }
        }
    }
}
