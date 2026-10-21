//! New projects from an instrument list, and export (PDF from the native
//! engraver, MIDI and MusicXML from the performance compiler). Everything
//! here is synchronous and UI-free; the app runs dialogs around it.

use corinzia_layout::{layout_pages, Options, PageSpec};
use corinzia_model::{Interval, Project};
use std::path::Path;

/// Known instruments: names (lower case, with aliases), display name,
/// abbreviation, clef ("treble+bass": a grand staff, one staff per clef).
/// Scores are written at concert pitch.
const INSTRUMENTS: &[(&[&str], &str, &str, &str)] = &[
    (&["piccolo"], "Piccolo", "Picc.", "treble"),
    (&["flute", "fl"], "Flute", "Fl.", "treble"),
    (&["oboe", "ob"], "Oboe", "Ob.", "treble"),
    (&["cor anglais", "english horn"], "Cor anglais", "C. a.", "treble"),
    (&["clarinet", "cl", "clarinet in bb", "bb clarinet"], "Clarinet in B♭", "Cl.", "treble"),
    (&["clarinet in a", "a clarinet"], "Clarinet in A", "Cl.", "treble"),
    (&["bass clarinet"], "Bass clarinet", "B. Cl.", "bass"),
    (&["bassoon", "bsn"], "Bassoon", "Bsn.", "bass"),
    (&["contrabassoon"], "Contrabassoon", "Cbsn.", "bass_8"),
    (&["soprano saxophone", "soprano sax"], "Soprano saxophone", "S. Sax.", "treble"),
    (&["saxophone", "alto saxophone", "alto sax", "sax"], "Alto saxophone", "A. Sax.", "treble"),
    (&["tenor saxophone", "tenor sax"], "Tenor saxophone", "T. Sax.", "treble"),
    (&["baritone saxophone", "baritone sax", "bari sax"], "Baritone saxophone", "Bar. Sax.", "bass"),
    (&["horn", "french horn", "hn", "horn in f"], "Horn in F", "Hn.", "treble"),
    (&["trumpet", "tpt", "trumpet in bb", "bb trumpet"], "Trumpet in B♭", "Tpt.", "treble"),
    (&["trumpet in c", "c trumpet"], "Trumpet in C", "Tpt.", "treble"),
    (&["trombone", "tbn"], "Trombone", "Tbn.", "bass"),
    (&["bass trombone"], "Bass trombone", "B. Tbn.", "bass"),
    (&["tuba"], "Tuba", "Tba.", "bass"),
    (&["timpani", "timp"], "Timpani", "Timp.", "bass"),
    (&["percussion", "perc"], "Percussion", "Perc.", "percussion"),
    (&["harp"], "Harp", "Hp.", "treble+bass"),
    (&["piano", "pno"], "Piano", "Pno.", "treble+bass"),
    (&["organ", "org"], "Organ", "Org.", "treble+bass"),
    (&["celesta"], "Celesta", "Cel.", "treble+bass"),
    (&["guitar", "gtr"], "Guitar", "Gtr.", "treble_8"),
    (&["soprano", "s"], "Soprano", "S.", "treble"),
    (&["alto", "a", "mezzo"], "Alto", "A.", "treble"),
    (&["tenor", "t"], "Tenor", "T.", "treble_8"),
    (&["baritone"], "Baritone", "Bar.", "bass"),
    (&["bass", "b"], "Bass", "B.", "bass"),
    (&["violin", "vln", "violin i", "violin ii"], "Violin", "Vln.", "treble"),
    (&["viola", "vla"], "Viola", "Vla.", "alto"),
    (&["cello", "violoncello", "vc"], "Cello", "Vc.", "bass"),
    (&["double bass", "contrabass", "db", "cb"], "Double bass", "D.B.", "bass_8"),
];

/// Transposing instruments: display name -> written minus sounding.
/// (Octave-transposing ones use octave clefs instead, e.g. "treble_8".)
const TRANSPOSITIONS: &[(&str, Interval)] = &[
    ("Clarinet in B♭", Interval::new(1, 2)),
    ("Clarinet in A", Interval::new(2, 3)),
    ("Bass clarinet", Interval::new(1, 2)),
    ("Cor anglais", Interval::new(4, 7)),
    ("Soprano saxophone", Interval::new(1, 2)),
    ("Alto saxophone", Interval::new(5, 9)),
    ("Tenor saxophone", Interval::new(8, 14)),
    ("Baritone saxophone", Interval::new(12, 21)),
    ("Horn in F", Interval::new(4, 7)),
    ("Trumpet in B♭", Interval::new(1, 2)),
];

pub fn transposition_of(name: &str) -> Interval {
    TRANSPOSITIONS.iter().find(|(n, _)| name.starts_with(n)).map(|t| t.1).unwrap_or_default()
}

