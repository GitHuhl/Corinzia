//! Plugin hosting: loading bundles (`.clap` shared objects, `.vst3` bundles
//! through [`crate::vst3`], or the built-in Corinzia Sine in-process),
//! creating plugin instances, driving their lifecycle on the right threads,
//! and their editors (in windows of [`crate::gui`]).
//!
//! Threads, as CLAP requires: `create`, `activate`, `deactivate`, `destroy`,
//! state and `on_main_thread` on the main (UI) thread; `start_processing`,
//! `process`, `stop_processing` on the audio thread. A [`Plugin`] is shared
//! between the two through `Arc`; the audio thread only ever calls the
//! audio-thread functions (`Plugin::rt_*`). The engine speaks CLAP to every
//! plugin: a VST3 plugin translates the CLAP process call to its own.

use clap_sys::entry::clap_plugin_entry;
use clap_sys::events::*;
use crate::gui;
use clap_sys::ext::audio_ports::*;
use clap_sys::ext::gui::*;
use clap_sys::ext::log::*;
use clap_sys::ext::params::*;
use clap_sys::ext::posix_fd_support::*;
use clap_sys::ext::timer_support::*;
use clap_sys::ext::note_ports::*;
use clap_sys::ext::state::*;
use clap_sys::ext::thread_check::*;
use clap_sys::factory::plugin_factory::*;
use clap_sys::host::clap_host;
use clap_sys::id::clap_id;
use clap_sys::plugin::*;
use clap_sys::process::*;
use clap_sys::stream::*;
use clap_sys::version::{clap_version_is_compatible, CLAP_VERSION};
use std::cell::Cell;
use std::ffi::{c_char, c_void, CStr, CString};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

thread_local! {
    static AUDIO_THREAD: Cell<bool> = const { Cell::new(false) };
}
/// Marks the calling thread as the audio thread (for the plugin's thread checks).
pub fn mark_audio_thread() {
    AUDIO_THREAD.with(|a| a.set(true));
}
fn is_audio_thread() -> bool {
    AUDIO_THREAD.with(|a| a.get())
}

/// A loaded plugin library: a CLAP bundle on disk, the built-in
/// instrument, or a VST3 module.
pub struct Library {
    kind: LibraryKind,
    pub path: PathBuf,
}
enum LibraryKind {
    Clap {
        _lib: Option<libloading::Library>, // keeps the code mapped
        entry: *const clap_plugin_entry,
    },
    Vst3(crate::vst3::Module),
}
unsafe impl Send for Library {}
unsafe impl Sync for Library {}

impl Drop for Library {
    fn drop(&mut self) {
        if let LibraryKind::Clap { entry, .. } = self.kind {
            unsafe {
                if let Some(deinit) = (*entry).deinit {
                    deinit();
                }
            }
        }
    }
}

/// The plugin formats Corinzia hosts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Format {
    Clap,
    Vst3,
}

impl Format {
    /// The format of a bundle, by its extension.
    pub fn of(path: &Path) -> Format {
        if path.extension().is_some_and(|x| x.eq_ignore_ascii_case("vst3")) {
            Format::Vst3
        } else {
            Format::Clap
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Format::Clap => "CLAP",
            Format::Vst3 => "VST3",
        }
    }
}

/// A plugin parameter (CLAP).
#[derive(Clone, Debug, PartialEq)]
pub struct ParamInfo {
    pub id: u32,
    pub name: String,
    pub min: f64,
    pub max: f64,
    pub default: f64,
    pub stepped: bool,
    pub automatable: bool,
}

/// A CLAP parameter-value event at a frame of the block.
pub fn param_event(id: u32, value: f64, time: u32) -> clap_event_param_value {
    clap_event_param_value {
        header: clap_event_header { size: std::mem::size_of::<clap_event_param_value>() as u32, time, space_id: CLAP_CORE_EVENT_SPACE_ID, type_: CLAP_EVENT_PARAM_VALUE, flags: 0 },
        param_id: id,
        cookie: std::ptr::null_mut(),
        note_id: -1,
        port_index: -1,
        channel: -1,
        key: -1,
        value,
    }
}

/// Bundle path of the built-in instrument.
pub const BUILTIN: &str = "builtin:corinzia-sine";

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PluginInfo {
    /// The CLAP plugin id, or the VST3 class id in hex.
    pub id: String,
    pub name: String,
    pub vendor: String,
    pub version: String,
    /// CLAP features; for VST3, the subcategories in lower case.
    pub features: Vec<String>,
    pub bundle: PathBuf,
    pub format: Format,
}
impl PluginInfo {
    pub fn is_instrument(&self) -> bool {
        self.features.iter().any(|f| f == "instrument")
    }
}

fn cstr(p: *const c_char) -> String {
    if p.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
    }
}

impl Library {
    /// Loads a `.clap` bundle (a shared object) and initialises its entry,
    /// or a `.vst3` bundle and its factory.
    pub fn load(path: &Path) -> Result<Arc<Library>, String> {
        if path.as_os_str() == BUILTIN {
            return Ok(Self::builtin());
        }
        if Format::of(path) == Format::Vst3 {
            let module = crate::vst3::Module::load(path)?;
            return Ok(Arc::new(Library { kind: LibraryKind::Vst3(module), path: path.to_path_buf() }));
        }
        unsafe {
            let lib = libloading::Library::new(path).map_err(|e| format!("{}: {e}", path.display()))?;
            let sym: libloading::Symbol<*const clap_plugin_entry> = lib.get(b"clap_entry\0").map_err(|_| format!("{}: not a CLAP plugin (no clap_entry)", path.display()))?;
            let entry = *sym;
            Self::init(Some(lib), entry, path)
        }
    }

