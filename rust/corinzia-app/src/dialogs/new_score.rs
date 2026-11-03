//! The New score card: its fields and creating the score.

use crate::*;
use iced::widget::column;

/// The "New score" card.
#[derive(Default)]
pub(crate) struct NewScore {
    pub(crate) title: String,
    pub(crate) composer: String,
    pub(crate) instruments: String,
    pub(crate) key: String,
    pub(crate) time: String,
    pub(crate) tempo: String,
    pub(crate) bars: String,
    pub(crate) error: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum NewField {
    Title,
    Composer,
    Instruments,
    Key,
    Time,
    Tempo,
    Bars,
}

impl App {
    pub(crate) fn create_new(&mut self) {
        let Some(n) = self.new_score.as_mut() else { return };
        let bars = n.bars.trim().parse::<i64>().unwrap_or(32).clamp(1, 2000);
        let title = if n.title.trim().is_empty() { "Untitled" } else { n.title.trim() };
        // Key, time (a pickup too: "3/4 pickup 1/4") and tempo, checked first.
        let key = if n.key.trim().is_empty() { None } else {
            match corinzia_edit::parse_key(&n.key) {
                Some(k) => Some(k),
                None => return n.error = Some(format!("Key not understood: {} (try C, Eb, f# minor)", n.key.trim())),
            }
        };
        let (time_text, pickup_text) = match n.time.to_lowercase().split_once("pickup") {
            Some((t, p)) => (t.trim().to_string(), Some(format!("pickup {}", p.trim()))),
            None => (n.time.trim().to_string(), None),
        };
        let time = if time_text.is_empty() { None } else {
            match corinzia_edit::parse_time(&time_text) {
                Some(t) => Some(t),
                None => return n.error = Some(format!("Time not understood: {time_text} (try 3/4, 6/8, C)")),
            }
        };
        let tempo = if n.tempo.trim().is_empty() { None } else {
            match corinzia_edit::parse_tempo(&n.tempo) {
                Some(t) => Some(t),
                None => return n.error = Some(format!("Tempo not understood: {} (try Allegro, Andante 84, q=96)", n.tempo.trim())),
            }
        };
        match files::new_project(title, &n.instruments, bars).map(|mut p| {
            p.composer = n.composer.trim().to_string();
            if let Some(m) = p.movements.first_mut() {
                if let Some(k) = key {
                    corinzia_edit::set_key(m, Frac::ZERO, k);
                }
                if let Some((num, den)) = time {
                    corinzia_edit::set_meter(m, Frac::ZERO, num, den);
                    m.length = Frac::new(num, den) * Frac::int(bars);
                }
                if let Some(t) = &tempo {
                    corinzia_edit::set_tempo(m, Frac::ZERO, t);
                }
                if let Some(len) = pickup_text.as_deref().and_then(|t| corinzia_edit::parse_pickup(t, Frac::new(1, time.map(|t| t.1).unwrap_or(4)))) {
                    corinzia_edit::set_pickup(m, len);
                }
                // Voice 1 is rests to the end in the new meter.
                for l in m.lanes.iter_mut() {
                    for v in l.voices.iter_mut() {
                        v.events = std::sync::Arc::new(Vec::new());
                    }
                }
            }
            p
        }) {
            Ok(p) => {
                let (view, pb) = (self.view, std::mem::replace(&mut self.playback, playback::Playback::new(false, &prefs::Prefs::default())));
                *self = App::with_playback(p, None, view, "New score · N starts note input".into(), pb);
                self.editor.set_input_mode(true);
            }
            Err(e) => n.error = Some(e),
        }
    }
}

/// The New score card.
pub(crate) fn new_score_card(n: &NewScore) -> Element<'_, Message> {
    let bold = iced::Font { weight: iced::font::Weight::Bold, ..iced::Font::DEFAULT };
    let field = |label: &'static str, hint: &'static str, value: &str, f: NewField| {
        column![text(label).size(12).style(theme::dim), text_input(hint, value).on_input(move |v| Message::NewInput(f, v)).on_submit(Message::NewCreate).padding(6).size(14)].spacing(3)
    };
    let title = column![
        text("Title").size(12).style(theme::dim),
        text_input("Untitled", &n.title).id(NEW_TITLE).on_input(|v| Message::NewInput(NewField::Title, v)).on_submit(Message::NewCreate).padding(6).size(14)
    ]
    .spacing(3);
    // Ready-made ensembles fill in the instruments.
    let templates = iced::widget::Row::with_children(files::TEMPLATES.iter().map(|(label, list)| {
        let on = n.instruments == *list;
        button(text(*label).size(12)).padding([3, 8]).style(theme::segment(on)).on_press(Message::NewInput(NewField::Instruments, list.to_string())).into()
    }))
    .spacing(4)
    .wrap();
    let mut card = column![
        text("New score").size(18).font(bold),
        row![title.width(Length::FillPortion(3)), field("Composer", "", &n.composer, NewField::Composer).width(Length::FillPortion(2))].spacing(10),
        text("Ensemble").size(12).style(theme::dim),
        templates,
        field("Instruments (comma separated; \"2 Violin\" for two; any name)", "Flute, Clarinet, Violin, Cello", &n.instruments, NewField::Instruments),
        row![
            field("Key", "C", &n.key, NewField::Key).width(Length::Fill),
            field("Time", "4/4 (3/4 pickup 1/4)", &n.time, NewField::Time).width(Length::Fill),
            field("Tempo", "Allegro 120", &n.tempo, NewField::Tempo).width(Length::Fill),
            field("Bars", "32", &n.bars, NewField::Bars).width(Length::Fill),
        ]
        .spacing(10),
    ]
    .spacing(10)
    .width(560);
    if let Some(e) = &n.error {
        card = card.push(text(e).size(12).style(theme::danger));
    }
    card = card.push(
        row![
            panels::spacer(),
            button(text("Cancel").size(13)).padding([6, 14]).style(theme::tool(false)).on_press(Message::NewCancel),
            button(text("Create").size(13)).padding([6, 14]).style(theme::primary).on_press(Message::NewCreate)
        ]
        .spacing(8),
    );
    card.into()
}
