//! Corinzia Sine: a small polyphonic CLAP instrument.
//!
//! It is Corinzia's built-in sound (tracks without a plugin play through it,
//! in-process via [`entry`]) and the test plugin of the CLAP host. As a
//! cdylib it is also a regular `.clap` bundle any host can load.
//!
//! A soft organ-like tone (fundamental plus a quiet octave), a short attack
//! and release, 32 voices with voice stealing, one stereo output, one note
//! input (CLAP and MIDI dialects), and state: the output gain.

#![allow(clippy::missing_safety_doc)]

use clap_sys::events::*;
use clap_sys::ext::audio_ports::*;
use clap_sys::ext::note_ports::*;
use clap_sys::ext::state::*;
use clap_sys::factory::plugin_factory::*;
use clap_sys::host::clap_host;
use clap_sys::id::CLAP_INVALID_ID;
use clap_sys::plugin::*;
use clap_sys::process::*;
use clap_sys::stream::*;
use clap_sys::version::CLAP_VERSION;
use std::ffi::{c_char, c_void, CStr};
use std::ptr;

pub mod effects;

pub const PLUGIN_ID: &CStr = c"org.corinzia.sine";

pub struct Sync<T>(pub T);
unsafe impl<T> std::marker::Sync for Sync<T> {}

static FEATURES: Sync<[*const c_char; 4]> = Sync([c"instrument".as_ptr(), c"synthesizer".as_ptr(), c"stereo".as_ptr(), ptr::null()]);

static DESCRIPTOR: Sync<clap_plugin_descriptor> = Sync(clap_plugin_descriptor {
    clap_version: CLAP_VERSION,
    id: PLUGIN_ID.as_ptr(),
    name: c"Corinzia Sine".as_ptr(),
    vendor: c"Corinzia".as_ptr(),
    url: c"".as_ptr(),
    manual_url: c"".as_ptr(),
    support_url: c"".as_ptr(),
    version: c"0.1.0".as_ptr(),
    description: c"A soft polyphonic sine instrument".as_ptr(),
    features: &FEATURES.0 as *const _ as *const *const c_char,
});

const VOICES: usize = 32;
const STATE_MAGIC: &[u8; 4] = b"CZS1";

#[derive(Clone, Copy, Default)]
struct Voice {
    active: bool,
    releasing: bool,
    note_id: i32,
    port: i16,
    channel: i16,
    key: i16,
    phase: f64,
    step: f64,
    level: f32, // envelope
    velocity: f32,
    age: u64,
}

/// The sounds a MIDI program change selects: (name, octave, twelfth) levels
/// over the fundamental.
pub const PROGRAMS: [(&str, f64, f64); 3] = [("Soft organ", 0.18, 0.0), ("Pure sine", 0.0, 0.0), ("Bright", 0.45, 0.25)];

struct Synth {
    plugin: clap_plugin,
    host: *const clap_host,
    sample_rate: f64,
    gain: f32,
    program: u8,
    /// MIDI expression (CC11) and volume (CC7), 0..1.
    expression: f32,
    volume: f32,
    voices: [Voice; VOICES],
    clock: u64,
}

impl Synth {
    unsafe fn from<'a>(p: *const clap_plugin) -> &'a mut Synth {
        &mut *((*p).plugin_data as *mut Synth)
    }

    fn note_on(&mut self, note_id: i32, port: i16, channel: i16, key: i16, velocity: f64) {
        self.clock += 1;
        // A free voice, else the oldest releasing one, else the oldest.
        let i = (0..VOICES)
            .find(|&i| !self.voices[i].active)
            .or_else(|| (0..VOICES).filter(|&i| self.voices[i].releasing).min_by_key(|&i| self.voices[i].age))
            .unwrap_or_else(|| (0..VOICES).min_by_key(|&i| self.voices[i].age).unwrap());
        let freq = 440.0 * 2f64.powf((key as f64 - 69.0) / 12.0);
        self.voices[i] = Voice {
            active: true,
            releasing: false,
            note_id,
            port,
            channel,
            key,
            phase: 0.0,
            step: freq / self.sample_rate,
            level: 0.0,
            velocity: velocity.clamp(0.0, 1.0) as f32,
            age: self.clock,
        };
    }

    fn note_off(&mut self, note_id: i32, channel: i16, key: i16) {
        for v in self.voices.iter_mut().filter(|v| v.active && !v.releasing) {
            let by_id = note_id >= 0 && v.note_id == note_id;
            let by_key = note_id < 0 && (key < 0 || v.key == key) && (channel < 0 || v.channel == channel);
            if by_id || by_key {
                v.releasing = true;
            }
        }
    }

