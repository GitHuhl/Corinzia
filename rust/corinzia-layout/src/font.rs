//! The SMuFL font (Bravura 1.392, vendored in assets/fonts/bravura): outlines
//! from the OpenType file, bounding boxes, anchors and engraving defaults from
//! its metadata. All metrics are in staff spaces (SMuFL: 1 em = 4 spaces).

use serde::Deserialize;
use std::collections::HashMap;
use std::sync::OnceLock;

pub static FONT_DATA: &[u8] = include_bytes!("../../../assets/fonts/bravura/Bravura.otf");
static METADATA: &str = include_str!("../../../assets/fonts/bravura/bravura_metadata.json");

/// The glyphs the engraver uses: (SMuFL name, codepoint).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[allow(missing_docs)]
pub enum Glyph {
    NoteheadBlack, NoteheadHalf, NoteheadWhole,
    GClef, FClef, CClef, GClef8vb, FClef8vb, PercussionClef,
    Flat, Natural, Sharp, DoubleSharp, DoubleFlat,
    RestWhole, RestHalf, RestQuarter, Rest8th, Rest16th, Rest32nd, Rest64th,
    Flag8thUp, Flag8thDown, Flag16thUp, Flag16thDown, Flag32ndUp, Flag32ndDown, Flag64thUp, Flag64thDown,
    Dot,
    TimeSigDigit(u8), TimeSigCommon, TimeSigCut,
    AccentAbove, AccentBelow, StaccatoAbove, StaccatoBelow, TenutoAbove, TenutoBelow,
    StaccatissimoAbove, StaccatissimoBelow, MarcatoAbove, MarcatoBelow, FermataAbove, FermataBelow,
    DynamicP, DynamicM, DynamicF,
    Trill, Mordent, Turn, BreathMark, Caesura, Tremolo3, Arpeggiato, RepeatDot,
    MetNoteQuarterUp, TupletDigit(u8),
    CsymSharp, CsymFlat, CsymNatural, CsymDiminished, CsymHalfDiminished, CsymMajorSeventh,
    BracketTop, BracketBottom,
}

