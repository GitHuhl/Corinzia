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
    assert!(m.lanes[2].clefs.iter().any(|c| c.clef == "treble" && c.at == f(4, 4)));
}

#[test]
fn bars_keys_meter_tempo_and_the_pickup() {
    let m = &load().movements[0];
    // A quarter pickup, then three bars of 3/4.
    assert_eq!(m.length, f(10, 4));
    assert!(m.meter[0].pickup);
    assert_eq!((m.meter[0].at, m.meter[0].num, m.meter[0].den), (Frac::ZERO, 1, 4));
    assert_eq!((m.meter[1].at, m.meter[1].num, m.meter[1].den, m.meter[1].pickup), (f(1, 4), 3, 4, false));
    assert_eq!(m.keys[0].fifths, -1);
    assert_eq!(m.tempo[0].bpm, 72.0);
    assert_eq!(m.tempo[0].text, "Andante");
    let voice = &m.lanes[0].voices[0].events;
    assert_eq!(names(&voice[..1]), ["C5"]);
    assert_eq!(voice[0].start, Frac::ZERO);
}

#[test]
fn notes_chords_ties_marks_and_lyrics() {
    let m = &load().movements[0];
    let v = &m.lanes[0].voices[0].events;
    assert_eq!(names(v), ["C5", "F4", "G4", "A4", "Bb4", "Bb4", "r", "C5"]);
    assert_eq!(v[0].lyrics[0].text, "Oh");
    assert!(v[1].lyrics[0].hyphen && !v[2].lyrics[0].hyphen);
    assert!(v[1].spans & span::SLUR_START != 0 && v[3].spans & span::SLUR_END != 0);
    assert!(v[3].articulations & art::STACCATO != 0);
    assert_eq!(v[1].dynamic, Some(2)); // p
    assert!(v[1].spans & span::CRESC_START != 0);
    assert!(v[3].spans & span::HAIRPIN_END != 0);
    assert_eq!(v[4].dynamic, Some(5)); // f
    assert!(v[4].heads[0].tie && !v[5].heads[0].tie);
    assert!(v[5].articulations & art::FERMATA != 0);
    // Chord symbols in Corinzia's spelling.
    let chords: Vec<&str> = m.lanes[0].chords.iter().map(|c| c.text.as_str()).collect();
    assert_eq!(chords, ["F", "Bb7/D"]);
    // The piano: a chord in the right hand; two voices in the left.
    let rh = &m.lanes[1].voices[0].events;
    // (the pickup's rest, then bar 1)
    assert_eq!(names(rh)[..2], ["r", "F4+A4+C5"]);
    let lh = &m.lanes[2];
    assert_eq!(lh.voices.len(), 2);
    assert_eq!(names(&lh.voices[0].events)[..5], ["r", "F2", "C3", "A3", "F3"]);
    assert_eq!(names(&lh.voices[1].events), ["F3"]);
    assert_eq!(lh.voices[1].events[0].start, f(1, 4));
}

