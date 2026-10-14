//! The Mix tab: a strip per track (instrument plugin, gain fader, pan,
//! mute, solo) and the plugin browser for the selected strip.

use crate::playback::Playback;
use corinzia_model::{Id, Project};
use iced::widget::{button, column, container, row, scrollable, slider, text, vertical_slider, Space};
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
    let toggle = |label: &'a str, on: bool, m: MixMsg| button(text(label).size(12)).padding([3, 8]).style(if on { button::primary } else { button::secondary }).on_press(m);
    let plugin = play.plugin_name(t.id);
    let players: Vec<String> = p.instruments.iter().filter(|i| i.track == Some(t.id)).map(|i| i.name.clone()).collect();
    let body = column![
        text(&t.name).size(14),
        text(players.join(", ")).size(11).style(text::secondary),
        button(text(plugin).size(12)).width(Length::Fill).padding([4, 6]).style(button::secondary).on_press(MixMsg::Select(t.id)),
        container(vertical_slider(-60.0..=12.0, t.gain_db, move |v| MixMsg::Gain(t.id, v)).step(0.5).on_release(MixMsg::Released).height(130)).center_x(Length::Fill),
        text(if t.gain_db <= -59.9 { "−∞ dB".to_string() } else { format!("{:+.1} dB", t.gain_db) }).size(12),
        slider(-1.0..=1.0, t.pan, move |v| MixMsg::Pan(t.id, v)).step(0.01).on_release(MixMsg::Released),
        text(match t.pan {
            x if x.abs() < 0.005 => "C".to_string(),
            x if x < 0.0 => format!("L{:.0}", -x * 100.0),
            x => format!("R{:.0}", x * 100.0),
        })
        .size(11),
        row![toggle("M", t.mute, MixMsg::Mute(t.id)), toggle("S", t.solo, MixMsg::Solo(t.id))].spacing(6),
    ]
    .spacing(8)
    .align_x(Alignment::Center)
    .width(120);
    container(body).padding(10).style(if selected { container::bordered_box } else { container::rounded_box }).into()
}

pub fn view<'a>(p: &'a Project, play: &'a Playback, selected: Option<Id>) -> Element<'a, MixMsg> {
    let strips = row(p.tracks.iter().map(|t| strip(p, play, t, selected == Some(t.id)))).spacing(8).padding(12);
    let mut browser = column![text("Instruments").size(16)].spacing(6).padding(12).width(300);
    match selected.and_then(|id| p.track(id)) {
        None => browser = browser.push(text("Select a strip to choose its instrument.").size(13).style(text::secondary)),
        Some(t) => {
            browser = browser.push(text(format!("For {}:", t.name)).size(13));
            let current = play.plugin_name(t.id);
            for (i, info) in play.plugins.iter().enumerate().filter(|(_, i)| i.is_instrument()) {
                let label = format!("{} — {}", info.name, info.vendor);
                let on = info.name == current;
                browser = browser.push(button(text(label).size(13)).width(Length::Fill).padding([5, 8]).style(if on { button::primary } else { button::secondary }).on_press(MixMsg::Assign(t.id, i)));
            }
            let effects = play.plugins.iter().filter(|i| !i.is_instrument()).count();
            if effects > 0 {
                browser = browser.push(text(format!("{effects} effect plugin(s) found; effect routing comes with buses.")).size(11).style(text::secondary));
            }
        }
    }
    browser = browser.push(Space::new().height(8)).push(button(text("Rescan plugins").size(13)).style(button::secondary).on_press(MixMsg::Rescan));
    for e in &play.scan_errors {
        browser = browser.push(text(e).size(11).style(text::danger));
    }
    for e in &play.pool.errors {
        browser = browser.push(text(e).size(11).style(text::danger));
    }
    row![
        scrollable(strips).direction(scrollable::Direction::Horizontal(scrollable::Scrollbar::default())).width(Length::Fill).height(Length::Fill),
        container(scrollable(browser)).height(Length::Fill).style(container::bordered_box),
    ]
    .into()
}
