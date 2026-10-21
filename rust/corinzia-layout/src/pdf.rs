//! Engraved pages as a vector PDF: every glyph and text run is written as
//! its outline, so the file needs no embedded fonts and matches the screen
//! exactly. Text uses the fonts given to `display::set_text_font`; without
//! them text is left out (as in the raster previews).

use crate::display::{text_fonts, DisplayList, Item};
use crate::font::{Font, FONT_DATA};
use pdf_writer::types::LineCapStyle;
use pdf_writer::{Content, Finish, Pdf, Rect, Ref, TextStr};

/// Points per staff space: the page's own, or as wide as A4.
fn pt_per_space(page: &DisplayList) -> f32 {
    if page.space_pt > 0.0 {
        page.space_pt
    } else {
        595.28 / page.width.max(1.0)
    }
}

/// Writes outlines through a mapping from font units to page points.
struct Outline<'a, F: Fn(f32, f32) -> (f32, f32)> {
    c: &'a mut Content,
    map: F,
    last: (f32, f32),
}
impl<F: Fn(f32, f32) -> (f32, f32)> ttf_parser::OutlineBuilder for Outline<'_, F> {
    fn move_to(&mut self, x: f32, y: f32) {
        let p = (self.map)(x, y);
        self.c.move_to(p.0, p.1);
        self.last = p;
    }
    fn line_to(&mut self, x: f32, y: f32) {
        let p = (self.map)(x, y);
        self.c.line_to(p.0, p.1);
        self.last = p;
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        // Quadratic -> cubic.
        let (q, p) = ((self.map)(x1, y1), (self.map)(x, y));
        let c1 = (self.last.0 + 2.0 / 3.0 * (q.0 - self.last.0), self.last.1 + 2.0 / 3.0 * (q.1 - self.last.1));
        let c2 = (p.0 + 2.0 / 3.0 * (q.0 - p.0), p.1 + 2.0 / 3.0 * (q.1 - p.1));
        self.c.cubic_to(c1.0, c1.1, c2.0, c2.1, p.0, p.1);
        self.last = p;
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let (a, b, p) = ((self.map)(x1, y1), (self.map)(x2, y2), (self.map)(x, y));
        self.c.cubic_to(a.0, a.1, b.0, b.1, p.0, p.1);
        self.last = p;
    }
    fn close(&mut self) {
        self.c.close_path();
    }
}

