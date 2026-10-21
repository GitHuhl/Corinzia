//! VST3 hosting against a small VST3 instrument defined here: a processor
//! and a separate controller (as most VST3 plugins are made), a parameter,
//! and an editor view. It outputs `gain × velocity` while a note is held.

#![allow(non_snake_case)]
#![allow(clippy::unnecessary_cast)] // the SDK's enums are i32 on Windows

use clap_sys::audio_buffer::clap_audio_buffer;
use clap_sys::events::*;
use clap_sys::fixedpoint::CLAP_BEATTIME_FACTOR;
use clap_sys::process::*;
use corinzia_audio::host::{Format, Library, Plugin};
use std::cell::{Cell, RefCell};
use std::ffi::{c_char, c_void, CStr};
use std::path::Path;
use std::sync::{Arc, Mutex};
use vst3::Steinberg::Linux::*;
use vst3::Steinberg::Vst::*;
use vst3::Steinberg::*;
use vst3::{uid, Class, ComPtr, ComRef, ComWrapper};

const PROCESSOR: TUID = uid(0x1C0A5A11, 0x2F3B4C5D, 0x8E9FA0B1, 0xC2D3E4F5);
const CONTROLLER: TUID = uid(0x9A8B7C6D, 0x5E4F3A2B, 0x1C0D9E8F, 0x7A6B5C4D);
const GAIN: ParamID = 7;

/// What the test watches.
#[derive(Default)]
struct Seen {
    host_name: String,
    connected: u32,
    controller_got_state: Option<f64>,
    handler: Option<ComPtr<IComponentHandler>>,
    tempo: f64,
    playing: bool,
    attached: Option<(usize, String)>,
    removed: bool,
    frame: Option<ComPtr<IPlugFrame>>,
}

type Shared = Arc<Mutex<Seen>>;

fn fill(dst: &mut [c_char], s: &str) {
    for (d, b) in dst.iter_mut().zip(s.bytes().chain([0])) {
        *d = b as c_char;
    }
}

struct Factory {
    seen: Shared,
}
impl Class for Factory {
    type Interfaces = (IPluginFactory2,);
}
impl IPluginFactoryTrait for Factory {
    unsafe fn getFactoryInfo(&self, info: *mut PFactoryInfo) -> tresult {
        let i = &mut *info;
        fill(&mut i.vendor, "Corinzia Tests");
        kResultOk
    }
    unsafe fn countClasses(&self) -> int32 {
        2
    }
    unsafe fn getClassInfo(&self, index: int32, info: *mut PClassInfo) -> tresult {
        let i = &mut *info;
        match index {
            0 => {
                i.cid = PROCESSOR;
                fill(&mut i.category, "Audio Module Class");
                fill(&mut i.name, "Test Tone");
            }
            1 => {
                i.cid = CONTROLLER;
                fill(&mut i.category, "Component Controller Class");
                fill(&mut i.name, "Test Tone Controller");
            }
            _ => return kInvalidArgument,
        }
        kResultOk
    }
    unsafe fn createInstance(&self, cid: FIDString, _iid: FIDString, obj: *mut *mut c_void) -> tresult {
        let cid = std::slice::from_raw_parts(cid as *const u8, 16);
        let as_bytes = |t: &TUID| t.iter().map(|b| *b as u8).collect::<Vec<u8>>();
        if cid == as_bytes(&PROCESSOR) {
            let p = ComWrapper::new(Processor { seen: self.seen.clone(), gain: Cell::new(1.0), level: Cell::new(0.0) });
            *obj = p.to_com_ptr::<IComponent>().unwrap().into_raw() as *mut c_void;
            kResultOk
        } else if cid == as_bytes(&CONTROLLER) {
            let c = ComWrapper::new(Controller { seen: self.seen.clone(), gain: Cell::new(1.0) });
            *obj = c.to_com_ptr::<IEditController>().unwrap().into_raw() as *mut c_void;
            kResultOk
        } else {
            kNoInterface
        }
    }
}
impl IPluginFactory2Trait for Factory {
    unsafe fn getClassInfo2(&self, index: int32, info: *mut PClassInfo2) -> tresult {
        let mut base: PClassInfo = std::mem::zeroed();
        let r = self.getClassInfo(index, &mut base);
        let i = &mut *info;
        i.cid = base.cid;
        i.category = base.category;
        i.name = base.name;
        fill(&mut i.subCategories, if index == 0 { "Instrument|Synth" } else { "" });
        fill(&mut i.vendor, "Corinzia Tests");
        fill(&mut i.version, "1.0.0");
        r
    }
}

