//! The side panels: Notes (input state, marks, dynamics, lines, editing)
//! and Structure (popovers, bars). Buttons show SMuFL glyphs from Bravura;
//! the active state is highlighted; tooltips name the keyboard shortcut.

use crate::editor::{Editor, PopoverKind};
use crate::keys::Action;
use corinzia_score::score::art;
use iced::widget::{button, column, container, row, scrollable, text, tooltip, Space};
use iced::{Element, Font, Length};

pub const WIDTH: f32 = 272.0;
pub const MUSIC: Font = Font::with_name("Bravura");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Panel {
    Notes,
    Structure,
}

pub fn tip<'a, M: 'a>(content: impl Into<Element<'a, M>>, label: &'a str) -> Element<'a, M> {
    tooltip(content, container(text(label).size(12)).padding([3, 6]).style(container::rounded_box), tooltip::Position::Bottom).into()
}

/// A square tool button with a music glyph (or text) and a tooltip.
fn tool<'a>(label: &'a str, music: bool, active: bool, hint: &'a str, action: Action) -> Element<'a, Action> {
    let t = if music {
        text(label).font(MUSIC).size(22).line_height(1.0)
    } else {
        text(label).size(14)
    };
    let b = button(container(t).center_x(Length::Fill).center_y(Length::Fill))
        .width(30)
        .height(30)
        .padding(0)
        .style(if active { button::primary } else { button::secondary })
        .on_press(action);
    tip(b, hint)
}

fn wide<'a>(label: &'a str, active: bool, hint: &'a str, action: Action) -> Element<'a, Action> {
    let b = button(text(label).size(13)).padding([5, 8]).width(Length::Fill).style(if active { button::primary } else { button::secondary }).on_press(action);
    tip(b, hint)
}

fn heading(t: &str) -> Element<'_, Action> {
    text(t).size(12).style(text::secondary).into()
}

fn grid<'a>(items: Vec<Element<'a, Action>>, per_row: usize) -> Element<'a, Action> {
    let mut rows = column![].spacing(4);
    let mut it = items.into_iter().peekable();
    while it.peek().is_some() {
        let mut r = row![].spacing(4);
        for _ in 0..per_row {
            if let Some(e) = it.next() {
                r = r.push(e);
            }
        }
        rows = rows.push(r);
    }
    rows.into()
}

