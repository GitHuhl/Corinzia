//! Corinzia's built-in effects, in the same CLAP library as the sine:
//! Corinzia Gain (level and pan), Corinzia Reverb (a Schroeder–Moorer
//! room: parallel damped combs into allpasses, per channel), Corinzia EQ
//! (low cut, low shelf, two bells, high shelf, high cut: RBJ biquads) and
//! Corinzia Limiter (stereo-linked, 1 ms look-ahead: never over its
//! ceiling). All are stereo in, stereo out, with parameters (automatable)
//! and state.

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
pub const EQ_ID: &CStr = c"org.corinzia.eq";
pub const LIMITER_ID: &CStr = c"org.corinzia.limiter";

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
pub static EQ_DESCRIPTOR: Sync<clap_plugin_descriptor> = Sync(descriptor(EQ_ID, c"Corinzia EQ", c"Low and high cut, shelves and two bells"));
pub static LIMITER_DESCRIPTOR: Sync<clap_plugin_descriptor> = Sync(descriptor(LIMITER_ID, c"Corinzia Limiter", c"A look-ahead peak limiter"));

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

const EQ_PARAMS: [Param; 12] = [
    Param { name: "Low cut", min: 20.0, max: 1000.0, default: 20.0, unit: "Hz" },
    Param { name: "Low gain", min: -18.0, max: 18.0, default: 0.0, unit: "dB" },
    Param { name: "Low freq", min: 30.0, max: 1000.0, default: 100.0, unit: "Hz" },
    Param { name: "Mid 1 gain", min: -18.0, max: 18.0, default: 0.0, unit: "dB" },
    Param { name: "Mid 1 freq", min: 60.0, max: 10000.0, default: 400.0, unit: "Hz" },
    Param { name: "Mid 1 Q", min: 0.2, max: 10.0, default: 1.0, unit: "" },
    Param { name: "Mid 2 gain", min: -18.0, max: 18.0, default: 0.0, unit: "dB" },
    Param { name: "Mid 2 freq", min: 150.0, max: 18000.0, default: 2500.0, unit: "Hz" },
    Param { name: "Mid 2 Q", min: 0.2, max: 10.0, default: 1.0, unit: "" },
    Param { name: "High gain", min: -18.0, max: 18.0, default: 0.0, unit: "dB" },
    Param { name: "High freq", min: 1000.0, max: 18000.0, default: 8000.0, unit: "Hz" },
    Param { name: "High cut", min: 1000.0, max: 20000.0, default: 20000.0, unit: "Hz" },
];
const LIMITER_PARAMS: [Param; 3] = [
    Param { name: "Input", min: 0.0, max: 24.0, default: 0.0, unit: "dB" },
    Param { name: "Ceiling", min: -24.0, max: 0.0, default: -0.3, unit: "dB" },
    Param { name: "Release", min: 1.0, max: 1000.0, default: 80.0, unit: "ms" },
];

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Gain,
    Reverb,
    Eq,
    Limiter,
}

/// A biquad filter (transposed direct form II), coefficients normalised.
#[derive(Clone, Copy, Default)]
struct Biquad {
    b: [f32; 3],
    a: [f32; 2],
    z: [f32; 2],
}

/// The RBJ cookbook's filter shapes.
#[derive(Clone, Copy)]
enum Shape {
    HighPass,
    LowPass,
    LowShelf,
    HighShelf,
    Bell,
}