    fn event(&mut self, h: *const clap_event_header) {
        unsafe {
            if (*h).space_id != CLAP_CORE_EVENT_SPACE_ID {
                return;
            }
            match (*h).type_ {
                CLAP_EVENT_NOTE_ON => {
                    let n = &*(h as *const clap_event_note);
                    self.note_on(n.note_id, n.port_index, n.channel, n.key, n.velocity);
                }
                CLAP_EVENT_NOTE_OFF | CLAP_EVENT_NOTE_CHOKE => {
                    let n = &*(h as *const clap_event_note);
                    self.note_off(n.note_id, n.channel, n.key);
                }
                CLAP_EVENT_MIDI => {
                    let m = &*(h as *const clap_event_midi);
                    let (status, key, vel) = (m.data[0] & 0xF0, m.data[1] as i16, m.data[2]);
                    let ch = (m.data[0] & 0x0F) as i16;
                    match (status, vel) {
                        (0x90, v) if v > 0 => self.note_on(-1, m.port_index as i16, ch, key, v as f64 / 127.0),
                        (0x80, _) | (0x90, _) => self.note_off(-1, ch, key),
                        (0xB0, v) if key == 11 => self.expression = v as f32 / 127.0,
                        (0xB0, v) if key == 7 => self.volume = v as f32 / 127.0,
                        (0xC0, _) => self.program = (key as u8).min(PROGRAMS.len() as u8 - 1),
                        _ => {}
                    }
                }
                _ => {}
            }
        }
    }

    /// Renders `[from, to)` frames into the output buffers.
    fn render(&mut self, left: &mut [f32], right: &mut [f32], out: *const clap_output_events, time: u32) {
        let attack = (1.0 / (0.006 * self.sample_rate)) as f32;
        let release = (1.0 / (0.15 * self.sample_rate)) as f32;
        let (_, octave, twelfth) = PROGRAMS[self.program as usize];
        for v in self.voices.iter_mut().filter(|v| v.active) {
            for (l, r) in left.iter_mut().zip(right.iter_mut()) {
                if v.releasing {
                    v.level -= release;
                    if v.level <= 0.0 {
                        v.active = false;
                        break;
                    }
                } else if v.level < 1.0 {
                    v.level = (v.level + attack).min(1.0);
                }
                let ph = v.phase * std::f64::consts::TAU;
                let s = (ph.sin() + octave * (2.0 * ph).sin() + twelfth * (3.0 * ph).sin()) as f32 * v.level * v.velocity * 0.18 * self.gain * self.expression * self.volume;
                v.phase = (v.phase + v.step).fract();
                *l += s;
                *r += s;
            }
            if !v.active && !out.is_null() {
                // Tell the host the voice is gone.
                let end = clap_event_note {
                    header: clap_event_header { size: std::mem::size_of::<clap_event_note>() as u32, time, space_id: CLAP_CORE_EVENT_SPACE_ID, type_: CLAP_EVENT_NOTE_END, flags: 0 },
                    note_id: v.note_id,
                    port_index: v.port,
                    channel: v.channel,
                    key: v.key,
                    velocity: 0.0,
                };
                unsafe {
                    if let Some(push) = (*out).try_push {
                        push(out, &end.header);
                    }
                }
            }
        }
    }
}

// ------------------------------------------------------------ plugin calls --
unsafe extern "C" fn init(_p: *const clap_plugin) -> bool {
    true
}
unsafe extern "C" fn destroy(p: *const clap_plugin) {
    drop(Box::from_raw((*p).plugin_data as *mut Synth));
}
unsafe extern "C" fn activate(p: *const clap_plugin, sr: f64, _min: u32, _max: u32) -> bool {
    let s = Synth::from(p);
    s.sample_rate = sr;
    s.voices = [Voice::default(); VOICES];
    true
}
unsafe extern "C" fn deactivate(_p: *const clap_plugin) {}
unsafe extern "C" fn start_processing(_p: *const clap_plugin) -> bool {
    true
}
unsafe extern "C" fn stop_processing(_p: *const clap_plugin) {}
unsafe extern "C" fn reset(p: *const clap_plugin) {
    Synth::from(p).voices = [Voice::default(); VOICES];
}

