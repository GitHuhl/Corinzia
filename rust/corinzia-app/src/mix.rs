//! The Mix tab: a strip per track (instrument plugin, effects, output,
//! sends, gain fader, pan, mute, solo), then the buses and the master, and
//! beside them the plugin browser for the selected strip or the selected
//! effect's parameters. A click on a strip's instrument shows its editor.

use crate::editor::MASTER;
use crate::fader::{Fader, PanBar, MIN_DB};
use crate::playback::Playback;
use corinzia_model::{Id, Project};
use iced::widget::{button, canvas, column, container, pick_list, row, scrollable, slider, text, Space};
use iced::{Alignment, Element, Length};

#[derive(Debug, Clone)]
pub enum MixMsg {
    Select(Id),
    /// Shows the track plugin's editor, or closes it when open.
    Editor(Id),
    /// Gain, pan and mute of a track, a bus or the master (`MASTER`).
    Gain(Id, f64),
    Pan(Id, f64),
    Released,
    Mute(Id),
    Solo(Id),
    Assign(Id, usize), // plugin index in Playback::plugins
    Rescan,
    /// An effect from `Playback::plugins` at the end of a strip's chain.
    AddEffect(Id, usize),
    /// Selects an effect (its parameters show beside the strips).
    SelectEffect(Id),
    EffectEditor(Id),
    RemoveEffect(Id, Id),
    MoveEffect(Id, Id, i32),
    Bypass(Id),
    Param(Id, u32, f64),
    AddBus,
    RemoveBus(Id),
    /// A track's output by its label ("Master" or a bus name).
    Output(Id, String),
    ToggleSend(Id, Id),
    SendLevel(Id, usize, f64),
    PresetName(String),
    SavePreset(Target),
    LoadPreset(Target, String),
    DeletePreset(Target, String),
    /// A track's MIDI program by its label ("41 Violin", or "None").
    Program(Id, String),
    /// The automation a track's arrange lane shows, by its label ("Off": none).
    AutoShow(Id, String),
    /// Dynamics as a controller, by label.
    DynamicsCc(Id, String),
    /// The track's expression map: add a line, remove one, or change one
    /// (technique, kind of switch, value, second value: all by label).
    TechAdd(Id),
    TechRemove(Id, usize),
    TechEdit(Id, usize, TechField, String),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TechField {
    Technique,
    Kind,
    Value,
    Value2,
}

/// Techniques a map line can name: texts, then articulations.
pub fn technique_names() -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    for (_, t) in corinzia_perform::TECHNIQUES {
        if !v.iter().any(|x| x == t) {
            v.push(t.to_string());
        }
    }
    v.extend(corinzia_perform::ARTICULATIONS.iter().map(|s| s.to_string()));
    v
}

pub const SWITCH_KINDS: [&str; 4] = ["Keyswitch", "Program", "Controller", "Channel"];

/// A key's name, middle C as C4 ("C-1" for 0).
pub fn key_name(k: u8) -> String {
    const N: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    format!("{}{}", N[(k % 12) as usize], k as i32 / 12 - 1)
}

/// A choice and its options, as a pick list shows them.
type Choice = (String, Vec<String>);

/// A switch line's labels: (kind, value, its choices, the second value).
fn switch_labels(a: corinzia_model::SwitchAction) -> (&'static str, String, Vec<String>, Option<Choice>) {
    use corinzia_model::SwitchAction::*;
    let numbers = |n: u32, from: u32| (from..from + n).map(|i| i.to_string()).collect::<Vec<_>>();
    match a {
        Keyswitch(k) => ("Keyswitch", key_name(k), (0..128).map(key_name).collect(), None),
        Program(p) => ("Program", crate::presets::program_label(p), (0..128).map(crate::presets::program_label).collect(), None),
        Cc(c, v) => ("Controller", format!("CC{c}"), (0..128).map(|i| format!("CC{i}")).collect(), Some((v.to_string(), numbers(128, 0)))),
        Channel(c) => ("Channel", (c + 1).to_string(), numbers(16, 1), None),
    }
}

