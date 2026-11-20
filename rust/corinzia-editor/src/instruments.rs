//! Instruments from what is typed ("Flute, 2 Violin, Cello"), through the
//! catalogue (`corinzia_model::catalog`), and new projects built from them.

use corinzia_model::*;
use corinzia_score::Frac;
use std::sync::Arc;

/// The transposition a name not in the catalogue's aliases borrows: its
/// kind's, if the name starts with the kind's own ("Clarinet in B♭ (Bb)"),
/// else none ("Clarinet in E♭" is not a B♭ clarinet).
pub fn transposition_of(name: &str) -> Interval {
    corinzia_model::catalog::infer(name).filter(|k| name.starts_with(k.name)).map(|k| k.transposition).unwrap_or_default()
}

/// A comma-separated instrument list ("Flute, 2 Violin, Cello") ->
/// (name, abbreviation, clef) per staff. A leading count repeats an
/// instrument, numbered. A name not in the list is taken as written, with
/// a clef guessed from it.
pub fn parse_instruments(text: &str) -> Result<Vec<(String, String, String)>, String> {
    let mut out = Vec::new();
    for item in text.split([',', ';', '\n']).map(str::trim).filter(|s| !s.is_empty()) {
        let (count, name) = match item.split_once(' ') {
            Some((n, rest)) if n.parse::<u32>().is_ok() => (n.parse::<u32>().unwrap().clamp(1, 16), rest.trim()),
            _ => (1, item),
        };
        match corinzia_model::catalog::lookup(name).map(|k| (k.name, k.abbr, k.clef)) {
            Some((full, abbr, clef)) => {
                for i in 1..=count {
                    let suffix = if count > 1 { format!(" {i}") } else { String::new() };
                    out.push((format!("{full}{suffix}"), format!("{abbr}{suffix}"), clef.to_string()));
                }
            }
            None => {
                // As typed, capitalised; abbreviated to its first syllable.
                let mut full: String = name.chars().take(40).collect();
                if let Some(c) = full.get(..1) {
                    full = c.to_uppercase() + &full[1..];
                }
                let abbr = format!("{}.", full.chars().take(3).collect::<String>());
                for i in 1..=count {
                    let suffix = if count > 1 { format!(" {i}") } else { String::new() };
                    out.push((format!("{full}{suffix}"), format!("{abbr}{suffix}"), corinzia_model::catalog::guess_clef(name).to_string()));
                }
            }
        }
    }
    if out.is_empty() {
        return Err("Name at least one instrument, e.g. \"Flute, Violin, Cello\"".into());
    }
    Ok(out)
}

/// One instrument by name: from the known list (a trailing number is kept,
/// "Violin 2"), or as typed with a clef guessed from it. "Name / Abbr."
/// sets the short name later systems show.
pub fn instrument_spec(name: &str) -> Option<crate::InstrumentSpec> {
    if let Some((name, abbr)) = name.split_once('/') {
        let mut spec = instrument_spec(name)?;
        if !abbr.trim().is_empty() {
            spec.abbr = abbr.trim().to_string();
        }
        return Some(spec);
    }
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    let (base, number) = match name.rsplit_once(' ') {
        Some((b, n)) if n.chars().all(|c| c.is_ascii_digit()) && !b.trim().is_empty() => (b.trim(), Some(n)),
        _ => (name, None),
    };
    let suffix = number.map(|n| format!(" {n}")).unwrap_or_default();
    let known = corinzia_model::catalog::lookup(base);
    let (full, abbr, clef) = match known {
        Some(k) => (format!("{}{suffix}", k.name), format!("{}{suffix}", k.abbr), k.clef.to_string()),
        None => {
            let full: String = name.chars().take(40).collect();
            let full = match full.get(..1) {
                Some(c) => c.to_uppercase() + &full[1..],
                None => full,
            };
            let abbr = format!("{}.", full.chars().take(3).collect::<String>());
            (full, abbr, corinzia_model::catalog::guess_clef(base).to_string())
        }
    };
    let transposition = transposition_of(&full);
    // What it is: the kind typed, else the kind its words suggest (None: a
    // name the catalogue does not know, which only renames an instrument).
    let kind = known.or_else(|| corinzia_model::catalog::infer(name));
    Some(crate::InstrumentSpec { name: full, abbr, clef, transposition, kind, section: corinzia_model::catalog::section_of(name) })
}

/// A player's instruments, comma separated: the first one, then doublings.
pub fn parse_instrument_list(text: &str) -> Result<Vec<crate::InstrumentSpec>, String> {
    let list: Vec<_> = text.split([',', ';']).filter_map(instrument_spec).collect();
    if list.is_empty() {
        return Err("Each player needs an instrument".into());
    }
    Ok(list)
}

pub fn new_project(title: &str, instruments: &str, bars: i64) -> Result<Project, String> {
    let list = parse_instruments(instruments)?;
    let refs: Vec<(&str, &str, &str, Interval)> = list.iter().map(|(a, b, c)| (a.as_str(), b.as_str(), c.as_str(), transposition_of(a))).collect();
    let mut p = blank_with(title, &refs, bars);
    p.phrasing = true; // new scores play phrased; older ones as they were
    Ok(p)
}

/// A new, empty project: these instruments (name, abbreviation, clef), each
/// on its own player, over `bars` bars of 4/4 filled with whole-bar rests.
pub fn blank(title: &str, instruments: &[(&str, &str, &str)], bars: i64) -> Project {
    let with: Vec<(&str, &str, &str, Interval)> = instruments.iter().map(|(a, b, c)| (*a, *b, *c, Interval::default())).collect();
    blank_with(title, &with, bars)
}