impl Biquad {
    /// Sets the shape; the state carries over (no click on a change).
    fn set(&mut self, shape: Shape, freq: f64, q: f64, gain_db: f64, sr: f64) {
        use std::f64::consts::PI;
        let f = freq.clamp(10.0, sr * 0.49);
        let w = 2.0 * PI * f / sr;
        let (s, c) = w.sin_cos();
        let a = 10f64.powf(gain_db / 40.0);
        let alpha = s / (2.0 * q.max(0.05));
        let (b0, b1, b2, a0, a1, a2) = match shape {
            Shape::HighPass => ((1.0 + c) / 2.0, -(1.0 + c), (1.0 + c) / 2.0, 1.0 + alpha, -2.0 * c, 1.0 - alpha),
            Shape::LowPass => ((1.0 - c) / 2.0, 1.0 - c, (1.0 - c) / 2.0, 1.0 + alpha, -2.0 * c, 1.0 - alpha),
            Shape::Bell => (1.0 + alpha * a, -2.0 * c, 1.0 - alpha * a, 1.0 + alpha / a, -2.0 * c, 1.0 - alpha / a),
            Shape::LowShelf | Shape::HighShelf => {
                let k = 2.0 * a.sqrt() * alpha;
                let sign = if matches!(shape, Shape::LowShelf) { 1.0 } else { -1.0 };
                let (p, m) = (a + 1.0, a - 1.0);
                if sign > 0.0 {
                    (a * (p - m * c + k), 2.0 * a * (m - p * c), a * (p - m * c - k), p + m * c + k, -2.0 * (m + p * c), p + m * c - k)
                } else {
                    (a * (p + m * c + k), -2.0 * a * (m + p * c), a * (p + m * c - k), p - m * c + k, 2.0 * (m - p * c), p - m * c - k)
                }
            }
        };
        self.b = [(b0 / a0) as f32, (b1 / a0) as f32, (b2 / a0) as f32];
        self.a = [(a1 / a0) as f32, (a2 / a0) as f32];
    }
    fn tick(&mut self, x: f32) -> f32 {
        let y = self.b[0] * x + self.z[0];
        self.z[0] = self.b[1] * x - self.a[0] * y + self.z[1];
        self.z[1] = self.b[2] * x - self.a[1] * y;
        y
    }
}

/// The limiter's look-ahead: the input delayed, the gains each sample
/// needs, their minimum over the look-ahead smoothed by a moving average
/// (so the gain is down by the time a peak comes out, without a click),
/// then released slowly.
#[derive(Default)]
struct Limiter {
    delay: [Vec<f32>; 2],
    need: Vec<f32>,
    mins: Vec<f32>,
    sum: f64,
    at: usize,
    env: f32,
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
    sample_rate: f64,
    /// The EQ's six filters per channel, and the values they were set for.
    eq: [[Biquad; 6]; 2],
    eq_for: Vec<f64>,
    limiter: Limiter,
}