pub fn notes(ed: &Editor) -> Element<'_, Action> {
    // SMuFL: metNote* (durations), accidentals, articulations, dynamics.
    const DUR: [(&str, &str); 7] = [
        ("\u{ECA2}", "Whole note (7)"),
        ("\u{ECA3}", "Half note (6)"),
        ("\u{ECA5}", "Quarter note (5)"),
        ("\u{ECA7}", "Eighth note (4)"),
        ("\u{ECA9}", "16th note (3)"),
        ("\u{ECAB}", "32nd note (2)"),
        ("\u{ECAD}", "64th note (1)"),
    ];
    let durations: Vec<_> = DUR.iter().enumerate().map(|(i, (g, hint))| tool(g, true, ed.spec.base as usize == i, hint, Action::Duration(i as u8))).collect();
    let rhythm = vec![
        tool("·", false, ed.spec.dots == 1, "Dotted (.)", Action::Dots),
        tool("··", false, ed.spec.dots == 2, "Double dotted (. twice)", Action::Dots),
        tool("3", false, ed.spec.triplet, "Triplet (;)", Action::Triplet),
        tool("\u{E4E5}", true, false, "Rest (R or , in input mode)", Action::Rest),
        tool("♬", false, ed.chord_mode, "Chord mode (Q)", Action::ChordMode),
    ];
    let accidentals = vec![
        tool("\u{E260}", true, ed.accidental == Some(-1), "Flat (-)", Action::Accidental(-1)),
        tool("\u{E261}", true, ed.accidental == Some(0), "Natural (0)", Action::Accidental(0)),
        tool("\u{E262}", true, ed.accidental == Some(1), "Sharp (=)", Action::Accidental(1)),
    ];
    let voices: Vec<_> = (1..=4u8)
        .map(|v| {
            const LABELS: [&str; 4] = ["1", "2", "3", "4"];
            tool(LABELS[v as usize - 1], false, ed.voice == v, "Voice (V cycles)", Action::SetVoice(v))
        })
        .collect();
    let arts = vec![
        tool("\u{E4A2}", true, false, "Staccato ([)", Action::Articulation(art::STACCATO)),
        tool("\u{E4A0}", true, false, "Accent (')", Action::Articulation(art::ACCENT)),
        tool("\u{E4A4}", true, false, "Tenuto (])", Action::Articulation(art::TENUTO)),
        tool("\u{E4AC}", true, false, "Marcato (\\)", Action::Articulation(art::MARCATO)),
        tool("\u{E4A8}", true, false, "Staccatissimo", Action::Articulation(art::STACCATISSIMO)),
        tool("\u{E4C0}", true, false, "Fermata", Action::Articulation(art::FERMATA)),
    ];
    let ornaments = vec![
        tool("\u{E566}", true, false, "Trill", Action::Articulation(art::TRILL)),
        tool("\u{E56C}", true, false, "Mordent", Action::Articulation(art::MORDENT)),
        tool("\u{E567}", true, false, "Turn", Action::Articulation(art::TURN)),
        tool("\u{E222}", true, false, "Tremolo", Action::Articulation(art::TREMOLO)),
        tool("\u{EAA9}", true, false, "Arpeggio", Action::Articulation(art::ARPEGGIO)),
        tool("\u{E4CE}", true, false, "Breath mark", Action::Articulation(art::BREATH)),
        tool("\u{E4D1}", true, false, "Caesura", Action::Articulation(art::CAESURA)),
    ];
    const DYN: [(&str, &str); 8] = [
        ("\u{E52A}", "ppp"),
        ("\u{E52B}", "pp"),
        ("\u{E520}", "p"),
        ("\u{E52C}", "mp"),
        ("\u{E52D}", "mf"),
        ("\u{E522}", "f"),
        ("\u{E52F}", "ff"),
        ("\u{E530}", "fff"),
    ];
    let mut dynamics: Vec<_> = DYN.iter().enumerate().map(|(i, (g, hint))| tool(g, true, false, hint, Action::Dynamic(Some(i as u8)))).collect();
    dynamics.push(tool("✕", false, false, "Remove dynamic", Action::Dynamic(None)));
    let lines = vec![
        tool("⌒", false, false, "Slur (S)", Action::Slur),
        tool("\u{E53E}", true, false, "Crescendo (H)", Action::Hairpin { crescendo: true }),
        tool("\u{E53F}", true, false, "Diminuendo (Shift+H)", Action::Hairpin { crescendo: false }),
        tool("‿", false, false, "Tie (T)", Action::Tie),
    ];
    let editing = vec![
        tool("▲", false, false, "Transpose up (↑, Ctrl+↑ octave)", Action::Transpose(1)),
        tool("▼", false, false, "Transpose down (↓, Ctrl+↓ octave)", Action::Transpose(-1)),
        tool("⧉", false, false, "Copy (Ctrl+C)", Action::Copy),
        tool("✂", false, false, "Cut (Ctrl+X)", Action::Cut),
        tool("⎘", false, ed.has_clip(), "Paste (Ctrl+V)", Action::Paste),
        tool("↻", false, false, "Repeat selection (R)", Action::Repeat),
        tool("⌫", false, false, "Delete (Del)", Action::Delete),
        tool("▣", false, false, "Select all (Ctrl+A)", Action::SelectAll),
    ];
    column![
        wide(if ed.input_mode { "✎ Note input: on" } else { "✎ Note input" }, ed.input_mode, "N toggles; letters A–G enter notes", Action::SetInput(!ed.input_mode)),
        heading("Duration"),
        grid(durations, 7),
        grid(rhythm, 7),
        heading("Accidental"),
        grid(accidentals, 7),
        heading("Voice"),
        grid(voices, 7),
        heading("Articulations"),
        grid(arts, 7),
        heading("Ornaments"),
        grid(ornaments, 7),
        heading("Dynamics"),
        grid(dynamics, 7),
        heading("Lines"),
        grid(lines, 7),
        heading("Edit"),
        grid(editing, 7),
    ]
    .spacing(6)
    .into()
}

pub fn structure(_ed: &Editor) -> Element<'_, Action> {
    let pop = |label, kind: PopoverKind, key| wide(label, false, key, Action::Popover(kind));
    column![
        heading("At the selection or caret"),
        pop("Key signature…", PopoverKind::Key, "Shift+K"),
        pop("Time signature…", PopoverKind::Time, "Shift+M"),
        pop("Clef…", PopoverKind::Clef, "Shift+C"),
        pop("Tempo…", PopoverKind::Tempo, "Shift+T"),
        pop("Barline…", PopoverKind::Barline, "Shift+B"),
        pop("Expression text…", PopoverKind::Expression, "Shift+X"),
        pop("Technique text…", PopoverKind::Technique, "Shift+P"),
        pop("Ornament…", PopoverKind::Ornament, "Shift+O"),
        wide("Rehearsal mark", false, "Shift+A (toggles)", Action::Rehearsal),
        heading("Repeats"),
        pop("Ending…", PopoverKind::Ending, "Shift+R"),
        wide("1st and 2nd endings", false, "The selected bars and as many after", Action::FirstSecondEndings),
        heading("Bars"),
        wide("Insert bar", false, "Ctrl+B: before the selected bar", Action::InsertBar),
        wide("Delete bars", false, "Ctrl+Shift+Backspace", Action::DeleteBars),
    ]
    .spacing(6)
    .into()
}

pub fn panel(ed: &Editor, which: Panel) -> Element<'_, Action> {
    let body = match which {
        Panel::Notes => notes(ed),
        Panel::Structure => structure(ed),
    };
    container(scrollable(container(body).padding([8, 12])).height(Length::Fill)).width(WIDTH).height(Length::Fill).style(container::bordered_box).into()
}

pub fn spacer() -> Space {
    Space::new().width(Length::Fill)
}
