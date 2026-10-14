use corinzia_import::musicxml;
use corinzia_model::{BarKind, Event, Project};
use corinzia_score::score::{art, span};
use corinzia_score::Frac;

fn f(n: i64, d: i64) -> Frac {
    Frac::new(n, d)
}

fn load() -> Project {
    musicxml(include_bytes!("data/features.musicxml")).expect("imports")
}

fn names(evs: &[Event]) -> Vec<String> {
    evs.iter()
        .map(|e| {
            if e.is_rest() {
                "r".into()
            } else {
                e.heads.iter().map(|h| format!("{}{}{}", "CDEFGAB".as_bytes()[h.pitch.step as usize] as char, ["bb", "b", "", "#", "##"][(h.pitch.alter + 2) as usize], h.pitch.octave)).collect::<Vec<_>>().join("+")
            }
        })
        .collect()
}

#[test]
fn parts_staves_and_header() {
    let p = load();
    assert_eq!(p.title, "Import Test");
    assert_eq!(p.composer, "A. Tester");
    assert_eq!(p.players.len(), 2);
    assert_eq!(p.tracks.len(), 2);
    assert_eq!(p.instruments[1].name, "Piano");
    assert_eq!(p.instruments[1].engraver_name, "Pno.");
    let m = &p.movements[0];
    // Voice, piano right hand, piano left hand.
    assert_eq!(m.lanes.len(), 3);
    assert_eq!(m.lanes[1].player, m.lanes[2].player);
    // The left hand starts in the bass clef and changes to treble in bar 3.
    assert_eq!(m.lanes[2].clefs[0].clef, "bass");
    assert_eq!(m.lanes[2].clefs[0].at, Frac::ZERO);
    assert!(m.lanes[2].clefs.iter().any(|c| c.clef == "treble" && c.at == f(6, 4)));
}

#[test]
fn bars_keys_meter_tempo_and_the_pickup() {
    let m = &load().movements[0];
    // The pickup is padded to a full 3/4 bar: four bars of 3/4.
    assert_eq!(m.length, f(3, 1));
    assert_eq!((m.meter[0].num, m.meter[0].den), (3, 4));
    assert_eq!(m.keys[0].fifths, -1);
    assert_eq!(m.tempo[0].bpm, 72.0);
    assert_eq!(m.tempo[0].text, "Andante");
    // The pickup note sits on the last beat of bar 1, rests before it.
    let voice = &m.lanes[0].voices[0].events;
    assert_eq!(names(&voice[..2]), ["r", "C5"]);
    assert_eq!(voice[1].start, f(2, 4));
}

#[test]
fn notes_chords_ties_marks_and_lyrics() {
    let m = &load().movements[0];
    let v = &m.lanes[0].voices[0].events;
    assert_eq!(names(v), ["r", "C5", "F4", "G4", "A4", "Bb4", "Bb4", "r", "C5"]);
    assert_eq!(v[1].lyrics[0].text, "Oh");
    assert!(v[2].lyrics[0].hyphen && !v[3].lyrics[0].hyphen);
    assert!(v[2].spans & span::SLUR_START != 0 && v[4].spans & span::SLUR_END != 0);
    assert!(v[4].articulations & art::STACCATO != 0);
    assert_eq!(v[2].dynamic, Some(2)); // p
    assert!(v[2].spans & span::CRESC_START != 0);
    assert!(v[4].spans & span::HAIRPIN_END != 0);
    assert_eq!(v[5].dynamic, Some(5)); // f
    assert!(v[5].heads[0].tie && !v[6].heads[0].tie);
    assert!(v[6].articulations & art::FERMATA != 0);
    // Chord symbols in Corinzia's spelling.
    let chords: Vec<&str> = m.lanes[0].chords.iter().map(|c| c.text.as_str()).collect();
    assert_eq!(chords, ["F", "Bb7/D"]);
    // The piano: a chord in the right hand; two voices in the left.
    let rh = &m.lanes[1].voices[0].events;
    // (padding before the pickup, the pickup's own rest, then bar 2)
    assert_eq!(names(rh)[..3], ["r", "r", "F4+A4+C5"]);
    let lh = &m.lanes[2];
    assert_eq!(lh.voices.len(), 2);
    assert_eq!(names(&lh.voices[0].events)[..6], ["r", "r", "F2", "C3", "A3", "F3"]);
    assert_eq!(names(&lh.voices[1].events), ["F3"]);
    assert_eq!(lh.voices[1].events[0].start, f(3, 4));
}

#[test]
fn repeats_and_endings() {
    let m = &load().movements[0];
    let kinds: Vec<(Frac, BarKind)> = m.barlines.iter().map(|b| (b.at, b.kind)).collect();
    assert!(kinds.contains(&(f(3, 4), BarKind::RepeatStart)), "{kinds:?}");
    assert!(kinds.contains(&(f(9, 4), BarKind::RepeatEnd)), "{kinds:?}");
    assert!(kinds.contains(&(f(12, 4), BarKind::Final)), "{kinds:?}");
    let endings: Vec<(Frac, Frac, u8)> = m.endings.iter().map(|e| (e.from, e.to, e.passes)).collect();
    assert_eq!(endings, [(f(6, 4), f(9, 4), 1), (f(9, 4), f(12, 4), 2)]);
}

#[test]
fn compressed_mxl_and_bad_input() {
    use std::io::Write;
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut z = zip::ZipWriter::new(&mut buf);
        let o = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        z.start_file("META-INF/container.xml", o).unwrap();
        z.write_all(br#"<?xml version="1.0"?><container><rootfiles><rootfile full-path="score.xml"/></rootfiles></container>"#).unwrap();
        z.start_file("score.xml", o).unwrap();
        z.write_all(include_bytes!("data/features.musicxml")).unwrap();
        z.finish().unwrap();
    }
    let p = musicxml(buf.get_ref()).expect("mxl imports");
    assert_eq!(p.title, "Import Test");
    assert!(musicxml(b"<html/>").is_err());
    assert!(musicxml(b"not xml").is_err());
}

/// Imports and engraves every MusicXML file in $CORINZIA_MUSICXML_DIR
/// (e.g. the W3C examples); skipped when it is unset.
#[test]
fn corpus_imports_and_engraves() {
    let Ok(dir) = std::env::var("CORINZIA_MUSICXML_DIR") else { return };
    let mut files: Vec<std::path::PathBuf> = Vec::new();
    fn walk(d: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for e in std::fs::read_dir(d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else if matches!(p.extension().and_then(|x| x.to_str()), Some("musicxml" | "xml" | "mxl")) {
                out.push(p);
            }
        }
    }
    walk(std::path::Path::new(&dir), &mut files);
    files.sort();
    let (mut ok, mut failed) = (0, Vec::new());
    for f in &files {
        match musicxml(&std::fs::read(f).unwrap()) {
            Ok(p) => {
                for m in &p.movements {
                    let s = corinzia_model::notation::score(&p, m);
                    let pages = corinzia_layout::layout_pages(&s, &corinzia_layout::Options::default(), &corinzia_layout::PageSpec::default());
                    let notes: usize = m.lanes.iter().flat_map(|l| l.voices.iter()).map(|v| v.events.iter().filter(|e| !e.is_rest()).count()).sum();
                    eprintln!("{:60} {} staves, {notes} notes, {} pages", f.file_name().unwrap().to_string_lossy(), m.lanes.len(), pages.len());
                }
                ok += 1;
            }
            Err(e) => failed.push(format!("{}: {e}", f.display())),
        }
    }
    eprintln!("imported {ok} of {}", files.len());
    for f in &failed {
        eprintln!("  {f}");
    }
}
