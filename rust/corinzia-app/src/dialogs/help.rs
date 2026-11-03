//! The keyboard shortcut overview (F1).

use crate::*;
use iced::widget::column;

/// The keyboard shortcut overview (F1).
pub(crate) fn help_card() -> Element<'static, Message> {
    const SECTIONS: [(&str, &[(&str, &str)]); 4] = [
        (
            "Note input",
            &[
                ("N", "Note input on / off"),
                ("A – G", "Enter a note"),
                ("Shift+A – G", "Add to the chord"),
                ("1 – 7", "64th … whole note"),
                (".", "Dot, double dot"),
                (";", "Tuplet: 3, 5, 5:4…   : ends it"),
                ("/  ?", "Grace notes (slashed, plain)"),
                ("R  ,", "Rest"),
                ("−  =  0", "Flat, sharp, natural"),
                ("T", "Tie"),
                ("V", "Next voice"),
                ("Q", "Chord mode"),
                ("Backspace", "Delete the last note"),
            ],
        ),
        (
            "Moving around",
            &[
                ("← →", "Previous / next note"),
                ("Ctrl+← →", "Previous / next bar"),
                ("Shift+← →", "Extend the selection"),
                ("Home  End", "First / last note"),
                ("Alt+↑ ↓", "Staff above / below"),
                ("↑ ↓", "Transpose a step"),
                ("Ctrl+↑ ↓", "Transpose an octave"),
                ("Space", "Play / stop"),
                ("Ctrl+= −", "Zoom in / out"),
                ("Ctrl+0", "Fit the page width"),
                ("Ctrl+T", "Transposed / concert pitch"),
                ("Ctrl+PgUp PgDn", "Previous / next movement"),
            ],
        ),
        (
            "Marks and text",
            &[
                ("[  '  ]  \\", "Staccato, accent, tenuto, marcato"),
                ("S", "Slur"),
                ("Click its curve", "Select a slur; drag an end to a note"),
                ("Shift+← →  Alt+← →", "Slur's end, start by a note"),
                ("X", "Flip a slur"),
                ("Double-click", "Retype a title, tempo, text, dynamic…"),
                ("H  Shift+H", "Crescendo, diminuendo"),
                ("Shift+K  M  T", "Key, time, tempo"),
                ("Shift+C", "Clef; 8va, 8vb, loco"),
                ("Shift+I", "Instrument change (a doubling)"),
                ("Ctrl+Shift+T", "Transpose: up M2, to Eb…"),
                ("Ctrl+F", "Select only: top, voice 2, above C5, F#…"),
                ("Enter  Ctrl+Enter", "System break, page break before the bar"),
                ("Ctrl+Shift+A", "Select similar"),
                ("Shift+B  R", "Barline; ending, segno, D.C.…"),
                ("Shift+X  P  O", "Expression, technique (ped), ornament"),
                ("Shift+D", "Dynamic: pp, mf, sfz, fp…"),
                ("Shift+F", "Fingering"),
                ("Shift+A", "Rehearsal mark"),
                ("Ctrl+L", "Lyrics"),
                ("Ctrl+K", "Chord symbols"),
            ],
        ),
        (
            "Editing and files",
            &[
                ("Ctrl+Z  Ctrl+Y", "Undo, redo"),
                ("Ctrl+C  X  V", "Copy, cut, paste"),
                ("Ctrl+A", "Select all"),
                ("Delete", "Delete the selection"),
                ("R", "Repeat the selection"),
                ("1 – 7  .", "Selection's duration, dots"),
                ("A – G", "Selection's pitch"),
                ("Alt+← →", "Move by the rhythmic grid"),
                ("Ctrl+B", "Insert a bar"),
                ("Ctrl+Shift+⌫", "Delete bars"),
                ("Ctrl+N  O  S", "New, open, save"),
                ("Ctrl+1  2", "Score, Mix"),
                ("F1", "This overview"),
            ],
        ),
    ];
    let bold = iced::Font { weight: iced::font::Weight::Bold, ..iced::Font::DEFAULT };
    let mut cols = row![].spacing(20);
    for (title, keys) in SECTIONS {
        let mut c = column![text(title.to_uppercase()).size(10).style(theme::faint)].spacing(5).width(200);
        for (k, what) in keys {
            c = c.push(row![text(*k).size(12).font(bold).style(theme::accent).width(92), text(*what).size(12)].spacing(8));
        }
        cols = cols.push(c);
    }
    let close = button(text("✕").size(12)).padding([1, 6]).style(theme::flat(false)).on_press(Message::Act(Action::Help));
    container(column![row![text("Keyboard shortcuts").size(16).font(bold), panels::spacer(), close], cols].spacing(14)).padding(20).style(theme::card).into()
}
