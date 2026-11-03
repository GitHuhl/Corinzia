//! The Players card: the instrumentation being edited, applied as one undo step.

use crate::*;
use iced::widget::column;

/// The Players card: the instrumentation being edited, applied as a whole.
pub(crate) struct PlayersDialog {
    pub(crate) rows: Vec<PlayerRow>,
    /// Instruments to add ("Flute, 2 Violin").
    pub(crate) add: String,
    pub(crate) error: Option<String>,
}

pub(crate) struct PlayerRow {
    /// The player in the score, or None for one to add.
    pub(crate) player: Option<corinzia_model::Id>,
    /// Its instruments as typed: the first, then doublings.
    pub(crate) instruments: String,
    pub(crate) detail: String,
}

#[derive(Debug, Clone)]
pub(crate) enum PlayersMsg {
    Open,
    Move(usize, isize),
    Remove(usize),
    /// A row's instruments retyped.
    Edit(usize, String),
    AddInput(String),
    Add,
    /// One of the quick-add buttons.
    Quick(&'static str),
    Apply,
    Cancel,
}

/// The quick-add buttons of the Players card.
pub(crate) const QUICK_ADD: [&str; 16] = ["Flute", "Oboe", "Clarinet", "Bassoon", "Horn", "Trumpet", "Trombone", "Piano", "Soprano", "Alto", "Tenor", "Bass", "Violin", "Viola", "Cello", "Double bass"];

/// The Players card: the list (reorder, remove), and instruments to add.
pub(crate) fn players_card(d: &PlayersDialog) -> Element<'_, Message> {
    let bold = iced::Font { weight: iced::font::Weight::Bold, ..iced::Font::DEFAULT };
    let small = |label: &'static str, hint: &'static str, m: Option<PlayersMsg>| {
        let mut b = button(text(label).size(12)).padding([2, 7]).style(theme::flat(false));
        if let Some(m) = m {
            b = b.on_press(Message::Players(m));
        }
        panels::tip(b, hint)
    };
    let n = d.rows.len();
    let rows = column(d.rows.iter().enumerate().map(|(i, r)| {
        row![
            text(format!("{}", i + 1)).size(12).style(theme::dim).width(22),
            column![
                text_input("Instrument, then doublings: Flute, Piccolo", &r.instruments).on_input(move |v| Message::Players(PlayersMsg::Edit(i, v))).on_submit(Message::Players(PlayersMsg::Apply)).padding([3, 6]).size(14),
                text(&r.detail).size(11).style(theme::dim)
            ]
            .spacing(2)
            .width(Length::Fill),
            small("↑", "Move up", (i > 0).then_some(PlayersMsg::Move(i, -1))),
            small("↓", "Move down", (i + 1 < n).then_some(PlayersMsg::Move(i, 1))),
            small("✕", "Remove the player and its music", (n > 1).then_some(PlayersMsg::Remove(i))),
        ]
        .spacing(4)
        .align_y(iced::Alignment::Center)
        .into()
    }))
    .spacing(6);
    let quick = iced::widget::Row::with_children(QUICK_ADD.iter().map(|name| button(text(*name).size(12)).padding([3, 8]).style(theme::segment(false)).on_press(Message::Players(PlayersMsg::Quick(name))).into())).spacing(4).wrap();
    // Add first, the list below: the buttons stay put while the list grows.
    let mut card = column![
        text("Players").size(18).font(bold),
        text("Add").size(12).style(theme::dim),
        quick,
        row![
            text_input("Instruments to add: \"Flute, 2 Violin\"; any name", &d.add).id(PLAYERS_ADD).on_input(|v| Message::Players(PlayersMsg::AddInput(v))).on_submit(Message::Players(PlayersMsg::Add)).padding(6).size(14),
            button(text("Add").size(13)).padding([6, 14]).style(theme::tool(false)).on_press(Message::Players(PlayersMsg::Add)),
        ]
        .spacing(8),
    ]
    .spacing(10)
    .width(480);
    if let Some(e) = &d.error {
        card = card.push(text(e).size(12).style(theme::danger));
    }
    card.push(text("In the score, top to bottom. Retype an instrument to change it, or its short name after \"/\"; add doublings after a comma (Flute, Piccolo). Removing a player removes its music.").size(12).style(theme::dim))
        .push(iced::widget::scrollable(rows).height(Length::Fixed((n.max(1) as f32 * 52.0).min(400.0))))
        .push(
        row![
            panels::spacer(),
            button(text("Cancel").size(13)).padding([6, 14]).style(theme::tool(false)).on_press(Message::Players(PlayersMsg::Cancel)),
            button(text("Apply").size(13)).padding([6, 14]).style(theme::primary).on_press(Message::Players(PlayersMsg::Apply)),
        ]
        .spacing(8),
    )
    .into()
}

