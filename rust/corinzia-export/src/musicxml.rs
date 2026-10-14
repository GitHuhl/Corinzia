//! MusicXML 4.0 (partwise) from the score. Pitches are concert pitch (the
//! model stores sounding pitch). Events crossing barlines, and durations no
//! single note value can show, are split into tied notes.

use crate::doc::{art, span, BarKind, Document, Event, Part, Score};
use crate::frac::{lcm, Frac};
use corinzia_score::values::{candidates, decompose, NoteValue};
use std::fmt::Write;

// ------------------------------------------------------------------ helpers --
fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            _ => o.push(c),
        }
    }
    o
}

fn clef_xml(clef: &str) -> String {
    let (sign, line, oct) = match clef {
        "bass" => ("F", 4, 0),
        "bass_8" => ("F", 4, -1),
        "alto" => ("C", 3, 0),
        "tenor" => ("C", 4, 0),
        "treble_8" => ("G", 2, -1),
        "percussion" => ("percussion", 0, 0),
        _ => ("G", 2, 0),
    };
    let mut s = format!("<clef><sign>{sign}</sign>");
    if sign != "percussion" {
        let _ = write!(s, "<line>{line}</line>");
    }
    if oct != 0 {
        let _ = write!(s, "<clef-octave-change>{oct}</clef-octave-change>");
    }
    s + "</clef>"
}

fn rehearsal_letter(i: usize) -> String {
    let (letter, round) = ((b'A' + (i % 26) as u8) as char, i / 26);
    std::iter::repeat_n(letter, round + 1).collect()
}

// ----------------------------------------------------------------- measures --
#[derive(Clone, Copy, Debug)]
struct Measure {
    start: Frac,
    len: Frac,
    num: i64,
    den: i64,
    implicit: bool,
}

fn measures(s: &Score) -> Vec<Measure> {
    let mut meters = s.meter.clone();
    meters.sort_by(|a, b| a.at.cmp(&b.at));
    if meters.is_empty() || meters[0].at > Frac::ZERO {
        meters.insert(0, crate::doc::Meter { at: Frac::ZERO, num: 4, den: 4, pickup: false });
    }
    let mut out = Vec::new();
    let mut at = Frac::ZERO;
    while at < s.length && out.len() < 100_000 {
        let m = meters.iter().rev().find(|m| m.at <= at).copied().unwrap_or(meters[0]);
        let mut len = Frac::new(m.num.max(1), m.den.max(1));
        // A meter change inside this bar, or the movement end, cuts it short.
        if let Some(next) = meters.iter().find(|x| x.at > at && x.at < at + len) {
            len = next.at - at;
        }
        if at + len > s.length {
            len = s.length - at;
        }
        // A pickup is written in the following meter, as an implicit bar.
        let (num, den) = match meters.iter().find(|x| m.pickup && x.at > m.at && !x.pickup) {
            Some(next) => (next.num, next.den),
            None => (m.num, m.den),
        };
        out.push(Measure { start: at, len, num, den, implicit: m.pickup });
        at = at + len;
    }
    out
}

/// A note's tuplet context: the time modification, the `<tuplet>` marks
/// it carries and its sounding duration.
struct Tm {
    actual: i64,
    normal: i64,
    marks: String,
    real: Frac,
}

// -------------------------------------------------------------------- writer --
struct Writer<'a> {
    score: &'a Score,
    divisions: i64, // per quarter note
    out: String,
    /// Inside a hyphenated word, per (voice, verse): for <syllabic>.
    in_word: std::collections::HashMap<(u8, u8), bool>,
}

impl<'a> Writer<'a> {
    fn dur(&self, f: Frac) -> i64 {
        // whole-note fraction -> divisions
        let q = f * Frac::int(4 * self.divisions);
        q.num / q.den
    }

    fn line(&mut self, s: &str) {
        self.out.push_str(s);
        self.out.push('\n');
    }

