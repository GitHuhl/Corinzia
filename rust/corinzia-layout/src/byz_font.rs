//! Neanes, the Byzantine notation font (SBMuFL), and its metadata: glyph
//! codepoints by name, advance widths, bounding boxes and the anchors that
//! place marks on neumes.
//!
//! Display lists store glyphs as codepoints. A Byzantine glyph carries the
//! [`BYZ`] bit so renderers draw it from Neanes rather than Bravura (both
//! use the Private Use Area). Its metadata is in ems; at `scale` 1 an em
//! is 4 staff spaces, as in Bravura, so the two fonts draw alike.

use crate::font::BBox;
use std::collections::HashMap;
use std::sync::OnceLock;

pub static BYZ_FONT_DATA: &[u8] = include_bytes!("../../../assets/fonts/neanes/Neanes.otf");
static METADATA: &str = include_str!("../../../assets/fonts/neanes/neanes.metadata.json");
static GLYPHNAMES: &str = include_str!("../../../assets/fonts/neanes/glyphnames.json");

/// Set on a display-list glyph drawn from Neanes.
pub const BYZ: u32 = 0x40_0000;

/// Staff spaces per em at scale 1.
pub const SPACES_PER_EM: f32 = 4.0;

pub struct ByzFont {
    codepoints: HashMap<String, u32>,
    advances: HashMap<String, f32>,
    boxes: HashMap<u32, BBox>,
    anchors: HashMap<String, HashMap<String, (f32, f32)>>,
}

fn pair(v: &serde_json::Value) -> Option<(f32, f32)> {
    Some((v.get(0)?.as_f64()? as f32, v.get(1)?.as_f64()? as f32))
}

impl ByzFont {
    pub fn get() -> &'static ByzFont {
        static FONT: OnceLock<ByzFont> = OnceLock::new();
        FONT.get_or_init(|| {
            let names: serde_json::Value = serde_json::from_str(GLYPHNAMES).expect("glyphnames.json");
            let meta: serde_json::Value = serde_json::from_str(METADATA).expect("neanes.metadata.json");
            let codepoints: HashMap<String, u32> = names
                .as_object()
                .expect("glyph names")
                .iter()
                .filter_map(|(k, v)| Some((k.clone(), u32::from_str_radix(v.get("codepoint")?.as_str()?.trim_start_matches("U+"), 16).ok()?)))
                .collect();
            let advances = meta["glyphAdvanceWidths"].as_object().into_iter().flatten().filter_map(|(k, v)| Some((k.clone(), v.as_f64()? as f32))).collect();
            let boxes = meta["glyphBBoxes"]
                .as_object()
                .into_iter()
                .flatten()
                .filter_map(|(k, v)| {
                    let (sw, ne) = (pair(&v["bBoxSW"])?, pair(&v["bBoxNE"])?);
                    Some((*codepoints.get(k)?, BBox { sw: (sw.0 * SPACES_PER_EM, sw.1 * SPACES_PER_EM), ne: (ne.0 * SPACES_PER_EM, ne.1 * SPACES_PER_EM) }))
                })
                .collect();
            let anchors = meta["glyphsWithAnchors"]
                .as_object()
                .into_iter()
                .flatten()
                .map(|(k, v)| (k.clone(), v.as_object().into_iter().flatten().filter_map(|(a, p)| Some((a.clone(), pair(p)?))).collect()))
                .collect();
            ByzFont { codepoints, advances, boxes, anchors }
        })
    }

    /// The display-list glyph (with [`BYZ`] set) of a glyph by name.
    pub fn glyph(&self, name: &str) -> Option<u32> {
        self.codepoints.get(name).map(|c| c | BYZ)
    }

    /// Advance width in ems.
    pub fn advance(&self, name: &str) -> f32 {
        self.advances.get(name).copied().unwrap_or(0.0)
    }

    /// An anchor in ems (y up).
    pub fn anchor(&self, name: &str, anchor: &str) -> Option<(f32, f32)> {
        self.anchors.get(name)?.get(anchor).copied()
    }

    /// Box of a display-list glyph in staff spaces at scale 1 (y up).
    pub fn bbox_of(&self, glyph: u32) -> BBox {
        self.boxes.get(&(glyph & !BYZ)).copied().unwrap_or_default()
    }
}

/// The font data and character a display-list glyph is drawn with.
pub fn glyph_source(glyph: u32) -> (&'static [u8], Option<char>) {
    if glyph & BYZ != 0 {
        (BYZ_FONT_DATA, char::from_u32(glyph & !BYZ))
    } else {
        (crate::font::FONT_DATA, char::from_u32(glyph))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_and_outlines_agree() {
        let f = ByzFont::get();
        let face = ttf_parser::Face::parse(BYZ_FONT_DATA, 0).unwrap();
        assert_eq!(face.units_per_em(), 1000);
        let oligon = f.glyph("oligon").unwrap();
        assert_eq!(oligon, 0xE001 | BYZ);
        let (data, ch) = glyph_source(oligon);
        assert!(ttf_parser::Face::parse(data, 0).unwrap().glyph_index(ch.unwrap()).is_some());
        assert!((f.advance("oligon") - 1.515).abs() < 1e-6);
        assert_eq!(f.anchor("klasmaAbove", "klasmaTop"), Some((0.084, 0.604)));
        assert!(f.bbox_of(oligon).width() > 5.0);
    }
}
