//! Plugin editor windows and the main-thread run loop that plugin GUIs need.
//!
//! * [`Window`] is a plain top-level window the host owns. A plugin's editor
//!   is embedded into it (CLAP `set_parent`, VST3 `IPlugView::attached`):
//!   an X11 window on Linux (XWayland under Wayland), an HWND on Windows.
//! * The run loop carries the timers and file descriptors plugins register
//!   (CLAP `timer-support` and `posix-fd-support`, VST3 `Linux::IRunLoop`).
//!   On Linux, plugin GUIs have no event loop of their own: the host drives
//!   them through these. On Windows, plugins use the thread's message loop
//!   (winit's), so only the timers are used there.
//!
//! Everything here lives on the main (UI) thread; [`pump`] is called by
//! the application several dozen times a second while an editor is open.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

/// Who registered a timer or descriptor: a plugin instance, so that what it
/// left behind can be dropped with it.
pub type Owner = u64;

static NEXT_OWNER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
pub fn new_owner() -> Owner {
    NEXT_OWNER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

// ----------------------------------------------------------- run loop --

/// Readiness flags of a descriptor (the CLAP values).
pub const FD_READ: u32 = 1;
pub const FD_WRITE: u32 = 2;
pub const FD_ERROR: u32 = 4;

pub type TimerCall = Rc<dyn Fn()>;
pub type FdCall = Rc<dyn Fn(u32)>;

struct Timer {
    owner: Owner,
    key: u64,
    period: Duration,
    due: Instant,
    call: TimerCall,
}

struct Fd {
    owner: Owner,
    key: u64,
    fd: i32,
    flags: u32,
    #[cfg_attr(not(unix), allow(dead_code))] // descriptors are a POSIX affair
    call: FdCall,
}

#[derive(Default)]
struct RunLoop {
    timers: Vec<Timer>,
    fds: Vec<Fd>,
    next_timer: u64,
}

thread_local! {
    static RUN_LOOP: RefCell<RunLoop> = RefCell::default();
}

/// A fresh timer key.
pub fn timer_key() -> u64 {
    RUN_LOOP.with(|r| {
        let mut r = r.borrow_mut();
        r.next_timer += 1;
        r.next_timer
    })
}

/// Registers a periodic timer; `key` identifies it for [`unregister_timer`]
/// (`None` allocates one, which is returned).
pub fn register_timer(owner: Owner, key: Option<u64>, period_ms: u64, call: TimerCall) -> u64 {
    let key = key.unwrap_or_else(timer_key);
    RUN_LOOP.with(|r| {
        let mut r = r.borrow_mut();
        let period = Duration::from_millis(period_ms.clamp(1, 60_000));
        r.timers.push(Timer { owner, key, period, due: Instant::now() + period, call });
        key
    })
}

pub fn unregister_timer(owner: Owner, key: u64) -> bool {
    RUN_LOOP.with(|r| {
        let mut r = r.borrow_mut();
        let n = r.timers.len();
        r.timers.retain(|t| !(t.owner == owner && t.key == key));
        r.timers.len() != n
    })
}

/// Watches a descriptor; `flags` are [`FD_READ`] | [`FD_WRITE`] | [`FD_ERROR`].
pub fn register_fd(owner: Owner, key: u64, fd: i32, flags: u32, call: FdCall) {
    RUN_LOOP.with(|r| r.borrow_mut().fds.push(Fd { owner, key, fd, flags, call }));
}

pub fn modify_fd(owner: Owner, fd: i32, flags: u32) -> bool {
    RUN_LOOP.with(|r| {
        let mut r = r.borrow_mut();
        let mut found = false;
        for f in r.fds.iter_mut().filter(|f| f.owner == owner && f.fd == fd) {
            f.flags = flags;
            found = true;
        }
        found
    })
}

/// Stops watching what `owner` registered under `key` (a descriptor, or an
/// event handler that may watch several).
pub fn unregister_fds(owner: Owner, key: u64) -> bool {
    RUN_LOOP.with(|r| {
        let mut r = r.borrow_mut();
        let n = r.fds.len();
        r.fds.retain(|f| !(f.owner == owner && f.key == key));
        r.fds.len() != n
    })
}

/// Forgets every timer and descriptor of an owner (its plugin is going).
pub fn forget(owner: Owner) {
    RUN_LOOP.with(|r| {
        let mut r = r.borrow_mut();
        r.timers.retain(|t| t.owner != owner);
        r.fds.retain(|f| f.owner != owner);
    });
}

/// Whether anything is registered (the app keeps pumping while so).
pub fn has_work() -> bool {
    RUN_LOOP.with(|r| {
        let r = r.borrow();
        !r.timers.is_empty() || !r.fds.is_empty()
    }) || WINDOWS.with(|w| w.get() > 0)
}

/// Main thread: delivers window events, ready descriptors and due timers.
/// Callbacks may register and unregister freely (nothing is borrowed while
/// they run).
pub fn pump() {
    platform::pump_events();
    let ready: Vec<(FdCall, u32)> = RUN_LOOP.with(|r| poll_fds(&r.borrow().fds));
    for (call, flags) in ready {
        call(flags);
    }
    let now = Instant::now();
    let due: Vec<(Owner, u64, TimerCall)> = RUN_LOOP.with(|r| {
        let mut r = r.borrow_mut();
        r.timers
            .iter_mut()
            .filter(|t| t.due <= now)
            .map(|t| {
                // Late timers fire once, not once per missed period.
                t.due = (t.due + t.period).max(now + t.period / 2);
                (t.owner, t.key, t.call.clone())
            })
            .collect()
    });
    for (owner, key, call) in due {
        // A timer unregistered by an earlier callback of this round does not fire.
        if RUN_LOOP.with(|r| r.borrow().timers.iter().any(|t| t.owner == owner && t.key == key)) {
            call();
        }
    }
}

#[cfg(unix)]
fn poll_fds(fds: &[Fd]) -> Vec<(FdCall, u32)> {
    if fds.is_empty() {
        return Vec::new();
    }
    let mut p: Vec<libc::pollfd> = fds
        .iter()
        .map(|f| {
            let mut events = 0;
            if f.flags & FD_READ != 0 {
                events |= libc::POLLIN;
            }
            if f.flags & FD_WRITE != 0 {
                events |= libc::POLLOUT;
            }
            libc::pollfd { fd: f.fd, events, revents: 0 }
        })
        .collect();
    let n = unsafe { libc::poll(p.as_mut_ptr(), p.len() as libc::nfds_t, 0) };
    if n <= 0 {
        return Vec::new();
    }
    fds.iter()
        .zip(&p)
        .filter(|(_, p)| p.revents != 0)
        .map(|(f, p)| {
            let mut flags = 0;
            if p.revents & libc::POLLIN != 0 {
                flags |= FD_READ;
            }
            if p.revents & libc::POLLOUT != 0 {
                flags |= FD_WRITE;
            }
            if p.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
                flags |= FD_ERROR;
            }
            (f.call.clone(), flags)
        })
        .collect()
}

