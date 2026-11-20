//! Articulations, ornaments, bowings, fermatas and fingering.
//!
//! Ported from GNU LilyPond (`scm/script.scm`,
//! `lily/side-position-interface.cc`), Copyright (C) 1997–2026 Han-Wen
//! Nienhuys, Jan Nieuwenhuizen and the LilyPond authors, under the GNU
//! General Public License, version 3 or later (see `THIRD-PARTY-NOTICES.md`).

use super::*;

impl<'a> Engraver<'a> {
    pub(super) fn draw_marks(&mut self) {
        for i in 0..self.notes.len() {
            let n = self.notes[i].clone();
            if !n.first || n.rest {
                continue;
            }
            let top = self.top(n.part);
            let poly = self.notes.iter().any(|o| o.part == n.part && o.measure == n.measure && o.voice != n.voice && !o.whole_bar);
            if !n.rest {
                // Close articulations as LilyPond's Script: on the notehead
                // side (side-relative-direction DOWN), or for the voice in
                // polyphony; stacked outwards by script-priority, each
                // `padding` clear of the note and of the scripts before it.
                // Staccato, staccatissimo, tenuto and marcato are quantized
                // into the staff's spaces; the rest keep clear of the staff
                // too (it joins their support), by at least staff-padding.
                let below = if poly { n.voice % 2 == 0 } else { n.stem_up };
                let dir = if below { 1.0 } else { -1.0 }; // y down
                let stem_side = n.has_stem() && n.stem_up != below;
                let outer = if below { n.heads[0].pos } else { n.heads[n.heads.len() - 1].pos };
                let mut edge = self.y(n.part, outer) + dir * 0.5;
                if stem_side {
                    let stem = self.notes[i].stem_end + if n.beamed { dir * 0.25 } else { 0.0 };
                    edge = if below { edge.max(stem) } else { edge.min(stem) };
                }
                let staff_edge = if below { top + 4.0 } else { top };
                // (bit, above, below, script-priority, padding, quantized)
                let mut list = [
                    (art::STACCATO, Glyph::StaccatoAbove, Glyph::StaccatoBelow, -100, 0.2, true),
                    (art::TENUTO, Glyph::TenutoAbove, Glyph::TenutoBelow, -50, 0.2, true),
                    (art::STACCATISSIMO, Glyph::StaccatissimoAbove, Glyph::StaccatissimoBelow, 0, 0.2, true),
                    (art::ACCENT, Glyph::AccentAbove, Glyph::AccentBelow, 0, 0.2, false),
                    (art::MARCATO, Glyph::MarcatoAbove, Glyph::MarcatoBelow, 0, 0.2, true),
                ];
                list.sort_by_key(|e| e.3);
                for (bit, up_g, down_g, _, padding, quantized) in list {
                    if n.art & bit == 0 {
                        continue;
                    }
                    let g = if below { down_g } else { up_g };
                    let bb = self.font.bbox(g);
                    // Glyph extent below and above its origin (y down).
                    let (asc, desc) = (bb.ne.1, -bb.sw.1);
                    let half = (asc + desc) / 2.0;
                    let mut near = edge + dir * padding;
                    if !quantized {
                        near = if below { near.max(staff_edge + 0.25) } else { near.min(staff_edge - 0.25) };
                    }
                    let mut centre = near + dir * half;
                    if quantized {
                        // Side_position_interface's quantize-position.
                        let p = (top + 2.0 - centre) * 2.0;
                        if p.abs() <= 5.0 {
                            let mut r = if below { p.floor() } else { p.ceil() };
                            if (r as i32) % 2 == 0 && r.abs() <= 4.0 {
                                r -= dir;
                            }
                            centre = top + 2.0 - r / 2.0;
                        }
                    }
                    // Toward the stem when on its side (toward-stem-shift).
                    let cx = if stem_side && matches!(bit, art::STACCATO | art::STACCATISSIMO) { n.stem_x } else { n.x + HEAD_W / 2.0 };
                    let x = cx - (bb.sw.0 + bb.ne.0) / 2.0;
                    let y = centre + (asc - desc) / 2.0; // origin from centre
                    self.push(Item::glyph(g, x, y, n.id));
                    edge = centre + dir * half;
                }
                if n.art & art::TREMOLO != 0 && n.has_stem() {
                    // Through the stem, midway along its free part (from the
                    // head at its root to its end). SMuFL's tremolo glyphs
                    // are centred on their origin.
                    let g = Glyph::Tremolo3;
                    let b = self.font.bbox(g);
                    let root = if n.stem_up { n.heads[n.heads.len() - 1].pos } else { n.heads[0].pos };
                    let head_edge = self.y(n.part, root) + if n.stem_up { -0.5 } else { 0.5 };
                    let centre = (self.notes[i].stem_end + head_edge) / 2.0;
                    let x = n.stem_x - (b.sw.0 + b.ne.0) / 2.0;
                    let y = centre + (b.ne.1 + b.sw.1) / 2.0;
                    self.push(Item::glyph(g, x, y, n.id));
                }
                if n.art & art::ARPEGGIO != 0 && n.heads.len() > 1 {
                    let (ylo, yhi) = (self.y(n.part, n.heads[0].pos) + 0.5, self.y(n.part, n.heads[n.heads.len() - 1].pos) - 0.5);
                    let seg = self.font.bbox(Glyph::Arpeggiato).width().max(0.5);
                    // The wiggle segment is horizontal: stack it vertically via rotated segments is renderer
                    // work; draw it as a vertical column of short strokes instead.
                    let mut yy = ylo;
                    let x = n.x - 1.0 + n.heads.iter().map(|h| h.acc_x.min(0.0)).fold(0.0f32, f32::min);
                    while yy > yhi {
                        self.push(Item::line(x, yy, x + 0.25, yy - seg / 2.0, 0.14, n.id));
                        self.push(Item::line(x + 0.25, yy - seg / 2.0, x, yy - seg, 0.14, n.id));
                        yy -= seg;
                    }
                }
                if n.art & (art::BREATH | art::CAESURA) != 0 {
                    let g = if n.art & art::BREATH != 0 { Glyph::BreathMark } else { Glyph::Caesura };
                    let x = n.x + HEAD_W + 0.6 + n.value.dots as f32 * 0.5;
                    let y = if g == Glyph::BreathMark { top - 0.5 } else { top + 1.0 };
                    self.push(Item::glyph(g, x, y, n.id));
                }
            }
        }
    }

