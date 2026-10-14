//! CLAP hosting: loading bundles (`.clap` shared objects, or the built-in
//! Corinzia Sine in-process), creating plugin instances and driving their
//! lifecycle on the right threads.
//!
//! Threads, as CLAP requires: `create`, `activate`, `deactivate`, `destroy`,
//! state and `on_main_thread` on the main (UI) thread; `start_processing`,
//! `process`, `stop_processing` on the audio thread. A [`Plugin`] is shared
//! between the two through `Arc`; the audio thread only ever calls the
//! audio-thread functions (`Plugin::rt_*`).

use clap_sys::entry::clap_plugin_entry;
use clap_sys::ext::audio_ports::*;
use clap_sys::ext::note_ports::*;
use clap_sys::ext::state::*;
use clap_sys::ext::thread_check::*;
use clap_sys::factory::plugin_factory::*;
use clap_sys::host::clap_host;
use clap_sys::plugin::*;
use clap_sys::process::*;
use clap_sys::stream::*;
use clap_sys::version::{clap_version_is_compatible, CLAP_VERSION};
use std::cell::Cell;
use std::ffi::{c_char, c_void, CStr, CString};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

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

/// A loaded CLAP library: a bundle on disk, or the built-in instrument.
pub struct Library {
    _lib: Option<libloading::Library>, // keeps the code mapped
    entry: *const clap_plugin_entry,
    pub path: PathBuf,
}
unsafe impl Send for Library {}
unsafe impl Sync for Library {}

impl Drop for Library {
    fn drop(&mut self) {
        unsafe {
            if let Some(deinit) = (*self.entry).deinit {
                deinit();
            }
        }
    }
}

/// Bundle path of the built-in instrument.
pub const BUILTIN: &str = "builtin:corinzia-sine";

#[derive(Clone, Debug, PartialEq)]
pub struct PluginInfo {
    pub id: String,
    pub name: String,
    pub vendor: String,
    pub version: String,
    pub features: Vec<String>,
    pub bundle: PathBuf,
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
    /// Loads a `.clap` bundle (a shared object) and initialises its entry.
    pub fn load(path: &Path) -> Result<Arc<Library>, String> {
        if path.as_os_str() == BUILTIN {
            return Ok(Self::builtin());
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
        Ok(Arc::new(Library { _lib: lib, entry, path: path.to_path_buf() }))
    }

    fn factory(&self) -> Option<&clap_plugin_factory> {
        unsafe {
            let f = (*self.entry).get_factory?(CLAP_PLUGIN_FACTORY_ID.as_ptr()) as *const clap_plugin_factory;
            f.as_ref()
        }
    }

    pub fn plugins(&self) -> Vec<PluginInfo> {
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
                    PluginInfo { id: cstr(d.id), name: cstr(d.name), vendor: cstr(d.vendor), version: cstr(d.version), features, bundle: self.path.clone() }
                })
                .collect()
        }
    }
}

/// What the host tells the plugin about itself; one per instance.
struct HostData {
    host: clap_host,
    restart: AtomicBool,
    callback: AtomicBool,
    process: AtomicBool,
    dirty: AtomicBool,
}