/// Ready-made ensembles for the New score card: (label, instrument list).
pub const TEMPLATES: &[(&str, &str)] = &[
    ("Piano", "Piano"),
    ("Voice and piano", "Soprano, Piano"),
    ("String quartet", "2 Violin, Viola, Cello"),
    ("SATB choir", "Soprano, Alto, Tenor, Bass"),
    ("Wind quintet", "Flute, Oboe, Clarinet, Horn, Bassoon"),
    ("Brass quintet", "2 Trumpet, Horn, Trombone, Tuba"),
    ("Piano trio", "Violin, Cello, Piano"),
    ("Orchestra", "2 Flute, 2 Oboe, 2 Clarinet, 2 Bassoon, 2 Horn, 2 Trumpet, Timpani, 2 Violin, Viola, Cello, Double bass"),
];

/// The staff clef for an instrument not in the list, from its name.
fn guess_clef(name: &str) -> &'static str {
    let n = name.to_lowercase();
    if ["bass", "cello", "tuba", "bassoon", "trombone", "baritone", "contra", "timpani"].iter().any(|w| n.contains(w)) {
        "bass"
    } else if n.contains("viola") {
        "alto"
    } else if ["keyboard", "synth", "piano", "harpsichord", "marimba"].iter().any(|w| n.contains(w)) {
        "treble+bass"
    } else if ["drum", "perc", "kit"].iter().any(|w| n.contains(w)) {
        "percussion"
    } else {
        "treble"
    }
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
        let key = name.to_lowercase();
        match INSTRUMENTS.iter().find(|(names, ..)| names.contains(&key.as_str())) {
            Some((_, full, abbr, clef)) => {
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
                    out.push((format!("{full}{suffix}"), format!("{abbr}{suffix}"), guess_clef(name).to_string()));
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
/// "Violin 2"), or as typed with a clef guessed from it.
pub fn instrument_spec(name: &str) -> Option<crate::editor::InstrumentSpec> {
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    let (base, number) = match name.rsplit_once(' ') {
        Some((b, n)) if n.chars().all(|c| c.is_ascii_digit()) && !b.trim().is_empty() => (b.trim(), Some(n)),
        _ => (name, None),
    };
    let suffix = number.map(|n| format!(" {n}")).unwrap_or_default();
    let key = base.to_lowercase();
    let (full, abbr, clef) = match INSTRUMENTS.iter().find(|(names, ..)| names.contains(&key.as_str())) {
        Some((_, full, abbr, clef)) => (format!("{full}{suffix}"), format!("{abbr}{suffix}"), clef.to_string()),
        None => {
            let full: String = name.chars().take(40).collect();
            let full = match full.get(..1) {
                Some(c) => c.to_uppercase() + &full[1..],
                None => full,
            };
            let abbr = format!("{}.", full.chars().take(3).collect::<String>());
            (full, abbr, guess_clef(base).to_string())
        }
    };
    let transposition = transposition_of(&full);
    Some(crate::editor::InstrumentSpec { name: full, abbr, clef, transposition })
}

/// A player's instruments, comma separated: the first one, then doublings.
pub fn parse_instrument_list(text: &str) -> Result<Vec<crate::editor::InstrumentSpec>, String> {
    let list: Vec<_> = text.split([',', ';']).filter_map(instrument_spec).collect();
    if list.is_empty() {
        return Err("Each player needs an instrument".into());
    }
    Ok(list)
}

pub fn new_project(title: &str, instruments: &str, bars: i64) -> Result<Project, String> {
    let list = parse_instruments(instruments)?;
    let refs: Vec<(&str, &str, &str, Interval)> = list.iter().map(|(a, b, c)| (a.as_str(), b.as_str(), c.as_str(), transposition_of(a))).collect();
    Ok(crate::demo::blank_with(title, &refs, bars))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Pdf,
    /// Every player's part, one after another, in one PDF.
    Parts,
    Midi,
    MusicXml,
}
impl Format {
    pub fn extension(self) -> &'static str {
        match self {
            Format::Pdf | Format::Parts => "pdf",
            Format::Midi => "mid",
            Format::MusicXml => "musicxml",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Format::Pdf => "PDF",
            Format::Parts => "Parts (PDF)",
            Format::Midi => "MIDI file",
            Format::MusicXml => "MusicXML",
        }
    }
}

/// The current movement in a format, as bytes.
/// PDF follows the view (`transposed`: written pitches); MIDI and MusicXML
/// are written at concert pitch.
pub fn export(project: &Project, movement: usize, format: Format, transposed: bool) -> Result<Vec<u8>, String> {
    let m = project.movements.get(movement).ok_or("No movement to export")?;
    match format {
        Format::Pdf => {
            // Text is written as outlines of the text font: make sure it is loaded.
            if corinzia_layout::display::text_fonts().0.is_none() {
                crate::load_text_fonts();
            }
            Ok(corinzia_layout::pdf::to_pdf(&work_pages(project, transposed), if project.title.is_empty() { "Untitled" } else { &project.title }))
        }
        Format::Parts => {
            if corinzia_layout::display::text_fonts().0.is_none() {
                crate::load_text_fonts();
            }
            Ok(corinzia_layout::pdf::to_pdf(&parts_pages(project), if project.title.is_empty() { "Parts" } else { &project.title }))
        }
        Format::Midi => corinzia_perform::midi(project, m),
        Format::MusicXml => corinzia_perform::musicxml(project, m).map(String::into_bytes),
    }
}

/// Every movement's pages, as printed: the first under the work's title
/// and composer, each later one from a new page under its own title, page
/// numbers running on.
pub fn work_pages(project: &Project, transposed: bool) -> Vec<corinzia_layout::DisplayList> {
    let mut pages = Vec::new();
    for (i, m) in project.movements.iter().enumerate() {
        let mut score = corinzia_model::notation::score_as(project, m, transposed);
        if i > 0 {
            score.work_title.clear();
            score.composer.clear();
        }
        let spec = PageSpec { first_page: pages.len() + 1, ..PageSpec::default() };
        pages.extend(layout_pages(&score, &Options::default(), &spec));
    }
    pages
}

/// Every player's part (all movements, as written, with multi-bar rests),
/// each from a new page, its page numbers from 1.
pub fn parts_pages(project: &Project) -> Vec<corinzia_layout::DisplayList> {
    let opt = Options { multirest: true, names: false, ..Options::default() };
    let mut pages = Vec::new();
    for player in &project.players {
        let mut part: Vec<corinzia_layout::DisplayList> = Vec::new();
        for (i, m) in project.movements.iter().enumerate() {
            if !m.lanes.iter().any(|l| l.player == player.id) {
                continue;
            }
            let mut score = corinzia_model::notation::part_score(project, m, player.id);
            if i > 0 {
                score.work_title.clear();
                score.composer.clear();
            }
            let spec = PageSpec { first_page: part.len() + 1, ..PageSpec::default() };
            part.extend(layout_pages(&score, &opt, &spec));
        }
        pages.extend(part);
    }
    pages
}

pub fn export_to(project: &Project, movement: usize, format: Format, transposed: bool, path: &Path) -> Result<(), String> {
    let bytes = export(project, movement, format, transposed)?;
    std::fs::write(path, bytes).map_err(|e| format!("{}: {e}", path.display()))
}

/// A sibling path with another extension ("song.project" -> "song.pdf").
pub fn sibling(path: Option<&Path>, title: &str, ext: &str) -> std::path::PathBuf {
    match path {
        Some(p) => p.with_extension(ext),
        None => {
            let stem: String = title.chars().map(|c| if c.is_alphanumeric() || c == '-' { c } else { '_' }).collect();
            std::path::PathBuf::from(format!("{}.{ext}", if stem.is_empty() { "untitled".into() } else { stem }))
        }
    }
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
        let k = parse_instruments("Flute, kazoo, bass guitar").unwrap();
        assert_eq!((k[1].0.as_str(), k[1].2.as_str()), ("Kazoo", "treble"));
        assert_eq!(k[2].2, "bass");
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

    #[test]
    fn exports_every_format() {
        let p = crate::demo::project();
        let pdf = export(&p, 0, Format::Pdf, false).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
        let mid = export(&p, 0, Format::Midi, false).unwrap();
        assert_eq!(&mid[..4], b"MThd");
        assert_eq!(mid.windows(4).filter(|w| w == b"MTrk").count(), 3, "tempo track + flute + cello");
        let xml = String::from_utf8(export(&p, 0, Format::MusicXml, false).unwrap()).unwrap();
        assert!(xml.contains("<part-name>Flute</part-name>") && xml.contains("<part-name>Cello</part-name>"));
        assert_eq!(sibling(Some(Path::new("/x/song.project")), "", "pdf"), Path::new("/x/song.pdf"));
        assert_eq!(sibling(None, "Serenade (demo)", "mid"), Path::new("Serenade__demo_.mid"));
    }
}

/// Whether a file is MusicXML (by its extension).
pub fn is_musicxml(path: &std::path::Path) -> bool {
    matches!(path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref(), Some("musicxml" | "xml" | "mxl"))
}

/// Opens a project, or imports a MusicXML file as a new (unsaved) project.
/// The path is kept only for projects.
pub fn load(path: &std::path::Path) -> Result<(corinzia_model::Project, Option<std::path::PathBuf>, String), String> {
    if is_musicxml(path) {
        let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
        let p = corinzia_import::musicxml(&bytes)?;
        let staves: usize = p.movements.iter().map(|m| m.lanes.len()).sum();
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let msg = format!("Imported {name} · {staves} staves");
        return Ok((p, None, msg));
    }
    let p = corinzia_model::store::load(path).map_err(|e| e.to_string())?;
    Ok((p, Some(path.to_path_buf()), format!("Opened {}", path.display())))
}