#[test]
fn repeats_and_endings() {
    let m = &load().movements[0];
    let kinds: Vec<(Frac, BarKind)> = m.barlines.iter().map(|b| (b.at, b.kind)).collect();
    assert!(kinds.contains(&(f(1, 4), BarKind::RepeatStart)), "{kinds:?}");
    assert!(kinds.contains(&(f(7, 4), BarKind::RepeatEnd)), "{kinds:?}");
    assert!(kinds.contains(&(f(10, 4), BarKind::Final)), "{kinds:?}");
    let endings: Vec<(Frac, Frac, u8)> = m.endings.iter().map(|e| (e.from, e.to, e.passes)).collect();
    assert_eq!(endings, [(f(4, 4), f(7, 4), 1), (f(7, 4), f(10, 4), 2)]);
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

#[test]
fn transposing_parts_come_in_at_concert_pitch() {
    // A B-flat clarinet written in D major: written E5 sounds D5.
    let xml = r#"<?xml version="1.0"?>
<score-partwise version="4.0"><part-list><score-part id="P1"><part-name>Clarinet in Bb</part-name></score-part></part-list>
<part id="P1"><measure number="1">
  <attributes><divisions>1</divisions><key><fifths>2</fifths></key><time><beats>4</beats><beat-type>4</beat-type></time>
    <clef><sign>G</sign><line>2</line></clef><transpose><diatonic>-1</diatonic><chromatic>-2</chromatic></transpose></attributes>
  <harmony><root><root-step>E</root-step></root><kind>minor</kind></harmony>
  <note><pitch><step>E</step><octave>5</octave></pitch><duration>4</duration><voice>1</voice><type>whole</type></note>
</measure></part></score-partwise>"#;
    let p = musicxml(xml.as_bytes()).unwrap();
    let m = &p.movements[0];
    assert_eq!(p.instruments[0].transposition, corinzia_model::Interval::new(1, 2));
    assert_eq!(m.keys[0].fifths, 0, "D major written is C major at concert pitch");
    assert_eq!(names(&m.lanes[0].voices[0].events), ["D5"]);
    assert_eq!(m.lanes[0].chords[0].text, "Dm");
}

#[test]
fn tuplets_import_and_survive_a_round_trip() {
    // Beat 1: an eighth triplet; beat 2: a sixteenth quintuplet; then a half.
    let n = |step: &str, dur: i64, ty: &str, tm: (i64, i64), mark: &str| {
        format!(
            "<note><pitch><step>{step}</step><octave>5</octave></pitch><duration>{dur}</duration><voice>1</voice><type>{ty}</type>\
             <time-modification><actual-notes>{}</actual-notes><normal-notes>{}</normal-notes></time-modification>{mark}</note>",
            tm.0, tm.1
        )
    };
    let start = "<notations><tuplet type=\"start\"/></notations>";
    let stop = "<notations><tuplet type=\"stop\"/></notations>";
    // divisions 30 per quarter: triplet eighth = 10, quintuplet sixteenth = 6.
    let notes = [
        n("C", 10, "eighth", (3, 2), start),
        n("D", 10, "eighth", (3, 2), ""),
        n("E", 10, "eighth", (3, 2), stop),
        n("F", 6, "16th", (5, 4), start),
        n("G", 6, "16th", (5, 4), ""),
        n("A", 6, "16th", (5, 4), ""),
        n("B", 6, "16th", (5, 4), ""),
        n("C", 6, "16th", (5, 4), stop),
    ]
    .join("");
    let xml = format!(
        r#"<?xml version="1.0"?>
<score-partwise version="4.0"><part-list><score-part id="P1"><part-name>Flute</part-name></score-part></part-list>
<part id="P1"><measure number="1"><attributes><divisions>30</divisions><time><beats>4</beats><beat-type>4</beat-type></time><clef><sign>G</sign><line>2</line></clef></attributes>
{notes}<note><pitch><step>D</step><octave>5</octave></pitch><duration>60</duration><voice>1</voice><type>half</type></note>
</measure></part></score-partwise>"#
    );
    let check = |p: &Project| {
        let v = &p.movements[0].lanes[0].voices[0];
        let ts: Vec<(Frac, Frac, i64, i64)> = v.tuplets.iter().map(|t| (t.start, t.length, t.actual, t.normal)).collect();
        assert_eq!(ts, [(Frac::ZERO, f(1, 4), 3, 2), (f(1, 4), f(1, 4), 5, 4)]);
        assert_eq!(v.events[3].start, f(1, 4));
        assert_eq!(v.events[3].duration, f(1, 20));
    };
    let p = musicxml(xml.as_bytes()).unwrap();
    check(&p);
    let out = corinzia_perform::musicxml(&p, &p.movements[0]).unwrap();
    assert!(out.contains("<actual-notes>5</actual-notes><normal-notes>4</normal-notes>"), "{out}");
    check(&musicxml(out.as_bytes()).unwrap());
}

#[test]
fn grace_notes_import_and_survive_a_round_trip() {
    let xml = r#"<?xml version="1.0"?>
<score-partwise version="4.0"><part-list><score-part id="P1"><part-name>Flute</part-name></score-part></part-list>
<part id="P1"><measure number="1"><attributes><divisions>1</divisions><time><beats>4</beats><beat-type>4</beat-type></time><clef><sign>G</sign><line>2</line></clef></attributes>
<note><grace slash="yes"/><pitch><step>D</step><octave>5</octave></pitch><voice>1</voice><type>eighth</type></note>
<note><pitch><step>C</step><octave>5</octave></pitch><duration>2</duration><voice>1</voice><type>half</type></note>
<note><grace/><pitch><step>E</step><octave>5</octave></pitch><voice>1</voice><type>16th</type></note>
<note><grace/><chord/><pitch><step>G</step><octave>5</octave></pitch><voice>1</voice><type>16th</type></note>
<note><grace/><pitch><step>F</step><octave>5</octave></pitch><voice>1</voice><type>16th</type></note>
<note><pitch><step>D</step><octave>5</octave></pitch><duration>2</duration><voice>1</voice><type>half</type></note>
</measure></part></score-partwise>"#;
    let check = |p: &Project| {
        let v = &p.movements[0].lanes[0].voices[0].events;
        assert_eq!(names(v), ["C5", "D5"]);
        let g0: Vec<(String, u8, bool)> = v[0].graces.iter().map(|g| (names(&[Event { heads: g.heads.clone(), ..v[0].clone() }])[0].clone(), g.base, g.slash)).collect();
        assert_eq!(g0, [("D5".to_string(), 8, true)]);
        let g1: Vec<String> = v[1].graces.iter().map(|g| names(&[Event { heads: g.heads.clone(), ..v[1].clone() }])[0].clone()).collect();
        assert_eq!(g1, ["E5+G5", "F5"]);
    };
    let p = musicxml(xml.as_bytes()).unwrap();
    check(&p);
    let out = corinzia_perform::musicxml(&p, &p.movements[0]).unwrap();
    check(&musicxml(out.as_bytes()).unwrap());
}