unsafe extern "C" fn process(p: *const clap_plugin, process: *const clap_process) -> clap_process_status {
    let s = Synth::from(p);
    let pr = &*process;
    if pr.audio_outputs_count == 0 || (*pr.audio_outputs).channel_count < 2 {
        return CLAP_PROCESS_ERROR;
    }
    let frames = pr.frames_count as usize;
    let chans = (*pr.audio_outputs).data32;
    let left = std::slice::from_raw_parts_mut(*chans, frames);
    let right = std::slice::from_raw_parts_mut(*chans.add(1), frames);
    left.fill(0.0);
    right.fill(0.0);
    let events = pr.in_events;
    let count = if events.is_null() { 0 } else { (*events).size.map(|f| f(events)).unwrap_or(0) };
    // Sample-accurate: render up to each event, apply it, continue.
    let mut at = 0usize;
    for i in 0..count {
        let Some(h) = (*events).get.map(|g| g(events, i)) else { continue };
        if h.is_null() {
            continue;
        }
        let t = ((*h).time as usize).min(frames);
        if t > at {
            s.render(&mut left[at..t], &mut right[at..t], pr.out_events, t as u32);
            at = t;
        }
        s.event(h);
    }
    if at < frames {
        s.render(&mut left[at..], &mut right[at..], pr.out_events, frames as u32 - 1);
    }
    if s.voices.iter().any(|v| v.active) {
        CLAP_PROCESS_CONTINUE
    } else {
        CLAP_PROCESS_SLEEP
    }
}

// ------------------------------------------------------------- extensions --
unsafe extern "C" fn audio_ports_count(_p: *const clap_plugin, is_input: bool) -> u32 {
    if is_input {
        0
    } else {
        1
    }
}
unsafe extern "C" fn audio_ports_get(_p: *const clap_plugin, index: u32, is_input: bool, info: *mut clap_audio_port_info) -> bool {
    if is_input || index != 0 {
        return false;
    }
    let i = &mut *info;
    i.id = 0;
    write_name(&mut i.name, "Output");
    i.flags = CLAP_AUDIO_PORT_IS_MAIN;
    i.channel_count = 2;
    i.port_type = CLAP_PORT_STEREO.as_ptr();
    i.in_place_pair = CLAP_INVALID_ID;
    true
}
static AUDIO_PORTS: clap_plugin_audio_ports = clap_plugin_audio_ports { count: Some(audio_ports_count), get: Some(audio_ports_get) };

unsafe extern "C" fn note_ports_count(_p: *const clap_plugin, is_input: bool) -> u32 {
    if is_input {
        1
    } else {
        0
    }
}
unsafe extern "C" fn note_ports_get(_p: *const clap_plugin, index: u32, is_input: bool, info: *mut clap_note_port_info) -> bool {
    if !is_input || index != 0 {
        return false;
    }
    let i = &mut *info;
    i.id = 0;
    i.supported_dialects = CLAP_NOTE_DIALECT_CLAP | CLAP_NOTE_DIALECT_MIDI;
    i.preferred_dialect = CLAP_NOTE_DIALECT_CLAP;
    write_name(&mut i.name, "Notes");
    true
}
static NOTE_PORTS: clap_plugin_note_ports = clap_plugin_note_ports { count: Some(note_ports_count), get: Some(note_ports_get) };

unsafe extern "C" fn state_save(p: *const clap_plugin, stream: *const clap_ostream) -> bool {
    let s = Synth::from(p);
    let mut bytes = STATE_MAGIC.to_vec();
    bytes.extend_from_slice(&s.gain.to_le_bytes());
    bytes.push(s.program);
    write_all(stream, &bytes)
}

/// Writes all of `bytes` to a CLAP stream.
pub(crate) unsafe fn write_all(stream: *const clap_ostream, bytes: &[u8]) -> bool {
    let Some(write) = (*stream).write else { return false };
    let mut done = 0;
    while done < bytes.len() {
        let n = write(stream, bytes[done..].as_ptr() as *const c_void, (bytes.len() - done) as u64);
        if n <= 0 {
            return false;
        }
        done += n as usize;
    }
    true
}

/// Reads a CLAP stream to its end.
pub(crate) unsafe fn read_all(stream: *const clap_istream) -> Option<Vec<u8>> {
    let read = (*stream).read?;
    let mut bytes = Vec::new();
    let mut buf = [0u8; 64];
    loop {
        let n = read(stream, buf.as_mut_ptr() as *mut c_void, buf.len() as u64);
        if n < 0 {
            return None;
        }
        if n == 0 {
            return Some(bytes);
        }
        bytes.extend_from_slice(&buf[..n as usize]);
    }
}

unsafe extern "C" fn state_load(p: *const clap_plugin, stream: *const clap_istream) -> bool {
    let Some(bytes) = read_all(stream) else { return false };
    if bytes.len() < 8 || &bytes[..4] != STATE_MAGIC {
        return false;
    }
    Synth::from(p).gain = f32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    if let Some(&program) = bytes.get(8) {
        Synth::from(p).program = program.min(PROGRAMS.len() as u8 - 1);
    }
    true
}
static STATE: clap_plugin_state = clap_plugin_state { save: Some(state_save), load: Some(state_load) };

