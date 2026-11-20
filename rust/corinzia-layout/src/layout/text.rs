//! Lines (octave, pedal, cues), text, chord symbols, lyrics, tempo marks and
//! endings.

use super::*;

pub(super) const TEXT_SIZE: f32 = 2.0;

pub(super) const TEMPO_SIZE: f32 = 2.2;

pub(super) const LYRIC_SIZE: f32 = 2.0;

pub(super) const CHORD_SIZE: f32 = 2.3;

/// Space between two words of a verse; room for a hyphen inside a word.
pub(super) const WORD_GAP: f32 = 0.9;

pub(super) const HYPHEN_GAP: f32 = 2.0;

pub(super) const CHORD_GLYPH_SCALE: f32 = 0.55;

pub(super) fn chord_runs(text: &str) -> Vec<ChordRun> {
    let chars: Vec<char> = text.chars().collect();
    let mut out: Vec<ChordRun> = Vec::new();
    let mut buf = String::new();
    let flush = |buf: &mut String, out: &mut Vec<ChordRun>| {
        if !buf.is_empty() {
            out.push(ChordRun::Text(std::mem::take(buf)));
        }
    };
    for (i, &c) in chars.iter().enumerate() {
        let prev = if i > 0 { Some(chars[i - 1]) } else { None };
        let next = chars.get(i + 1).copied();
        let after_letter = prev.is_some_and(|p| ('A'..='G').contains(&p));
        let before_digit = next.is_some_and(|n| n.is_ascii_digit());
        let glyph = match c {
            // Accidentals centred on the capitals; qualities raised like exponents.
            '♯' => Some((Glyph::CsymSharp, 1.0)),
            '♭' => Some((Glyph::CsymFlat, 0.9)),
            '♮' => Some((Glyph::CsymNatural, 1.0)),
            '#' if after_letter || before_digit => Some((Glyph::CsymSharp, 1.0)),
            'b' if after_letter || before_digit => Some((Glyph::CsymFlat, 0.9)),
            '°' => Some((Glyph::CsymDiminished, 1.35)),
            'ø' => Some((Glyph::CsymHalfDiminished, 1.35)),
            'Δ' => Some((Glyph::CsymMajorSeventh, 1.2)),
            _ => None,
        };
        match glyph {
            Some((g, raise)) => {
                flush(&mut buf, &mut out);
                out.push(ChordRun::Glyph(g, raise));
            }
            None => buf.push(c),
        }
    }
    flush(&mut buf, &mut out);
    out
}