/// As [`blank`], with each instrument's transposition.
pub fn blank_with(title: &str, instruments: &[(&str, &str, &str, Interval)], bars: i64) -> Project {
    let mut p = Project { title: title.into(), sample_rate: 48000.0, next_id: 1, ..Default::default() };
    let mut lanes = Vec::new();
    for (k, (name, abbr, clefs, transposition)) in instruments.iter().enumerate() {
        // "treble+bass": a grand staff, one staff per clef.
        let clefs: Vec<&str> = clefs.split('+').collect();
        let (track, player, inst) = (p.allocate_id(), p.allocate_id(), p.allocate_id());
        // A drum set starts with its map, and plays on the drum channel (10).
        let (kind, section) = Instrument::kind_from_name(name);
        let drums = if clefs[0] == "percussion" { kind.map(|k| k.default_drums()).unwrap_or_default() } else { vec![] };
        // Channel 10 is the drums' (General MIDI): others skip it.
        let midi_channel = if !drums.is_empty() { 9 } else { [0, 1, 2, 3, 4, 5, 6, 7, 8, 10, 11, 12, 13, 14, 15][k % 15] };
        p.tracks.push(Track { id: track, name: name.to_string(), instrument_node: None, note_port: 0, output_bus: None, midi_channel, gain_db: 0.0, pan: 0.0, mute: false, solo: false, inserts: vec![], sends: vec![], dynamics_cc: None, program: None, bank: None, techniques: vec![], color: None, folder: String::new(), audio: false, clips: vec![] });
        p.instruments.push(Instrument { id: inst, player, name: name.to_string(), engraver_name: abbr.to_string(), transposition: *transposition, clef: clefs[0].to_string(), track: Some(track), drums, tuning: kind.map(|k| k.default_tuning()).unwrap_or_default(), kind, section });
        p.players.push(Player { id: player, name: name.to_string(), instruments: vec![inst] });
        for (si, clef) in clefs.iter().enumerate() {
            let rests: Vec<Event> = (0..bars)
                .map(|b| Event { id: p.allocate_id(), kind: EventKind::Rest, start: Frac::int(b), duration: Frac::int(1), heads: vec![], articulations: 0, dynamic: None, spans: 0, lyrics: vec![], graces: vec![], fingering: vec![] })
                .collect();
            let (change, voice) = (p.allocate_id(), p.allocate_id());
            let clef_changes = if si == 0 { vec![] } else { vec![ClefChange { id: p.allocate_id(), at: Frac::ZERO, clef: clef.to_string() }] };
            lanes.push(Lane {
                player,
                changes: vec![InstrumentChange { id: change, at: Frac::ZERO, instrument: inst }],
                clefs: clef_changes,
                texts: vec![],
                chords: vec![],
                lines: vec![],
                voices: vec![Voice { id: voice, index: 1, events: Arc::new(rests), tuplets: vec![] }],
            });
        }
    }
    let id = p.allocate_id();
    p.movements.push(Movement {
        id,
        ordinal: 0,
        title: String::new(),
        length: Frac::int(bars.max(1)),
        revision: 0,
        tempo: vec![Tempo { at: Frac::ZERO, bpm: 120.0, ramp: false, text: String::new(), hidden: false }],
        meter: vec![Meter { at: Frac::ZERO, num: 4, den: 4, pickup: false, beams: [0; 8] }],
        keys: vec![Key { at: Frac::ZERO, fifths: 0, minor: false }],
        barlines: vec![],
        rehearsal: vec![],
        navigation: vec![],
        slurs: vec![],
        endings: vec![],
        lanes, breaks: vec![], cross: vec![], automation: vec![], tweaks: vec![],
    });
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instrument_lists() {
        let l = parse_instruments("Flute, 2 violin; Cello").unwrap();
        let names: Vec<_> = l.iter().map(|x| x.0.as_str()).collect();
        assert_eq!(names, ["Flute", "Violin 1", "Violin 2", "Cello"]);
        assert_eq!(l[3].2, "bass");
        // Any other name is taken as typed, with a clef from the name.
        let k = parse_instruments("Flute, kazoo, bass ocarina").unwrap();
        assert_eq!((k[1].0.as_str(), k[1].2.as_str()), ("Kazoo", "treble"));
        assert_eq!(k[2].2, "bass");
        assert_eq!(parse_instruments("bass guitar").unwrap()[0], ("Bass guitar".into(), "B. Gtr.".into(), "bass_8".into()));
        assert!(parse_instruments(" , ").is_err());
        let p = new_project("Quartet", "violin, violin, viola, cello", 4).unwrap();
        assert_eq!(p.movements[0].lanes.len(), 4);
        assert_eq!(p.instruments[2].clef, "alto");
        // A piano is a grand staff: two staves of one player, bass below.
        let p = new_project("Duo", "Flute, Piano", 4).unwrap();
        let m = &p.movements[0];
        assert_eq!(m.lanes.len(), 3);
        assert_eq!(p.players.len(), 2);
        assert_eq!(m.lanes[1].player, m.lanes[2].player);
        assert_eq!(m.lanes[2].clefs[0].clef, "bass");
        let s = corinzia_model::notation::score(&p, m);
        assert!(!s.parts[0].brace && s.parts[1].brace && !s.parts[2].brace);
    }
}
