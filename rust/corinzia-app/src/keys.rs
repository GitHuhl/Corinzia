//! The score keyboard: note names enter notes, digits pick
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
    Pitch(i32),   // a letter outside note input: the selection's pitch
    TupletOff,    // ends tuplet input
    Movement(usize), // show movement i
    MovementStep(i64), // the previous (-1) or next (+1) movement
    AddMovement,
    DeleteMovement,
    GraceInput(bool), // grace note input on/off (slashed?)
    Nudge(i64),   // move the selection by the rhythmic grid
    Grid(usize),  // choose the rhythmic grid (editor::GRIDS)
    GridStep(i64), // a coarser (-1) or finer (+1) grid
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
    /// Select the previous/next note or rest (`bar`: the next bar's first).
    Step { dir: i64, bar: bool },
    /// Grow or shrink the passage by an event.
    Extend(i64),
    /// Select the first/last event of the voice.
    Edge { last: bool },
    /// Select on the staff above/below.
    Staff(i64),
    Transpose(i32),
    Backspace,
    Delete,
    Copy,
    Cut,
    Paste,
    /// The clipboard as a cue (small, silent notes in voice 2).
    PasteCue,
    /// Tablature on or off for the staff's instrument.
    Tab,
    /// Selected notes onto the staff above (-1) or below (1).
    CrossStaff(i8),
    SelectAll,
    /// Pick what is like the selected element.
    SelectSimilar,
    /// Beams by hand: a `span::BEAMS` bit, or 0 for automatic.
    Beam(u8),
    /// A system (or page) break before the selected bar, or none.
    Break { page: bool },
    /// The Layout card.
    Layout,
    /// The Preferences card.
    Preferences,
    /// Show every movement on the pages, one after another, or one.
    AllMovements,
    /// Chords of one staff spread over the selected staves.
    Explode,
    /// The selected staves combined into chords on one.
    Reduce,
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
    /// Lyrics popover for a verse.
    Lyrics(u8),
    ShowScore,
    ShowMix,
    /// Zoom so the page fills the width.
    FitWidth,
    /// Concert pitch / transposed score.
    Transposed,
    /// Show or hide the keyboard shortcuts.
    Help,
    /// Flip the selected slur to the other side.
    Flip,
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
            (Named::Enter, false, false) if !input_mode => Break { page: false },
            (Named::Enter, true, false) => Break { page: true },
            (Named::F1, ..) => Help,
            (Named::Space, false, false) => PlayStop,
            // Input mode moves the caret; otherwise the arrows walk notes.
            (Named::ArrowLeft | Named::ArrowRight, _, false) => {
                let dir = if *n == Named::ArrowLeft { -1 } else { 1 };
                if input_mode {
                    CaretBy(dir)
                } else if shift {
                    Extend(dir)
                } else {
                    Step { dir, bar: ctrl }
                }
            }
            // Alt+arrows move the selection by the rhythmic grid.
            (Named::ArrowLeft, false, true) => Nudge(-1),
            (Named::ArrowRight, false, true) => Nudge(1),
            (Named::PageUp, true, _) => MovementStep(-1),
            (Named::PageDown, true, _) => MovementStep(1),
            (Named::Home, ..) if !input_mode => Edge { last: false },
            (Named::End, ..) if !input_mode => Edge { last: true },
            (Named::ArrowUp, _, true) => if input_mode { CaretStaff(-1) } else { Staff(-1) },
            (Named::ArrowDown, _, true) => if input_mode { CaretStaff(1) } else { Staff(1) },
            (Named::ArrowUp, true, _) if shift => CrossStaff(-1),
            (Named::ArrowDown, true, _) if shift => CrossStaff(1),
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
    if alt && !ctrl && (c == '[' || c == ']') {
        return Some(GridStep(if c == '[' { -1 } else { 1 }));
    }
    if ctrl {
        return Some(match (c, shift) {
            ('z', false) => Undo,
            ('z', true) | ('y', _) => Redo,
            ('c', _) => Copy,
            ('x', _) => Cut,
            ('v', false) => Paste,
            ('v', true) => PasteCue,
            ('a', false) => SelectAll,
            ('a', true) => SelectSimilar,
            ('f', _) => Popover(PopoverKind::Filter),
            (',', _) => Preferences,
            ('b', _) => InsertBar,
            ('s', false) => Save,
            ('s', true) => SaveAs,
            ('o', _) => Open,
            ('n', _) => New,
            ('=' | '+', _) => ZoomIn,
            ('0', _) => FitWidth,
            ('t', false) => Transposed,
            ('t', true) => Popover(PopoverKind::Transpose),
            ('-', _) => ZoomOut,
            ('l', false) => Lyrics(1),
            ('l', true) => Lyrics(2),
            ('k', _) => Popover(PopoverKind::Chord),
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
            'f' if !input_mode => Some(PopoverKind::Fingering),
            'd' if !input_mode => Some(PopoverKind::Dynamic),
            'c' if !input_mode => Some(PopoverKind::Clef),
            'b' if !input_mode => Some(PopoverKind::Barline),
            'r' if !input_mode => Some(PopoverKind::Ending),
            'i' => Some(PopoverKind::Instrument),
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
        _ if note.is_some() && !shift => Pitch(note.unwrap() as i32),
        '1'..='7' => Duration(7 - (c as u8 - b'0')),
        '.' => Dots,
        ';' => Popover(PopoverKind::Tuplet),
        ':' if input_mode => TupletOff,
        '/' if input_mode => GraceInput(true),
        '?' if input_mode => GraceInput(false),
        'r' | ',' if input_mode => Rest,
        'r' => Repeat,
        's' => Slur,
        'h' => Hairpin { crescendo: !shift },
        'q' => ChordMode,
        'x' => Flip,
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
        assert_eq!(map(&k("c"), none, false), Some(Action::Pitch(0)));
        assert_eq!(map(&Key::Named(Named::ArrowRight), Modifiers::ALT, false), Some(Action::Nudge(1)));
        assert_eq!(map(&Key::Named(Named::ArrowLeft), Modifiers::ALT, true), Some(Action::Nudge(-1)));
        assert_eq!(map(&k("]"), Modifiers::ALT, false), Some(Action::GridStep(1)));
        assert_eq!(map(&k("5"), none, false), Some(Action::Duration(2))); // quarter
        assert_eq!(map(&k("r"), none, true), Some(Action::Rest));
        assert_eq!(map(&k("r"), none, false), Some(Action::Repeat));
        assert_eq!(map(&k("z"), Modifiers::CTRL, false), Some(Action::Undo));
        assert_eq!(map(&k("z"), Modifiers::CTRL | Modifiers::SHIFT, false), Some(Action::Redo));
        assert_eq!(map(&Key::Named(Named::ArrowUp), Modifiers::ALT, false), Some(Action::Staff(-1)));
        assert_eq!(map(&Key::Named(Named::ArrowUp), Modifiers::ALT, true), Some(Action::CaretStaff(-1)));
        assert_eq!(map(&Key::Named(Named::ArrowRight), none, false), Some(Action::Step { dir: 1, bar: false }));
        assert_eq!(map(&Key::Named(Named::ArrowLeft), Modifiers::CTRL, false), Some(Action::Step { dir: -1, bar: true }));
        assert_eq!(map(&Key::Named(Named::ArrowRight), Modifiers::SHIFT, false), Some(Action::Extend(1)));
        assert_eq!(map(&Key::Named(Named::ArrowRight), none, true), Some(Action::CaretBy(1)));
        assert_eq!(map(&Key::Named(Named::End), none, false), Some(Action::Edge { last: true }));
        assert_eq!(map(&Key::Named(Named::ArrowUp), Modifiers::CTRL, false), Some(Action::Transpose(7)));
        assert_eq!(map(&Key::Named(Named::Backspace), Modifiers::CTRL | Modifiers::SHIFT, false), Some(Action::DeleteBars));
        assert_eq!(map(&k("h"), Modifiers::SHIFT, false), Some(Action::Hairpin { crescendo: false }));
        assert_eq!(map(&Key::Named(Named::Space), none, true), Some(Action::PlayStop));
        assert_eq!(map(&k("2"), Modifiers::CTRL, false), Some(Action::ShowMix));
        assert_eq!(map(&k("l"), Modifiers::CTRL, false), Some(Action::Lyrics(1)));
        assert_eq!(map(&k("k"), Modifiers::CTRL, false), Some(Action::Popover(PopoverKind::Chord)));
        assert_eq!(map(&k("x"), none, false), Some(Action::Flip));
    }
}