/// The expression map table of a track.
fn techniques_block<'a>(t: &'a corinzia_model::Track) -> Element<'a, MixMsg> {
    let track = t.id;
    let mut col = column![
        text("Playing techniques").size(13),
        text("Technique texts (pizz., con sord.…) and articulations switch the sound; \"ord.\" is the normal one.").size(11).style(crate::theme::dim),
    ]
    .spacing(4);
    let names = technique_names();
    for (i, line) in t.techniques.iter().enumerate() {
        let (kind, value, values, second) = switch_labels(line.action);
        let mut r = row![
            pick_list(names.clone(), Some(line.technique.clone()), move |v| MixMsg::TechEdit(track, i, TechField::Technique, v)).text_size(11).padding([2, 4]).width(90),
            pick_list(SWITCH_KINDS.map(String::from).to_vec(), Some(kind.to_string()), move |v| MixMsg::TechEdit(track, i, TechField::Kind, v)).text_size(11).padding([2, 4]).width(84),
            pick_list(values, Some(value), move |v| MixMsg::TechEdit(track, i, TechField::Value, v)).text_size(11).padding([2, 4]).width(Length::Fill),
        ]
        .spacing(3)
        .align_y(Alignment::Center);
        if let Some((v2, choices)) = second {
            r = r.push(pick_list(choices, Some(v2), move |v| MixMsg::TechEdit(track, i, TechField::Value2, v)).text_size(11).padding([2, 4]).width(48));
        }
        r = r.push(button(text("×").size(11)).padding([2, 5]).style(crate::theme::flat(false)).on_press(MixMsg::TechRemove(track, i)));
        col = col.push(r);
    }
    col.push(button(text("+ Technique").size(12)).padding([3, 8]).style(crate::theme::tool(false)).on_press(MixMsg::TechAdd(track))).into()
}

/// What a track's lane can automate: (label, target, min, max); "Off" first.
pub fn auto_options(p: &Project, play: &Playback, track: Id) -> Vec<(String, Option<corinzia_model::AutoTarget>, f64, f64)> {
    use corinzia_model::AutoTarget;
    let mut v = vec![("Off".to_string(), None, 0.0, 1.0), ("Gain".into(), Some(AutoTarget::Gain(track)), MIN_DB, 6.0), ("Pan".into(), Some(AutoTarget::Pan(track)), -1.0, 1.0)];
    let Some(t) = p.track(track) else { return v };
    let mut plugins: Vec<(Id, std::sync::Arc<corinzia_audio::host::Plugin>)> = Vec::new();
    if let (Some(n), Some(pl)) = (t.instrument_node, play.pool.plugin_of_track(track)) {
        plugins.push((n, pl.clone()));
    }
    plugins.extend(t.inserts.iter().filter_map(|&n| Some((n, play.pool.effect(n)?.clone()))));
    for (node, pl) in plugins {
        for info in pl.params().into_iter().filter(|i| i.automatable) {
            v.push((format!("{}: {}", pl.info.name, info.name), Some(AutoTarget::Param { node, param: info.id }), info.min, info.max));
        }
    }
    v
}

pub const DYNAMICS_CC: [&str; 3] = ["Velocity only", "CC1 (modulation)", "CC11 (expression)"];

/// Whose plugin a preset is for: a track's instrument, or an effect.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Target {
    Track(Id),
    Effect(Id),
}

/// The selected plugin's presets (its id and their names), and the name typed.
pub struct Presets<'a> {
    pub names: &'a [String],
    pub name: &'a str,
}

fn presets_block<'a>(target: Target, ps: &Presets<'a>) -> Element<'a, MixMsg> {
    let mut col = column![text("Presets").size(13)].spacing(4);
    for n in ps.names {
        col = col.push(
            row![
                button(text(n.clone()).size(12)).width(Length::Fill).padding([3, 8]).style(crate::theme::tool(false)).on_press(MixMsg::LoadPreset(target, n.clone())),
                crate::panels::tip(button(text("×").size(12)).padding([3, 6]).style(crate::theme::flat(false)).on_press(MixMsg::DeletePreset(target, n.clone())), "Delete the preset"),
            ]
            .spacing(2),
        );
    }
    col.push(
        row![
            iced::widget::text_input("Preset name", ps.name).on_input(MixMsg::PresetName).on_submit(MixMsg::SavePreset(target)).padding(4).size(12),
            button(text("Save").size(12)).padding([4, 8]).style(crate::theme::tool(false)).on_press(MixMsg::SavePreset(target)),
        ]
        .spacing(4),
    )
    .into()
}

