//! VST3 hosting, through the `vst3` crate's bindings of the SDK headers.
//!
//! A [`Module`] is a loaded `.vst3` bundle and its plugin factory; an
//! [`Instance`] is one audio processor with its edit controller. The engine
//! speaks CLAP to every plugin, so an instance translates CLAP's process
//! call: note and MIDI events become VST3 events, the transport becomes a
//! `ProcessContext`, and the engine's stereo buffers are the main output bus.
//!
//! Parameters changed in the plugin's editor reach the controller's
//! `IComponentHandler` on the main thread; they are passed to the audio
//! thread through a wait-free queue and delivered to the processor as
//! `IParameterChanges` in the next block (VST3 keeps the two apart).
//!
//! Threads, as VST3 requires: creation, `initialize`, `setActive`, state,
//! the controller and the editor on the main thread; `setProcessing` and
//! `process` on the audio thread.

use crate::gui;
use crate::host::{Format, PluginInfo};
use clap_sys::events::*;
use clap_sys::fixedpoint::{CLAP_BEATTIME_FACTOR, CLAP_SECTIME_FACTOR};
use clap_sys::process::*;
use std::cell::{Cell, UnsafeCell};
use std::collections::HashMap;
use std::ffi::{c_char, c_void, CStr, CString};
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};
use std::sync::Mutex;
use vst3::Steinberg::Linux::*;
use vst3::Steinberg::Vst::*;
use vst3::Steinberg::*;
use vst3::{Class, ComPtr, ComRef, ComWrapper, Interface};

const AUDIO_MODULE_CLASS: &str = "Audio Module Class";
/// Parameter edits from the editor waiting for the audio thread.
const PARAM_QUEUE: usize = 4096;
/// Distinct parameters changed in one block.
const PARAMS_PER_BLOCK: usize = 128;
const EVENTS_PER_BLOCK: usize = 1024;

fn ok(r: tresult) -> bool {
    r == kResultOk
}

/// A NUL-terminated `char8` buffer as a string.
fn text(buf: &[c_char]) -> String {
    let bytes: Vec<u8> = buf.iter().take_while(|c| **c != 0).map(|c| *c as u8).collect();
    String::from_utf8_lossy(&bytes).trim().to_string()
}

/// A class id as 32 hex digits (as the SDK prints FUIDs).
pub fn class_id_hex(cid: &TUID) -> String {
    cid.iter().map(|b| format!("{:02X}", *b as u8)).collect()
}

fn class_id(hex: &str) -> Option<TUID> {
    if hex.len() != 32 || !hex.is_ascii() {
        return None;
    }
    let mut cid: TUID = [0; 16];
    for (i, c) in cid.iter_mut().enumerate() {
        *c = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).ok()? as c_char;
    }
    Some(cid)
}

fn iid<I: Interface>() -> *const c_char {
    I::IID.as_ptr() as *const c_char
}

fn same_iid<I: Interface>(id: *const TUID) -> bool {
    !id.is_null() && unsafe { std::slice::from_raw_parts(id as *const u8, 16) } == I::IID
}

fn copy_wide(src: &str, dst: &mut [TChar]) {
    let mut n = 0;
    let room = dst.len().saturating_sub(1);
    for (s, d) in src.encode_utf16().zip(dst.iter_mut().take(room)) {
        *d = s as TChar;
        n += 1;
    }
    if let Some(d) = dst.get_mut(n) {
        *d = 0;
    }
}

// ------------------------------------------------------------- module --

type ExitFn = unsafe extern "C" fn() -> bool;

/// A loaded `.vst3` module and its factory.
pub struct Module {
    factory: Option<ComPtr<IPluginFactory>>,
    exit: Option<ExitFn>,
    _lib: Option<libloading::Library>, // last: unloaded after the exit call
}

impl Drop for Module {
    fn drop(&mut self) {
        self.factory = None;
        if let Some(exit) = self.exit {
            unsafe { exit() };
        }
    }
}

/// The shared library inside a bundle (`Contents/<arch>-linux/<name>.so`,
/// `Contents/<arch>-win/<name>.vst3`), or the path itself for a
/// single-file module.
pub fn module_binary(bundle: &Path) -> PathBuf {
    if !bundle.is_dir() {
        return bundle.to_path_buf();
    }
    let stem = bundle.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let contents = bundle.join("Contents");
    if cfg!(windows) {
        let arch = match std::env::consts::ARCH {
            "aarch64" => "arm64-win",
            "x86" => "x86-win",
            _ => "x86_64-win",
        };
        contents.join(arch).join(format!("{stem}.vst3"))
    } else {
        contents.join(format!("{}-linux", std::env::consts::ARCH)).join(format!("{stem}.so"))
    }
}

impl Module {
    pub fn load(bundle: &Path) -> Result<Module, String> {
        let bin = module_binary(bundle);
        let err = |e: &dyn std::fmt::Display| format!("{}: {e}", bundle.display());
        unsafe {
            #[cfg(unix)]
            let (lib, entry_ok, exit) = {
                use libloading::os::unix::{Library, RTLD_LAZY, RTLD_LOCAL};
                let lib = Library::open(Some(&bin), RTLD_LAZY | RTLD_LOCAL).map_err(|e| err(&e))?;
                // ModuleEntry wants the dlopen handle.
                let handle = lib.into_raw();
                let lib = Library::from_raw(handle);
                let entry_ok = match lib.get::<unsafe extern "C" fn(*mut c_void) -> bool>(b"ModuleEntry\0") {
                    Ok(f) => f(handle),
                    Err(_) => true,
                };
                let exit = lib.get::<ExitFn>(b"ModuleExit\0").ok().map(|f| *f);
                (libloading::Library::from(lib), entry_ok, exit)
            };
            #[cfg(not(unix))]
            let (lib, entry_ok, exit) = {
                let lib = libloading::Library::new(&bin).map_err(|e| err(&e))?;
                let entry_ok = match lib.get::<unsafe extern "C" fn() -> bool>(b"InitDll\0") {
                    Ok(f) => f(),
                    Err(_) => true,
                };
                let exit = lib.get::<ExitFn>(b"ExitDll\0").ok().map(|f| *f);
                (lib, entry_ok, exit)
            };
            if !entry_ok {
                return Err(err(&"the module's entry failed"));
            }
            let get = match lib.get::<unsafe extern "system" fn() -> *mut IPluginFactory>(b"GetPluginFactory\0") {
                Ok(f) => *f,
                Err(_) => {
                    exit.map(|f| f());
                    return Err(err(&"not a VST3 module (no GetPluginFactory)"));
                }
            };
            let Some(factory) = ComPtr::from_raw(get()) else {
                exit.map(|f| f());
                return Err(err(&"the module has no plugin factory"));
            };
            Ok(Module { factory: Some(factory), exit, _lib: Some(lib) })
        }
    }

