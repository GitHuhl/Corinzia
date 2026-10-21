//! Plugin presets: a plugin's state saved under a name, kept per plugin in
//! the configuration folder (`presets/<plugin id>/<name>.preset`), for any
//! project. And the General MIDI program names, for tracks' programs.

use std::path::{Path, PathBuf};

fn safe(s: &str) -> String {
    s.chars().map(|c| if c.is_alphanumeric() || " .-_()".contains(c) { c } else { '_' }).collect::<String>().trim().to_string()
}

/// The folder of a plugin's presets under `base`.
pub fn folder(base: &Path, plugin_id: &str) -> PathBuf {
    base.join("presets").join(safe(plugin_id))
}

/// The base folder (the configuration folder).
pub fn base() -> Option<PathBuf> {
    crate::prefs::folder()
}

/// A plugin's preset names, sorted.
pub fn list(base: &Path, plugin_id: &str) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(folder(base, plugin_id))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            if p.extension()? != "preset" {
                return None;
            }
            Some(p.file_stem()?.to_string_lossy().into_owned())
        })
        .collect();
    v.sort_by_key(|s| s.to_lowercase());
    v
}

pub fn save(base: &Path, plugin_id: &str, name: &str, state: &[u8]) -> Result<String, String> {
    let name = safe(name);
    if name.is_empty() {
        return Err("Name the preset first".into());
    }
    let dir = folder(base, plugin_id);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    std::fs::write(dir.join(format!("{name}.preset")), state).map_err(|e| e.to_string())?;
    Ok(name)
}

pub fn load(base: &Path, plugin_id: &str, name: &str) -> Result<Vec<u8>, String> {
    std::fs::read(folder(base, plugin_id).join(format!("{}.preset", safe(name)))).map_err(|e| format!("{name}: {e}"))
}

pub fn delete(base: &Path, plugin_id: &str, name: &str) -> Result<(), String> {
    std::fs::remove_file(folder(base, plugin_id).join(format!("{}.preset", safe(name)))).map_err(|e| e.to_string())
}

/// General MIDI program names, 0-127.
pub const GM: [&str; 128] = [
    "Acoustic Grand Piano", "Bright Acoustic Piano", "Electric Grand Piano", "Honky-tonk Piano", "Electric Piano 1", "Electric Piano 2", "Harpsichord", "Clavinet",
    "Celesta", "Glockenspiel", "Music Box", "Vibraphone", "Marimba", "Xylophone", "Tubular Bells", "Dulcimer",
    "Drawbar Organ", "Percussive Organ", "Rock Organ", "Church Organ", "Reed Organ", "Accordion", "Harmonica", "Tango Accordion",
    "Acoustic Guitar (nylon)", "Acoustic Guitar (steel)", "Electric Guitar (jazz)", "Electric Guitar (clean)", "Electric Guitar (muted)", "Overdriven Guitar", "Distortion Guitar", "Guitar Harmonics",
    "Acoustic Bass", "Electric Bass (finger)", "Electric Bass (pick)", "Fretless Bass", "Slap Bass 1", "Slap Bass 2", "Synth Bass 1", "Synth Bass 2",
    "Violin", "Viola", "Cello", "Contrabass", "Tremolo Strings", "Pizzicato Strings", "Orchestral Harp", "Timpani",
    "String Ensemble 1", "String Ensemble 2", "Synth Strings 1", "Synth Strings 2", "Choir Aahs", "Voice Oohs", "Synth Voice", "Orchestra Hit",
    "Trumpet", "Trombone", "Tuba", "Muted Trumpet", "French Horn", "Brass Section", "Synth Brass 1", "Synth Brass 2",
    "Soprano Sax", "Alto Sax", "Tenor Sax", "Baritone Sax", "Oboe", "English Horn", "Bassoon", "Clarinet",
    "Piccolo", "Flute", "Recorder", "Pan Flute", "Blown Bottle", "Shakuhachi", "Whistle", "Ocarina",
    "Lead 1 (square)", "Lead 2 (sawtooth)", "Lead 3 (calliope)", "Lead 4 (chiff)", "Lead 5 (charang)", "Lead 6 (voice)", "Lead 7 (fifths)", "Lead 8 (bass + lead)",
    "Pad 1 (new age)", "Pad 2 (warm)", "Pad 3 (polysynth)", "Pad 4 (choir)", "Pad 5 (bowed)", "Pad 6 (metallic)", "Pad 7 (halo)", "Pad 8 (sweep)",
    "FX 1 (rain)", "FX 2 (soundtrack)", "FX 3 (crystal)", "FX 4 (atmosphere)", "FX 5 (brightness)", "FX 6 (goblins)", "FX 7 (echoes)", "FX 8 (sci-fi)",
    "Sitar", "Banjo", "Shamisen", "Koto", "Kalimba", "Bagpipe", "Fiddle", "Shanai",
    "Tinkle Bell", "Agogo", "Steel Drums", "Woodblock", "Taiko Drum", "Melodic Tom", "Synth Drum", "Reverse Cymbal",
    "Guitar Fret Noise", "Breath Noise", "Seashore", "Bird Tweet", "Telephone Ring", "Helicopter", "Applause", "Gunshot",
];

/// "41 Violin" for program 40; the inverse of [`program_of`].
pub fn program_label(p: u8) -> String {
    format!("{} {}", p as u32 + 1, GM[(p & 0x7f) as usize])
}

pub fn program_of(label: &str) -> Option<u8> {
    let n: u32 = label.split_whitespace().next()?.parse().ok()?;
    (1..=128).contains(&n).then(|| (n - 1) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_list_load_delete() {
        let base = std::env::temp_dir().join(format!("corinzia-presets-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        assert_eq!(save(&base, "org.corinzia.reverb", "Big hall", b"CZF1abc").unwrap(), "Big hall");
        save(&base, "org.corinzia.reverb", "a/b", b"x").unwrap();
        assert_eq!(list(&base, "org.corinzia.reverb"), ["a_b", "Big hall"]);
        assert_eq!(load(&base, "org.corinzia.reverb", "Big hall").unwrap(), b"CZF1abc");
        delete(&base, "org.corinzia.reverb", "a_b").unwrap();
        assert_eq!(list(&base, "org.corinzia.reverb").len(), 1);
        assert!(save(&base, "x", "  ", b"").is_err());
        assert_eq!((program_label(40).as_str(), program_of("41 Violin")), ("41 Violin", Some(40)));
        std::fs::remove_dir_all(&base).ok();
    }
}
