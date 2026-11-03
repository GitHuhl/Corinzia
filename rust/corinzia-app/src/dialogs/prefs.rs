//! The Preferences card: theme, audio, MIDI keyboard and plugin folders.

use crate::app::*;
use crate::LIVE_AUDIO;
use crate::{midi_in, panels, playback, prefs, theme};
use iced::widget::{button, column, row, text, text_input};
use iced::{Element, Length};

/// The Preferences card: a copy being edited, the plugin folders as typed,
/// the output devices found.
pub(crate) struct PrefsDialog {
    pub(crate) draft: prefs::Prefs,
    pub(crate) paths: String,
    pub(crate) devices: Vec<(String, String)>,
    pub(crate) midi_ports: Vec<(String, String)>,
    pub(crate) error: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) enum PrefsMsg {
    Open,
    Theme(String),
    Device(String),
    Rate(u32),
    Buffer(u32),
    Midi(String),
    Paths(String),
    DefaultPaths(bool),
    Cancel,
    Apply,
}

/// The Preferences card.
pub(crate) fn prefs_card(d: &PrefsDialog) -> Element<'_, Message> {
    let bold = iced::Font { weight: iced::font::Weight::Bold, ..iced::Font::DEFAULT };
    let pm = |m: PrefsMsg| Message::Prefs(m);
    let label = |t: &'static str| text(t).size(12).style(theme::dim);
    let themes: Vec<String> = theme::THEMES.iter().map(|t| t.0.to_string()).collect();
    let mut devices: Vec<String> = vec!["System default".into()];
    devices.extend(d.devices.iter().map(|x| x.1.clone()));
    let device = d.devices.iter().find(|x| x.0 == d.draft.output_device).map(|x| x.1.clone()).unwrap_or_else(|| if d.draft.output_device.is_empty() { "System default".into() } else { d.draft.output_device.clone() });
    let pick = |opts: Vec<String>, cur: String, f: fn(String) -> PrefsMsg| iced::widget::pick_list(opts, Some(cur), move |v| pm(f(v))).text_size(13).padding([5, 9]).width(Length::Fill);
    let mut midi_ports: Vec<String> = vec!["None".into()];
    midi_ports.extend(d.midi_ports.iter().map(|x| x.1.clone()));
    let midi_port = d.midi_ports.iter().find(|x| x.0 == d.draft.midi_input).map(|x| x.1.clone()).unwrap_or_else(|| if d.draft.midi_input.is_empty() { "None".into() } else { d.draft.midi_input.clone() });
    let rates: Vec<String> = prefs::SAMPLE_RATES.iter().map(|r| r.to_string()).collect();
    let buffers: Vec<String> = prefs::BUFFERS.iter().map(|r| r.to_string()).collect();
    let mut card = column![
        text("Preferences").size(18).font(bold),
        column![label("Theme"), pick(themes, d.draft.theme.clone(), PrefsMsg::Theme)].spacing(3),
        column![label("Audio output"), pick(devices, device, PrefsMsg::Device)].spacing(3),
        column![label("MIDI keyboard (note input)"), pick(midi_ports, midi_port, PrefsMsg::Midi)].spacing(3),
        row![
            column![label("Sample rate (Hz)"), pick(rates, d.draft.sample_rate.to_string(), |v| PrefsMsg::Rate(v.parse().unwrap_or(48000)))].spacing(3).width(Length::Fill),
            column![label("Buffer (frames)"), pick(buffers, d.draft.buffer.to_string(), |v| PrefsMsg::Buffer(v.parse().unwrap_or(2048)))].spacing(3).width(Length::Fill),
        ]
        .spacing(10),
        column![
            label("More plugin folders (separated by ;)"),
            text_input("/path/to/plugins", &d.paths).on_input(move |v| pm(PrefsMsg::Paths(v))).padding(5).size(14),
            button(text("Also search the usual folders (and CLAP_PATH, VST3_PATH)").size(13)).padding([5, 10]).style(theme::tool(d.draft.default_plugin_paths)).on_press(pm(PrefsMsg::DefaultPaths(!d.draft.default_plugin_paths))),
        ]
        .spacing(5),
        text("Changing the output, rate or buffer restarts the audio; plugins keep their settings.").size(11).style(theme::dim),
    ]
    .spacing(10)
    .width(520);
    if let Some(e) = &d.error {
        card = card.push(text(e).size(12).style(theme::danger));
    }
    card.push(
        row![
            panels::spacer(),
            button(text("Cancel").size(13)).padding([6, 14]).style(theme::tool(false)).on_press(pm(PrefsMsg::Cancel)),
            button(text("Apply").size(13)).padding([6, 14]).style(theme::primary).on_press(pm(PrefsMsg::Apply)),
        ]
        .spacing(8),
    )
    .into()
}