    /// A module around a factory already in memory (tests).
    pub fn from_factory(factory: ComPtr<IPluginFactory>) -> Module {
        Module { factory: Some(factory), exit: None, _lib: None }
    }

    fn factory(&self) -> &ComPtr<IPluginFactory> {
        self.factory.as_ref().expect("factory lives as long as the module")
    }

    /// The audio processors the module offers.
    pub fn classes(&self, bundle: &Path) -> Vec<PluginInfo> {
        let f = self.factory();
        let f2 = f.cast::<IPluginFactory2>();
        unsafe {
            let mut finfo: PFactoryInfo = std::mem::zeroed();
            let factory_vendor = if ok(f.getFactoryInfo(&mut finfo)) { text(&finfo.vendor) } else { String::new() };
            let mut out = Vec::new();
            for i in 0..f.countClasses().max(0) {
                let mut ci: PClassInfo = std::mem::zeroed();
                if !ok(f.getClassInfo(i, &mut ci)) || text(&ci.category) != AUDIO_MODULE_CLASS {
                    continue;
                }
                let mut info = PluginInfo { id: class_id_hex(&ci.cid), name: text(&ci.name), vendor: factory_vendor.clone(), version: String::new(), features: Vec::new(), bundle: bundle.to_path_buf(), format: Format::Vst3 };
                if let Some(f2) = &f2 {
                    let mut c2: PClassInfo2 = std::mem::zeroed();
                    if ok(f2.getClassInfo2(i, &mut c2)) {
                        if !text(&c2.vendor).is_empty() {
                            info.vendor = text(&c2.vendor);
                        }
                        info.version = text(&c2.version);
                        info.features = text(&c2.subCategories).split('|').map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty()).collect();
                    }
                }
                out.push(info);
            }
            out
        }
    }

    /// Main thread: an initialised instance of class `id` (hex).
    pub fn create(&self, id: &str, name: &str, owner: gui::Owner) -> Result<Instance, String> {
        let cid = class_id(id).ok_or_else(|| format!("{name}: bad class id {id}"))?;
        let f = self.factory();
        let host = ComWrapper::new(HostApplication);
        let context = host.as_com_ref::<IHostApplication>().unwrap().as_ptr() as *mut FUnknown;
        unsafe {
            let make = |cid: &TUID, what: *const c_char| -> Option<*mut c_void> {
                let mut obj: *mut c_void = std::ptr::null_mut();
                (ok(f.createInstance(cid.as_ptr(), what, &mut obj)) && !obj.is_null()).then_some(obj)
            };
            let component = make(&cid, iid::<IComponent>()).and_then(|o| ComPtr::from_raw(o as *mut IComponent)).ok_or_else(|| format!("{name}: could not create it"))?;
            if !ok(component.initialize(context)) {
                return Err(format!("{name}: initialize failed"));
            }
            let Some(processor) = component.cast::<IAudioProcessor>() else {
                component.terminate();
                return Err(format!("{name}: not an audio processor"));
            };
            // The controller: the component itself, or a class of its own.
            let (controller, separate) = match component.cast::<IEditController>() {
                Some(c) => (Some(c), false),
                None => {
                    let mut ccid: TUID = [0; 16];
                    let c = if ok(component.getControllerClassId(&mut ccid)) { make(&ccid, iid::<IEditController>()).and_then(|o| ComPtr::from_raw(o as *mut IEditController)) } else { None };
                    match c {
                        Some(c) if ok(c.initialize(context)) => (Some(c), true),
                        _ => (None, false),
                    }
                }
            };
            let (tx, rx) = rtrb::RingBuffer::new(PARAM_QUEUE);
            let handler = ComWrapper::new(ComponentHandler { edits: Mutex::new(tx) });
            let mut inst = Instance {
                controller,
                separate,
                connection: None,
                processor,
                component,
                handler,
                _host: host,
                owner,
                name: name.to_string(),
                in_channels: Vec::new(),
                out_channels: Vec::new(),
                event_inputs: 0,
                rt: UnsafeCell::new(Rt::new(rx)),
            };
            inst.connect();
            inst.configure_buses();
            Ok(inst)
        }
    }
}

// ----------------------------------------------------------- instance --

/// What the audio thread works with; built on the main thread while the
/// plugin is not processing.
struct Rt {
    max_frames: u32,
    sample_rate: f64,
    scratch: Vec<Vec<f32>>,
    in_ptrs: Vec<Vec<*mut f32>>,
    out_ptrs: Vec<Vec<*mut f32>>,
    inputs: Vec<AudioBusBuffers>,
    outputs: Vec<AudioBusBuffers>,
    events: ComWrapper<EventList>,
    params: ComWrapper<ParameterChanges>,
    edits: rtrb::Consumer<(ParamID, ParamValue)>,
    /// The channel each key last started on (note-offs on "any channel").
    key_channel: [i16; 128],
}

impl Rt {
    fn new(edits: rtrb::Consumer<(ParamID, ParamValue)>) -> Rt {
        Rt {
            max_frames: 0,
            sample_rate: 48000.0,
            scratch: Vec::new(),
            in_ptrs: Vec::new(),
            out_ptrs: Vec::new(),
            inputs: Vec::new(),
            outputs: Vec::new(),
            events: ComWrapper::new(EventList { events: UnsafeCell::new(Vec::with_capacity(EVENTS_PER_BLOCK)) }),
            params: ComWrapper::new(ParameterChanges::new()),
            edits,
            key_channel: [0; 128],
        }
    }
}