unsafe extern "C" fn get_extension(_p: *const clap_plugin, id: *const c_char) -> *const c_void {
    let id = CStr::from_ptr(id);
    if id == CLAP_EXT_AUDIO_PORTS {
        &AUDIO_PORTS as *const _ as *const c_void
    } else if id == CLAP_EXT_NOTE_PORTS {
        &NOTE_PORTS as *const _ as *const c_void
    } else if id == CLAP_EXT_STATE {
        &STATE as *const _ as *const c_void
    } else {
        ptr::null()
    }
}
unsafe extern "C" fn on_main_thread(_p: *const clap_plugin) {}

pub(crate) fn write_name(dst: &mut [c_char], s: &str) {
    let n = dst.len();
    for (d, b) in dst.iter_mut().take(n.saturating_sub(1)).zip(s.bytes()) {
        *d = b as c_char;
    }
    if let Some(end) = dst.get_mut(s.len().min(n.saturating_sub(1))) {
        *end = 0;
    }
}

/// Sets the output gain of an instance (tests and the built-in default).
pub unsafe fn set_gain(p: *const clap_plugin, gain: f32) {
    Synth::from(p).gain = gain;
}

// ------------------------------------------------------- factory and entry --
unsafe extern "C" fn factory_count(_f: *const clap_plugin_factory) -> u32 {
    5
}
unsafe extern "C" fn factory_descriptor(_f: *const clap_plugin_factory, index: u32) -> *const clap_plugin_descriptor {
    match index {
        0 => &DESCRIPTOR.0,
        1 => &effects::GAIN_DESCRIPTOR.0,
        2 => &effects::REVERB_DESCRIPTOR.0,
        3 => &effects::EQ_DESCRIPTOR.0,
        4 => &effects::LIMITER_DESCRIPTOR.0,
        _ => ptr::null(),
    }
}
unsafe extern "C" fn factory_create(_f: *const clap_plugin_factory, host: *const clap_host, id: *const c_char) -> *const clap_plugin {
    if id.is_null() {
        return ptr::null();
    }
    if CStr::from_ptr(id) != PLUGIN_ID {
        return effects::create(host, CStr::from_ptr(id));
    }
    let synth = Box::into_raw(Box::new(Synth {
        plugin: clap_plugin {
            desc: &DESCRIPTOR.0,
            plugin_data: ptr::null_mut(),
            init: Some(init),
            destroy: Some(destroy),
            activate: Some(activate),
            deactivate: Some(deactivate),
            start_processing: Some(start_processing),
            stop_processing: Some(stop_processing),
            reset: Some(reset),
            process: Some(process),
            get_extension: Some(get_extension),
            on_main_thread: Some(on_main_thread),
        },
        host,
        sample_rate: 48000.0,
        gain: 1.0,
        program: 0,
        expression: 1.0,
        volume: 1.0,
        voices: [Voice::default(); VOICES],
        clock: 0,
    }));
    (*synth).plugin.plugin_data = synth as *mut c_void;
    let _ = (*synth).host;
    &(*synth).plugin
}
static FACTORY: clap_plugin_factory = clap_plugin_factory {
    get_plugin_count: Some(factory_count),
    get_plugin_descriptor: Some(factory_descriptor),
    create_plugin: Some(factory_create),
};

unsafe extern "C" fn entry_init(_path: *const c_char) -> bool {
    true
}
unsafe extern "C" fn entry_deinit() {}
unsafe extern "C" fn entry_get_factory(id: *const c_char) -> *const c_void {
    if !id.is_null() && CStr::from_ptr(id) == CLAP_PLUGIN_FACTORY_ID {
        &FACTORY as *const _ as *const c_void
    } else {
        ptr::null()
    }
}

/// The CLAP entry point (exported for `.clap` bundles; the host also uses
/// it in-process for the built-in instrument).
#[allow(non_upper_case_globals)]
#[no_mangle]
pub static clap_entry: clap_plugin_entry = clap_plugin_entry {
    clap_version: CLAP_VERSION,
    init: Some(entry_init),
    deinit: Some(entry_deinit),
    get_factory: Some(entry_get_factory),
};

pub use clap_sys::entry::clap_plugin_entry;

/// The entry as a pointer, for in-process hosting.
pub fn entry() -> &'static clap_plugin_entry {
    &clap_entry
}
