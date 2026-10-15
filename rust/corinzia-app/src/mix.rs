//! The Mix tab: a strip per track (instrument plugin, gain fader, pan,
//! mute, solo) and the plugin browser for the selected strip.

use crate::playback::Playback;
use corinzia_model::{Id, Project};
use crate::fader::{Fader, PanBar, MIN_DB};
use iced::widget::{button, canvas, column, container, row, scrollable, text, Space};
use iced::{Alignment, Element, Length};

#[derive(Debug, Clone)]
pub enum MixMsg {
    Select(Id),
    Gain(Id, f64),
    Pan(Id, f64),
    Released,
    Mute(Id),
    Solo(Id),
    Assign(Id, usize), // plugin index in Playback::plugins
    Rescan,
}

fn strip<'a>(p: &'a Project, play: &'a Playback, t: &'a corinzia_model::Track, selected: bool) -> Element<'a, MixMsg> {
    let toggle = |label: &'a str, on: bool, m: MixMsg| button(text(label).size(12)).padding([3, 8]).style(crate::theme::tool(on)).on_press(m);
    let plugin = play.plugin_name(t.id);
    let players: Vec<String> = p.instruments.iter().filter(|i| i.track == Some(t.id)).map(|i| i.name.clone()).collect();
    let body = column![
        text(&t.name).size(14),
        text(players.join(", ")).size(11).style(crate::theme::dim),
        button(text(plugin).size(12)).width(Length::Fill).padding([4, 6]).style(crate::theme::tool(false)).on_press(MixMsg::Select(t.id)),
        crate::panels::tip(
            canvas(Fader { track: t.id, db: t.gain_db }).width(Length::Fill).height(160),
            "Drag (Shift: fine), click to jump, double-click: 0 dB, wheel: ±0.5 dB",
        ),
        row![
            text(if t.gain_db <= MIN_DB { "−∞ dB".to_string() } else { format!("{:+.1} dB", t.gain_db) }).size(12).width(Length::Fill),
            toggle("M", t.mute, MixMsg::Mute(t.id)),
            toggle("S", t.solo, MixMsg::Solo(t.id)),
        ]
        .spacing(4)
        .align_y(Alignment::Center),
        row![
            crate::panels::tip(canvas(PanBar { track: t.id, pan: t.pan }).width(Length::Fill).height(18), "Pan: drag, double-click to centre"),
            text(match t.pan {
                x if x.abs() < 0.005 => "C".to_string(),
                x if x < 0.0 => format!("L{:.0}", -x * 100.0),
                x => format!("R{:.0}", x * 100.0),
            })
            .size(11)
            .width(28)
            .align_x(Alignment::End),
        ]
        .align_y(Alignment::Center),
    ]
    .spacing(8)
    .align_x(Alignment::Center)
    .width(120);
    container(body).padding(10).style(crate::theme::strip(selected)).into()
}

pub fn view<'a>(p: &'a Project, play: &'a Playback, selected: Option<Id>) -> Element<'a, MixMsg> {
    let strips = row(p.tracks.iter().map(|t| strip(p, play, t, selected == Some(t.id)))).spacing(8).padding(12);
    let mut browser = column![text("Instruments").size(16)].spacing(6).padding(12).width(300);
    match selected.and_then(|id| p.track(id)) {
        None => browser = browser.push(text("Select a strip to choose its instrument.").size(13).style(crate::theme::dim)),
        Some(t) => {
            browser = browser.push(text(format!("For {}:", t.name)).size(13));
            let current = play.plugin_name(t.id);
            for (i, info) in play.plugins.iter().enumerate().filter(|(_, i)| i.is_instrument()) {
                let label = format!("{} — {}", info.name, info.vendor);
                let on = info.name == current;
                browser = browser.push(button(text(label).size(13)).width(Length::Fill).padding([5, 8]).style(crate::theme::tool(on)).on_press(MixMsg::Assign(t.id, i)));
            }
            let effects = play.plugins.iter().filter(|i| !i.is_instrument()).count();
            if effects > 0 {
                browser = browser.push(text(format!("{effects} effect plugin(s) found; effect routing comes with buses.")).size(11).style(crate::theme::dim));
            }
        }
    }
    browser = browser.push(Space::new().height(8)).push(button(text("Rescan plugins").size(13)).padding([5, 10]).style(crate::theme::tool(false)).on_press(MixMsg::Rescan));
    for e in &play.scan_errors {
        browser = browser.push(text(e).size(11).style(crate::theme::danger));
    }
    for e in &play.pool.errors {
        browser = browser.push(text(e).size(11).style(crate::theme::danger));
    }
    row![
        scrollable(strips).direction(scrollable::Direction::Horizontal(scrollable::Scrollbar::default())).width(Length::Fill).height(Length::Fill),
        container(scrollable(browser).height(Length::Fill)).padding(10).height(Length::Fill).style(crate::theme::strip(false)),
    ]
    .into()
}
