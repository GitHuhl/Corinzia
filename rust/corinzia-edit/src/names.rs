//! Names and small lookups shared by the editor and the app: pitch and
//! interval names, spelling a MIDI key, dynamics typed as text, and the
//! clef and key in force at a position.

use corinzia_model::*;
use crate::key_alter;
use corinzia_score::Frac;

/// A MIDI key spelled in a key: the scale's own note where the key has
/// one, else a sharp in sharp keys (and C major), a flat in flat keys.
pub fn spell_key(key: i32, fifths: i32) -> Pitch {
    const SEMIS: [i32; 7] = [0, 2, 4, 5, 7, 9, 11];
    let pc = key.rem_euclid(12);
    let octave = key.div_euclid(12) - 1;
    let pitch = |step: i32, alter: i32| {
        // B# and Cb cross the octave line.
        let o = octave + if SEMIS[step as usize] + alter >= 12 { -1 } else if SEMIS[step as usize] + alter < 0 { 1 } else { 0 };
        Pitch { step: step as i8, alter: alter as i8, octave: o as i8 }
    };
    for step in 0..7 {
        let alter = key_alter(fifths, step);
        if (SEMIS[step as usize] + alter).rem_euclid(12) == pc {
            return pitch(step, alter);
        }
    }
    // Outside the scale: the key's own accidental, else natural, else the other.
    let first = if fifths < 0 { -1 } else { 1 };
    for alter in [first, 0, -first] {
        if let Some(step) = (0..7).find(|&s| (SEMIS[s as usize] + alter).rem_euclid(12) == pc) {
            return pitch(step, alter);
        }
    }
    pitch(0, 0)
}

/// The key signature (fifths) in force at a position.
pub fn key_fifths_at(m: &Movement, pos: Frac) -> i32 {
    m.keys.iter().filter(|k| k.at <= pos).last().map(|k| k.fifths).unwrap_or(0)
}

/// "up a major 2nd", "down a perfect 5th".
pub fn interval_name(i: Interval) -> String {
    let (dir, i) = if i.semitones < 0 || (i.semitones == 0 && i.steps < 0) { ("down", i.inverse()) } else { ("up", i) };
    const BASE: [i32; 7] = [0, 2, 4, 5, 7, 9, 11];
    let class = i.steps.rem_euclid(7);
    let diff = i.semitones - BASE[class as usize] - 12 * i.steps.div_euclid(7);
    let perfect = matches!(class, 0 | 3 | 4);
    let quality = match (perfect, diff) {
        (true, 0) => "perfect",
        (false, 0) => "major",
        (false, -1) => "minor",
        (_, d) if d > 0 => "augmented",
        _ => "diminished",
    };
    let n = i.steps + 1;
    let suffix = match (n % 10, n % 100) {
        (1, x) if x != 11 => "st",
        (2, x) if x != 12 => "nd",
        (3, x) if x != 13 => "rd",
        _ => "th",
    };
    let article = if quality.starts_with(['a', 'e', 'i', 'o', 'u']) { "an" } else { "a" };
    match (quality, n) {
        ("perfect", 8) => format!("{dir} an octave"),
        _ => format!("{dir} {article} {quality} {n}{suffix}"),
    }
}

/// A dynamic typed into its popover: `None` (empty) clears.
pub fn parse_dynamic(text: &str) -> Result<Option<u8>, String> {
    let t = text.trim().to_lowercase();
    if t.is_empty() {
        return Ok(None);
    }
    dynamic_index(&t).map(Some).ok_or_else(|| format!("Dynamic not understood: {}", DYNAMICS.join(" ")))
}

/// "C♯4", "B♭3".
pub fn pitch_name(p: Pitch) -> String {
    const NAMES: [&str; 7] = ["C", "D", "E", "F", "G", "A", "B"];
    let acc = match p.alter {
        -2 => "𝄫",
        -1 => "♭",
        1 => "♯",
        2 => "𝄪",
        _ => "",
    };
    format!("{}{}{}", NAMES[p.step.clamp(0, 6) as usize], acc, p.octave)
}

/// The diatonic number (C4 = 28) of a staff's top line for a clef.
pub fn top_line(clef: &str) -> i32 {
    match clef {
        "bass" => 26,      // A3
        "alto" => 32,      // G4
        "tenor" => 30,     // E4
        "treble_8" => 31,  // F4
        "bass_8" => 19,    // A2
        _ => 38,           // treble, percussion: F5
    }
}

/// The clef in force on a staff at a position: the last clef change, else
/// the instrument's own clef.
pub fn clef_at(project: &Project, m: &Movement, lane: usize, at: Frac) -> String {
    let l = &m.lanes[lane];
    if let Some(c) = l.clefs.iter().filter(|c| c.at <= at).last() {
        return c.clef.clone();
    }
    l.instrument_at(at).and_then(|i| project.instrument(i)).map(|i| i.clef.clone()).unwrap_or_else(|| "treble".into())
}