#[cfg(not(unix))]
fn poll_fds(_fds: &[Fd]) -> Vec<(FdCall, u32)> {
    Vec::new()
}

// ------------------------------------------------------------- windows --

thread_local! {
    static WINDOWS: Cell<usize> = const { Cell::new(0) };
}

/// The native handle a plugin embeds its editor into.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Handle {
    X11(u32),
    Win32(*mut std::ffi::c_void),
}

/// What happened to a window since the last look.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Events {
    /// The user closed it.
    pub close: bool,
    /// Its client area has a new size.
    pub resized: Option<(u32, u32)>,
}

/// A top-level window holding a plugin editor. Main thread only.
pub struct Window {
    inner: platform::Window,
}

/// Readies the windowing system before a plugin creates its editor: on
/// X11, connects and makes sure the window-manager atoms that plugin
/// toolkits look up exist (on a bare X server they might not).
pub fn prepare() -> Result<(), String> {
    platform::prepare()
}

impl Window {
    /// A window with a client area of `width` × `height`, not yet shown.
    pub fn new(title: &str, width: u32, height: u32, resizable: bool) -> Result<Window, String> {
        let inner = platform::Window::new(title, width.max(1), height.max(1), resizable)?;
        WINDOWS.with(|w| w.set(w.get() + 1));
        Ok(Window { inner })
    }
    pub fn handle(&self) -> Handle {
        self.inner.handle()
    }
    pub fn show(&self) {
        self.inner.show()
    }
    /// Resizes the client area (a plugin asked for it).
    pub fn set_size(&self, width: u32, height: u32) {
        self.inner.set_size(width.max(1), height.max(1))
    }
    pub fn set_resizable(&self, resizable: bool, width: u32, height: u32) {
        self.inner.set_resizable(resizable, width.max(1), height.max(1))
    }
    pub fn raise(&self) {
        self.inner.raise()
    }
    /// Takes the events gathered by [`pump`].
    pub fn events(&self) -> Events {
        self.inner.events()
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        WINDOWS.with(|w| w.set(w.get().saturating_sub(1)));
    }
}