    fn part(&mut self, pi: usize, part: &Part, ms: &[Measure]) {
        let s = self.score;
        let _ = writeln!(self.out, "  <part id=\"P{}\">", pi + 1);
        let mut keys = s.keys.clone();
        keys.sort_by(|a, b| a.at.cmp(&b.at));
        let mut tempo = s.tempo.clone();
        tempo.sort_by(|a, b| a.at.cmp(&b.at));
        let mut marks = s.marks.clone();
        marks.sort();
        let mut slur_open = [false; 4];
        let mut wedge_open = false;
        let mut prev_meter = (0, 0);
        for (mi, m) in ms.iter().enumerate() {
            let end = m.start + m.len;
            let pickup = ms.first().is_some_and(|m| m.implicit) as usize;
            if m.implicit {
                let _ = writeln!(self.out, "    <measure number=\"{}\" implicit=\"yes\">", mi + 1 - pickup);
            } else {
                let _ = writeln!(self.out, "    <measure number=\"{}\">", mi + 1 - pickup);
            }
            // Left barline: start repeat and/or ending start.
            let start_repeat = s
                .barlines
                .iter()
                .any(|b| b.at == m.start && matches!(b.kind, BarKind::RepeatStart | BarKind::RepeatBoth) && mi > 0)
                || (mi == 0 && s.barlines.iter().any(|b| b.at == Frac::ZERO && b.kind == BarKind::RepeatStart));
            let ending_start = s.endings.iter().find(|e| e.from == m.start);
            if start_repeat || ending_start.is_some() {
                self.line("      <barline location=\"left\">");
                if start_repeat {
                    self.line("        <bar-style>heavy-light</bar-style>");
                }
                if let Some(e) = ending_start {
                    let nums: Vec<String> = e.numbers.iter().map(|n| n.to_string()).collect();
                    let label: Vec<String> = e.numbers.iter().map(|n| format!("{n}.")).collect();
                    let _ = writeln!(
                        self.out,
                        "        <ending number=\"{}\" type=\"start\">{}</ending>",
                        nums.join(","),
                        esc(&label.join(" "))
                    );
                }
                if start_repeat {
                    self.line("        <repeat direction=\"forward\"/>");
                }
                self.line("      </barline>");
            }
            // Attributes.
            let key_here = keys.iter().rev().find(|k| k.at <= m.start && (mi == 0 || k.at > ms[mi - 1].start));
            let meter_changed = (m.num, m.den) != prev_meter && (m.len == Frac::new(m.num, m.den) || m.implicit) || mi == 0;
            if mi == 0 || key_here.is_some() || meter_changed {
                self.line("      <attributes>");
                if mi == 0 {
                    let _ = writeln!(self.out, "        <divisions>{}</divisions>", self.divisions);
                }
                if let Some(k) = key_here.or(if mi == 0 { keys.first() } else { None }) {
                    let _ = writeln!(
                        self.out,
                        "        <key><fifths>{}</fifths><mode>{}</mode></key>",
                        k.fifths,
                        if k.minor { "minor" } else { "major" }
                    );
                }
                if meter_changed {
                    let _ = writeln!(self.out, "        <time><beats>{}</beats><beat-type>{}</beat-type></time>", m.num, m.den);
                    prev_meter = (m.num, m.den);
                }
                if mi == 0 {
                    let _ = writeln!(self.out, "        {}", clef_xml(&part.clef));
                }
                self.line("      </attributes>");
            }

            // Directions anchored in this bar (emitted before the first voice-1
            // note at or after their position).
            let mut directions: Vec<(Frac, String)> = Vec::new();
            if pi == 0 {
                for t in tempo.iter().filter(|t| t.at >= m.start && t.at < end) {
                    let mut d = String::from("      <direction placement=\"above\">\n");
                    if !t.text.is_empty() {
                        let _ = writeln!(
                            d,
                            "        <direction-type><words font-weight=\"bold\">{}</words></direction-type>",
                            esc(&t.text)
                        );
                    }
                    let _ = writeln!(
                        d,
                        "        <direction-type><metronome><beat-unit>quarter</beat-unit><per-minute>{}</per-minute></metronome></direction-type>",
                        t.bpm.round()
                    );
                    let _ = writeln!(d, "        <sound tempo=\"{}\"/>", t.bpm);
                    d.push_str("      </direction>");
                    directions.push((t.at, d));
                }
                for (i, at) in marks.iter().enumerate() {
                    if *at >= m.start && *at < end {
                        directions.push((
                            *at,
                            format!(
                                "      <direction placement=\"above\"><direction-type><rehearsal>{}</rehearsal></direction-type></direction>",
                                rehearsal_letter(i)
                            ),
                        ));
                    }
                }
            }
            for t in part.texts.iter().filter(|t| t.at >= m.start && t.at < end) {
                let (place, style) = if t.technique { ("above", "") } else { ("below", " font-style=\"italic\"") };
                directions.push((
                    t.at,
                    format!(
                        "      <direction placement=\"{place}\"><direction-type><words{style}>{}</words></direction-type></direction>",
                        esc(&t.text)
                    ),
                ));
            }
            for c in part.chords.iter().filter(|c| c.at >= m.start && c.at < end) {
                if let Some(h) = harmony_xml(&c.text) {
                    directions.push((c.at, h));
                }
            }
            for c in part.clefs.iter().filter(|c| c.at >= m.start && c.at < end && c.at > Frac::ZERO) {
                directions.push((c.at, format!("      <attributes>{}</attributes>", clef_xml(&c.clef))));
            }
            directions.sort_by(|a, b| a.0.cmp(&b.0));

            // Voices.
            let mut voices: Vec<&crate::doc::Voice> = part.voices.iter().collect();
            voices.sort_by_key(|v| v.index);
            let has_v1 = voices.iter().any(|v| v.index == 1);
            let mut written_v1 = false;
            let mut cursor = Frac::ZERO; // position inside the measure after the last write
            for v in voices.iter() {
                let pieces = self.pieces(&v.events, m.start, end);
                if v.index != 1 && pieces.is_empty() {
                    continue;
                }
                if cursor > Frac::ZERO {
                    let _ = writeln!(self.out, "      <backup><duration>{}</duration></backup>", self.dur(cursor));
                }
                let vi = (v.index.clamp(1, 4) - 1) as usize;
                if v.index == 1 && pieces.is_empty() {
                    self.emit_directions(&mut directions, end);
                    self.measure_rest(m, 1);
                    cursor = m.len;
                    written_v1 = true;
                    continue;
                }
                let mut at = m.start;
                let mut triplet_run = 0usize;
                for (pi_idx, p) in pieces.iter().enumerate() {
                    if p.from > at {
                        let gap = p.from - at;
                        if v.index == 1 {
                            self.emit_directions(&mut directions, p.from);
                            for nv in decompose(gap) {
                                self.rest(nv, 1);
                            }
                        } else {
                            let _ = writeln!(self.out, "      <forward><duration>{}</duration></forward>", self.dur(gap));
                        }
                    }
                    if v.index == 1 {
                        self.emit_directions(&mut directions, p.from);
                    }
                    // Explicit tuplets around the piece (outermost first).
                    let mut around: Vec<&crate::doc::Tuplet> = v.tuplets.iter().filter(|t| p.from >= t.start && p.from < t.start + t.length).collect();
                    around.sort_by(|a, b| a.start.cmp(&b.start).then(b.length.cmp(&a.length)));
                    if !around.is_empty() {
                        let (actual, normal) = around.iter().fold((1, 1), |(a, n), t| (a * t.actual, n * t.normal));
                        let values = decompose((p.to - p.from) * Frac::new(actual, normal));
                        for (k, nv) in values.iter().enumerate() {
                            let (first, last) = (k == 0, k + 1 == values.len());
                            let real = nv.length() * Frac::new(normal, actual);
                            let mut marks = String::new();
                            for (depth, t) in around.iter().enumerate() {
                                let show = format!(
                                    "<tuplet-actual><tuplet-number>{}</tuplet-number></tuplet-actual><tuplet-normal><tuplet-number>{}</tuplet-number></tuplet-normal>",
                                    t.actual, t.normal
                                );
                                if first && p.from == t.start && p.event_first {
                                    let _ = write!(marks, "<tuplet type=\"start\" number=\"{}\" bracket=\"yes\">{show}</tuplet>", depth + 1);
                                }
                                if last && p.to == t.start + t.length && p.event_last {
                                    let _ = write!(marks, "<tuplet type=\"stop\" number=\"{}\"/>", depth + 1);
                                }
                            }
                            let tm = Tm { actual, normal, marks, real };
                            self.note(p, *nv, v.index, first && p.event_first, last, &tm, &mut slur_open[vi], &mut wedge_open, first);
                        }
                        triplet_run = 0;
                        at = p.to;
                        continue;
                    }
                    let values = decompose(p.to - p.from);
                    let next_is_triplet = |idx: usize| pieces.get(idx).is_some_and(|q| decompose(q.to - q.from).first().is_some_and(|x| x.triplet));
                    for (k, nv) in values.iter().enumerate() {
                        let first = k == 0;
                        let last = k + 1 == values.len();
                        // Tuplet brackets around runs of triplet values.
                        let tuplet = if nv.triplet {
                            triplet_run += 1;
                            let more = !last && values[k + 1].triplet || last && next_is_triplet(pi_idx + 1);
                            if triplet_run == 1 && more {
                                Some("start")
                            } else if !more && triplet_run > 1 {
                                triplet_run = 0;
                                Some("stop")
                            } else {
                                if !more {
                                    triplet_run = 0;
                                }
                                None
                            }
                        } else {
                            triplet_run = 0;
                            None
                        };
                        let tm = Tm {
                            actual: if nv.triplet { 3 } else { 1 },
                            normal: if nv.triplet { 2 } else { 1 },
                            marks: tuplet.map(|t| format!("<tuplet type=\"{t}\"/>")).unwrap_or_default(),
                            real: nv.length(),
                        };
                        self.note(p, *nv, v.index, first && p.event_first, last, &tm, &mut slur_open[vi], &mut wedge_open, first);
                    }
                    at = p.to;
                }
                if at < end && v.index == 1 {
                    self.emit_directions(&mut directions, end);
                    for nv in decompose(end - at) {
                        self.rest(nv, 1);
                    }
                    at = end;
                }
                cursor = at - m.start;
                if v.index == 1 {
                    written_v1 = true;
                }
            }
            if !has_v1 && !written_v1 {
                self.emit_directions(&mut directions, end);
                self.measure_rest(m, 1);
            }
            self.emit_directions(&mut directions, end + Frac::int(1)); // anything left

            // Right barline.
            let bar = s.barlines.iter().find(|b| b.at == end);
            let ending_end = s.endings.iter().find(|e| e.to == end);
            let back_repeat = bar.is_some_and(|b| matches!(b.kind, BarKind::RepeatEnd | BarKind::RepeatBoth));
            let style = match bar.map(|b| b.kind) {
                Some(BarKind::Double) => Some("light-light"),
                Some(BarKind::Final) | Some(BarKind::RepeatEnd) => Some("light-heavy"),
                Some(BarKind::RepeatBoth) => Some("light-heavy"),
                _ if mi + 1 == ms.len() => Some("light-heavy"),
                _ => None,
            };
            if style.is_some() || ending_end.is_some() {
                self.line("      <barline location=\"right\">");
                if let Some(st) = style {
                    let _ = writeln!(self.out, "        <bar-style>{st}</bar-style>");
                }
                if let Some(e) = ending_end {
                    let nums: Vec<String> = e.numbers.iter().map(|n| n.to_string()).collect();
                    let _ = writeln!(
                        self.out,
                        "        <ending number=\"{}\" type=\"{}\"/>",
                        nums.join(","),
                        if back_repeat { "stop" } else { "discontinue" }
                    );
                }
                if back_repeat {
                    self.line("        <repeat direction=\"backward\"/>");
                }
                self.line("      </barline>");
            }
            self.line("    </measure>");
        }
        self.line("  </part>");
    }