impl Effect {
    unsafe fn from<'a>(p: *const clap_plugin) -> &'a mut Effect {
        &mut *((*p).plugin_data as *mut Effect)
    }
    fn params(&self) -> &'static [Param] {
        match self.kind {
            Kind::Gain => &GAIN_PARAMS,
            Kind::Reverb => &REVERB_PARAMS,
            Kind::Eq => &EQ_PARAMS,
            Kind::Limiter => &LIMITER_PARAMS,
        }
    }
    /// The EQ's filters for the current values (when they changed).
    fn tune_eq(&mut self) {
        let v: Vec<f64> = (0..EQ_PARAMS.len()).map(|i| self.get(i)).collect();
        if v == self.eq_for {
            return;
        }
        let sr = self.sample_rate;
        for ch in 0..2 {
            let f = &mut self.eq[ch];
            f[0].set(Shape::HighPass, v[0], std::f64::consts::FRAC_1_SQRT_2, 0.0, sr);
            f[1].set(Shape::LowShelf, v[2], std::f64::consts::FRAC_1_SQRT_2, v[1], sr);
            f[2].set(Shape::Bell, v[4], v[5], v[3], sr);
            f[3].set(Shape::Bell, v[7], v[8], v[6], sr);
            f[4].set(Shape::HighShelf, v[10], std::f64::consts::FRAC_1_SQRT_2, v[9], sr);
            f[5].set(Shape::LowPass, v[11], std::f64::consts::FRAC_1_SQRT_2, 0.0, sr);
        }
        self.eq_for = v;
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
        self.sample_rate = sample_rate;
        self.eq = Default::default();
        self.eq_for.clear();
        let look = ((sample_rate * 0.001).round() as usize).max(1);
        self.limiter = Limiter { delay: [vec![0.0; look], vec![0.0; look]], need: vec![1.0; look], mins: vec![1.0; look], sum: look as f64, at: 0, env: 1.0 };
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
            Kind::Eq => {
                self.tune_eq();
                // Off at the ends of their ranges: the cuts.
                let (low_cut, high_cut) = (self.get(0) > 20.5, self.get(11) < 19999.0);
                for (ch, (o, x)) in [(&mut *ol, inp[0]), (&mut *or, inp[1])].into_iter().enumerate() {
                    let f = &mut self.eq[ch];
                    for (y, &v) in o.iter_mut().zip(x) {
                        let mut s = v;
                        if low_cut {
                            s = f[0].tick(s);
                        }
                        for band in &mut f[1..5] {
                            s = band.tick(s);
                        }
                        if high_cut {
                            s = f[5].tick(s);
                        }
                        *y = s;
                    }
                }
            }
            Kind::Limiter => {
                let input = 10f32.powf(self.get(0) as f32 / 20.0);
                let ceiling = 10f32.powf(self.get(1) as f32 / 20.0);
                let release = (-1.0 / (self.get(2) * 0.001 * self.sample_rate).max(1.0)).exp() as f32;
                let l = &mut self.limiter;
                let n = l.need.len();
                for i in 0..ol.len() {
                    // The sample leaving the delay now, and the gain it needs.
                    let (d0, d1, out_need) = (l.delay[0][l.at], l.delay[1][l.at], l.need[l.at]);
                    let (x0, x1) = (inp[0][i] * input, inp[1][i] * input);
                    let peak = x0.abs().max(x1.abs());
                    (l.delay[0][l.at], l.delay[1][l.at]) = (x0, x1);
                    l.need[l.at] = if peak > ceiling { ceiling / peak } else { 1.0 };
                    // The least gain any sample in the look-ahead needs,
                    // averaged over the look-ahead: by the time a peak comes
                    // out the gain is down to it, having slid there.
                    let m = l.need.iter().copied().fold(1.0f32, f32::min);
                    l.sum += (m - l.mins[l.at]) as f64;
                    l.mins[l.at] = m;
                    let smooth = (l.sum / n as f64) as f32;
                    // Released slowly; never above what the outgoing sample needs.
                    l.env = if smooth < l.env { smooth } else { smooth + (l.env - smooth) * release };
                    let g = l.env.min(out_need);
                    ol[i] = (d0 * g).clamp(-ceiling, ceiling);
                    or[i] = (d1 * g).clamp(-ceiling, ceiling);
                    l.at = (l.at + 1) % n;
                }
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
        e.eq[ch].iter_mut().for_each(|f| f.z = [0.0; 2]);
    }
    let sr = e.sample_rate;
    e.build(sr);
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
    let (kind, desc, params): (Kind, _, &[Param]) = if id == GAIN_ID {
        (Kind::Gain, &GAIN_DESCRIPTOR.0, &GAIN_PARAMS)
    } else if id == REVERB_ID {
        (Kind::Reverb, &REVERB_DESCRIPTOR.0, &REVERB_PARAMS)
    } else if id == EQ_ID {
        (Kind::Eq, &EQ_DESCRIPTOR.0, &EQ_PARAMS)
    } else if id == LIMITER_ID {
        (Kind::Limiter, &LIMITER_DESCRIPTOR.0, &LIMITER_PARAMS)
    } else {
        return ptr::null();
    };
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
        sample_rate: 48000.0,
        eq: Default::default(),
        eq_for: vec![],
        limiter: Limiter::default(),
    }));
    (*e).plugin.plugin_data = e as *mut c_void;
    (*e).build(48000.0);
    &(*e).plugin
}