fn page_content(page: &DisplayList) -> Vec<u8> {
    let k = pt_per_space(page);
    let height = page.height * k;
    // Staff spaces (y down) -> points (y up).
    let to = move |x: f32, y: f32| (x * k, height - y * k);
    let mut c = Content::new();
    c.set_line_cap(LineCapStyle::ButtCap);
    c.set_fill_gray(0.0);
    c.set_stroke_gray(0.0);
    let music = ttf_parser::Face::parse(FONT_DATA, 0).expect("Bravura");
    let ups = Font::bravura().units_per_space;
    let (regular, bold) = text_fonts();
    let regular = regular.and_then(|b| ttf_parser::Face::parse(b, 0).ok());
    let bold = bold.and_then(|b| ttf_parser::Face::parse(b, 0).ok());
    let stroke = |c: &mut Content, a: (f32, f32), b: (f32, f32), w: f32| {
        c.set_line_width(w * k);
        c.move_to(a.0, a.1);
        c.line_to(b.0, b.1);
        c.stroke();
    };
    for st in &page.staves {
        for l in 0..5 {
            let y = st.top + l as f32;
            stroke(&mut c, to(st.left, y), to(st.right, y), st.line_width);
        }
    }
    for item in &page.items {
        match item {
            Item::Glyph { glyph, at, scale, .. } => {
                let Some(gid) = char::from_u32(*glyph).and_then(|ch| music.glyph_index(ch)) else { continue };
                let s = scale / ups;
                let (ax, ay) = (at.x, at.y);
                let mut o = Outline { c: &mut c, map: |x: f32, y: f32| to(ax + x * s, ay - y * s), last: (0.0, 0.0) };
                if music.outline_glyph(gid, &mut o).is_some() {
                    c.fill_nonzero();
                }
            }
            Item::Text { text, at, size, italic, bold: is_bold, anchor, .. } => {
                let Some(face) = (if *is_bold { bold.as_ref().or(regular.as_ref()) } else { regular.as_ref() }) else { continue };
                let u = size / face.units_per_em() as f32;
                let width: f32 = text.chars().filter_map(|ch| face.glyph_index(ch)).filter_map(|g| face.glyph_hor_advance(g)).map(|a| a as f32 * u).sum();
                let mut x = at.x - width * (*anchor as f32) / 2.0;
                let slant = if *italic { 0.2 } else { 0.0 };
                for ch in text.chars() {
                    let Some(gid) = face.glyph_index(ch) else { continue };
                    let (gx, gy) = (x, at.y);
                    let mut o = Outline { c: &mut c, map: |fx: f32, fy: f32| to(gx + (fx + fy * slant) * u, gy - fy * u), last: (0.0, 0.0) };
                    if face.outline_glyph(gid, &mut o).is_some() {
                        c.fill_nonzero();
                    }
                    x += face.glyph_hor_advance(gid).unwrap_or(0) as f32 * u;
                }
            }
            Item::Line { from, to: end, width, .. } => stroke(&mut c, to(from.x, from.y), to(end.x, end.y), *width),
            Item::Quad { p, .. } => {
                let a = to(p[0].x, p[0].y);
                c.move_to(a.0, a.1);
                for q in &p[1..] {
                    let b = to(q.x, q.y);
                    c.line_to(b.0, b.1);
                }
                c.close_path();
                c.fill_nonzero();
            }
            Item::Crescent { from, c1, c2, to: end, d1, d2, .. } => {
                let pts = [from, c1, c2, end, d2, d1].map(|p| to(p.x, p.y));
                c.move_to(pts[0].0, pts[0].1);
                c.cubic_to(pts[1].0, pts[1].1, pts[2].0, pts[2].1, pts[3].0, pts[3].1);
                c.cubic_to(pts[4].0, pts[4].1, pts[5].0, pts[5].1, pts[0].0, pts[0].1);
                c.close_path();
                c.fill_nonzero();
            }
        }
    }
    c.finish()
}

/// A PDF with one page per display list (A4 width; height from the list).
pub fn to_pdf(pages: &[DisplayList], title: &str) -> Vec<u8> {
    let mut pdf = Pdf::new();
    let (catalog, tree, info) = (Ref::new(1), Ref::new(2), Ref::new(3));
    let ids: Vec<(Ref, Ref)> = (0..pages.len() as i32).map(|i| (Ref::new(10 + 2 * i), Ref::new(11 + 2 * i))).collect();
    pdf.catalog(catalog).pages(tree);
    pdf.pages(tree).kids(ids.iter().map(|p| p.0)).count(pages.len() as i32);
    pdf.document_info(info).title(TextStr(title)).creator(TextStr("Corinzia"));
    for (page, (page_id, content_id)) in pages.iter().zip(&ids) {
        let k = pt_per_space(page);
        let mut p = pdf.page(*page_id);
        p.media_box(Rect::new(0.0, 0.0, page.width * k, page.height * k));
        p.parent(tree);
        p.contents(*content_id);
        p.finish();
        pdf.stream(*content_id, &page_content(page));
    }
    pdf.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{layout_pages, notation, Options, PageSpec};

    #[test]
    fn writes_a_pdf_page_per_page() {
        let score = notation::score(&[("treble", &["c'4 d' e' f' g'1"]), ("bass", &["c1 g,1"])], (4, 4), 0);
        let pages = layout_pages(&score, &Options::default(), &PageSpec::default());
        let bytes = to_pdf(&pages, "Test");
        assert!(bytes.starts_with(b"%PDF-"));
        let s = String::from_utf8_lossy(&bytes);
        let page_objects = s.matches("/Type /Page").count() - s.matches("/Type /Pages").count();
        assert_eq!(page_objects, pages.len());
        assert!(s.contains("/MediaBox"));
        assert!(bytes.len() > 4000, "outlines written: {} bytes", bytes.len());
    }
}
