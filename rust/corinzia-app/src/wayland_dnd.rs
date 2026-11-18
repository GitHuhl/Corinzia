//! Files dropped on the window from a file manager, under Wayland.
//!
//! The window system Corinzia runs on (winit, under iced) takes drops on
//! X11 and Windows, not on Wayland. This listens for them itself: on the
//! window's own Wayland connection, with an event queue of its own on a
//! thread of its own, a data device for each seat; a drag of files
//! (`text/uri-list`) over the window is accepted as a copy, and when it is
//! dropped the list is read through a pipe and handed over with where it
//! landed (the window's logical coordinates).

use std::collections::HashMap;
use std::io::Read;
use std::os::fd::{AsFd, FromRawFd, OwnedFd};
use std::path::PathBuf;
use wayland_client::backend::{Backend, ObjectId};
use wayland_client::protocol::{wl_data_device, wl_data_device_manager, wl_data_offer, wl_registry, wl_seat, wl_surface};
use wayland_client::{event_created_child, Connection, Dispatch, Proxy, QueueHandle};

const URI_LIST: &str = "text/uri-list";

/// What happens to a drag over the window.
#[derive(Debug, Clone)]
pub enum DndEvent {
    /// Files dragged over the window, here (or gone: `None`).
    Hover(Option<(f32, f32)>),
    /// Files dropped here.
    Drop(Vec<PathBuf>, (f32, f32)),
}

struct State {
    surface: ObjectId,
    manager: Option<wl_data_device_manager::WlDataDeviceManager>,
    seats: Vec<wl_seat::WlSeat>,
    devices: Vec<wl_data_device::WlDataDevice>,
    /// Each offer's types, as announced.
    types: HashMap<ObjectId, Vec<String>>,
    /// The drag over the window: its offer and where it is.
    current: Option<(wl_data_offer::WlDataOffer, f32, f32)>,
    send: Box<dyn Fn(DndEvent) + Send>,
}

/// Starts listening on a Wayland window: `display` is its `wl_display`,
/// `surface` its `wl_surface` (raw pointers, as the window hands them out).
/// Events go to `send` from the listening thread.
///
/// # Safety
/// Both pointers must stay valid for as long as the program runs (the
/// window lives as long as the app).
pub unsafe fn start(display: usize, surface: usize, send: impl Fn(DndEvent) + Send + 'static) -> Result<(), String> {
    let backend = unsafe { Backend::from_foreign_display(display as *mut _) };
    let conn = Connection::from_backend(backend);
    let surface = unsafe { ObjectId::from_ptr(wl_surface::WlSurface::interface(), surface as *mut _) }.map_err(|e| format!("the window's surface: {e}"))?;
    let mut queue = conn.new_event_queue::<State>();
    let qh = queue.handle();
    conn.display().get_registry(&qh, ());
    let mut state = State { surface, manager: None, seats: vec![], devices: vec![], types: HashMap::new(), current: None, send: Box::new(send) };
    // The globals, then a data device for each seat.
    queue.roundtrip(&mut state).map_err(|e| e.to_string())?;
    let manager = state.manager.clone().ok_or("no wl_data_device_manager")?;
    for seat in state.seats.clone() {
        state.devices.push(manager.get_data_device(&seat, &qh, ()));
    }
    queue.flush().map_err(|e| e.to_string())?;
    std::thread::Builder::new()
        .name("corinzia-dnd".into())
        .spawn(move || {
            while queue.blocking_dispatch(&mut state).is_ok() {}
        })
        .map_err(|e| e.to_string())?;
    Ok(())
}

impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(state: &mut Self, registry: &wl_registry::WlRegistry, event: wl_registry::Event, _: &(), _: &Connection, qh: &QueueHandle<Self>) {
        if let wl_registry::Event::Global { name, interface, version } = event {
            match interface.as_str() {
                "wl_data_device_manager" => state.manager = Some(registry.bind(name, version.min(3), qh, ())),
                "wl_seat" => state.seats.push(registry.bind(name, version.min(5), qh, ())),
                _ => {}
            }
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for State {
    fn event(_: &mut Self, _: &wl_seat::WlSeat, _: wl_seat::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<wl_data_device_manager::WlDataDeviceManager, ()> for State {
    fn event(_: &mut Self, _: &wl_data_device_manager::WlDataDeviceManager, _: wl_data_device_manager::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<wl_data_offer::WlDataOffer, ()> for State {
    fn event(state: &mut Self, offer: &wl_data_offer::WlDataOffer, event: wl_data_offer::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let wl_data_offer::Event::Offer { mime_type } = event {
            state.types.entry(offer.id()).or_default().push(mime_type);
        }
    }
}

impl Dispatch<wl_data_device::WlDataDevice, ()> for State {
    fn event(state: &mut Self, _: &wl_data_device::WlDataDevice, event: wl_data_device::Event, _: &(), conn: &Connection, _: &QueueHandle<Self>) {
        use wl_data_device::Event;
        match event {
            Event::Enter { serial, surface, x, y, id } => {
                let Some(offer) = id else { return };
                let files = surface.id() == state.surface && state.types.get(&offer.id()).is_some_and(|t| t.iter().any(|m| m == URI_LIST));
                if !files {
                    offer.accept(serial, None);
                    return;
                }
                offer.accept(serial, Some(URI_LIST.into()));
                if offer.version() >= 3 {
                    offer.set_actions(wl_data_device_manager::DndAction::Copy, wl_data_device_manager::DndAction::Copy);
                }
                state.current = Some((offer, x as f32, y as f32));
                (state.send)(DndEvent::Hover(Some((x as f32, y as f32))));
            }
            Event::Motion { x, y, .. } => {
                if let Some(c) = state.current.as_mut() {
                    (c.1, c.2) = (x as f32, y as f32);
                    (state.send)(DndEvent::Hover(Some((x as f32, y as f32))));
                }
            }
            Event::Leave => {
                if let Some((offer, ..)) = state.current.take() {
                    state.types.remove(&offer.id());
                    offer.destroy();
                    (state.send)(DndEvent::Hover(None));
                }
            }
            Event::Drop => {
                let Some((offer, x, y)) = state.current.take() else { return };
                let paths = receive(conn, &offer).map(|t| parse_uri_list(&t)).unwrap_or_default();
                if offer.version() >= 3 {
                    offer.finish();
                }
                state.types.remove(&offer.id());
                offer.destroy();
                let _ = conn.flush();
                (state.send)(DndEvent::Hover(None));
                if !paths.is_empty() {
                    (state.send)(DndEvent::Drop(paths, (x, y)));
                }
            }
            _ => {}
        }
    }

    event_created_child!(State, wl_data_device::WlDataDevice, [
        wl_data_device::EVT_DATA_OFFER_OPCODE => (wl_data_offer::WlDataOffer, ()),
    ]);
}

/// The offer's uri list, read through a pipe.
fn receive(conn: &Connection, offer: &wl_data_offer::WlDataOffer) -> Option<String> {
    let mut fds = [0; 2];
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        return None;
    }
    let (read, write) = unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
    offer.receive(URI_LIST.into(), write.as_fd());
    conn.flush().ok()?;
    // Ours closed: the reading ends when the source has written all.
    drop(write);
    let mut text = String::new();
    std::fs::File::from(read).read_to_string(&mut text).ok()?;
    Some(text)
}

/// `file://` URIs, one a line ("#" lines are comments), as paths.
pub fn parse_uri_list(text: &str) -> Vec<PathBuf> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| l.strip_prefix("file://"))
        // file://host/path: the path from its first '/'.
        .filter_map(|l| l.find('/').map(|i| &l[i..]))
        .map(|p| PathBuf::from(percent_decode(p)))
        .collect()
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn uri_lists_become_paths() {
        let list = "# a comment\r\nfile:///home/me/Music/Thunder%20Storm.wav\r\nfile://host/tmp/a%2Bb.flac\r\nhttps://example.com/x.wav\r\n";
        assert_eq!(parse_uri_list(list), [PathBuf::from("/home/me/Music/Thunder Storm.wav"), PathBuf::from("/tmp/a+b.flac")]);
        assert_eq!(percent_decode("caf%C3%A9%2"), "café%2");
    }
}