struct Processor {
    seen: Shared,
    gain: Cell<f64>,
    level: Cell<f32>,
}
impl Class for Processor {
    type Interfaces = (IComponent, IAudioProcessor, IConnectionPoint);
}
impl IPluginBaseTrait for Processor {
    unsafe fn initialize(&self, context: *mut FUnknown) -> tresult {
        // The host context is an IHostApplication with a name.
        let host = ComRef::from_raw(context).and_then(|c| c.cast::<IHostApplication>()).expect("host application");
        let mut name: String128 = [0; 128];
        host.getName(&mut name);
        let n = name.iter().position(|c| *c == 0).unwrap_or(0);
        self.seen.lock().unwrap().host_name = String::from_utf16_lossy(&name[..n].iter().map(|c| *c as u16).collect::<Vec<_>>());
        kResultOk
    }
    unsafe fn terminate(&self) -> tresult {
        kResultOk
    }
}
impl IComponentTrait for Processor {
    unsafe fn getControllerClassId(&self, class_id: *mut TUID) -> tresult {
        *class_id = CONTROLLER;
        kResultOk
    }
    unsafe fn setIoMode(&self, _mode: IoMode) -> tresult {
        kResultOk
    }
    unsafe fn getBusCount(&self, t: MediaType, dir: BusDirection) -> int32 {
        let audio_out = t == MediaTypes_::kAudio as MediaType && dir == BusDirections_::kOutput as BusDirection;
        let event_in = t == MediaTypes_::kEvent as MediaType && dir == BusDirections_::kInput as BusDirection;
        (audio_out || event_in) as int32
    }
    unsafe fn getBusInfo(&self, t: MediaType, dir: BusDirection, index: int32, bus: *mut BusInfo) -> tresult {
        if index != 0 || self.getBusCount(t, dir) == 0 {
            return kInvalidArgument;
        }
        let b = &mut *bus;
        b.mediaType = t;
        b.direction = dir;
        b.channelCount = if t == MediaTypes_::kAudio as MediaType { 2 } else { 16 };
        kResultOk
    }
    unsafe fn getRoutingInfo(&self, _i: *mut RoutingInfo, _o: *mut RoutingInfo) -> tresult {
        kNotImplemented
    }
    unsafe fn activateBus(&self, _t: MediaType, _d: BusDirection, _i: int32, _s: TBool) -> tresult {
        kResultOk
    }
    unsafe fn setActive(&self, _state: TBool) -> tresult {
        kResultOk
    }
    unsafe fn setState(&self, _state: *mut IBStream) -> tresult {
        kResultOk
    }
    unsafe fn getState(&self, state: *mut IBStream) -> tresult {
        let s = ComRef::from_raw(state).unwrap();
        let mut bytes = 0.75f64.to_le_bytes();
        let mut n = 0;
        s.write(bytes.as_mut_ptr() as *mut c_void, 8, &mut n);
        kResultOk
    }
}
impl IAudioProcessorTrait for Processor {
    unsafe fn setBusArrangements(&self, _i: *mut SpeakerArrangement, _ni: int32, _o: *mut SpeakerArrangement, _no: int32) -> tresult {
        kResultTrue
    }
    unsafe fn getBusArrangement(&self, _dir: BusDirection, _index: int32, arr: *mut SpeakerArrangement) -> tresult {
        *arr = SpeakerArr::kStereo;
        kResultOk
    }
    unsafe fn canProcessSampleSize(&self, size: int32) -> tresult {
        if size == SymbolicSampleSizes_::kSample32 as i32 {
            kResultOk
        } else {
            kResultFalse
        }
    }
    unsafe fn getLatencySamples(&self) -> uint32 {
        0
    }
    unsafe fn setupProcessing(&self, _setup: *mut ProcessSetup) -> tresult {
        kResultOk
    }
    unsafe fn setProcessing(&self, _state: TBool) -> tresult {
        kNotImplemented // as many plugins answer
    }
    unsafe fn process(&self, data: *mut ProcessData) -> tresult {
        let d = &mut *data;
        if let Some(changes) = ComRef::from_raw(d.inputParameterChanges) {
            for i in 0..changes.getParameterCount() {
                let q = ComRef::from_raw(changes.getParameterData(i)).unwrap();
                let (mut at, mut v) = (0, 0.0);
                if q.getParameterId() == GAIN && q.getPoint(q.getPointCount() - 1, &mut at, &mut v) == kResultOk {
                    self.gain.set(v);
                }
            }
        }
        if let Some(events) = ComRef::from_raw(d.inputEvents) {
            for i in 0..events.getEventCount() {
                let mut e: Event = std::mem::zeroed();
                events.getEvent(i, &mut e);
                if e.r#type == Event_::EventTypes_::kNoteOnEvent as u16 {
                    self.level.set(e.__field0.noteOn.velocity);
                } else if e.r#type == Event_::EventTypes_::kNoteOffEvent as u16 {
                    self.level.set(0.0);
                }
            }
        }
        if let Some(ctx) = d.processContext.as_ref() {
            let mut s = self.seen.lock().unwrap();
            s.tempo = ctx.tempo;
            s.playing = ctx.state & ProcessContext_::StatesAndFlags_::kPlaying as u32 != 0;
        }
        let out = &*d.outputs;
        let v = self.level.get() * self.gain.get() as f32;
        for c in 0..out.numChannels as usize {
            let ch = *out.__field0.channelBuffers32.add(c);
            std::slice::from_raw_parts_mut(ch, d.numSamples as usize).fill(v);
        }
        kResultOk
    }
    unsafe fn getTailSamples(&self) -> uint32 {
        0
    }
}
impl IConnectionPointTrait for Processor {
    unsafe fn connect(&self, _other: *mut IConnectionPoint) -> tresult {
        self.seen.lock().unwrap().connected += 1;
        kResultOk
    }
    unsafe fn disconnect(&self, _other: *mut IConnectionPoint) -> tresult {
        kResultOk
    }
    unsafe fn notify(&self, _message: *mut IMessage) -> tresult {
        kResultOk
    }
}