    /// Corinzia Sine, linked into the application.
    pub fn builtin() -> Arc<Library> {
        unsafe { Self::init(None, corinzia_sine::entry(), Path::new(BUILTIN)).expect("built-in instrument") }
    }

    unsafe fn init(lib: Option<libloading::Library>, entry: *const clap_plugin_entry, path: &Path) -> Result<Arc<Library>, String> {
        if entry.is_null() || !clap_version_is_compatible((*entry).clap_version) {
            return Err(format!("{}: incompatible CLAP version", path.display()));
        }
        let c = CString::new(path.to_string_lossy().as_bytes()).unwrap_or_default();
        if !(*entry).init.is_some_and(|f| f(c.as_ptr())) {
            return Err(format!("{}: plugin entry init failed", path.display()));
        }
        Ok(Arc::new(Library { kind: LibraryKind::Clap { _lib: lib, entry }, path: path.to_path_buf() }))
    }

    /// A VST3 module from a factory already in memory (tests).
    #[doc(hidden)]
    pub fn from_vst3_factory(factory: vst3::ComPtr<vst3::Steinberg::IPluginFactory>, path: &Path) -> Arc<Library> {
        Arc::new(Library { kind: LibraryKind::Vst3(crate::vst3::Module::from_factory(factory)), path: path.to_path_buf() })
    }

    pub fn format(&self) -> Format {
        match self.kind {
            LibraryKind::Clap { .. } => Format::Clap,
            LibraryKind::Vst3(_) => Format::Vst3,
        }
    }

    fn factory(&self) -> Option<&clap_plugin_factory> {
        let LibraryKind::Clap { entry, .. } = self.kind else { return None };
        unsafe {
            let f = (*entry).get_factory?(CLAP_PLUGIN_FACTORY_ID.as_ptr()) as *const clap_plugin_factory;
            f.as_ref()
        }
    }

    pub fn plugins(&self) -> Vec<PluginInfo> {
        if let LibraryKind::Vst3(m) = &self.kind {
            return m.classes(&self.path);
        }
        let Some(f) = self.factory() else { return vec![] };
        unsafe {
            let n = f.get_plugin_count.map(|c| c(f)).unwrap_or(0);
            (0..n)
                .filter_map(|i| f.get_plugin_descriptor?(f, i).as_ref())
                .map(|d| {
                    let mut features = Vec::new();
                    let mut p = d.features;
                    while !p.is_null() && !(*p).is_null() {
                        features.push(cstr(*p));
                        p = p.add(1);
                    }
                    PluginInfo { id: cstr(d.id), name: cstr(d.name), vendor: cstr(d.vendor), version: cstr(d.version), features, bundle: self.path.clone(), format: Format::Clap }
                })
                .collect()
        }
    }
}

/// What the host tells the plugin about itself; one per instance.
struct HostData {
    host: clap_host,
    /// The plugin, once created (timers and descriptors call into it).
    plugin: AtomicPtr<clap_plugin>,
    owner: gui::Owner,
    restart: AtomicBool,
    callback: AtomicBool,
    process: AtomicBool,
    dirty: AtomicBool,
    /// GUI requests, handled on the next idle: a new size (w << 32 | h, 0
    /// for none), the plugin closed its window, hints changed.
    resize: AtomicU64,
    closed: AtomicBool,
    destroyed: AtomicBool,
    hints: AtomicBool,
    show: AtomicBool,
}

unsafe extern "C" fn host_get_extension(_h: *const clap_host, id: *const c_char) -> *const c_void {
    let id = CStr::from_ptr(id);
    if id == CLAP_EXT_THREAD_CHECK {
        &THREAD_CHECK as *const _ as *const c_void
    } else if id == CLAP_EXT_STATE {
        &HOST_STATE as *const _ as *const c_void
    } else if id == CLAP_EXT_GUI {
        &HOST_GUI as *const _ as *const c_void
    } else if id == CLAP_EXT_TIMER_SUPPORT {
        &HOST_TIMERS as *const _ as *const c_void
    } else if id == CLAP_EXT_POSIX_FD_SUPPORT && cfg!(unix) {
        &HOST_FDS as *const _ as *const c_void
    } else if id == CLAP_EXT_PARAMS {
        &HOST_PARAMS as *const _ as *const c_void
    } else if id == CLAP_EXT_LOG {
        &HOST_LOG as *const _ as *const c_void
    } else {
        std::ptr::null()
    }
}
unsafe fn host_data<'a>(h: *const clap_host) -> &'a HostData {
    &*((*h).host_data as *const HostData)
}
unsafe extern "C" fn host_request_restart(h: *const clap_host) {
    host_data(h).restart.store(true, Ordering::Release);
}
unsafe extern "C" fn host_request_process(h: *const clap_host) {
    host_data(h).process.store(true, Ordering::Release);
}
unsafe extern "C" fn host_request_callback(h: *const clap_host) {
    host_data(h).callback.store(true, Ordering::Release);
}
unsafe extern "C" fn is_main(_h: *const clap_host) -> bool {
    !is_audio_thread()
}
unsafe extern "C" fn is_audio(_h: *const clap_host) -> bool {
    is_audio_thread()
}
static THREAD_CHECK: clap_host_thread_check = clap_host_thread_check { is_main_thread: Some(is_main), is_audio_thread: Some(is_audio) };
unsafe extern "C" fn mark_dirty(h: *const clap_host) {
    host_data(h).dirty.store(true, Ordering::Release);
}
static HOST_STATE: clap_host_state = clap_host_state { mark_dirty: Some(mark_dirty) };

