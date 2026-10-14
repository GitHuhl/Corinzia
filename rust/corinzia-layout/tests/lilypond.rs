//! The engraver measured against LilyPond: the same score is written as a
//! .ly file (each notehead and rest tagged with its id), LilyPond lays it
//! out (no output) and reports every position, and the gaps between
//! successive notes are compared with ours. Skipped when `lilypond` is not
//! on PATH (it is in the dev shell and the Nix check).

use corinzia_layout::display::Item;
use corinzia_layout::font::Glyph;
use corinzia_layout::notation::score;
use corinzia_layout::{layout_line, layout_line_at, Options};
use corinzia_score::score::{Event, Score};
use corinzia_score::values::decompose;
use corinzia_score::Frac;
use std::collections::HashMap;
use std::fmt::Write;

/// LilyPond pitch name: c' = C4.
fn ly_pitch(step: u8, alter: i8, octave: i8) -> String {
    let mut s = String::from(["c", "d", "e", "f", "g", "a", "b"][step as usize]);
    for _ in 0..alter.max(0) {
        s.push_str("is");
    }
    for _ in 0..(-alter).max(0) {
        s.push_str("es");
    }
    let marks = octave as i32 - 3;
    for _ in 0..marks.max(0) {
        s.push('\'');
    }
    for _ in 0..(-marks).max(0) {
        s.push(',');
    }
    s
}

fn ly_event(e: &Event) -> String {
    let values = decompose(e.dur);
    let mut out = String::new();
    for (k, v) in values.iter().enumerate() {
        let mut d = format!("{}", v.base);
        for _ in 0..v.dots {
            d.push('.');
        }
        let tag = |id: u64| if k == 0 { format!("\\tweak details.cz-id {id} ") } else { String::new() };
        let body = if e.pitches.is_empty() {
            format!("{}r{d}", tag(e.id))
        } else if e.pitches.len() == 1 {
            let p = &e.pitches[0];
            format!("{}{}{d}", tag(p.id), ly_pitch(p.step, p.alter, p.octave))
        } else {
            let heads: Vec<String> = e.pitches.iter().map(|p| format!("{}{}", tag(p.id), ly_pitch(p.step, p.alter, p.octave))).collect();
            format!("<{}>{d}", heads.join(" "))
        };
        let tie = k + 1 < values.len() || e.pitches.iter().any(|p| p.tie);
        let _ = write!(out, "{body}{} ", if tie && !e.pitches.is_empty() { "~" } else { "" });
        if v.triplet {
            out = format!("\\tuplet 3/2 {{ {out}}} ");
        }
    }
    out
}

