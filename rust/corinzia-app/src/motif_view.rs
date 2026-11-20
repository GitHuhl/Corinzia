//! The motif bank on screen: the Motifs page of the Score tab's side panel
//! and the Explore tab show each motif as a card with its notes drawn
//! (a small piano roll), its name, roles and tags, and what can be done
//! with it.

use crate::keys::Action;
use crate::panels::tip;
use crate::theme;
use corinzia_model::motif::{self, Motif, Texture};
use corinzia_model::Project;
use iced::mouse;
use iced::widget::canvas::{self, Frame, Geometry, Path, Stroke};
use iced::widget::{button, column, container, row, text, Canvas};
use iced::{Element, Length, Point, Rectangle, Renderer, Size, Theme};

/// A key's name: C4 is middle C.
pub fn key_name(k: i32) -> String {
    const N: [&str; 12] = ["C", "C♯", "D", "E♭", "E", "F", "F♯", "G", "A♭", "A", "B♭", "B"];
    format!("{}{}", N[k.rem_euclid(12) as usize], k.div_euclid(12) - 1)
}

/// The notes drawn: time across, pitch up, a bar per note (beats faintly
/// marked).
pub struct Roll<'a> {
    pub motif: &'a Motif,
}

impl<M> canvas::Program<M> for Roll<'_> {
    type State = ();
    fn draw(&self, _: &(), renderer: &Renderer, _: &Theme, bounds: Rectangle, _: mouse::Cursor) -> Vec<Geometry> {
        let p = theme::palette();
        let mut f = Frame::new(renderer, bounds.size());
        f.fill_rectangle(Point::ORIGIN, bounds.size(), p.base);
        let m = self.motif;
        let len = m.length_q().max(m.notes.iter().map(|n| n.end_q()).fold(0.0, f64::max)).max(0.25);
        let (lo, hi) = (m.notes.iter().map(|n| n.key()).min().unwrap_or(60), m.notes.iter().map(|n| n.key()).max().unwrap_or(60));
        // At least an octave tall, centred.
        let span = (hi - lo).max(12) as f32;
        let mid = (hi + lo) as f32 / 2.0;
        let (w, h) = (bounds.width, bounds.height);
        let pad = 3.0;
        let row = (h - 2.0 * pad) / (span + 1.0);
        let x = |q: f64| pad + (q / len) as f32 * (w - 2.0 * pad);
        let y = |k: i32| h / 2.0 - (k as f32 - mid) * row - row / 2.0;
        // Beats.
        let beat = 4.0 / m.meter.1.max(1) as f64;
        let mut b = beat;
        while b < len - 1e-6 {
            let bar = (b / (beat * m.meter.0 as f64) - (b / (beat * m.meter.0 as f64)).round()).abs() < 1e-6;
            let c = if bar { p.line } else { iced::Color { a: 0.35, ..p.line } };
            f.stroke(&Path::line(Point::new(x(b), 0.0), Point::new(x(b), h)), Stroke::default().with_color(c).with_width(1.0));
            b += beat;
        }
        for n in &m.notes {
            let (x0, x1) = (x(n.start_q()), x(n.end_q()));
            let top = y(n.key());
            f.fill_rectangle(Point::new(x0 + 0.5, top), Size::new((x1 - x0 - 1.0).max(1.5), row.clamp(1.5, 6.0)), p.accent);
        }
        vec![f.into_geometry()]
    }
}