#[cfg(target_os = "linux")]
mod platform {
    //! X11 through x11rb (pure Rust, its own connection). Under Wayland the
    //! editor windows go through XWayland: plugins only know X11.
    use super::{Events, Handle};
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;
    use x11rb::connection::Connection;
    use x11rb::properties::WmSizeHints;
    use x11rb::protocol::xproto::*;
    use x11rb::protocol::Event;
    use x11rb::rust_connection::RustConnection;
    use x11rb::wrapper::ConnectionExt as _;

    /// A window's pending events and the last size seen.
    type Tracked = (Events, (u32, u32));

    struct Display {
        conn: RustConnection,
        screen: usize,
        wm_protocols: Atom,
        wm_delete: Atom,
        net_wm_name: Atom,
        utf8: Atom,
        windows: RefCell<HashMap<u32, Tracked>>,
    }

    thread_local! {
        static DISPLAY: RefCell<Option<Rc<Display>>> = const { RefCell::new(None) };
    }

    fn display() -> Result<Rc<Display>, String> {
        if let Some(d) = DISPLAY.with(|d| d.borrow().clone()) {
            return Ok(d);
        }
        let (conn, screen) = x11rb::connect(None).map_err(|e| format!("plugin editors need an X11 display (XWayland): {e}"))?;
        let atom = |name: &[u8]| -> Result<Atom, String> { Ok(conn.intern_atom(false, name).map_err(|e| e.to_string())?.reply().map_err(|e| e.to_string())?.atom) };
        // Toolkits such as JUCE take these "only if they exist" and set
        // them on their windows, which fails when no window manager made them.
        for name in [&b"WM_TAKE_FOCUS"[..], b"WM_STATE", b"WM_CHANGE_STATE", b"_NET_WM_PING", b"_NET_WM_STATE", b"_NET_WM_STATE_HIDDEN", b"_NET_WM_STATE_FOCUSED", b"_NET_WM_WINDOW_TYPE", b"_NET_WM_WINDOW_TYPE_NORMAL", b"_NET_WM_WINDOW_TYPE_DIALOG", b"_NET_FRAME_EXTENTS", b"_NET_SUPPORTED", b"_XEMBED", b"_XEMBED_INFO"] {
            atom(name)?;
        }
        let d = Rc::new(Display {
            wm_protocols: atom(b"WM_PROTOCOLS")?,
            wm_delete: atom(b"WM_DELETE_WINDOW")?,
            net_wm_name: atom(b"_NET_WM_NAME")?,
            utf8: atom(b"UTF8_STRING")?,
            conn,
            screen,
            windows: RefCell::new(HashMap::new()),
        });
        DISPLAY.with(|x| *x.borrow_mut() = Some(d.clone()));
        Ok(d)
    }

    pub fn prepare() -> Result<(), String> {
        display().map(|_| ())
    }