struct Controller {
    seen: Shared,
    gain: Cell<f64>,
}
impl Class for Controller {
    type Interfaces = (IEditController, IConnectionPoint);
}
impl IPluginBaseTrait for Controller {
    unsafe fn initialize(&self, _context: *mut FUnknown) -> tresult {
        kResultOk
    }
    unsafe fn terminate(&self) -> tresult {
        kResultOk
    }
}
impl IEditControllerTrait for Controller {
    unsafe fn setComponentState(&self, state: *mut IBStream) -> tresult {
        let s = ComRef::from_raw(state).unwrap();
        let mut b = [0u8; 8];
        let mut n = 0;
        s.read(b.as_mut_ptr() as *mut c_void, 8, &mut n);
        self.seen.lock().unwrap().controller_got_state = (n == 8).then(|| f64::from_le_bytes(b));
        kResultOk
    }
    unsafe fn setState(&self, _state: *mut IBStream) -> tresult {
        kResultOk
    }
    unsafe fn getState(&self, _state: *mut IBStream) -> tresult {
        kResultOk
    }
    unsafe fn getParameterCount(&self) -> int32 {
        1
    }
    unsafe fn getParameterInfo(&self, _index: int32, _info: *mut ParameterInfo) -> tresult {
        kNotImplemented
    }
    unsafe fn getParamStringByValue(&self, _id: ParamID, _v: ParamValue, _s: *mut String128) -> tresult {
        kNotImplemented
    }
    unsafe fn getParamValueByString(&self, _id: ParamID, _s: *mut TChar, _v: *mut ParamValue) -> tresult {
        kNotImplemented
    }
    unsafe fn normalizedParamToPlain(&self, _id: ParamID, v: ParamValue) -> ParamValue {
        v
    }
    unsafe fn plainParamToNormalized(&self, _id: ParamID, v: ParamValue) -> ParamValue {
        v
    }
    unsafe fn getParamNormalized(&self, _id: ParamID) -> ParamValue {
        self.gain.get()
    }
    unsafe fn setParamNormalized(&self, _id: ParamID, v: ParamValue) -> tresult {
        self.gain.set(v);
        kResultOk
    }
    unsafe fn setComponentHandler(&self, handler: *mut IComponentHandler) -> tresult {
        self.seen.lock().unwrap().handler = ComRef::from_raw(handler).map(|h| h.to_com_ptr());
        kResultOk
    }
    unsafe fn createView(&self, _name: FIDString) -> *mut IPlugView {
        let v = ComWrapper::new(View { seen: self.seen.clone(), frame: RefCell::new(None) });
        v.to_com_ptr::<IPlugView>().unwrap().into_raw()
    }
}
impl IConnectionPointTrait for Controller {
    unsafe fn connect(&self, _other: *mut IConnectionPoint) -> tresult {
        self.seen.lock().unwrap().connected += 1;
        kResultOk
    }
    unsafe fn disconnect(&self, _other: *mut IConnectionPoint) -> tresult {
        kResultOk
    }
    unsafe fn notify(&self, _message: *mut IMessage) -> tresult {
        kResultOk
    }
}

