//! MusicXML import: a `score-partwise` file (plain `.musicxml`/`.xml`, or
//! compressed `.mxl`) becomes a Corinzia [`Project`] with one movement.
//!
//! What comes across:
//! * parts as players, instruments and tracks; a part's staves (a piano's
//!   two) as one staff each;
//! * notes, rests, chords, ties and up to four voices per staff;
//! * keys, time signatures, clefs (and clef changes), tempo marks;
//! * dynamics, hairpins, slurs, articulations, fermatas, ornaments,
//!   arpeggios, breath marks;
//! * lyrics (verses, hyphens, extenders), chord symbols, expression text;
//! * barlines, repeats, voltas and rehearsal marks.
//!
//! A pickup bar is padded with rests at the front so the bars line up.
//! Grace and cue notes are skipped.

use corinzia_model::{
    BarKind, Barline, ChordSymbol, ClefChange, Ending, Event, EventKind, Head, Id, Instrument, InstrumentChange, Key, Lane, Meter, Movement, Pitch, Player, Project,
    RehearsalMark, Tempo, TextItem, TextKind, Track, Voice, DYNAMICS,
};
use corinzia_score::score::{art, span, Lyric};
use corinzia_score::Frac;
use roxmltree::{Document, Node};
use std::collections::BTreeMap;
use std::sync::Arc;

/// Reads MusicXML from a file's bytes: compressed (.mxl) or plain.
pub fn musicxml(bytes: &[u8]) -> Result<Project, String> {
    let xml = if bytes.starts_with(b"PK") { unzip_mxl(bytes)? } else { decode_text(bytes)? };
    musicxml_str(&xml)
}

/// The score inside a compressed MusicXML file: the root file named by
/// META-INF/container.xml, else the first .xml/.musicxml entry.
fn unzip_mxl(bytes: &[u8]) -> Result<String, String> {
    use std::io::Read;
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| format!("not a MusicXML archive: {e}"))?;
    let mut read = |name: &str| -> Option<Vec<u8>> {
        let mut f = zip.by_name(name).ok()?;
        let mut v = Vec::new();
        f.read_to_end(&mut v).ok()?;
        Some(v)
    };
    let root = read("META-INF/container.xml").and_then(|c| {
        let c = decode_text(&c).ok()?;
        let doc = Document::parse(&c).ok()?;
        doc.descendants().find(|n| n.has_tag_name("rootfile")).and_then(|n| n.attribute("full-path")).map(String::from)
    });
    let name = match root {
        Some(r) => r,
        None => {
            let names: Vec<String> = zip.file_names().map(String::from).collect();
            names.into_iter().find(|n| !n.starts_with("META-INF") && (n.ends_with(".xml") || n.ends_with(".musicxml"))).ok_or("no score in the archive")?
        }
    };
    let mut f = zip.by_name(&name).map_err(|e| format!("{name}: {e}"))?;
    let mut v = Vec::new();
    f.read_to_end(&mut v).map_err(|e| e.to_string())?;
    decode_text(&v)
}

/// UTF-8 (with or without a byte-order mark) or UTF-16.
fn decode_text(b: &[u8]) -> Result<String, String> {
    if let Some(rest) = b.strip_prefix(&[0xef, 0xbb, 0xbf]) {
        return String::from_utf8(rest.to_vec()).map_err(|e| e.to_string());
    }
    let utf16 = |le: bool, b: &[u8]| -> Result<String, String> {
        let units: Vec<u16> = b.chunks_exact(2).map(|c| if le { u16::from_le_bytes([c[0], c[1]]) } else { u16::from_be_bytes([c[0], c[1]]) }).collect();
        String::from_utf16(&units).map_err(|e| e.to_string())
    };
    match b {
        [0xff, 0xfe, rest @ ..] => utf16(true, rest),
        [0xfe, 0xff, rest @ ..] => utf16(false, rest),
        _ => String::from_utf8(b.to_vec()).map_err(|e| format!("not UTF-8 text: {e}")),
    }
}

