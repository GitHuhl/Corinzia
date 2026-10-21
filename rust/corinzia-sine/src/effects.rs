//! Corinzia's built-in effects, in the same CLAP library as the sine:
//! Corinzia Gain (level and pan) and Corinzia Reverb (a Schroeder–Moorer
//! room: parallel damped combs into allpasses, per channel). Both are
//! stereo in, stereo out, with parameters (automatable) and state.

use crate::{read_all, write_all, write_name, Sync};
use clap_sys::events::*;
use clap_sys::ext::audio_ports::*;
use clap_sys::ext::params::*;
use clap_sys::ext::state::*;
use clap_sys::host::clap_host;
use clap_sys::id::{clap_id, CLAP_INVALID_ID};
use clap_sys::plugin::*;
use clap_sys::process::*;
use clap_sys::stream::*;
use clap_sys::version::CLAP_VERSION;
use std::ffi::{c_char, c_void, CStr};
use std::ptr;
use std::sync::atomic::{AtomicU64, Ordering};

pub const GAIN_ID: &CStr = c"org.corinzia.gain";
pub const REVERB_ID: &CStr = c"org.corinzia.reverb";

static FX_FEATURES: Sync<[*const c_char; 3]> = Sync([c"audio-effect".as_ptr(), c"stereo".as_ptr(), ptr::null()]);

const fn descriptor(id: &'static CStr, name: &'static CStr, description: &'static CStr) -> clap_plugin_descriptor {
    clap_plugin_descriptor {
        clap_version: CLAP_VERSION,
        id: id.as_ptr(),
        name: name.as_ptr(),
        vendor: c"Corinzia".as_ptr(),
        url: c"".as_ptr(),
        manual_url: c"".as_ptr(),
        support_url: c"".as_ptr(),
        version: c"0.1.0".as_ptr(),
        description: description.as_ptr(),
        features: &FX_FEATURES.0 as *const _ as *const *const c_char,
    }
}

pub static GAIN_DESCRIPTOR: Sync<clap_plugin_descriptor> = Sync(descriptor(GAIN_ID, c"Corinzia Gain", c"Level and pan"));
pub static REVERB_DESCRIPTOR: Sync<clap_plugin_descriptor> = Sync(descriptor(REVERB_ID, c"Corinzia Reverb", c"A small room reverb"));

/// A parameter: (name, min, max, default, unit).
struct Param {
    name: &'static str,
    min: f64,
    max: f64,
    default: f64,
    unit: &'static str,
}

const GAIN_PARAMS: [Param; 2] = [Param { name: "Gain", min: -60.0, max: 12.0, default: 0.0, unit: "dB" }, Param { name: "Pan", min: -1.0, max: 1.0, default: 0.0, unit: "" }];
const REVERB_PARAMS: [Param; 3] = [
    Param { name: "Mix", min: 0.0, max: 1.0, default: 0.25, unit: "" },
    Param { name: "Size", min: 0.0, max: 1.0, default: 0.6, unit: "" },
    Param { name: "Damping", min: 0.0, max: 1.0, default: 0.5, unit: "" },
];

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Gain,
    Reverb,
}

const COMBS: [usize; 4] = [1116, 1188, 1277, 1356];
const ALLPASSES: [usize; 2] = [556, 441];
const SPREAD: usize = 23;

struct Comb {
    buf: Vec<f32>,
    at: usize,
    store: f32,
}
impl Comb {
    fn tick(&mut self, x: f32, feedback: f32, damp: f32) -> f32 {
        let y = self.buf[self.at];
        self.store = y * (1.0 - damp) + self.store * damp;
        self.buf[self.at] = x + self.store * feedback;
        self.at = (self.at + 1) % self.buf.len();
        y
    }
}
struct Allpass {
    buf: Vec<f32>,
    at: usize,
}
impl Allpass {
    fn tick(&mut self, x: f32) -> f32 {
        let b = self.buf[self.at];
        let y = b - x;
        self.buf[self.at] = x + b * 0.5;
        self.at = (self.at + 1) % self.buf.len();
        y
    }
}

pub struct Effect {
    plugin: clap_plugin,
    kind: Kind,
    /// Values as f64 bits: the audio thread writes, the main thread reads.
    values: Vec<AtomicU64>,
    combs: [Vec<Comb>; 2],
    allpasses: [Vec<Allpass>; 2],
    /// The gains reached at the end of the last block (smoothing).
    applied: (f32, f32),
    /// The input, copied (it may share memory with the output); sized on activation.
    scratch: [Vec<f32>; 2],
}