impl<'a> Engraver<'a> {
    /// Octave lines (`pedal` false) or pedal marks over this system: an
    /// octave line is its sign and a dashed line to the last note, with a
    /// hook where it ends (above for 8va/15ma, below for 8vb/15mb); a pedal
    /// is Ped. under its first note and the release sign at its end.
    pub(super) fn draw_lines(&mut self, pedal: bool) {
        let (r0, r1) = self.range;
        for (pi, part) in self.score.parts.iter().enumerate() {
            for l in part.lines.iter().filter(|l| (l.kind == LineKind::Pedal) == pedal && l.from < r1 && l.to > r0) {
                let notes: Vec<&Note> = self.notes.iter().filter(|n| n.part == pi && n.start >= l.from && n.start < l.to && !n.whole_bar).collect();
                let starts = l.from >= r0;
                let ends = l.to <= r1;
                let x0 = if starts { self.col_x(l.from) } else { self.ms.first().map(|m| m.x0).unwrap_or(self.left) };
                let last_note = notes.iter().map(|n| n.x + HEAD_W).fold(x0 + HEAD_W, f32::max);
                let x1 = if ends { last_note } else { self.ms.last().map(|m| m.x1).unwrap_or(x0) };
                let id = l.id;
                match l.kind {
                    LineKind::Pedal => {
                        let (ped, up) = (self.font.bbox(Glyph::PedalPed), self.font.bbox(Glyph::PedalUp));
                        // Released at its end: before the barline when it ends a bar.
                        let on_bar = self.ms.iter().any(|m| m.start + m.len == l.to);
                        let at_end = if on_bar { self.bar_x(l.to) - up.width() - 0.4 } else { self.col_x(l.to) - up.width() / 2.0 };
                        let after_sign = if starts { x0 - 0.2 + ped.width() + 0.6 } else { x0 };
                        let release = if ends { at_end.max(last_note - 0.2).max(after_sign) } else { x1 };
                        let y = self.sky[pi].bottom_over(x0, release + up.width()) + 0.6 + ped.ne.1;
                        if starts {
                            self.place(pi, Item::glyph(Glyph::PedalPed, x0 - 0.2, y, id));
                        }
                        if ends {
                            self.place(pi, Item::glyph(Glyph::PedalUp, release, y, id));
                        }
                    }
                    LineKind::Cue if part.audio => {
                        // An audio clip: its file's name in bold in a box
                        // above the line, then a heavy line along the
                        // staff's line to where it ends, with a stroke there.
                        let Some(t) = part.texts.iter().find(|t| t.id == id) else { continue };
                        let x0 = if starts { self.col_x(l.from) } else { x0 };
                        let x1 = if ends { self.col_x(l.to.min(r1)).max(x0 + 1.0) } else { self.ms.last().map(|m| m.x1).unwrap_or(x0) };
                        let mid = self.top(pi) + 2.0;
                        if starts {
                            let w = text_width(&t.text, TEXT_SIZE, true);
                            let (pad, h) = (0.5, 0.72 * TEXT_SIZE);
                            let (bx0, bx1, by1) = (x0, x0 + w + 2.0 * pad, mid - 0.5);
                            let by0 = by1 - h - 2.0 * pad;
                            let base = by1 - pad - 0.12 * TEXT_SIZE;
                            self.place(pi, Item::Text { text: t.text.clone(), at: pt(bx0 + pad, base), size: TEXT_SIZE, italic: false, bold: true, anchor: 0, id });
                            let bw = 0.12;
                            for (a, b, c, d) in [(bx0, by0, bx1, by0), (bx1, by0, bx1, by1), (bx1, by1, bx0, by1), (bx0, by1, bx0, by0)] {
                                self.place(pi, Item::line(a, b, c, d, bw, id));
                            }
                            self.place(pi, Item::line(x0, by1, x0, mid + 1.0, bw, id));
                        }
                        self.place(pi, Item::line(x0, mid, x1, mid, 0.4, id));
                        if ends {
                            self.place(pi, Item::line(x1, mid - 1.0, x1, mid + 1.0, 0.12, id));
                        }
                    }
                    LineKind::Cue => {
                        // The clip's name, then a line to its end, hooked down.
                        let Some(t) = part.texts.iter().find(|t| t.id == id) else { continue };
                        let x0 = if starts { self.col_x(l.from) } else { x0 };
                        let x1 = if ends { self.col_x(l.to.min(r1)).max(x0 + 1.0) } else { x1 };
                        let w = if starts { text_width(&t.text, TEXT_SIZE, false) } else { 0.0 };
                        let base = self.sky[pi].top_over(x0, x1) - 0.4 - 0.22 * TEXT_SIZE;
                        if starts {
                            self.place(pi, Item::Text { text: t.text.clone(), at: pt(x0, base), size: TEXT_SIZE, italic: false, bold: false, anchor: 0, id });
                        }
                        let ly = base - 0.3 * TEXT_SIZE;
                        let from = x0 + w + if starts { 0.4 } else { 0.0 };
                        if from < x1 {
                            self.place(pi, Item::line(from, ly, x1, ly, 0.12, id));
                        }
                        if ends {
                            self.place(pi, Item::line(x1, ly, x1, ly + 0.9, 0.12, id));
                        }
                    }
                    LineKind::Octave(k) => {
                        let above = k > 0;
                        let g = match k {
                            1 => Glyph::Ottava,
                            -1 => Glyph::OttavaBassa,
                            2 => Glyph::Quindicesima,
                            _ => Glyph::QuindicesimaBassa,
                        };
                        let b = self.font.bbox(g);
                        let y = if above { self.sky[pi].top_over(x0, x1) - 0.5 + b.sw.1 } else { self.sky[pi].bottom_over(x0, x1) + 0.5 + b.ne.1 };
                        let sx = x0 - 0.3;
                        self.place(pi, Item::glyph(g, sx, y, id));
                        // The dashed line at the sign's middle.
                        let ly = y - (b.ne.1 + b.sw.1) / 2.0;
                        let t = 0.12;
                        let mut x = sx + b.width() + 0.3;
                        while x < x1 {
                            let e = (x + 0.6).min(x1);
                            self.place(pi, Item::line(x, ly, e, ly, t, id));
                            x += 1.0;
                        }
                        if ends {
                            let hook = if above { 0.9 } else { -0.9 };
                            self.place(pi, Item::line(x1, ly, x1, ly + hook, t, id));
                        }
                    }
                }
            }
        }
    }

