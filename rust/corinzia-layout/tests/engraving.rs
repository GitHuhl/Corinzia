//! Engraving checks: positions against the rules (numeric), plus PNG previews
//! written to target/engraving/ for eyeballing.

use corinzia_layout::display::Item;
use corinzia_layout::font::Glyph;
use corinzia_layout::notation::score;
use corinzia_layout::{layout_line, Options};

fn glyphs(list: &corinzia_layout::DisplayList, g: Glyph) -> Vec<(f32, f32, u64)> {
    list.items
        .iter()
        .filter_map(|i| match i {
            Item::Glyph { glyph, at, id } if *glyph == g.codepoint() => Some((at.x, at.y, *id)),
            _ => None,
        })
        .collect()
}

fn preview(name: &str, list: &corinzia_layout::DisplayList) {
    #[cfg(feature = "raster")]
    {
        let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("engraving");
        std::fs::create_dir_all(&dir).unwrap();
        let pm = corinzia_layout::raster::render(list, 12.0, 2.0);
        pm.save_png(dir.join(format!("{name}.png"))).unwrap();
        // Ink exists.
        assert!(pm.pixels().iter().filter(|p| p.red() < 128).count() > 500, "{name}: nothing drawn");
    }
}

#[test]
fn noteheads_sit_on_their_lines_and_spaces() {
    // Treble: E4 bottom line, G4 second line, C4 one ledger below, A5 one ledger above.
    let s = score(&[("treble", &["e'4 g' c' a''"])], (4, 4), 0);
    let l = layout_line(&s, &Options::default());
    let heads = glyphs(&l, Glyph::NoteheadBlack);
    let ys: Vec<f32> = heads.iter().map(|h| h.1).collect();
    assert_eq!(ys, vec![4.0, 3.0, 5.0, -1.0]);
    // Ledger lines: C4 one at y=5, A5 one at y=-1.
    let ledgers: Vec<f32> = l
        .items
        .iter()
        .filter_map(|i| match i {
            Item::Line { from, to, width, .. } if (*width - 0.16).abs() < 1e-6 && from.y == to.y => Some(from.y),
            _ => None,
        })
        .collect();
    assert_eq!(ledgers, vec![5.0, -1.0]);
    // Bass clef: D3 on the middle line.
    let s = score(&[("bass", &["d1"])], (4, 4), 0);
    let l = layout_line(&s, &Options::default());
    assert_eq!(glyphs(&l, Glyph::NoteheadWhole)[0].1, 2.0);
    preview("positions", &l);
}

#[test]
fn stems_follow_the_middle_line_rule() {
    let s = score(&[("treble", &["a'4 b' c'' <g' d''>"])], (4, 4), 0);
    let l = layout_line(&s, &Options::default());
    let stems: Vec<(f32, f32)> = l
        .items
        .iter()
        .filter_map(|i| match i {
            Item::Line { from, to, width, .. } if (*width - 0.12).abs() < 1e-6 => Some((from.y, to.y)),
            _ => None,
        })
        .collect();
    assert_eq!(stems.len(), 4);
    assert!(stems[0].1 < stems[0].0, "A4: stem up");
    assert!(stems[1].1 > stems[1].0, "B4 on the middle line: stem down");
    assert!(stems[2].1 > stems[2].0, "C5: stem down");
    assert!(stems[3].1 > stems[3].0, "G4+D5: balanced around the middle line: down");
    // Length: an octave (3.5 spaces) from the head.
    assert!((stems[0].0 - stems[0].1 - 3.5).abs() < 0.2);
}

#[test]
fn accidentals_follow_key_and_bar() {
    // G major: F# needs nothing, F natural needs a natural, a second F in the bar nothing,
    // and the next bar's F# needs nothing again... until it is F natural.
    let s = score(&[("treble", &["fis'4 f' f' fis' fis'1"])], (4, 4), 1);
    let l = layout_line(&s, &Options::default());
    assert_eq!(glyphs(&l, Glyph::Natural).len(), 1);
    assert_eq!(glyphs(&l, Glyph::Sharp).len(), 1 + 1); // key signature + the cancelling sharp in bar 1
    preview("accidentals", &l);
}