/// The facts read off a motif's notes, as label and value lines.
pub fn facts(project: &Project, m: &Motif) -> Vec<(&'static str, String)> {
    let a = motif::analyse(m);
    let mut out = Vec::new();
    if let Some(s) = &m.source {
        let inst = s.instrument.and_then(|i| project.instrument(i)).map_or("Staff".to_string(), |i| i.name.clone());
        let movement = project.movements.iter().find(|x| x.id == s.movement).map(|x| x.title.clone()).unwrap_or_default();
        let bars = if s.bars.0 == s.bars.1 { format!("bar {}", s.bars.0) } else { format!("bars {}–{}", s.bars.0, s.bars.1) };
        out.push(("From", [inst, bars, movement].into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join(" · ")));
    }
    let voice = a.voice_range.map(|v| format!(" · {}", v.name())).unwrap_or_default();
    out.push(("Range", format!("{}–{} ({} semitones){voice} · {} register", key_name(a.lowest), key_name(a.highest), a.ambitus, a.register)));
    let texture = match a.texture {
        Texture::Line => "a single line",
        Texture::Chordal => "chords",
        Texture::Mixed => "lines and chords",
    };
    out.push(("Shape", format!("{}, {} · {:.0}% steps, {:.0}% leaps (largest {}), {:.0}% repeats", texture, a.shape.name(), a.steps * 100.0, a.leaps * 100.0, a.largest_leap, a.repeats * 100.0)));
    let mut rhythm = format!("{} notes in {} beats · {:.1} per quarter · {:.0}% sounding", a.notes, fmt_beats(a.length_q * m.meter.1 as f64 / 4.0), a.density, a.sounding * 100.0);
    if a.syncopation > 0.0 {
        rhythm += &format!(" · {:.0}% syncopated", a.syncopation * 100.0);
    }
    if a.dotted {
        rhythm += " · dotted";
    }
    if a.repeats_itself {
        rhythm += " · repeats itself";
    }
    out.push(("Rhythm", rhythm));
    let key = format!("{} {}", key_label(m.fifths, m.minor), if m.minor { "minor" } else { "major" });
    let mut degrees = format!("in {key} · from {} to {} ({})", a.first_degree, a.last_degree, if a.closed { "closed" } else { "open" });
    if a.chromatic > 0 {
        degrees += &format!(" · {} outside the key", a.chromatic);
    }
    out.push(("Degrees", degrees));
    let mut touch = Vec::new();
    if a.staccato > 0.0 {
        touch.push(format!("{:.0}% staccato", a.staccato * 100.0));
    }
    if a.accents > 0.0 {
        touch.push(format!("{:.0}% accented", a.accents * 100.0));
    }
    if let Some((lo, hi)) = a.dynamics {
        let d = |i: u8| corinzia_model::DYNAMICS.get(i as usize).copied().unwrap_or("");
        touch.push(if lo == hi { d(lo).to_string() } else { format!("{}–{}", d(lo), d(hi)) });
    }
    if !touch.is_empty() {
        out.push(("Touch", touch.join(" · ")));
    }
    out.push(("Context", format!("{}/{} · ♩ = {:.0}", m.meter.0, m.meter.1, m.bpm)));
    out
}

fn fmt_beats(b: f64) -> String {
    if (b - b.round()).abs() < 1e-6 { format!("{}", b.round()) } else { format!("{b:.1}") }
}

fn key_label(fifths: i32, minor: bool) -> String {
    let t = motif::tonic(fifths, minor);
    // Sharps keys spell sharps, flat keys flats.
    const SHARP: [&str; 12] = ["C", "C♯", "D", "D♯", "E", "F", "F♯", "G", "G♯", "A", "A♯", "B"];
    const FLAT: [&str; 12] = ["C", "D♭", "D", "E♭", "E", "F", "G♭", "G", "A♭", "A", "B♭", "B"];
    (if fifths >= 0 { SHARP } else { FLAT })[t as usize].to_string()
}

