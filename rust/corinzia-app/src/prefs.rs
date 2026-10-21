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
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs { theme: "Dark".into(), output_device: String::new(), sample_rate: 48000, buffer: 2048, plugin_paths: vec![], default_plugin_paths: true, midi_input: String::new() }
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
        self
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
