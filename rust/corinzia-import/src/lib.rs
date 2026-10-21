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
    RehearsalMark, Shown, Tempo, TextItem, TextKind, Track, Tuplet, Voice, DYNAMICS, Grace, LineKind, LineSpan, Nav, NavMark, NavTo,
};
use corinzia_score::score::{art, span, Lyric, Slur};
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
    /// Words: (at, text, written above the staff).
    texts: Vec<(Frac, String, bool)>,
    chords: Vec<(Frac, String)>,
    /// MusicXML voice name -> events (in time order).
    voices: BTreeMap<String, Vec<Event>>,
    /// MusicXML voice name -> its tuplets, and those still open
    /// (start, actual, normal).
    tuplets: BTreeMap<String, Vec<Tuplet>>,
    open_tuplets: BTreeMap<String, Vec<(Frac, i64, i64)>>,
    /// Octave lines and pedals, and those still open (start, kind).
    lines: Vec<LineSpan>,
    open_lines: Vec<(Frac, LineKind)>,
    /// MusicXML voice name -> grace notes waiting for their note.
    graces: BTreeMap<String, Vec<Grace>>,
    /// Dynamics and hairpins waiting for the next event at or after a time.
    dynamics: Vec<(Frac, u8)>,
    wedges: Vec<(Frac, u8)>,
}

/// A <slur> mark: (time, staff and voice, note, start?, number, side).
type SlurMark = (Frac, (usize, String), Id, bool, u8, i8);

/// A part's slurs, paired by their `number` across its voices and staves.
#[derive(Default)]
struct PartSlurs {
    /// Every <slur> mark: (time, staff and voice, note, start?, number, side).
    marks: Vec<SlurMark>,
}

impl PartSlurs {
    /// In time order, each stop takes the open start of its number, the
    /// one in its own voice if there is one (files reuse numbers across
    /// voices, and a voice written later can hold an earlier start).
    fn pair(mut self, p: &mut Project) -> Vec<Slur> {
        self.marks.sort_by(|a, b| a.0.cmp(&b.0).then(a.3.cmp(&b.3))); // stops first at a time
        let mut open: Vec<SlurMark> = Vec::new();
        let mut out = Vec::new();
        for mark in self.marks {
            let (at, voice, note, start, number, _) = mark.clone();
            if start {
                open.push(mark);
                continue;
            }
            let cands = || open.iter().enumerate().filter(|(_, o)| o.4 == number && o.0 < at && o.2 != note);
            let pick = cands().filter(|(_, o)| o.1 == voice).next_back().or_else(|| cands().next_back()).map(|(i, _)| i);
            if let Some(i) = pick {
                let (_, _, from, _, _, dir) = open.remove(i);
                out.push(Slur { id: p.allocate_id(), from, to: note, dir });
            }
        }
        out
    }
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
    let mut pickup: Option<Frac> = None;
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
            let len = if longest > Frac::ZERO { longest } else { meter_len };
            if mi == 0 && len < meter_len {
                pickup = Some(len);
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
        navigation: vec![],
        slurs: vec![],
        endings: vec![],
        lanes: vec![], breaks: vec![], cross: vec![],
    };