/// A motif's card: its name, roles, notes drawn, tags, and its actions.
pub fn card<'a>(m: &'a Motif, wide: bool) -> Element<'a, Action> {
    let bold = iced::Font { weight: iced::font::Weight::Bold, ..iced::Font::DEFAULT };
    let roles = m.roles.iter().map(|r| r.name()).collect::<Vec<_>>().join(" · ");
    let small = |label: &'a str, hint: &'a str, a: Action| tip(button(text(label).size(12)).padding([2, 7]).style(theme::flat(false)).on_press(a), hint);
    let mut c = column![
        row![
            text(m.name.as_str()).size(13).font(bold).width(Length::Fill),
            small("▶", "Listen (on its instrument)", Action::AuditionMotif(m.id)),
            small("Show", "Select it in the score", Action::ShowMotif(m.id)),
            small("Edit", "Name it, say what it is", Action::EditMotif(m.id)),
        ]
        .spacing(2)
        .align_y(iced::Alignment::Center),
        Canvas::new(Roll { motif: m }).width(Length::Fill).height(if wide { 70 } else { 40 }),
    ]
    .spacing(4);
    if !roles.is_empty() {
        c = c.push(text(roles).size(11).style(theme::accent));
    }
    if !m.tags.is_empty() {
        c = c.push(text(m.tags.join(", ")).size(11).style(theme::dim));
    }
    if wide && !m.comment.is_empty() {
        c = c.push(text(m.comment.as_str()).size(11).style(theme::dim));
    }
    container(c).padding(8).width(Length::Fill).style(theme::card).into()
}

/// The Motifs page of the side panel: add the selection, then the bank.
pub fn page(ed: &crate::editor::Editor) -> Element<'_, Action> {
    let add = tip(
        button(text("＋  Add the selection as a motif").size(13)).padding([6, 10]).width(Length::Fill).style(theme::tool(false)).on_press(Action::AddMotif),
        "Select notes on one staff, then name the motif and say what it is (Ctrl+M)",
    );
    let mut c = column![add].spacing(8).padding(iced::Padding { top: 4.0, ..iced::Padding::ZERO });
    if ed.project.motifs.is_empty() {
        c = c.push(text("The motif bank is empty. Select a passage on one staff (a melody, a bass line, a call and its response…) and add it; the Explore tab builds on what is here.").size(12).style(theme::dim));
    }
    for m in &ed.project.motifs {
        c = c.push(card(m, false));
    }
    c.into()
}

/// The Explore tab: the bank as the material accompaniment and composition
/// will draw on (played from a live performer's MIDI input), grouped by
/// what each motif is.
pub fn explore(ed: &crate::editor::Editor) -> Element<'_, Action> {
    use corinzia_model::motif::Role;
    let bold = iced::Font { weight: iced::font::Weight::Bold, ..iced::Font::DEFAULT };
    let bank = &ed.project.motifs;
    let mut c = column![
        text("Explore").size(20).font(bold),
        text("Accompaniment and composition built on this piece: a performer plays (a MIDI keyboard), and Corinzia answers from the motif bank, tailored to the score. The bank is what it draws on: add to it from the Score tab's Motifs page (select notes on one staff, Ctrl+M).").size(13).style(theme::dim),
    ]
    .spacing(10)
    .padding(20)
    .max_width(1100);
    if bank.is_empty() {
        c = c.push(text("The bank is empty.").size(13));
        return iced::widget::scrollable(c).height(Length::Fill).width(Length::Fill).into();
    }
    // What the bank holds, by role.
    let counts: Vec<String> = Role::ALL
        .iter()
        .filter_map(|r| {
            let n = bank.iter().filter(|m| m.roles.contains(r)).count();
            (n > 0).then(|| format!("{} {}", n, r.name().to_lowercase()))
        })
        .collect();
    let untagged = bank.iter().filter(|m| m.roles.is_empty()).count();
    let mut summary = format!("{} motif{}", bank.len(), if bank.len() == 1 { "" } else { "s" });
    if !counts.is_empty() {
        summary += &format!(": {}", counts.join(" · "));
    }
    if untagged > 0 {
        summary += &format!(" · {untagged} without a role yet");
    }
    c = c.push(text(summary).size(13));
    let mut grid = column![].spacing(10);
    for pair in bank.chunks(2) {
        let mut r = row![].spacing(10);
        for m in pair {
            r = r.push(container(card(m, true)).width(Length::FillPortion(1)));
        }
        if pair.len() == 1 {
            r = r.push(container(text("")).width(Length::FillPortion(1)));
        }
        grid = grid.push(r);
    }
    c = c.push(grid);
    iced::widget::scrollable(container(c).width(Length::Fill).align_x(iced::Alignment::Center)).height(Length::Fill).width(Length::Fill).into()
}