    pub fn pump_events() {
        let Some(d) = DISPLAY.with(|d| d.borrow().clone()) else { return };
        while let Ok(Some(e)) = d.conn.poll_for_event() {
            let mut w = d.windows.borrow_mut();
            match e {
                Event::ClientMessage(m) if m.type_ == d.wm_protocols && m.format == 32 && m.data.as_data32()[0] == d.wm_delete => {
                    if let Some((ev, _)) = w.get_mut(&m.window) {
                        ev.close = true;
                    }
                }
                Event::ConfigureNotify(c) => {
                    if let Some((ev, last)) = w.get_mut(&c.window) {
                        let size = (c.width as u32, c.height as u32);
                        if size != *last {
                            *last = size;
                            ev.resized = Some(size);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    pub struct Window {
        d: Rc<Display>,
        id: u32,
    }

    impl Window {
        pub fn new(title: &str, width: u32, height: u32, resizable: bool) -> Result<Window, String> {
            let d = display()?;
            let err = |e: &dyn std::fmt::Display| format!("cannot open a window: {e}");
            let screen = &d.conn.setup().roots[d.screen];
            let id = d.conn.generate_id().map_err(|e| err(&e))?;
            let aux = CreateWindowAux::new().background_pixel(screen.black_pixel).event_mask(EventMask::STRUCTURE_NOTIFY);
            d.conn
                .create_window(x11rb::COPY_DEPTH_FROM_PARENT, id, screen.root, 0, 0, width as u16, height as u16, 0, WindowClass::INPUT_OUTPUT, x11rb::COPY_FROM_PARENT, &aux)
                .map_err(|e| err(&e))?;
            let w = Window { d: d.clone(), id };
            d.conn.change_property32(PropMode::REPLACE, id, d.wm_protocols, AtomEnum::ATOM, &[d.wm_delete]).map_err(|e| err(&e))?;
            d.conn.change_property8(PropMode::REPLACE, id, AtomEnum::WM_NAME, AtomEnum::STRING, title.as_bytes()).map_err(|e| err(&e))?;
            d.conn.change_property8(PropMode::REPLACE, id, d.net_wm_name, d.utf8, title.as_bytes()).map_err(|e| err(&e))?;
            d.conn.change_property8(PropMode::REPLACE, id, AtomEnum::WM_CLASS, AtomEnum::STRING, b"corinzia-plugin\0Corinzia\0").map_err(|e| err(&e))?;
            w.set_resizable(resizable, width, height);
            // The window must exist on the server before a plugin reparents into it.
            d.conn.sync().map_err(|e| err(&e))?;
            d.windows.borrow_mut().insert(id, (Events::default(), (width, height)));
            Ok(w)
        }
        pub fn handle(&self) -> Handle {
            Handle::X11(self.id)
        }
        pub fn show(&self) {
            let _ = self.d.conn.map_window(self.id);
            let _ = self.d.conn.flush();
        }
        pub fn raise(&self) {
            let _ = self.d.conn.configure_window(self.id, &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE));
            let _ = self.d.conn.flush();
        }
        pub fn set_size(&self, width: u32, height: u32) {
            if let Some((_, last)) = self.d.windows.borrow_mut().get_mut(&self.id) {
                *last = (width, height);
            }
            let _ = self.d.conn.configure_window(self.id, &ConfigureWindowAux::new().width(width).height(height));
            let _ = self.d.conn.flush();
        }
        pub fn set_resizable(&self, resizable: bool, width: u32, height: u32) {
            let mut hints = WmSizeHints::new();
            if !resizable {
                hints.min_size = Some((width as i32, height as i32));
                hints.max_size = Some((width as i32, height as i32));
            }
            let _ = hints.set_normal_hints(&self.d.conn, self.id);
            let _ = self.d.conn.flush();
        }
        pub fn events(&self) -> Events {
            self.d.windows.borrow_mut().get_mut(&self.id).map(|(e, _)| std::mem::take(e)).unwrap_or_default()
        }
    }

    impl Drop for Window {
        fn drop(&mut self) {
            self.d.windows.borrow_mut().remove(&self.id);
            let _ = self.d.conn.destroy_window(self.id);
            let _ = self.d.conn.flush();
        }
    }
}

#[cfg(windows)]
mod platform {
    //! Win32: a plain window class; the thread's message loop (winit's)
    //! dispatches its messages, the window procedure records them.
    use super::{Events, Handle};
    use std::cell::Cell;
    use windows_sys::Win32::Foundation::*;
    use windows_sys::Win32::Graphics::Gdi::*;
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::WindowsAndMessaging::*;

    pub fn prepare() -> Result<(), String> {
        Ok(())
    }

    pub fn pump_events() {}

    struct State {
        events: Cell<Events>,
        last: Cell<(u32, u32)>,
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    const CLASS: &str = "CorinziaPluginEditor";

    unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        let state = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const State;
        if let Some(s) = state.as_ref() {
            match msg {
                WM_CLOSE => {
                    let mut e = s.events.get();
                    e.close = true;
                    s.events.set(e);
                    return 0; // the host closes the editor, then the window
                }
                WM_SIZE => {
                    let size = ((lparam & 0xffff) as u32, ((lparam >> 16) & 0xffff) as u32);
                    if size != s.last.get() && size.0 > 0 && size.1 > 0 {
                        s.last.set(size);
                        let mut e = s.events.get();
                        e.resized = Some(size);
                        s.events.set(e);
                    }
                }
                _ => {}
            }
        }
        DefWindowProcW(hwnd, msg, wparam, lparam)
    }

    fn style(resizable: bool) -> u32 {
        if resizable {
            WS_OVERLAPPEDWINDOW
        } else {
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX
        }
    }

    fn outer(width: u32, height: u32, style: u32) -> (i32, i32) {
        let mut r = RECT { left: 0, top: 0, right: width as i32, bottom: height as i32 };
        unsafe { AdjustWindowRectEx(&mut r, style, 0, 0) };
        (r.right - r.left, r.bottom - r.top)
    }

    pub struct Window {
        hwnd: HWND,
        state: Box<State>,
    }

    impl Window {
        pub fn new(title: &str, width: u32, height: u32, resizable: bool) -> Result<Window, String> {
            unsafe {
                let instance = GetModuleHandleW(std::ptr::null());
                let class = wide(CLASS);
                let wc = WNDCLASSW {
                    style: CS_HREDRAW | CS_VREDRAW,
                    lpfnWndProc: Some(wndproc),
                    cbClsExtra: 0,
                    cbWndExtra: 0,
                    hInstance: instance,
                    hIcon: std::ptr::null_mut(),
                    hCursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW),
                    hbrBackground: GetStockObject(BLACK_BRUSH) as HBRUSH,
                    lpszMenuName: std::ptr::null(),
                    lpszClassName: class.as_ptr(),
                };
                RegisterClassW(&wc); // fails harmlessly once registered
                let st = style(resizable);
                let (w, h) = outer(width, height, st);
                let hwnd = CreateWindowExW(0, class.as_ptr(), wide(title).as_ptr(), st, CW_USEDEFAULT, CW_USEDEFAULT, w, h, std::ptr::null_mut(), std::ptr::null_mut(), instance, std::ptr::null());
                if hwnd.is_null() {
                    return Err(format!("cannot open a window (error {})", GetLastError()));
                }
                let state = Box::new(State { events: Cell::new(Events::default()), last: Cell::new((width, height)) });
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, &*state as *const State as isize);
                Ok(Window { hwnd, state })
            }
        }
        pub fn handle(&self) -> Handle {
            Handle::Win32(self.hwnd as *mut std::ffi::c_void)
        }
        pub fn show(&self) {
            unsafe { ShowWindow(self.hwnd, SW_SHOWNORMAL) };
        }
        pub fn raise(&self) {
            unsafe { SetForegroundWindow(self.hwnd) };
        }
        pub fn set_size(&self, width: u32, height: u32) {
            self.state.last.set((width, height));
            unsafe {
                let st = GetWindowLongW(self.hwnd, GWL_STYLE) as u32;
                let (w, h) = outer(width, height, st);
                SetWindowPos(self.hwnd, std::ptr::null_mut(), 0, 0, w, h, SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE);
            }
        }
        pub fn set_resizable(&self, resizable: bool, width: u32, height: u32) {
            unsafe {
                let visible = GetWindowLongW(self.hwnd, GWL_STYLE) as u32 & WS_VISIBLE;
                SetWindowLongW(self.hwnd, GWL_STYLE, (style(resizable) | visible) as i32);
            }
            self.set_size(width, height);
        }
        pub fn events(&self) -> Events {
            self.state.events.take()
        }
    }

    impl Drop for Window {
        fn drop(&mut self) {
            unsafe {
                SetWindowLongPtrW(self.hwnd, GWLP_USERDATA, 0);
                DestroyWindow(self.hwnd);
            }
        }
    }
}

#[cfg(not(any(target_os = "linux", windows)))]
mod platform {
    use super::{Events, Handle};
    pub fn prepare() -> Result<(), String> {
        Err("plugin editors are not supported on this platform yet".into())
    }
    pub fn pump_events() {}
    pub struct Window;
    impl Window {
        pub fn new(_: &str, _: u32, _: u32, _: bool) -> Result<Window, String> {
            Err("plugin editors are not supported on this platform yet".into())
        }
        pub fn handle(&self) -> Handle {
            Handle::X11(0)
        }
        pub fn show(&self) {}
        pub fn raise(&self) {}
        pub fn set_size(&self, _: u32, _: u32) {}
        pub fn set_resizable(&self, _: bool, _: u32, _: u32) {}
        pub fn events(&self) -> Events {
            Events::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timers_fire_and_unregister() {
        let owner = new_owner();
        let hits = Rc::new(Cell::new(0));
        let h = hits.clone();
        let id = register_timer(owner, None, 1, Rc::new(move || h.set(h.get() + 1)));
        std::thread::sleep(Duration::from_millis(3));
        pump();
        assert_eq!(hits.get(), 1);
        assert!(unregister_timer(owner, id));
        std::thread::sleep(Duration::from_millis(3));
        pump();
        assert_eq!(hits.get(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn descriptors_report_readiness() {
        let owner = new_owner();
        let mut fds = [0i32; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        let seen = Rc::new(Cell::new(0u32));
        let s = seen.clone();
        register_fd(owner, fds[0] as u64, fds[0], FD_READ, Rc::new(move |f| s.set(f)));
        pump();
        assert_eq!(seen.get(), 0);
        unsafe { libc::write(fds[1], b"x".as_ptr() as *const _, 1) };
        pump();
        assert_eq!(seen.get() & FD_READ, FD_READ);
        forget(owner);
        assert!(!unregister_fds(owner, fds[0] as u64));
        unsafe {
            libc::close(fds[0]);
            libc::close(fds[1]);
        }
    }
}