unsafe extern "C" fn host_get_extension(_h: *const clap_host, id: *const c_char) -> *const c_void {
    let id = CStr::from_ptr(id);
    if id == CLAP_EXT_THREAD_CHECK {
        &THREAD_CHECK as *const _ as *const c_void
    } else if id == CLAP_EXT_STATE {
        &HOST_STATE as *const _ as *const c_void
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

/// A plugin instance. Created and activated on the main thread; processed
/// on the audio thread through the `rt_*` functions.
pub struct Plugin {
    pub info: PluginInfo,
    plugin: *const clap_plugin,
    host: Box<HostData>,
    _lib: Arc<Library>,
    active: AtomicBool,
    processing: AtomicBool,
    /// The note dialect the host sends: CLAP notes, or MIDI.
    pub midi_dialect: bool,
    pub output_channels: u32,
}
unsafe impl Send for Plugin {}
unsafe impl Sync for Plugin {}

impl Plugin {
    pub fn create(lib: &Arc<Library>, id: &str) -> Result<Plugin, String> {
        let info = lib.plugins().into_iter().find(|p| p.id == id).ok_or_else(|| format!("{}: no plugin {id}", lib.path.display()))?;
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
            restart: AtomicBool::new(false),
            callback: AtomicBool::new(false),
            process: AtomicBool::new(false),
            dirty: AtomicBool::new(false),
        });
        host.host.host_data = &*host as *const HostData as *mut c_void;
        let cid = CString::new(id).map_err(|_| "bad plugin id")?;
        unsafe {
            let p = f.create_plugin.ok_or("factory cannot create")?(f, &host.host, cid.as_ptr());
            if p.is_null() {
                return Err(format!("{}: could not create {id}", lib.path.display()));
            }
            if !(*p).init.is_some_and(|i| i(p)) {
                (*p).destroy.map(|d| d(p));
                return Err(format!("{id}: init failed"));
            }
            let mut plugin = Plugin { info, plugin: p, host, _lib: lib.clone(), active: AtomicBool::new(false), processing: AtomicBool::new(false), midi_dialect: false, output_channels: 2 };
            plugin.query_ports();
            Ok(plugin)
        }
    }

    unsafe fn extension<T>(&self, id: &CStr) -> Option<&T> {
        let e = (*self.plugin).get_extension?(self.plugin, id.as_ptr()) as *const T;
        e.as_ref()
    }

    fn query_ports(&mut self) {
        unsafe {
            if let Some(np) = self.extension::<clap_plugin_note_ports>(CLAP_EXT_NOTE_PORTS) {
                if np.count.is_some_and(|c| c(self.plugin, true) > 0) {
                    let mut info: clap_note_port_info = std::mem::zeroed();
                    if np.get.is_some_and(|g| g(self.plugin, 0, true, &mut info)) {
                        let clap_ok = info.supported_dialects & CLAP_NOTE_DIALECT_CLAP != 0;
                        self.midi_dialect = !clap_ok && info.supported_dialects & CLAP_NOTE_DIALECT_MIDI != 0;
                    }
                }
            }
            if let Some(ap) = self.extension::<clap_plugin_audio_ports>(CLAP_EXT_AUDIO_PORTS) {
                let mut info: clap_audio_port_info = std::mem::zeroed();
                if ap.count.is_some_and(|c| c(self.plugin, false) > 0) && ap.get.is_some_and(|g| g(self.plugin, 0, false, &mut info)) {
                    self.output_channels = info.channel_count.max(1);
                }
            }
        }
    }

    /// Main thread.
    pub fn activate(&self, sample_rate: f64, max_frames: u32) -> Result<(), String> {
        if self.active.load(Ordering::Acquire) {
            return Ok(());
        }
        unsafe {
            if !(*self.plugin).activate.is_some_and(|a| a(self.plugin, sample_rate, 1, max_frames)) {
                return Err(format!("{}: activate failed", self.info.name));
            }
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
            unsafe { (*self.plugin).deactivate.map(|d| d(self.plugin)) };
        }
    }

    /// Main thread: run deferred plugin work it asked for.
    pub fn idle(&self) {
        if self.host.callback.swap(false, Ordering::AcqRel) {
            unsafe { (*self.plugin).on_main_thread.map(|f| f(self.plugin)) };
        }
    }

    pub fn wants_restart(&self) -> bool {
        self.host.restart.swap(false, Ordering::AcqRel)
    }

    /// Main thread: the plugin's state chunk.
    pub fn save_state(&self) -> Option<Vec<u8>> {
        unsafe extern "C" fn write(s: *const clap_ostream, buf: *const c_void, size: u64) -> i64 {
            let v = &mut *((*s).ctx as *mut Vec<u8>);
            v.extend_from_slice(std::slice::from_raw_parts(buf as *const u8, size as usize));
            size as i64
        }
        unsafe {
            let st = self.extension::<clap_plugin_state>(CLAP_EXT_STATE)?;
            let mut out: Vec<u8> = Vec::new();
            let stream = clap_ostream { ctx: &mut out as *mut Vec<u8> as *mut c_void, write: Some(write) };
            st.save?(self.plugin, &stream).then_some(out)
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
        unsafe {
            let Some(st) = self.extension::<clap_plugin_state>(CLAP_EXT_STATE) else { return false };
            let mut r = Reader(bytes, 0);
            let stream = clap_istream { ctx: &mut r as *mut Reader as *mut c_void, read: Some(read) };
            st.load.is_some_and(|l| l(self.plugin, &stream))
        }
    }

    pub fn raw(&self) -> *const clap_plugin {
        self.plugin
    }

    // ------------------------------------------------------- audio thread --
    pub fn rt_start(&self) -> bool {
        if !self.is_active() {
            return false;
        }
        if !self.processing.load(Ordering::Acquire) {
            let ok = unsafe { (*self.plugin).start_processing.is_some_and(|s| s(self.plugin)) };
            self.processing.store(ok, Ordering::Release);
        }
        self.processing.load(Ordering::Acquire)
    }

    pub fn rt_stop(&self) {
        if self.processing.swap(false, Ordering::AcqRel) {
            unsafe { (*self.plugin).stop_processing.map(|s| s(self.plugin)) };
        }
    }

    pub fn is_processing(&self) -> bool {
        self.processing.load(Ordering::Acquire)
    }

    /// # Safety
    /// Audio thread only, between `rt_start` and `rt_stop`.
    pub unsafe fn rt_process(&self, p: &clap_process) -> clap_process_status {
        match (*self.plugin).process {
            Some(f) => f(self.plugin, p),
            None => CLAP_PROCESS_ERROR,
        }
    }
}

impl Drop for Plugin {
    fn drop(&mut self) {
        // Main thread, after the audio thread let go (see engine::Engine).
        if self.processing.load(Ordering::Acquire) {
            // Should not happen; stopping here is better than leaking a running plugin.
            unsafe { (*self.plugin).stop_processing.map(|s| s(self.plugin)) };
        }
        self.deactivate();
        unsafe { (*self.plugin).destroy.map(|d| d(self.plugin)) };
    }
}

/// Where CLAP plugins live: CLAP_PATH, ~/.clap, /usr/lib/clap, and the NixOS
/// profile directories.
pub fn search_paths() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::env::var("CLAP_PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    if let Ok(home) = std::env::var("HOME") {
        v.push(PathBuf::from(&home).join(".clap"));
        v.push(PathBuf::from(&home).join(".nix-profile/lib/clap"));
    }
    if let Ok(user) = std::env::var("USER") {
        v.push(PathBuf::from(format!("/etc/profiles/per-user/{user}/lib/clap")));
    }
    v.push(PathBuf::from("/run/current-system/sw/lib/clap"));
    v.push(PathBuf::from("/usr/lib/clap"));
    v.push(PathBuf::from("/usr/local/lib/clap"));
    v.dedup();
    v
}

/// Every `.clap` bundle under the search paths (recursively; Linux bundles
/// are plain shared objects named *.clap).
pub fn find_bundles(paths: &[PathBuf]) -> Vec<PathBuf> {
    fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x == "clap") && p.is_file() {
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