impl Effect {
    unsafe fn from<'a>(p: *const clap_plugin) -> &'a mut Effect {
        &mut *((*p).plugin_data as *mut Effect)
    }
    fn params(&self) -> &'static [Param] {
        match self.kind {
            Kind::Gain => &GAIN_PARAMS,
            Kind::Reverb => &REVERB_PARAMS,
        }
    }
    fn get(&self, i: usize) -> f64 {
        self.values.get(i).map_or(0.0, |v| f64::from_bits(v.load(Ordering::Relaxed)))
    }
    fn set(&self, i: usize, v: f64) {
        if let (Some(slot), Some(p)) = (self.values.get(i), self.params().get(i)) {
            slot.store(v.clamp(p.min, p.max).to_bits(), Ordering::Relaxed);
        }
    }
    fn build(&mut self, sample_rate: f64) {
        let scale = sample_rate / 44100.0;
        for ch in 0..2 {
            let off = ch * SPREAD;
            self.combs[ch] = COMBS.iter().map(|n| Comb { buf: vec![0.0; ((n + off) as f64 * scale) as usize + 1], at: 0, store: 0.0 }).collect();
            self.allpasses[ch] = ALLPASSES.iter().map(|n| Allpass { buf: vec![0.0; ((n + off) as f64 * scale) as usize + 1], at: 0 }).collect();
        }
    }
    fn run(&mut self, inp: [&[f32]; 2], out: [&mut [f32]; 2]) {
        let [ol, or] = out;
        match self.kind {
            Kind::Gain => {
                let db = self.get(0) as f32;
                let g = if db <= -60.0 { 0.0 } else { 10f32.powf(db / 20.0) };
                let angle = (self.get(1) as f32 + 1.0) * std::f32::consts::FRAC_PI_4;
                let (gl, gr) = (g * angle.cos() * std::f32::consts::SQRT_2, g * angle.sin() * std::f32::consts::SQRT_2);
                let (l0, r0) = self.applied;
                let n = ol.len().max(1) as f32;
                for i in 0..ol.len() {
                    let k = (i + 1) as f32 / n;
                    ol[i] = inp[0][i] * (l0 + (gl - l0) * k);
                    or[i] = inp[1][i] * (r0 + (gr - r0) * k);
                }
                self.applied = (gl, gr);
            }
            Kind::Reverb => {
                let (mix, size, damp) = (self.get(0) as f32, self.get(1) as f32, self.get(2) as f32 * 0.4);
                let feedback = 0.7 + 0.28 * size;
                for i in 0..ol.len() {
                    let x = (inp[0][i] + inp[1][i]) * 0.015;
                    #[allow(clippy::needless_range_loop)] // one comb bank per channel
                    for ch in 0..2 {
                        let mut acc = 0.0;
                        for c in self.combs[ch].iter_mut() {
                            acc += c.tick(x, feedback, damp);
                        }
                        for a in self.allpasses[ch].iter_mut() {
                            acc = a.tick(acc);
                        }
                        let dry = inp[ch][i];
                        let y = dry * (1.0 - mix) + acc * mix * 3.0;
                        if ch == 0 {
                            ol[i] = y;
                        } else {
                            or[i] = y;
                        }
                    }
                }
            }
        }
    }
    fn param_event(&mut self, h: *const clap_event_header) {
        unsafe {
            if (*h).space_id == CLAP_CORE_EVENT_SPACE_ID && (*h).type_ == CLAP_EVENT_PARAM_VALUE {
                let e = &*(h as *const clap_event_param_value);
                self.set(e.param_id as usize, e.value);
            }
        }
    }
}

unsafe extern "C" fn init(_p: *const clap_plugin) -> bool {
    true
}
unsafe extern "C" fn destroy(p: *const clap_plugin) {
    drop(Box::from_raw((*p).plugin_data as *mut Effect));
}
unsafe extern "C" fn activate(p: *const clap_plugin, sr: f64, _min: u32, max: u32) -> bool {
    let e = Effect::from(p);
    e.build(sr);
    e.scratch = [vec![0.0; max as usize], vec![0.0; max as usize]];
    true
}
unsafe extern "C" fn deactivate(_p: *const clap_plugin) {}
unsafe extern "C" fn start_processing(_p: *const clap_plugin) -> bool {
    true
}
unsafe extern "C" fn stop_processing(_p: *const clap_plugin) {}
unsafe extern "C" fn reset(p: *const clap_plugin) {
    let e = Effect::from(p);
    for ch in 0..2 {
        e.combs[ch].iter_mut().for_each(|c| c.buf.fill(0.0));
        e.allpasses[ch].iter_mut().for_each(|a| a.buf.fill(0.0));
    }
}

