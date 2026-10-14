//! The score keyboard, as in the Qt app: note names enter notes, digits pick
//! durations, punctuation adds marks, Shift+letter opens a popover.

use crate::editor::PopoverKind;
use corinzia_score::score::art;
use iced::keyboard::key::Named;
use iced::keyboard::{Key, Modifiers};

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    ToggleInput,
    Escape,
    Note { step: i32, chord: bool },
    Rest,
    Duration(u8), // 0 = whole .. 6 = 64th
    Dots,
    Triplet,
    Accidental(i8),
    ChordMode,
    Tie,
    Voice,
    Articulation(u16),
    Slur,
    Hairpin { crescendo: bool },
    Repeat,
    CaretBy(i64),
    CaretStaff(i64),
    Transpose(i32),
    Backspace,
    Delete,
    Copy,
    Cut,
    Paste,
    SelectAll,
    InsertBar,
    DeleteBars,
    Undo,
    Redo,
    Save,
    SaveAs,
    Open,
    New,
    Popover(PopoverKind),
    Rehearsal,
    ZoomIn,
    ZoomOut,
    PlayStop,
    ShowScore,
    ShowMix,
    // Panel-only actions.
    SetInput(bool),
    SetVoice(u8),
    Dynamic(Option<u8>),
    FirstSecondEndings,
}

/// `key` is the logical key without Shift applied (iced's `key`).
pub fn map(key: &Key, mods: Modifiers, input_mode: bool) -> Option<Action> {
    use Action::*;
    let (shift, ctrl, alt) = (mods.shift(), mods.control() || mods.logo(), mods.alt());
    let ch: Option<char> = match key.as_ref() {
        Key::Character(c) => c.chars().next().map(|c| c.to_ascii_lowercase()),
        _ => None,
    };
    if let Key::Named(n) = key {
        return Some(match (n, ctrl, alt) {
            (Named::Escape, ..) => Escape,
            (Named::Space, false, false) => PlayStop,
            (Named::ArrowLeft, false, false) => CaretBy(-1),
            (Named::ArrowRight, false, false) => CaretBy(1),
            (Named::ArrowUp, _, true) => CaretStaff(-1),
            (Named::ArrowDown, _, true) => CaretStaff(1),
            (Named::ArrowUp, true, _) => Transpose(7),
            (Named::ArrowDown, true, _) => Transpose(-7),
            (Named::ArrowUp, ..) => Transpose(1),
            (Named::ArrowDown, ..) => Transpose(-1),
            (Named::Backspace, true, _) if shift => DeleteBars,
            (Named::Backspace, ..) => {
                if input_mode {
                    Backspace
                } else {
                    Delete
                }
            }
            (Named::Delete, ..) => Delete,
            _ => return None,
        });
    }
    let c = ch?;
    if ctrl {
        return Some(match (c, shift) {
            ('z', false) => Undo,
            ('z', true) | ('y', _) => Redo,
            ('c', _) => Copy,
            ('x', _) => Cut,
            ('v', _) => Paste,
            ('a', _) => SelectAll,
            ('b', _) => InsertBar,
            ('s', false) => Save,
            ('s', true) => SaveAs,
            ('o', _) => Open,
            ('n', _) => New,
            ('=' | '+', _) => ZoomIn,
            ('-', _) => ZoomOut,
            ('1', _) => ShowScore,
            ('2', _) => ShowMix,
            _ => return None,
        });
    }
    // Popovers (Shift+letter). Shift+A..G are chord notes in input mode.
    if shift && !alt {
        let pop = match c {
            'k' => Some(PopoverKind::Key),
            'm' => Some(PopoverKind::Time),
            't' => Some(PopoverKind::Tempo),
            'x' => Some(PopoverKind::Expression),
            'p' => Some(PopoverKind::Technique),
            'o' => Some(PopoverKind::Ornament),
            'c' if !input_mode => Some(PopoverKind::Clef),
            'b' if !input_mode => Some(PopoverKind::Barline),
            'r' if !input_mode => Some(PopoverKind::Ending),
            _ => None,
        };
        if let Some(p) = pop {
            return Some(Popover(p));
        }
        if c == 'a' && !input_mode {
            return Some(Rehearsal);
        }
    }
    let note = "cdefgab".find(c);
    Some(match c {
        'n' => ToggleInput,
        _ if input_mode && note.is_some() => Note { step: note.unwrap() as i32, chord: shift },
        '1'..='7' => Duration(7 - (c as u8 - b'0')),
        '.' => Dots,
        ';' => Triplet,
        'r' | ',' if input_mode => Rest,
        'r' => Repeat,
        's' => Slur,
        'h' => Hairpin { crescendo: !shift },
        'q' => ChordMode,
        't' => Tie,
        'v' => Voice,
        '-' => Accidental(-1),
        '=' | '+' => Accidental(1),
        '0' => Accidental(0),
        '[' => Articulation(art::STACCATO),
        '\'' => Articulation(art::ACCENT),
        ']' => Articulation(art::TENUTO),
        '\\' => Articulation(art::MARCATO),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn k(c: &str) -> Key {
        Key::Character(c.into())
    }
    #[test]
    fn keyboard_map() {
        let none = Modifiers::empty();
        assert_eq!(map(&k("c"), none, true), Some(Action::Note { step: 0, chord: false }));
        assert_eq!(map(&k("c"), Modifiers::SHIFT, true), Some(Action::Note { step: 0, chord: true }));
        assert_eq!(map(&k("c"), Modifiers::SHIFT, false), Some(Action::Popover(PopoverKind::Clef)));
        assert_eq!(map(&k("c"), none, false), None);
        assert_eq!(map(&k("5"), none, false), Some(Action::Duration(2))); // quarter
        assert_eq!(map(&k("r"), none, true), Some(Action::Rest));
        assert_eq!(map(&k("r"), none, false), Some(Action::Repeat));
        assert_eq!(map(&k("z"), Modifiers::CTRL, false), Some(Action::Undo));
        assert_eq!(map(&k("z"), Modifiers::CTRL | Modifiers::SHIFT, false), Some(Action::Redo));
        assert_eq!(map(&Key::Named(Named::ArrowUp), Modifiers::ALT, false), Some(Action::CaretStaff(-1)));
        assert_eq!(map(&Key::Named(Named::ArrowUp), Modifiers::CTRL, false), Some(Action::Transpose(7)));
        assert_eq!(map(&Key::Named(Named::Backspace), Modifiers::CTRL | Modifiers::SHIFT, false), Some(Action::DeleteBars));
        assert_eq!(map(&k("h"), Modifiers::SHIFT, false), Some(Action::Hairpin { crescendo: false }));
        assert_eq!(map(&Key::Named(Named::Space), none, true), Some(Action::PlayStop));
        assert_eq!(map(&k("2"), Modifiers::CTRL, false), Some(Action::ShowMix));
    }
}
