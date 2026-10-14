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
            Item::Glyph { glyph, at, id, .. } if *glyph == g.codepoint() => Some((at.x, at.y, *id)),
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

#[test]
fn outer_marks_avoid_each_other() {
    use corinzia_score::score::{ClefChange, Ending, Tempo, Text};
    use corinzia_score::Frac;
    let mut s = score(
        &[
            ("treble", &["c''8(\\f\\cr d'' e'' f'' g''4 a'')\\ff b''8\\decr a'' g'' f'' e''4 d''\\p c''2\\fermata e''4*2/3 f'' g'' c'''1\\trill"]),
            ("bass", &["c4( e g c') c2 g, c1 f,4*2/3 g, a, c2 c1"]),
        ],
        (4, 4),
        0,
    );
    s.tempo = vec![Tempo { at: Frac::ZERO, bpm: 132.0, text: "Allegro".into() }];
    s.marks = vec![Frac::int(1)];
    s.endings = vec![Ending { from: Frac::int(1), to: Frac::int(2), numbers: vec![1] }];
    s.barlines = vec![corinzia_score::score::Barline { at: Frac::int(2), kind: corinzia_score::score::BarKind::RepeatEnd }];
    s.parts[0].name = "Flute".into();
    s.parts[1].name = "Cello".into();
    s.parts[0].texts = vec![Text { at: Frac::ZERO, text: "dolce".into(), technique: false }, Text { at: Frac::int(1), text: "flz.".into(), technique: true }];
    s.parts[1].clefs = vec![ClefChange { at: Frac::new(3, 2), clef: "tenor".into() }];
    let l = layout_line(&s, &Options::default());
    preview("outer_marks", &l);

    // No two outer marks overlap (texts, dynamics, ornaments, numbers, brackets of the top staff).
    let font = corinzia_layout::font::Font::bravura();
    let outer: Vec<[f32; 4]> = l
        .items
        .iter()
        .filter(|i| match i {
            Item::Text { anchor, .. } => *anchor != 2,
            Item::Glyph { glyph, .. } => [Glyph::DynamicF, Glyph::DynamicP, Glyph::FermataAbove, Glyph::Trill, Glyph::TupletDigit(3), Glyph::MetNoteQuarterUp]
                .iter()
                .any(|g| g.codepoint() == *glyph),
            _ => false,
        })
        .map(|i| i.bounds(font))
        .collect();
    for (a, ra) in outer.iter().enumerate() {
        for rb in &outer[a + 1..] {
            let overlap = ra[0] < rb[2] - 0.05 && rb[0] < ra[2] - 0.05 && ra[1] < rb[3] - 0.05 && rb[1] < ra[3] - 0.05;
            assert!(!overlap, "outer marks overlap: {ra:?} {rb:?}");
        }
    }
    // Two slurs, one hairpin of each kind (2 strokes each), tuplet numbers, the clef change (scaled).
    let curves = l.items.iter().filter(|i| matches!(i, Item::Crescent { .. })).count();
    assert!(curves >= 2, "slurs drawn: {curves}");
    assert_eq!(glyphs(&l, Glyph::TupletDigit(3)).len(), 2);
    assert!(l.items.iter().any(|i| matches!(i, Item::Glyph { scale, .. } if (*scale - 0.8).abs() < 1e-6)));
    assert!(l.items.iter().any(|i| matches!(i, Item::Text { text, .. } if text == "Allegro (")));
    assert!(l.items.iter().any(|i| matches!(i, Item::Text { text, bold: true, .. } if text == "A")));
    assert!(l.items.iter().any(|i| matches!(i, Item::Text { text, .. } if text == "1.")));
    // The staves start after the instrument names.
    assert!(l.staves[0].left > 6.0);
}

#[test]
fn pages_break_into_justified_systems() {
    use corinzia_layout::{layout_pages, PageSpec};
    // 24 bars of mixed rhythms on two staves, a tie over every 6th barline.
    let bar = "c''8 d'' e'' f'' g''4 a''8 b'' c'''4 g''2. a''4~ a''1 g''8 f'' e'' d'' c''2";
    let low = "c2 g, c1 e4 g c' e c'1 g,1 c2 g,";
    let treble = std::iter::repeat_n(bar, 4).collect::<Vec<_>>().join(" ");
    let bass = std::iter::repeat_n(low, 4).collect::<Vec<_>>().join(" ");
    let mut s = score(&[("treble", &[treble.as_str()]), ("bass", &[bass.as_str()])], (4, 4), 2);
    s.title = "Étude".into();
    s.parts[0].name = "Violin".into();
    s.parts[0].abbr = "Vln.".into();
    s.parts[1].name = "Cello".into();
    s.parts[1].abbr = "Vc.".into();
    let spec = PageSpec::default();
    let pages = layout_pages(&s, &Options::default(), &spec);
    assert!(!pages.is_empty());
    let mut systems = 0;
    for (pi, p) in pages.iter().enumerate() {
        preview(&format!("page{}", pi + 1), p);
        // Staves come in pairs (one system = 2 staves); every staff ends at the right margin
        // except possibly the last system's.
        let tops: Vec<f32> = p.staves.iter().filter(|st| st.part == 0).map(|st| st.top).collect();
        systems += tops.len();
        for st in &p.staves {
            assert!(st.right <= spec.width - spec.margin_x + 0.01, "staff past the margin: {}", st.right);
            assert!(st.left >= spec.margin_x - 0.01);
        }
        // Systems don't overlap: consecutive tops are at least a system apart.
        for w in tops.windows(2) {
            assert!(w[1] - w[0] > 16.0, "systems too close: {w:?}");
        }
        assert!(p.items.iter().all(|i| { let r = i.bounds(corinzia_layout::font::Font::bravura()); r[3] <= spec.height + 0.01 && r[1] >= -0.01 }));
    }
    assert!(systems >= 3, "24 bars break into several systems ({systems})");
    // Justified: all but the last system end at the right margin.
    let ends: Vec<f32> = pages.iter().flat_map(|p| p.staves.iter().filter(|st| st.part == 0).map(|st| st.right)).collect();
    for e in &ends[..ends.len() - 1] {
        assert!((e - (spec.width - spec.margin_x)).abs() < 0.6, "system not justified: {e}");
    }
    // Abbreviated names after the first system.
    let texts: Vec<&str> = pages.iter().flat_map(|p| p.items.iter()).filter_map(|i| match i { Item::Text { text, .. } => Some(text.as_str()), _ => None }).collect();
    assert!(texts.contains(&"Violin") && texts.contains(&"Vln.") && texts.contains(&"Étude"));
}

#[test]
fn ties_cross_system_breaks_as_halves() {
    use corinzia_layout::{layout_pages, PageSpec};
    // Narrow pages: one long tied note per system boundary.
    let s = score(&[("treble", &["g'1~ g'1~ g'1~ g'1"])], (4, 4), 0);
    let narrow = PageSpec { width: 40.0, ..PageSpec::default() };
    let pages = layout_pages(&s, &Options::default(), &narrow);
    let systems: usize = pages.iter().map(|p| p.staves.len()).sum();
    let ties: usize = pages.iter().map(|p| p.items.iter().filter(|i| matches!(i, Item::Crescent { .. })).count()).sum();
    assert!(systems >= 2);
    // Three ties; each crossing a break is drawn twice (outgoing + incoming half).
    assert_eq!(ties, 3 + (systems - 1));
}