fn db_label(db: f64) -> String {
    if db <= MIN_DB {
        "−∞ dB".to_string()
    } else {
        format!("{db:+.1} dB")
    }
}

fn pan_label(pan: f64) -> String {
    match pan {
        x if x.abs() < 0.005 => "C".to_string(),
        x if x < 0.0 => format!("L{:.0}", -x * 100.0),
        x => format!("R{:.0}", x * 100.0),
    }
}

/// A strip's effects: each a button (selects it), a bypass toggle; then "+ Effect".
fn inserts<'a>(p: &'a Project, owner: Id, list: &'a [Id], fx: Option<Id>) -> Element<'a, MixMsg> {
    let mut col = column![text("Effects").size(10).style(crate::theme::faint)].spacing(3);
    for &n in list {
        let Some(node) = p.node(n) else { continue };
        let name = if node.bypassed { format!("({})", node.name) } else { node.name.clone() };
        col = col.push(
            row![
                button(text(name).size(11)).width(Length::Fill).padding([2, 5]).style(crate::theme::tool(fx == Some(n))).on_press(MixMsg::SelectEffect(n)),
                crate::panels::tip(button(text("⏻").size(11)).padding([2, 5]).style(crate::theme::flat(!node.bypassed)).on_press(MixMsg::Bypass(n)), "Bypass (or bring back)"),
            ]
            .spacing(2),
        );
    }
    col.push(crate::panels::tip(button(text("+ Effect").size(11)).width(Length::Fill).padding([2, 5]).style(crate::theme::flat(false)).on_press(MixMsg::Select(owner)), "Choose an effect for this strip on the right"))
        .into()
}

fn fader_block<'a>(id: Id, db: f64, pan: Option<f64>, mute: Option<bool>, solo: Option<bool>) -> Element<'a, MixMsg> {
    let toggle = |label: &'a str, on: bool, m: MixMsg| button(text(label).size(12)).padding([3, 8]).style(crate::theme::tool(on)).on_press(m);
    let mut levels = row![text(db_label(db)).size(12).width(Length::Fill)].spacing(4).align_y(Alignment::Center);
    if let Some(m) = mute {
        levels = levels.push(toggle("M", m, MixMsg::Mute(id)));
    }
    if let Some(s) = solo {
        levels = levels.push(toggle("S", s, MixMsg::Solo(id)));
    }
    let mut col = column![
        crate::panels::tip(canvas(Fader { track: id, db }).width(Length::Fill).height(160), "Drag (Shift: fine), click to jump, double-click: 0 dB, wheel: ±0.5 dB"),
        levels,
    ]
    .spacing(8);
    if let Some(pan) = pan {
        col = col.push(
            row![
                crate::panels::tip(canvas(PanBar { track: id, pan }).width(Length::Fill).height(18), "Pan: drag, double-click to centre"),
                text(pan_label(pan)).size(11).width(28).align_x(Alignment::End),
            ]
            .align_y(Alignment::Center),
        );
    }
    col.into()
}