/// A VST3 processor and its controller.
pub struct Instance {
    controller: Option<ComPtr<IEditController>>,
    separate: bool,
    connection: Option<(ComPtr<IConnectionPoint>, ComPtr<IConnectionPoint>)>,
    processor: ComPtr<IAudioProcessor>,
    component: ComPtr<IComponent>,
    handler: ComWrapper<ComponentHandler>,
    _host: ComWrapper<HostApplication>,
    owner: gui::Owner,
    name: String,
    /// Channels per audio bus.
    in_channels: Vec<i32>,
    out_channels: Vec<i32>,
    event_inputs: i32,
    rt: UnsafeCell<Rt>,
}

impl Instance {
    /// Connects a separate controller to its processor (messages both
    /// ways), gives it the processor's state and our component handler.
    unsafe fn connect(&mut self) {
        let Some(ctrl) = &self.controller else { return };
        if self.separate {
            if let (Some(a), Some(b)) = (self.component.cast::<IConnectionPoint>(), ctrl.cast::<IConnectionPoint>()) {
                a.connect(b.as_ptr());
                b.connect(a.as_ptr());
                self.connection = Some((a, b));
            }
            let stream = ComWrapper::new(Stream::default());
            let s = stream.as_com_ref::<IBStream>().unwrap().as_ptr();
            if ok(self.component.getState(s)) {
                stream.rewind();
                ctrl.setComponentState(s);
            }
        }
        ctrl.setComponentHandler(self.handler.as_com_ref::<IComponentHandler>().unwrap().as_ptr());
    }

    /// Stereo on the main output where the plugin agrees; the main output
    /// and the first event input active.
    unsafe fn configure_buses(&mut self) {
        let c = &self.component;
        let p = &self.processor;
        let n_in = c.getBusCount(MediaTypes_::kAudio as i32, BusDirections_::kInput as i32).max(0);
        let n_out = c.getBusCount(MediaTypes_::kAudio as i32, BusDirections_::kOutput as i32).max(0);
        let arrangement = |dir: i32, i: i32| {
            let mut a: SpeakerArrangement = 0;
            p.getBusArrangement(dir, i, &mut a);
            a
        };
        let mut ins: Vec<SpeakerArrangement> = (0..n_in).map(|i| arrangement(BusDirections_::kInput as i32, i)).collect();
        let mut outs: Vec<SpeakerArrangement> = (0..n_out).map(|i| arrangement(BusDirections_::kOutput as i32, i)).collect();
        if let Some(main) = outs.first_mut() {
            if *main != SpeakerArr::kStereo {
                *main = SpeakerArr::kStereo;
                p.setBusArrangements(ins.as_mut_ptr(), n_in, outs.as_mut_ptr(), n_out);
            }
        }
        let channels = |dir: i32, i: i32| {
            let mut info: BusInfo = std::mem::zeroed();
            if ok(c.getBusInfo(MediaTypes_::kAudio as i32, dir, i, &mut info)) {
                info.channelCount.max(0)
            } else {
                arrangement(dir, i).count_ones() as i32
            }
        };
        self.in_channels = (0..n_in).map(|i| channels(BusDirections_::kInput as i32, i)).collect();
        self.out_channels = (0..n_out).map(|i| channels(BusDirections_::kOutput as i32, i)).collect();
        for i in 0..n_out {
            c.activateBus(MediaTypes_::kAudio as i32, BusDirections_::kOutput as i32, i, (i == 0) as TBool);
        }
        for i in 0..n_in {
            c.activateBus(MediaTypes_::kAudio as i32, BusDirections_::kInput as i32, i, 0);
        }
        self.event_inputs = c.getBusCount(MediaTypes_::kEvent as i32, BusDirections_::kInput as i32).max(0);
        for i in 0..self.event_inputs {
            c.activateBus(MediaTypes_::kEvent as i32, BusDirections_::kInput as i32, i, (i == 0) as TBool);
        }
    }

    /// Channels of the main audio input (0: an instrument).
    pub fn input_channels(&self) -> u32 {
        self.in_channels.first().copied().unwrap_or(0).max(0) as u32
    }

    pub fn output_channels(&self) -> u32 {
        self.out_channels.first().copied().unwrap_or(2).max(1) as u32
    }

    /// Main thread, not processing.
    pub fn activate(&self, sample_rate: f64, max_frames: u32) -> Result<(), String> {
        unsafe {
            let mut setup = ProcessSetup { processMode: ProcessModes_::kRealtime as i32, symbolicSampleSize: SymbolicSampleSizes_::kSample32 as i32, maxSamplesPerBlock: max_frames as i32, sampleRate: sample_rate };
            if !ok(self.processor.canProcessSampleSize(SymbolicSampleSizes_::kSample32 as i32)) {
                return Err("needs 64-bit processing".into());
            }
            if !ok(self.processor.setupProcessing(&mut setup)) {
                return Err("setupProcessing failed".into());
            }
            let rt = &mut *self.rt.get();
            rt.max_frames = max_frames;
            rt.sample_rate = sample_rate;
            rt.scratch.clear();
            let bus = |channels: &[i32], ptrs: &mut Vec<Vec<*mut f32>>, scratch: &mut Vec<Vec<f32>>| {
                ptrs.clear();
                for &n in channels {
                    let mut v = Vec::new();
                    for _ in 0..n {
                        scratch.push(vec![0.0; max_frames as usize]);
                        v.push(scratch.last_mut().unwrap().as_mut_ptr());
                    }
                    ptrs.push(v);
                }
            };
            bus(&self.in_channels, &mut rt.in_ptrs, &mut rt.scratch);
            bus(&self.out_channels, &mut rt.out_ptrs, &mut rt.scratch);
            let buffers = |ptrs: &mut Vec<Vec<*mut f32>>| -> Vec<AudioBusBuffers> {
                ptrs.iter_mut().map(|p| AudioBusBuffers { numChannels: p.len() as i32, silenceFlags: 0, __field0: AudioBusBuffers__type0 { channelBuffers32: p.as_mut_ptr() } }).collect()
            };
            rt.inputs = buffers(&mut rt.in_ptrs);
            rt.outputs = buffers(&mut rt.out_ptrs);
            if !ok(self.component.setActive(1)) {
                return Err("setActive failed".into());
            }
        }
        Ok(())
    }

