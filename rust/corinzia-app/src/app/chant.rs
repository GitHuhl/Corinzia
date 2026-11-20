//! A Byzantine chant in the Score tab: its keys and clicks.
//!
//! Keys (the logical key, without Shift applied):
//! * 0 ison, 1 oligon, 2 kentima, 3 oligon with kentima above, 4 oligon
//!   with ypsili right, 5 oligon with ypsili left;
//! * a apostrofos, e elafron (Shift+E with apostrofos), c chamili,
//!   y yporroi, p petasti, k kentimata (Shift+K under an oligon),
//!   Shift+I oligon with ison and kentimata;
//! * `.` klasma, g gorgon → digorgon → trigorgon, d dipli → tripli,
//!   f fthora (diatonic → hard chromatic → soft chromatic), on the neume
//!   before the caret;
//! * m martyria, Enter line break, ←/→ Home/End the caret, Backspace and
//!   Delete; Ctrl+L syllables, Shift+M the mode, Shift+D the ison (drone).
//!
//! Anything else (Ctrl+Z, Space, F1…) goes on to the usual key map.

use super::*;
use corinzia_byz::{Quantity, TimeSign};

impl App {
    /// A key with a chant shown: `None` when the usual key map should have it.
    pub(crate) fn chant_key(&mut self, key: &iced::keyboard::Key, mods: iced::keyboard::Modifiers) -> Option<Task<Message>> {
        use iced::keyboard::{key::Named, Key};
        let ed = &mut self.editor;
        let (shift, ctrl) = (mods.shift(), mods.control() || mods.logo());
        match key.as_ref() {
            Key::Named(Named::ArrowLeft) if !ctrl => ed.chant_move(-1),
            Key::Named(Named::ArrowRight) if !ctrl => ed.chant_move(1),
            Key::Named(Named::Home) => ed.chant_move(i64::MIN / 2),
            Key::Named(Named::End) => ed.chant_move(i64::MAX / 2),
            Key::Named(Named::Backspace) => ed.chant_delete(false),
            Key::Named(Named::Delete) => ed.chant_delete(true),
            Key::Named(Named::Enter) if !ctrl => ed.chant_break(),
            Key::Character(c) if ctrl => {
                return match c.to_ascii_lowercase().as_str() {
                    "l" => Some(self.open_popover(PopoverKind::ChantLyric)),
                    _ => None,
                };
            }
            Key::Character(c) => {
                let q = |q: Quantity| Some(q);
                let neume = match (c.to_ascii_lowercase().as_str(), shift) {
                    ("0", false) => q(Quantity::Ison),
                    ("1", false) => q(Quantity::Oligon),
                    ("2", false) => q(Quantity::Kentima),
                    ("3", false) => q(Quantity::OligonKentimaAbove),
                    ("4", false) => q(Quantity::OligonYpsiliRight),
                    ("5", false) => q(Quantity::OligonYpsiliLeft),
                    ("a", false) => q(Quantity::Apostrofos),
                    ("e", false) => q(Quantity::Elafron),
                    ("e", true) => q(Quantity::ElafronApostrofos),
                    ("c", false) => q(Quantity::Chamili),
                    ("y", false) => q(Quantity::Yporroi),
                    ("p", false) => q(Quantity::Petasti),
                    ("k", false) => q(Quantity::Kentimata),
                    ("k", true) => q(Quantity::OligonKentimataBelow),
                    ("i", true) => q(Quantity::OligonIsonKentimata),
                    _ => None,
                };
                if let Some(n) = neume {
                    ed.chant_neume(n);
                    self.audition_chant();
                } else {
                    match (c.to_ascii_lowercase().as_str(), shift) {
                        (".", false) => ed.chant_time(&[TimeSign::Klasma]),
                        ("g", false) => ed.chant_time(&[TimeSign::Gorgon, TimeSign::Digorgon, TimeSign::Trigorgon]),
                        ("d", false) => ed.chant_time(&[TimeSign::Dipli, TimeSign::Tripli]),
                        ("d", true) => ed.chant_toggle_ison(),
                        ("f", false) => ed.chant_fthora(),
                        ("m", false) => ed.chant_martyria(),
                        ("m", true) => return Some(self.open_popover(PopoverKind::ChantMode)),
                        // Other letters are not Western edits here.
                        _ => {}
                    }
                }
            }
            _ => return None,
        }
        Some(Task::none())
    }