fn strip<'a>(p: &'a Project, play: &'a Playback, t: &'a corinzia_model::Track, selected: bool, fx: Option<Id>, shown: Option<corinzia_model::AutoTarget>) -> Element<'a, MixMsg> {
    let plugin = play.plugin_name(t.id);
    let players: Vec<String> = p.instruments.iter().filter(|i| i.track == Some(t.id)).map(|i| i.name.clone()).collect();
    let mut outputs: Vec<String> = vec!["Master".into()];
    outputs.extend(p.buses.iter().map(|b| b.name.clone()));
    let out = t.output_bus.and_then(|b| p.bus(b)).map(|b| b.name.clone()).unwrap_or_else(|| "Master".into());
    let track = t.id;
    let mut body = column![
        button(text(&t.name).size(14)).padding([0, 4]).style(crate::theme::flat(selected)).on_press(MixMsg::Select(t.id)),
        text(players.join(", ")).size(11).style(crate::theme::dim),
        crate::panels::tip(
            button(text(plugin).size(12)).width(Length::Fill).padding([4, 6]).style(crate::theme::tool(play.pool.editor_is_open(t.id))).on_press(MixMsg::Editor(t.id)),
            "Show the instrument's editor (click again to close it)",
        ),
        inserts(p, t.id, &t.inserts, fx),
        column![text("Output").size(10).style(crate::theme::faint), pick_list(outputs, Some(out), move |o| MixMsg::Output(track, o)).text_size(11).padding([2, 5]).width(Length::Fill)].spacing(2),
    ]
    .spacing(6)
    .width(130);
    if !p.buses.is_empty() {
        let mut sends = column![text("Sends").size(10).style(crate::theme::faint)].spacing(2);
        for b in &p.buses {
            let at = t.sends.iter().position(|s| s.bus == b.id);
            sends = sends.push(crate::panels::tip(button(text(format!("→ {}", b.name)).size(11)).width(Length::Fill).padding([2, 5]).style(crate::theme::tool(at.is_some())).on_press(MixMsg::ToggleSend(t.id, b.id)), "A send to this bus (click again to remove it)"));
            if let Some(i) = at {
                let db = t.sends[i].gain_db;
                sends = sends.push(row![slider(MIN_DB..=6.0, db, move |v| MixMsg::SendLevel(track, i, (v * 2.0).round() / 2.0)).step(0.5).on_release(MixMsg::Released), text(db_label(db)).size(10).width(52)].spacing(4).align_y(Alignment::Center));
            }
        }
        body = body.push(sends);
    }
    let options = auto_options(p, play, t.id);
    let shown = options.iter().find(|o| o.1.is_some() && o.1 == shown).map(|o| o.0.clone());
    let autos: Vec<String> = options.into_iter().map(|o| o.0).collect();
    let dynamics = match t.dynamics_cc {
        Some(1) => DYNAMICS_CC[1],
        Some(11) => DYNAMICS_CC[2],
        _ => DYNAMICS_CC[0],
    };
    body = body.push(
        column![
            text("Dynamics").size(10).style(crate::theme::faint),
            crate::panels::tip(pick_list(DYNAMICS_CC.map(String::from).to_vec(), Some(dynamics.to_string()), move |l| MixMsg::DynamicsCc(track, l)).text_size(11).padding([2, 5]).width(Length::Fill), "Send the dynamics and hairpins as a controller too (sample libraries)"),
            text("Automation lane").size(10).style(crate::theme::faint),
            crate::panels::tip(pick_list(autos, shown, move |l| MixMsg::AutoShow(track, l)).placeholder("Show…").text_size(11).padding([2, 5]).width(Length::Fill), "Show a curve on the track's lane above: gain, pan or a plugin parameter"),
        ]
        .spacing(2),
    );
    body = body.push(fader_block(t.id, t.gain_db, Some(t.pan), Some(t.mute), Some(t.solo)));
    container(body.align_x(Alignment::Center)).padding(10).style(crate::theme::strip(selected)).into()
}

fn bus_strip<'a>(p: &'a Project, b: &'a corinzia_model::Bus, selected: bool, fx: Option<Id>) -> Element<'a, MixMsg> {
    let body = column![
        row![
            button(text(&b.name).size(14)).padding([0, 4]).style(crate::theme::flat(selected)).on_press(MixMsg::Select(b.id)),
            crate::panels::spacer(),
            crate::panels::tip(button(text("×").size(12)).padding([0, 5]).style(crate::theme::flat(false)).on_press(MixMsg::RemoveBus(b.id)), "Remove the bus (tracks sent here lose the send)"),
        ]
        .align_y(Alignment::Center),
        text("Bus").size(11).style(crate::theme::dim),
        inserts(p, b.id, &b.inserts, fx),
        fader_block(b.id, b.gain_db, Some(b.pan), Some(b.mute), None),
    ]
    .spacing(6)
    .width(130);
    container(body).padding(10).style(crate::theme::strip(selected)).into()
}

fn master_strip<'a>(p: &'a Project, selected: bool, fx: Option<Id>) -> Element<'a, MixMsg> {
    let body = column![
        button(text("Master").size(14)).padding([0, 4]).style(crate::theme::flat(selected)).on_press(MixMsg::Select(MASTER)),
        text("Everything").size(11).style(crate::theme::dim),
        inserts(p, MASTER, &p.master.inserts, fx),
        fader_block(MASTER, p.master.gain_db, None, None, None),
    ]
    .spacing(6)
    .width(130);
    container(body).padding(10).style(crate::theme::strip(selected)).into()
}