// The in-place pair id above names the input port (0): unused otherwise.
const _: clap_id = CLAP_INVALID_ID;

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f64 = 48000.0;

    /// A fresh effect, its parameters set.
    fn effect(id: &CStr, params: &[(usize, f64)]) -> &'static mut Effect {
        unsafe {
            let p = create(ptr::null(), id);
            let e = Effect::from(p);
            e.build(SR);
            for &(i, v) in params {
                e.set(i, v);
            }
            e
        }
    }

    fn sine(freq: f64, amp: f32, n: usize) -> Vec<f32> {
        (0..n).map(|i| (i as f64 * freq / SR * std::f64::consts::TAU).sin() as f32 * amp).collect()
    }

    /// Runs a signal through (both channels) and gives the left out.
    fn through(e: &mut Effect, x: &[f32]) -> Vec<f32> {
        let (mut l, mut r) = (vec![0.0; x.len()], vec![0.0; x.len()]);
        for (k, chunk) in x.chunks(256).enumerate() {
            let at = k * 256;
            e.run([chunk, chunk], [&mut l[at..at + chunk.len()], &mut r[at..at + chunk.len()]]);
        }
        assert_eq!(l, r);
        l
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
    }

    /// Gain at a frequency, in dB (steady state).
    fn gain_db(e: &mut Effect, freq: f64) -> f32 {
        let x = sine(freq, 0.5, 24000);
        let y = through(e, &x);
        20.0 * (rms(&y[12000..]) / rms(&x[12000..])).log10()
    }

    #[test]
    fn the_eq_is_flat_until_told_otherwise() {
        let e = effect(EQ_ID, &[]);
        for f in [50.0, 400.0, 2500.0, 12000.0] {
            assert!(gain_db(e, f).abs() < 0.05, "{f} Hz");
        }
    }

    #[test]
    fn the_eq_cuts_shelves_and_bells() {
        // Low cut at 400 Hz: 100 Hz well down, 5 kHz untouched.
        let e = effect(EQ_ID, &[(0, 400.0)]);
        assert!(gain_db(e, 100.0) < -20.0);
        assert!(gain_db(e, 5000.0).abs() < 0.1);
        // A bell of +12 dB at 1 kHz: +12 there, little an octave and more away.
        let e = effect(EQ_ID, &[(3, 12.0), (4, 1000.0), (5, 2.0)]);
        assert!((gain_db(e, 1000.0) - 12.0).abs() < 0.2);
        assert!(gain_db(e, 250.0) < 1.5);
        // A low shelf of -6 dB under 200 Hz.
        let e = effect(EQ_ID, &[(1, -6.0), (2, 200.0)]);
        assert!((gain_db(e, 40.0) + 6.0).abs() < 0.3);
        assert!(gain_db(e, 4000.0).abs() < 0.2);
        // High cut at 2 kHz.
        let e = effect(EQ_ID, &[(11, 2000.0)]);
        assert!(gain_db(e, 10000.0) < -20.0);
    }

    #[test]
    fn the_limiter_never_passes_its_ceiling_and_leaves_quiet_sound_alone() {
        // A loud tone (0 dBFS peaks) under a -6 dB ceiling.
        let e = effect(LIMITER_ID, &[(1, -6.0)]);
        let x = sine(440.0, 1.0, 48000);
        let y = through(e, &x);
        let ceiling = 10f32.powf(-6.0 / 20.0);
        assert!(y.iter().all(|v| v.abs() <= ceiling + 1e-6));
        let peak = y[24000..].iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(peak > ceiling * 0.95, "close to the ceiling: {peak}");
        // A sudden burst: still under, from its first sample.
        let mut burst = vec![0.0; 4800];
        burst.extend(sine(1000.0, 4.0, 4800));
        let e = effect(LIMITER_ID, &[(1, -1.0)]);
        let y = through(e, &burst);
        assert!(y.iter().all(|v| v.abs() <= 10f32.powf(-1.0 / 20.0) + 1e-6));
        // Quiet sound comes out as it went in, a millisecond late.
        let e = effect(LIMITER_ID, &[]);
        let x = sine(440.0, 0.25, 4800);
        let y = through(e, &x);
        let late = (SR * 0.001).round() as usize;
        assert!(x[..x.len() - late].iter().zip(&y[late..]).all(|(a, b)| (a - b).abs() < 1e-6));
    }
}