    /// Fingering just above the note (above the stem tip when the stem is
    /// up), one digit per head stacked from the bottom head's.
    pub(super) fn draw_fingering(&mut self) {
        for i in 0..self.notes.len() {
            let n = self.notes[i].clone();
            if !n.first || n.rest || n.heads.is_empty() {
                continue;
            }
            let Some(e) = self.score.parts.get(n.part).and_then(|p| p.voices.iter().find(|v| v.index == n.voice)).and_then(|v| v.events.iter().find(|e| e.id == n.id || e.pitches.first().is_some_and(|p| p.id == n.heads[0].id))) else { continue };
            let fingers: Vec<u8> = e.fingering.iter().copied().filter(|&f| f > 0).collect();
            if fingers.is_empty() {
                continue;
            }
            let top = self.y(n.part, n.heads[n.heads.len() - 1].pos);
            let mut y = if n.stem_up && n.has_stem() { n.stem_end.min(top) - 0.5 } else { top - 0.9 };
            // Clear of a beam or flag at the stem tip.
            if n.stem_up && n.beamed {
                y -= 0.6;
            }
            // Bottom head's finger lowest.
            for f in fingers.iter() {
                let g = Glyph::Fingering(*f);
                let b = self.font.bbox(g);
                let x = n.x + HEAD_W / 2.0 - b.width() / 2.0;
                self.place(n.part, Item::glyph(g, x, y, n.heads[0].id));
                y -= b.height() + 0.15;
            }
        }
    }

    /// Fermatas and ornaments above the note, clear of everything there.
    pub(super) fn draw_ornaments(&mut self) {
        for i in 0..self.notes.len() {
            let n = self.notes[i].clone();
            if !n.first || n.rest {
                continue;
            }
            // LilyPond's script-priority order outwards (turn and mordent
            // 0, trill 150, fermata 175, bowings 180), each `padding` above
            // what is there (0.4 for the fermata, 0.2 for the rest) and
            // staff-padding (0.25) above the staff.
            for (bit, g, padding) in [
                (art::MORDENT, Glyph::Mordent, 0.2),
                (art::TURN, Glyph::Turn, 0.2),
                (art::TRILL, Glyph::Trill, 0.2),
                (art::FERMATA, Glyph::FermataAbove, 0.4),
                (art::DOWN_BOW, Glyph::DownBow, 0.2),
                (art::UP_BOW, Glyph::UpBow, 0.2),
            ] {
                if n.art & bit == 0 {
                    continue;
                }
                let b = self.font.bbox(g);
                let x = n.x + HEAD_W / 2.0 - (b.sw.0 + b.ne.0) / 2.0;
                let (x0, x1) = (x + b.sw.0, x + b.ne.0);
                let top = self.top(n.part);
                let objects = self.sky[n.part].objects_over(x0, x1).map(|r| r[1]).fold(f32::MAX, f32::min);
                let bottom = (objects - padding).min(top - padding.max(0.25));
                let y = bottom + b.sw.1;
                self.place(n.part, Item::glyph(g, x, y, n.id));
            }
        }
    }
}
