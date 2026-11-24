//! Corinzia Ensemble: Corinzia's built-in instruments, as a CLAP plugin.
//!
//! Tracks without a plugin play through it, in-process via [`entry`]; as a
//! cdylib it is also a regular `.clap` bundle any host can load, and the
//! test plugin of Corinzia's CLAP host. It also holds the built-in effects
//! ([`effects`]).
//!
//! Sixteen MIDI channels, each with its own sound, chosen by General MIDI
//! program (bank 1: string sections and choir; bank 128: the drum kit; see
//! [`patches`]). The sounds are calibrated against each other, so a score
//! is balanced with every fader at 0 dB ([`patches::trims`]).
//!
//! Controllers: CC1 the dynamic of held notes (hairpins), CC3 the playing
//! technique ([`voice::tech`]), CC9 the articulation ([`voice::articulation`]),
//! CC7 volume, CC11 expression, CC68 legato
//! (slurred notes join), CC120/123 all sound/notes off, pitch bend ±2
//! semitones.

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

pub mod dsp;
pub mod effects;
pub mod patches;
pub mod body;
pub mod bowed;
mod bowed_tuning;
mod bowed_levels;
pub mod voice;

use voice::Voice;

pub const PLUGIN_ID: &CStr = c"org.corinzia.ensemble";
/// The controller giving how many players play a channel's part (a string
/// section's size, 1 to 32; 0: the sound's own).
pub const PLAYERS_CC: u8 = 86;

/// The id of the instrument this one replaced; projects that name it get this.
pub const OLD_PLUGIN_ID: &CStr = c"org.corinzia.sine";

pub struct Sync<T>(pub T);
unsafe impl<T> std::marker::Sync for Sync<T> {}

static FEATURES: Sync<[*const c_char; 4]> = Sync([c"instrument".as_ptr(), c"synthesizer".as_ptr(), c"stereo".as_ptr(), ptr::null()]);

static DESCRIPTOR: Sync<clap_plugin_descriptor> = Sync(clap_plugin_descriptor {
    clap_version: CLAP_VERSION,
    id: PLUGIN_ID.as_ptr(),
    name: c"Corinzia Ensemble".as_ptr(),
    vendor: c"Corinzia".as_ptr(),
    url: c"".as_ptr(),
    manual_url: c"".as_ptr(),
    support_url: c"".as_ptr(),
    version: c"0.3.1".as_ptr(),
    description: c"Synthesized orchestral, keyboard and percussion instruments".as_ptr(),
    features: &FEATURES.0 as *const _ as *const *const c_char,
});

/// The name the built-in instruments go by.
pub const NAME: &str = "Corinzia Ensemble";

const VOICES: usize = 48;
const STATE_MAGIC: &[u8; 4] = b"CZE1";

#[derive(Clone, Copy)]
struct Channel {
    program: u8,
    bank_msb: u8,
    bank_lsb: u8,
    patch: usize,
    /// CC1 once received: held notes follow it.
    dynamic: Option<f32>,
    technique: u8,
    /// The articulation controller (CC9), and a bowed line's next stroke.
    articulation: u8,
    down_bow: bool,
    legato: bool,
    expression: f32,
    volume: f32,
    /// Pitch bend in semitones (the usual ±2 range).
    bend: f32,
    /// How many play its part (0: the sound's own).
    players: u8,
}

impl Default for Channel {
    fn default() -> Self {
        Channel { program: 0, bank_msb: 0, bank_lsb: 0, patch: patches::patch_for(0, 0), dynamic: None, technique: 0, articulation: 0, down_bow: true, legato: false, expression: 1.0, volume: 1.0, bend: 0.0, players: 0 }
    }
}

struct Synth {
    plugin: clap_plugin,
    host: *const clap_host,
    sample_rate: f64,
    gain: f32,
    channels: [Channel; 16],
    voices: Vec<Voice>,
    /// The bowed strings' players, shared by the voices.
    pool: voice::PlayerPool,
    clock: u64,
    trims: &'static [f32; patches::COUNT],
    scratch: Vec<f32>,
}

