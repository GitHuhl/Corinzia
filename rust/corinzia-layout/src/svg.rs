//! Display list -> SVG, shaped like LilyPond's output so the Qt canvas can show
//! native engravings with its existing machinery (vector tiles, hit index,
//! instant-feedback layer): staves as `<g class="staff ln-N">` groups of
//! translated `<line>`s, and noteheads / rests in `<g id="e-<base36>">`
//! groups. Coordinates are staff spaces, shifted so that y >= 0.

use crate::display::{DisplayList, Item};
use crate::font::{Font, Glyph, FONT_DATA};
use std::collections::HashMap;
use std::fmt::Write;

/// Space above the first staff (marks, ledger lines), in staff spaces.
pub const TOP_PAD: f32 = 8.0;
const BOTTOM_PAD: f32 = 8.0;

pub fn svg_id(id: u64) -> String {
    const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut v = id;
    let mut buf = Vec::new();
    loop {
        buf.push(DIGITS[(v % 36) as usize]);
        v /= 36;
        if v == 0 {
            break;
        }
    }
    buf.reverse();
    format!("e-{}", String::from_utf8(buf).unwrap())
}

struct PathText {
    d: String,
    ups: f32,
}
impl ttf_parser::OutlineBuilder for PathText {
    fn move_to(&mut self, x: f32, y: f32) {
        let _ = write!(self.d, "M{:.3} {:.3}", x / self.ups, -y / self.ups);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        let _ = write!(self.d, "L{:.3} {:.3}", x / self.ups, -y / self.ups);
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let u = self.ups;
        let _ = write!(self.d, "Q{:.3} {:.3} {:.3} {:.3}", x1 / u, -y1 / u, x / u, -y / u);
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let u = self.ups;
        let _ = write!(self.d, "C{:.3} {:.3} {:.3} {:.3} {:.3} {:.3}", x1 / u, -y1 / u, x2 / u, -y2 / u, x / u, -y / u);
    }
    fn close(&mut self) {
        self.d.push('Z');
    }
}

fn is_hit_target(codepoint: u32) -> bool {
    use Glyph::*;
    [NoteheadBlack, NoteheadHalf, NoteheadWhole, RestWhole, RestHalf, RestQuarter, Rest8th, Rest16th, Rest32nd, Rest64th]
        .iter()
        .any(|g| g.codepoint() == codepoint)
}

pub fn to_svg(list: &DisplayList) -> String {
    let face = ttf_parser::Face::parse(FONT_DATA, 0).expect("font");
    let ups = Font::bravura().units_per_space;
    let mut paths: HashMap<u32, String> = HashMap::new();
    let mut glyph_path = |cp: u32| -> String {
        paths
            .entry(cp)
            .or_insert_with(|| {
                let mut pt = PathText { d: String::new(), ups };
                if let Some(gid) = char::from_u32(cp).and_then(|c| face.glyph_index(c)) {
                    face.outline_glyph(gid, &mut pt);
                }
                pt.d
            })
            .clone()
    };
    let oy = TOP_PAD;
    let (w, h) = (list.width, list.height + TOP_PAD + BOTTOM_PAD);
    let mut o = String::with_capacity(list.items.len() * 160);
    let _ = writeln!(
        o,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" version=\"1.2\" width=\"{:.2}mm\" height=\"{:.2}mm\" viewBox=\"0 0 {:.4} {:.4}\">",
        w * 1.75,
        h * 1.75,
        w,
        h
    );
    o.push_str("<rect x=\"0\" y=\"0\" width=\"100%\" height=\"100%\" fill=\"white\"/>\n");
    for st in &list.staves {
        let _ = writeln!(o, "<g class=\"staff ln-{}\">", st.part);
        for k in 0..5 {
            let _ = writeln!(
                o,
                "<g transform=\"translate({:.4}, {:.4})\"><line stroke-linecap=\"butt\" stroke-width=\"{:.4}\" stroke=\"currentColor\" x1=\"0\" y1=\"0\" x2=\"{:.4}\" y2=\"0\"/></g>",
                st.left,
                st.top + k as f32 + oy,
                st.line_width,
                st.right - st.left
            );
        }
        o.push_str("</g>\n");
    }
    for item in &list.items {
        match item {
            Item::Glyph { glyph, at, id } => {
                let d = glyph_path(*glyph);
                let path = format!("<path transform=\"translate({:.4}, {:.4})\" d=\"{}\"/>", at.x, at.y + oy, d);
                if *id != 0 && is_hit_target(*glyph) {
                    let _ = writeln!(o, "<g id=\"{}\" class=\"ev\">{}</g>", svg_id(*id), path);
                } else {
                    o.push_str(&path);
                    o.push('\n');
                }
            }
            Item::Line { from, to, width, .. } => {
                let _ = writeln!(
                    o,
                    "<line stroke-linecap=\"butt\" stroke-width=\"{:.4}\" stroke=\"black\" x1=\"{:.4}\" y1=\"{:.4}\" x2=\"{:.4}\" y2=\"{:.4}\"/>",
                    width,
                    from.x,
                    from.y + oy,
                    to.x,
                    to.y + oy
                );
            }
            Item::Quad { p, .. } => {
                let _ = writeln!(
                    o,
                    "<path d=\"M{:.4} {:.4}L{:.4} {:.4}L{:.4} {:.4}L{:.4} {:.4}Z\"/>",
                    p[0].x,
                    p[0].y + oy,
                    p[1].x,
                    p[1].y + oy,
                    p[2].x,
                    p[2].y + oy,
                    p[3].x,
                    p[3].y + oy
                );
            }
            Item::Crescent { from, c1, c2, to, d1, d2, .. } => {
                let _ = writeln!(
                    o,
                    "<path d=\"M{:.4} {:.4}C{:.4} {:.4} {:.4} {:.4} {:.4} {:.4}C{:.4} {:.4} {:.4} {:.4} {:.4} {:.4}Z\"/>",
                    from.x,
                    from.y + oy,
                    c1.x,
                    c1.y + oy,
                    c2.x,
                    c2.y + oy,
                    to.x,
                    to.y + oy,
                    d2.x,
                    d2.y + oy,
                    d1.x,
                    d1.y + oy,
                    from.x,
                    from.y + oy
                );
            }
        }
    }
    o.push_str("</svg>\n");
    o
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notation::score;
    use crate::{layout_line, Options};

    #[test]
    fn svg_ids_match_the_cpp_encoding() {
        assert_eq!(svg_id(0), "e-0");
        assert_eq!(svg_id(35), "e-z");
        assert_eq!(svg_id(36), "e-10");
    }

    #[test]
    fn staves_and_heads_are_tagged() {
        let s = score(&[("treble", &["c'4 r4 e'2"]), ("bass", &["c1"])], (4, 4), 0);
        let svg = to_svg(&layout_line(&s, &Options::default()));
        assert_eq!(svg.matches("class=\"staff ln-").count(), 2);
        assert_eq!(svg.matches("<line stroke-linecap=\"butt\" stroke-width=\"0.1300\" stroke=\"currentColor\"").count(), 10);
        // Two heads + one rest in staff 1, one head in staff 2 (whole-bar rests have no id).
        assert_eq!(svg.matches("class=\"ev\"").count(), 4);
        let head = &s.parts[0].voices[0].events[0].pitches[0];
        assert!(svg.contains(&format!("id=\"{}\"", svg_id(head.id))));
    }
}