struct View {
    seen: Shared,
    frame: RefCell<Option<ComPtr<IPlugFrame>>>,
}
impl Class for View {
    type Interfaces = (IPlugView,);
}
impl IPlugViewTrait for View {
    unsafe fn isPlatformTypeSupported(&self, t: FIDString) -> tresult {
        let t = CStr::from_ptr(t);
        if t == CStr::from_ptr(kPlatformTypeX11EmbedWindowID) || t == CStr::from_ptr(kPlatformTypeHWND) {
            kResultTrue
        } else {
            kResultFalse
        }
    }
    unsafe fn attached(&self, parent: *mut c_void, t: FIDString) -> tresult {
        let mut s = self.seen.lock().unwrap();
        s.attached = Some((parent as usize, CStr::from_ptr(t).to_string_lossy().into_owned()));
        s.frame = self.frame.borrow().clone();
        kResultOk
    }
    unsafe fn removed(&self) -> tresult {
        self.seen.lock().unwrap().removed = true;
        kResultOk
    }
    unsafe fn onWheel(&self, _d: f32) -> tresult {
        kResultFalse
    }
    unsafe fn onKeyDown(&self, _k: char16, _c: int16, _m: int16) -> tresult {
        kResultFalse
    }
    unsafe fn onKeyUp(&self, _k: char16, _c: int16, _m: int16) -> tresult {
        kResultFalse
    }
    unsafe fn getSize(&self, size: *mut ViewRect) -> tresult {
        *size = ViewRect { left: 0, top: 0, right: 320, bottom: 200 };
        kResultOk
    }
    unsafe fn onSize(&self, _size: *mut ViewRect) -> tresult {
        kResultOk
    }
    unsafe fn onFocus(&self, _state: TBool) -> tresult {
        kResultOk
    }
    unsafe fn setFrame(&self, frame: *mut IPlugFrame) -> tresult {
        *self.frame.borrow_mut() = ComRef::from_raw(frame).map(|f| f.to_com_ptr());
        kResultOk
    }
    unsafe fn canResize(&self) -> tresult {
        kResultFalse
    }
    unsafe fn checkSizeConstraint(&self, _rect: *mut ViewRect) -> tresult {
        kResultOk
    }
}