impl App {
    /// The Preferences card: edit a copy, apply it (saved; the audio output
    /// restarts when the device, rate or buffer change).
    pub(crate) fn prefs_msg(&mut self, m: PrefsMsg) {
        if let PrefsMsg::Open = m {
            self.prefs_dialog = Some(PrefsDialog { paths: self.prefs.plugin_paths.join("; "), draft: self.prefs.clone(), devices: playback::Playback::devices(), midi_ports: midi_in::ports(), error: None });
            return;
        }
        let Some(d) = self.prefs_dialog.as_mut() else { return };
        d.error = None;
        match m {
            PrefsMsg::Open => {}
            PrefsMsg::Theme(t) => {
                d.draft.theme = t;
                theme::set(&d.draft.theme); // seen at once
            }
            PrefsMsg::Device(label) => d.draft.output_device = d.devices.iter().find(|x| x.1 == label).map(|x| x.0.clone()).unwrap_or_default(),
            PrefsMsg::Rate(r) => d.draft.sample_rate = r,
            PrefsMsg::Midi(label) => d.draft.midi_input = d.midi_ports.iter().find(|x| x.1 == label).map(|x| x.0.clone()).unwrap_or_default(),
            PrefsMsg::Buffer(b) => d.draft.buffer = b,
            PrefsMsg::Paths(t) => d.paths = t,
            PrefsMsg::DefaultPaths(on) => d.draft.default_plugin_paths = on,
            PrefsMsg::Cancel => {
                theme::set(&self.prefs.theme);
                self.prefs_dialog = None;
            }
            PrefsMsg::Apply => {
                let mut next = d.draft.clone();
                next.plugin_paths = d.paths.split(';').map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect();
                let next = next.sane();
                if !cfg!(test) {
                    if let Err(e) = next.save() {
                        d.error = Some(format!("Could not save the preferences: {e}"));
                        return;
                    }
                }
                self.prefs_dialog = None;
                theme::set(&next.theme);
                if self.prefs.audio_differs(&next) {
                    // A new engine at the new rate: plugins come back with their state.
                    self.playback.stop();
                    self.playback.pool.store_states(&mut self.editor.project);
                    let live = LIVE_AUDIO.load(std::sync::atomic::Ordering::Relaxed);
                    let plugins = std::mem::take(&mut self.playback.plugins);
                    self.playback = playback::Playback::new(live, &next);
                    self.playback.plugins = plugins;
                    self.playback.sync(&self.editor.project, self.editor.music_revision, self.editor.revision, self.path.as_deref().and_then(|p| p.parent()));
                }
                if next.midi_input != self.prefs.midi_input || (self.midi.is_none() && !next.midi_input.is_empty()) {
                    self.open_midi(&next.midi_input);
                }
                let paths = next.plugin_search_paths();
                if paths != self.playback.search_paths {
                    self.playback.search_paths = paths;
                    self.playback.scan();
                }
                self.editor.status = match &self.playback.problem {
                    Some(p) => format!("Preferences saved · {p}"),
                    None => format!("Preferences saved · {} Hz, {} frames", next.sample_rate, next.buffer),
                };
                self.prefs = next;
            }
        }
    }
}