impl App {
    /// The Players card: open it on the score's players, edit the list,
    /// apply it as one undo step.
    pub(crate) fn players_msg(&mut self, m: PlayersMsg) {
        if let PlayersMsg::Open = m {
            let p = &self.editor.project;
            let lanes = self.editor.movement().map(|m| m.lanes.clone()).unwrap_or_default();
            let rows = p
                .players
                .iter()
                .map(|pl| {
                    // "Name / Abbr." (the abbreviation names later systems).
                    let instruments: Vec<String> = pl
                        .instruments
                        .iter()
                        .filter_map(|i| p.instrument(*i))
                        .map(|i| if i.engraver_name.is_empty() { i.name.clone() } else { format!("{} / {}", i.name, i.engraver_name) })
                        .collect();
                    let staves = lanes.iter().filter(|l| l.player == pl.id).count();
                    let detail = if staves == 1 { "1 staff".to_string() } else { format!("{staves} staves") };
                    PlayerRow { player: Some(pl.id), instruments: instruments.join(", "), detail }
                })
                .collect();
            self.players = Some(PlayersDialog { rows, add: String::new(), error: None });
            self.help = false;
            self.export_menu = false;
            return;
        }
        let Some(d) = self.players.as_mut() else { return };
        d.error = None;
        let add = |d: &mut PlayersDialog, text: &str| match files::parse_instruments(text) {
            Ok(list) => {
                for (name, _, clef) in list {
                    let staves = clef.split('+').count();
                    let detail = format!("new · {}", if staves == 1 { "1 staff".to_string() } else { format!("{staves} staves") });
                    d.rows.push(PlayerRow { player: None, instruments: name, detail });
                }
                true
            }
            Err(e) => {
                d.error = Some(e);
                false
            }
        };
        match m {
            PlayersMsg::Open => {}
            PlayersMsg::Move(i, by) => {
                let j = i as isize + by;
                if i < d.rows.len() && j >= 0 && (j as usize) < d.rows.len() {
                    d.rows.swap(i, j as usize);
                }
            }
            PlayersMsg::Remove(i) => {
                if i < d.rows.len() {
                    d.rows.remove(i);
                }
            }
            PlayersMsg::Edit(i, v) => {
                if let Some(r) = d.rows.get_mut(i) {
                    r.instruments = v;
                }
            }
            PlayersMsg::AddInput(v) => d.add = v,
            PlayersMsg::Add => {
                let text = d.add.clone();
                if add(d, &text) {
                    d.add.clear();
                }
            }
            PlayersMsg::Quick(name) => {
                add(d, name);
            }
            PlayersMsg::Cancel => self.players = None,
            PlayersMsg::Apply => {
                let mut slots = Vec::new();
                for (k, r) in d.rows.iter().enumerate() {
                    match files::parse_instrument_list(&r.instruments) {
                        Ok(instruments) => slots.push(match r.player {
                            Some(id) => editor::PlayerSlot::Keep { id, instruments },
                            None => editor::PlayerSlot::New { instruments },
                        }),
                        Err(e) => {
                            d.error = Some(format!("Player {}: {e}", k + 1));
                            return;
                        }
                    }
                }
                match self.editor.set_players(&slots) {
                    Ok(()) => {
                        self.players = None;
                        // A part whose player went shows the full score.
                        if self.part.is_some_and(|p| self.editor.project.player(p).is_none()) {
                            self.part = None;
                        }
                    }
                    Err(e) => d.error = Some(e),
                }
            }
        }
    }
}