    fn emit_directions(&mut self, dirs: &mut Vec<(Frac, String)>, upto: Frac) {
        while dirs.first().is_some_and(|d| d.0 <= upto) {
            let (_, d) = dirs.remove(0);
            self.line(&d);
        }
    }

    fn measure_rest(&mut self, m: &Measure, voice: u8) {
        let _ = writeln!(
            self.out,
            "      <note><rest measure=\"yes\"/><duration>{}</duration><voice>{voice}</voice></note>",
            self.dur(m.len)
        );
    }

    fn rest(&mut self, nv: NoteValue, voice: u8) {
        let mut s = format!("      <note><rest/><duration>{}</duration><voice>{voice}</voice><type>{}</type>", self.dur(nv.length()), nv.type_name());
        for _ in 0..nv.dots {
            s.push_str("<dot/>");
        }
        if nv.triplet {
            s.push_str("<time-modification><actual-notes>3</actual-notes><normal-notes>2</normal-notes></time-modification>");
        }
        s.push_str("</note>");
        self.line(&s);
    }

    /// Events of a voice clipped to [from, to), in order.
    fn pieces(&self, events: &'a [Event], from: Frac, to: Frac) -> Vec<Piece<'a>> {
        let mut out = Vec::new();
        for (i, e) in events.iter().enumerate() {
            let e_end = e.start + e.dur;
            if e_end <= from || e.start >= to {
                continue;
            }
            let prev_ties = i > 0 && {
                let p = &events[i - 1];
                p.start + p.dur == e.start && p.pitches.iter().any(|h| h.tie)
            };
            out.push(Piece {
                ev: e,
                from: e.start.max(from),
                to: e_end.min(to),
                event_first: e.start >= from,
                event_last: e_end <= to,
                tied_in: prev_ties,
            });
        }
        out
    }

    #[allow(clippy::too_many_arguments)]
    fn note(
        &mut self,
        p: &Piece,
        nv: NoteValue,
        voice: u8,
        event_start: bool,
        piece_last: bool,
        tm: &Tm,
        slur_open: &mut bool,
        wedge_open: &mut bool,
        piece_first: bool,
    ) {
        let e = p.ev;
        let is_rest = e.pitches.is_empty();
        // Dynamics and hairpins go before the event's first note.
        if event_start && voice == 1 || event_start && !is_rest {
            if (e.spans & span::HAIRPIN_END != 0 || e.dynamic.is_some()) && *wedge_open {
                self.line("      <direction><direction-type><wedge type=\"stop\"/></direction-type></direction>");
                *wedge_open = false;
            }
            if let Some(d) = &e.dynamic {
                let _ = writeln!(
                    self.out,
                    "      <direction placement=\"below\"><direction-type><dynamics><{}/></dynamics></direction-type></direction>",
                    esc(d)
                );
            }
            if e.spans & (span::CRESC_START | span::DIM_START) != 0 {
                let kind = if e.spans & span::CRESC_START != 0 { "crescendo" } else { "diminuendo" };
                let _ = writeln!(self.out, "      <direction placement=\"below\"><direction-type><wedge type=\"{kind}\"/></direction-type></direction>");
                *wedge_open = true;
            }
        }
        let continues_after = !piece_last || !p.event_last; // more of this event follows (tie)
        let continued_before = !piece_first || !p.event_first;
        let heads: Vec<Option<&crate::doc::Pitch>> = if is_rest { vec![None] } else { e.pitches.iter().map(Some).collect() };
        for (hi, h) in heads.iter().enumerate() {
            let mut s = String::from("      <note>");
            if hi > 0 {
                s.push_str("<chord/>");
            }
            match h {
                None => s.push_str("<rest/>"),
                Some(h) => {
                    let step = b"CDEFGAB"[h.step.min(6) as usize] as char;
                    let _ = write!(s, "<pitch><step>{step}</step>");
                    if h.alter != 0 {
                        let _ = write!(s, "<alter>{}</alter>", h.alter);
                    }
                    let _ = write!(s, "<octave>{}</octave></pitch>", h.octave);
                }
            }
            let _ = write!(s, "<duration>{}</duration>", self.dur(tm.real));
            let tie_stop = h.is_some() && (continued_before || p.tied_in && piece_first && p.event_first);
            let tie_start = h.is_some_and(|h| continues_after || h.tie && piece_last && p.event_last);
            if tie_stop {
                s.push_str("<tie type=\"stop\"/>");
            }
            if tie_start {
                s.push_str("<tie type=\"start\"/>");
            }
            let _ = write!(s, "<voice>{voice}</voice><type>{}</type>", nv.type_name());
            for _ in 0..nv.dots {
                s.push_str("<dot/>");
            }
            if tm.actual != tm.normal {
                let _ = write!(s, "<time-modification><actual-notes>{}</actual-notes><normal-notes>{}</normal-notes></time-modification>", tm.actual, tm.normal);
            }
            // Notations.
            let mut n = String::new();
            if tie_stop {
                n.push_str("<tied type=\"stop\"/>");
            }
            if tie_start {
                n.push_str("<tied type=\"start\"/>");
            }
            if hi == 0 {
                n.push_str(&tm.marks);
            }
            if event_start && hi == 0 && !is_rest {
                if e.spans & span::SLUR_END != 0 && *slur_open {
                    n.push_str("<slur type=\"stop\" number=\"1\"/>");
                    *slur_open = false;
                }
                if e.spans & span::SLUR_START != 0 {
                    n.push_str("<slur type=\"start\" number=\"1\"/>");
                    *slur_open = true;
                }
                let a = e.art;
                let mut arts = String::new();
                for (bit, tag) in [
                    (art::ACCENT, "accent"),
                    (art::STACCATO, "staccato"),
                    (art::TENUTO, "tenuto"),
                    (art::MARCATO, "strong-accent"),
                    (art::STACCATISSIMO, "staccatissimo"),
                    (art::BREATH, "breath-mark"),
                    (art::CAESURA, "caesura"),
                ] {
                    if a & bit != 0 {
                        let _ = write!(arts, "<{tag}/>");
                    }
                }
                if !arts.is_empty() {
                    let _ = write!(n, "<articulations>{arts}</articulations>");
                }
                let mut orn = String::new();
                for (bit, tag) in [(art::TRILL, "trill-mark"), (art::MORDENT, "mordent"), (art::TURN, "turn")] {
                    if a & bit != 0 {
                        let _ = write!(orn, "<{tag}/>");
                    }
                }
                if a & art::TREMOLO != 0 {
                    orn.push_str("<tremolo type=\"single\">3</tremolo>");
                }
                if !orn.is_empty() {
                    let _ = write!(n, "<ornaments>{orn}</ornaments>");
                }
                if a & art::FERMATA != 0 {
                    n.push_str("<fermata type=\"upright\"/>");
                }
            }
            if event_start && !is_rest && e.art & art::ARPEGGIO != 0 && e.pitches.len() > 1 {
                n.push_str("<arpeggiate/>");
            }
            if !n.is_empty() {
                let _ = write!(s, "<notations>{n}</notations>");
            }
            if event_start && hi == 0 && !is_rest {
                for l in &e.lyrics {
                    let was = self.in_word.insert((voice, l.verse), l.hyphen).unwrap_or(false);
                    let syllabic = match (was, l.hyphen) {
                        (false, false) => "single",
                        (false, true) => "begin",
                        (true, true) => "middle",
                        (true, false) => "end",
                    };
                    let _ = write!(s, "<lyric number=\"{}\"><syllabic>{syllabic}</syllabic><text>{}</text>{}</lyric>", l.verse, esc(&l.text), if l.extend { "<extend/>" } else { "" });
                }
            }
            s.push_str("</note>");
            self.line(&s);
        }
    }
}

struct Piece<'a> {
    ev: &'a Event,
    from: Frac,
    to: Frac,
    event_first: bool, // the piece starts the event
    event_last: bool,  // the piece ends the event
    tied_in: bool,     // the previous event ties into this one
}