impl Synth {
    unsafe fn from<'a>(p: *const clap_plugin) -> &'a mut Synth {
        &mut *((*p).plugin_data as *mut Synth)
    }

    fn new_voices(&mut self) {
        self.voices = (0..VOICES).map(|_| Voice::new(self.sample_rate)).collect();
        self.pool = voice::PlayerPool::new(self.sample_rate, voice::POOL);
        self.scratch = vec![0.0; 1024];
    }

    fn note_on(&mut self, note_id: i32, port: i16, channel: i16, key: i16, velocity: f64) {
        self.clock += 1;
        let ch = (channel & 15).max(0) as usize;
        let c = self.channels[ch];
        let velocity = velocity.clamp(0.0, 1.0) as f32;
        // A slur: the line's held note moves on instead of a new attack.
        if c.legato {
            if let Some(v) = self.voices.iter_mut().filter(|v| v.active && !v.releasing && v.sustained && v.channel as usize == ch && v.patch == c.patch && v.key != key).max_by_key(|v| v.age) {
                if patches::PATCHES[c.patch].legato {
                    v.legato_to(note_id, key, velocity, self.clock);
                    return;
                }
            }
        }
        // A closed hi-hat stops an open one.
        if c.patch == patches::KIT && matches!(key, 42 | 44) {
            for v in self.voices.iter_mut().filter(|v| v.active && v.channel as usize == ch && v.key == 46) {
                v.active = false;
            }
        }
        // A free voice, else the oldest releasing one, else the oldest.
        let i = (0..self.voices.len())
            .find(|&i| !self.voices[i].active)
            .or_else(|| (0..self.voices.len()).filter(|&i| self.voices[i].releasing).min_by_key(|&i| self.voices[i].age))
            .unwrap_or_else(|| (0..self.voices.len()).min_by_key(|&i| self.voices[i].age).unwrap());
        let seed = self.clock as u32;
        // A note right after another on the instrument, still ringing: a
        // bow change or a new breath, not a start from silence.
        let follows = self.voices.iter().any(|v| v.is_ringing() && v.channel as usize == ch && v.patch == c.patch);
        self.voices[i].start(c.patch, c.technique, note_id, port, channel, key, velocity, c.dynamic, self.clock, seed);
        // The stroke: as written, else down and up by turns.
        use voice::articulation as a;
        let down = if c.articulation & a::DOWN_BOW != 0 {
            true
        } else if c.articulation & a::UP_BOW != 0 {
            false
        } else {
            c.down_bow
        };
        self.channels[ch].down_bow = !down;
        self.voices[i].articulate(c.articulation, down);
        // Bowed strings: physically modelled players, as many as the part
        // has (else the sound's own). With every player busy, the oldest
        // notes already ending give theirs up, then the oldest held.
        if self.voices[i].wants_players() {
            let n = if c.players > 0 { c.players as usize } else { patches::PATCHES[c.patch].unison.max(1) as usize };
            let mut slots = [0u16; voice::MAX_SECTION];
            let mut k = self.take_players(i, n, &mut slots);
            if k == 0 {
                let mut order: Vec<usize> = (0..self.voices.len()).filter(|&o| o != i && !self.voices[o].players().is_empty()).collect();
                order.sort_by_key(|&o| (!self.voices[o].releasing, self.voices[o].age));
                for o in order {
                    // Silenced, its players free.
                    self.voices[o].active = false;
                    k = self.take_players(i, n, &mut slots);
                    if k > 0 {
                        break;
                    }
                }
            }
            self.voices[i].attach_players(&slots[..k], &mut self.pool, down);
        }
        if follows && self.voices[i].sustained {
            self.voices[i].connect();
        }
    }

    /// Up to `n` free players for voice `i`.
    fn take_players(&mut self, i: usize, n: usize, slots: &mut [u16; voice::MAX_SECTION]) -> usize {
        let voices = &self.voices;
        self.pool.take(i as u32, n, |o, slot| voices.get(o as usize).is_some_and(|v| v.players().contains(&slot)), slots)
    }