// GUI requests are recorded and carried out on the next idle (main thread).
unsafe extern "C" fn gui_hints_changed(h: *const clap_host) {
    host_data(h).hints.store(true, Ordering::Release);
}
unsafe extern "C" fn gui_request_resize(h: *const clap_host, width: u32, height: u32) -> bool {
    host_data(h).resize.store((width.max(1) as u64) << 32 | height.max(1) as u64, Ordering::Release);
    true
}
unsafe extern "C" fn gui_request_show(h: *const clap_host) -> bool {
    host_data(h).show.store(true, Ordering::Release);
    true
}
unsafe extern "C" fn gui_request_hide(_h: *const clap_host) -> bool {
    false
}
unsafe extern "C" fn gui_closed(h: *const clap_host, was_destroyed: bool) {
    let d = host_data(h);
    d.destroyed.store(was_destroyed, Ordering::Release);
    d.closed.store(true, Ordering::Release);
}
static HOST_GUI: clap_host_gui = clap_host_gui {
    resize_hints_changed: Some(gui_hints_changed),
    request_resize: Some(gui_request_resize),
    request_show: Some(gui_request_show),
    request_hide: Some(gui_request_hide),
    closed: Some(gui_closed),
};

unsafe extern "C" fn timer_register(h: *const clap_host, period_ms: u32, timer_id: *mut clap_id) -> bool {
    if is_audio_thread() || timer_id.is_null() {
        return false;
    }
    let d = host_data(h) as *const HostData;
    let id = gui::timer_key();
    let call = move || {
        let p = (*d).plugin.load(Ordering::Acquire);
        if let Some(f) = clap_extension::<clap_plugin_timer_support>(p, CLAP_EXT_TIMER_SUPPORT).and_then(|t| t.on_timer) {
            f(p, id as clap_id);
        }
    };
    gui::register_timer((*d).owner, Some(id), period_ms as u64, Rc::new(call));
    *timer_id = id as clap_id;
    true
}
unsafe extern "C" fn timer_unregister(h: *const clap_host, timer_id: clap_id) -> bool {
    gui::unregister_timer(host_data(h).owner, timer_id as u64)
}
static HOST_TIMERS: clap_host_timer_support = clap_host_timer_support { register_timer: Some(timer_register), unregister_timer: Some(timer_unregister) };

unsafe extern "C" fn fd_register(h: *const clap_host, fd: i32, flags: clap_posix_fd_flags) -> bool {
    if is_audio_thread() {
        return false;
    }
    let d = host_data(h) as *const HostData;
    gui::register_fd(
        (*d).owner,
        fd as u64,
        fd,
        flags,
        Rc::new(move |ready| {
            let p = (*d).plugin.load(Ordering::Acquire);
            if let Some(f) = clap_extension::<clap_plugin_posix_fd_support>(p, CLAP_EXT_POSIX_FD_SUPPORT).and_then(|x| x.on_fd) {
                f(p, fd, ready);
            }
        }),
    );
    true
}
unsafe extern "C" fn fd_modify(h: *const clap_host, fd: i32, flags: clap_posix_fd_flags) -> bool {
    gui::modify_fd(host_data(h).owner, fd, flags)
}
unsafe extern "C" fn fd_unregister(h: *const clap_host, fd: i32) -> bool {
    gui::unregister_fds(host_data(h).owner, fd as u64)
}
static HOST_FDS: clap_host_posix_fd_support = clap_host_posix_fd_support { register_fd: Some(fd_register), modify_fd: Some(fd_modify), unregister_fd: Some(fd_unregister) };

// Parameters: the plugin's own GUI changes its parameters and reports them
// in `process`, which the engine calls for every plugin of the plan, playing
// or not; a flush request needs nothing more.
unsafe extern "C" fn params_rescan(_h: *const clap_host, _flags: clap_param_rescan_flags) {}
unsafe extern "C" fn params_clear(_h: *const clap_host, _id: clap_id, _flags: clap_param_clear_flags) {}
unsafe extern "C" fn params_request_flush(h: *const clap_host) {
    host_data(h).process.store(true, Ordering::Release);
}
static HOST_PARAMS: clap_host_params = clap_host_params { rescan: Some(params_rescan), clear: Some(params_clear), request_flush: Some(params_request_flush) };

unsafe extern "C" fn host_log(_h: *const clap_host, severity: clap_log_severity, msg: *const c_char) {
    if severity >= CLAP_LOG_WARNING && !is_audio_thread() {
        eprintln!("corinzia: plugin: {}", cstr(msg));
    }
}
static HOST_LOG: clap_host_log = clap_host_log { log: Some(host_log) };

unsafe fn clap_extension<'a, T>(p: *const clap_plugin, id: &CStr) -> Option<&'a T> {
    if p.is_null() {
        return None;
    }
    let e = (*p).get_extension?(p, id.as_ptr()) as *const T;
    e.as_ref()
}

/// A CLAP instance and what the host keeps for it.
struct Clap {
    plugin: *const clap_plugin,
    host: Box<HostData>,
}

enum Backend {
    Clap(Clap),
    Vst3(Box<crate::vst3::Instance>),
}