/// A chord symbol ("F♯m7♭5/E") as a MusicXML <harmony>: root, kind (with
/// the symbol's own spelling as text) and bass.
fn harmony_xml(text: &str) -> Option<String> {
    fn note(s: &str) -> Option<(char, i32, &str)> {
        let mut cs = s.chars();
        let step = cs.next()?;
        if !('A'..='G').contains(&step) {
            return None;
        }
        let rest = cs.as_str();
        Some(match rest.chars().next() {
            Some('♯') | Some('#') => (step, 1, &rest[rest.chars().next().unwrap().len_utf8()..]),
            Some('♭') => (step, -1, &rest['♭'.len_utf8()..]),
            Some('b') if !rest.starts_with("b5") => (step, -1, &rest[1..]),
            _ => (step, 0, rest),
        })
    }
    let (main, bass) = match text.rsplit_once('/') {
        Some((m, b)) if b.chars().next().is_some_and(|c| ('A'..='G').contains(&c)) => (m, Some(b)),
        _ => (text, None),
    };
    let (step, alter, quality) = note(main)?;
    let q = quality.replace('♭', "b").replace('♯', "#");
    let kind = match q.as_str() {
        "" => "major",
        "m" | "min" | "-" => "minor",
        "7" => "dominant",
        "maj7" | "M7" | "Δ" | "Δ7" => "major-seventh",
        "m7" | "min7" | "-7" => "minor-seventh",
        "°" | "dim" => "diminished",
        "°7" | "dim7" => "diminished-seventh",
        "ø" | "ø7" | "m7b5" => "half-diminished",
        "+" | "aug" => "augmented",
        "sus" | "sus4" => "suspended-fourth",
        "sus2" => "suspended-second",
        "6" => "major-sixth",
        "m6" => "minor-sixth",
        "9" => "dominant-ninth",
        "maj9" => "major-ninth",
        "m9" => "minor-ninth",
        "11" => "dominant-11th",
        "13" => "dominant-13th",
        "mmaj7" | "m(maj7)" => "major-minor",
        _ => "other",
    };
    let alter_xml = |a: i32, tag: &str| if a != 0 { format!("<{tag}>{a}</{tag}>") } else { String::new() };
    let mut x = format!(
        "      <harmony><root><root-step>{step}</root-step>{}</root><kind text=\"{}\">{kind}</kind>",
        alter_xml(alter, "root-alter"),
        esc(quality)
    );
    if let Some(b) = bass {
        let (bs, ba, _) = note(b)?;
        let _ = write!(x, "<bass><bass-step>{bs}</bass-step>{}</bass>", alter_xml(ba, "bass-alter"));
    }
    x.push_str("</harmony>");
    Some(x)
}

