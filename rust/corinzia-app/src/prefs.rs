//! Preferences: the look, the audio output and where plugins are looked
//! for. Kept as JSON in the user's configuration folder
//! (`$XDG_CONFIG_HOME/corinzia`, `~/.config/corinzia`, or `%APPDATA%\Corinzia`).

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    /// A name from `theme::THEMES`.
    pub theme: String,
    /// The output device's id; empty for the system default.
    pub output_device: String,
    pub sample_rate: u32,
    /// Frames the engine renders at a time (the most a backend asks for).
    pub buffer: u32,
    /// Folders searched for plugins besides the usual ones.
    pub plugin_paths: Vec<String>,
    /// Also search the usual folders (and CLAP_PATH / VST3_PATH).
    pub default_plugin_paths: bool,
    /// The MIDI input port for note entry; empty for none.
    pub midi_input: String,
    /// The sizes of the window's panes, as last dragged.
    pub layout: Layout,
    /// The metronome while playing, and the bars of clicks counted in
    /// before playback starts (0..=2).
    pub metronome: bool,
    pub count_in: u8,
    /// With a passage selected, Play sounds only its staves, from its start
    /// to its end (as other score editors do); off: everything from there on.
    pub play_selection_only: bool,
    /// Scores opened or saved lately, the latest first.
    pub recent: Vec<String>,
}

/// The Mix tab's panes: the mixer's height, the side panel's width, the
/// arrange view's track height and header width (pixels).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Layout {
    pub mixer_height: f32,
    pub side_width: f32,
    pub lane_height: f32,
    pub header_width: f32,
}

impl Default for Layout {
    fn default() -> Self {
        Layout { mixer_height: 420.0, side_width: 300.0, lane_height: 56.0, header_width: crate::arrange::HEADER }
    }
}

impl Layout {
    pub const MIXER: (f32, f32) = (140.0, 1200.0);
    pub const SIDE: (f32, f32) = (200.0, 700.0);
    pub fn sane(mut self) -> Layout {
        self.mixer_height = self.mixer_height.clamp(Self::MIXER.0, Self::MIXER.1);
        self.side_width = self.side_width.clamp(Self::SIDE.0, Self::SIDE.1);
        self.lane_height = self.lane_height.clamp(crate::arrange::LANE_HEIGHTS.0, crate::arrange::LANE_HEIGHTS.1);
        self.header_width = self.header_width.clamp(crate::arrange::HEADER_WIDTHS.0, crate::arrange::HEADER_WIDTHS.1);
        self
    }
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs { theme: "Dark".into(), output_device: String::new(), sample_rate: 48000, buffer: 2048, plugin_paths: vec![], default_plugin_paths: true, midi_input: String::new(), layout: Layout::default(), metronome: false, count_in: 0, play_selection_only: true, recent: vec![] }
    }
}

pub const SAMPLE_RATES: [u32; 4] = [44100, 48000, 88200, 96000];
pub const BUFFERS: [u32; 6] = [128, 256, 512, 1024, 2048, 4096];

/// The configuration folder.
pub fn folder() -> Option<PathBuf> {
    if cfg!(windows) {
        return std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("Corinzia"));
    }
    std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config"))).map(|d| d.join("corinzia"))
}

fn file() -> Option<PathBuf> {
    folder().map(|d| d.join("preferences.json"))
}

impl Prefs {
    /// The saved preferences, or the defaults (a broken file is ignored).
    pub fn load() -> Prefs {
        file().and_then(|f| std::fs::read_to_string(f).ok()).and_then(|t| serde_json::from_str::<Prefs>(&t).ok()).map(Prefs::sane).unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), String> {
        let f = file().ok_or("no configuration folder")?;
        if let Some(d) = f.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        std::fs::write(&f, serde_json::to_string_pretty(self).map_err(|e| e.to_string())?).map_err(|e| format!("{}: {e}", f.display()))
    }

    /// Values in range.
    pub fn sane(mut self) -> Prefs {
        self.sample_rate = self.sample_rate.clamp(22050, 192000);
        self.buffer = self.buffer.clamp(64, 8192);
        self.layout = self.layout.sane();
        self.count_in = self.count_in.min(2);
        self
    }

    /// A score opened or saved: first in the recent list (at most eight).
    pub fn remember(&mut self, path: &std::path::Path) {
        let p = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()).to_string_lossy().into_owned();
        self.recent.retain(|r| *r != p);
        self.recent.insert(0, p);
        self.recent.truncate(8);
    }

    /// Where plugins are looked for.
    pub fn plugin_search_paths(&self) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = self.plugin_paths.iter().filter(|p| !p.trim().is_empty()).map(|p| PathBuf::from(p.trim())).collect();
        if self.default_plugin_paths {
            v.extend(corinzia_audio::host::search_paths());
        }
        let mut seen = std::collections::HashSet::new();
        v.retain(|p| seen.insert(p.clone()));
        v
    }

    /// Whether the audio output must be restarted to apply `other`.
    pub fn audio_differs(&self, other: &Prefs) -> bool {
        (self.output_device.as_str(), self.sample_rate, self.buffer) != (other.output_device.as_str(), other.sample_rate, other.buffer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_defaults() {
        let p = Prefs { theme: "Light".into(), sample_rate: 96000, buffer: 256, plugin_paths: vec!["/opt/clap".into()], ..Prefs::default() };
        let back: Prefs = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!(back, p);
        // Missing fields take their defaults; values out of range are clamped.
        let q: Prefs = serde_json::from_str(r#"{"buffer": 3}"#).unwrap();
        assert_eq!((q.sane().buffer, Prefs::default().sample_rate), (64, 48000));
        assert!(p.plugin_search_paths()[0].ends_with("opt/clap"));
        assert!(p.audio_differs(&Prefs::default()));
    }
}