/// The selected effect: its editor, place in the chain, and parameters.
fn effect_panel<'a>(p: &'a Project, play: &'a Playback, node: Id, ps: &Presets<'a>) -> Element<'a, MixMsg> {
    let owner = if p.master.inserts.contains(&node) { Some(MASTER) } else { p.tracks.iter().find(|t| t.inserts.contains(&node)).map(|t| t.id).or_else(|| p.buses.iter().find(|b| b.inserts.contains(&node)).map(|b| b.id)) };
    let name = p.node(node).map(|n| n.name.clone()).unwrap_or_default();
    let mut col = column![text(name).size(16)].spacing(6);
    let Some(owner) = owner else { return col.into() };
    let small = |label: &'a str, hint: &'a str, m: MixMsg| crate::panels::tip(button(text(label).size(12)).padding([4, 8]).style(crate::theme::tool(false)).on_press(m), hint);
    let plugin = play.pool.effect(node);
    let mut tools = row![small("←", "Earlier in the chain", MixMsg::MoveEffect(owner, node, -1)), small("→", "Later in the chain", MixMsg::MoveEffect(owner, node, 1)), small("⏻", "Bypass", MixMsg::Bypass(node)), small("Remove", "Take the effect out", MixMsg::RemoveEffect(owner, node))].spacing(4);
    if plugin.is_some_and(|x| x.has_editor()) {
        let open = plugin.is_some_and(|x| x.editor_is_open());
        tools = tools.push(button(text(if open { "Close editor" } else { "Editor" }).size(12)).padding([4, 8]).style(crate::theme::tool(open)).on_press(MixMsg::EffectEditor(node)));
    }
    col = col.push(tools);
    if p.node(node).is_some_and(|n| n.bypassed) {
        col = col.push(text("Bypassed").size(12).style(crate::theme::dim));
    }
    match plugin {
        None => col = col.push(text("Not loaded (bypassed, or it failed to load).").size(12).style(crate::theme::dim)),
        Some(pl) => {
            let params = pl.params();
            if params.is_empty() && !pl.has_editor() {
                col = col.push(text("No parameters to show.").size(12).style(crate::theme::dim));
            }
            for info in params {
                let v = pl.param_value(info.id).unwrap_or(info.default);
                let id = info.id;
                let step = if info.stepped { 1.0 } else { ((info.max - info.min) / 200.0).max(1e-6) };
                col = col.push(
                    column![
                        row![text(info.name.clone()).size(12).width(Length::Fill), text(pl.param_text(id, v)).size(12).style(crate::theme::dim)],
                        slider(info.min..=info.max, v, move |x| MixMsg::Param(node, id, x)).step(step),
                    ]
                    .spacing(2),
                );
            }
            col = col.push(presets_block(Target::Effect(node), ps));
        }
    }
    col.into()
}

