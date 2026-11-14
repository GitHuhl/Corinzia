//! The Byzantine engraver: a [`Chant`] laid out on pages as lines of
//! neumes with their syllables underneath.
//!
//! Each element becomes a box: a neume with its marks (time signs, gorgons,
//! fthoras) placed by the font's anchors, a martyria (the note and its root
//! sign), or a break. Lines are filled greedily and justified, all but the
//! last line of the chant; the chant's title and mode signature head the
//! first page. Melismas carry an extender line, or hyphens when the word
//! goes on (a syllable typed with a trailing `-`).
//!
//! Coordinates are staff spaces, as for the Western engraver; neumes are
//! drawn at `neume_scale` (1 = an em of 4 spaces).

use crate::byz_font::{ByzFont, SPACES_PER_EM};
use crate::display::{pt, DisplayList, Item};
use corinzia_byz::*;

/// The page and its type sizes, in staff spaces.
#[derive(Clone, Copy, Debug)]
pub struct ByzPage {
    pub width: f32,
    pub height: f32,
    pub margin_x: f32,
    pub margin_y: f32,
    /// Points per staff space when printed.
    pub space_pt: f32,
    /// Neume size: 1 = an em of 4 staff spaces.
    pub neume_scale: f32,
    pub lyric_size: f32,
    pub title_size: f32,
    /// Distance between the neume lines of consecutive lines, in ems.
    pub line_ems: f32,
}

impl Default for ByzPage {
    /// A4 with a staff space of 1.75 mm: neumes at about 20 pt, lyrics at
    /// 12 pt.
    fn default() -> ByzPage {
        let space_pt = 1.75 * 72.0 / 25.4;
        ByzPage {
            width: 210.0 / 1.75,
            height: 297.0 / 1.75,
            margin_x: 11.0,
            margin_y: 11.0,
            space_pt,
            neume_scale: 1.0,
            lyric_size: 12.0 / space_pt,
            title_size: 18.0 / space_pt,
            line_ems: 2.5,
        }
    }
}

/// The glyph of a quantity neume.
pub fn quantity_glyph(q: Quantity) -> &'static str {
    use Quantity::*;
    match q {
        Ison => "ison",
        Oligon => "oligon",
        Petasti => "petasti",
        Kentimata => "kentimata",
        Kentima => "kentima",
        OligonKentimaAbove => "oligonKentimaAbove",
        OligonYpsiliRight => "oligonYpsiliRight",
        OligonYpsiliLeft => "oligonYpsiliLeft",
        Apostrofos => "apostrofos",
        Elafron => "elafron",
        ElafronApostrofos => "elafronApostrofos",
        Chamili => "chamili",
        Yporroi => "yporroi",
        OligonKentimataBelow => "oligonKentimataBelow",
        OligonIsonKentimata => "oligonIsonKentimata",
    }
}

/// A time sign's glyph and the anchor it hangs on.
fn time_glyph(t: TimeSign) -> (&'static str, &'static str) {
    match t {
        TimeSign::Klasma => ("klasmaAbove", "klasmaTop"),
        TimeSign::Dipli => ("dipli", "apli"),
        TimeSign::Tripli => ("tripli", "apli"),
        TimeSign::Gorgon => ("gorgonAbove", "gorgonTop"),
        TimeSign::Digorgon => ("digorgon", "gorgonTop"),
        TimeSign::Trigorgon => ("trigorgon", "gorgonTop"),
    }
}

/// Steps from Pa: the parity that picks a chromatic sign's form.
fn from_pa(d: Degree) -> i32 {
    d.index() - Degree::Pa.index()
}

/// The glyph of a fthora written over a neume.
pub fn fthora_glyph(f: Fthora) -> String {
    let degree = match f.genus {
        Genus::Diatonic => match f.degree {
            Degree::Ni => "NiLow",
            d => d.name(),
        },
        Genus::HardChromatic => ["Pa", "Di"][from_pa(f.degree).rem_euclid(2) as usize],
        Genus::SoftChromatic => ["Ke", "Di"][from_pa(f.degree).rem_euclid(2) as usize],
    };
    let genus = match f.genus {
        Genus::Diatonic => "Diatonic",
        Genus::HardChromatic => "HardChromatic",
        Genus::SoftChromatic => "SoftChromatic",
    };
    format!("fthora{genus}{degree}Above")
}