    pub fn deactivate(&self) {
        unsafe { self.component.setActive(0) };
    }

    pub fn rt_start(&self) -> bool {
        // Many plugins don't implement setProcessing; that's not a failure.
        unsafe { self.processor.setProcessing(1) };
        true
    }

    pub fn rt_stop(&self) {
        unsafe { self.processor.setProcessing(0) };
    }

    /// The audio thread: one block, from the engine's CLAP process call.
    ///
    /// # Safety
    /// Audio thread only, while active and processing.
    #[allow(clippy::unnecessary_cast)] // the SDK's enums are i32 on Windows
    pub unsafe fn rt_process(&self, p: &clap_process) -> clap_process_status {
        let rt = &mut *self.rt.get();
        let frames = p.frames_count.min(rt.max_frames);
        if rt.outputs.is_empty() || p.audio_outputs.is_null() || (*p.audio_outputs).channel_count < 2 {
            return CLAP_PROCESS_ERROR;
        }
        let out = &*p.audio_outputs;
        let (left, right) = (*out.data32, *out.data32.add(1));

        // Events.
        let events = &mut *rt.events.events.get();
        events.clear();
        if !p.in_events.is_null() && self.event_inputs > 0 {
            let list = &*p.in_events;
            let n = list.size.map(|s| s(list)).unwrap_or(0);
            for i in 0..n {
                let Some(h) = list.get.map(|g| g(list, i)).filter(|h| !h.is_null()) else { continue };
                if (*h).space_id != CLAP_CORE_EVENT_SPACE_ID || events.len() >= EVENTS_PER_BLOCK {
                    continue;
                }
                let note = match (*h).type_ {
                    CLAP_EVENT_NOTE_ON | CLAP_EVENT_NOTE_OFF => {
                        let e = &*(h as *const clap_event_note);
                        Some(((*h).type_ == CLAP_EVENT_NOTE_ON, e.channel, e.key, e.velocity as f32, e.note_id))
                    }
                    CLAP_EVENT_MIDI => {
                        let e = &*(h as *const clap_event_midi);
                        match e.data[0] & 0xf0 {
                            0x90 if e.data[2] > 0 => Some((true, (e.data[0] & 15) as i16, e.data[1] as i16, e.data[2] as f32 / 127.0, -1)),
                            0x80 | 0x90 => Some((false, (e.data[0] & 15) as i16, e.data[1] as i16, e.data[2] as f32 / 127.0, -1)),
                            _ => None,
                        }
                    }
                    _ => None,
                };
                let Some((on, channel, key, velocity, note_id)) = note else { continue };
                if !(0..128).contains(&key) {
                    continue;
                }
                let channel = if channel < 0 { rt.key_channel[key as usize] } else { channel };
                if on {
                    rt.key_channel[key as usize] = channel;
                }
                let mut ev: Event = std::mem::zeroed();
                ev.sampleOffset = (*h).time.min(frames.saturating_sub(1)) as i32;
                ev.flags = Event_::EventFlags_::kIsLive as u16;
                if on {
                    ev.r#type = Event_::EventTypes_::kNoteOnEvent as u16;
                    ev.__field0.noteOn = NoteOnEvent { channel, pitch: key, tuning: 0.0, velocity, length: 0, noteId: note_id };
                } else {
                    ev.r#type = Event_::EventTypes_::kNoteOffEvent as u16;
                    ev.__field0.noteOff = NoteOffEvent { channel, pitch: key, velocity, noteId: note_id, tuning: 0.0 };
                }
                events.push(ev);
            }
        }

        // Parameter edits from the editor.
        rt.params.clear();
        while let Ok((id, value)) = rt.edits.pop() {
            rt.params.set(id, value);
        }

        // Transport.
        let mut ctx: ProcessContext = std::mem::zeroed();
        ctx.sampleRate = rt.sample_rate;
        if p.steady_time >= 0 {
            ctx.continousTimeSamples = p.steady_time;
            ctx.state |= ProcessContext_::StatesAndFlags_::kContTimeValid as u32;
        }
        if let Some(t) = p.transport.as_ref() {
            use ProcessContext_::StatesAndFlags_::*;
            ctx.tempo = t.tempo;
            ctx.projectTimeMusic = t.song_pos_beats as f64 / CLAP_BEATTIME_FACTOR as f64;
            ctx.barPositionMusic = t.bar_start as f64 / CLAP_BEATTIME_FACTOR as f64;
            ctx.projectTimeSamples = (t.song_pos_seconds as f64 / CLAP_SECTIME_FACTOR as f64 * rt.sample_rate) as i64;
            ctx.timeSigNumerator = t.tsig_num as i32;
            ctx.timeSigDenominator = t.tsig_denom as i32;
            ctx.state |= (kTempoValid | kProjectTimeMusicValid | kBarPositionValid | kTimeSigValid) as u32;
            if t.flags & CLAP_TRANSPORT_IS_PLAYING != 0 {
                ctx.state |= kPlaying as u32;
            }
        }

        // Buffers: the main output writes straight into the engine's.
        let main = &mut rt.out_ptrs[0];
        let mono = main.len() == 1;
        if let Some(c) = main.get_mut(0) {
            *c = left;
        }
        if let Some(c) = main.get_mut(1) {
            *c = right;
        }
        // The main input takes the engine's audio (an effect's signal); the rest is silent.
        let given = if p.audio_inputs.is_null() || p.audio_inputs_count == 0 { None } else { Some(&*p.audio_inputs) };
        for (bi, bus) in rt.in_ptrs.iter().enumerate() {
            for (ci, &c) in bus.iter().enumerate() {
                let dst = std::slice::from_raw_parts_mut(c, frames as usize);
                match given.filter(|g| bi == 0 && g.channel_count > 0) {
                    Some(g) => dst.copy_from_slice(std::slice::from_raw_parts(*g.data32.add(ci.min(g.channel_count as usize - 1)), frames as usize)),
                    None => dst.fill(0.0),
                }
            }
        }
        for b in rt.inputs.iter_mut().chain(rt.outputs.iter_mut()) {
            b.silenceFlags = 0;
        }
        let mut data = ProcessData {
            processMode: ProcessModes_::kRealtime as i32,
            symbolicSampleSize: SymbolicSampleSizes_::kSample32 as i32,
            numSamples: frames as i32,
            numInputs: rt.inputs.len() as i32,
            numOutputs: rt.outputs.len() as i32,
            inputs: if rt.inputs.is_empty() { std::ptr::null_mut() } else { rt.inputs.as_mut_ptr() },
            outputs: rt.outputs.as_mut_ptr(),
            inputParameterChanges: rt.params.as_com_ref::<IParameterChanges>().unwrap().as_ptr(),
            outputParameterChanges: std::ptr::null_mut(),
            inputEvents: rt.events.as_com_ref::<IEventList>().unwrap().as_ptr(),
            outputEvents: std::ptr::null_mut(),
            processContext: &mut ctx,
        };
        let r = self.processor.process(&mut data);
        if mono {
            std::ptr::copy_nonoverlapping(left, right, frames as usize);
        }
        if ok(r) {
            CLAP_PROCESS_CONTINUE
        } else {
            CLAP_PROCESS_ERROR
        }
    }