pub fn view<'a>(p: &'a Project, play: &'a Playback, selected: Option<Id>, fx: Option<Id>, ps: Presets<'a>, shown: &std::collections::HashMap<Id, corinzia_model::AutoTarget>) -> Element<'a, MixMsg> {
    let mut strips = row(p.tracks.iter().map(|t| strip(p, play, t, selected == Some(t.id), fx, shown.get(&t.id).copied()))).spacing(8).padding(12);
    for b in &p.buses {
        strips = strips.push(bus_strip(p, b, selected == Some(b.id), fx));
    }
    strips = strips.push(crate::panels::tip(button(text("+ Bus").size(12)).padding([6, 10]).style(crate::theme::tool(false)).on_press(MixMsg::AddBus), "A bus: send tracks to it for shared effects (a reverb)"));
    strips = strips.push(master_strip(p, selected == Some(MASTER), fx));
    let mut browser = column![].spacing(6).padding(12).width(300);
    if let Some(node) = fx.filter(|n| p.node(*n).is_some()) {
        browser = browser.push(effect_panel(p, play, node, &ps)).push(Space::new().height(12));
    }
    let effect_list = |mut b: iced::widget::Column<'a, MixMsg>, owner: Id| {
        b = b.push(text(format!("Add an effect to {}", if owner == MASTER { "the master".to_string() } else { p.track(owner).map(|t| t.name.clone()).or_else(|| p.bus(owner).map(|x| x.name.clone())).unwrap_or_default() })).size(13));
        for (i, info) in play.plugins.iter().enumerate().filter(|(_, i)| !i.is_instrument()) {
            let label = if info.bundle.as_os_str() == corinzia_audio::host::BUILTIN { info.name.clone() } else { format!("{} — {} ({})", info.name, info.vendor, info.format.label()) };
            b = b.push(button(text(label).size(13)).width(Length::Fill).padding([5, 8]).style(crate::theme::tool(false)).on_press(MixMsg::AddEffect(owner, i)));
        }
        if play.plugins.is_empty() {
            b = b.push(text("Rescan to list plugins.").size(11).style(crate::theme::dim));
        }
        b
    };
    match selected {
        None => browser = browser.push(text("Instruments and effects").size(16)).push(text("Select a strip to choose its instrument or add effects.").size(13).style(crate::theme::dim)),
        Some(owner) if p.track(owner).is_none() => browser = effect_list(browser, owner),
        Some(id) => {
            let t = p.track(id).unwrap();
            browser = browser.push(text("Instruments").size(16));
            let current = play.plugin_name(t.id);
            let open = play.pool.editor_is_open(t.id);
            let editor = play.pool.plugin_of_track(t.id).is_some_and(|p| p.has_editor());
            let mut show = button(text(if open { "Close editor" } else { "Show editor" }).size(13)).padding([5, 10]).style(crate::theme::tool(open));
            if editor {
                show = show.on_press(MixMsg::Editor(t.id));
            }
            browser = browser.push(row![text(format!("{}: {current}", t.name)).size(13).width(Length::Fill), show].spacing(8).align_y(Alignment::Center));
            if !editor && play.pool.plugin_of_track(t.id).is_some() {
                browser = browser.push(text(format!("{current} has no editor of its own.")).size(11).style(crate::theme::dim));
            }
            let mut programs: Vec<String> = vec!["None".into()];
            programs.extend((0..128u8).map(crate::presets::program_label));
            let track = t.id;
            browser = browser.push(
                row![
                    text("MIDI program").size(12).style(crate::theme::dim),
                    pick_list(programs, Some(t.program.map(crate::presets::program_label).unwrap_or_else(|| "None".into())), move |l| MixMsg::Program(track, l)).text_size(12).padding([3, 6]).width(Length::Fill),
                ]
                .spacing(6)
                .align_y(Alignment::Center),
            );
            if fx.is_none() && play.pool.plugin_of_track(t.id).is_some() {
                browser = browser.push(presets_block(Target::Track(t.id), &ps));
            }
            if fx.is_none() {
                browser = browser.push(techniques_block(t));
            }
            browser = browser.push(Space::new().height(4));
            let current_bundle = play.pool.plugin_of_track(t.id).map(|p| (p.info.bundle.clone(), p.info.id.clone()));
            for (i, info) in play.plugins.iter().enumerate().filter(|(_, i)| i.is_instrument()) {
                let label = if info.bundle.as_os_str() == corinzia_audio::host::BUILTIN {
                    format!("{} — {}", info.name, info.vendor)
                } else {
                    format!("{} — {} ({})", info.name, info.vendor, info.format.label())
                };
                let on = current_bundle.as_ref().is_some_and(|(b, id)| *b == info.bundle && *id == info.id);
                browser = browser.push(button(text(label).size(13)).width(Length::Fill).padding([5, 8]).style(crate::theme::tool(on)).on_press(MixMsg::Assign(t.id, i)));
            }
            browser = effect_list(browser.push(Space::new().height(8)), t.id);
        }
    }
    browser = browser.push(Space::new().height(8)).push(button(text("Rescan plugins").size(13)).padding([5, 10]).style(crate::theme::tool(false)).on_press(MixMsg::Rescan));
    for e in &play.scan_errors {
        browser = browser.push(text(e).size(11).style(crate::theme::danger));
    }
    for e in play.pool.errors.iter().chain(&play.clip_errors) {
        browser = browser.push(text(e).size(11).style(crate::theme::danger));
    }
    row![
        scrollable(strips).direction(scrollable::Direction::Both { vertical: scrollable::Scrollbar::default(), horizontal: scrollable::Scrollbar::default() }).width(Length::Fill).height(Length::Fill),
        container(scrollable(browser).height(Length::Fill)).padding(10).height(Length::Fill).style(crate::theme::strip(false)),
    ]
    .into()
}
