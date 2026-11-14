//! The MIDI keyboard: opening a port, reading it, sounding keys.

use super::*;

/// Ticks to read the MIDI keyboard (every 8 ms) while one is open.
pub(crate) fn midi_clock() -> impl iced::futures::Stream<Item = Message> {
    let (mut tx, rx) = iced::futures::channel::mpsc::channel(1);
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_millis(8));
        if let Err(e) = tx.try_send(Message::MidiPoll) {
            if e.is_disconnected() {
                break;
            }
        }
    });
    rx
}

impl App {
    /// Opens a MIDI input port (none for an empty id).
    pub(crate) fn open_midi(&mut self, id: &str) {
        self.midi = None;
        self.midi_held.clear();
        if id.is_empty() {
            return;
        }
        match midi_in::Input::open(id) {
            Ok(i) => self.midi = Some(i),
            Err(e) => self.editor.status = format!("MIDI input: {e}"),
        }
    }

    /// Keys from the MIDI keyboard: in note input they are entered (keys
    /// held together make a chord); they always sound on the staff's track.
    pub(crate) fn midi_poll(&mut self) {
        let Some(input) = &self.midi else { return };
        let msgs: Vec<midi_in::MidiMsg> = input.rx.try_iter().collect();
        for m in msgs {
            match m {
                midi_in::MidiMsg::On { key, .. } => {
                    if self.editor.input_mode && self.tab == Tab::Score {
                        let chord = !self.midi_held.is_empty();
                        self.editor.enter_midi_key(key, chord);
                    }
                    self.midi_held.push(key);
                    self.sound_key(key);
                }
                midi_in::MidiMsg::Off { key } => self.midi_held.retain(|k| *k != key),
            }
        }
    }

    /// Plays a key on the instrument of the caret's staff.
    pub(crate) fn sound_key(&mut self, key: u8) {
        let ed = &self.editor;
        let Some(m) = ed.movement() else { return };
        let Some(inst) = m.lanes.get(ed.caret_lane).and_then(|l| l.instrument_at(ed.caret)) else { return };
        let Some(track) = ed.project.instrument(inst).and_then(|i| i.track) else { return };
        let channel = ed.project.track(track).map(|t| t.midi_channel as i16).unwrap_or(0);
        self.sync();
        self.playback.preview(track, channel, &[key as i32]);
    }
}