unsafe extern "C" fn process(p: *const clap_plugin, process: *const clap_process) -> clap_process_status {
    let e = Effect::from(p);
    let pr = &*process;
    if pr.audio_outputs_count == 0 || (*pr.audio_outputs).channel_count < 2 {
        return CLAP_PROCESS_ERROR;
    }
    let frames = pr.frames_count as usize;
    let outs = (*pr.audio_outputs).data32;
    let (ol, or) = (std::slice::from_raw_parts_mut(*outs, frames), std::slice::from_raw_parts_mut(*outs.add(1), frames));
    // Parameter changes apply at their sample.
    let events = pr.in_events;
    let count = if events.is_null() { 0 } else { (*events).size.map(|f| f(events)).unwrap_or(0) };
    let has_input = pr.audio_inputs_count > 0 && !pr.audio_inputs.is_null() && (*pr.audio_inputs).channel_count >= 1;
    let [mut il, mut ir] = std::mem::take(&mut e.scratch);
    if il.len() < frames || ir.len() < frames {
        e.scratch = [il, ir];
        return CLAP_PROCESS_ERROR; // more than the activation allowed
    }
    // Inputs may share memory with the outputs (in place): copy them first.
    for (ch, buf) in [&mut il, &mut ir].into_iter().enumerate() {
        if has_input {
            let ins = (*pr.audio_inputs).data32;
            let n = (*pr.audio_inputs).channel_count as usize;
            buf[..frames].copy_from_slice(std::slice::from_raw_parts(*ins.add(ch.min(n - 1)), frames));
        } else {
            buf[..frames].fill(0.0);
        }
    }
    let mut at = 0usize;
    for i in 0..count {
        let Some(h) = (*events).get.map(|g| g(events, i)).filter(|h| !h.is_null()) else { continue };
        let t = ((*h).time as usize).min(frames);
        if t > at {
            e.run([&il[at..t], &ir[at..t]], [&mut ol[at..t], &mut or[at..t]]);
            at = t;
        }
        e.param_event(h);
    }
    if at < frames {
        e.run([&il[at..frames], &ir[at..frames]], [&mut ol[at..], &mut or[at..]]);
    }
    e.scratch = [il, ir];
    CLAP_PROCESS_CONTINUE
}

unsafe extern "C" fn ports_count(_p: *const clap_plugin, _is_input: bool) -> u32 {
    1
}
unsafe extern "C" fn ports_get(_p: *const clap_plugin, index: u32, is_input: bool, info: *mut clap_audio_port_info) -> bool {
    if index != 0 {
        return false;
    }
    let i = &mut *info;
    i.id = 0;
    write_name(&mut i.name, if is_input { "Input" } else { "Output" });
    i.flags = CLAP_AUDIO_PORT_IS_MAIN;
    i.channel_count = 2;
    i.port_type = CLAP_PORT_STEREO.as_ptr();
    i.in_place_pair = 0;
    true
}
static PORTS: clap_plugin_audio_ports = clap_plugin_audio_ports { count: Some(ports_count), get: Some(ports_get) };