    /// Technique text above (`above`), expression text below, in italics.
    pub(super) fn draw_texts(&mut self, above: bool) {
        for (pi, part) in self.score.parts.iter().enumerate() {
            // A cue's text is drawn with its line.
            let cued = |id: u64| part.lines.iter().any(|l| l.kind == LineKind::Cue && l.id == id);
            for t in part.texts.iter().filter(|t| t.technique == above && t.at >= self.range.0 && t.at < self.range.1 && !cued(t.id)) {
                let x = self.col_x(t.at);
                let w = text_width(&t.text, TEXT_SIZE, false);
                let y = if above {
                    self.sky[pi].top_over(x, x + w) - 0.4 - 0.22 * TEXT_SIZE
                } else {
                    self.sky[pi].bottom_over(x, x + w) + 0.4 + 0.72 * TEXT_SIZE
                };
                let item = Item::Text { text: t.text.clone(), at: pt(x, y), size: TEXT_SIZE, italic: !above, bold: false, anchor: 0, id: t.id };
                let r = item.bounds(self.font);
                self.sky[pi].add(r);
                self.out.items.push(item);
            }
        }
    }

    /// Chord symbols: text with Bravura's chord-symbol accidentals and
    /// qualities, on a common baseline above each staff of the system.
    pub(super) fn draw_chord_symbols(&mut self) {
        for pi in 0..self.score.parts.len() {
            let chords: Vec<_> = self.score.parts[pi].chords.iter().filter(|c| c.at >= self.range.0 && c.at < self.range.1).cloned().collect();
            if chords.is_empty() {
                continue;
            }
            let runs: Vec<(f32, Vec<ChordRun>, u64)> = chords.iter().map(|c| (self.col_x(c.at), chord_runs(&c.text), c.id)).collect();
            let width = |r: &[ChordRun]| r.iter().map(|x| self.chord_run_width(x)).sum::<f32>();
            let top = runs.iter().map(|(x, r, _)| self.sky[pi].top_over(*x, x + width(r))).fold(self.top(pi), f32::min);
            let y = top - 0.8;
            for (x0, r, id) in runs {
                let mut x = x0;
                for run in r {
                    let w = self.chord_run_width(&run);
                    let item = match run {
                        ChordRun::Text(t) => Item::Text { text: t, at: pt(x, y), size: CHORD_SIZE, italic: false, bold: false, anchor: 0, id },
                        ChordRun::Glyph(g, centre) => {
                            // The glyph's box centred `centre` above the baseline.
                            let b = self.font.bbox(g);
                            let cy = (b.sw.1 + b.ne.1) / 2.0 * CHORD_GLYPH_SCALE;
                            Item::Glyph { glyph: g.codepoint(), at: pt(x + 0.05 - b.sw.0 * CHORD_GLYPH_SCALE, y - centre + cy), scale: CHORD_GLYPH_SCALE, id }
                        }
                    };
                    self.place(pi, item);
                    x += w;
                }
            }
        }
    }