/// An open CLAP editor: embedded in a host window, or floating (the
/// plugin's own window).
struct ClapEditor {
    window: Option<gui::Window>,
    resizable: bool,
}

enum Editor {
    Clap(ClapEditor),
    Vst3(crate::vst3::Editor),
}

/// A plugin instance. Created and activated on the main thread; processed
/// on the audio thread through the `rt_*` functions.
pub struct Plugin {
    pub info: PluginInfo,
    backend: Backend,
    active: AtomicBool,
    processing: AtomicBool,
    /// The note dialect the host sends: CLAP notes, or MIDI.
    pub midi_dialect: bool,
    pub output_channels: u32,
    /// Channels of its main audio input: 0 for an instrument, else an effect.
    pub input_channels: u32,
    /// Its timers and descriptors in the run loop.
    owner: gui::Owner,
    /// Main thread only (a Mutex because the plugin is shared with the audio thread).
    editor: Mutex<Option<Editor>>,
    /// Parameter values set from the main thread, waiting for the next
    /// block (the audio thread only ever try-locks it).
    pending: Mutex<Vec<(u32, f64)>>,
    _lib: Arc<Library>, // last: the code stays mapped until the instance is gone
}
unsafe impl Send for Plugin {}
unsafe impl Sync for Plugin {}

#[cfg(target_os = "linux")]
const CLAP_API: &CStr = CLAP_WINDOW_API_X11;
#[cfg(not(target_os = "linux"))]
const CLAP_API: &CStr = CLAP_WINDOW_API_WIN32;

fn clap_window(h: gui::Handle) -> clap_window {
    clap_window {
        api: CLAP_API.as_ptr(),
        specific: match h {
            gui::Handle::X11(id) => clap_window_handle { x11: id as clap_xwnd },
            gui::Handle::Win32(hwnd) => clap_window_handle { win32: hwnd },
        },
    }
}

impl Plugin {
    pub fn create(lib: &Arc<Library>, id: &str) -> Result<Plugin, String> {
        let info = lib.plugins().into_iter().find(|p| p.id == id).ok_or_else(|| format!("{}: no plugin {id}", lib.path.display()))?;
        if info.bundle.as_os_str() != BUILTIN {
            // Some toolkits look the display over when the plugin is made,
            // long before its editor opens. Without a display, nothing to do.
            let _ = gui::prepare();
        }
        let owner = gui::new_owner();
        if let LibraryKind::Vst3(m) = &lib.kind {
            let inst = m.create(id, &info.name, owner)?;
            let output_channels = inst.output_channels();
            let input_channels = inst.input_channels();
            return Ok(Plugin { info, backend: Backend::Vst3(Box::new(inst)), active: AtomicBool::new(false), processing: AtomicBool::new(false), midi_dialect: false, output_channels, input_channels, owner, editor: Mutex::new(None), pending: Mutex::new(Vec::with_capacity(64)), _lib: lib.clone() });
        }
        let f = lib.factory().ok_or("no plugin factory")?;
        let mut host = Box::new(HostData {
            host: clap_host {
                clap_version: CLAP_VERSION,
                host_data: std::ptr::null_mut(),
                name: c"Corinzia".as_ptr(),
                vendor: c"Corinzia".as_ptr(),
                url: c"".as_ptr(),
                version: c"0.1.0".as_ptr(),
                get_extension: Some(host_get_extension),
                request_restart: Some(host_request_restart),
                request_process: Some(host_request_process),
                request_callback: Some(host_request_callback),
            },
            plugin: AtomicPtr::new(std::ptr::null_mut()),
            owner,
            restart: AtomicBool::new(false),
            callback: AtomicBool::new(false),
            process: AtomicBool::new(false),
            dirty: AtomicBool::new(false),
            resize: AtomicU64::new(0),
            closed: AtomicBool::new(false),
            destroyed: AtomicBool::new(false),
            hints: AtomicBool::new(false),
            show: AtomicBool::new(false),
        });
        host.host.host_data = &*host as *const HostData as *mut c_void;
        let cid = CString::new(id).map_err(|_| "bad plugin id")?;
        unsafe {
            let p = f.create_plugin.ok_or("factory cannot create")?(f, &host.host, cid.as_ptr());
            if p.is_null() {
                return Err(format!("{}: could not create {id}", lib.path.display()));
            }
            host.plugin.store(p as *mut clap_plugin, Ordering::Release);
            if !(*p).init.is_some_and(|i| i(p)) {
                gui::forget(owner);
                if let Some(d) = (*p).destroy {
                    d(p);
                }
                return Err(format!("{id}: init failed"));
            }
            let mut plugin = Plugin {
                info,
                backend: Backend::Clap(Clap { plugin: p, host }),
                active: AtomicBool::new(false),
                processing: AtomicBool::new(false),
                midi_dialect: false,
                output_channels: 2,
                input_channels: 0,
                owner,
                editor: Mutex::new(None),
                pending: Mutex::new(Vec::with_capacity(64)),
                _lib: lib.clone(),
            };
            plugin.query_ports();
            Ok(plugin)
        }
    }

    fn clap(&self) -> Option<&Clap> {
        match &self.backend {
            Backend::Clap(c) => Some(c),
            Backend::Vst3(_) => None,
        }
    }

    unsafe fn extension<T>(&self, id: &CStr) -> Option<&T> {
        clap_extension(self.clap()?.plugin, id)
    }