/// A martyria's two glyphs: the note and its root sign (below it, or
/// above it for the low notes).
pub fn martyria_glyphs(note: Note, genus: Genus) -> (String, &'static str) {
    let register = match note.octave {
        0 => "",
        o if o < 0 => "Low",
        _ => "High",
    };
    let name = format!("martyriaNote{}{register}", note.degree.name());
    let rel = note.steps() - 1; // Pa = 0
    let low = note.steps() <= -2; // Ke and below
    let sign = match genus {
        Genus::Diatonic => {
            // Neanes' diatonic root signs, from Pa: alpha, legetos, nana,
            // dotted delta, dotted alpha, legetos, nana, then alpha again.
            const UP: [&str; 7] = ["Alpha", "Legetos", "Nana", "DeltaDotted", "AlphaDotted", "Legetos", "Nana"];
            match rel {
                -1 => "Delta",
                -2 => "Zo",
                r if r >= 0 => UP[(r % 7) as usize],
                r => ["Delta", "Alpha", "Legetos", "Nana"][((r + 400) % 4) as usize],
            }
        }
        Genus::HardChromatic => ["HardChromaticPa", "HardChromaticDi"][rel.rem_euclid(2) as usize],
        Genus::SoftChromatic => ["SoftChromaticKe", "SoftChromaticDi"][rel.rem_euclid(2) as usize],
    };
    let root = match (sign, low) {
        ("Alpha", false) => "martyriaAlphaBelow",
        ("Alpha", true) => "martyriaAlphaAbove",
        ("Delta", false) => "martyriaDeltaBelow",
        ("Delta", true) => "martyriaDeltaAbove",
        ("Legetos", false) => "martyriaLegetosBelow",
        ("Legetos", true) => "martyriaLegetosAbove",
        ("Nana", false) => "martyriaNanaBelow",
        ("Nana", true) => "martyriaNanaAbove",
        ("DeltaDotted", false) => "martyriaDeltaDottedBelow",
        ("DeltaDotted", true) => "martyriaDeltaDottedAbove",
        ("AlphaDotted", false) => "martyriaAlphaDottedBelow",
        ("AlphaDotted", true) => "martyriaAlphaDottedAbove",
        ("Zo", _) => "martyriaZoBelow",
        ("HardChromaticPa", false) => "martyriaHardChromaticPaBelow",
        ("HardChromaticPa", true) => "martyriaHardChromaticPaAbove",
        ("HardChromaticDi", false) => "martyriaHardChromaticDiBelow",
        ("HardChromaticDi", true) => "martyriaHardChromaticDiAbove",
        ("SoftChromaticKe", false) => "martyriaSoftChromaticKeBelow",
        ("SoftChromaticKe", true) => "martyriaSoftChromaticKeAbove",
        ("SoftChromaticDi", false) => "martyriaSoftChromaticDiBelow",
        _ => "martyriaSoftChromaticDiAbove",
    };
    (name, root)
}

/// The glyphs of a mode signature: "Ἦχος", plagal, the mode's letter (or
/// "βαρύς"), then the base note.
pub fn mode_glyphs(m: &ModeKey) -> Vec<String> {
    let mut g = vec!["modeWordEchos".to_string()];
    let (plagal, letter) = match m.echos {
        Echos::First => (false, "modeAlpha"),
        Echos::Second => (false, "modeBeta"),
        Echos::Third => (false, "modeGamma"),
        Echos::Fourth => (false, "modeDelta"),
        Echos::PlagalFirst => (true, "modeAlpha"),
        Echos::PlagalSecond => (true, "modeBeta"),
        Echos::Varys => (false, "modeWordVarys"),
        Echos::PlagalFourth => (true, "modeDelta"),
    };
    if plagal {
        g.push("modePlagal".into());
    }
    g.push(letter.into());
    g.push(format!("mode{}", m.base.degree.name()));
    g
}

/// A glyph placed in a box, offset from the box's origin (spaces, y down).
struct Placed {
    glyph: u32,
    x: f32,
    y: f32,
}