    pub(super) fn chord_run_width(&self, r: &ChordRun) -> f32 {
        match r {
            ChordRun::Text(t) => text_width(t, CHORD_SIZE, false),
            ChordRun::Glyph(g, _) => self.width(*g) * CHORD_GLYPH_SCALE + 0.15,
        }
    }

    /// Lyrics: one line per verse under each staff, below everything drawn
    /// so far in the system; syllables centred under their notes, hyphens
    /// centred between the syllables of a word, extenders through melismas.
    pub(super) fn draw_lyrics(&mut self) {
        for pi in 0..self.score.parts.len() {
            let idx: Vec<usize> = (0..self.notes.len()).filter(|&i| self.notes[i].part == pi && !self.notes[i].lyrics.is_empty()).collect();
            let Some(verses) = idx.iter().flat_map(|&i| self.notes[i].lyrics.iter().map(|l| l.verse)).max() else { continue };
            let (x0, x1) = (self.left, self.out.width);
            let base = (self.sky[pi].bottom_over(x0, x1) + 0.8 + 0.75 * LYRIC_SIZE).max(self.top(pi) + 4.0 + 2.6);
            let mut boxes = Vec::new();
            for v in 1..=verses {
                let y = base + (v - 1) as f32 * LYRIC_SIZE * 1.2;
                let mut syl: Vec<(usize, corinzia_score::score::Lyric)> = idx.iter().filter_map(|&i| self.notes[i].lyrics.iter().find(|l| l.verse == v).map(|l| (i, l.clone()))).collect();
                syl.sort_by(|a, b| (self.notes[a.0].voice, self.notes[a.0].start).cmp(&(self.notes[b.0].voice, self.notes[b.0].start)));
                for (k, (i, l)) in syl.iter().enumerate() {
                    let n = &self.notes[*i];
                    let cx = n.x + HEAD_W / 2.0;
                    let w = text_width(&l.text, LYRIC_SIZE, false);
                    let right = cx + w / 2.0;
                    let item = Item::Text { text: l.text.clone(), at: pt(cx, y), size: LYRIC_SIZE, italic: false, bold: false, anchor: 1, id: ids::lyric(n.id, l.verse) };
                    boxes.push(item.bounds(self.font));
                    self.out.items.push(item);
                    let next = syl.get(k + 1).filter(|(j, _)| self.notes[*j].voice == n.voice);
                    let next_left = next.map(|(j, nl)| self.notes[*j].x + HEAD_W / 2.0 - text_width(&nl.text, LYRIC_SIZE, false) / 2.0);
                    if l.hyphen {
                        // Centred in the gap; at the end of the system when the word goes on in the next one.
                        let hx = match next_left {
                            Some(nl) if nl - right > 0.7 => (right + nl) / 2.0,
                            Some(_) => continue,
                            None => right + 0.9,
                        };
                        self.out.items.push(Item::Text { text: "-".into(), at: pt(hx, y), size: LYRIC_SIZE, italic: false, bold: false, anchor: 1, id: 0 });
                    } else if l.extend {
                        // Through the following notes of the voice up to the next syllable.
                        let until = next.map(|(j, _)| self.notes[*j].start).unwrap_or(self.range.1);
                        let last = self.notes.iter().filter(|o| o.part == pi && o.voice == n.voice && !o.rest && o.start > n.start && o.start < until).map(|o| o.x + HEAD_W).fold(n.x + HEAD_W, f32::max);
                        if last > right + 0.5 {
                            self.out.items.push(Item::line(right + 0.25, y + 0.15, last, y + 0.15, 0.12, 0));
                        }
                    }
                }
            }
            for b in boxes {
                self.sky[pi].add(b);
            }
        }
    }