    fn query_ports(&mut self) {
        let Some(c) = self.clap() else { return };
        let plugin = c.plugin;
        unsafe {
            if let Some(np) = self.extension::<clap_plugin_note_ports>(CLAP_EXT_NOTE_PORTS) {
                if np.count.is_some_and(|c| c(plugin, true) > 0) {
                    let mut info: clap_note_port_info = std::mem::zeroed();
                    if np.get.is_some_and(|g| g(plugin, 0, true, &mut info)) {
                        let clap_ok = info.supported_dialects & CLAP_NOTE_DIALECT_CLAP != 0;
                        self.midi_dialect = !clap_ok && info.supported_dialects & CLAP_NOTE_DIALECT_MIDI != 0;
                    }
                }
            }
            if let Some(ap) = self.extension::<clap_plugin_audio_ports>(CLAP_EXT_AUDIO_PORTS).copied() {
                let mut info: clap_audio_port_info = std::mem::zeroed();
                if ap.count.is_some_and(|c| c(plugin, false) > 0) && ap.get.is_some_and(|g| g(plugin, 0, false, &mut info)) {
                    self.output_channels = info.channel_count.max(1);
                }
                let mut info: clap_audio_port_info = std::mem::zeroed();
                if ap.count.is_some_and(|c| c(plugin, true) > 0) && ap.get.is_some_and(|g| g(plugin, 0, true, &mut info)) {
                    self.input_channels = info.channel_count;
                }
            }
        }
    }