    // ------------------------------------------------------------ editor --

    pub fn has_editor(&self) -> bool {
        self.controller.is_some()
    }

    /// The controller (tests).
    pub fn controller(&self) -> Option<&ComPtr<IEditController>> {
        self.controller.as_ref()
    }

    /// Main thread: creates the editor view and shows it in a window.
    pub fn open_editor(&self, title: &str) -> Result<Editor, String> {
        let none = || format!("{} has no editor", self.name);
        let ctrl = self.controller.as_ref().ok_or_else(none)?;
        #[cfg(windows)]
        let platform = kPlatformTypeHWND;
        #[cfg(not(windows))]
        let platform = kPlatformTypeX11EmbedWindowID;
        unsafe {
            let view = ComPtr::<IPlugView>::from_raw(ctrl.createView(c"editor".as_ptr())).ok_or_else(none)?;
            if view.isPlatformTypeSupported(platform) != kResultTrue {
                return Err(format!("{}: its editor does not support this platform's windows", self.name));
            }
            let mut rect: ViewRect = std::mem::zeroed();
            view.getSize(&mut rect);
            let (w, h) = ((rect.right - rect.left).max(0) as u32, (rect.bottom - rect.top).max(0) as u32);
            let (w, h) = if w == 0 || h == 0 { (640, 480) } else { (w, h) };
            let resizable = view.canResize() == kResultTrue;
            let window = Rc::new(gui::Window::new(title, w, h, resizable)?);
            let frame = ComWrapper::new(Frame { owner: self.owner, window: Rc::downgrade(&window) });
            view.setFrame(frame.as_com_ref::<IPlugFrame>().unwrap().as_ptr());
            let parent = match window.handle() {
                gui::Handle::X11(id) => id as usize as *mut c_void,
                gui::Handle::Win32(hwnd) => hwnd,
            };
            if !ok(view.attached(parent, platform)) {
                view.setFrame(std::ptr::null_mut());
                return Err(format!("{}: its editor would not open in a window", self.name));
            }
            // Some editors settle on their size once attached.
            let mut after: ViewRect = std::mem::zeroed();
            if ok(view.getSize(&mut after)) {
                let (aw, ah) = ((after.right - after.left).max(1) as u32, (after.bottom - after.top).max(1) as u32);
                if (aw, ah) != (w, h) {
                    window.set_resizable(resizable, aw, ah);
                }
            }
            window.show();
            Ok(Editor { view, _frame: frame, window, resizable })
        }
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        // Main thread, the editor closed and the plugin deactivated (host::Plugin).
        unsafe {
            if let Some(c) = &self.controller {
                c.setComponentHandler(std::ptr::null_mut());
            }
            if let Some((a, b)) = self.connection.take() {
                a.disconnect(b.as_ptr());
                b.disconnect(a.as_ptr());
            }
            if let (Some(c), true) = (self.controller.take(), self.separate) {
                c.terminate();
            }
            self.component.terminate();
        }
    }
}

/// An open VST3 editor.
pub struct Editor {
    view: ComPtr<IPlugView>,
    _frame: ComWrapper<Frame>,
    window: Rc<gui::Window>,
    resizable: bool,
}

impl Editor {
    pub fn raise(&self) {
        self.window.raise();
    }

    /// Main thread: follows the window; true when the user closed it.
    pub fn idle(&mut self) -> bool {
        let ev = self.window.events();
        if ev.close {
            return true;
        }
        if let (Some((w, h)), true) = (ev.resized, self.resizable) {
            unsafe {
                let mut r = ViewRect { left: 0, top: 0, right: w as i32, bottom: h as i32 };
                self.view.checkSizeConstraint(&mut r);
                let (cw, ch) = ((r.right - r.left).max(1) as u32, (r.bottom - r.top).max(1) as u32);
                if (cw, ch) != (w, h) {
                    self.window.set_size(cw, ch);
                }
                self.view.onSize(&mut r);
            }
        }
        false
    }

    pub fn close(self) {
        unsafe {
            self.view.removed();
            self.view.setFrame(std::ptr::null_mut());
        }
        drop(self.view);
        drop(self.window);
    }
}

// --------------------------------------------- host-side COM objects --

/// The host context given to `initialize`: our name, and messages for
/// processor ↔ controller communication.
struct HostApplication;

impl Class for HostApplication {
    type Interfaces = (IHostApplication,);
}

impl IHostApplicationTrait for HostApplication {
    unsafe fn getName(&self, name: *mut String128) -> tresult {
        match name.as_mut() {
            Some(n) => {
                copy_wide("Corinzia", n);
                kResultOk
            }
            None => kInvalidArgument,
        }
    }