    /// Above the top staff: volta brackets, rehearsal marks, tempo marks.
    pub(super) fn draw_tempo_marks_endings(&mut self) {
        if self.sky.is_empty() {
            return;
        }
        let d = self.font.defaults;
        // Endings.
        let (r0, r1) = self.range;
        for e in self.score.endings.clone() {
            if e.to <= r0 || e.from >= r1 {
                continue;
            }
            let starts_here = e.from >= r0;
            let (x0, x1) = (self.bar_x(e.from.max(r0)) + if starts_here { 0.3 } else { 0.0 }, self.bar_x(e.to.min(r1)) - 0.3);
            if x1 <= x0 {
                continue;
            }
            let label: Vec<String> = e.numbers.iter().map(|n| format!("{n}.")).collect();
            let label = label.join(", ");
            let lw = text_width(&label, 1.6, false);
            // The line clears everything under it; the label (hanging below the
            // line at the start) clears what is under the label.
            let y = (self.sky[0].top_over(x0, x1) - 0.8)
                .min(self.sky[0].top_over(x0, x0 + lw + 0.8) - 2.2)
                .min(self.top(0) - 2.5);
            let w = d.repeat_ending_line_thickness;
            let closed = e.to <= r1 && self.score.barlines.iter().any(|b| b.at == e.to && matches!(b.kind, BarKind::RepeatEnd | BarKind::RepeatBoth));
            if starts_here {
                self.place(0, Item::line(x0, y + 2.0, x0, y, w, 0));
            }
            self.place(0, Item::line(x0, y, x1, y, w, 0));
            if closed {
                self.place(0, Item::line(x1, y, x1, y + 2.0, w, 0));
            }
            if starts_here {
                self.place(0, Item::Text { text: label, at: pt(x0 + 0.4, y + 1.6), size: 1.6, italic: false, bold: false, anchor: 0, id: 0 });
            }
        }
        // Rehearsal marks: boxed letters at their barline.
        let mut marks = self.score.marks.clone();
        marks.sort();
        // The bar number at the start of every system but the first, above
        // the clef.
        if !self.first_system {
            if let Some(m) = self.ms.first() {
                let text = m.number.to_string();
                let w = text_width(&text, BAR_NUMBER_SIZE, false);
                let x = self.left;
                let y = (self.sky[0].top_over(x, x + w) - 0.5).min(self.top(0) - 1.0);
                self.place(0, Item::Text { text, at: pt(x, y), size: BAR_NUMBER_SIZE, italic: false, bold: false, anchor: 0, id: 0 });
            }
        }
        for (k, at) in marks.iter().enumerate().filter(|(_, at)| **at >= r0 && **at < r1) {
            let letter: String = std::iter::repeat_n((b'A' + (k % 26) as u8) as char, k / 26 + 1).collect();
            let size = 2.4;
            let w = text_width(&letter, size, true);
            let x = self.bar_x(*at);
            let (bx0, bx1) = (x - w / 2.0 - 0.4, x + w / 2.0 + 0.4);
            let bottom = self.sky[0].top_over(bx0, bx1) - 0.6;
            let top = bottom - 0.72 * size - 0.8;
            let t = 0.16;
            for (a, b) in [((bx0, top), (bx1, top)), ((bx1, top), (bx1, bottom)), ((bx1, bottom), (bx0, bottom)), ((bx0, bottom), (bx0, top))] {
                self.place(0, Item::line(a.0, a.1, b.0, b.1, t, 0));
            }
            self.place(0, Item::Text { text: letter, at: pt(x, bottom - 0.4), size, italic: false, bold: true, anchor: 1, id: 0 });
        }
        // Navigation: the segno and coda signs over their barline; Fine,
        // "To Coda" and the jumps in italics, ending at their barline.
        for n in self.score.navigation.clone() {
            let here = if n.kind.at_bar_start() { n.at >= r0 && n.at < r1 } else { n.at > r0 && n.at <= r1 };
            if !here {
                continue;
            }
            let x = self.bar_x(n.at);
            match n.kind {
                Nav::Segno | Nav::Coda => {
                    let g = if n.kind == Nav::Segno { Glyph::Segno } else { Glyph::Coda };
                    let b = self.font.bbox(g);
                    let x0 = x - b.width() / 2.0;
                    let y = self.sky[0].top_over(x0, x0 + b.width()) - 0.6 + b.sw.1;
                    self.place(0, Item::glyph(g, x0, y, n.id));
                }
                k => {
                    let text = k.text();
                    let w = text_width(text, TEXT_SIZE, false);
                    // The sign after "To Coda" is the words' size.
                    const SIGN: f32 = 0.7;
                    let coda = if k == Nav::ToCoda { self.font.bbox(Glyph::Coda) } else { Default::default() };
                    let cw = if k == Nav::ToCoda { coda.width() * SIGN + 0.3 } else { 0.0 };
                    let x1 = x - 0.2;
                    let x0 = x1 - w - cw;
                    let y = self.sky[0].top_over(x0, x1) - 0.6;
                    self.place(0, Item::Text { text: text.into(), at: pt(x0, y), size: TEXT_SIZE, italic: true, bold: false, anchor: 0, id: n.id });
                    if k == Nav::ToCoda {
                        // Centred on the words' middle.
                        let yg = y - 0.6 + (coda.ne.1 + coda.sw.1) / 2.0 * SIGN;
                        self.place(0, Item::glyph_scaled(Glyph::Coda, x1 - coda.width() * SIGN, yg, SIGN, n.id));
                    }
                }
            }
        }
        // Tempo marks: words (bold) and a metronome mark; repeated only when they change.
        let mut tempo = self.score.tempo.clone();
        tempo.sort_by(|a, b| a.at.cmp(&b.at));
        let mut printed = -1i64;
        for t in tempo {
            if t.at < r0 || t.at >= r1 {
                if t.at < r0 {
                    printed = t.bpm.round() as i64;
                }
                continue;
            }
            let bpm = t.bpm.round() as i64;
            if t.text.is_empty() && bpm == printed {
                continue;
            }
            printed = bpm;
            let mut x = if t.at == Frac::ZERO { self.left + 1.0 } else { self.col_x(t.at) - 0.5 };
            // The words bold; the metronome mark, in parentheses after
            // words, in the regular weight, both parentheses alike.
            let words = t.text.clone();
            let open = if words.is_empty() { String::new() } else { " (".to_string() };
            let tail = format!(" = {bpm}{}", if words.is_empty() { "" } else { ")" });
            let total = text_width(&words, TEMPO_SIZE, true) + text_width(&open, TEMPO_SIZE, false) + 1.0 + text_width(&tail, TEMPO_SIZE, false);
            let y = (self.sky[0].top_over(x, x + total) - 0.6).min(self.top(0) - 2.0);
            if !words.is_empty() {
                let item = Item::Text { text: words.clone(), at: pt(x, y), size: TEMPO_SIZE, italic: false, bold: true, anchor: 0, id: t.id };
                x += text_width(&words, TEMPO_SIZE, true);
                self.place(0, item);
                let item = Item::Text { text: open.clone(), at: pt(x, y), size: TEMPO_SIZE, italic: false, bold: false, anchor: 0, id: t.id };
                x += text_width(&open, TEMPO_SIZE, false) + 0.35;
                self.place(0, item);
            }
            self.place(0, Item::glyph_scaled(Glyph::MetNoteQuarterUp, x, y - 0.1, 0.6, t.id));
            x += 0.9;
            self.place(0, Item::Text { text: tail, at: pt(x, y), size: TEMPO_SIZE, italic: false, bold: false, anchor: 0, id: t.id });
        }
    }
}