fn child<'a, 'i>(n: Node<'a, 'i>, name: &str) -> Option<Node<'a, 'i>> {
    n.children().find(|c| c.has_tag_name(name))
}
fn children<'a, 'i: 'a>(n: Node<'a, 'i>, name: &'a str) -> impl Iterator<Item = Node<'a, 'i>> + 'a {
    n.children().filter(move |c| c.has_tag_name(name))
}
fn text_of(n: Node, name: &str) -> Option<String> {
    child(n, name).and_then(|c| c.text()).map(|t| t.trim().to_string())
}
fn num<T: std::str::FromStr>(n: Node, name: &str) -> Option<T> {
    text_of(n, name).and_then(|t| t.parse().ok())
}
/// A number that may be written as a decimal ("2.0", "-1").
fn int(n: Node, name: &str) -> Option<i64> {
    text_of(n, name).and_then(|t| t.parse::<f64>().ok()).map(|v| v.round() as i64)
}

/// Clef sign/line/octave to Corinzia's clef names.
fn clef_name(c: Node) -> String {
    let sign = text_of(c, "sign").unwrap_or_default();
    let line: i64 = int(c, "line").unwrap_or(0);
    let octave: i64 = int(c, "clef-octave-change").unwrap_or(0);
    match (sign.as_str(), line, octave) {
        ("G", _, -1) => "treble_8",
        ("F", _, -1) => "bass_8",
        ("F", _, _) => "bass",
        ("C", 4, _) => "tenor",
        ("C", _, _) => "alto",
        ("percussion", _, _) => "percussion",
        _ => "treble",
    }
    .to_string()
}

/// A chord symbol as Corinzia writes them ("F#m7b5/E").
fn harmony_text(h: Node) -> Option<String> {
    let root = child(h, "root")?;
    let step = text_of(root, "root-step")?;
    let acc = |alter: Option<i64>| match alter {
        Some(a) if a > 0 => "#".repeat(a as usize),
        Some(a) if a < 0 => "b".repeat((-a) as usize),
        _ => String::new(),
    };
    let mut s = format!("{step}{}", acc(int(root, "root-alter")));
    let kind = text_of(h, "kind").unwrap_or_else(|| "major".into());
    s.push_str(match kind.as_str() {
        "major" => "",
        "minor" => "m",
        "augmented" => "+",
        "diminished" => "dim",
        "dominant" => "7",
        "major-seventh" => "maj7",
        "minor-seventh" => "m7",
        "diminished-seventh" => "dim7",
        "augmented-seventh" => "+7",
        "half-diminished" => "m7b5",
        "major-minor" => "m(maj7)",
        "major-sixth" => "6",
        "minor-sixth" => "m6",
        "dominant-ninth" => "9",
        "major-ninth" => "maj9",
        "minor-ninth" => "m9",
        "dominant-11th" => "11",
        "major-11th" => "maj11",
        "minor-11th" => "m11",
        "dominant-13th" => "13",
        "major-13th" => "maj13",
        "minor-13th" => "m13",
        "suspended-second" => "sus2",
        "suspended-fourth" => "sus4",
        "power" => "5",
        _ => "",
    });
    if let Some(b) = child(h, "bass") {
        if let Some(bs) = text_of(b, "bass-step") {
            s.push_str(&format!("/{bs}{}", acc(int(b, "bass-alter"))));
        }
    }
    Some(s)
}

/// One staff's music while reading: events per MusicXML voice.
#[derive(Default)]
struct StaffMusic {
    clef: Option<String>,
    clefs: Vec<(Frac, String)>,
    texts: Vec<(Frac, String)>,
    chords: Vec<(Frac, String)>,
    /// MusicXML voice name -> events (in time order).
    voices: BTreeMap<String, Vec<Event>>,
    /// Dynamics and hairpins waiting for the next event at or after a time.
    dynamics: Vec<(Frac, u8)>,
    wedges: Vec<(Frac, u8)>,
}

struct PartInfo {
    name: String,
    abbr: String,
}

