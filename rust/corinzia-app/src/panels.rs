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
    tooltip(content, container(text(label).size(12)).padding([4, 8]).style(crate::theme::tooltip), tooltip::Position::Bottom).into()
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
        .style(crate::theme::tool(active))
        .on_press(action);
    tip(b, hint)
}

fn wide<'a>(label: &'a str, active: bool, hint: &'a str, action: Action) -> Element<'a, Action> {
    let b = button(text(label).size(13)).padding([6, 10]).width(Length::Fill).style(crate::theme::tool(active)).on_press(action);
    tip(b, hint)
}

/// A section heading: small, spaced capitals in the dim colour.
fn heading(t: &str) -> Element<'_, Action> {
    container(text(t.to_uppercase()).size(10).style(crate::theme::faint)).padding(iced::Padding { top: 8.0, right: 0.0, bottom: 1.0, left: 0.0 }).into()
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
        tool("3", false, ed.tuplet_input.is_some(), "Triplets; ; for any tuplet, : to end", Action::Triplet),
        tool("\u{E4E5}", true, false, "Rest (R or , in input mode)", Action::Rest),
        tool("\u{E560}", true, ed.grace_input.is_some(), "Grace notes (/ slashed, ? plain)", Action::GraceInput(true)),
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
        tool("◠", false, false, "Slur (S)", Action::Slur),
        tool("\u{E53E}", true, false, "Crescendo (H)", Action::Hairpin { crescendo: true }),
        tool("\u{E53F}", true, false, "Diminuendo (Shift+H)", Action::Hairpin { crescendo: false }),
        tool("‿", false, false, "Tie (T)", Action::Tie),
    ];
    let editing = vec![
        tool("▲", false, false, "Transpose up (↑, Ctrl+↑ octave)", Action::Transpose(1)),
        tool("▼", false, false, "Transpose down (↓, Ctrl+↓ octave)", Action::Transpose(-1)),
        tool("❐", false, false, "Copy (Ctrl+C)", Action::Copy),
        tool("✂", false, false, "Cut (Ctrl+X)", Action::Cut),
        tool("❏", false, ed.has_clip(), "Paste (Ctrl+V)", Action::Paste),
        tool("↻", false, false, "Repeat selection (R)", Action::Repeat),
        tool("⌫", false, false, "Delete (Del)", Action::Delete),
        tool("▣", false, false, "Select all (Ctrl+A)", Action::SelectAll),
        tool("≈", false, false, "Select similar (Ctrl+Shift+A)", Action::SelectSimilar),
        tool("⧩", false, false, "Select only… top, voice 2, above C5, F#, staccato, dynamics (Ctrl+F)", Action::Popover(PopoverKind::Filter)),
    ];
    column![
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

pub fn structure(ed: &Editor) -> Element<'_, Action> {
    let pop = |label, kind: PopoverKind, key| wide(label, false, key, Action::Popover(kind));
    let movements: Vec<Element<'_, Action>> = ed
        .project
        .movements
        .iter()
        .enumerate()
        .map(|(i, m)| wide(if m.title.is_empty() { "Untitled" } else { m.title.as_str() }, i == ed.movement, "Show this movement", Action::Movement(i)))
        .collect();
    column![
        heading("Movements"),
        column(movements).spacing(6),
        wide("New movement", false, "After this one, with the same instruments", Action::AddMovement),
        pop("Rename movement…", PopoverKind::MovementTitle, "The movement's title"),
        wide("Delete movement", false, "This movement (undo brings it back)", Action::DeleteMovement),
        heading("At the selection or caret"),
        pop("Key signature…", PopoverKind::Key, "Shift+K"),
        pop("Time signature…", PopoverKind::Time, "Shift+M"),
        pop("Clef or octave line…", PopoverKind::Clef, "Shift+C: treble, bass… 8va, 8vb, loco"),
        pop("Instrument change…", PopoverKind::Instrument, "Shift+I: a doubling (Piccolo…), from here on"),
        pop("Transpose…", PopoverKind::Transpose, "Ctrl+Shift+T: the passage, or the whole score"),
        pop("Tempo…", PopoverKind::Tempo, "Shift+T"),
        pop("Barline…", PopoverKind::Barline, "Shift+B"),
        pop("Expression text…", PopoverKind::Expression, "Shift+X"),
        pop("Technique text or pedal…", PopoverKind::Technique, "Shift+P: pizz., arco… ped, *"),
        pop("Ornament…", PopoverKind::Ornament, "Shift+O: tr, mordent… up bow, down bow"),
        pop("Fingering…", PopoverKind::Fingering, "Shift+F: 1, or 1 3 5 for a chord"),
        wide("Rehearsal mark", false, "Shift+A (toggles)", Action::Rehearsal),
        heading("Lyrics and chords"),
        wide("Lyrics, verse 1…", false, "Ctrl+L: space moves on, - hyphen, _ extender", Action::Lyrics(1)),
        wide("Lyrics, verse 2…", false, "Ctrl+Shift+L", Action::Lyrics(2)),
        wide("Lyrics, verse 3…", false, "Verse 3", Action::Lyrics(3)),
        pop("Chord symbol…", PopoverKind::Chord, "Ctrl+K: space moves to the next beat"),
        heading("Repeats"),
        pop("Ending or jump…", PopoverKind::Ending, "Shift+R: 1, 2… segno, coda, D.C. al Fine"),
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
    container(scrollable(container(body).padding(iced::Padding { top: 0.0, right: 12.0, bottom: 16.0, left: 12.0 })).height(Length::Fill)).width(WIDTH).height(Length::Fill).into()
}

pub fn spacer() -> Space {
    Space::new().width(Length::Fill)
}
