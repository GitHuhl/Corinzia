//! Rasterizes a display list with tiny-skia: reference images for tests and
//! previews. Glyphs are drawn from the font's outlines, so any scale is sharp.

use crate::display::{DisplayList, Item, Point};
use crate::font::{Font, FONT_DATA};
use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, Stroke, Transform};

struct Outline<'a>(&'a mut PathBuilder);
impl ttf_parser::OutlineBuilder for Outline<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.move_to(x, y);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.0.line_to(x, y);
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.0.quad_to(x1, y1, x, y);
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.0.cubic_to(x1, y1, x2, y2, x, y);
    }
    fn close(&mut self) {
        self.0.close();
    }
}

/// `scale`: pixels per staff space; `pad`: margin in staff spaces.
pub fn render(list: &DisplayList, scale: f32, pad: f32) -> Pixmap {
    let w = ((list.width + 2.0 * pad) * scale).ceil().max(1.0) as u32;
    let h = ((list.height + 2.0 * pad) * scale + 12.0 * scale).ceil().max(1.0) as u32;
    let mut pm = Pixmap::new(w, h).expect("pixmap size");
    pm.fill(tiny_skia::Color::WHITE);
    let mut paint = Paint::default();
    paint.set_color_rgba8(0, 0, 0, 255);
    paint.anti_alias = true;
    // Staff space -> pixels, leaving room above the first staff for marks.
    let base = Transform::from_scale(scale, scale).pre_translate(pad, pad + 6.0);
    let face = ttf_parser::Face::parse(FONT_DATA, 0).expect("font");
    let ups = Font::bravura().units_per_space;
    let p = |q: &Point| (q.x, q.y);
    for item in &list.items {
        match item {
            Item::Glyph { glyph, at, .. } => {
                let Some(ch) = char::from_u32(*glyph) else { continue };
                let Some(gid) = face.glyph_index(ch) else { continue };
                let mut pb = PathBuilder::new();
                if face.outline_glyph(gid, &mut Outline(&mut pb)).is_none() {
                    continue;
                }
                let Some(path) = pb.finish() else { continue };
                // Font units (y up) -> staff spaces (y down) at the glyph origin.
                let t = base.pre_translate(at.x, at.y).pre_scale(1.0 / ups, -1.0 / ups);
                pm.fill_path(&path, &paint, FillRule::Winding, t, None);
            }
            Item::Line { from, to, width, .. } => {
                let mut pb = PathBuilder::new();
                pb.move_to(from.x, from.y);
                pb.line_to(to.x, to.y);
                if let Some(path) = pb.finish() {
                    let stroke = Stroke { width: *width, ..Stroke::default() };
                    pm.stroke_path(&path, &paint, &stroke, base, None);
                }
            }
            Item::Quad { p: q, .. } => {
                let mut pb = PathBuilder::new();
                pb.move_to(q[0].x, q[0].y);
                for c in &q[1..] {
                    pb.line_to(c.x, c.y);
                }
                pb.close();
                if let Some(path) = pb.finish() {
                    pm.fill_path(&path, &paint, FillRule::Winding, base, None);
                }
            }
            Item::Crescent { from, c1, c2, to, d1, d2, .. } => {
                let mut pb = PathBuilder::new();
                let (a, b, c, d, e, f) = (p(from), p(c1), p(c2), p(to), p(d1), p(d2));
                pb.move_to(a.0, a.1);
                pb.cubic_to(b.0, b.1, c.0, c.1, d.0, d.1);
                pb.cubic_to(f.0, f.1, e.0, e.1, a.0, a.1);
                pb.close();
                if let Some(path) = pb.finish() {
                    pm.fill_path(&path, &paint, FillRule::Winding, base, None);
                }
            }
        }
    }
    pm
}