    /// Main thread.
    pub fn activate(&self, sample_rate: f64, max_frames: u32) -> Result<(), String> {
        if self.active.load(Ordering::Acquire) {
            return Ok(());
        }
        match &self.backend {
            Backend::Clap(c) => unsafe {
                if !(*c.plugin).activate.is_some_and(|a| a(c.plugin, sample_rate, 1, max_frames)) {
                    return Err(format!("{}: activate failed", self.info.name));
                }
            },
            Backend::Vst3(v) => v.activate(sample_rate, max_frames).map_err(|e| format!("{}: {e}", self.info.name))?,
        }
        self.active.store(true, Ordering::Release);
        Ok(())
    }

    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::Acquire)
    }

    /// Main thread; the audio thread must have stopped processing it.
    pub fn deactivate(&self) {
        if self.active.swap(false, Ordering::AcqRel) {
            match &self.backend {
                Backend::Clap(c) => unsafe {
                    if let Some(d) = (*c.plugin).deactivate {
                        d(c.plugin);
                    }
                },
                Backend::Vst3(v) => v.deactivate(),
            }
        }
    }

    /// Main thread: run deferred plugin work it asked for, and look after
    /// its editor (closed, resized).
    pub fn idle(&self) {
        if let Some(c) = self.clap() {
            if c.host.callback.swap(false, Ordering::AcqRel) {
                unsafe { (*c.plugin).on_main_thread.map(|f| f(c.plugin)) };
            }
        }
        self.editor_idle();
    }

    pub fn wants_restart(&self) -> bool {
        self.clap().is_some_and(|c| c.host.restart.swap(false, Ordering::AcqRel))
    }

    /// Main thread: the plugin's state chunk (CLAP only, for now).
    pub fn save_state(&self) -> Option<Vec<u8>> {
        unsafe extern "C" fn write(s: *const clap_ostream, buf: *const c_void, size: u64) -> i64 {
            let v = &mut *((*s).ctx as *mut Vec<u8>);
            v.extend_from_slice(std::slice::from_raw_parts(buf as *const u8, size as usize));
            size as i64
        }
        let c = self.clap()?;
        unsafe {
            let st = self.extension::<clap_plugin_state>(CLAP_EXT_STATE)?;
            let mut out: Vec<u8> = Vec::new();
            let stream = clap_ostream { ctx: &mut out as *mut Vec<u8> as *mut c_void, write: Some(write) };
            st.save?(c.plugin, &stream).then_some(out)
        }
    }

    /// Main thread.
    pub fn load_state(&self, bytes: &[u8]) -> bool {
        struct Reader<'a>(&'a [u8], usize);
        unsafe extern "C" fn read(s: *const clap_istream, buf: *mut c_void, size: u64) -> i64 {
            let r = &mut *((*s).ctx as *mut Reader);
            let n = (r.0.len() - r.1).min(size as usize);
            std::ptr::copy_nonoverlapping(r.0[r.1..].as_ptr(), buf as *mut u8, n);
            r.1 += n;
            n as i64
        }
        let Some(c) = self.clap() else { return false };
        unsafe {
            let Some(st) = self.extension::<clap_plugin_state>(CLAP_EXT_STATE) else { return false };
            let mut r = Reader(bytes, 0);
            let stream = clap_istream { ctx: &mut r as *mut Reader as *mut c_void, read: Some(read) };
            st.load.is_some_and(|l| l(c.plugin, &stream))
        }
    }

    /// Main thread: the plugin's parameters (CLAP), hidden ones left out.
    pub fn params(&self) -> Vec<ParamInfo> {
        let Some(c) = self.clap() else { return vec![] };
        unsafe {
            let Some(pa) = self.extension::<clap_plugin_params>(CLAP_EXT_PARAMS) else { return vec![] };
            let n = pa.count.map(|f| f(c.plugin)).unwrap_or(0);
            (0..n)
                .filter_map(|i| {
                    let mut info: clap_param_info = std::mem::zeroed();
                    if !pa.get_info?(c.plugin, i, &mut info) || info.flags & CLAP_PARAM_IS_HIDDEN != 0 {
                        return None;
                    }
                    Some(ParamInfo {
                        id: info.id,
                        name: cstr(info.name.as_ptr()),
                        min: info.min_value,
                        max: info.max_value,
                        default: info.default_value,
                        stepped: info.flags & CLAP_PARAM_IS_STEPPED != 0,
                        automatable: info.flags & CLAP_PARAM_IS_AUTOMATABLE != 0,
                    })
                })
                .collect()
        }
    }

    /// Main thread: a parameter's value (a pending change, else the plugin's).
    pub fn param_value(&self, id: u32) -> Option<f64> {
        if let Some(v) = self.pending.lock().ok().and_then(|q| q.iter().rev().find(|x| x.0 == id).map(|x| x.1)) {
            return Some(v);
        }
        let c = self.clap()?;
        unsafe {
            let pa = self.extension::<clap_plugin_params>(CLAP_EXT_PARAMS)?;
            let mut v = 0.0;
            pa.get_value?(c.plugin, id, &mut v).then_some(v)
        }
    }

    /// Main thread: a value as the plugin writes it ("-6.0 dB").
    pub fn param_text(&self, id: u32, value: f64) -> String {
        let fallback = || format!("{value:.2}");
        let Some(c) = self.clap() else { return fallback() };
        unsafe {
            let Some(pa) = self.extension::<clap_plugin_params>(CLAP_EXT_PARAMS) else { return fallback() };
            let mut buf = [0 as c_char; 64];
            match pa.value_to_text {
                Some(f) if f(c.plugin, id, value, buf.as_mut_ptr(), buf.len() as u32) => cstr(buf.as_ptr()),
                _ => fallback(),
            }
        }
    }

    /// Main thread: sets a parameter. While the plugin processes, the
    /// change goes with its next block; otherwise it is flushed now.
    pub fn set_param(&self, id: u32, value: f64) {
        if self.is_processing() {
            if let Ok(mut q) = self.pending.lock() {
                q.retain(|x| x.0 != id);
                q.push((id, value));
            }
            return;
        }
        let Some(c) = self.clap() else { return };
        unsafe {
            let Some(pa) = self.extension::<clap_plugin_params>(CLAP_EXT_PARAMS) else { return };
            let Some(flush) = pa.flush else { return };
            let ev = param_event(id, value, 0);
            struct One(clap_event_param_value);
            unsafe extern "C" fn size(_l: *const clap_input_events) -> u32 {
                1
            }
            unsafe extern "C" fn get(l: *const clap_input_events, _i: u32) -> *const clap_event_header {
                &(*((*l).ctx as *const One)).0.header
            }
            unsafe extern "C" fn push(_l: *const clap_output_events, _e: *const clap_event_header) -> bool {
                true
            }
            let one = One(ev);
            let input = clap_input_events { ctx: &one as *const One as *mut c_void, size: Some(size), get: Some(get) };
            let output = clap_output_events { ctx: std::ptr::null_mut(), try_push: Some(push) };
            flush(c.plugin, &input, &output);
        }
    }

    /// Audio thread: hands the waiting parameter changes to `f` (never
    /// blocks: if the main thread holds the queue, they wait a block).
    pub fn rt_take_params(&self, mut f: impl FnMut(u32, f64)) {
        if let Ok(mut q) = self.pending.try_lock() {
            for (id, v) in q.drain(..) {
                f(id, v);
            }
        }
    }

    /// The CLAP plugin (null for a VST3 plugin).
    pub fn raw(&self) -> *const clap_plugin {
        self.clap().map(|c| c.plugin).unwrap_or(std::ptr::null())
    }

    /// The VST3 instance (tests).
    #[doc(hidden)]
    pub fn vst3(&self) -> Option<&crate::vst3::Instance> {
        match &self.backend {
            Backend::Vst3(v) => Some(v),
            Backend::Clap(_) => None,
        }
    }

    // ------------------------------------------------------------ editor --

    /// Whether the plugin has an editor of its own that can be shown here.
    pub fn has_editor(&self) -> bool {
        match &self.backend {
            Backend::Clap(c) => unsafe {
                self.extension::<clap_plugin_gui>(CLAP_EXT_GUI)
                    .and_then(|g| g.is_api_supported)
                    .is_some_and(|ok| ok(c.plugin, CLAP_API.as_ptr(), false) || ok(c.plugin, CLAP_API.as_ptr(), true))
            },
            Backend::Vst3(v) => v.has_editor(),
        }
    }

    pub fn editor_is_open(&self) -> bool {
        self.editor.lock().map(|e| e.is_some()).unwrap_or(false)
    }

    /// Main thread: shows the plugin's editor in a window (brings it to the
    /// front when it is already open). `title` names the window.
    pub fn open_editor(&self, title: &str) -> Result<(), String> {
        let mut slot = self.editor.lock().map_err(|_| "editor unavailable")?;
        if let Some(e) = slot.as_ref() {
            match e {
                Editor::Clap(ClapEditor { window: Some(w), .. }) => w.raise(),
                Editor::Clap(ClapEditor { window: None, .. }) => unsafe {
                    if let (Some(g), Some(c)) = (self.extension::<clap_plugin_gui>(CLAP_EXT_GUI), self.clap()) {
                        if let Some(s) = g.show {
                            s(c.plugin);
                        }
                    }
                },
                Editor::Vst3(v) => v.raise(),
            }
            return Ok(());
        }
        gui::prepare()?;
        let editor = match &self.backend {
            Backend::Clap(c) => Editor::Clap(self.open_clap_editor(c, title)?),
            Backend::Vst3(v) => Editor::Vst3(v.open_editor(title)?),
        };
        *slot = Some(editor);
        Ok(())
    }

    fn open_clap_editor(&self, c: &Clap, title: &str) -> Result<ClapEditor, String> {
        let p = c.plugin;
        let none = || format!("{} has no editor", self.info.name);
        unsafe {
            let g = self.extension::<clap_plugin_gui>(CLAP_EXT_GUI).ok_or_else(none)?;
            let supported = |floating| g.is_api_supported.is_some_and(|f| f(p, CLAP_API.as_ptr(), floating));
            // Embedded in our window when the plugin can; floating otherwise.
            let floating = if supported(false) {
                false
            } else if supported(true) {
                true
            } else {
                return Err(none());
            };
            for d in [&c.host.closed, &c.host.destroyed, &c.host.hints, &c.host.show] {
                d.store(false, Ordering::Release);
            }
            c.host.resize.store(0, Ordering::Release);
            if !g.create.is_some_and(|f| f(p, CLAP_API.as_ptr(), floating)) {
                return Err(format!("{}: could not create its editor", self.info.name));
            }
            if floating {
                let t = CString::new(title).unwrap_or_default();
                if let Some(f) = g.suggest_title {
                    f(p, t.as_ptr());
                }
                if let Some(f) = g.show {
                    f(p);
                }
                return Ok(ClapEditor { window: None, resizable: false });
            }
            let (mut w, mut h) = (0u32, 0u32);
            if !g.get_size.is_some_and(|f| f(p, &mut w, &mut h)) || w == 0 || h == 0 {
                (w, h) = (640, 480);
            }
            let resizable = g.can_resize.is_some_and(|f| f(p));
            let window = match gui::Window::new(title, w, h, resizable) {
                Ok(w) => w,
                Err(e) => {
                    if let Some(f) = g.destroy {
                        f(p);
                    }
                    return Err(e);
                }
            };
            let cw = clap_window(window.handle());
            if !g.set_parent.is_some_and(|f| f(p, &cw)) {
                if let Some(f) = g.destroy {
                    f(p);
                }
                return Err(format!("{}: its editor would not open in a window", self.info.name));
            }
            if let Some(f) = g.show {
                f(p);
            }
            window.show();
            Ok(ClapEditor { window: Some(window), resizable })
        }
    }

    /// Main thread: closes the editor if it is open.
    pub fn close_editor(&self) {
        let Some(editor) = self.editor.lock().ok().and_then(|mut e| e.take()) else { return };
        self.drop_editor(editor, false);
    }

    fn drop_editor(&self, editor: Editor, plugin_destroyed_it: bool) {
        match editor {
            Editor::Clap(ed) => {
                if let (Some(c), false) = (self.clap(), plugin_destroyed_it) {
                    unsafe {
                        if let Some(g) = self.extension::<clap_plugin_gui>(CLAP_EXT_GUI) {
                            if let Some(f) = g.hide {
                                f(c.plugin);
                            }
                            if let Some(f) = g.destroy {
                                f(c.plugin);
                            }
                        }
                    }
                }
                drop(ed.window); // after the plugin let go of it
            }
            Editor::Vst3(v) => v.close(),
        }
    }

    fn editor_idle(&self) {
        let Ok(mut slot) = self.editor.lock() else { return };
        let Some(editor) = slot.as_mut() else { return };
        let close = match editor {
            Editor::Vst3(v) => v.idle(),
            Editor::Clap(ed) => {
                let Some(c) = self.clap() else { return };
                let p = c.plugin;
                let Some(g) = (unsafe { self.extension::<clap_plugin_gui>(CLAP_EXT_GUI) }) else { return };
                if c.host.closed.swap(false, Ordering::AcqRel) {
                    let destroyed = c.host.destroyed.load(Ordering::Acquire);
                    let e = slot.take().unwrap();
                    drop(slot);
                    self.drop_editor(e, destroyed);
                    return;
                }
                let mut close = false;
                if let Some(w) = &ed.window {
                    unsafe {
                        if c.host.hints.swap(false, Ordering::AcqRel) {
                            ed.resizable = g.can_resize.is_some_and(|f| f(p));
                            let (mut cw, mut ch) = (0, 0);
                            if g.get_size.is_some_and(|f| f(p, &mut cw, &mut ch)) {
                                w.set_resizable(ed.resizable, cw, ch);
                            }
                        }
                        let req = c.host.resize.swap(0, Ordering::AcqRel);
                        if req != 0 {
                            w.set_size((req >> 32) as u32, req as u32);
                        }
                        if c.host.show.swap(false, Ordering::AcqRel) {
                            w.raise();
                        }
                        let ev = w.events();
                        close = ev.close;
                        if let (Some((mut nw, mut nh)), true, false) = (ev.resized, ed.resizable, close) {
                            let (rw, rh) = (nw, nh);
                            if let Some(f) = g.adjust_size {
                                f(p, &mut nw, &mut nh);
                            }
                            if let Some(f) = g.set_size {
                                f(p, nw, nh);
                            }
                            if (nw, nh) != (rw, rh) {
                                w.set_size(nw, nh);
                            }
                        }
                    }
                }
                close
            }
        };
        if close {
            let e = slot.take().unwrap();
            drop(slot);
            self.drop_editor(e, false);
        }
    }

    // ------------------------------------------------------- audio thread --
    pub fn rt_start(&self) -> bool {
        if !self.is_active() {
            return false;
        }
        if !self.processing.load(Ordering::Acquire) {
            let ok = match &self.backend {
                Backend::Clap(c) => unsafe { (*c.plugin).start_processing.is_some_and(|s| s(c.plugin)) },
                Backend::Vst3(v) => v.rt_start(),
            };
            self.processing.store(ok, Ordering::Release);
        }
        self.processing.load(Ordering::Acquire)
    }

    pub fn rt_stop(&self) {
        if self.processing.swap(false, Ordering::AcqRel) {
            match &self.backend {
                Backend::Clap(c) => unsafe {
                    if let Some(s) = (*c.plugin).stop_processing {
                        s(c.plugin);
                    }
                },
                Backend::Vst3(v) => v.rt_stop(),
            }
        }
    }

    pub fn is_processing(&self) -> bool {
        self.processing.load(Ordering::Acquire)
    }

    /// # Safety
    /// Audio thread only, between `rt_start` and `rt_stop`.
    pub unsafe fn rt_process(&self, p: &clap_process) -> clap_process_status {
        match &self.backend {
            Backend::Clap(c) => match (*c.plugin).process {
                Some(f) => f(c.plugin, p),
                None => CLAP_PROCESS_ERROR,
            },
            Backend::Vst3(v) => v.rt_process(p),
        }
    }
}

