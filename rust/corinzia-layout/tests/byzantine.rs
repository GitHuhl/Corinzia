//! The Byzantine engraver on the demo chant. Set CORINZIA_BYZ_OUT to a
//! directory to also get its pages as PNG files.

use corinzia_layout::byz::{engrave, ByzPage};
use corinzia_layout::byz_font::{ByzFont, BYZ};
use corinzia_layout::display::Item;

#[test]
fn the_demo_chant_is_engraved_on_justified_lines() {
    let chant = corinzia_byz::demo();
    let page = ByzPage::default();
    let pages = engrave(&chant, &page);
    assert_eq!(pages.len(), 1);
    let p = &pages[0];
    let f = ByzFont::get();
    let glyphs: Vec<(u32, f32, f32)> = p.items.iter().filter_map(|i| match i {
        Item::Glyph { glyph, at, .. } => Some((*glyph, at.x, at.y)),
        _ => None,
    }).collect();
    assert!(glyphs.iter().all(|g| g.0 & BYZ != 0), "only Byzantine glyphs");
    let has = |name: &str| glyphs.iter().any(|g| g.0 == f.glyph(name).unwrap());
    for name in ["modeWordEchos", "modeAlpha", "modePa", "ison", "oligon", "apostrofos", "yporroi", "klasmaAbove", "gorgonAbove", "dipli", "fthoraHardChromaticDiAbove", "fthoraDiatonicPaAbove", "martyriaNotePa", "martyriaAlphaBelow"] {
        assert!(has(name), "{name} is drawn");
    }
    // Lyrics: the syllables without their hyphens, and hyphens between.
    let texts: Vec<&str> = p.items.iter().filter_map(|i| match i {
        Item::Text { text, .. } => Some(text.as_str()),
        _ => None,
    }).collect();
    assert_eq!(texts[0], "Κύριε ἐλέησον");
    assert!(texts.contains(&"Κύ") && texts.contains(&"σον") && texts.contains(&"-"));
    assert!(!texts.iter().any(|t| t.len() > 1 && t.ends_with('-')));
    // A melisma's extender line.
    assert!(p.items.iter().any(|i| matches!(i, Item::Line { .. })));
    // Everything inside the margins; the first full line reaches the right one.
    let right = page.width - page.margin_x;
    for (_, x, _) in &glyphs {
        assert!(*x >= page.margin_x - 0.01 && *x < right, "{x}");
    }
    let neume_lines: std::collections::BTreeSet<i32> = p.items.iter().filter_map(|i| match i {
        Item::Glyph { at, id, .. } if *id != 0 => Some((at.y * 10.0) as i32 / 30),
        _ => None,
    }).collect();
    assert!(neume_lines.len() >= 3, "several lines: {neume_lines:?}");
    let first_line_end = p.items.iter().filter_map(|i| match i {
        Item::Glyph { at, id, glyph, .. } if *id != 0 => {
            let b = f.bbox_of(*glyph);
            Some((at.y, at.x + b.ne.0))
        }
        _ => None,
    }).fold((f32::MAX, 0.0f32), |(y0, x), (y, r)| if y < y0 - 0.5 { (y, r) } else if (y - y0).abs() <= 3.0 { (y0, x.max(r)) } else { (y0, x) });
    assert!(first_line_end.1 > right - 4.0, "the first line is justified: ends at {} of {right}", first_line_end.1);

    if let Ok(dir) = std::env::var("CORINZIA_BYZ_OUT") {
        let text = std::env::var("CORINZIA_TEXT_FONT").ok().and_then(|p| std::fs::read(p).ok());
        if let Some(t) = text {
            corinzia_layout::display::set_text_font(t, None);
        }
        for (k, page) in pages.iter().enumerate() {
            corinzia_layout::raster::render(page, 8.0, 0.0).save_png(format!("{dir}/byz-{k}.png")).unwrap();
        }
    }
}