struct Timer {
    hits: Arc<Mutex<u32>>,
}
impl Class for Timer {
    type Interfaces = (ITimerHandler,);
}
impl ITimerHandlerTrait for Timer {
    unsafe fn onTimer(&self) {
        *self.hits.lock().unwrap() += 1;
    }
}

/// One stereo block through the plugin's CLAP face, with these notes
/// (key, velocity; velocity 0 ends the note).
fn block(p: &Plugin, frames: usize, notes: &[(i16, f64)], transport: Option<&clap_event_transport>) -> f32 {
    struct Events(Vec<clap_event_note>);
    unsafe extern "C" fn size(l: *const clap_input_events) -> u32 {
        (*((*l).ctx as *const Events)).0.len() as u32
    }
    unsafe extern "C" fn get(l: *const clap_input_events, i: u32) -> *const clap_event_header {
        &(&*((*l).ctx as *const Events)).0[i as usize].header
    }
    let ev = Events(
        notes
            .iter()
            .map(|&(key, velocity)| clap_event_note {
                header: clap_event_header { size: std::mem::size_of::<clap_event_note>() as u32, time: 0, space_id: CLAP_CORE_EVENT_SPACE_ID, type_: if velocity > 0.0 { CLAP_EVENT_NOTE_ON } else { CLAP_EVENT_NOTE_OFF }, flags: 0 },
                note_id: -1,
                port_index: 0,
                channel: 0,
                key,
                velocity,
            })
            .collect(),
    );
    let list = clap_input_events { ctx: &ev as *const Events as *mut c_void, size: Some(size), get: Some(get) };
    let (mut l, mut r) = (vec![0f32; frames], vec![0f32; frames]);
    let mut ptrs = [l.as_mut_ptr(), r.as_mut_ptr()];
    let mut out = clap_audio_buffer { data32: ptrs.as_mut_ptr(), data64: std::ptr::null_mut(), channel_count: 2, latency: 0, constant_mask: 0 };
    let proc = clap_process {
        steady_time: 0,
        frames_count: frames as u32,
        transport: transport.map(|t| t as *const _).unwrap_or(std::ptr::null()),
        audio_inputs: std::ptr::null(),
        audio_outputs: &mut out,
        audio_inputs_count: 0,
        audio_outputs_count: 1,
        in_events: &list,
        out_events: std::ptr::null(),
    };
    assert!(p.rt_start());
    assert_eq!(unsafe { p.rt_process(&proc) }, CLAP_PROCESS_CONTINUE);
    assert_eq!(l, r);
    l[frames - 1]
}