/// Parses a `score-partwise` document.
pub fn musicxml_str(xml: &str) -> Result<Project, String> {
    // MusicXML files declare their DTD; it is not needed to read them.
    let opts = roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() };
    let doc = Document::parse_with_options(xml, opts).map_err(|e| format!("not valid XML: {e}"))?;
    let root = doc.root_element();
    if root.has_tag_name("score-timewise") {
        return Err("timewise MusicXML is not supported (export it as partwise)".into());
    }
    if !root.has_tag_name("score-partwise") {
        return Err("not a MusicXML score".into());
    }
    let mut p = Project { sample_rate: 48000.0, next_id: 1, ..Default::default() };
    p.title = child(root, "work").and_then(|w| text_of(w, "work-title")).or_else(|| text_of(root, "movement-title")).unwrap_or_default();
    p.composer = child(root, "identification")
        .and_then(|i| children(i, "creator").find(|c| c.attribute("type") == Some("composer")))
        .and_then(|c| c.text())
        .map(|t| t.trim().to_string())
        .unwrap_or_default();
    let movement_title = text_of(root, "movement-title").filter(|t| Some(t) != Some(&p.title)).unwrap_or_default();

    // Part names.
    let mut info: BTreeMap<String, PartInfo> = BTreeMap::new();
    if let Some(list) = child(root, "part-list") {
        for sp in children(list, "score-part") {
            let id = sp.attribute("id").unwrap_or_default().to_string();
            let name = text_of(sp, "part-name").filter(|n| !n.is_empty()).unwrap_or_else(|| id.clone());
            let abbr = text_of(sp, "part-abbreviation").unwrap_or_default();
            info.insert(id, PartInfo { name, abbr });
        }
    }
    let parts: Vec<Node> = children(root, "part").collect();
    if parts.is_empty() {
        return Err("the score has no parts".into());
    }

    // Bar starts come from the first part: each bar is as long as its
    // longest voice (or its time signature when empty).
    let mut meter_len = Frac::int(1);
    let mut bar_starts: Vec<Frac> = Vec::new();
    let mut lead = Frac::ZERO; // rests in front of a pickup
    {
        let mut divisions: i64 = 1;
        let mut at = Frac::ZERO;
        for (mi, m) in children(parts[0], "measure").enumerate() {
            bar_starts.push(at);
            let mut cursor = Frac::ZERO;
            let mut longest = Frac::ZERO;
            let mut last_dur = Frac::ZERO;
            for e in m.children().filter(|c| c.is_element()) {
                match e.tag_name().name() {
                    "attributes" => {
                        if let Some(d) = int(e, "divisions") {
                            divisions = d.max(1);
                        }
                        if let Some(t) = child(e, "time") {
                            if let (Some(b), Some(bt)) = (beats(t), int(t, "beat-type")) {
                                meter_len = Frac::new(b, bt.max(1));
                            }
                        }
                    }
                    "note" => {
                        if child(e, "grace").is_some() || child(e, "cue").is_some() {
                            continue;
                        }
                        let d = Frac::new(int(e, "duration").unwrap_or(0), divisions * 4);
                        if child(e, "chord").is_some() {
                            cursor = cursor - last_dur;
                        }
                        cursor = cursor + d;
                        last_dur = d;
                    }
                    "backup" => cursor = cursor - Frac::new(int(e, "duration").unwrap_or(0), divisions * 4),
                    "forward" => cursor = cursor + Frac::new(int(e, "duration").unwrap_or(0), divisions * 4),
                    _ => {}
                }
                longest = longest.max(cursor);
            }
            let mut len = if longest > Frac::ZERO { longest } else { meter_len };
            if mi == 0 && len < meter_len {
                // A pickup: pad it to a full bar.
                lead = meter_len - len;
                len = meter_len;
            }
            at = at + len;
        }
        bar_starts.push(at);
    }
    let end = *bar_starts.last().unwrap();

    let mut movement = Movement {
        id: 0,
        ordinal: 0,
        title: movement_title,
        length: end.max(Frac::int(1)),
        revision: 0,
        tempo: vec![],
        meter: vec![],
        keys: vec![],
        barlines: vec![],
        rehearsal: vec![],
        endings: vec![],
        lanes: vec![],
    };

    for (pi, part) in parts.iter().enumerate() {
        let pid = part.attribute("id").unwrap_or_default();
        let first = pi == 0;
        let mut staves: Vec<StaffMusic> = vec![StaffMusic::default()];
        let mut divisions: i64 = 1;
        let mut open_endings: BTreeMap<String, Frac> = BTreeMap::new();
        for (mi, m) in children(*part, "measure").enumerate() {
            let Some(&bar) = bar_starts.get(mi) else { break };
            let bar_end = bar_starts.get(mi + 1).copied().unwrap_or(end);
            // The pickup's music sits at the end of the first bar.
            let offset = if mi == 0 { lead } else { Frac::ZERO };
            let mut cursor = Frac::ZERO;
            let mut last_start = Frac::ZERO;
            let q = |d: i64, div: i64| Frac::new(d, div * 4);
            for e in m.children().filter(|c| c.is_element()) {
                let at = bar + offset + cursor;
                match e.tag_name().name() {
                    "attributes" => {
                        if let Some(d) = int(e, "divisions") {
                            divisions = d.max(1);
                        }
                        if let Some(n) = int(e, "staves") {
                            while staves.len() < n.clamp(1, 8) as usize {
                                staves.push(StaffMusic::default());
                            }
                        }
                        if first {
                            if let Some(k) = child(e, "key") {
                                if let Some(f) = int(k, "fifths") {
                                    let minor = text_of(k, "mode").as_deref() == Some("minor");
                                    let at = if mi == 0 { Frac::ZERO } else { at };
                                    movement.keys.retain(|x| x.at != at);
                                    movement.keys.push(Key { at, fifths: f.clamp(-7, 7) as i32, minor });
                                }
                            }
                            if let Some(t) = child(e, "time") {
                                if let (Some(b), Some(bt)) = (beats(t), int(t, "beat-type")) {
                                    let at = if mi == 0 { Frac::ZERO } else { bar };
                                    movement.meter.retain(|x| x.at != at);
                                    movement.meter.push(Meter { at, num: b, den: bt.max(1) });
                                }
                            }
                        }
                        for c in children(e, "clef") {
                            let staff = c.attribute("number").and_then(|n| n.parse::<usize>().ok()).unwrap_or(1).max(1) - 1;
                            while staves.len() <= staff {
                                staves.push(StaffMusic::default());
                            }
                            let name = clef_name(c);
                            let s = &mut staves[staff];
                            if s.clef.is_none() && at <= Frac::ZERO + offset {
                                s.clef = Some(name);
                            } else if s.clef.as_deref() != Some(name.as_str()) || !s.clefs.is_empty() {
                                s.clefs.push((at, name));
                            }
                        }
                    }
                    "backup" => cursor = cursor - q(int(e, "duration").unwrap_or(0), divisions),
                    "forward" => cursor = cursor + q(int(e, "duration").unwrap_or(0), divisions),
                    "direction" => {
                        let off = int(e, "offset").map(|o| q(o, divisions)).unwrap_or(Frac::ZERO);
                        let at = (at + off).max(Frac::ZERO);
                        let staff = num::<usize>(e, "staff").unwrap_or(1).max(1) - 1;
                        while staves.len() <= staff {
                            staves.push(StaffMusic::default());
                        }
                        direction(e, at, first, &mut staves[staff], &mut movement, &mut p);
                    }
                    "harmony" => {
                        if let Some(t) = harmony_text(e) {
                            let staff = num::<usize>(e, "staff").unwrap_or(1).max(1) - 1;
                            while staves.len() <= staff {
                                staves.push(StaffMusic::default());
                            }
                            staves[staff].chords.push((at, t));
                        }
                    }
                    "barline" if first => barline(e, bar, bar_end, &mut open_endings, &mut movement, &mut p),
                    "note" => {
                        if child(e, "grace").is_some() || child(e, "cue").is_some() {
                            continue;
                        }
                        let dur = q(int(e, "duration").unwrap_or(0), divisions);
                        let chord = child(e, "chord").is_some();
                        let start = if chord { bar + offset + last_start } else { at };
                        if !chord {
                            last_start = cursor;
                            cursor = cursor + dur;
                        }
                        if dur <= Frac::ZERO {
                            continue;
                        }
                        let staff = num::<usize>(e, "staff").unwrap_or(1).max(1) - 1;
                        while staves.len() <= staff {
                            staves.push(StaffMusic::default());
                        }
                        let voice = text_of(e, "voice").unwrap_or_else(|| "1".into());
                        note(e, start, dur, chord, &mut staves[staff], voice, &mut p);
                    }
                    _ => {}
                }
            }
        }

        // The part becomes a player, an instrument and a track; each staff a lane.
        let pinfo = info.get(pid);
        let name = pinfo.map(|i| i.name.clone()).unwrap_or_else(|| format!("Part {}", pi + 1));
        let abbr = pinfo.map(|i| i.abbr.clone()).unwrap_or_default();
        let (track, player, inst) = (p.allocate_id(), p.allocate_id(), p.allocate_id());
        let clef0 = staves[0].clef.clone().unwrap_or_else(|| "treble".into());
        p.tracks.push(Track { id: track, name: name.clone(), instrument_node: None, note_port: 0, output_bus: None, midi_channel: (pi % 16) as u8, gain_db: 0.0, pan: 0.0, mute: false, solo: false });
        p.instruments.push(Instrument { id: inst, player, name: name.clone(), engraver_name: abbr, transposition: 0, clef: clef0, track: Some(track) });
        p.players.push(Player { id: player, name, instruments: vec![inst] });
        for mut staff in staves {
            // A staff starting in another clef than the instrument's (a
            // piano's left hand) gets it as a change at the start.
            let own = staff.clef.clone().unwrap_or_else(|| "treble".into());
            let inst_clef = p.instruments.last().map(|i| i.clef.clone()).unwrap_or_default();
            if own != inst_clef {
                staff.clefs.insert(0, (Frac::ZERO, own));
            }
            let lane = finish_staff(staff, player, inst, movement.length, &mut p);
            movement.lanes.push(lane);
        }
    }

    // Defaults where the first part had none.
    if movement.meter.is_empty() {
        movement.meter.push(Meter { at: Frac::ZERO, num: 4, den: 4 });
    }
    if movement.keys.is_empty() {
        movement.keys.push(Key { at: Frac::ZERO, fifths: 0, minor: false });
    }
    if movement.tempo.is_empty() {
        movement.tempo.push(Tempo { at: Frac::ZERO, bpm: 120.0, ramp: false, text: String::new() });
    }
    movement.meter.sort_by(|a, b| a.at.cmp(&b.at));
    movement.keys.sort_by(|a, b| a.at.cmp(&b.at));
    movement.tempo.sort_by(|a, b| a.at.cmp(&b.at));
    movement.id = p.allocate_id();
    p.movements.push(movement);
    Ok(p)
}