impl Glyph {
    pub fn info(self) -> (&'static str, u32) {
        use Glyph::*;
        match self {
            NoteheadBlack => ("noteheadBlack", 0xE0A4),
            NoteheadHalf => ("noteheadHalf", 0xE0A3),
            NoteheadWhole => ("noteheadWhole", 0xE0A2),
            GClef => ("gClef", 0xE050),
            FClef => ("fClef", 0xE062),
            CClef => ("cClef", 0xE05C),
            GClef8vb => ("gClef8vb", 0xE052),
            FClef8vb => ("fClef8vb", 0xE064),
            PercussionClef => ("unpitchedPercussionClef1", 0xE069),
            Flat => ("accidentalFlat", 0xE260),
            Natural => ("accidentalNatural", 0xE261),
            Sharp => ("accidentalSharp", 0xE262),
            DoubleSharp => ("accidentalDoubleSharp", 0xE263),
            DoubleFlat => ("accidentalDoubleFlat", 0xE264),
            RestWhole => ("restWhole", 0xE4E3),
            RestHalf => ("restHalf", 0xE4E4),
            RestQuarter => ("restQuarter", 0xE4E5),
            Rest8th => ("rest8th", 0xE4E6),
            Rest16th => ("rest16th", 0xE4E7),
            Rest32nd => ("rest32nd", 0xE4E8),
            Rest64th => ("rest64th", 0xE4E9),
            Flag8thUp => ("flag8thUp", 0xE240),
            Flag8thDown => ("flag8thDown", 0xE241),
            Flag16thUp => ("flag16thUp", 0xE242),
            Flag16thDown => ("flag16thDown", 0xE243),
            Flag32ndUp => ("flag32ndUp", 0xE244),
            Flag32ndDown => ("flag32ndDown", 0xE245),
            Flag64thUp => ("flag64thUp", 0xE246),
            Flag64thDown => ("flag64thDown", 0xE247),
            Dot => ("augmentationDot", 0xE1E7),
            TimeSigDigit(d) => (
                ["timeSig0", "timeSig1", "timeSig2", "timeSig3", "timeSig4", "timeSig5", "timeSig6", "timeSig7", "timeSig8", "timeSig9"]
                    [d.min(9) as usize],
                0xE080 + d.min(9) as u32,
            ),
            TimeSigCommon => ("timeSigCommon", 0xE08A),
            TimeSigCut => ("timeSigCutCommon", 0xE08B),
            AccentAbove => ("articAccentAbove", 0xE4A0),
            AccentBelow => ("articAccentBelow", 0xE4A1),
            StaccatoAbove => ("articStaccatoAbove", 0xE4A2),
            StaccatoBelow => ("articStaccatoBelow", 0xE4A3),
            TenutoAbove => ("articTenutoAbove", 0xE4A4),
            TenutoBelow => ("articTenutoBelow", 0xE4A5),
            StaccatissimoAbove => ("articStaccatissimoAbove", 0xE4A6),
            StaccatissimoBelow => ("articStaccatissimoBelow", 0xE4A7),
            MarcatoAbove => ("articMarcatoAbove", 0xE4AC),
            MarcatoBelow => ("articMarcatoBelow", 0xE4AD),
            FermataAbove => ("fermataAbove", 0xE4C0),
            FermataBelow => ("fermataBelow", 0xE4C1),
            DynamicP => ("dynamicPiano", 0xE520),
            DynamicM => ("dynamicMezzo", 0xE521),
            DynamicF => ("dynamicForte", 0xE522),
            Trill => ("ornamentTrill", 0xE566),
            Mordent => ("ornamentMordent", 0xE56D),
            Turn => ("ornamentTurn", 0xE567),
            BreathMark => ("breathMarkComma", 0xE4CE),
            Caesura => ("caesura", 0xE4D1),
            Tremolo3 => ("tremolo3", 0xE222),
            Arpeggiato => ("wiggleArpeggiatoUp", 0xEAA9),
            RepeatDot => ("repeatDot", 0xE044),
            MetNoteQuarterUp => ("metNoteQuarterUp", 0xECA5),
            TupletDigit(d) => (
                ["tuplet0", "tuplet1", "tuplet2", "tuplet3", "tuplet4", "tuplet5", "tuplet6", "tuplet7", "tuplet8", "tuplet9"]
                    [d.min(9) as usize],
                0xE880 + d.min(9) as u32,
            ),
            CsymSharp => ("csymAccidentalSharp", 0xED62),
            CsymFlat => ("csymAccidentalFlat", 0xED60),
            CsymNatural => ("csymAccidentalNatural", 0xED61),
            CsymDiminished => ("csymDiminished", 0xE870),
            CsymHalfDiminished => ("csymHalfDiminished", 0xE871),
            CsymMajorSeventh => ("csymMajorSeventh", 0xE873),
            BracketTop => ("bracketTop", 0xE003),
            BracketBottom => ("bracketBottom", 0xE004),
        }
    }
    pub fn name(self) -> &'static str {
        self.info().0
    }

    /// Every glyph the engraver can emit.
    pub fn all() -> Vec<Glyph> {
        use Glyph::*;
        let mut v = vec![
            NoteheadBlack, NoteheadHalf, NoteheadWhole, GClef, FClef, CClef, GClef8vb, FClef8vb, PercussionClef, Flat, Natural,
            Sharp, DoubleSharp, DoubleFlat, RestWhole, RestHalf, RestQuarter, Rest8th, Rest16th, Rest32nd, Rest64th, Flag8thUp,
            Flag8thDown, Flag16thUp, Flag16thDown, Flag32ndUp, Flag32ndDown, Flag64thUp, Flag64thDown, Dot, TimeSigCommon,
            TimeSigCut, AccentAbove, AccentBelow, StaccatoAbove, StaccatoBelow, TenutoAbove, TenutoBelow, StaccatissimoAbove,
            StaccatissimoBelow, MarcatoAbove, MarcatoBelow, FermataAbove, FermataBelow, DynamicP, DynamicM, DynamicF, Trill,
            Mordent, Turn, BreathMark, Caesura, Tremolo3, Arpeggiato, RepeatDot, MetNoteQuarterUp, CsymSharp, CsymFlat,
            CsymNatural, CsymDiminished, CsymHalfDiminished, CsymMajorSeventh, BracketTop, BracketBottom,
        ];
        v.extend((0..10).map(TimeSigDigit));
        v.extend((0..10).map(TupletDigit));
        v
    }
    pub fn codepoint(self) -> u32 {
        self.info().1
    }
}

/// Glyph box in staff spaces, y up (SMuFL convention): sw = (left, bottom), ne = (right, top).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BBox {
    pub sw: (f32, f32),
    pub ne: (f32, f32),
}
impl BBox {
    pub fn width(&self) -> f32 {
        self.ne.0 - self.sw.0
    }
}