    fn note_off(&mut self, note_id: i32, channel: i16, key: i16) {
        for v in self.voices.iter_mut().filter(|v| v.active && !v.releasing) {
            let by_id = note_id >= 0 && v.note_id == note_id;
            let by_key = note_id < 0 && (key < 0 || v.key == key) && (channel < 0 || v.channel == channel);
            if by_id || by_key {
                v.release();
            }
        }
    }

    fn control(&mut self, ch: usize, cc: u8, value: u8) {
        let x = value as f32 / 127.0;
        let c = &mut self.channels[ch];
        match cc {
            0 => c.bank_msb = value,
            32 => c.bank_lsb = value,
            1 => {
                c.dynamic = Some(x);
                for v in self.voices.iter_mut().filter(|v| v.active && v.sustained && v.channel as usize == ch) {
                    v.dyn_target = x;
                }
            }
            3 => {
                c.technique = value;
                for v in self.voices.iter_mut().filter(|v| v.active && !v.releasing && v.channel as usize == ch) {
                    v.set_technique(value);
                }
            }
            7 => c.volume = x,
            11 => c.expression = x,
            9 => c.articulation = value,
            68 => c.legato = value >= 64,
            PLAYERS_CC => c.players = value.min(voice::MAX_SECTION as u8),
            120 | 123 => {
                for v in self.voices.iter_mut().filter(|v| v.active && v.channel as usize == ch) {
                    if cc == 120 {
                        v.active = false;
                    } else {
                        v.release();
                    }
                }
            }
            121 => {
                c.dynamic = None;
                c.technique = 0;
                c.articulation = 0;
                c.down_bow = true;
                c.legato = false;
                c.expression = 1.0;
                c.bend = 0.0;
            }
            _ => {}
        }
    }