    unsafe fn createInstance(&self, cid: *mut TUID, _iid: *mut TUID, obj: *mut *mut c_void) -> tresult {
        if obj.is_null() {
            return kInvalidArgument;
        }
        *obj = std::ptr::null_mut();
        if same_iid::<IMessage>(cid) {
            let m = ComWrapper::new(Message { id: Mutex::new(None), attributes: ComWrapper::new(AttributeList::default()) });
            *obj = m.to_com_ptr::<IMessage>().unwrap().into_raw() as *mut c_void;
            kResultOk
        } else if same_iid::<IAttributeList>(cid) {
            let a = ComWrapper::new(AttributeList::default());
            *obj = a.to_com_ptr::<IAttributeList>().unwrap().into_raw() as *mut c_void;
            kResultOk
        } else {
            kNoInterface
        }
    }
}

/// Receives the editor's parameter edits (main thread) for the audio thread.
struct ComponentHandler {
    edits: Mutex<rtrb::Producer<(ParamID, ParamValue)>>,
}

impl Class for ComponentHandler {
    type Interfaces = (IComponentHandler,);
}

impl IComponentHandlerTrait for ComponentHandler {
    unsafe fn beginEdit(&self, _id: ParamID) -> tresult {
        kResultOk
    }
    unsafe fn performEdit(&self, id: ParamID, value: ParamValue) -> tresult {
        match self.edits.lock().map(|mut q| q.push((id, value)).is_ok()) {
            Ok(true) => kResultOk,
            _ => kResultFalse,
        }
    }
    unsafe fn endEdit(&self, _id: ParamID) -> tresult {
        kResultOk
    }
    unsafe fn restartComponent(&self, _flags: int32) -> tresult {
        kResultOk
    }
}

/// The editor's frame: resizes its window, and on Linux is the run loop
/// the editor registers its timers and descriptors with.
struct Frame {
    owner: gui::Owner,
    window: Weak<gui::Window>,
}

impl Class for Frame {
    type Interfaces = (IPlugFrame, IRunLoop);
}

impl IPlugFrameTrait for Frame {
    unsafe fn resizeView(&self, view: *mut IPlugView, size: *mut ViewRect) -> tresult {
        let (Some(view), Some(r)) = (ComRef::from_raw(view), size.as_mut()) else { return kInvalidArgument };
        if let Some(w) = self.window.upgrade() {
            w.set_size((r.right - r.left).max(1) as u32, (r.bottom - r.top).max(1) as u32);
        }
        view.onSize(r);
        kResultOk
    }
}

impl IRunLoopTrait for Frame {
    unsafe fn registerEventHandler(&self, handler: *mut IEventHandler, fd: FileDescriptor) -> tresult {
        let Some(h) = ComRef::from_raw(handler).map(|h| h.to_com_ptr()) else { return kInvalidArgument };
        gui::register_fd(self.owner, handler as usize as u64, fd, gui::FD_READ, Rc::new(move |_| h.onFDIsSet(fd)));
        kResultOk
    }
    unsafe fn unregisterEventHandler(&self, handler: *mut IEventHandler) -> tresult {
        gui::unregister_fds(self.owner, handler as usize as u64);
        kResultOk
    }
    unsafe fn registerTimer(&self, handler: *mut ITimerHandler, ms: TimerInterval) -> tresult {
        let Some(h) = ComRef::from_raw(handler).map(|h| h.to_com_ptr()) else { return kInvalidArgument };
        gui::register_timer(self.owner, Some(handler as usize as u64), ms, Rc::new(move || h.onTimer()));
        kResultOk
    }
    unsafe fn unregisterTimer(&self, handler: *mut ITimerHandler) -> tresult {
        gui::unregister_timer(self.owner, handler as usize as u64);
        kResultOk
    }
}

/// A block's input events (audio thread).
struct EventList {
    events: UnsafeCell<Vec<Event>>,
}

impl Class for EventList {
    type Interfaces = (IEventList,);
}

impl IEventListTrait for EventList {
    unsafe fn getEventCount(&self) -> int32 {
        (&*self.events.get()).len() as int32
    }
    unsafe fn getEvent(&self, index: int32, e: *mut Event) -> tresult {
        match ((&*self.events.get()).get(index as usize), e.as_mut()) {
            (Some(ev), Some(out)) => {
                *out = *ev;
                kResultOk
            }
            _ => kInvalidArgument,
        }
    }
    unsafe fn addEvent(&self, _e: *mut Event) -> tresult {
        kResultFalse
    }
}

/// A block's parameter changes (audio thread); preallocated queues, one
/// point each: the latest value at the start of the block.
struct ParameterChanges {
    queues: Vec<ComWrapper<ParamQueue>>,
    count: Cell<usize>,
}

impl ParameterChanges {
    fn new() -> ParameterChanges {
        ParameterChanges { queues: (0..PARAMS_PER_BLOCK).map(|_| ComWrapper::new(ParamQueue { id: Cell::new(0), value: Cell::new(0.0) })).collect(), count: Cell::new(0) }
    }
    fn clear(&self) {
        self.count.set(0);
    }
    fn set(&self, id: ParamID, value: ParamValue) {
        let n = self.count.get();
        if let Some(q) = self.queues[..n].iter().find(|q| q.id.get() == id) {
            q.value.set(value);
        } else if n < self.queues.len() {
            self.queues[n].id.set(id);
            self.queues[n].value.set(value);
            self.count.set(n + 1);
        }
    }
}

impl Class for ParameterChanges {
    type Interfaces = (IParameterChanges,);
}

impl IParameterChangesTrait for ParameterChanges {
    unsafe fn getParameterCount(&self) -> int32 {
        self.count.get() as int32
    }
    unsafe fn getParameterData(&self, index: int32) -> *mut IParamValueQueue {
        if index < 0 || index as usize >= self.count.get() {
            return std::ptr::null_mut();
        }
        self.queues[index as usize].as_com_ref::<IParamValueQueue>().unwrap().as_ptr()
    }
    unsafe fn addParameterData(&self, _id: *const ParamID, _index: *mut int32) -> *mut IParamValueQueue {
        std::ptr::null_mut()
    }
}