impl Drop for Plugin {
    fn drop(&mut self) {
        // Main thread, after the audio thread let go (see engine::Engine).
        self.close_editor();
        // Should not be processing; stopping here is better than leaking a running plugin.
        self.rt_stop();
        self.deactivate();
        gui::forget(self.owner);
        if let Backend::Clap(c) = &self.backend {
            unsafe { (*c.plugin).destroy.map(|d| d(c.plugin)) };
        }
        // A VST3 instance terminates when its backend drops, before the library.
    }
}

/// Where plugins live, CLAP and VST3:
/// * CLAP: `CLAP_PATH`, then on Linux ~/.clap, /usr/lib/clap and the NixOS
///   profile directories; on Windows %LOCALAPPDATA%\Programs\Common\CLAP
///   and %COMMONPROGRAMFILES%\CLAP (the CLAP specification's);
/// * VST3: `VST3_PATH`, then on Linux ~/.vst3, /usr/lib/vst3,
///   /usr/local/lib/vst3 and the NixOS profiles; on Windows
///   %LOCALAPPDATA%\Programs\Common\VST3 and %COMMONPROGRAMFILES%\VST3
///   (Steinberg's).
pub fn search_paths() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = Vec::new();
    for (var, dir) in [("CLAP_PATH", "clap"), ("VST3_PATH", "vst3")] {
        v.extend(std::env::var(var).map(|p| std::env::split_paths(&p).collect::<Vec<_>>()).unwrap_or_default());
        if cfg!(windows) {
            let folder = dir.to_uppercase();
            if let Ok(local) = std::env::var("LOCALAPPDATA") {
                v.push(PathBuf::from(local).join("Programs").join("Common").join(&folder));
            }
            if let Ok(common) = std::env::var("COMMONPROGRAMFILES") {
                v.push(PathBuf::from(common).join(&folder));
            }
            continue;
        }
        if let Ok(home) = std::env::var("HOME") {
            v.push(PathBuf::from(&home).join(format!(".{dir}")));
            v.push(PathBuf::from(&home).join(format!(".nix-profile/lib/{dir}")));
        }
        if let Ok(user) = std::env::var("USER") {
            v.push(PathBuf::from(format!("/etc/profiles/per-user/{user}/lib/{dir}")));
        }
        v.push(PathBuf::from(format!("/run/current-system/sw/lib/{dir}")));
        v.push(PathBuf::from(format!("/usr/lib/{dir}")));
        v.push(PathBuf::from(format!("/usr/local/lib/{dir}")));
    }
    let mut seen = std::collections::HashSet::new();
    v.retain(|p| seen.insert(p.clone()));
    v
}