    fn program(&mut self, ch: usize, program: u8) {
        let c = &mut self.channels[ch];
        c.program = program & 0x7f;
        let bank = ((c.bank_msb as u16) << 7) | c.bank_lsb as u16;
        c.patch = patches::patch_for(c.program, bank);
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
                    let ch = (m.data[0] & 0x0F) as usize;
                    match (status, vel) {
                        (0x90, v) if v > 0 => self.note_on(-1, m.port_index as i16, ch as i16, key, v as f64 / 127.0),
                        (0x80, _) | (0x90, _) => self.note_off(-1, ch as i16, key),
                        (0xB0, v) => self.control(ch, key as u8, v),
                        (0xE0, msb) => {
                            let value = (key as u16 & 0x7f) | ((msb as u16 & 0x7f) << 7);
                            self.channels[ch].bend = (value as f32 - 8192.0) / 8192.0 * 2.0;
                        }
                        (0xC0, _) => self.program(ch, key as u8),
                        _ => {}
                    }
                }
                _ => {}
            }
        }
    }

    /// Renders frames into the output buffers (mono, both sides).
    fn render(&mut self, left: &mut [f32], right: &mut [f32], out: *const clap_output_events, time: u32) {
        let n = left.len();
        if self.scratch.len() < n {
            self.scratch.resize(n, 0.0);
        }
        let trims = self.trims;
        for v in self.voices.iter_mut().filter(|v| v.active) {
            let c = self.channels[(v.channel & 15).max(0) as usize];
            let buf = &mut self.scratch[..n];
            buf.fill(0.0);
            let alive = v.render_with(buf, c.bend, trims, &mut self.pool);
            let g = self.gain * c.expression * c.volume;
            for (l, x) in left.iter_mut().zip(buf.iter()) {
                *l += x * g;
            }
            if !alive && !out.is_null() {
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
        right.copy_from_slice(left);
    }
}

// ------------------------------------------------------------ plugin calls --
unsafe extern "C" fn init(_p: *const clap_plugin) -> bool {
    true
}
unsafe extern "C" fn destroy(p: *const clap_plugin) {
    drop(Box::from_raw((*p).plugin_data as *mut Synth));
}
unsafe extern "C" fn activate(p: *const clap_plugin, sr: f64, _min: u32, max: u32) -> bool {
    let s = Synth::from(p);
    s.sample_rate = sr;
    s.new_voices();
    // The largest block, so the audio thread never allocates.
    s.scratch = vec![0.0; (max as usize).max(1024)];
    true
}
unsafe extern "C" fn deactivate(_p: *const clap_plugin) {}
unsafe extern "C" fn start_processing(_p: *const clap_plugin) -> bool {
    true
}
unsafe extern "C" fn stop_processing(_p: *const clap_plugin) {}
unsafe extern "C" fn reset(p: *const clap_plugin) {
    let s = Synth::from(p);
    for v in s.voices.iter_mut() {
        v.active = false;
    }
    for c in s.channels.iter_mut() {
        c.bend = 0.0;
        c.dynamic = None;
        c.legato = false;
        c.technique = 0;
        c.articulation = 0;
        c.down_bow = true;
    }
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
    for c in &s.channels {
        bytes.extend_from_slice(&[c.program, c.bank_msb, c.bank_lsb]);
    }
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
    // Corinzia Sine's state ("CZS1": its gain) is taken too.
    if bytes.len() < 8 || (&bytes[..4] != STATE_MAGIC && &bytes[..4] != b"CZS1") {
        return false;
    }
    let s = Synth::from(p);
    s.gain = f32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    if &bytes[..4] == STATE_MAGIC {
        for (ch, c) in bytes[8..].chunks_exact(3).take(16).enumerate() {
            s.channels[ch].bank_msb = c[1] & 0x7f;
            s.channels[ch].bank_lsb = c[2] & 0x7f;
            s.program(ch, c[0]);
        }
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
    7
}
unsafe extern "C" fn factory_descriptor(_f: *const clap_plugin_factory, index: u32) -> *const clap_plugin_descriptor {
    match index {
        0 => &DESCRIPTOR.0,
        1 => &effects::GAIN_DESCRIPTOR.0,
        2 => &effects::REVERB_DESCRIPTOR.0,
        3 => &effects::EQ_DESCRIPTOR.0,
        4 => &effects::LIMITER_DESCRIPTOR.0,
        5 => &effects::COMPRESSOR_DESCRIPTOR.0,
        6 => &effects::GATE_DESCRIPTOR.0,
        _ => ptr::null(),
    }
}
unsafe extern "C" fn factory_create(_f: *const clap_plugin_factory, host: *const clap_host, id: *const c_char) -> *const clap_plugin {
    if id.is_null() {
        return ptr::null();
    }
    let wanted = CStr::from_ptr(id);
    if wanted != PLUGIN_ID && wanted != OLD_PLUGIN_ID {
        return effects::create(host, wanted);
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
        channels: [Channel::default(); 16],
        voices: Vec::new(),
        pool: voice::PlayerPool::empty(),
        clock: 0,
        trims: {
            // Made here, on the main thread, never on the audio thread.
            for b in 0..body::BODIES as u8 {
                body::ready(b, false);
            }
            patches::trims()
        },
        scratch: Vec::new(),
    }));
    (*synth).new_voices();
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

#[cfg(test)]
mod tests {
    use super::*;

    fn synth() -> &'static mut Synth {
        unsafe { Synth::from(factory_create(ptr::null(), ptr::null(), PLUGIN_ID.as_ptr())) }
    }

    /// A violin section of `players` (or the solo violin) on channel 0.
    fn violins(s: &mut Synth, section: bool, players: u8) {
        if section {
            s.control(0, 0, (patches::SECTION_BANK >> 7) as u8);
            s.control(0, 32, (patches::SECTION_BANK & 0x7f) as u8);
        }
        s.program(0, 40);
        s.control(0, PLAYERS_CC, players);
    }

    fn render(s: &mut Synth, seconds: f32) -> Vec<f32> {
        let n = (s.sample_rate as f32 * seconds) as usize;
        let (mut l, mut r) = (vec![0.0f32; n], vec![0.0f32; n]);
        for (lc, rc) in l.chunks_mut(64).zip(r.chunks_mut(64)) {
            s.render(lc, rc, ptr::null(), 0);
        }
        l
    }

    fn db(x: &[f32]) -> f32 {
        10.0 * (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).max(1e-20).log10()
    }

    #[test]
    fn a_section_plays_with_as_many_players_as_it_has() {
        // Over several notes: one note's sum of a few players is a throw of
        // the dice (their tuning and timing are each their own).
        let mut levels = Vec::new();
        for players in [4u8, 14] {
            let mut power = 0.0;
            for key in [62i16, 64, 66, 67, 69, 71, 72, 74] {
                let s = synth();
                violins(s, true, players);
                s.note_on(1, 0, 0, key, 0.6);
                let v = s.voices.iter().find(|v| v.active).unwrap();
                assert_eq!(v.players().len(), players as usize, "physically modelled players");
                let x = render(s, 0.8);
                power += 10f32.powf(db(&x[x.len() / 2..]) / 10.0);
            }
            levels.push(10.0 * power.log10());
        }
        // More players, louder: as their number, not in step with it.
        let more = levels[1] - levels[0];
        let expected = 10.0 * (14.0f32 / 4.0).log10();
        assert!((more - expected).abs() < 2.0, "14 players against 4: {more:+.1} dB (about {expected:+.1})");
    }

    #[test]
    fn with_every_player_busy_the_oldest_ending_notes_give_theirs_up() {
        let s = synth();
        violins(s, true, 32);
        // Four notes of 32 players fill the pool; the first two end.
        for (i, key) in [57i16, 62, 67, 71].into_iter().enumerate() {
            s.note_on(i as i32, 0, 0, key, 0.6);
        }
        s.note_off(0, 0, 57);
        s.note_off(1, 0, 62);
        render(s, 0.05);
        // A fifth: the oldest ending note gives way, the held ones sound on.
        s.note_on(4, 0, 0, 76, 0.6);
        let playing: Vec<&Voice> = s.voices.iter().filter(|v| v.active).collect();
        assert!(playing.iter().all(|v| v.physical && !v.players().is_empty()), "every note on its players");
        assert!(playing.iter().any(|v| v.key == 76) && playing.iter().any(|v| v.key == 67) && playing.iter().any(|v| v.key == 71));
        assert!(!playing.iter().any(|v| v.key == 57), "the oldest ending note gave way");
        assert!(db(&render(s, 0.3)) > -80.0);
    }
}

