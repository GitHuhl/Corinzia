//! The engraver's output: a flat, renderer-agnostic display list in staff
//! spaces (x right, y down; the top line of the first staff is y = 0). Any
//! front end (Qt scene graph, wgpu in iced, tiny-skia for tests) draws it,
//! and `id`s map every mark back to the model for hit testing.

use crate::font::Glyph;
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}
pub fn pt(x: f32, y: f32) -> Point {
    Point { x, y }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "k", rename_all = "snake_case")]
pub enum Item {
    /// A SMuFL glyph with its origin at `at` (SMuFL origin); `scale` 1 = full size
    /// (clef changes are drawn smaller).
    Glyph { glyph: u32, at: Point, scale: f32, id: u64 },
    /// Text: baseline at `at`, `size` = em in staff spaces; anchor 0 start, 1 middle, 2 end.
    Text { text: String, at: Point, size: f32, italic: bool, bold: bool, anchor: u8, id: u64 },
    /// A straight stroke (staff lines, stems, ledger lines, barlines).
    Line { from: Point, to: Point, width: f32, id: u64 },
    /// A filled quadrilateral (beams).
    Quad { p: [Point; 4], id: u64 },
    /// A filled crescent between two cubic Béziers sharing end points (ties, slurs):
    /// the outer curve via `c1`/`c2`, the inner one via `d1`/`d2`.
    Crescent { from: Point, c1: Point, c2: Point, to: Point, d1: Point, d2: Point, id: u64 },
}

impl Item {
    pub fn glyph(g: Glyph, x: f32, y: f32, id: u64) -> Item {
        Item::Glyph { glyph: g.codepoint(), at: pt(x, y), scale: 1.0, id }
    }
    pub fn glyph_scaled(g: Glyph, x: f32, y: f32, scale: f32, id: u64) -> Item {
        Item::Glyph { glyph: g.codepoint(), at: pt(x, y), scale, id }
    }
    pub fn line(x0: f32, y0: f32, x1: f32, y1: f32, width: f32, id: u64) -> Item {
        Item::Line { from: pt(x0, y0), to: pt(x1, y1), width, id }
    }
    pub fn id(&self) -> u64 {
        match self {
            Item::Glyph { id, .. } | Item::Text { id, .. } | Item::Line { id, .. } | Item::Quad { id, .. } | Item::Crescent { id, .. } => *id,
        }
    }
}

/// Geometry besides the marks: staves (drawn by every renderer from this,
/// and used for the caret and mouse entry) and the time -> x map (playhead).
#[derive(Clone, Debug, Default, Serialize)]
pub struct StaffGeom {
    pub part: usize,
    pub top: f32, // y of the top line; five lines one space apart
    pub left: f32,
    pub right: f32,
    pub line_width: f32,
    pub system: u32, // index of the system on its page
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct Column {
    pub quarter: f64, // written position in quarters from the movement start
    pub x: f32,
    pub system: u32,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct DisplayList {
    pub width: f32,
    pub height: f32,
    pub items: Vec<Item>,
    pub staves: Vec<StaffGeom>,
    pub columns: Vec<Column>,
}

static TEXT_FONT: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
static TEXT_FONT_BOLD: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();

/// The text fonts the renderer draws with (.ttf/.otf; the bold cut is
/// optional). With them, text widths are measured exactly instead of
/// estimated. Set once, at startup.
pub fn set_text_font(regular: Vec<u8>, bold: Option<Vec<u8>>) -> bool {
    let ok = ttf_parser::Face::parse(&regular, 0).is_ok() && TEXT_FONT.set(regular).is_ok();
    if let Some(b) = bold.filter(|b| ttf_parser::Face::parse(b, 0).is_ok()) {
        let _ = TEXT_FONT_BOLD.set(b);
    }
    ok
}

/// Advance of `text` (staff spaces) in the score's serif text font: measured
/// when the renderer's font is known (`set_text_font`), else estimated.
pub fn text_width(text: &str, size: f32, bold: bool) -> f32 {
    let bold_face = if bold { TEXT_FONT_BOLD.get() } else { None };
    if let Some(face) = bold_face.or(TEXT_FONT.get()).and_then(|b| ttf_parser::Face::parse(b, 0).ok()) {
        let k = size / face.units_per_em() as f32;
        let w: f32 = text
            .chars()
            .map(|c| face.glyph_index(c).and_then(|g| face.glyph_hor_advance(g)).unwrap_or(face.units_per_em() / 2) as f32 * k)
            .sum();
        // Without the bold cut, bold is ~7 % wider than regular.
        return if bold && bold_face.is_none() { w * 1.07 } else { w };
    }
    let per = if bold { 0.56 } else { 0.5 };
    text.chars().map(|c| if c == ' ' { 0.28 } else if c.is_uppercase() { per * 1.25 } else { per }).sum::<f32>() * size
}

impl Item {
    pub fn translated(self, dx: f32, dy: f32) -> Item {
        let m = |p: Point| pt(p.x + dx, p.y + dy);
        match self {
            Item::Glyph { glyph, at, scale, id } => Item::Glyph { glyph, at: m(at), scale, id },
            Item::Text { text, at, size, italic, bold, anchor, id } => Item::Text { text, at: m(at), size, italic, bold, anchor, id },
            Item::Line { from, to, width, id } => Item::Line { from: m(from), to: m(to), width, id },
            Item::Quad { p, id } => Item::Quad { p: p.map(m), id },
            Item::Crescent { from, c1, c2, to, d1, d2, id } => {
                Item::Crescent { from: m(from), c1: m(c1), c2: m(c2), to: m(to), d1: m(d1), d2: m(d2), id }
            }
        }
    }

    /// Bounding box (x0, y0, x1, y1), y down; control points make curves conservative.
    pub fn bounds(&self, font: &crate::font::Font) -> [f32; 4] {
        let span = |pts: &[Point]| {
            let mut r = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
            for p in pts {
                r = [r[0].min(p.x), r[1].min(p.y), r[2].max(p.x), r[3].max(p.y)];
            }
            r
        };
        match self {
            Item::Glyph { glyph, at, scale, .. } => {
                let b = font.bbox_of(*glyph);
                [at.x + b.sw.0 * scale, at.y - b.ne.1 * scale, at.x + b.ne.0 * scale, at.y - b.sw.1 * scale]
            }
            Item::Text { text, at, size, bold, anchor, .. } => {
                let w = text_width(text, *size, *bold);
                let x0 = at.x - w * (*anchor as f32) / 2.0;
                [x0, at.y - 0.72 * size, x0 + w, at.y + 0.22 * size]
            }
            Item::Line { from, to, width, .. } => {
                let r = span(&[*from, *to]);
                [r[0] - width / 2.0, r[1] - width / 2.0, r[2] + width / 2.0, r[3] + width / 2.0]
            }
            Item::Quad { p, .. } => span(p),
            Item::Crescent { from, c1, c2, to, d1, d2, .. } => {
                // The curve passes 3/4 of the way to its control points' offset.
                let mid = |a: &Point, c: &Point| pt(c.x, a.y + (c.y - a.y) * 0.75);
                span(&[*from, *to, mid(from, c1), mid(to, c2), mid(from, d1), mid(to, d2)])
            }
        }
    }
}