#[test]
fn hosts_a_vst3_instrument() {
    let seen: Shared = Arc::default();
    let factory = ComWrapper::new(Factory { seen: seen.clone() }).to_com_ptr::<IPluginFactory2>().unwrap().upcast::<IPluginFactory>();
    let lib = Library::from_vst3_factory(factory, Path::new("/test/Test Tone.vst3"));
    assert_eq!(lib.format(), Format::Vst3);

    // Only the audio processor is offered, as an instrument.
    let infos = lib.plugins();
    assert_eq!(infos.len(), 1, "{infos:?}");
    let info = &infos[0];
    assert_eq!((info.name.as_str(), info.vendor.as_str(), info.version.as_str()), ("Test Tone", "Corinzia Tests", "1.0.0"));
    assert!(info.is_instrument() && info.format == Format::Vst3);
    assert_eq!(info.id.len(), 32);

    let p = Plugin::create(&lib, &info.id).unwrap();
    {
        let s = seen.lock().unwrap();
        assert_eq!(s.host_name, "Corinzia");
        assert_eq!(s.connected, 2, "processor and controller connected both ways");
        assert_eq!(s.controller_got_state, Some(0.75), "the controller has the processor's state");
        assert!(s.handler.is_some(), "the controller has our component handler");
    }
    assert!(!p.midi_dialect);
    assert_eq!(p.output_channels, 2);
    assert!(p.has_editor());
    p.activate(48000.0, 256).unwrap();

    // Notes in, sound out; the transport arrives as a process context.
    let mut t: clap_event_transport = unsafe { std::mem::zeroed() };
    t.flags = CLAP_TRANSPORT_IS_PLAYING | CLAP_TRANSPORT_HAS_TEMPO;
    t.tempo = 96.0;
    t.song_pos_beats = 2 * CLAP_BEATTIME_FACTOR;
    t.tsig_num = 3;
    t.tsig_denom = 4;
    assert_eq!(block(&p, 64, &[], Some(&t)), 0.0);
    assert_eq!(block(&p, 64, &[(60, 0.5)], Some(&t)), 0.5);
    {
        let s = seen.lock().unwrap();
        assert_eq!(s.tempo, 96.0);
        assert!(s.playing);
    }

    // A parameter moved in the editor reaches the processor in the next block.
    let handler = seen.lock().unwrap().handler.clone().unwrap();
    unsafe {
        assert_eq!(handler.beginEdit(GAIN), kResultOk);
        assert_eq!(handler.performEdit(GAIN, 0.9), kResultOk);
        assert_eq!(handler.performEdit(GAIN, 0.5), kResultOk);
        assert_eq!(handler.endEdit(GAIN), kResultOk);
    }
    assert_eq!(block(&p, 64, &[], None), 0.25, "the latest value of the gesture applies");
    assert_eq!(block(&p, 64, &[(60, 0.0)], None), 0.0);

    // The editor, where there is a display to show it on.
    if std::env::var_os("DISPLAY").is_some() {
        p.open_editor("Test Tone").unwrap();
        assert!(p.editor_is_open());
        let (parent, platform) = seen.lock().unwrap().attached.clone().expect("the view is attached");
        assert_ne!(parent, 0);
        assert_eq!(platform, "X11EmbedWindowID");
        // The frame is the run loop on Linux: timers fire from the app's pump.
        let frame = seen.lock().unwrap().frame.clone().expect("the view has a frame");
        let run_loop = frame.cast::<IRunLoop>().expect("the frame is a run loop");
        let hits = Arc::new(Mutex::new(0));
        let timer = ComWrapper::new(Timer { hits: hits.clone() }).to_com_ptr::<ITimerHandler>().unwrap();
        unsafe { assert_eq!(run_loop.registerTimer(timer.as_ptr(), 1), kResultOk) };
        std::thread::sleep(std::time::Duration::from_millis(5));
        corinzia_audio::gui::pump();
        assert_eq!(*hits.lock().unwrap(), 1);
        unsafe { run_loop.unregisterTimer(timer.as_ptr()) };
        drop((run_loop, frame));
        p.idle();
        p.close_editor();
        assert!(!p.editor_is_open());
        assert!(seen.lock().unwrap().removed);
    } else {
        eprintln!("no DISPLAY: the editor is not opened");
    }
    p.rt_stop();
    drop(p);
    seen.lock().unwrap().handler = None;
}

/// A real VST3 plugin's state survives a save and load (when Dexed is installed).
#[test]
fn a_real_vst3_state_round_trips() {
    let Some(path) = std::env::var_os("CORINZIA_TEST_VST3").map(std::path::PathBuf::from).filter(|p| p.exists()) else { return };
    let lib = corinzia_audio::host::Library::load(&path).unwrap();
    let info = lib.plugins().into_iter().next().unwrap();
    let p = corinzia_audio::host::Plugin::create(&lib, &info.id).unwrap();
    p.activate(48000.0, 256).unwrap();
    let state = p.save_state().expect("a state");
    assert!(state.starts_with(b"CZV3") && state.len() > 8, "{} bytes", state.len());
    assert!(p.load_state(&state));
    assert_eq!(p.save_state().unwrap(), state, "the same after loading");
}
