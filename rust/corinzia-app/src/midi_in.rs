//! MIDI keyboard input. On Linux the raw MIDI devices (`/dev/snd/midiC*D*`,
//! every USB or built-in MIDI port) are read directly, with no library to
//! install; on Windows ports come through WinMM (midir). A thread reads the
//! bytes and hands note-ons and note-offs to the app through a channel.

use std::sync::mpsc::{channel, Receiver};
#[cfg(not(windows))]
use std::sync::mpsc::Sender;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MidiMsg {
    On { key: u8, velocity: u8 },
    Off { key: u8 },
}

/// A MIDI byte stream to messages: running status, note-on at velocity 0
/// as note-off, system real-time bytes anywhere, system exclusive skipped.
#[derive(Default)]
pub struct Parser {
    status: u8,
    data: Vec<u8>,
    sysex: bool,
}

impl Parser {
    pub fn feed(&mut self, byte: u8, out: &mut impl FnMut(MidiMsg)) {
        if byte >= 0xf8 {
            return; // clock, active sensing…
        }
        if byte == 0xf0 {
            self.sysex = true;
            return;
        }
        if byte == 0xf7 {
            self.sysex = false;
            return;
        }
        if self.sysex {
            return;
        }
        if byte & 0x80 != 0 {
            self.status = if byte < 0xf0 { byte } else { 0 };
            self.data.clear();
            return;
        }
        if self.status == 0 {
            return;
        }
        self.data.push(byte);
        let needed = match self.status & 0xf0 {
            0xc0 | 0xd0 => 1,
            _ => 2,
        };
        if self.data.len() < needed {
            return;
        }
        let (a, b) = (self.data[0], *self.data.get(1).unwrap_or(&0));
        self.data.clear();
        match self.status & 0xf0 {
            0x90 if b > 0 => out(MidiMsg::On { key: a, velocity: b }),
            0x90 | 0x80 => out(MidiMsg::Off { key: a }),
            _ => {}
        }
    }
}

/// The ports to choose from: (id, label).
pub fn ports() -> Vec<(String, String)> {
    #[cfg(target_os = "linux")]
    {
        let mut v: Vec<(String, String)> = std::fs::read_dir("/dev/snd")
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                let rest = name.strip_prefix("midiC")?;
                let (card, dev) = rest.split_once('D')?;
                let card_name = std::fs::read_to_string(format!("/proc/asound/card{card}/id")).map(|s| s.trim().to_string()).unwrap_or_else(|_| format!("Card {card}"));
                Some((format!("/dev/snd/{name}"), format!("{card_name} (port {dev})")))
            })
            .collect();
        v.sort();
        v
    }
    #[cfg(windows)]
    {
        let Ok(input) = midir::MidiInput::new("Corinzia") else { return vec![] };
        input.ports().iter().filter_map(|p| input.port_name(p).ok()).map(|n| (n.clone(), n)).collect()
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    Vec::new()
}

/// An open input: messages arrive on `rx`; dropping it closes the port.
pub struct Input {
    pub rx: Receiver<MidiMsg>,
    /// Cleared when the input is dropped: the reader thread stops.
    #[cfg(not(windows))]
    alive: std::sync::Arc<std::sync::atomic::AtomicBool>,
    #[cfg(windows)]
    _conn: midir::MidiInputConnection<()>,
}

impl Input {
    /// Opens a port by its id (on Linux any file of MIDI bytes will do).
    pub fn open(id: &str) -> Result<Input, String> {
        let (tx, rx) = channel();
        #[cfg(not(windows))]
        {
            // Non-blocking: the reader looks in every few milliseconds, so it
            // can let the (exclusive) port go as soon as it is closed.
            let mut opts = std::fs::OpenOptions::new();
            opts.read(true);
            #[cfg(target_os = "linux")]
            {
                use std::os::unix::fs::OpenOptionsExt;
                opts.custom_flags(0o4000); // O_NONBLOCK
            }
            let file = opts.open(id).map_err(|e| format!("{id}: {e}"))?;
            let alive = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
            spawn_reader(file, tx, alive.clone());
            Ok(Input { rx, alive })
        }
        #[cfg(windows)]
        {
            let input = midir::MidiInput::new("Corinzia").map_err(|e| e.to_string())?;
            let port = input.ports().into_iter().find(|p| input.port_name(p).is_ok_and(|n| n == id)).ok_or(format!("No MIDI input {id}"))?;
            let mut parser = Parser::default();
            let conn = input
                .connect(
                    &port,
                    "corinzia-in",
                    move |_, bytes, _| {
                        for &b in bytes {
                            parser.feed(b, &mut |m| {
                                let _ = tx.send(m);
                            });
                        }
                    },
                    (),
                )
                .map_err(|e| e.to_string())?;
            Ok(Input { rx, _conn: conn })
        }
    }
}

#[cfg(not(windows))]
impl Drop for Input {
    fn drop(&mut self) {
        self.alive.store(false, std::sync::atomic::Ordering::Relaxed);
    }
}

#[cfg(not(windows))]
fn spawn_reader(mut file: std::fs::File, tx: Sender<MidiMsg>, alive: std::sync::Arc<std::sync::atomic::AtomicBool>) {
    use std::io::Read;
    let _ = std::thread::Builder::new().name("corinzia-midi-in".into()).spawn(move || {
        let mut parser = Parser::default();
        let mut buf = [0u8; 256];
        loop {
            let n = match file.read(&mut buf) {
                Ok(0) => {
                    // A file at its end (tests): wait for more until closed.
                    if !alive.load(std::sync::atomic::Ordering::Relaxed) {
                        return;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(5));
                    continue;
                }
                Ok(n) => n,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    if !alive.load(std::sync::atomic::Ordering::Relaxed) {
                        return;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(2));
                    continue;
                }
                Err(_) => return,
            };
            let mut gone = false;
            for &b in &buf[..n] {
                parser.feed(b, &mut |m| gone |= tx.send(m).is_err());
            }
            if gone || !alive.load(std::sync::atomic::Ordering::Relaxed) {
                return;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn running_status_velocity_zero_and_realtime_bytes() {
        let mut p = Parser::default();
        let mut got = Vec::new();
        for b in [0x90, 60, 100, 64, 90, 0xf8, 60, 0, 0x80, 64, 10, 0xf0, 1, 2, 0xf7, 0xc0, 5, 0x91, 67, 1] {
            p.feed(b, &mut |m| got.push(m));
        }
        use MidiMsg::*;
        assert_eq!(got, [On { key: 60, velocity: 100 }, On { key: 64, velocity: 90 }, Off { key: 60 }, Off { key: 64 }, On { key: 67, velocity: 1 }]);
    }
}
