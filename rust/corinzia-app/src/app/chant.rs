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
//!   Delete; Ctrl+L syllables, Shift+M the mode.
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