pub fn write(doc: &Document) -> Result<String, String> {
    let s = &doc.score;
    if s.parts.is_empty() {
        return Err("score has no parts".into());
    }
    let ms = measures(s);
    // Divisions per quarter: fine enough for every position and length.
    let mut div = 1i64;
    let mut add = |f: Frac| {
        let q = f * Frac::int(4);
        div = lcm(div, q.den);
    };
    for m in &ms {
        add(m.start);
        add(m.len);
    }
    for p in &s.parts {
        for v in &p.voices {
            for e in &v.events {
                add(e.start);
                add(e.dur);
                for nv in decompose(e.dur) {
                    add(nv.length());
                }
            }
        }
    }
    for nv in candidates() {
        if nv.length() >= Frac::new(1, 64) {
            add(nv.length());
        }
    }
    if div > 100_000 {
        return Err("positions too fine to export".into());
    }

    let mut w = Writer { score: s, divisions: div, out: String::new(), in_word: Default::default() };
    w.line("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"no\"?>");
    w.line("<!DOCTYPE score-partwise PUBLIC \"-//Recordare//DTD MusicXML 4.0 Partwise//EN\" \"http://www.musicxml.org/dtds/partwise.dtd\">");
    w.line("<score-partwise version=\"4.0\">");
    if !doc.title.is_empty() {
        let _ = writeln!(w.out, "  <work><work-title>{}</work-title></work>", esc(&doc.title));
    }
    if !s.title.is_empty() {
        let _ = writeln!(w.out, "  <movement-title>{}</movement-title>", esc(&s.title));
    }
    w.line("  <identification>");
    if !doc.composer.is_empty() {
        let _ = writeln!(w.out, "    <creator type=\"composer\">{}</creator>", esc(&doc.composer));
    }
    w.line("    <encoding><software>Corinzia</software></encoding>");
    w.line("  </identification>");
    w.line("  <part-list>");
    for (i, p) in s.parts.iter().enumerate() {
        let _ = writeln!(w.out, "    <score-part id=\"P{}\">", i + 1);
        let _ = writeln!(w.out, "      <part-name>{}</part-name>", esc(&p.name));
        if !p.abbr.is_empty() {
            let _ = writeln!(w.out, "      <part-abbreviation>{}</part-abbreviation>", esc(&p.abbr));
        }
        w.line("    </score-part>");
    }
    w.line("  </part-list>");
    for (i, p) in s.parts.iter().enumerate() {
        w.part(i, p, &ms);
    }
    w.line("</score-partwise>");
    Ok(w.out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rehearsal_letters() {
        assert_eq!(rehearsal_letter(0), "A");
        assert_eq!(rehearsal_letter(25), "Z");
        assert_eq!(rehearsal_letter(26), "AA");
    }

    #[test]
    fn chord_symbols_as_harmony() {
        let h = harmony_xml("F♯m7♭5/E").unwrap();
        assert!(h.contains("<root-step>F</root-step><root-alter>1</root-alter>"), "{h}");
        assert!(h.contains("<kind text=\"m7♭5\">half-diminished</kind>"), "{h}");
        assert!(h.contains("<bass><bass-step>E</bass-step></bass>"), "{h}");
        assert!(harmony_xml("B♭maj7").unwrap().contains("<root-alter>-1</root-alter></root><kind text=\"maj7\">major-seventh</kind>"));
        assert!(harmony_xml("C6/9").unwrap().contains(">other</kind>") && !harmony_xml("C6/9").unwrap().contains("<bass>"));
        assert!(harmony_xml("H7").is_none());
    }
}