/// `<beats>` may be "3+2": the sum.
fn beats(t: Node) -> Option<i64> {
    let s = text_of(t, "beats")?;
    s.split('+').map(|x| x.trim().parse::<i64>().ok()).sum()
}

fn direction(e: Node, at: Frac, first: bool, staff: &mut StaffMusic, movement: &mut Movement, p: &mut Project) {
    let mut words: Vec<String> = Vec::new();
    let mut bpm: Option<f64> = None;
    for dt in children(e, "direction-type") {
        for c in dt.children().filter(|c| c.is_element()) {
            match c.tag_name().name() {
                "dynamics" => {
                    for d in c.children().filter(|d| d.is_element()) {
                        let name = d.tag_name().name();
                        match DYNAMICS.iter().position(|x| *x == name) {
                            Some(i) => staff.dynamics.push((at, i as u8)),
                            None => {
                                let t = if name == "other-dynamics" { d.text().unwrap_or_default().trim().to_string() } else { name.to_string() };
                                staff.texts.push((at, t));
                            }
                        }
                    }
                }
                "wedge" => match c.attribute("type") {
                    Some("crescendo") => staff.wedges.push((at, span::CRESC_START)),
                    Some("diminuendo") => staff.wedges.push((at, span::DIM_START)),
                    Some("stop") => staff.wedges.push((at, span::HAIRPIN_END)),
                    _ => {}
                },
                "words" => {
                    if let Some(t) = c.text().map(str::trim).filter(|t| !t.is_empty()) {
                        words.push(t.to_string());
                    }
                }
                "metronome" => {
                    let unit = text_of(c, "beat-unit").unwrap_or_else(|| "quarter".into());
                    let dotted = child(c, "beat-unit-dot").is_some();
                    if let Some(pm) = text_of(c, "per-minute").and_then(|t| t.parse::<f64>().ok()) {
                        let quarters = match unit.as_str() {
                            "whole" => 4.0,
                            "half" => 2.0,
                            "eighth" => 0.5,
                            "16th" => 0.25,
                            _ => 1.0,
                        } * if dotted { 1.5 } else { 1.0 };
                        bpm = Some(pm * quarters);
                    }
                }
                "rehearsal" if first => {
                    let id = p.allocate_id();
                    if !movement.rehearsal.iter().any(|r| r.at == at) {
                        movement.rehearsal.push(RehearsalMark { id, at });
                    }
                }
                _ => {}
            }
        }
    }
    if bpm.is_none() {
        bpm = child(e, "sound").and_then(|s| s.attribute("tempo")).and_then(|t| t.parse::<f64>().ok());
    }
    match bpm {
        Some(b) if first => {
            movement.tempo.retain(|t| t.at != at);
            movement.tempo.push(Tempo { at, bpm: b, ramp: false, text: words.join(" ") });
        }
        _ => {
            for w in words {
                staff.texts.push((at, w));
            }
        }
    }
}

