//! Systems and pages: line breaking, and stacking systems onto pages with
//! vertical spacing.
//!
//! Ported from GNU LilyPond (`ly/paper-defaults-init.ly`), Copyright (C)
//! 1997–2026 Han-Wen Nienhuys, Jan Nieuwenhuizen and the LilyPond authors,
//! under the GNU General Public License, version 3 or later (see
//! `THIRD-PARTY-NOTICES.md`).

use super::*;

/// LilyPond's `staff-staff-spacing` padding: the least room between one
/// staff's music and the next staff's, where they overlap horizontally.
pub(super) const STAFF_PADDING: f32 = 1.0;

/// Engraves, then, where the music of neighbouring staves would come closer
/// than `STAFF_PADDING` (lyrics, chord symbols, high or low notes), moves
/// the staves apart and engraves again. As in LilyPond, the skylines are
/// compared column by column, and staves never come closer than
/// `staff_distance` (`basic-distance`).
pub(super) fn engrave_spaced(score: &Score, opt: &Options, ms: Vec<Measure>, notes: Vec<Note>, first: bool, width: Option<f32>) -> DisplayList {
    let (list, sky) = Engraver::new(score, opt, ms.clone(), notes.clone(), first, width).engrave();
    let mut tops = vec![0.0f32; sky.len()];
    for p in 1..sky.len() {
        let need = sky[p - 1].clearance_above(&sky[p], STAFF_PADDING);
        tops[p] = tops[p - 1] + need.max(opt.staff_distance);
    }
    if tops.iter().enumerate().all(|(p, t)| (t - p as f32 * opt.staff_distance).abs() < 0.01) {
        return list;
    }
    let mut e = Engraver::new(score, opt, ms, notes, first, width);
    e.tops = tops;
    e.engrave().0
}

/// One system of bars [a, b) from prepared notes, optionally justified.
pub(super) fn system(score: &Score, opt: &Options, ms: &[Measure], notes: &[Note], a: usize, b: usize, width: Option<f32>) -> DisplayList {
    let mut sm: Vec<Measure> = ms[a..b].to_vec();
    for m in sm.iter_mut() {
        m.x0 = 0.0;
        m.x1 = 0.0;
    }
    if a > 0 {
        // Each system restates the key; the time only where it changes.
        let fifths = key_at(score, sm[0].start);
        sm[0].key = Some(sm[0].key.unwrap_or(Key { at: sm[0].start, fifths, minor: false }));
    }
    let sn: Vec<Note> = notes
        .iter()
        .filter(|n| n.measure >= a && n.measure < b)
        .cloned()
        .map(|mut n| {
            n.measure -= a;
            n
        })
        .collect();
    engrave_spaced(score, opt, sm, sn, a == 0, width)
}

/// Page geometry in staff spaces (A4 at an 18 pt staff: 132.3 × 187.1).
#[derive(Clone, Debug)]
pub struct PageSpec {
    pub width: f32,
    pub height: f32,
    pub margin_x: f32,
    pub margin_y: f32,
    /// The least space between systems' outermost marks (LilyPond's
    /// `system-system-spacing` padding; its other distances apply too).
    pub system_gap: f32,
    /// The number of the first page (a later movement goes on counting).
    pub first_page: usize,
    /// Bars on each system (0: the line breaker chooses).
    pub bars_per_system: usize,
    /// Page numbers (and a running header) on every page but the first.
    pub page_numbers: bool,
    /// Points per staff space when printed (0: as wide as A4).
    pub space_pt: f32,
}

impl Default for PageSpec {
    fn default() -> Self {
        PageSpec { width: 132.3, height: 187.1, margin_x: 8.0, margin_y: 8.0, system_gap: 1.0, first_page: 1, bars_per_system: 0, page_numbers: true, space_pt: 0.0 }
    }
}

