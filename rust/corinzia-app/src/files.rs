//! New projects from an instrument list, and export (PDF from the native
//! engraver, MIDI and MusicXML from the performance compiler). Everything
//! here is synchronous and UI-free; the app runs dialogs around it.

use corinzia_layout::{layout_pages, Options, PageSpec};
use corinzia_model::Project;
use std::path::Path;


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





#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Pdf,
    /// Every player's part, one after another, in one PDF.
    Parts,
    Midi,
    MusicXml,
    /// The movement as it plays: a stereo WAV file (rendered offline, by
    /// the app, not `export`).
    Audio,
    /// Each track on its own (through its effects, the buses and the
    /// master), a WAV file each beside the name chosen.
    Stems,
}
impl Format {
    pub fn extension(self) -> &'static str {
        match self {
            Format::Pdf | Format::Parts => "pdf",
            Format::Midi => "mid",
            Format::MusicXml => "musicxml",
            Format::Audio | Format::Stems => "wav",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Format::Pdf => "PDF",
            Format::Parts => "Parts (PDF)",
            Format::Midi => "MIDI file",
            Format::MusicXml => "MusicXML",
            Format::Audio => "Audio (WAV)",
            Format::Stems => "Audio stems (WAV)",
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
        Format::Audio | Format::Stems => Err("Audio is rendered by the app's bounce".into()),
    }
}

/// A stem's file: beside `path`, its name and the track's ("Take -
/// Violin I.wav"), characters a file name cannot hold left out.
pub fn stem_path(path: &std::path::Path, track: &str) -> std::path::PathBuf {
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Stems".into());
    let clean: String = track.chars().filter(|c| !matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')).collect();
    path.with_file_name(format!("{stem} - {}.wav", clean.trim()))
}

/// The export format a file name asks for, by its extension.
pub fn format_of(path: &std::path::Path) -> Option<Format> {
    match path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
        Some("pdf") => Some(Format::Pdf),
        Some("mid" | "midi") => Some(Format::Midi),
        Some("musicxml" | "xml") => Some(Format::MusicXml),
        Some("wav") => Some(Format::Audio),
        _ => None,
    }
}

/// A Byzantine chant as a PDF.
pub fn chant_pdf(chant: &corinzia_byz::Chant) -> Vec<u8> {
    if corinzia_layout::display::text_fonts().0.is_none() {
        crate::load_text_fonts();
    }
    let pages = corinzia_layout::byz::engrave(chant, &corinzia_layout::byz::ByzPage::default());
    corinzia_layout::pdf::to_pdf(&pages, if chant.title.is_empty() { "Chant" } else { &chant.title })
}

/// The engraver's options and page for a layout (a part's: multi-bar
/// rests, no staff names).
pub fn page_setup(l: &corinzia_model::Layout, part: bool) -> (Options, PageSpec) {
    let (w, h) = l.page_spaces();
    let space = l.space_mm();
    let opt = Options { staff_distance: l.staff_distance as f32, spacing: l.spacing as f32, multirest: part, names: !part, ..Options::default() };
    let spec = PageSpec {
        width: w as f32,
        height: h as f32,
        margin_x: (l.margin_x_mm / space) as f32,
        margin_y: (l.margin_y_mm / space) as f32,
        system_gap: l.system_gap as f32,
        first_page: 1,
        bars_per_system: l.bars_per_system as usize,
        page_numbers: l.page_numbers,
        space_pt: (space * 72.0 / 25.4) as f32,
    };
    (opt, spec)
}

/// Every movement's pages, as printed: the first under the work's title
/// and composer, each later one from a new page under its own title, page
/// numbers running on.
pub fn work_pages(project: &Project, transposed: bool) -> Vec<corinzia_layout::DisplayList> {
    let mut pages = Vec::new();
    for (i, m) in project.movements.iter().enumerate() {
        let mut score = corinzia_perform::score_as(project, m, transposed);
        if project.layout.condense {
            corinzia_model::notation::condense(&mut score);
        }
        if i > 0 {
            score.work_title.clear();
            score.composer.clear();
            score.subtitle.clear();
            score.lyricist.clear();
            score.arranger.clear();
            score.copyright.clear();
        }
        let (opt, spec) = page_setup(&project.layout, false);
        let spec = PageSpec { first_page: pages.len() + 1, ..spec };
        pages.extend(layout_pages(&score, &opt, &spec));
    }
    pages
}

/// Every player's part (all movements, as written, with multi-bar rests),
/// each from a new page, its page numbers from 1.
pub fn parts_pages(project: &Project) -> Vec<corinzia_layout::DisplayList> {
    let (opt, base) = page_setup(&project.part_layout, true);
    let mut pages = Vec::new();
    for player in &project.players {
        let mut part: Vec<corinzia_layout::DisplayList> = Vec::new();
        for (i, m) in project.movements.iter().enumerate() {
            if !m.lanes.iter().any(|l| l.player == player.id) {
                continue;
            }
            let mut score = corinzia_perform::part_score(project, m, player.id);
            if i > 0 {
                score.work_title.clear();
                score.composer.clear();
                score.subtitle.clear();
                score.lyricist.clear();
                score.arranger.clear();
                score.copyright.clear();
            }
            let spec = PageSpec { first_page: part.len() + 1, ..base.clone() };
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pdf_pages_follow_the_layout() {
        let mut p = crate::editor::instruments::new_project("T", "Flute, Viola", 8).unwrap();
        let media = |bytes: &[u8]| -> Vec<String> {
            let text = String::from_utf8_lossy(bytes);
            text.match_indices("/MediaBox [").map(|(i, _)| text[i + 11..].split(']').next().unwrap().to_string()).collect()
        };
        let a4 = media(&export(&p, 0, Format::Pdf, false).unwrap());
        assert!(a4[0].starts_with("0 0 595.2"), "{a4:?}");
        p.layout = corinzia_model::Layout { page_width_mm: 215.9, page_height_mm: 279.4, landscape: true, staff_mm: 7.0, ..Default::default() };
        let letter = media(&export(&p, 0, Format::Pdf, false).unwrap());
        let dims: Vec<f32> = letter[0].split_whitespace().map(|v| v.parse().unwrap()).collect();
        assert!((dims[2] - 792.0).abs() < 0.5 && (dims[3] - 612.0).abs() < 0.5, "{letter:?}");
        // Parts follow their own layout (A4 by default).
        let parts = media(&export(&p, 0, Format::Parts, false).unwrap());
        assert!(parts[0].starts_with("0 0 595.2"), "{parts:?}");
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