fn barline(e: Node, bar: Frac, bar_end: Frac, open: &mut BTreeMap<String, Frac>, movement: &mut Movement, p: &mut Project) {
    let right = e.attribute("location").unwrap_or("right") == "right";
    let at = if right { bar_end } else { bar };
    let style = text_of(e, "bar-style").unwrap_or_default();
    let repeat = child(e, "repeat").and_then(|r| r.attribute("direction").map(String::from));
    let kind = match (repeat.as_deref(), style.as_str()) {
        (Some("forward"), _) => Some(BarKind::RepeatStart),
        (Some("backward"), _) => Some(BarKind::RepeatEnd),
        (_, "light-heavy") => Some(BarKind::Final),
        (_, "light-light") => Some(BarKind::Double),
        _ => None,
    };
    if let Some(kind) = kind {
        // A backward repeat meeting a forward one is both.
        if let Some(b) = movement.barlines.iter_mut().find(|b| b.at == at) {
            let both = matches!((b.kind, kind), (BarKind::RepeatEnd, BarKind::RepeatStart) | (BarKind::RepeatStart, BarKind::RepeatEnd));
            if both {
                b.kind = BarKind::RepeatBoth;
            }
        } else {
            let id = p.allocate_id();
            movement.barlines.push(Barline { id, at, kind });
        }
    }
    if let Some(en) = child(e, "ending") {
        let numbers = en.attribute("number").unwrap_or("1").to_string();
        match en.attribute("type") {
            Some("start") => {
                open.insert(numbers, bar);
            }
            Some("stop") | Some("discontinue") => {
                let from = open.remove(&numbers).unwrap_or(bar);
                let passes = numbers.split([',', ' ']).filter_map(|n| n.trim().parse::<u8>().ok()).filter(|n| (1..=8).contains(n)).fold(0u8, |acc, n| acc | 1 << (n - 1));
                let id = p.allocate_id();
                movement.endings.push(Ending { id, from, to: bar_end, passes: if passes == 0 { 1 } else { passes } });
            }
            _ => {}
        }
    }
}