    for (pi, part) in parts.iter().enumerate() {
        let pid = part.attribute("id").unwrap_or_default();
        let first = pi == 0;
        let mut staves: Vec<StaffMusic> = vec![StaffMusic::default()];
        let mut slurs = PartSlurs::default();
        let mut divisions: i64 = 1;
        let mut open_endings: BTreeMap<String, Frac> = BTreeMap::new();
        // A transposing part is written: <transpose> says what turns its
        // written pitches into sounding ones.
        let mut to_concert = corinzia_model::Interval::default();
        for (mi, m) in children(*part, "measure").enumerate() {
            let Some(&bar) = bar_starts.get(mi) else { break };
            let bar_end = bar_starts.get(mi + 1).copied().unwrap_or(end);
            let mut cursor = Frac::ZERO;
            let mut last_start = Frac::ZERO;
            let q = |d: i64, div: i64| Frac::new(d, div * 4);
            for e in m.children().filter(|c| c.is_element()) {
                let at = bar + cursor;
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
                        if let Some(t) = child(e, "transpose") {
                            let oct = int(t, "octave-change").unwrap_or(0) as i32;
                            to_concert = corinzia_model::Interval::new(int(t, "diatonic").unwrap_or(0) as i32 + 7 * oct, int(t, "chromatic").unwrap_or(0) as i32 + 12 * oct);
                        }
                        if first {
                            if let Some(k) = child(e, "key") {
                                if let Some(f) = int(k, "fifths") {
                                    let minor = text_of(k, "mode").as_deref() == Some("minor");
                                    let at = if mi == 0 { Frac::ZERO } else { at };
                                    // The written key of a transposing part, at concert pitch.
                                    let f = corinzia_model::shift_key(f.clamp(-7, 7) as i32, to_concert.fifths());
                                    movement.keys.retain(|x| x.at != at);
                                    movement.keys.push(Key { at, fifths: f, minor });
                                }
                            }
                            if let Some(t) = child(e, "time") {
                                if let (Some(b), Some(bt)) = (beats(t), int(t, "beat-type")) {
                                    let at = if mi == 0 { Frac::ZERO } else { bar };
                                    movement.meter.retain(|x| x.at != at);
                                    movement.meter.push(Meter { at, num: b, den: bt.max(1), pickup: false, beams: [0; 8] });
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
                            if s.clef.is_none() && at <= Frac::ZERO {
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
                        direction(e, at, (bar, bar_end), first, &mut staves[staff], &mut movement, &mut p);
                    }
                    "harmony" => {
                        if let Some(t) = harmony_text(e) {
                            let staff = num::<usize>(e, "staff").unwrap_or(1).max(1) - 1;
                            while staves.len() <= staff {
                                staves.push(StaffMusic::default());
                            }
                            let t = if to_concert.is_zero() { t } else { corinzia_model::notation::transpose_chord_text(&t, to_concert) };
                            staves[staff].chords.push((at, t));
                        }
                    }
                    "barline" if first => barline(e, bar, bar_end, &mut open_endings, &mut movement, &mut p),
                    "note" => {
                        if child(e, "cue").is_some() {
                            continue;
                        }
                        if child(e, "grace").is_some() {
                            let staff = num::<usize>(e, "staff").unwrap_or(1).max(1) - 1;
                            while staves.len() <= staff {
                                staves.push(StaffMusic::default());
                            }
                            let voice = text_of(e, "voice").unwrap_or_else(|| "1".into());
                            grace_note(e, &mut staves[staff], voice, to_concert, &mut p);
                            continue;
                        }
                        let dur = q(int(e, "duration").unwrap_or(0), divisions);
                        let chord = child(e, "chord").is_some();
                        let start = if chord { bar + last_start } else { at };
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
                        if !chord {
                            tuplet_marks(e, start, dur, &voice, &mut staves[staff]);
                        }
                        note(e, start, dur, chord, &mut staves[staff], staff, voice, to_concert, &mut slurs, &mut p);
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
        p.instruments.push(Instrument { id: inst, player, name: name.clone(), engraver_name: abbr, transposition: to_concert.inverse(), clef: clef0, track: Some(track), drums: vec![], tuning: vec![] });
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
        movement.slurs.extend(slurs.pair(&mut p));
    }

    // Defaults where the first part had none.
    if movement.meter.is_empty() {
        movement.meter.push(Meter { at: Frac::ZERO, num: 4, den: 4, pickup: false, beams: [0; 8] });
    }
    if movement.keys.is_empty() {
        movement.keys.push(Key { at: Frac::ZERO, fifths: 0, minor: false });
    }
    if movement.tempo.is_empty() {
        movement.tempo.push(Tempo { at: Frac::ZERO, bpm: 120.0, ramp: false, text: String::new() });
    }
    movement.meter.sort_by(|a, b| a.at.cmp(&b.at));
    movement.navigation.sort_by(|a, b| a.at.cmp(&b.at));
    // A short first bar is a pickup in the opening meter.
    if let Some(len) = pickup {
        if movement.meter.len() == 1 || movement.meter[1].at != len {
            movement.meter[0].at = len;
        } else {
            movement.meter.remove(0);
        }
        movement.meter.insert(0, Meter { at: Frac::ZERO, num: len.num, den: len.den, pickup: true, beams: [0; 8] });
    }
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

/// A navigation mark from the first part (one per kind and position).
fn add_nav(movement: &mut Movement, p: &mut Project, at: Frac, kind: Nav) {
    if !movement.navigation.iter().any(|n| n.at == at && n.kind == kind) {
        movement.navigation.push(NavMark { id: p.allocate_id(), at, kind });
    }
}

fn direction(e: Node, at: Frac, (bar, bar_end): (Frac, Frac), first: bool, staff: &mut StaffMusic, movement: &mut Movement, p: &mut Project) {
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
                                staff.texts.push((at, t, false));
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
                "octave-shift" | "pedal" => {
                    let ty = c.attribute("type").unwrap_or("");
                    let octave = c.tag_name().name() == "octave-shift";
                    let close = |staff: &mut StaffMusic, p: &mut Project, pedal: bool| {
                        if let Some(i) = staff.open_lines.iter().rposition(|(_, k)| (*k == LineKind::Pedal) == pedal) {
                            let (from, kind) = staff.open_lines.remove(i);
                            if at > from {
                                staff.lines.push(LineSpan { id: p.allocate_id(), from, to: at, kind });
                            }
                        }
                    };
                    match (octave, ty) {
                        (true, "stop") => close(staff, p, false),
                        (true, "down" | "up") => {
                            let size = c.attribute("size").and_then(|s| s.parse::<i32>().ok()).unwrap_or(8);
                            let k = if size >= 15 { 2 } else { 1 } * if ty == "down" { 1 } else { -1 };
                            staff.open_lines.push((at, LineKind::Octave(k)));
                        }
                        (false, "start") => staff.open_lines.push((at, LineKind::Pedal)),
                        (false, "stop") => close(staff, p, true),
                        (false, "change") => {
                            close(staff, p, true);
                            staff.open_lines.push((at, LineKind::Pedal));
                        }
                        _ => {}
                    }
                }
                "segno" if first => add_nav(movement, p, if at >= bar_end { bar_end } else { bar }, Nav::Segno),
                "coda" if first => add_nav(movement, p, if at >= bar_end { bar_end } else { bar }, Nav::Coda),
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
    // Jumps, Fine and To Coda: from the words, else from <sound>; they
    // happen at the end of their bar.
    if first {
        let sound = child(e, "sound");
        let said = |a: &str| sound.and_then(|s| s.attribute(a)).is_some_and(|v| !v.is_empty() && v != "no");
        let from_words = words.iter().position(|w| corinzia_edit::parse_nav(w).is_some_and(|n| !n.at_bar_start()));
        let kind = from_words.and_then(|i| corinzia_edit::parse_nav(&words[i])).or_else(|| {
            if said("dacapo") {
                Some(Nav::DaCapo(NavTo::End))
            } else if said("dalsegno") {
                Some(Nav::DalSegno(NavTo::End))
            } else if said("tocoda") {
                Some(Nav::ToCoda)
            } else if said("fine") {
                Some(Nav::Fine)
            } else {
                None
            }
        });
        if let Some(k) = kind {
            if let Some(i) = from_words {
                words.remove(i);
            }
            add_nav(movement, p, if at > bar { bar_end } else { bar }, k);
        }
    }
    match bpm {
        Some(b) if first => {
            movement.tempo.retain(|t| t.at != at);
            movement.tempo.push(Tempo { at, bpm: b, ramp: false, text: words.join(" ") });
        }
        _ => {
            // Words above the staff are directions (technique text, set
            // above); below, expression.
            let above = e.attribute("placement") == Some("above");
            for w in words {
                staff.texts.push((at, w, above));
            }
        }
    }
}

fn barline(e: Node, bar: Frac, bar_end: Frac, open: &mut BTreeMap<String, Frac>, movement: &mut Movement, p: &mut Project) {
    let right = e.attribute("location").unwrap_or("right") == "right";
    // A sign on a barline starts the bar after it.
    for (tag, kind) in [("segno", Nav::Segno), ("coda", Nav::Coda)] {
        if child(e, tag).is_some() {
            add_nav(movement, p, if right { bar_end } else { bar }, kind);
        }
    }
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

#[allow(clippy::too_many_arguments)]
/// A `<grace>` note: waits in its voice for the next note (a `<chord/>`
/// grace joins the one before).
fn grace_note(e: Node, staff: &mut StaffMusic, voice: String, to_concert: corinzia_model::Interval, p: &mut Project) {
    let Some(pitch) = note_pitch(e, to_concert) else { return };
    let pending = staff.graces.entry(voice).or_default();
    let head = Head { id: p.allocate_id(), pitch, tie: false, velocity: 0 };
    if child(e, "chord").is_some() {
        if let Some(g) = pending.last_mut() {
            g.heads.push(head);
            g.heads.sort_by_key(|h| h.pitch.diatonic());
            return;
        }
    }
    let base = match text_of(e, "type").as_deref() {
        Some("16th") => 16,
        Some("32nd") => 32,
        Some("64th") => 64,
        _ => 8,
    };
    let slash = child(e, "grace").and_then(|g| g.attribute("slash")) == Some("yes");
    pending.push(Grace { id: p.allocate_id(), heads: vec![head], base, slash });
}

/// A note's pitch (unpitched notes at their display position), at concert pitch.
fn note_pitch(e: Node, to_concert: corinzia_model::Interval) -> Option<Pitch> {
    let pitch = child(e, "pitch").map(|pn| {
        let step = text_of(pn, "step").and_then(|s| "CDEFGAB".find(&s)).unwrap_or(0) as i8;
        let alter = text_of(pn, "alter").and_then(|a| a.parse::<f64>().ok()).map(|a| a.round() as i8).unwrap_or(0).clamp(-2, 2);
        let octave = num::<i8>(pn, "octave").unwrap_or(4);
        Pitch { step, alter, octave }.transposed(to_concert)
    });
    pitch.or_else(|| {
        child(e, "unpitched").map(|u| Pitch {
            step: text_of(u, "display-step").and_then(|s| "CDEFGAB".find(&s)).unwrap_or(6) as i8,
            alter: 0,
            octave: num::<i8>(u, "display-octave").unwrap_or(4),
        })
    })
}

#[allow(clippy::too_many_arguments)]
fn note(e: Node, start: Frac, dur: Frac, chord: bool, staff: &mut StaffMusic, staff_index: usize, voice: String, to_concert: corinzia_model::Interval, slurs: &mut PartSlurs, p: &mut Project) {
    let rest = child(e, "rest").is_some();
    let pitch = note_pitch(e, to_concert);
    let tie = children(e, "tie").any(|t| t.attribute("type") == Some("start"))
        || child(e, "notations").is_some_and(|n| children(n, "tied").any(|t| t.attribute("type") == Some("start")));
    let mut arts = 0u16;
    let mut finger: Option<u8> = None;
    let mut slur_marks: Vec<(bool, u8, i8)> = Vec::new();
    // Every <notations> of the note (some files split them).
    for nt in children(e, "notations") {
        for c in nt.children().filter(|c| c.is_element()) {
            match c.tag_name().name() {
                "slur" => {
                    let number = c.attribute("number").and_then(|n| n.parse::<u8>().ok()).unwrap_or(1);
                    let dir = match c.attribute("placement").or(c.attribute("orientation")) {
                        Some("above" | "over") => 1,
                        Some("below" | "under") => -1,
                        _ => 0,
                    };
                    match c.attribute("type") {
                        Some("start") => slur_marks.push((true, number, dir)),
                        Some("stop") => slur_marks.push((false, number, 0)),
                        _ => {}
                    }
                }
                "fermata" => arts |= art::FERMATA,
                "technical" => {
                    for t in c.children().filter(|t| t.is_element()) {
                        match t.tag_name().name() {
                            "up-bow" => arts |= art::UP_BOW,
                            "down-bow" => arts |= art::DOWN_BOW,
                            "fingering" => finger = t.text().and_then(|x| x.trim().parse::<u8>().ok()).filter(|f| *f <= 5),
                            _ => {}
                        }
                    }
                }
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
    let graces = if chord { vec![] } else { staff.graces.remove(&voice).unwrap_or_default() };
    let events = staff.voices.entry(voice.clone()).or_default();
    if chord && !rest {
        if let (Some(last), Some(pitch)) = (events.last_mut(), pitch) {
            if last.start == start && !last.is_rest() {
                let id = p.allocate_id();
                // Fingers stay with their heads as the chord is sorted.
                let mut fingers = std::mem::take(&mut last.fingering);
                fingers.resize(last.heads.len(), 0);
                let mut both: Vec<(Head, u8)> = last.heads.drain(..).zip(fingers).collect();
                both.push((Head { id, pitch, tie, velocity: 0 }, finger.unwrap_or(0)));
                both.sort_by_key(|(h, _)| h.pitch.diatonic());
                let (heads, fingers): (Vec<Head>, Vec<u8>) = both.into_iter().unzip();
                last.heads = heads;
                last.fingering = if fingers.iter().any(|&f| f > 0) { fingers } else { vec![] };
                last.kind = EventKind::Chord;
                last.articulations |= arts;
                let note = last.id;
                slurs.marks.extend(slur_marks.iter().map(|&(st, n, d)| (start, (staff_index, voice.clone()), note, st, n, d)));
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
    events.push(Event { id, kind, start, duration: dur, heads, articulations: arts, dynamic: None, spans: 0, lyrics, graces, fingering: finger.filter(|_| !rest).into_iter().collect() });
    if !rest {
        slurs.marks.extend(slur_marks.iter().map(|&(st, n, d)| (start, (staff_index, voice.clone()), id, st, n, d)));
    }
}

/// `<tuplet type="start|stop">` on a note (nested ones too): the ratio
/// from `<tuplet-actual>`/`<tuplet-normal>`, else `<time-modification>`.
fn tuplet_marks(e: Node, start: Frac, dur: Frac, voice: &str, s: &mut StaffMusic) {
    let tm = child(e, "time-modification");
    let (ta, tn) = (tm.and_then(|t| int(t, "actual-notes")).unwrap_or(3), tm.and_then(|t| int(t, "normal-notes")).unwrap_or(2));
    let marks: Vec<Node> = children(e, "notations").flat_map(|n| children(n, "tuplet")).collect();
    for t in marks.iter().filter(|t| t.attribute("type") == Some("start")) {
        let number = |tag: &str| child(*t, tag).and_then(|x| int(x, "tuplet-number"));
        let (a, n) = (number("tuplet-actual").unwrap_or(ta), number("tuplet-normal").unwrap_or(tn));
        s.open_tuplets.entry(voice.to_string()).or_default().push((start, a.max(1), n.max(1)));
    }
    for _ in marks.iter().filter(|t| t.attribute("type") == Some("stop")) {
        if let Some((from, actual, normal)) = s.open_tuplets.get_mut(voice).and_then(|o| o.pop()) {
            s.tuplets.entry(voice.to_string()).or_default().push(Tuplet { start: from, length: start + dur - from, actual, normal });
        }
    }
}

/// A staff's lane: voices numbered in order of appearance (at most four;
/// further ones join the fourth), voice 1 filled with rests where it is
/// silent; dynamics and hairpins on the next note.
fn finish_staff(mut s: StaffMusic, player: Id, inst: Id, length: Frac, p: &mut Project) -> Lane {
    let mut voices: Vec<Vec<Event>> = Vec::new();
    let mut tuplets: Vec<Vec<Tuplet>> = Vec::new();
    // The voice with the most music first.
    let mut named: Vec<(String, Vec<Event>)> = std::mem::take(&mut s.voices).into_iter().collect();
    named.sort_by_key(|(name, evs)| (std::cmp::Reverse(evs.iter().filter(|e| !e.is_rest()).count()), name.clone()));
    for (name, mut evs) in named {
        evs.sort_by_key(|e| e.start);
        let ts = s.tuplets.remove(&name).unwrap_or_default();
        if voices.len() < 4 {
            voices.push(evs);
            tuplets.push(ts);
        } else {
            voices[3].extend(evs);
            voices[3].sort_by_key(|e| e.start);
            tuplets[3].extend(ts);
        }
    }
    if voices.is_empty() {
        voices.push(Vec::new());
        tuplets.push(Vec::new());
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
    for (i, (evs, ts)) in voices.into_iter().zip(tuplets).enumerate() {
        let id = p.allocate_id();
        let ts = corinzia_edit::tuplets::sanitize(&evs, &ts);
        lane_voices.push(Voice { id, index: i as u8 + 1, events: Arc::new(evs), tuplets: ts });
    }
    // Lines never stopped run to the end.
    for (from, kind) in std::mem::take(&mut s.open_lines) {
        if from < length {
            s.lines.push(LineSpan { id: p.allocate_id(), from, to: length, kind });
        }
    }
    s.lines.sort_by(|a, b| a.from.cmp(&b.from));
    let change = p.allocate_id();
    let clefs = s.clefs.into_iter().map(|(at, clef)| ClefChange { id: p.allocate_id(), at, clef }).collect();
    let texts = s.texts.into_iter().map(|(at, text, above)| TextItem { id: p.allocate_id(), at, text, kind: if above { TextKind::Technique } else { TextKind::Expression }, shown: Shown::Both }).collect();
    let chords = s.chords.into_iter().map(|(at, text)| ChordSymbol { id: p.allocate_id(), at, text }).collect();
    Lane { player, changes: vec![InstrumentChange { id: change, at: Frac::ZERO, instrument: inst }], clefs, texts, chords, lines: s.lines, voices: lane_voices }
}

fn rest(p: &mut Project, start: Frac, duration: Frac) -> Event {
    Event { id: p.allocate_id(), kind: EventKind::Rest, start, duration, heads: vec![], articulations: 0, dynamic: None, spans: 0, lyrics: vec![], graces: vec![], fingering: vec![] }
}