#[cfg(test)]
mod techniques {
    use super::*;
    use voice::tech;

    fn synth() -> &'static mut Synth {
        unsafe { Synth::from(factory_create(ptr::null(), ptr::null(), PLUGIN_ID.as_ptr())) }
    }

    fn section(s: &mut Synth) {
        s.control(0, 0, (patches::SECTION_BANK >> 7) as u8);
        s.control(0, 32, (patches::SECTION_BANK & 0x7f) as u8);
        s.program(0, 40);
        s.control(0, PLAYERS_CC, 8);
        s.control(0, 1, 79);
    }

    fn render(s: &mut Synth, seconds: f32) -> Vec<f32> {
        let n = (s.sample_rate as f32 * seconds) as usize;
        let (mut l, mut r) = (vec![0.0f32; n], vec![0.0f32; n]);
        for (lc, rc) in l.chunks_mut(64).zip(r.chunks_mut(64)) {
            s.render(lc, rc, ptr::null(), 0);
        }
        l
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
    }

    /// The level every 5 ms.
    fn envelope(x: &[f32]) -> Vec<f32> {
        x.chunks(240).map(rms).collect()
    }

    #[test]
    fn arco_pizz_tremolo_and_back_each_on_the_players_in_their_technique() {
        let s = synth();
        section(s);
        let mut out = Vec::new();
        for (i, (t, name)) in [(tech::ORD, "arco"), (tech::PIZZ, "pizz."), (tech::TREM, "trem."), (tech::COL_LEGNO, "col legno"), (tech::HARMONICS, "harmonics"), (tech::ORD, "arco")].into_iter().enumerate() {
            s.control(0, 3, t);
            s.note_on(i as i32, 0, 0, 69, 0.62);
            let v = s.voices.iter().filter(|v| v.active && !v.releasing).max_by_key(|v| v.age).unwrap();
            assert!(v.physical && v.players().len() == 8, "{name}: on its players");
            assert_eq!(v.technique(), t, "{name}");
            assert_eq!(v.plucked(), matches!(t, tech::PIZZ | tech::COL_LEGNO), "{name}: plucked or bowed");
            let x = render(s, 0.6);
            s.note_off(i as i32, 0, 69);
            assert!(rms(&x) > 1e-4, "{name}: sounds");
            out.extend(x);
        }
        // No click at any change: no 5 ms step jumps out of its neighbours.
        let d: Vec<f32> = out.windows(2).map(|w| (w[1] - w[0]).abs()).collect();
        let typical = rms(&d);
        assert!(d.iter().all(|&x| x < 25.0 * typical), "a click: {} against {typical}", d.iter().fold(0.0f32, |m, &x| m.max(x)));
    }

    #[test]
    fn a_pizzicato_dies_away_and_a_bowed_note_holds() {
        for (t, holds) in [(tech::ORD, true), (tech::PIZZ, false)] {
            let s = synth();
            section(s);
            s.control(0, 3, t);
            s.note_on(1, 0, 0, 62, 0.62);
            let e = envelope(&render(s, 1.5));
            let (early, late) = (e[10..30].iter().cloned().fold(0.0, f32::max), e[250..290].iter().sum::<f32>() / 40.0);
            if holds {
                assert!(late > 0.5 * early, "arco holds: {late} against {early}");
            } else {
                assert!(late < 0.3 * early, "pizz. dies away: {late} against {early}");
            }
        }
    }

    #[test]
    fn a_tremolo_comes_in_quick_strokes() {
        let s = synth();
        s.program(0, 40); // one player: the strokes are clear
        s.control(0, 1, 79);
        s.control(0, 3, tech::TREM);
        s.note_on(1, 0, 0, 69, 0.62);
        let e = envelope(&render(s, 2.0));
        // Strokes: the level dips at each change of bow; count them over a second.
        let e = &e[100..300];
        let mean = e.iter().sum::<f32>() / e.len() as f32;
        let dips = e.windows(2).filter(|w| w[0] >= 0.7 * mean && w[1] < 0.7 * mean).count();
        assert!((8..=16).contains(&dips), "{dips} strokes a second");
        // Arco on the same pitch: steady.
        let s = synth();
        s.program(0, 40);
        s.control(0, 1, 79);
        s.note_on(1, 0, 0, 69, 0.62);
        let e = envelope(&render(s, 2.0));
        let e = &e[100..300];
        let mean = e.iter().sum::<f32>() / e.len() as f32;
        assert!(e.windows(2).filter(|w| w[0] >= 0.7 * mean && w[1] < 0.7 * mean).count() <= 2, "arco is steady");
    }

    #[test]
    fn a_new_technique_leaves_an_ending_note_as_it_was() {
        let s = synth();
        section(s);
        s.note_on(1, 0, 0, 69, 0.62);
        render(s, 0.3);
        s.note_off(1, 0, 69);
        s.control(0, 3, tech::TREM);
        s.note_on(2, 0, 0, 72, 0.62);
        let ending = s.voices.iter().find(|v| v.active && v.releasing).unwrap();
        assert_eq!(ending.technique(), tech::ORD, "its tail is not tremolo");
        let new = s.voices.iter().find(|v| v.active && !v.releasing).unwrap();
        assert_eq!(new.technique(), tech::TREM);
    }
}