/// One element laid out on its own.
struct Boxed {
    width: f32,
    /// Where the neume's glyphs start inside the box.
    glyph_x: f32,
    glyphs: Vec<Placed>,
    lyric: Option<(String, f32)>,
    /// The syllable goes on to the next one (written with a trailing `-`).
    hyphen: bool,
    neume: bool,
    break_after: bool,
    id: u64,
}

/// The id of element `i` on the page (its index + 1).
pub fn element_id(i: usize) -> u64 {
    i as u64 + 1
}

/// Lays a chant out on pages.
pub fn engrave(chant: &Chant, page: &ByzPage) -> Vec<DisplayList> {
    let f = ByzFont::get();
    let em = SPACES_PER_EM * page.neume_scale;
    let glyph = |name: &str| f.glyph(name);
    // A mark on a base glyph: the base's anchor minus the mark's own.
    let mark = |base: &str, name: &str, anchor: &str| -> Option<Placed> {
        let (bx, by) = f.anchor(base, anchor)?;
        let (mx, my) = f.anchor(name, anchor).unwrap_or((0.0, 0.0));
        Some(Placed { glyph: glyph(name)?, x: (bx - mx) * em, y: -(by - my) * em })
    };
    let (sung, _) = chant.sing();
    let mut genus = chant.mode.genus;
    let mut boxes: Vec<Boxed> = vec![];
    let gap = 0.15 * em;
    for (i, el) in chant.elements.iter().enumerate() {
        let id = element_id(i);
        match el {
            Element::Neume(n) => {
                let Some(q) = n.quantity else { continue };
                let base = quantity_glyph(q);
                let mut glyphs: Vec<Placed> = glyph(base).map(|g| Placed { glyph: g, x: 0.0, y: 0.0 }).into_iter().collect();
                if let Some(t) = n.time {
                    let (name, anchor) = time_glyph(t);
                    glyphs.extend(mark(base, name, anchor));
                }
                if let Some(ft) = n.fthora {
                    genus = ft.genus;
                    glyphs.extend(mark(base, &fthora_glyph(ft), "fthoraTop"));
                }
                let w = f.advance(base) * em;
                let text = n.lyric.trim();
                let hyphen = text.ends_with('-');
                let shown = text.trim_end_matches('-').to_string();
                let lyric_w = if shown.is_empty() { 0.0 } else { crate::display::text_width(&shown, page.lyric_size, false) };
                let width = w.max(lyric_w + 0.5) + gap;
                let glyph_x = (width - gap - w) / 2.0;
                let lyric = (!shown.is_empty()).then(|| (shown, lyric_w));
                boxes.push(Boxed { width, glyph_x, glyphs, lyric, hyphen, neume: true, break_after: false, id });
            }
            Element::Martyria(d) => {
                // The note the melody is on, in the register it is sung.
                let at = sung.iter().rev().find(|s| s.element < i).map(|s| s.note).unwrap_or(chant.mode.base);
                let up = (d.index() - at.degree.index()).rem_euclid(7);
                let note = at.step(if up > 3 { up - 7 } else { up });
                let (name, root) = martyria_glyphs(note, genus);
                let mut glyphs: Vec<Placed> = glyph(&name).map(|g| Placed { glyph: g, x: 0.0, y: 0.0 }).into_iter().collect();
                glyphs.extend(mark(&name, root, "martyriaBottom").or_else(|| mark(&name, root, "martyriaTop")));
                let w = f.advance(&name) * em;
                boxes.push(Boxed { width: w + 3.0 * gap, glyph_x: gap, glyphs, lyric: None, hyphen: false, neume: false, break_after: false, id });
            }
            Element::Break => {
                if let Some(b) = boxes.last_mut() {
                    b.break_after = true;
                }
            }
        }
    }

    let new_page = || DisplayList { width: page.width, height: page.height, space_pt: page.space_pt, ..Default::default() };
    let mut pages = vec![new_page()];
    let left = page.margin_x;
    let right = page.width - page.margin_x;
    let mut y = page.margin_y;

    // The title and the mode signature, centred.
    if !chant.title.is_empty() {
        y += page.title_size;
        pages[0].items.push(Item::Text { text: chant.title.clone(), at: pt(page.width / 2.0, y), size: page.title_size, italic: false, bold: true, anchor: 1, id: 0 });
        y += page.title_size * 0.6;
    }
    let mode: Vec<(u32, f32)> = mode_glyphs(&chant.mode)
        .iter()
        .filter_map(|n| {
            let g = glyph(n)?;
            let b = f.bbox_of(g);
            Some((g, (f.advance(n) * em).max(b.width() * page.neume_scale) + 0.4 * gap))
        })
        .collect();
    let mode_w: f32 = mode.iter().map(|m| m.1).sum();
    y += 1.4 * em;
    let mut x = (page.width - mode_w) / 2.0;
    for (g, w) in mode {
        pages[0].items.push(Item::Glyph { glyph: g, at: pt(x, y), scale: page.neume_scale, id: 0 });
        x += w;
    }
    y += 0.4 * em;

    // Fill lines greedily.
    let width = right - left;
    let mut lines: Vec<std::ops::Range<usize>> = vec![];
    let mut start = 0;
    let mut used = 0.0;
    for (k, b) in boxes.iter().enumerate() {
        if k > start && used + b.width > width {
            lines.push(start..k);
            start = k;
            used = 0.0;
        }
        used += b.width;
        if b.break_after {
            lines.push(start..k + 1);
            start = k + 1;
            used = 0.0;
        }
    }
    if start < boxes.len() {
        lines.push(start..boxes.len());
    }

    let line_h = page.line_ems * em;
    for (li, line) in lines.iter().enumerate() {
        if y + line_h > page.height - page.margin_y {
            pages.push(new_page());
            y = page.margin_y;
        }
        let baseline = y + 1.25 * em;
        let lyric_y = baseline + 0.75 * em;
        let items = &mut pages.last_mut().expect("a page").items;
        let natural: f32 = boxes[line.clone()].iter().map(|b| b.width).sum();
        // Justify every line that is full: not the last, nor one broken by hand.
        let last = li + 1 == lines.len() || boxes[line.end - 1].break_after;
        let extra = if last || line.len() < 2 { 0.0 } else { (width - natural).max(0.0) / (line.len() - 1) as f32 };
        let mut x = left;
        let mut spans: Vec<(f32, f32)> = vec![]; // each box's x range on the line
        for b in &boxes[line.clone()] {
            for g in &b.glyphs {
                items.push(Item::Glyph { glyph: g.glyph, at: pt(x + b.glyph_x + g.x, baseline + g.y), scale: page.neume_scale, id: b.id });
            }
            if let Some((text, _)) = &b.lyric {
                items.push(Item::Text { text: text.clone(), at: pt(x + (b.width - gap) / 2.0, lyric_y), size: page.lyric_size, italic: false, bold: false, anchor: 1, id: b.id });
            }
            spans.push((x, x + b.width - gap));
            x += b.width + extra;
        }
        // Melismas: an extender after a word's last syllable, hyphens
        // between the syllables of a word.
        let line_boxes = &boxes[line.clone()];
        for (k, b) in line_boxes.iter().enumerate() {
            let Some((_, lw)) = &b.lyric else { continue };
            let centre = (spans[k].0 + spans[k].1) / 2.0;
            let end = centre + lw / 2.0;
            let next = line_boxes[k + 1..].iter().position(|n| n.lyric.is_some() || !n.neume).map(|p| k + 1 + p);
            let melisma_end = match next {
                Some(n) if n > k + 1 => Some(spans[n - 1].1),
                None if k + 1 < line_boxes.len() => Some(spans[line_boxes.len() - 1].1),
                _ => None,
            };
            if b.hyphen {
                let to = next.filter(|n| line_boxes[*n].lyric.is_some()).map(|n| {
                    let (_, nw) = line_boxes[n].lyric.as_ref().expect("a lyric");
                    (spans[n].0 + spans[n].1) / 2.0 - nw / 2.0
                });
                let to = to.unwrap_or(end + 1.6);
                items.push(Item::Text { text: "-".into(), at: pt((end + to) / 2.0, lyric_y), size: page.lyric_size, italic: false, bold: false, anchor: 1, id: 0 });
            } else if let Some(m) = melisma_end.filter(|m| *m > end + 0.4) {
                items.push(Item::line(end + 0.2, lyric_y, m, lyric_y, 0.12, 0));
            }
        }
        y += line_h;
    }
    pages
}