/// System breaks by dynamic programming over bar boundaries, as LilyPond's
/// optimal breaker does: each line costs the square of its force (the room
/// it must gain or give up, over its stretchability). Lines may be
/// compressed down to `MAX_COMPRESSION`; a bar wider than the line gets a
/// system to itself. Like LilyPond (`ragged-last = ##f`), the last line is
/// justified too.
///
/// A system always ends before a bar in `forced`; with `per_system` > 0
/// systems hold that many bars instead (still ending at forced breaks).
pub(super) fn break_systems(natural: &[f32], stretch: &[f32], squeeze: &[f32], prefix: impl Fn(usize) -> f32, line: f32, forced: &[usize], per_system: usize) -> Vec<(usize, usize)> {
    let n = natural.len();
    let mut cuts: Vec<usize> = forced.iter().copied().filter(|&f| f > 0 && f < n).collect();
    cuts.push(0);
    cuts.push(n);
    cuts.sort();
    cuts.dedup();
    let mut out = Vec::new();
    for seg in cuts.windows(2) {
        let (lo, hi) = (seg[0], seg[1]);
        if per_system > 0 {
            let mut a = lo;
            while a < hi {
                let b = (a + per_system).min(hi);
                out.push((a, b));
                a = b;
            }
        } else {
            out.extend(break_range(natural, stretch, squeeze, &prefix, line, lo, hi));
        }
    }
    out
}

/// The optimal breaks of bars `lo..hi`.
pub(super) fn break_range(natural: &[f32], stretch: &[f32], squeeze: &[f32], prefix: &impl Fn(usize) -> f32, line: f32, lo: usize, hi: usize) -> Vec<(usize, usize)> {
    let n = hi;
    let mut best = vec![f32::INFINITY; n + 1];
    let mut from = vec![lo; n + 1];
    best[lo] = 0.0;
    for b in lo + 1..=n {
        let (mut sum, mut k, mut q) = (0.0, 0.0, 0.0);
        for a in (lo..b).rev() {
            sum += natural[a];
            k += stretch[a];
            q += squeeze[a];
            let width = prefix(a) + sum;
            let force = (line - width) / if width > line { q } else { k }.max(1e-3);
            if force < MAX_COMPRESSION && b - a > 1 {
                break;
            }
            let cost = force * force + LINE_PENALTY;
            if best[a] + cost < best[b] {
                best[b] = best[a] + cost;
                from[b] = a;
            }
        }
    }
    let mut out = Vec::new();
    let mut b = n;
    while b > lo {
        out.push((from[b], b));
        b = from[b];
    }
    out.reverse();
    out
}