fn note(e: Node, start: Frac, dur: Frac, chord: bool, staff: &mut StaffMusic, voice: String, p: &mut Project) {
    let rest = child(e, "rest").is_some();
    let pitch = child(e, "pitch").map(|pn| {
        let step = text_of(pn, "step").and_then(|s| "CDEFGAB".find(&s)).unwrap_or(0) as i8;
        let alter = text_of(pn, "alter").and_then(|a| a.parse::<f64>().ok()).map(|a| a.round() as i8).unwrap_or(0).clamp(-2, 2);
        let octave = num::<i8>(pn, "octave").unwrap_or(4);
        Pitch { step, alter, octave }
    });
    // Unpitched notes sound at their display position.
    let pitch = pitch.or_else(|| {
        child(e, "unpitched").map(|u| Pitch {
            step: text_of(u, "display-step").and_then(|s| "CDEFGAB".find(&s)).unwrap_or(6) as i8,
            alter: 0,
            octave: num::<i8>(u, "display-octave").unwrap_or(4),
        })
    });
    let tie = children(e, "tie").any(|t| t.attribute("type") == Some("start"))
        || child(e, "notations").is_some_and(|n| children(n, "tied").any(|t| t.attribute("type") == Some("start")));
    let mut arts = 0u16;
    let mut spans = 0u8;
    if let Some(nt) = child(e, "notations") {
        for c in nt.children().filter(|c| c.is_element()) {
            match c.tag_name().name() {
                "slur" => match c.attribute("type") {
                    Some("start") => spans |= span::SLUR_START,
                    Some("stop") => spans |= span::SLUR_END,
                    _ => {}
                },
                "fermata" => arts |= art::FERMATA,
                "arpeggiate" => arts |= art::ARPEGGIO,
                "articulations" => {
                    for a in c.children().filter(|a| a.is_element()) {
                        arts |= match a.tag_name().name() {
                            "staccato" => art::STACCATO,
                            "accent" => art::ACCENT,
                            "tenuto" => art::TENUTO,
                            "detached-legato" => art::TENUTO | art::STACCATO,
                            "strong-accent" => art::MARCATO,
                            "staccatissimo" | "spiccato" => art::STACCATISSIMO,
                            "breath-mark" => art::BREATH,
                            "caesura" => art::CAESURA,
                            _ => 0,
                        };
                    }
                }
                "ornaments" => {
                    for o in c.children().filter(|o| o.is_element()) {
                        arts |= match o.tag_name().name() {
                            "trill-mark" => art::TRILL,
                            "mordent" | "inverted-mordent" => art::MORDENT,
                            "turn" | "inverted-turn" => art::TURN,
                            "tremolo" => art::TREMOLO,
                            _ => 0,
                        };
                    }
                }
                _ => {}
            }
        }
    }
    let lyrics: Vec<Lyric> = children(e, "lyric")
        .filter_map(|l| {
            let text = text_of(l, "text")?;
            let verse = l.attribute("number").and_then(|n| n.parse::<u8>().ok()).unwrap_or(1).clamp(1, 9);
            let syl = text_of(l, "syllabic").unwrap_or_default();
            Some(Lyric { verse, text, hyphen: syl == "begin" || syl == "middle", extend: child(l, "extend").is_some() })
        })
        .collect();
    let events = staff.voices.entry(voice).or_default();
    if chord && !rest {
        if let (Some(last), Some(pitch)) = (events.last_mut(), pitch) {
            if last.start == start && !last.is_rest() {
                let id = p.allocate_id();
                last.heads.push(Head { id, pitch, tie, velocity: 0 });
                last.heads.sort_by_key(|h| h.pitch.diatonic());
                last.kind = EventKind::Chord;
                last.articulations |= arts;
                last.spans |= spans;
                return;
            }
        }
    }
    let id = p.allocate_id();
    let heads = match (rest, pitch) {
        (false, Some(pitch)) => vec![Head { id: p.allocate_id(), pitch, tie, velocity: 0 }],
        _ => vec![],
    };
    let kind = if heads.is_empty() { EventKind::Rest } else { EventKind::Note };
    events.push(Event { id, kind, start, duration: dur, heads, articulations: arts, dynamic: None, spans, lyrics });
}

