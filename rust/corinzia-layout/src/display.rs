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
    /// A SMuFL glyph with its origin at `at` (baseline, SMuFL origin), size 1 = staff space metrics.
    Glyph { glyph: u32, at: Point, id: u64 },
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
        Item::Glyph { glyph: g.codepoint(), at: pt(x, y), id }
    }
    pub fn line(x0: f32, y0: f32, x1: f32, y1: f32, width: f32, id: u64) -> Item {
        Item::Line { from: pt(x0, y0), to: pt(x1, y1), width, id }
    }
    pub fn id(&self) -> u64 {
        match self {
            Item::Glyph { id, .. } | Item::Line { id, .. } | Item::Quad { id, .. } | Item::Crescent { id, .. } => *id,
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
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct Column {
    pub quarter: f64, // written position in quarters from the movement start
    pub x: f32,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct DisplayList {
    pub width: f32,
    pub height: f32,
    pub items: Vec<Item>,
    pub staves: Vec<StaffGeom>,
    pub columns: Vec<Column>,
}