/// The movement on pages: each page a display list in page coordinates.
pub fn layout_pages(score: &Score, opt: &Options, page: &PageSpec) -> Vec<DisplayList> {
    let (ms, notes) = prepared(score, opt);
    if ms.is_empty() {
        return vec![];
    }
    // System coordinates start at the engraver's own margin, which lands on
    // the page margin: staves may run to x = line (+ that margin).
    let line = page.width - 2.0 * page.margin_x + opt.margin;
    // Natural bar widths from one unjustified line.
    let natural_line = {
        let mut e = Engraver::new(score, opt, ms.clone(), notes.clone(), true, None);
        e.beam_groups();
        e.space();
        e
    };
    let natural: Vec<f32> = (0..ms.len())
        .map(|i| natural_line.ms[i].x1 - if i == 0 { natural_line.ms[0].x0 } else { natural_line.ms[i - 1].x1 })
        .collect();
    let first_prefix = natural_line.ms[0].x0;
    let later_prefix = {
        let e = Engraver::new(score, opt, vec![], vec![], false, None);
        let clef_w = score.parts.iter().map(|p| e.width(clef_glyph(&p.clef).0)).fold(0.0f32, f32::max);
        e.left + 0.8 + clef_w + 0.8 + 3.0
    };
    let stretch: Vec<f32> = natural_line.ms.iter().map(|m| m.stretch).collect();
    let squeeze: Vec<f32> = natural_line.ms.iter().map(|m| m.squeeze).collect();
    // Breaks by hand: before the bars starting there.
    let bar_of = |t: Frac| ms.iter().position(|m| m.start == t);
    let forced: Vec<usize> = score.breaks.iter().filter_map(|b| bar_of(b.0)).collect();
    let page_breaks: Vec<usize> = score.breaks.iter().filter(|b| b.1).filter_map(|b| bar_of(b.0)).collect();
    let systems = break_systems(&natural, &stretch, &squeeze, |a| if a == 0 { first_prefix } else { later_prefix }, line, &forced, page.bars_per_system);

    // Lay out each system, then stack them on pages by their real extents.
    let mut pages: Vec<DisplayList> = Vec::new();
    let new_page = || DisplayList { width: page.width, height: page.height, space_pt: page.space_pt, ..Default::default() };
    let mut cur = new_page();
    let mut y = page.margin_y;
    // The header, as LilyPond sets it: the title centred and bold, the
    // movement as a subtitle, the composer flush right below them.
    let (title, subtitle) = if score.work_title.is_empty() { (&score.title, None) } else { (&score.work_title, Some(&score.title).filter(|t| !t.is_empty())) };
    let text = |s: &str, x: f32, y: f32, size: f32, bold: bool, anchor: u8, id: u64| Item::Text { text: s.to_string(), at: pt(x, y), size, italic: false, bold, anchor, id };
    let title_id = if score.work_title.is_empty() { ids::MOVEMENT_TITLE } else { ids::PROJECT_TITLE };
    if !title.is_empty() {
        cur.items.push(text(title, page.width / 2.0, y + 3.0, 3.2, true, 1, title_id));
        y += 5.0;
        // The work's subtitle (first movement only), then the movement's.
        if !score.subtitle.is_empty() && page.first_page == 1 {
            cur.items.push(text(&score.subtitle, page.width / 2.0, y + 2.0, 2.4, false, 1, ids::SUBTITLE));
            y += 3.5;
        }
        if let Some(sub) = subtitle {
            cur.items.push(text(sub, page.width / 2.0, y + 2.0, 2.4, false, 1, ids::MOVEMENT_TITLE));
            y += 3.5;
        }
        // Flush left: a part's instrument, the lyricist; flush right: the
        // composer, the arranger.
        let left: Vec<(&str, u64)> = [(score.instrument.as_str(), 0), (score.lyricist.as_str(), ids::LYRICIST)].into_iter().filter(|(t, _)| !t.is_empty()).collect();
        let right: Vec<(&str, u64)> = [(score.composer.as_str(), ids::COMPOSER), (score.arranger.as_str(), ids::ARRANGER)].into_iter().filter(|(t, _)| !t.is_empty()).collect();
        for row in 0..left.len().max(right.len()) {
            if let Some((t, id)) = left.get(row) {
                cur.items.push(text(t, page.margin_x, y + 1.6, 2.0, false, 0, *id));
            }
            if let Some((t, id)) = right.get(row) {
                cur.items.push(text(t, page.width - page.margin_x, y + 1.6, 2.0, false, 2, *id));
            }
            y += 2.5;
        }
        y += 2.0;
    }
    // Engrave every system, then break pages and space the systems with
    // LilyPond's flexible vertical spacing (in staff spaces, between the
    // systems' nearest staves' middle lines):
    //   top-system-spacing    basic 6,  minimum 0, padding 1
    //   system-system-spacing basic 12, minimum 8, padding 1, stretch 60
    //   last-bottom-spacing   basic 1,  minimum 0, padding 1, stretch 30
    // and, below the title block, markup-system-spacing (padding 0.5,
    // stretch 30). `system_gap` is the padding between systems.
    let font = Font::bravura();
    struct Sys {
        list: DisplayList,
        boxes: Vec<[f32; 4]>,
        top_ref: f32, // the top staff's middle line, system y
        bot_ref: f32, // the bottom staff's middle line
        y0: f32,      // highest ink
        y1: f32,      // lowest ink
    }
    let built: Vec<(usize, Sys)> = systems
        .iter()
        .map(|&(a, b)| {
            let list = system(score, opt, &ms, &notes, a, b, Some(line + opt.margin));
            let mut boxes: Vec<[f32; 4]> = list.items.iter().map(|it| it.bounds(font)).filter(|r| r.iter().all(|v| v.is_finite())).collect();
            boxes.extend(list.staves.iter().map(|st| [st.left, st.top, st.right, st.top + if st.single { 2.0 } else { 4.0 }]));
            let y0 = boxes.iter().map(|r| r[1]).fold(0.0f32, f32::min);
            let y1 = boxes.iter().map(|r| r[3]).fold(list.height, f32::max);
            let top_ref = list.staves.first().map(|st| st.top + 2.0).unwrap_or(0.0);
            let bot_ref = list.staves.last().map(|st| st.top + 2.0).unwrap_or(top_ref);
            (a, Sys { list, boxes, top_ref, bot_ref, y0, y1 })
        })
        .collect();
    // How far apart two systems' reference points must be for their ink to
    // keep `padding` apart where it overlaps horizontally.
    let skyline = |u: &Sys, l: &Sys, padding: f32| -> f32 {
        let mut need = f32::MIN;
        for p in &u.boxes {
            for q in &l.boxes {
                if p[0] < q[2] && q[0] < p[2] {
                    need = need.max((p[3] - u.bot_ref) - (q[1] - l.top_ref) + padding);
                }
            }
        }
        need
    };
    // A spring: (minimum, ideal, stretchability).
    let between = |u: &Sys, l: &Sys| {
        let min = skyline(u, l, page.system_gap).max(8.0);
        (min, min.max(12.0), 60.0)
    };
    let header_bottom = y; // below the title block, first page only
    let top_spring = |sys: &Sys, first: bool| {
        if first && header_bottom > page.margin_y {
            let min = header_bottom + 0.5 - page.margin_y + (sys.top_ref - sys.y0);
            (min, min, 30.0)
        } else {
            let min = (sys.top_ref - sys.y0 + 1.0).max(0.0);
            (min, min.max(6.0), 0.0)
        }
    };
    let bottom_spring = |sys: &Sys| {
        let min = (sys.y1 - sys.bot_ref + 1.0).max(0.0);
        (min, min.max(1.0), 30.0)
    };
    let avail = page.height - 2.0 * page.margin_y;
    // The springs and fixed heights of systems lo..hi on one page.
    let springs_of = |lo: usize, hi: usize, first: bool| -> (Vec<(f32, f32, f32)>, f32) {
        let mut sp = vec![top_spring(&built[lo].1, first)];
        for k in lo + 1..hi {
            sp.push(between(&built[k - 1].1, &built[k].1));
        }
        sp.push(bottom_spring(&built[hi - 1].1));
        let fixed = (lo..hi).map(|k| built[k].1.bot_ref - built[k].1.top_ref).sum::<f32>();
        (sp, fixed)
    };
    // The force that fills `room` (positive: stretched, negative: pressed
    // towards the minima; None: the minima don't fit).
    let force = |sp: &[(f32, f32, f32)], room: f32| -> Option<f32> {
        let min: f32 = sp.iter().map(|s| s.0).sum();
        if min > room + 1e-3 {
            return None;
        }
        let ideal: f32 = sp.iter().map(|s| s.1).sum();
        let stretch: f32 = sp.iter().map(|s| s.2).sum();
        Some(if ideal <= room { if stretch > 0.0 { (room - ideal) / stretch } else { 0.0 } } else { -(ideal - room) / (ideal - min).max(1e-3) })
    };
    let length = |s: (f32, f32, f32), f: f32| if f >= 0.0 { s.1 + f * s.2 } else { s.1 + f * (s.1 - s.0) };
    // Page breaks as LilyPond's optimal page breaker weighs them: each page
    // costs its force squared, stretched or squeezed (the last page only if
    // squeezed: it may stay short), and the cheapest set of pages wins.
    let starts_page = |k: usize| k > 0 && page_breaks.contains(&built[k].0);
    let n = built.len();
    let fits = |lo: usize, hi: usize| -> Option<f32> {
        if (lo + 1..hi).any(starts_page) {
            return None;
        }
        let (sp, fixed) = springs_of(lo, hi, lo == 0);
        force(&sp, avail - fixed)
    };
    // best[k] = (pages, cost) for systems 0..k, from[k] = where its last page starts.
    let mut best = vec![(usize::MAX, f32::INFINITY); n + 1];
    let mut from = vec![0usize; n + 1];
    best[0] = (0, 0.0);
    for hi in 1..=n {
        for lo in (0..hi).rev() {
            if best[lo].0 == usize::MAX {
                continue;
            }
            let f = match fits(lo, hi) {
                Some(f) => f,
                None if hi - lo == 1 => 0.0, // a system taller than the page
                None => break,
            };
            // The last page may stay short, but not be squeezed for free.
            let cost = if hi == n { f.min(0.0).powi(2) } else { f * f };
            let cand = (best[lo].0 + 1, best[lo].1 + cost);
            if cand.1 < best[hi].1 || cand.1 == best[hi].1 && cand.0 < best[hi].0 {
                best[hi] = cand;
                from[hi] = lo;
            }
            if starts_page(lo) {
                break;
            }
        }
    }
    let mut ranges = Vec::new();
    let mut hi = n;
    while hi > 0 {
        ranges.push((from[hi], hi));
        hi = from[hi];
    }
    ranges.reverse();
    for (pi, &(lo, hi)) in ranges.iter().enumerate() {
        if pi > 0 {
            pages.push(std::mem::replace(&mut cur, new_page()));
        }
        let (sp, fixed) = springs_of(lo, hi, lo == 0);
        let last = pi + 1 == ranges.len();
        // The last page stays ragged at the bottom (ragged-last-bottom),
        // pressed only if it must be.
        let f = force(&sp, avail - fixed).unwrap_or(-1.0);
        let f = if last { f.min(0.0) } else { f };
        let mut y = page.margin_y;
        for (k, idx) in (lo..hi).enumerate() {
            y += length(sp[k], f);
            let sys = &built[idx].1;
            let dy = y - sys.top_ref;
            let dx = page.margin_x - opt.margin;
            let sys_index = cur.staves.last().map(|s| s.system + 1).unwrap_or(0);
            cur.items.extend(sys.list.items.iter().cloned().map(|i| i.translated(dx, dy)));
            cur.staves.extend(sys.list.staves.iter().cloned().map(|mut st| {
                st.top += dy;
                st.left += dx;
                st.right += dx;
                st.system = sys_index;
                st
            }));
            cur.columns.extend(sys.list.columns.iter().cloned().map(|mut c| {
                c.x += dx;
                c.system = sys_index;
                c
            }));
            y = dy + sys.bot_ref;
        }
    }
    // (The last page stays ragged at the bottom, as in LilyPond.)
    pages.push(cur);
    // From page 2: the page number in the outer corner of the top margin,
    // and in a part its instrument centred (as LilyPond's headers).
    let running = score.instrument.clone();
    for (k, p) in pages.iter_mut().enumerate() {
        let n = k + page.first_page;
        if n < 2 || !page.page_numbers {
            continue;
        }
        let (x, anchor) = if n.is_multiple_of(2) { (page.margin_x, 0) } else { (page.width - page.margin_x, 2) };
        p.items.push(Item::Text { text: n.to_string(), at: pt(x, page.margin_y * 0.6), size: 2.0, italic: false, bold: false, anchor, id: 0 });
        if !running.is_empty() {
            p.items.push(Item::Text { text: running.clone(), at: pt(page.width / 2.0, page.margin_y * 0.6), size: 1.8, italic: true, bold: false, anchor: 1, id: 0 });
        }
    }
    // The copyright at the foot of the first page.
    if let (Some(p), false, 1) = (pages.first_mut(), score.copyright.is_empty(), page.first_page) {
        p.items.push(Item::Text { text: score.copyright.clone(), at: pt(page.width / 2.0, page.height - page.margin_y * 0.45), size: 1.6, italic: false, bold: false, anchor: 1, id: ids::COPYRIGHT });
    }
    pages
}