struct ParamQueue {
    id: Cell<ParamID>,
    value: Cell<ParamValue>,
}

impl Class for ParamQueue {
    type Interfaces = (IParamValueQueue,);
}

impl IParamValueQueueTrait for ParamQueue {
    unsafe fn getParameterId(&self) -> ParamID {
        self.id.get()
    }
    unsafe fn getPointCount(&self) -> int32 {
        1
    }
    unsafe fn getPoint(&self, index: int32, offset: *mut int32, value: *mut ParamValue) -> tresult {
        if index != 0 || offset.is_null() || value.is_null() {
            return kInvalidArgument;
        }
        *offset = 0;
        *value = self.value.get();
        kResultOk
    }
    unsafe fn addPoint(&self, _offset: int32, _value: ParamValue, _index: *mut int32) -> tresult {
        kResultFalse
    }
}

/// A message between a processor and its controller.
struct Message {
    id: Mutex<Option<CString>>,
    attributes: ComWrapper<AttributeList>,
}

impl Class for Message {
    type Interfaces = (IMessage,);
}

impl IMessageTrait for Message {
    unsafe fn getMessageID(&self) -> FIDString {
        self.id.lock().ok().and_then(|id| id.as_ref().map(|s| s.as_ptr())).unwrap_or(std::ptr::null())
    }
    unsafe fn setMessageID(&self, id: FIDString) {
        if let Ok(mut m) = self.id.lock() {
            *m = (!id.is_null()).then(|| CStr::from_ptr(id).to_owned());
        }
    }
    unsafe fn getAttributes(&self) -> *mut IAttributeList {
        // Not reference counted (the message keeps it), as in the SDK's host.
        self.attributes.as_com_ref::<IAttributeList>().unwrap().as_ptr()
    }
}

enum Attribute {
    Int(i64),
    Float(f64),
    String(Vec<TChar>),
    Binary(Vec<u8>),
}

#[derive(Default)]
struct AttributeList {
    values: Mutex<HashMap<Vec<u8>, Attribute>>,
}

impl Class for AttributeList {
    type Interfaces = (IAttributeList,);
}

unsafe fn attr_key(id: IAttributeList_::AttrID) -> Option<Vec<u8>> {
    (!id.is_null()).then(|| CStr::from_ptr(id).to_bytes().to_vec())
}

impl AttributeList {
    unsafe fn put(&self, id: IAttributeList_::AttrID, a: Attribute) -> tresult {
        match (attr_key(id), self.values.lock()) {
            (Some(k), Ok(mut v)) => {
                v.insert(k, a);
                kResultOk
            }
            _ => kInvalidArgument,
        }
    }
    unsafe fn with<R>(&self, id: IAttributeList_::AttrID, f: impl FnOnce(&Attribute) -> Option<R>) -> Option<R> {
        let k = attr_key(id)?;
        let v = self.values.lock().ok()?;
        v.get(&k).and_then(f)
    }
}

impl IAttributeListTrait for AttributeList {
    unsafe fn setInt(&self, id: IAttributeList_::AttrID, value: int64) -> tresult {
        self.put(id, Attribute::Int(value))
    }
    unsafe fn getInt(&self, id: IAttributeList_::AttrID, value: *mut int64) -> tresult {
        match (self.with(id, |a| if let Attribute::Int(i) = a { Some(*i) } else { None }), value.as_mut()) {
            (Some(i), Some(out)) => {
                *out = i;
                kResultOk
            }
            _ => kResultFalse,
        }
    }
    unsafe fn setFloat(&self, id: IAttributeList_::AttrID, value: f64) -> tresult {
        self.put(id, Attribute::Float(value))
    }
    unsafe fn getFloat(&self, id: IAttributeList_::AttrID, value: *mut f64) -> tresult {
        match (self.with(id, |a| if let Attribute::Float(f) = a { Some(*f) } else { None }), value.as_mut()) {
            (Some(f), Some(out)) => {
                *out = f;
                kResultOk
            }
            _ => kResultFalse,
        }
    }
    unsafe fn setString(&self, id: IAttributeList_::AttrID, string: *const TChar) -> tresult {
        if string.is_null() {
            return kInvalidArgument;
        }
        let mut n = 0;
        while *string.add(n) != 0 {
            n += 1;
        }
        self.put(id, Attribute::String(std::slice::from_raw_parts(string, n).to_vec()))
    }
    unsafe fn getString(&self, id: IAttributeList_::AttrID, string: *mut TChar, size_in_bytes: uint32) -> tresult {
        let cap = size_in_bytes as usize / std::mem::size_of::<TChar>();
        if string.is_null() || cap == 0 {
            return kInvalidArgument;
        }
        let s = self.with(id, |a| if let Attribute::String(s) = a { Some(s.clone()) } else { None });
        match s {
            Some(s) => {
                let n = s.len().min(cap - 1);
                std::ptr::copy_nonoverlapping(s.as_ptr(), string, n);
                *string.add(n) = 0;
                kResultOk
            }
            None => kResultFalse,
        }
    }
    unsafe fn setBinary(&self, id: IAttributeList_::AttrID, data: *const c_void, size_in_bytes: uint32) -> tresult {
        let bytes = if data.is_null() { Vec::new() } else { std::slice::from_raw_parts(data as *const u8, size_in_bytes as usize).to_vec() };
        self.put(id, Attribute::Binary(bytes))
    }
    unsafe fn getBinary(&self, id: IAttributeList_::AttrID, data: *mut *const c_void, size_in_bytes: *mut uint32) -> tresult {
        // The pointer stays valid while the attribute is unchanged (the
        // bytes live in the list, not in the guard).
        let b = self.with(id, |a| if let Attribute::Binary(b) = a { Some((b.as_ptr(), b.len())) } else { None });
        match (b, data.as_mut(), size_in_bytes.as_mut()) {
            (Some((p, n)), Some(d), Some(s)) => {
                *d = p as *const c_void;
                *s = n as uint32;
                kResultOk
            }
            _ => kResultFalse,
        }
    }
}