/// The score as LilyPond input that reports every notehead's and rest's
/// position; on one ragged line, or justified to `line_mm`.
fn to_ly(s: &Score, line_mm: Option<f32>) -> String {
    let mut ly = String::from(
        r#"\version "2.24.0"
#(define (report grob)
   (let* ((sys (ly:grob-system grob))
          (x (ly:grob-relative-coordinate grob sys X))
          (y (ly:grob-relative-coordinate grob sys Y))
          (id (ly:assoc-get 'cz-id (ly:grob-property grob 'details) 0)))
     (if (> id 0) (format #t "NH ~a ~a ~,4f ~,4f\n" id 0 x y))))
@PAPER@
\layout { \context { \Voice
  \override NoteHead.after-line-breaking = #report
  \override Rest.after-line-breaking = #report } }
"#,
    );
    let paper = match line_mm {
        None => "\\paper { indent = 0 ragged-right = ##t }".to_string(),
        Some(mm) => format!("\\paper {{ indent = 0 ragged-right = ##f line-width = {mm}\\mm }}"),
    };
    ly = ly.replace("@PAPER@", &paper);
    let key = s.keys.first().map(|k| k.fifths).unwrap_or(0);
    const MAJOR: [&str; 15] = ["ces", "ges", "des", "aes", "ees", "bes", "f", "c", "g", "d", "a", "e", "b", "fis", "cis"];
    let m = s.meter.first().map(|m| (m.num, m.den)).unwrap_or((4, 4));
    ly.push_str("<<\n");
    for p in &s.parts {
        let clef = match p.clef.as_str() {
            "bass" => "bass",
            "alto" => "alto",
            "tenor" => "tenor",
            _ => "treble",
        };
        let _ = write!(ly, "\\new Staff {{ \\clef {clef} \\key {} \\major \\time {}/{} ", MAJOR[(key + 7) as usize], m.0, m.1);
        for e in &p.voices[0].events {
            ly.push_str(&ly_event(e));
        }
        ly.push_str("}\n");
    }
    ly.push_str(">>\n");
    ly
}

/// id -> (system, x, y) from LilyPond; None without LilyPond.
fn lilypond_positions(ly: &str) -> Option<HashMap<u64, (u32, f32, f32)>> {
    let dir = std::env::temp_dir().join(format!("corinzia-ly-{}-{}", std::process::id(), ly.len()));
    std::fs::create_dir_all(&dir).ok()?;
    let file = dir.join("t.ly");
    std::fs::write(&file, ly).ok()?;
    let out = std::process::Command::new("lilypond").current_dir(&dir).args(["-dbackend=null", "-s", "t.ly"]).output().ok()?;
    std::fs::remove_dir_all(&dir).ok();
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let mut map: HashMap<u64, (u32, f32, f32)> = HashMap::new();
    for l in text.lines() {
        let f: Vec<&str> = l.split_whitespace().collect();
        if f.len() == 5 && f[0] == "NH" {
            let (id, sys, x, y): (u64, u32, f32, f32) = (f[1].parse().unwrap(), f[2].parse().unwrap(), f[3].parse().unwrap(), f[4].parse().unwrap());
            // A chord's heads share a column; keep the leftmost.
            let e = map.entry(id).or_insert((sys, x, y));
            e.1 = e.1.min(x);
        }
    }
    assert!(!map.is_empty(), "LilyPond reported nothing:\n{}\n{}", String::from_utf8_lossy(&out.stderr), ly);
    Some(map)
}

/// id -> x of our noteheads and rests.
fn our_positions(list: &corinzia_layout::DisplayList) -> HashMap<u64, f32> {
    use Glyph::*;
    let glyphs = [NoteheadBlack, NoteheadHalf, NoteheadWhole, RestWhole, RestHalf, RestQuarter, Rest8th, Rest16th, Rest32nd].map(|g| g.codepoint());
    let mut out = HashMap::new();
    for i in &list.items {
        if let Item::Glyph { glyph, at, id, .. } = i {
            if glyphs.contains(glyph) && *id != 0 {
                let x = out.entry(*id).or_insert(at.x);
                *x = at.x.min(*x);
            }
        }
    }
    out
}

/// Gaps between successive events of each staff's first voice (whole-bar
/// rests aside): (label, ours, LilyPond).
fn compare(name: &str, s: &Score) -> Option<Vec<(String, f32, f32)>> {
    let lily = lilypond_positions(&to_ly(s, None))?;
    let ours = our_positions(&layout_line(s, &Options::default()));
    let bar = s.meter.first().map(|m| Frac::new(m.num, m.den)).unwrap_or(Frac::int(1));
    let mut rows = Vec::new();
    for p in &s.parts {
        let events: Vec<&Event> = p.voices[0].events.iter().filter(|e| !(e.pitches.is_empty() && e.dur == bar)).collect();
        let id = |e: &Event| e.pitches.first().map_or(e.id, |p| p.id);
        for w in events.windows(2) {
            let (a, b) = (id(w[0]), id(w[1]));
            let (Some(la), Some(lb), Some(oa), Some(ob)) = (lily.get(&a), lily.get(&b), ours.get(&a), ours.get(&b)) else { continue };
            if la.0 != lb.0 {
                continue; // across a line break
            }
            let d = w[0].dur;
            let kind = if w[0].pitches.is_empty() { "r" } else { "" };
            rows.push((format!("{kind}{}/{}", d.num, d.den), ob - oa, lb.1 - la.1));
        }
    }
    eprintln!("{name}:");
    for (label, o, l) in &rows {
        eprintln!("  after {label:>6}: ours {o:6.2}  lilypond {l:6.2}  ratio {:.2}", o / l);
    }
    Some(rows)
}

fn corpus() -> Vec<(&'static str, Score)> {
    vec![
        ("quarters and eighths", score(&[("treble", &["c''4 d''8 e''8 f''2 | g'4 a'4 b'4 c''4"])], (4, 4), 0)),
        ("sixteenths", score(&[("treble", &["c''16 d'' e'' f'' g''8 a'' b''4 c'''4 | c''2 c''2"])], (4, 4), 0)),
        ("dotted", score(&[("treble", &["c''4. d''8 e''4. f''8 | g''2. r4"])], (4, 4), 0)),
        ("long notes", score(&[("treble", &["c''1 | d''2 e''2 | f''4 f''4 f''2"])], (4, 4), 0)),
        ("accidentals", score(&[("treble", &["cis''4 des''4 eis''8 fes''8 gis''4 | aes'4 bes'4 c''2"])], (4, 4), 0)),
        ("stem directions", score(&[("treble", &["c''4 c'4 c''4 g'4 | f'4 b'4 a''4 a'4 | c'4 g'4 c''4 e''4 | e''4 c''4 g'4 c'4"])], (4, 4), 0)),
        ("beams", score(&[("treble", &["c'8 g'8 c'8 e''8 a''8 e''8 g'8 c'8 | c''8 d''8 e''8 f''8 g''2"])], (4, 4), 0)),
        ("lone notes", score(&[("treble", &["c''4 c''4 c''4 c''4 | c''1 | g''2. a''4 | c'2 c'2"])], (4, 4), 0)),
        ("rests", score(&[("treble", &["c''4 r4 d''8 r8 r4 | r2 e''4 r4"])], (4, 4), 0)),
        ("chords", score(&[("treble", &["<c'' e'' g''>4 <d'' f''>4 <e' g' c''>2 | <f' a' c''>4 <g' b' d''>4 <c' e' g' c''>2"])], (4, 4), 0)),
        ("bass and 3/4", score(&[("bass", &["c4 e8 g8 c'4 | g,2. | f4 a4 c'4"])], (3, 4), 0)),
        ("key signature", score(&[("treble", &["fis'8 g'8 a'4 b'4 cis''4 | d''2 c''2"])], (4, 4), 2)),
        ("two staves", score(&[("treble", &["e''4 d''8 c''8 b'4 c''4 | g'2 g'2"]), ("bass", &["c2 g,2 | c8 d8 e8 f8 g4 c4"])], (4, 4), 0)),
    ]
}

/// Justified lines: LilyPond hands the extra room to the springs by their
/// stretchability. Each gap's share of the extra room is compared, which
/// leaves out the different widths of clefs and margins.
#[test]
fn justification_against_lilypond() {
    let pieces = [
        ("mixed", score(&[("treble", &["c''8 c''8 c''4 c''2 | c''4 c''4 c''4 c''4"])], (4, 4), 0)),
        ("dotted", score(&[("treble", &["c''4. d''8 e''4. f''8 | g''2. r4 | c''16 d'' e'' f'' g''4 a''2"])], (4, 4), 0)),
    ];
    for (name, s) in pieces {
        let Some(lr) = lilypond_positions(&to_ly(&s, None)) else {
            eprintln!("lilypond not found; skipping");
            return;
        };
        let lj = lilypond_positions(&to_ly(&s, Some(110.0))).unwrap();
        let natural = layout_line(&s, &Options::default());
        let or = our_positions(&natural);
        let oj = our_positions(&layout_line_at(&s, &Options::default(), natural.width * 1.5));
        let ids: Vec<u64> = s.parts[0].voices[0].events.iter().map(|e| e.pitches.first().map_or(e.id, |p| p.id)).collect();
        let shares = |r: &dyn Fn(u64) -> f32, j: &dyn Fn(u64) -> f32| -> Vec<f32> {
            let extra: Vec<f32> = ids.windows(2).map(|w| (j(w[1]) - j(w[0])) - (r(w[1]) - r(w[0]))).collect();
            let total: f32 = extra.iter().sum();
            extra.iter().map(|e| e / total).collect()
        };
        let lily = shares(&|id| lr[&id].1, &|id| lj[&id].1);
        let ours = shares(&|id| or[&id], &|id| oj[&id]);
        eprintln!("{name}: shares of the extra room");
        for (l, o) in lily.iter().zip(&ours) {
            eprintln!("  ours {o:.3}  lilypond {l:.3}");
            assert!((l - o).abs() < 0.01, "{name}: stretch differs from LilyPond's");
        }
    }
}

#[test]
fn spacing_against_lilypond() {
    let mut all = Vec::new();
    for (name, s) in corpus() {
        let Some(rows) = compare(name, &s) else {
            eprintln!("lilypond not found; skipping");
            return;
        };
        all.extend(rows);
    }
    let err = |(_, o, l): &(String, f32, f32)| (o / l - 1.0).abs();
    let worst = all.iter().map(err).fold(0.0f32, f32::max);
    let mean = all.iter().map(err).sum::<f32>() / all.len() as f32;
    eprintln!("spacing vs LilyPond: mean error {:.1} %, worst {:.1} % over {} gaps", mean * 100.0, worst * 100.0, all.len());
    assert!(mean < 0.015 && worst < 0.10, "spacing drifted from LilyPond's");
}

/// Bars that start each system, LilyPond's and ours, for one line width.
fn breaks(s: &Score, line_sp: f32) -> Option<(Vec<i64>, Vec<i64>)> {
    // LilyPond's default staff space: 20 pt staff / 4 = 1.7638 mm.
    let lily = lilypond_positions(&to_ly(s, Some(line_sp * 1.7638)).replace("ragged-right = ##f", "ragged-right = ##f ragged-last = ##f"))?;
    let bar = s.meter.first().map(|m| Frac::new(m.num, m.den)).unwrap_or(Frac::int(1));
    let bar_of = |t: Frac| (t.num * bar.den) / (t.den * bar.num);
    // A system starts where x falls back (LilyPond's System rank is not
    // reachable from Scheme here).
    let mut ly: Vec<i64> = Vec::new();
    let mut last_x = f32::INFINITY;
    for e in &s.parts[0].voices[0].events {
        let id = e.pitches.first().map_or(e.id, |p| p.id);
        if let Some(&(_, x, _)) = lily.get(&id) {
            if x < last_x {
                ly.push(bar_of(e.start));
            }
            last_x = x;
        }
    }
    let opt = Options { names: false, ..Options::default() };
    let spec = corinzia_layout::PageSpec { width: line_sp + 2.0 * 8.0 - opt.margin, height: 10_000.0, ..Default::default() };
    let pages = corinzia_layout::layout_pages(s, &opt, &spec);
    let mut ours: Vec<i64> = Vec::new();
    for p in &pages {
        let systems = p.columns.iter().map(|c| c.system).max().map_or(0, |m| m + 1);
        for k in 0..systems {
            let q = p.columns.iter().filter(|c| c.system == k).map(|c| c.quarter).fold(f64::INFINITY, f64::min);
            ours.push((q / 4.0 * bar.den as f64 / bar.num as f64).round() as i64);
        }
    }
    Some((ly, ours))
}

#[test]
fn line_breaks_against_lilypond() {
    let tune = "c''4 d''8 e''8 f''4 g''4 | a''2 g''4 f''4 | e''8 d''8 c''8 d''8 e''2 | d''1 | \
                e''4. f''8 g''4 e''4 | c''2 r4 g'4 | a'8 b'8 c''8 d''8 e''4 c''4 | g'1 | \
                c''16 d'' e'' f'' g''8 e''8 c''2 | f''4 a''4 g''4 f''4 | e''2 d''2 | c''1";
    let waltz = "g'4 c''4 e''4 | g''2 e''4 | f''4 d''4 b'4 | c''2. | e''8 f''8 g''4 g''4 | a''4. g''8 f''4 | \
                 e''4 d''4 c''4 | d''2 r4 | b'4 d''4 g''4 | f''2 d''4 | c''8 b'8 a'4 b'4 | c''2.";
    let mut a = String::new();
    let mut b = String::new();
    for _ in 0..3 {
        a.push_str(tune);
        a.push_str(" | ");
        b.push_str(waltz);
        b.push_str(" | ");
    }
    let pieces = [("4/4 tune", score(&[("treble", &[a.as_str()])], (4, 4), 0)), ("waltz", score(&[("treble", &[b.as_str()])], (3, 4), 0))];
    let (mut same, mut total) = (0, 0);
    for (name, s) in &pieces {
        for w in [45.0, 50.0, 55.0, 60.0, 64.0, 68.0, 72.0, 75.0, 78.0, 82.0, 86.0, 90.0, 95.0, 100.0, 105.0] {
            let Some((ly, ours)) = breaks(s, w) else {
                eprintln!("lilypond not found; skipping");
                return;
            };
            total += 1;
            if ly == ours {
                same += 1;
            } else {
                eprintln!("{name}, line {w}: lilypond {ly:?}\n                ours     {ours:?}");
            }
        }
    }
    eprintln!("same system breaks as LilyPond at {same} of {total} line widths");
    assert!(same * 4 >= total * 3, "line breaking drifted from LilyPond's: {same} of {total}");
}