    /// A click on the chant: the caret after the element clicked.
    pub(crate) fn chant_click(&mut self, hit: Option<u64>) {
        if let Some(id) = hit.filter(|id| *id > 0) {
            self.editor.chant_caret_after(id as usize - 1);
        }
    }

    /// Sounds the note just entered (nearest key; the intervals of the
    /// genus come with chant playback).
    fn audition_chant(&mut self) {
        let Some(c) = self.editor.shown_chant() else { return };
        let at = self.editor.chant_caret;
        let (sung, _) = c.sing();
        let keys: Vec<i32> = sung.iter().filter(|s| s.element + 1 == at).map(|s| corinzia_byz::Reference::default().key_and_cents(s.moria).0).collect();
        if let (Some(k), Some(track)) = (keys.last(), self.editor.project.tracks.iter().find(|t| !t.audio).map(|t| t.id)) {
            self.sync();
            self.playback.preview(track, 0, &[*k]);
        }
    }
}

/// The chant's place on the playback timeline.
pub(crate) const CHANT_SECTION: corinzia_model::Id = u64::MAX - 1;
/// The timeline section a motif of the bank plays in (see `audition_motif`).
pub(crate) const MOTIF_SECTION: corinzia_model::Id = u64::MAX - 2;

/// A chant as tones: the melody on `channel`, each note the nearest key
/// bent to its pitch in moria, and the ison (if sung) held on the next
/// channel. The length is in beats.
pub(crate) fn chant_tones(chant: &corinzia_byz::Chant, reference: corinzia_byz::Reference, channel: i16) -> (Vec<corinzia_audio::snapshot::Tone>, f64) {
    use corinzia_audio::snapshot::Tone;
    let (sung, _) = chant.sing();
    let length = sung.last().map_or(0.0, |s| quarters(s.start + s.length) / 4.0);
    // Bend within ±2 semitones: a cent is 1/200 of the range.
    let tone = |moria: i32, on: f64, off: f64, channel: i16, velocity: f32, element: u64| {
        let (key, cents) = reference.key_and_cents(moria);
        Tone { on, off, key: key.clamp(0, 127) as i16, bend: (cents / 200.0) as f32, channel, velocity, element }
    };
    let mut tones: Vec<Tone> = sung
        .iter()
        .map(|s| {
            let on = quarters(s.start) / 4.0;
            tone(s.moria, on, on + quarters(s.length) / 4.0, channel, 0.75, corinzia_layout::byz::element_id(s.element))
        })
        .collect();
    if chant.ison && length > 0.0 {
        tones.push(tone(chant.ison_note().1, 0.0, length, (channel + 1) & 15, 0.45, 0));
    }
    (tones, length)
}

impl App {
    /// Plays the chant shown from the caret (from the start at its end).
    pub(crate) fn play_chant(&mut self) {
        let Some(c) = self.editor.shown_chant() else { return };
        let Some(track) = self.editor.project.tracks.iter().find(|t| !t.audio) else {
            self.editor.status = "Add a player to sing the chant".into();
            return;
        };
        let (id, channel, name) = (track.id, track.midi_channel as i16 & 15, track.name.clone());
        let (tones, length) = chant_tones(c, corinzia_byz::Reference::default(), channel);
        if tones.is_empty() {
            return;
        }
        let (sung, _) = c.sing();
        let caret = self.editor.chant_caret;
        let from = sung.iter().find(|s| s.element >= caret).map_or(0.0, |s| quarters(s.start) / 4.0);
        let bpm = c.tempo as f64;
        self.playback.play_tones(&self.editor.project, self.editor.music_revision, CHANT_SECTION, bpm, length, id, &tones, from);
        self.editor.status = format!("Singing on {name}{}", if c.ison { ", with the ison" } else { "" });
    }

    /// While a chant plays: the neume being sung is highlighted.
    pub(crate) fn follow_chant(&mut self) {
        let Some((CHANT_SECTION, beat)) = self.playback.playhead() else { return };
        let Some(c) = self.editor.shown_chant() else { return };
        let (sung, _) = c.sing();
        let Some(s) = sung.iter().rev().find(|s| quarters(s.start) / 4.0 <= beat + 1e-9) else { return };
        let id = corinzia_layout::byz::element_id(s.element);
        if !(self.selected.len() == 1 && self.selected.contains(&id)) {
            self.selected = std::iter::once(id).collect();
            self.cache.clear();
        }
    }
}