/// An in-memory stream (component state for the controller).
#[derive(Default)]
struct Stream {
    data: Mutex<(Vec<u8>, usize)>,
}

impl Stream {
    fn rewind(&self) {
        if let Ok(mut d) = self.data.lock() {
            d.1 = 0;
        }
    }
}

impl Class for Stream {
    type Interfaces = (IBStream,);
}

impl IBStreamTrait for Stream {
    unsafe fn read(&self, buffer: *mut c_void, n: int32, read: *mut int32) -> tresult {
        let Ok(mut d) = self.data.lock() else { return kInternalError };
        let (bytes, pos) = &mut *d;
        let n = (n.max(0) as usize).min(bytes.len().saturating_sub(*pos));
        if n > 0 {
            std::ptr::copy_nonoverlapping(bytes[*pos..].as_ptr(), buffer as *mut u8, n);
        }
        *pos += n;
        if let Some(r) = read.as_mut() {
            *r = n as int32;
        }
        kResultOk
    }
    unsafe fn write(&self, buffer: *mut c_void, n: int32, written: *mut int32) -> tresult {
        let Ok(mut d) = self.data.lock() else { return kInternalError };
        let (bytes, pos) = &mut *d;
        let n = n.max(0) as usize;
        let src = std::slice::from_raw_parts(buffer as *const u8, n);
        if bytes.len() < *pos + n {
            bytes.resize(*pos + n, 0);
        }
        bytes[*pos..*pos + n].copy_from_slice(src);
        *pos += n;
        if let Some(w) = written.as_mut() {
            *w = n as int32;
        }
        kResultOk
    }
    unsafe fn seek(&self, to: int64, mode: int32, result: *mut int64) -> tresult {
        let Ok(mut d) = self.data.lock() else { return kInternalError };
        use IBStream_::IStreamSeekMode_::*;
        let base = match mode {
            m if m == kIBSeekSet as i32 => 0,
            m if m == kIBSeekCur as i32 => d.1 as i64,
            m if m == kIBSeekEnd as i32 => d.0.len() as i64,
            _ => return kInvalidArgument,
        };
        let p = (base + to).max(0);
        d.1 = p as usize;
        if let Some(r) = result.as_mut() {
            *r = p;
        }
        kResultOk
    }
    unsafe fn tell(&self, pos: *mut int64) -> tresult {
        let Ok(d) = self.data.lock() else { return kInternalError };
        match pos.as_mut() {
            Some(p) => {
                *p = d.1 as int64;
                kResultOk
            }
            None => kInvalidArgument,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn class_ids_round_trip() {
        let cid: TUID = vst3::uid(0x6E332252, 0x54224A00, 0xAA69301A, 0xF318797D);
        let hex = class_id_hex(&cid);
        assert_eq!(hex.len(), 32);
        assert_eq!(class_id(&hex), Some(cid));
        assert_eq!(class_id("xyz"), None);
    }

    #[test]
    fn bundles_resolve_to_their_binary() {
        let single = Path::new("/nonexistent/Plug.vst3");
        assert_eq!(module_binary(single), single);
        let dir = std::env::temp_dir().join(format!("corinzia-vst3-{}/Plug.vst3", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let bin = module_binary(&dir);
        assert!(bin.starts_with(dir.join("Contents")));
        assert!(bin.to_string_lossy().ends_with(if cfg!(windows) { "Plug.vst3" } else { "Plug.so" }));
        std::fs::remove_dir_all(dir.parent().unwrap()).ok();
    }

    #[test]
    fn messages_carry_attributes() {
        let host = ComWrapper::new(HostApplication);
        let h = host.to_com_ptr::<IHostApplication>().unwrap();
        let mut cid: TUID = [0; 16];
        for (d, s) in cid.iter_mut().zip(IMessage::IID) {
            *d = s as c_char;
        }
        let mut obj = std::ptr::null_mut();
        unsafe {
            assert_eq!(h.createInstance(&mut cid, &mut cid, &mut obj), kResultOk);
            let m = ComPtr::<IMessage>::from_raw(obj as *mut IMessage).unwrap();
            m.setMessageID(c"hello".as_ptr());
            assert_eq!(CStr::from_ptr(m.getMessageID()), c"hello");
            let a = ComRef::<IAttributeList>::from_raw(m.getAttributes()).unwrap();
            assert_eq!(a.setInt(c"n".as_ptr(), 42), kResultOk);
            let mut n = 0;
            assert_eq!(a.getInt(c"n".as_ptr(), &mut n), kResultOk);
            assert_eq!(n, 42);
            let wide: Vec<TChar> = "ok".encode_utf16().chain([0]).map(|c| c as TChar).collect();
            a.setString(c"s".as_ptr(), wide.as_ptr());
            let mut back = [0 as TChar; 8];
            assert_eq!(a.getString(c"s".as_ptr(), back.as_mut_ptr(), 16), kResultOk);
            assert_eq!(&back[..3], &wide[..]);
            let bytes = [1u8, 2, 3];
            a.setBinary(c"b".as_ptr(), bytes.as_ptr() as *const c_void, 3);
            let (mut p, mut len) = (std::ptr::null(), 0);
            assert_eq!(a.getBinary(c"b".as_ptr(), &mut p, &mut len), kResultOk);
            assert_eq!(std::slice::from_raw_parts(p as *const u8, len as usize), &bytes);
        }
    }

    #[test]
    fn streams_read_back_what_was_written() {
        let s = ComWrapper::new(Stream::default());
        let p = s.to_com_ptr::<IBStream>().unwrap();
        unsafe {
            let mut data = *b"abcdef";
            let mut n = 0;
            p.write(data.as_mut_ptr() as *mut c_void, 6, &mut n);
            assert_eq!(n, 6);
            let mut at = 0;
            p.seek(2, IBStream_::IStreamSeekMode_::kIBSeekSet as i32, &mut at);
            let mut buf = [0u8; 8];
            p.read(buf.as_mut_ptr() as *mut c_void, 8, &mut n);
            assert_eq!(&buf[..n as usize], b"cdef");
        }
    }
}