/// A staff's lane: voices numbered in order of appearance (at most four;
/// further ones join the fourth), voice 1 filled with rests where it is
/// silent; dynamics and hairpins on the next note.
fn finish_staff(mut s: StaffMusic, player: Id, inst: Id, length: Frac, p: &mut Project) -> Lane {
    let mut voices: Vec<Vec<Event>> = Vec::new();
    // The voice with the most music first.
    let mut named: Vec<(String, Vec<Event>)> = std::mem::take(&mut s.voices).into_iter().collect();
    named.sort_by_key(|(name, evs)| (std::cmp::Reverse(evs.iter().filter(|e| !e.is_rest()).count()), name.clone()));
    for (_, mut evs) in named {
        evs.sort_by_key(|e| e.start);
        if voices.len() < 4 {
            voices.push(evs);
        } else {
            voices[3].extend(evs);
            voices[3].sort_by_key(|e| e.start);
        }
    }
    if voices.is_empty() {
        voices.push(Vec::new());
    }
    // Dynamics and hairpins go on the first event at or after them (any voice, voice 1 first).
    let attach = |voices: &mut Vec<Vec<Event>>, at: Frac, f: &mut dyn FnMut(&mut Event)| {
        for v in voices.iter_mut() {
            if let Some(e) = v.iter_mut().find(|e| e.start >= at && !e.is_rest()) {
                f(e);
                return;
            }
        }
    };
    for (at, d) in std::mem::take(&mut s.dynamics) {
        attach(&mut voices, at, &mut |e| e.dynamic = Some(d));
    }
    for (at, w) in std::mem::take(&mut s.wedges) {
        if w == span::HAIRPIN_END {
            // A hairpin ends on the last note starting before its stop.
            for v in voices.iter_mut() {
                if let Some(e) = v.iter_mut().rev().find(|e| e.start < at && !e.is_rest()) {
                    e.spans |= span::HAIRPIN_END;
                    break;
                }
            }
        } else {
            attach(&mut voices, at, &mut |e| e.spans |= w);
        }
    }
    // Voice 1 runs from 0 to the end without gaps; overlaps are trimmed.
    let mut v1: Vec<Event> = Vec::new();
    let mut t = Frac::ZERO;
    for e in std::mem::take(&mut voices[0]) {
        if e.start < t {
            continue;
        }
        if e.start > t {
            v1.push(rest(p, t, e.start - t));
        }
        t = e.end();
        v1.push(e);
    }
    if t < length {
        v1.push(rest(p, t, length - t));
    }
    voices[0] = v1;
    let mut lane_voices = Vec::new();
    for (i, evs) in voices.into_iter().enumerate() {
        let id = p.allocate_id();
        lane_voices.push(Voice { id, index: i as u8 + 1, events: Arc::new(evs) });
    }
    let change = p.allocate_id();
    let clefs = s.clefs.into_iter().map(|(at, clef)| ClefChange { id: p.allocate_id(), at, clef }).collect();
    let texts = s.texts.into_iter().map(|(at, text)| TextItem { id: p.allocate_id(), at, text, kind: TextKind::Expression }).collect();
    let chords = s.chords.into_iter().map(|(at, text)| ChordSymbol { id: p.allocate_id(), at, text }).collect();
    Lane { player, changes: vec![InstrumentChange { id: change, at: Frac::ZERO, instrument: inst }], clefs, texts, chords, voices: lane_voices }
}

fn rest(p: &mut Project, start: Frac, duration: Frac) -> Event {
    Event { id: p.allocate_id(), kind: EventKind::Rest, start, duration, heads: vec![], articulations: 0, dynamic: None, spans: 0, lyrics: vec![] }
}