/// Every plugin bundle under the search paths (recursively): `.clap` files
/// (on Linux, plain shared objects), and `.vst3` bundles (directories, or
/// single-file modules), which are not looked into.
pub fn find_bundles(paths: &[PathBuf]) -> Vec<PathBuf> {
    fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            let ext = p.extension().map(|x| x.to_ascii_lowercase());
            let clap = ext.as_deref() == Some("clap".as_ref()) && p.is_file();
            if clap || ext.as_deref() == Some("vst3".as_ref()) {
                out.push(p);
            } else if p.is_dir() && depth < 4 {
                walk(&p, depth + 1, out);
            }
        }
    }
    let mut out = Vec::new();
    for p in paths {
        walk(p, 0, &mut out);
    }
    out.sort();
    out.dedup();
    out
}

/// The plugins of every bundle found, plus the built-in instrument first.
/// Bundles that fail to load are reported, not fatal.
pub fn scan(paths: &[PathBuf]) -> (Vec<PluginInfo>, Vec<String>) {
    let mut infos = Library::builtin().plugins();
    let mut errors = Vec::new();
    for b in find_bundles(paths) {
        match Library::load(&b) {
            Ok(lib) => infos.extend(lib.plugins()),
            Err(e) => errors.push(e),
        }
    }
    (infos, errors)
}