unsafe extern "C" fn params_count(p: *const clap_plugin) -> u32 {
    Effect::from(p).params().len() as u32
}
unsafe extern "C" fn params_info(p: *const clap_plugin, index: u32, info: *mut clap_param_info) -> bool {
    let Some(param) = Effect::from(p).params().get(index as usize) else { return false };
    let i = &mut *info;
    i.id = index as clap_id;
    i.flags = CLAP_PARAM_IS_AUTOMATABLE;
    i.cookie = ptr::null_mut();
    write_name(&mut i.name, param.name);
    write_name(&mut i.module, "");
    i.min_value = param.min;
    i.max_value = param.max;
    i.default_value = param.default;
    true
}
unsafe extern "C" fn params_value(p: *const clap_plugin, id: clap_id, out: *mut f64) -> bool {
    let e = Effect::from(p);
    if (id as usize) < e.params().len() {
        *out = e.get(id as usize);
        true
    } else {
        false
    }
}
unsafe extern "C" fn params_to_text(p: *const clap_plugin, id: clap_id, value: f64, buf: *mut c_char, cap: u32) -> bool {
    let Some(param) = Effect::from(p).params().get(id as usize) else { return false };
    let t = if param.unit.is_empty() { format!("{value:.2}") } else { format!("{value:.1} {}", param.unit) };
    let dst = std::slice::from_raw_parts_mut(buf, cap as usize);
    write_name(dst, &t);
    true
}
unsafe extern "C" fn params_from_text(p: *const clap_plugin, id: clap_id, text: *const c_char, out: *mut f64) -> bool {
    if (id as usize) >= Effect::from(p).params().len() {
        return false;
    }
    let t = CStr::from_ptr(text).to_string_lossy();
    match t.trim().trim_end_matches("dB").trim().parse::<f64>() {
        Ok(v) => {
            *out = v;
            true
        }
        Err(_) => false,
    }
}
unsafe extern "C" fn params_flush(p: *const clap_plugin, input: *const clap_input_events, _out: *const clap_output_events) {
    let e = Effect::from(p);
    let n = if input.is_null() { 0 } else { (*input).size.map(|f| f(input)).unwrap_or(0) };
    for i in 0..n {
        if let Some(h) = (*input).get.map(|g| g(input, i)).filter(|h| !h.is_null()) {
            e.param_event(h);
        }
    }
}
static PARAMS: clap_plugin_params = clap_plugin_params {
    count: Some(params_count),
    get_info: Some(params_info),
    get_value: Some(params_value),
    value_to_text: Some(params_to_text),
    text_to_value: Some(params_from_text),
    flush: Some(params_flush),
};

const MAGIC: &[u8; 4] = b"CZF1";
unsafe extern "C" fn state_save(p: *const clap_plugin, stream: *const clap_ostream) -> bool {
    let e = Effect::from(p);
    let mut bytes = MAGIC.to_vec();
    for i in 0..e.params().len() {
        bytes.extend_from_slice(&e.get(i).to_le_bytes());
    }
    write_all(stream, &bytes)
}
unsafe extern "C" fn state_load(p: *const clap_plugin, stream: *const clap_istream) -> bool {
    let Some(bytes) = read_all(stream) else { return false };
    if bytes.len() < 4 || &bytes[..4] != MAGIC {
        return false;
    }
    let e = Effect::from(p);
    for (i, chunk) in bytes[4..].chunks_exact(8).enumerate() {
        e.set(i, f64::from_le_bytes(chunk.try_into().unwrap()));
    }
    true
}
static STATE: clap_plugin_state = clap_plugin_state { save: Some(state_save), load: Some(state_load) };

unsafe extern "C" fn get_extension(_p: *const clap_plugin, id: *const c_char) -> *const c_void {
    let id = CStr::from_ptr(id);
    if id == CLAP_EXT_AUDIO_PORTS {
        &PORTS as *const _ as *const c_void
    } else if id == CLAP_EXT_PARAMS {
        &PARAMS as *const _ as *const c_void
    } else if id == CLAP_EXT_STATE {
        &STATE as *const _ as *const c_void
    } else {
        ptr::null()
    }
}
unsafe extern "C" fn on_main_thread(_p: *const clap_plugin) {}

/// A new instance of a built-in effect, or null for another id.
pub unsafe fn create(_host: *const clap_host, id: &CStr) -> *const clap_plugin {
    let (kind, desc) = if id == GAIN_ID {
        (Kind::Gain, &GAIN_DESCRIPTOR.0)
    } else if id == REVERB_ID {
        (Kind::Reverb, &REVERB_DESCRIPTOR.0)
    } else {
        return ptr::null();
    };
    let params: &[Param] = if kind == Kind::Gain { &GAIN_PARAMS } else { &REVERB_PARAMS };
    let e = Box::into_raw(Box::new(Effect {
        plugin: clap_plugin {
            desc,
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
        kind,
        values: params.iter().map(|p| AtomicU64::new(p.default.to_bits())).collect(),
        combs: [vec![], vec![]],
        allpasses: [vec![], vec![]],
        applied: (1.0, 1.0),
        scratch: [vec![], vec![]],
    }));
    (*e).plugin.plugin_data = e as *mut c_void;
    (*e).build(48000.0);
    &(*e).plugin
}

// The in-place pair id above names the input port (0): unused otherwise.
const _: clap_id = CLAP_INVALID_ID;