#[test]
fn eighths_are_beamed_by_beat() {
    let s = score(&[("treble", &["c''8 d'' e'' f'' g'4 a'8 b'"])], (4, 4), 0);
    let l = layout_line(&s, &Options::default());
    let beams = l.items.iter().filter(|i| matches!(i, Item::Quad { .. })).count();
    assert_eq!(beams, 3, "two pairs in beats 1-2 and one in beat 4");
    assert!(glyphs(&l, Glyph::Flag8thUp).is_empty() && glyphs(&l, Glyph::Flag8thDown).is_empty());
    preview("beams", &l);
}

#[test]
fn notes_over_the_barline_are_split_and_tied() {
    let s = score(&[("treble", &["g'2. a'2 b'4"])], (4, 4), 0);
    let l = layout_line(&s, &Options::default());
    // a'2 from beat 4: a'4 ~ a'4 across the barline.
    assert_eq!(glyphs(&l, Glyph::NoteheadBlack).len(), 3);
    assert_eq!(l.items.iter().filter(|i| matches!(i, Item::Crescent { .. })).count(), 1);
    preview("ties", &l);
}

#[test]
fn spacing_is_monotone_and_aligned_across_staves() {
    let s = score(&[("treble", &["c''8 d'' e'' f'' g''2"]), ("bass", &["c4 g, c2"])], (4, 4), 0);
    let l = layout_line(&s, &Options::default());
    let xs: Vec<f32> = l.columns.iter().map(|c| c.x).collect();
    assert!(xs.windows(2).all(|w| w[1] > w[0]), "{xs:?}");
    // The bass G2 (beat 2) aligns with the treble E5 (beat 2).
    let col = |q: f64| l.columns.iter().find(|c| (c.quarter - q).abs() < 1e-9).unwrap().x;
    let heads = glyphs(&l, Glyph::NoteheadBlack);
    assert!(heads.iter().any(|h| (h.0 - col(1.0)).abs() < 1e-4 && h.1 > 10.0));
    assert!(heads.iter().any(|h| (h.0 - col(1.0)).abs() < 1e-4 && h.1 < 4.0));
    preview("two_staves", &l);
}

#[test]
fn a_full_example_renders() {
    let s = score(
        &[
            ("treble", &["c''8\\p d'' e'' f''-. g''4-> <e'' g'' c'''>\\arpeggio a'16 b' c'' d'' e''8. f''16 g''4\\fermata r4 fis''2~ fis''8 g''4. c'''1\\trill"]),
            ("treble", &["e'2 d'4 r c'1 a'2 g' e'1"]),
            ("bass", &["c2 g,4 bes, f,1\\f c2. r4 c,1"]),
        ],
        (4, 4),
        -1,
    );
    let l = layout_line(&s, &Options::default());
    preview("example", &l);
}

#[test]
fn polyphony_keeps_voices_apart() {
    // Voice 1 staccato above (stem side), voice 2's below; two dynamics stacked.
    let s = score(&[("treble", &["c''4-.\\mp d''-. e'' f''", "e'4-.\\p f' g' a'"])], (4, 4), 0);
    let l = layout_line(&s, &Options::default());
    let above = glyphs(&l, Glyph::StaccatoAbove);
    let below = glyphs(&l, Glyph::StaccatoBelow);
    assert_eq!(above.len(), 2);
    assert_eq!(below.len(), 1);
    assert!(above.iter().all(|a| a.1 < 0.0), "voice 1 staccatos above the staff: {above:?}");
    let p = glyphs(&l, Glyph::DynamicP);
    let m = glyphs(&l, Glyph::DynamicM);
    assert_eq!(m.len(), 1);
    assert!(p.iter().any(|g| g.1 > m[0].1 + 1.5), "the second dynamic sits below the first");
    preview("polyphony", &l);
}
