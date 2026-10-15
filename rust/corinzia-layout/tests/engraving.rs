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
fn eighths_are_beamed_as_lilypond_does() {
    // 4/4: eighths in half bars (LilyPond's default), sixteenths by beat.
    let s = score(&[("treble", &["c''8 d'' e'' f'' g'4 a'8 b' | c''16 d'' e'' f'' g''16 a'' b'' c''' c''2"])], (4, 4), 0);
    let l = layout_line(&s, &Options::default());
    let beams = l.items.iter().filter(|i| matches!(i, Item::Quad { .. })).count();
    assert_eq!(beams, 2 + 2 * 2, "one beam over beats 1-2, one in beat 4; two double beams");
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
    // Every system but the first starts with its bar number, rising.
    let numbers: Vec<usize> = pages
        .iter()
        .flat_map(|p| p.items.iter())
        .filter_map(|i| match i { Item::Text { text, size, .. } if (*size - 1.6).abs() < 1e-6 => text.parse().ok(), _ => None })
        .collect();
    assert_eq!(numbers.len(), systems - 1, "{numbers:?}");
    assert!(numbers[0] > 1 && numbers.windows(2).all(|w| w[0] < w[1]), "{numbers:?}");
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

#[test]
fn lyrics_and_chord_symbols() {
    use corinzia_score::score::ChordSymbol;
    // Measure text with the font the preview draws with (as the app does).
    if let Ok(path) = std::env::var("CORINZIA_TEXT_FONT") {
        if let Ok(regular) = std::fs::read(&path) {
            corinzia_layout::display::set_text_font(regular, std::fs::read(path.replace(".ttf", "-Bold.ttf")).ok());
        }
    }
    use corinzia_score::Frac;
    let mut s = score(
        &[("treble", &["g'4\\ly=Glo- \\ly2=Ah a'8\\ly=ri- b'8\\ly=a c''2\\ly=in_ \\ly2=men | d''8\\ly=ex- c''8\\ly=cel- b'4\\ly=sis e'2\\ly=De-"])],
        (4, 4),
        0,
    );
    s.parts[0].chords = vec![
        ChordSymbol { at: Frac::ZERO, text: "G".into(), id: 1 },
        ChordSymbol { at: Frac::new(1, 2), text: "C♯m7♭5/E".into(), id: 2 },
        ChordSymbol { at: Frac::new(1, 1), text: "Dmaj7".into(), id: 3 },
        ChordSymbol { at: Frac::new(3, 2), text: "Bb°7".into(), id: 4 },
    ];
    let l = layout_line(&s, &Options::default());
    preview("lyrics_chords", &l);
    let texts: Vec<(String, f32, f32, f32)> = l
        .items
        .iter()
        .filter_map(|i| match i {
            Item::Text { text, at, size, .. } => Some((text.clone(), at.x, at.y, *size)),
            _ => None,
        })
        .collect();
    let y_of = |t: &str| texts.iter().find(|x| x.0 == t).unwrap_or_else(|| panic!("no {t}: {texts:?}")).2;
    // Verse 1 on one baseline under the staff, verse 2 below it.
    let v1 = y_of("Glo");
    for syl in ["ri", "a", "in", "ex", "cel", "sis", "De"] {
        assert!((y_of(syl) - v1).abs() < 1e-4, "{syl} on the verse-1 baseline");
    }
    assert!(v1 > 4.0 + 2.0, "below the staff: {v1}");
    assert!((y_of("Ah") - v1 - 2.4).abs() < 1e-3 && (y_of("men") - y_of("Ah")).abs() < 1e-4);
    // Hyphens inside words; one hangs after "De" at the end of the line.
    let hyphens = texts.iter().filter(|t| t.0 == "-").count();
    assert_eq!(hyphens, 5, "Glo-ri-a, ex-cel-sis, De-");
    // "in" has an extender through nothing (one note), so no line; but syllables never overlap.
    let mut v1s: Vec<&(String, f32, f32, f32)> = texts.iter().filter(|t| (t.2 - v1).abs() < 1e-4 && t.0 != "-").collect();
    v1s.sort_by(|a, b| a.1.total_cmp(&b.1));
    for w in v1s.windows(2) {
        let (a, b) = (w[0], w[1]);
        let (wa, wb) = (corinzia_layout::display::text_width(&a.0, a.3, false), corinzia_layout::display::text_width(&b.0, b.3, false));
        assert!(b.1 - wb / 2.0 > a.1 + wa / 2.0 + 0.5, "{} and {} collide", a.0, b.0);
    }
    // Chord symbols on one baseline above the staff, accidentals as SMuFL chord glyphs.
    let gy = y_of("G");
    assert!(gy < -0.5, "above the staff: {gy}");
    assert!((y_of("C") - gy).abs() < 1e-4 && (y_of("Dmaj7") - gy).abs() < 1e-4);
    assert_eq!(glyphs(&l, Glyph::CsymSharp).len(), 1);
    assert_eq!(glyphs(&l, Glyph::CsymFlat).len(), 2, "♭5 and B♭");
    assert_eq!(glyphs(&l, Glyph::CsymDiminished).len(), 1);
}

#[test]
fn spanners_continue_across_system_breaks() {
    use corinzia_layout::{layout_pages, PageSpec};
    // 24 bars of quarters: several systems. A slur and a crescendo from bar 3 to bar 22.
    let mut music = String::new();
    for bar in 0..24 {
        for beat in 0..4 {
            let mut n = String::from(["c''4", "d''4", "e''4", "f''4"][beat]);
            if bar == 2 && beat == 0 {
                n.push_str("(\\cr");
            }
            if bar == 21 && beat == 0 {
                n.push_str(")\\f");
            }
            music.push_str(&n);
            music.push(' ');
        }
    }
    let s = score(&[("treble", &[music.as_str()])], (4, 4), 0);
    let pages = layout_pages(&s, &Options::default(), &PageSpec::default());
    let systems: usize = pages.iter().map(|p| p.staves.iter().map(|s| s.system).max().map(|m| m as usize + 1).unwrap_or(0)).sum();
    assert!(systems >= 3, "the example needs several systems: {systems}");
    let slur_id = s.parts[0].voices[0].events[8].pitches[0].id;
    let slur_pieces: usize = pages.iter().flat_map(|p| p.items.iter()).filter(|i| matches!(i, Item::Crescent { id, .. } if *id == slur_id)).count();
    assert!(slur_pieces >= 3, "a piece of the slur in each system it crosses: {slur_pieces}");
    // The crescendo: a pair of lines per system; openings grow from piece to piece.
    let mut openings: Vec<(f32, f32)> = Vec::new();
    for p in &pages {
        let lines: Vec<_> = p.items.iter().filter_map(|i| match i { Item::Line { from, to, id, .. } if *id == slur_id => Some((*from, *to)), _ => None }).collect();
        for pair in lines.chunks(2) {
            if let [a, b] = pair {
                openings.push(((a.0.y - b.0.y).abs(), (a.1.y - b.1.y).abs()));
            }
        }
    }
    assert!(openings.len() >= 3, "{openings:?}");
    assert!(openings[0].0 < 1e-3, "starts closed");
    for w in openings.windows(2) {
        assert!((w[1].0 - w[0].1).abs() < 0.05, "each piece continues the previous opening: {openings:?}");
    }
    assert!((openings.last().unwrap().1 - 1.0).abs() < 1e-3, "ends fully open");
}

#[test]
fn grand_staff_has_a_brace_joined_barlines_and_one_name() {
    let mut s = score(&[("treble", &["c''4 e'' g'' c'''"]), ("bass", &["c2 g,2"])], (4, 4), 0);
    s.parts[0].name = "Piano".into();
    s.parts[1].name = "Piano".into();
    s.parts[0].brace = true;
    let l = layout_line(&s, &Options::default());
    let (top0, top1) = (l.staves[0].top, l.staves[1].top);
    // The brace: two halves from the first staff's top to the second's bottom.
    let crescents: Vec<_> = l.items.iter().filter(|i| matches!(i, Item::Crescent { id: 0, .. })).filter_map(|i| match i { Item::Crescent { from, to, .. } => Some((*from, *to)), _ => None }).collect();
    let brace: Vec<_> = crescents.iter().filter(|(f, _)| f.x < l.staves[0].left).collect();
    assert_eq!(brace.len(), 2);
    assert!((brace[0].0.y - top0).abs() < 1e-3 && (brace[1].0.y - (top1 + 4.0)).abs() < 1e-3);
    // Barlines run from the top staff's top line to the bottom staff's top line.
    let joined = l.items.iter().any(|i| matches!(i, Item::Line { from, to, .. } if (from.y - top0).abs() < 1e-3 && (to.y - top1).abs() < 1e-3 && from.x > l.staves[0].left + 1.0));
    assert!(joined, "barlines join the staves");
    // One name.
    let names = l.items.iter().filter(|i| matches!(i, Item::Text { text, .. } if text == "Piano")).count();
    assert_eq!(names, 1);
    preview("grand_staff", &l);
}

#[test]
fn page_header_and_numbers() {
    use corinzia_layout::{layout_pages, PageSpec};
    let mut music = String::new();
    for _ in 0..120 {
        music.push_str("c''4 d'' e'' f'' | ");
    }
    let mut s = score(&[("treble", &[music.as_str()])], (4, 4), 0);
    s.work_title = "Sonatina".into();
    s.title = "I. Allegro".into();
    s.composer = "A. Composer".into();
    let pages = layout_pages(&s, &Options::default(), &PageSpec::default());
    assert!(pages.len() >= 2, "{} pages", pages.len());
    let texts = |p: &corinzia_layout::DisplayList| -> Vec<(String, u8)> { p.items.iter().filter_map(|i| match i { Item::Text { text, anchor, .. } => Some((text.clone(), *anchor)), _ => None }).collect() };
    let first = texts(&pages[0]);
    assert!(first.contains(&("Sonatina".into(), 1)) && first.contains(&("I. Allegro".into(), 1)) && first.contains(&("A. Composer".into(), 2)));
    assert!(!first.iter().any(|t| t.0 == "1"), "no number on page 1");
    assert!(texts(&pages[1]).contains(&("2".into(), 0)), "page 2 numbered on the left");
    if pages.len() > 2 {
        assert!(texts(&pages[2]).contains(&("3".into(), 2)));
    }
    assert!(!texts(&pages[1]).iter().any(|t| t.0 == "Sonatina"));
    preview("page_header", &pages[0]);
}

#[test]
fn pickup_bar_counts_its_beats_from_the_end() {
    // Five eighths before the first 4/4 bar: the first is on beat 2 and
    // stands alone, the other four share the bar's second half.
    let mut s = score(&[("treble", &["c''8 d'' e'' f'' g'' a''1"])], (4, 4), 0);
    s.meter = vec![
        corinzia_score::score::Meter { at: corinzia_score::Frac::ZERO, num: 5, den: 8, pickup: true },
        corinzia_score::score::Meter { at: corinzia_score::Frac::new(5, 8), num: 4, den: 4, pickup: false },
    ];
    s.length = corinzia_score::Frac::new(13, 8);
    let l = layout_line(&s, &Options::default());
    // The time signature is written once, as 4/4.
    assert_eq!(glyphs(&l, Glyph::TimeSigDigit(4)).len(), 2);
    assert!(glyphs(&l, Glyph::TimeSigDigit(5)).is_empty() && glyphs(&l, Glyph::TimeSigDigit(8)).is_empty());
    let heads = glyphs(&l, Glyph::NoteheadBlack);
    let flags: Vec<f32> = glyphs(&l, Glyph::Flag8thDown).into_iter().chain(glyphs(&l, Glyph::Flag8thUp)).map(|f| f.0).collect();
    assert_eq!(flags.len(), 1);
    assert!(flags[0] < heads[1].0, "the flagged eighth is the first: {flags:?}");
    preview("pickup", &l);
}

#[test]
fn tuplets_show_their_number_and_a_bracket_without_a_beam() {
    // Two eighth triplets (a beam each, a 3 over each), a quintuplet of
    // sixteenths (one beam, a 5), then a quarter-note triplet: a bracket.
    let s = score(&[("treble", &["\\tuplet 3/2 { c''8 d'' e'' } \\tuplet 3/2 { f'' g'' a'' } \\tuplet 5/4 { c''16 d'' e'' f'' g'' } c''4 | \\tuplet 3/2 { c''4 d'' e'' } c''2"])], (4, 4), 0);
    let l = layout_line(&s, &Options::default());
    assert_eq!(glyphs(&l, Glyph::TupletDigit(3)).len(), 3);
    assert_eq!(glyphs(&l, Glyph::TupletDigit(5)).len(), 1);
    // Beams: one per triplet and two (8th + 16th levels) for the quintuplet.
    let beams = l.items.iter().filter(|i| matches!(i, Item::Quad { .. })).count();
    assert_eq!(beams, 1 + 1 + 2);
    // The quarter triplet is bracketed: four lines (two hooks, two halves).
    let digit3 = glyphs(&l, Glyph::TupletDigit(3)).into_iter().map(|g| g.0).fold(0.0f32, f32::max);
    let bracket = l
        .items
        .iter()
        .filter(|i| matches!(i, Item::Line { from, to, .. } if from.y == to.y && (from.x - digit3).abs() < 8.0 && from.y.fract() != 0.0))
        .count();
    assert!(bracket >= 2, "a bracket around the quarter triplet");
    // Quarter-triplet heads are half the width of a 3/2 quarter: written as quarters.
    assert_eq!(glyphs(&l, Glyph::NoteheadBlack).len(), 3 + 3 + 5 + 1 + 3);
    preview("tuplets", &l);
}

#[test]
fn grace_notes_are_small_and_take_room_before_their_note() {
    let plain = score(&[("treble", &["c''4 d''4 e''4 f''4"])], (4, 4), 0);
    let s = score(&[("treble", &["c''4 \\acciaccatura e''8 d''4 \\grace { f''16 g'' } e''4 \\grace bes'8 f''4"])], (4, 4), 0);
    let (a, b) = (layout_line(&plain, &Options::default()), layout_line(&s, &Options::default()));
    // Small heads: one slashed eighth, two beamed sixteenths, one with a flat.
    let small: Vec<f32> = b
        .items
        .iter()
        .filter_map(|i| match i {
            Item::Glyph { glyph, scale, at, .. } if *glyph == Glyph::NoteheadBlack.codepoint() && *scale < 1.0 => Some(at.x),
            _ => None,
        })
        .collect();
    assert_eq!(small.len(), 4);
    assert!(b.items.iter().any(|i| matches!(i, Item::Glyph { glyph, scale, .. } if *glyph == Glyph::Flat.codepoint() && *scale < 1.0)));
    // Each sits before its main note and the line gets wider.
    let heads = glyphs(&b, Glyph::NoteheadBlack);
    let mains: Vec<f32> = heads.iter().map(|h| h.0).filter(|x| !small.contains(x)).collect();
    assert_eq!(mains.len(), 4);
    assert!(small[0] > mains[0] && small[0] < mains[1]);
    assert!(small[1] > mains[1] && small[2] < mains[2]);
    assert!(small[3] > mains[2] && small[3] < mains[3]);
    let width = |l: &corinzia_layout::DisplayList| glyphs(l, Glyph::NoteheadBlack).iter().map(|h| h.0).fold(0.0f32, f32::max);
    assert!(width(&b) > width(&a) + 2.0);
    // The two sixteenths share two small beams.
    assert_eq!(b.items.iter().filter(|i| matches!(i, Item::Quad { .. })).count(), 2);
    preview("graces", &b);
}

#[test]
fn families_are_bracketed_and_their_barlines_joined() {
    let mut s = score(&[("treble", &["c''1"]), ("treble", &["e'1"]), ("treble", &["g'1"]), ("bass", &["c1"])], (4, 4), 0);
    // Two woodwinds (bracketed together), then a lone piano-like pair without.
    s.parts[0].group = 1;
    s.parts[1].group = 1;
    let l = layout_line(&s, &Options::default());
    let left = l.staves[0].left;
    // Serifs at both ends of the bracket, left of the system.
    let top = glyphs(&l, Glyph::BracketTop);
    let bottom = glyphs(&l, Glyph::BracketBottom);
    assert_eq!((top.len(), bottom.len()), (1, 1));
    assert!(top[0].0 < left && top[0].1 < l.staves[0].top && bottom[0].1 > l.staves[1].top + 4.0);
    // The final barline runs from staff 1 to staff 2 in one stroke, and not on to staff 3.
    let vertical: Vec<(f32, f32, f32)> = l
        .items
        .iter()
        .filter_map(|i| match i {
            Item::Line { from, to, .. } if from.x == to.x && from.x > left + 5.0 => Some((from.x, from.y, to.y)),
            _ => None,
        })
        .collect();
    let (t0, t1, t2) = (l.staves[0].top, l.staves[1].top, l.staves[2].top);
    assert!(vertical.iter().any(|v| v.1 == t0 && v.2 == t1), "joined through the family");
    assert!(!vertical.iter().any(|v| v.1 == t1 && v.2 > t2 - 0.1), "not on to the next staff");
    preview("brackets", &l);
}

#[test]
fn segno_coda_and_jumps_are_written() {
    use corinzia_score::score::{Nav, NavMark, NavTo};
    use corinzia_score::Frac;
    let mut s = score(&[("treble", &["c''1 | d''1 | e''1 | f''1"])], (4, 4), 0);
    s.navigation = vec![
        NavMark { id: 501, at: Frac::int(1), kind: Nav::Segno },
        NavMark { id: 502, at: Frac::int(2), kind: Nav::ToCoda },
        NavMark { id: 503, at: Frac::int(3), kind: Nav::DalSegno(NavTo::Coda) },
        NavMark { id: 504, at: Frac::int(3), kind: Nav::Coda },
    ];
    let l = layout_line(&s, &Options::default());
    let segno = glyphs(&l, Glyph::Segno);
    let coda = glyphs(&l, Glyph::Coda);
    assert_eq!(segno.len(), 1);
    assert_eq!(coda.len(), 2, "the coda sign and the one after To Coda");
    assert!(segno[0].1 < l.staves[0].top, "above the staff");
    let texts: Vec<(&str, f32)> = l.items.iter().filter_map(|i| match i { Item::Text { text, at, italic: true, .. } => Some((text.as_str(), at.x)), _ => None }).collect();
    assert!(texts.iter().any(|t| t.0 == "To Coda"));
    let ds = texts.iter().find(|t| t.0 == "D.S. al Coda").unwrap();
    // The jump ends at the barline after bar 3; the segno sits over the barline after bar 1.
    assert!(ds.1 > segno[0].0);
    preview("navigation", &l);
}

#[test]
fn octave_lines_lower_the_notes_and_pedals_sit_below() {
    use corinzia_score::score::{LineKind, LineSpan};
    use corinzia_score::Frac;
    let plain = score(&[("treble", &["c''''4 d'''' e'''' f'''' | c'1"])], (4, 4), 0);
    let mut s = score(&[("treble", &["c''''4 d'''' e'''' f'''' | c'1"])], (4, 4), 0);
    s.parts[0].lines = vec![
        LineSpan { id: 601, from: Frac::ZERO, to: Frac::int(1), kind: LineKind::Octave(1) },
        LineSpan { id: 602, from: Frac::int(1), to: Frac::int(2), kind: LineKind::Pedal },
    ];
    let (a, b) = (layout_line(&plain, &Options::default()), layout_line(&s, &Options::default()));
    let ys = |l: &corinzia_layout::DisplayList| glyphs(l, Glyph::NoteheadBlack).iter().map(|h| h.1).collect::<Vec<_>>();
    // An octave is 3.5 spaces: the four notes come down by that.
    for (y0, y1) in ys(&a).iter().zip(ys(&b)) {
        assert!((y1 - y0 - 3.5).abs() < 1e-4, "{y0} -> {y1}");
    }
    let ottava = glyphs(&b, Glyph::Ottava);
    assert_eq!(ottava.len(), 1);
    assert!(ottava[0].1 < b.staves[0].top);
    // Fewer ledger lines than without the line.
    let ledgers = |l: &corinzia_layout::DisplayList| l.items.iter().filter(|i| matches!(i, Item::Line { width, from, to, .. } if (*width - 0.16).abs() < 1e-6 && from.y == to.y)).count();
    assert!(ledgers(&b) < ledgers(&a));
    // Ped. and its release under the staff.
    let (ped, up) = (glyphs(&b, Glyph::PedalPed), glyphs(&b, Glyph::PedalUp));
    assert_eq!((ped.len(), up.len()), (1, 1));
    assert!(ped[0].1 > b.staves[0].top + 4.0 && up[0].0 > ped[0].0);
    preview("lines", &b);
}

#[test]
fn empty_bars_become_multi_bar_rests_in_parts() {
    use corinzia_score::Frac;
    // A bar of notes, five empty bars, a rehearsal mark, two more empty, a bar of notes.
    let mut s = score(&[("treble", &["c''1 | r1 | r1 | r1 | r1 | r1 | r1 | r1 | d''1"])], (4, 4), 0);
    s.marks = vec![Frac::int(6)];
    let opt = Options { multirest: true, ..Options::default() };
    let l = layout_line(&s, &opt);
    // Two H-bars (5 bars, then 2 after the mark), counted above.
    let hbars = l.items.iter().filter(|i| matches!(i, Item::Quad { p, .. } if (p[2].y - p[0].y - 1.0).abs() < 1e-4)).count();
    assert_eq!(hbars, 2);
    assert_eq!(glyphs(&l, Glyph::TimeSigDigit(5)).len(), 1);
    assert_eq!(glyphs(&l, Glyph::TimeSigDigit(2)).len(), 1);
    assert!(glyphs(&l, Glyph::RestWhole).is_empty());
    // Without the option: seven whole-bar rests.
    let full = layout_line(&s, &Options::default());
    assert_eq!(glyphs(&full, Glyph::RestWhole).len(), 7);
    assert!(l.width < full.width);
    preview("multirest", &l);
}