#[derive(Deserialize, Debug, Clone, Copy)]
#[serde(rename_all = "camelCase")]
pub struct EngravingDefaults {
    pub staff_line_thickness: f32,
    pub stem_thickness: f32,
    pub beam_thickness: f32,
    pub beam_spacing: f32,
    pub leger_line_thickness: f32,
    pub leger_line_extension: f32,
    pub thin_barline_thickness: f32,
    pub thick_barline_thickness: f32,
    pub barline_separation: f32,
    pub tie_endpoint_thickness: f32,
    pub tie_midpoint_thickness: f32,
    pub slur_endpoint_thickness: f32,
    pub slur_midpoint_thickness: f32,
    pub hairpin_thickness: f32,
    pub repeat_barline_dot_separation: f32,
    pub repeat_ending_line_thickness: f32,
    pub tuplet_bracket_thickness: f32,
}

#[derive(Deserialize)]
struct RawBox {
    #[serde(rename = "bBoxNE")]
    b_box_ne: [f32; 2],
    #[serde(rename = "bBoxSW")]
    b_box_sw: [f32; 2],
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawMetadata {
    engraving_defaults: EngravingDefaults,
    glyph_b_boxes: HashMap<String, RawBox>,
    glyphs_with_anchors: HashMap<String, HashMap<String, [f32; 2]>>,
}

pub struct Font {
    pub defaults: EngravingDefaults,
    boxes: HashMap<String, BBox>,
    by_codepoint: HashMap<u32, BBox>,
    anchors: HashMap<String, HashMap<String, [f32; 2]>>,
    pub units_per_space: f32,
}

impl Font {
    /// The vendored Bravura, parsed once.
    pub fn bravura() -> &'static Font {
        static FONT: OnceLock<Font> = OnceLock::new();
        FONT.get_or_init(|| {
            let raw: RawMetadata = serde_json::from_str(METADATA).expect("bravura_metadata.json");
            let face = ttf_parser::Face::parse(FONT_DATA, 0).expect("Bravura.otf");
            let boxes: HashMap<String, BBox> = raw
                .glyph_b_boxes
                .into_iter()
                .map(|(k, b)| (k, BBox { sw: (b.b_box_sw[0], b.b_box_sw[1]), ne: (b.b_box_ne[0], b.b_box_ne[1]) }))
                .collect();
            let by_codepoint = Glyph::all().iter().filter_map(|g| boxes.get(g.name()).map(|b| (g.codepoint(), *b))).collect();
            Font {
                defaults: raw.engraving_defaults,
                boxes,
                by_codepoint,
                anchors: raw.glyphs_with_anchors,
                units_per_space: face.units_per_em() as f32 / 4.0,
            }
        })
    }

    pub fn bbox(&self, g: Glyph) -> BBox {
        self.boxes.get(g.name()).copied().unwrap_or_default()
    }

    /// Box of a glyph by codepoint (display lists store codepoints).
    pub fn bbox_of(&self, codepoint: u32) -> BBox {
        self.by_codepoint.get(&codepoint).copied().unwrap_or_default()
    }

    /// An anchor (e.g. "stemUpSE"), staff spaces, y up.
    pub fn anchor(&self, g: Glyph, name: &str) -> Option<(f32, f32)> {
        self.anchors.get(g.name()).and_then(|a| a.get(name)).map(|p| (p[0], p[1]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_and_outlines_agree() {
        let f = Font::bravura();
        assert_eq!(f.units_per_space, 250.0);
        assert!((f.defaults.staff_line_thickness - 0.13).abs() < 1e-6);
        let head = f.bbox(Glyph::NoteheadBlack);
        assert!((head.width() - 1.18).abs() < 0.01, "{head:?}");
        assert_eq!(f.anchor(Glyph::NoteheadBlack, "stemUpSE"), Some((1.18, 0.168)));
        // Every glyph we use exists in the font and has metadata.
        let face = ttf_parser::Face::parse(FONT_DATA, 0).unwrap();
        let all = Glyph::all();
        for g in all {
            let ch = char::from_u32(g.codepoint()).unwrap();
            assert!(face.glyph_index(ch).is_some(), "{} missing from the font", g.name());
            assert!(f.bbox(g).width() > 0.0, "{} has no bbox", g.name());
        }
    }
}
