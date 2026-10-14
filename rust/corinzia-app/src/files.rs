//! New projects from an instrument list, and export (PDF from the native
//! engraver, MIDI and MusicXML from the performance compiler). Everything
//! here is synchronous and UI-free; the app runs dialogs around it.

use corinzia_layout::{layout_pages, Options, PageSpec};
use corinzia_model::Project;
use std::path::Path;

/// Known instruments: names (lower case, with aliases), display name,
/// abbreviation, clef. Scores are written at concert pitch.
const INSTRUMENTS: &[(&[&str], &str, &str, &str)] = &[
    (&["piccolo"], "Piccolo", "Picc.", "treble"),
    (&["flute", "fl"], "Flute", "Fl.", "treble"),
    (&["oboe", "ob"], "Oboe", "Ob.", "treble"),
    (&["cor anglais", "english horn"], "Cor anglais", "C. a.", "treble"),
    (&["clarinet", "cl"], "Clarinet", "Cl.", "treble"),
    (&["bass clarinet"], "Bass clarinet", "B. Cl.", "bass"),
    (&["bassoon", "bsn"], "Bassoon", "Bsn.", "bass"),
    (&["contrabassoon"], "Contrabassoon", "Cbsn.", "bass_8"),
    (&["saxophone", "alto sax", "sax"], "Alto saxophone", "A. Sax.", "treble"),
    (&["horn", "french horn", "hn"], "Horn", "Hn.", "treble"),
    (&["trumpet", "tpt"], "Trumpet", "Tpt.", "treble"),
    (&["trombone", "tbn"], "Trombone", "Tbn.", "bass"),
    (&["bass trombone"], "Bass trombone", "B. Tbn.", "bass"),
    (&["tuba"], "Tuba", "Tba.", "bass"),
    (&["timpani", "timp"], "Timpani", "Timp.", "bass"),
    (&["percussion", "perc"], "Percussion", "Perc.", "percussion"),
    (&["harp"], "Harp", "Hp.", "treble"),
    (&["piano", "pno", "piano rh"], "Piano", "Pno.", "treble"),
    (&["piano lh"], "Piano (LH)", "Pno.", "bass"),
    (&["celesta"], "Celesta", "Cel.", "treble"),
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

/// A comma-separated instrument list ("Flute, 2 Violin, Cello") ->
/// (name, abbreviation, clef) per staff. A leading count repeats an
/// instrument, numbered. Unknown names are an error naming them.
pub fn parse_instruments(text: &str) -> Result<Vec<(String, String, String)>, String> {
    let mut out = Vec::new();
    let mut unknown = Vec::new();
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
            None => unknown.push(name.to_string()),
        }
    }
    if !unknown.is_empty() {
        return Err(format!("Unknown instrument{}: {}", if unknown.len() == 1 { "" } else { "s" }, unknown.join(", ")));
    }
    if out.is_empty() {
        return Err("Name at least one instrument, e.g. \"Flute, Violin, Cello\"".into());
    }
    Ok(out)
}

pub fn new_project(title: &str, instruments: &str, bars: i64) -> Result<Project, String> {
    let list = parse_instruments(instruments)?;
    let refs: Vec<(&str, &str, &str)> = list.iter().map(|(a, b, c)| (a.as_str(), b.as_str(), c.as_str())).collect();
    Ok(crate::demo::blank(title, &refs, bars))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Pdf,
    Midi,
    MusicXml,
}
impl Format {
    pub fn extension(self) -> &'static str {
        match self {
            Format::Pdf => "pdf",
            Format::Midi => "mid",
            Format::MusicXml => "musicxml",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Format::Pdf => "PDF",
            Format::Midi => "MIDI file",
            Format::MusicXml => "MusicXML",
        }
    }
}

/// The current movement in a format, as bytes.
pub fn export(project: &Project, movement: usize, format: Format) -> Result<Vec<u8>, String> {
    let m = project.movements.get(movement).ok_or("No movement to export")?;
    match format {
        Format::Pdf => {
            // Text is written as outlines of the text font: make sure it is loaded.
            if corinzia_layout::display::text_fonts().0.is_none() {
                crate::load_text_fonts();
            }
            let score = corinzia_model::notation::score(project, m);
            let pages = layout_pages(&score, &Options::default(), &PageSpec::default());
            let title = if project.title.is_empty() { m.title.as_str() } else { project.title.as_str() };
            Ok(corinzia_layout::pdf::to_pdf(&pages, title))
        }
        Format::Midi => corinzia_perform::midi(project, m),
        Format::MusicXml => corinzia_perform::musicxml(project, m).map(String::into_bytes),
    }
}

pub fn export_to(project: &Project, movement: usize, format: Format, path: &Path) -> Result<(), String> {
    let bytes = export(project, movement, format)?;
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
        assert_eq!(parse_instruments("Flute, kazoo").unwrap_err(), "Unknown instrument: kazoo");
        assert!(parse_instruments(" , ").is_err());
        let p = new_project("Quartet", "violin, violin, viola, cello", 4).unwrap();
        assert_eq!(p.movements[0].lanes.len(), 4);
        assert_eq!(p.instruments[2].clef, "alto");
    }

    #[test]
    fn exports_every_format() {
        let p = crate::demo::project();
        let pdf = export(&p, 0, Format::Pdf).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
        let mid = export(&p, 0, Format::Midi).unwrap();
        assert_eq!(&mid[..4], b"MThd");
        assert_eq!(mid.windows(4).filter(|w| w == b"MTrk").count(), 3, "tempo track + flute + cello");
        let xml = String::from_utf8(export(&p, 0, Format::MusicXml).unwrap()).unwrap();
        assert!(xml.contains("<part-name>Flute</part-name>") && xml.contains("<part-name>Cello</part-name>"));
        assert_eq!(sibling(Some(Path::new("/x/song.project")), "", "pdf"), Path::new("/x/song.pdf"));
        assert_eq!(sibling(None, "Serenade (demo)", "mid"), Path::new("Serenade__demo_.mid"));
    }
}
