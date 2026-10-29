//! The score canvas: engraved pages (display lists from corinzia-layout)
//! drawn as vector geometry, so every zoom level is sharp. Glyph outlines
//! come straight from Bravura; text is converted to paths as well. Geometry
//! is cached and rebuilt only when the engraving, zoom, pan or selection
//! changes.

use crate::editor::SlurHandle;
use corinzia_layout::display::{DisplayList, Item};
use corinzia_score::score::ids;
use corinzia_layout::font::{Font as MusicFont, FONT_DATA};
use iced::mouse;
use iced::widget::canvas::{self, path, Cache, Event, Frame, Geometry, Path, Stroke, Text};
use iced::{Color, Point, Rectangle, Renderer, Size, Theme, Vector};
use std::collections::HashMap;
use std::sync::OnceLock;

/// Pixels per staff space at zoom 1.
pub const BASE_PX: f32 = 8.0;
/// Gap between pages, in staff spaces.
const PAGE_GAP: f32 = 6.0;

const INK: Color = Color::from_rgb(0.08, 0.08, 0.1);
/// The selection on the white page: the accent, a shade deeper for contrast.
const SELECTED: Color = Color::from_rgb(0.85, 0.28, 0.09);
const PLAYHEAD: Color = Color::from_rgb(0.16, 0.62, 0.36);
const CARET: Color = Color::from_rgb(0.12, 0.47, 0.90);

/// Bravura glyph outlines as paths in staff spaces (y down), built once.
fn glyph_path(codepoint: u32) -> Option<Path> {
    static CACHE: OnceLock<std::sync::Mutex<HashMap<u32, Option<Path>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Some(p) = cache.lock().unwrap().get(&codepoint) {
        return p.clone();
    }
    struct B(path::Builder, f32);
    impl ttf_parser::OutlineBuilder for B {
        fn move_to(&mut self, x: f32, y: f32) {
            self.0.move_to(Point::new(x / self.1, -y / self.1));
        }
        fn line_to(&mut self, x: f32, y: f32) {
            self.0.line_to(Point::new(x / self.1, -y / self.1));
        }
        fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
            self.0.quadratic_curve_to(Point::new(x1 / self.1, -y1 / self.1), Point::new(x / self.1, -y / self.1));
        }
        fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
            let u = self.1;
            self.0.bezier_curve_to(Point::new(x1 / u, -y1 / u), Point::new(x2 / u, -y2 / u), Point::new(x / u, -y / u));
        }
        fn close(&mut self) {
            self.0.close();
        }
    }
    let face = ttf_parser::Face::parse(FONT_DATA, 0).ok()?;
    let gid = char::from_u32(codepoint).and_then(|c| face.glyph_index(c));
    let path = gid.and_then(|g| {
        let mut b = B(path::Builder::new(), MusicFont::bravura().units_per_space);
        face.outline_glyph(g, &mut b).map(|_| b.0.build())
    });
    cache.lock().unwrap().insert(codepoint, path.clone());
    path
}

fn is_hit_target(codepoint: u32) -> bool {
    use corinzia_layout::font::Glyph::*;
    [NoteheadBlack, NoteheadHalf, NoteheadWhole, RestWhole, RestHalf, RestQuarter, Rest8th, Rest16th, Rest32nd, Rest64th]
        .iter()
        .any(|g| g.codepoint() == codepoint)
}

/// Where each page sits, in staff spaces: pages side by side (Panorama has one).
pub fn page_origins(pages: &[DisplayList], vertical: bool) -> Vec<Vector> {
    let mut out = Vec::new();
    let mut at = 0.0;
    for p in pages {
        out.push(if vertical { Vector::new(0.0, at) } else { Vector::new(at, 0.0) });
        at += if vertical { p.height } else { p.width } + PAGE_GAP;
    }
    out
}

/// A point on a cubic Bézier.
fn bezier(p: [corinzia_layout::display::Point; 4], t: f32) -> Point {
    let u = 1.0 - t;
    let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
    Point::new(a * p[0].x + b * p[1].x + c * p[2].x + d * p[3].x, a * p[0].y + b * p[1].y + c * p[2].y + d * p[3].y)
}

/// A curve's middle line (between its two edges), as points.
fn midline(item: &Item) -> Option<Vec<Point>> {
    let Item::Crescent { from, c1, c2, to, d1, d2, .. } = item else { return None };
    let outer = [*from, *c1, *c2, *to];
    let inner = [*from, *d1, *d2, *to];
    Some((0..=24).map(|k| k as f32 / 24.0).map(|t| {
        let (a, b) = (bezier(outer, t), bezier(inner, t));
        Point::new((a.x + b.x) / 2.0, (a.y + b.y) / 2.0)
    }).collect())
}

/// The pieces of curve `id` (a slur, one per system), as (page, item).
fn curve_pieces<'a>(pages: &'a [DisplayList], id: u64) -> impl Iterator<Item = (usize, &'a Item)> + 'a {
    pages.iter().enumerate().flat_map(move |(k, p)| p.items.iter().filter(move |i| matches!(i, Item::Crescent { id: c, .. } if *c == id)).map(move |i| (k, i)))
}

/// Whether slur `id` is drawn above its notes.
pub fn slur_above(pages: &[DisplayList], id: u64) -> Option<bool> {
    let (_, item) = curve_pieces(pages, id).next()?;
    let Item::Crescent { from, c1, c2, to, .. } = item else { return None };
    Some((c1.y + c2.y) / 2.0 < (from.y + to.y) / 2.0)
}

/// The drag handles of slur `id` in desk coordinates: its two ends.
pub fn slur_handles(pages: &[DisplayList], origins: &[Vector], id: u64) -> Vec<(SlurHandle, Point)> {
    let pieces: Vec<_> = curve_pieces(pages, id).collect();
    let (Some(&(k0, first)), Some(&(k1, last))) = (pieces.first(), pieces.last()) else { return vec![] };
    let at = |k: usize, p: Point| Point::new(p.x + origins[k].x, p.y + origins[k].y);
    let (Some(a), Some(b)) = (midline(first), midline(last)) else { return vec![] };
    vec![(SlurHandle::Start, at(k0, a[0])), (SlurHandle::End, at(k1, b[b.len() - 1]))]
}

/// The element under a point given in staff spaces (desk coordinates).
pub fn hit_test(pages: &[DisplayList], origins: &[Vector], at: Point) -> Option<u64> {
    let font = MusicFont::bravura();
    let mut best: Option<(f32, u64)> = None;
    for (p, o) in pages.iter().zip(origins) {
        let local = Point::new(at.x - o.x, at.y - o.y);
        for item in &p.items {
            // Slurs (and ties, which select their note) along their curve;
            // notes and rests under them win.
            if let (Item::Crescent { id, .. }, Some(line)) = (item, midline(item)) {
                if *id != 0 && line.iter().any(|q| q.distance(local) < 0.45) && best.is_none_or(|b| b.0 > 3.0) {
                    best = Some((3.0, *id));
                }
                continue;
            }
            // Notes, rests and dynamics, and text that belongs to something
            // (titles, tempo marks, texts, chord symbols, lyrics).
            let id = match item {
                Item::Glyph { glyph, id, .. } if *id != 0 && (is_hit_target(*glyph) || ids::as_dynamic(*id).is_some()) => id,
                Item::Text { id, .. } if *id != 0 => id,
                _ => continue,
            };
            let r = item.bounds(font);
            let pad = 0.25;
            if local.x >= r[0] - pad && local.x <= r[2] + pad && local.y >= r[1] - pad && local.y <= r[3] + pad {
                let area = (r[2] - r[0]) * (r[3] - r[1]);
                if best.is_none_or(|b| area < b.0) {
                    best = Some((area, *id));
                }
            }
        }
    }
    best.map(|b| b.1)
}

/// A point on a staff: which part, where in time (quarters from the
/// movement start) and which line or space (half spaces below the top line).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StaffPoint {
    /// The page it is on (movements shown one after another: which one).
    pub page: usize,
    pub part: usize,
    pub quarter: f64,
    pub step_below_top: i32,
}

/// The staff point under a desk position: the nearest staff of the system
/// it falls in, with time interpolated between that system's columns.
pub fn locate(pages: &[DisplayList], origins: &[Vector], at: Point) -> Option<StaffPoint> {
    locate_staff(pages, origins, at).map(|(sp, _)| sp)
}

/// [`locate`], with the staff found.
fn locate_staff<'a>(pages: &'a [DisplayList], origins: &[Vector], at: Point) -> Option<(StaffPoint, &'a corinzia_layout::display::StaffGeom)> {
    for (page, (p, o)) in pages.iter().zip(origins).enumerate() {
        let local = Point::new(at.x - o.x, at.y - o.y);
        if local.x < 0.0 || local.y < 0.0 || local.x > p.width || local.y > p.height {
            continue;
        }
        // Nearest staff by vertical distance to its middle line, within reach.
        let st = p
            .staves
            .iter()
            .filter(|s| local.x >= s.left - 1.0 && local.x <= s.right + 1.0)
            .min_by(|a, b| (local.y - a.top - 2.0).abs().total_cmp(&(local.y - b.top - 2.0).abs()))?;
        if (local.y - st.top - 2.0).abs() > 9.0 {
            return None;
        }
        let cols: Vec<_> = p.columns.iter().filter(|c| c.system == st.system).collect();
        let quarter = match cols.iter().position(|c| c.x > local.x) {
            None => cols.last()?.quarter,
            Some(0) => cols[0].quarter,
            Some(i) => {
                let (a, b) = (cols[i - 1], cols[i]);
                a.quarter + (b.quarter - a.quarter) * ((local.x - a.x) / (b.x - a.x).max(1e-6)) as f64
            }
        };
        return Some((StaffPoint { page, part: st.part, quarter, step_below_top: ((local.y - st.top) * 2.0).round() as i32 }, st));
    }
    None
}

/// What a right-click can change on the page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextTarget {
    Clef,
    Key,
    Time,
    Barline,
}

/// The clef, key signature, time signature or barline under a desk point.
/// Their marks carry no id: they are told apart by glyph (accidentals of
/// notes carry the note's id, a key signature's do not) and barlines are
/// the vertical strokes as tall as a staff.
pub fn context_target(pages: &[DisplayList], origins: &[Vector], at: Point) -> Option<ContextTarget> {
    use corinzia_layout::font::Glyph::*;
    let font = MusicFont::bravura();
    let clefs = [GClef, FClef, CClef, GClef8vb, FClef8vb, PercussionClef, TabClef6, TabClef4].map(|g| g.codepoint());
    let keys = [Sharp, Flat, Natural].map(|g| g.codepoint());
    let times: Vec<u32> = (0..10).map(TimeSigDigit).chain([TimeSigCommon, TimeSigCut]).map(|g| g.codepoint()).collect();
    let mut best: Option<(f32, ContextTarget)> = None;
    for (p, o) in pages.iter().zip(origins) {
        let local = Point::new(at.x - o.x, at.y - o.y);
        for item in &p.items {
            let found = match item {
                Item::Glyph { glyph, id: 0, .. } => {
                    let kind = if clefs.contains(glyph) {
                        ContextTarget::Clef
                    } else if keys.contains(glyph) {
                        ContextTarget::Key
                    } else if times.contains(glyph) {
                        ContextTarget::Time
                    } else {
                        continue;
                    };
                    let r = item.bounds(font);
                    let pad = 0.3;
                    if local.x < r[0] - pad || local.x > r[2] + pad || local.y < r[1] - pad || local.y > r[3] + pad {
                        continue;
                    }
                    ((local.x - (r[0] + r[2]) / 2.0).abs(), kind)
                }
                Item::Line { from, to, id: 0, .. } if (from.x - to.x).abs() < 1e-3 && (from.y - to.y).abs() >= 3.9 => {
                    let (y0, y1) = (from.y.min(to.y), from.y.max(to.y));
                    let dx = (local.x - from.x).abs();
                    if dx > 0.6 || local.y < y0 - 0.5 || local.y > y1 + 0.5 {
                        continue;
                    }
                    (dx, ContextTarget::Barline)
                }
                _ => continue,
            };
            if best.is_none_or(|b| found.0 < b.0) {
                best = Some(found);
            }
        }
    }
    best.map(|b| b.1)
}

/// x of a written position in one system, interpolated between its columns.
fn column_x(page: &DisplayList, system: u32, quarter: f64) -> Option<f32> {
    let cols: Vec<_> = page.columns.iter().filter(|c| c.system == system).collect();
    let (first, last) = (cols.first()?, cols.last()?);
    if quarter < first.quarter - 1e-9 || quarter > last.quarter + 1e-9 {
        return None;
    }
    match cols.iter().position(|c| c.quarter >= quarter - 1e-9) {
        Some(0) | None => Some(cols[0].x.max(if quarter > first.quarter { last.x } else { first.x })),
        Some(i) => {
            let (a, b) = (cols[i - 1], cols[i]);
            if (b.quarter - quarter).abs() < 1e-9 {
                return Some(b.x);
            }
            Some(a.x + ((quarter - a.quarter) / (b.quarter - a.quarter).max(1e-9)) as f32 * (b.x - a.x))
        }
    }
}

/// Every note/rest glyph id inside a desk rectangle.
pub fn ids_in(pages: &[DisplayList], origins: &[Vector], a: Point, b: Point) -> Vec<u64> {
    let (x0, x1, y0, y1) = (a.x.min(b.x), a.x.max(b.x), a.y.min(b.y), a.y.max(b.y));
    let font = MusicFont::bravura();
    let mut out = Vec::new();
    for (p, o) in pages.iter().zip(origins) {
        for item in &p.items {
            let Item::Glyph { glyph, id, .. } = item else { continue };
            if *id == 0 || !is_hit_target(*glyph) {
                continue;
            }
            let r = item.bounds(font);
            let (cx, cy) = ((r[0] + r[2]) / 2.0 + o.x, (r[1] + r[3]) / 2.0 + o.y);
            if cx >= x0 && cx <= x1 && cy >= y0 && cy <= y1 && !out.contains(id) {
                out.push(*id);
            }
        }
    }
    out
}

#[derive(Debug, Clone)]
pub enum ViewEvent {
    /// A click (no drag): the element hit, where, and with Shift held.
    Click { hit: Option<u64>, at: Option<StaffPoint>, shift: bool },
    /// A rubber band: the notes and rests inside it.
    Marquee { ids: Vec<u64>, shift: bool },
    Pan(Vector),
    Zoom { factor: f32, around: Point },
    /// An end of the selected slur dragged to a point in the score.
    SlurDrag { handle: SlurHandle, at: StaffPoint },
    /// The drag is over: the next one is a new undo step.
    SlurDragEnd,
    /// A second click on the same spot: edit what is there.
    DoubleClick { hit: Option<u64> },
    /// A right-click (no drag) on a clef, key or time signature or barline;
    /// `screen` is the pointer in the canvas.
    Context { target: ContextTarget, at: Option<StaffPoint>, screen: Point },
}

/// The note that note input would write under the pointer, drawn as a
/// shadow: its notehead and dots, snapped to the duration's grid.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ghost {
    pub glyph: u32,
    pub dots: u8,
    /// The grid, in quarters (the undotted duration).
    pub grid: f64,
}

/// The input caret: a part and a time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Caret {
    pub part: usize,
    pub quarter: f64,
}

/// A highlighted passage: parts `from..=to`, quarters `[start, end)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Highlight {
    pub part_from: usize,
    pub part_to: usize,
    pub start: f64,
    pub end: f64,
}

pub struct ScoreView<'a> {
    pub pages: &'a [DisplayList],
    pub vertical: bool,
    pub zoom: f32,
    pub pan: Vector, // screen pixels
    /// Ids drawn in the selection colour (a note, or a passage's contents).
    pub selected: &'a std::collections::HashSet<u64>,
    pub caret: Option<Caret>,
    pub highlight: Option<Highlight>,
    /// The playback position (written quarters) while playing, and the
    /// pages of the movement playing.
    pub playhead: Option<(f64, std::ops::Range<usize>)>,
    /// The pages of the movement being edited (several movements may be
    /// shown): the caret and the passage are drawn there.
    pub active: std::ops::Range<usize>,
    /// The slur selected: drawn with drag handles.
    pub slur: Option<u64>,
    /// In note input: the note a click would write.
    pub ghost: Option<Ghost>,
    pub cache: &'a Cache,
}

/// How soon a second click makes a double-click.
const DOUBLE_CLICK: std::time::Duration = std::time::Duration::from_millis(450);

#[derive(Default)]
pub struct Interaction {
    press: Option<Point>,
    current: Option<Point>,
    marquee: bool,
    panning: Option<Point>,
    /// The end of the selected slur being dragged.
    slur_drag: Option<SlurHandle>,
    /// When and where the last click was (for double-clicks).
    last_click: Option<(std::time::Instant, Point)>,
    modifiers: iced::keyboard::Modifiers,
    /// Where a right button went down, until it drags (then it pans).
    right_press: Option<Point>,
    /// The pointer over the canvas (for the note-input shadow).
    hover: Option<Point>,
}

impl ScoreView<'_> {
    fn scale(&self) -> f32 {
        self.zoom * BASE_PX
    }
    /// The end of the selected slur whose handle is at `desk`.
    fn slur_target(&self, desk: Point) -> Option<SlurHandle> {
        let id = self.slur?;
        let origins = page_origins(self.pages, self.vertical);
        // Handles keep a usable size when zoomed out.
        let reach = (0.7f32).max(6.0 / self.scale());
        let handles = slur_handles(self.pages, &origins, id);
        handles.iter().filter(|(_, p)| p.distance(desk) < reach).min_by(|a, b| a.1.distance(desk).total_cmp(&b.1.distance(desk))).map(|(h, _)| *h)
    }
    fn to_desk(&self, p: Point) -> Point {
        Point::new((p.x - self.pan.x) / self.scale(), (p.y - self.pan.y) / self.scale())
    }
}

impl canvas::Program<ViewEvent> for ScoreView<'_> {
    type State = Interaction;

    fn update(&self, state: &mut Interaction, event: &Event, bounds: Rectangle, cursor: mouse::Cursor) -> Option<canvas::Action<ViewEvent>> {
        let pos = cursor.position_in(bounds);
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let p = pos?;
                // On the selected slur: a handle, or the curve (moves it all).
                if let Some(h) = self.slur_target(self.to_desk(p)) {
                    state.slur_drag = Some(h);
                    return Some(canvas::Action::capture());
                }
                state.press = Some(p);
                state.current = Some(p);
                state.marquee = false;
                Some(canvas::Action::capture())
            }
            Event::Mouse(mouse::Event::ButtonPressed(b @ (mouse::Button::Middle | mouse::Button::Right))) => {
                let p = pos?;
                state.panning = Some(p);
                state.right_press = (*b == mouse::Button::Right).then_some(p);
                Some(canvas::Action::capture())
            }
            Event::Mouse(mouse::Event::CursorLeft) => {
                state.hover.take().map(|_| canvas::Action::request_redraw())
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let hover = std::mem::replace(&mut state.hover, pos.filter(|_| self.ghost.is_some()));
                if let (Some(from), Some(p)) = (state.right_press, pos) {
                    if p.distance(from) > 4.0 {
                        state.right_press = None; // a drag: it pans
                    }
                }
                if let (Some(from), Some(p)) = (state.panning, pos) {
                    state.panning = Some(p);
                    return Some(canvas::Action::publish(ViewEvent::Pan(p - from)).and_capture());
                }
                if let (Some(handle), Some(p)) = (state.slur_drag, pos) {
                    // The end snaps to the note nearest the pointer.
                    let origins = page_origins(self.pages, self.vertical);
                    return Some(match locate(self.pages, &origins, self.to_desk(p)) {
                        Some(at) => canvas::Action::publish(ViewEvent::SlurDrag { handle, at }).and_capture(),
                        None => canvas::Action::capture(),
                    });
                }
                let (from, p) = (state.press?, pos?);
                let d = p - from;
                if state.marquee || d.x.abs() + d.y.abs() > 4.0 {
                    state.marquee = true;
                    state.current = Some(p);
                    return Some(canvas::Action::request_redraw().and_capture());
                }
                (hover != state.hover).then(canvas::Action::request_redraw)
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Right)) => {
                state.panning.take()?;
                let p = state.right_press.take();
                let origins = page_origins(self.pages, self.vertical);
                match p.and_then(|p| context_target(self.pages, &origins, self.to_desk(p)).map(|t| (t, p))) {
                    Some((target, screen)) => Some(canvas::Action::publish(ViewEvent::Context { target, at: locate(self.pages, &origins, self.to_desk(screen)), screen }).and_capture()),
                    None => Some(canvas::Action::capture()),
                }
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Middle)) => {
                state.panning.take().map(|_| canvas::Action::capture())
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                if state.slur_drag.take().is_some() {
                    return Some(canvas::Action::publish(ViewEvent::SlurDragEnd).and_capture());
                }
                let from = state.press.take()?;
                let shift = state.modifiers.shift();
                let origins = page_origins(self.pages, self.vertical);
                if std::mem::take(&mut state.marquee) {
                    let to = state.current.take().unwrap_or(from);
                    let ids = ids_in(self.pages, &origins, self.to_desk(from), self.to_desk(to));
                    return Some(canvas::Action::publish(ViewEvent::Marquee { ids, shift }).and_capture());
                }
                let desk = self.to_desk(from);
                let hit = hit_test(self.pages, &origins, desk);
                // A second click soon after, on the same spot: a double-click.
                let now = std::time::Instant::now();
                let double = state.last_click.is_some_and(|(t, p)| now.duration_since(t) < DOUBLE_CLICK && p.distance(from) < 5.0);
                state.last_click = if double { None } else { Some((now, from)) };
                if double {
                    return Some(canvas::Action::publish(ViewEvent::DoubleClick { hit }).and_capture());
                }
                let at = locate(self.pages, &origins, desk);
                Some(canvas::Action::publish(ViewEvent::Click { hit, at, shift }).and_capture())
            }
            Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                let p = pos?;
                let (dx, dy) = match delta {
                    mouse::ScrollDelta::Lines { x, y } => (x * 40.0, y * 40.0),
                    mouse::ScrollDelta::Pixels { x, y } => (*x, *y),
                };
                // Ctrl+wheel zooms around the cursor; the wheel alone scrolls
                // (down the pages, or through time in Panorama); Shift+wheel sideways.
                let ev = if state.modifiers.control() {
                    ViewEvent::Zoom { factor: (1.0f32 + dy / 400.0).clamp(0.5, 2.0), around: p }
                } else if self.vertical && !state.modifiers.shift() {
                    ViewEvent::Pan(Vector::new(dx, dy))
                } else {
                    ViewEvent::Pan(Vector::new(dy + dx, 0.0))
                };
                Some(canvas::Action::publish(ev).and_capture())
            }
            Event::Keyboard(iced::keyboard::Event::ModifiersChanged(m)) => {
                state.modifiers = *m;
                None
            }
            _ => None,
        }
    }

    fn draw(&self, state: &Interaction, renderer: &Renderer, _theme: &Theme, bounds: Rectangle, _cursor: mouse::Cursor) -> Vec<Geometry> {
        let px = self.scale();
        let origins = page_origins(self.pages, self.vertical);
        let geometry = self.cache.draw(renderer, bounds.size(), |frame| {
            frame.fill_rectangle(Point::ORIGIN, bounds.size(), crate::theme::palette().desk);
            frame.translate(self.pan);
            frame.scale(px);
            for (k, (page, origin)) in self.pages.iter().zip(&origins).enumerate() {
                draw_page(frame, page, *origin, self.selected, px);
                if let (Some(h), true) = (self.highlight, self.active.contains(&k)) {
                    draw_highlight(frame, page, *origin, h);
                }
            }
        });
        // The overlay (caret, rubber band) changes often: drawn fresh each frame.
        let mut overlay = Frame::new(renderer, bounds.size());
        overlay.with_save(|f| {
            f.translate(self.pan);
            f.scale(px);
            if let Some(c) = self.caret {
                for (page, origin) in self.pages.iter().zip(&origins).skip(self.active.start).take(self.active.len()) {
                    draw_caret(f, page, *origin, c, px);
                }
            }
            if let Some((q, pages)) = &self.playhead {
                for (page, origin) in self.pages.iter().zip(&origins).skip(pages.start).take(pages.len()) {
                    draw_playhead(f, page, *origin, *q, px);
                }
            }
            if let (Some(g), Some(h)) = (self.ghost, state.hover) {
                if state.press.is_none() && state.panning.is_none() {
                    draw_ghost(f, self.pages, &origins, &self.active, self.to_desk(h), g, px);
                }
            }
            if let Some(id) = self.slur {
                let r = (0.35f32).max(4.0 / px);
                for (_, at) in slur_handles(self.pages, &origins, id) {
                    let square = Path::rectangle(Point::new(at.x - r, at.y - r), Size::new(2.0 * r, 2.0 * r));
                    f.fill(&square, Color::WHITE);
                    f.stroke(&square, Stroke::default().with_width(1.5).with_color(SELECTED));
                }
            }
        });
        if let (true, Some(a), Some(b)) = (state.marquee, state.press, state.current) {
            let r = Rectangle::new(Point::new(a.x.min(b.x), a.y.min(b.y)), Size::new((a.x - b.x).abs(), (a.y - b.y).abs()));
            overlay.fill_rectangle(r.position(), r.size(), Color { a: 0.12, ..SELECTED });
            overlay.stroke(&Path::rectangle(r.position(), r.size()), Stroke::default().with_width(1.0).with_color(SELECTED));
        }
        vec![geometry, overlay.into_geometry()]
    }

    fn mouse_interaction(&self, state: &Interaction, bounds: Rectangle, cursor: mouse::Cursor) -> mouse::Interaction {
        if state.panning.is_some() {
            mouse::Interaction::Grabbing
        } else if state.slur_drag.is_some() {
            mouse::Interaction::Grabbing
        } else if cursor.position_in(bounds).and_then(|p| self.slur_target(self.to_desk(p))).is_some() {
            mouse::Interaction::Grab
        } else if state.marquee {
            mouse::Interaction::Crosshair
        } else {
            mouse::Interaction::default()
        }
    }
}

/// Where the note-input shadow stands for the pointer at `desk`: the
/// notehead's origin, its staff's top line, the step below it, and the
/// staff line width.
pub fn ghost_spot(pages: &[DisplayList], origins: &[Vector], active: &std::ops::Range<usize>, desk: Point, g: Ghost) -> Option<(Point, f32, i32, f32)> {
    let (sp, st) = locate_staff(pages, origins, desk)?;
    if !active.contains(&sp.page) || !(-12..=20).contains(&sp.step_below_top) {
        return None;
    }
    let (page, o) = (&pages[sp.page], origins[sp.page]);
    let k = (sp.quarter / g.grid + 0.35).floor().max(0.0);
    let x = column_x(page, st.system, k * g.grid)?;
    let top = o.y + st.top;
    Some((Point::new(o.x + x, top + sp.step_below_top as f32 * 0.5), top, sp.step_below_top, st.line_width))
}

/// The note-input shadow: where a click at `desk` would write the note
/// (the same grid snap as note input), with ledger lines and dots.
fn draw_ghost(f: &mut Frame, pages: &[DisplayList], origins: &[Vector], active: &std::ops::Range<usize>, desk: Point, g: Ghost, px: f32) {
    let Some((Point { x, y }, top, step, line_width)) = ghost_spot(pages, origins, active, desk, g) else { return };
    let color = Color { a: 0.45, ..CARET };
    let r = Item::Glyph { glyph: g.glyph, at: corinzia_layout::display::Point { x: 0.0, y: 0.0 }, scale: 1.0, id: 0 }.bounds(MusicFont::bravura());
    let w = r[2] - r[0];
    // Ledger lines above and below the staff.
    let ledger = |f: &mut Frame, s: i32| {
        let y = top + s as f32 * 0.5;
        f.stroke(&Path::line(Point::new(x - 0.4, y), Point::new(x + w + 0.4, y)), Stroke::default().with_width(line_width * 1.6 * px).with_color(color));
    };
    for s in (step..=-2).filter(|s| s % 2 == 0) {
        ledger(f, s);
    }
    for s in (10..=step).filter(|s| s % 2 == 0) {
        ledger(f, s);
    }
    if let Some(p) = glyph_path(g.glyph) {
        f.with_save(|h| {
            h.translate(Vector::new(x, y));
            h.fill(&p, color);
        });
    }
    // Dots sit in a space.
    let dot_y = if step % 2 == 0 { y - 0.5 } else { y };
    for d in 0..g.dots {
        f.fill(&Path::circle(Point::new(x + w + 0.45 + d as f32 * 0.55, dot_y), 0.2), color);
    }
}

/// A line through the whole system at the playback position.
fn draw_playhead(frame: &mut Frame, page: &DisplayList, origin: Vector, quarter: f64, px: f32) {
    let systems: std::collections::BTreeSet<u32> = page.staves.iter().map(|s| s.system).collect();
    for sys in systems {
        let cols: Vec<_> = page.columns.iter().filter(|c| c.system == sys).collect();
        let (Some(first), Some(last)) = (cols.first(), cols.last()) else { continue };
        if quarter < first.quarter || quarter >= last.quarter {
            continue;
        }
        let Some(x) = column_x(page, sys, quarter) else { continue };
        let staves: Vec<_> = page.staves.iter().filter(|s| s.system == sys).collect();
        let (top, bottom) = (staves.first().map(|s| s.top).unwrap_or(0.0), staves.last().map(|s| s.top).unwrap_or(0.0));
        let line = Path::line(Point::new(origin.x + x - 0.6, origin.y + top - 2.0), Point::new(origin.x + x - 0.6, origin.y + bottom + 6.0));
        frame.stroke(&line, Stroke::default().with_width(0.3 * px).with_color(PLAYHEAD));
        return;
    }
}

/// Translucent band over the staves of a passage, per system.
fn draw_highlight(frame: &mut Frame, page: &DisplayList, origin: Vector, h: Highlight) {
    let systems: std::collections::BTreeSet<u32> = page.staves.iter().map(|s| s.system).collect();
    for sys in systems {
        let staves: Vec<_> = page.staves.iter().filter(|s| s.system == sys && s.part >= h.part_from && s.part <= h.part_to).collect();
        let (Some(top), Some(bottom)) = (staves.first(), staves.last()) else { continue };
        let cols: Vec<_> = page.columns.iter().filter(|c| c.system == sys).collect();
        let (Some(c0), Some(c1)) = (cols.first(), cols.last()) else { continue };
        if h.end <= c0.quarter || h.start >= c1.quarter {
            continue;
        }
        let x0 = column_x(page, sys, h.start.max(c0.quarter)).unwrap_or(c0.x) - 0.8;
        let x1 = if h.end >= c1.quarter { top.right } else { column_x(page, sys, h.end).unwrap_or(c1.x) - 0.8 };
        let (y0, y1) = (top.top - 1.5, bottom.top + 5.5);
        frame.fill_rectangle(Point::new(origin.x + x0, origin.y + y0), Size::new(x1 - x0, y1 - y0), Color { a: 0.13, ..SELECTED });
    }
}

/// Where the caret stands on a page: x and the top line of its staff.
fn caret_spot(page: &DisplayList, c: Caret) -> Option<(f32, f32)> {
    for st in page.staves.iter().filter(|s| s.part == c.part) {
        let cols: Vec<_> = page.columns.iter().filter(|k| k.system == st.system).collect();
        let (Some(first), Some(last)) = (cols.first(), cols.last()) else { continue };
        // The end of a system belongs to the next one, except at the very end.
        let is_last_system = page.columns.iter().all(|k| k.quarter <= last.quarter + 1e-9);
        if c.quarter < first.quarter - 1e-9 || c.quarter > last.quarter + 1e-9 || (c.quarter >= last.quarter - 1e-9 && !is_last_system && c.quarter > first.quarter) {
            continue;
        }
        let Some(x) = column_x(page, st.system, c.quarter) else { continue };
        return Some((x, st.top));
    }
    None
}

fn draw_caret(frame: &mut Frame, page: &DisplayList, origin: Vector, c: Caret, px: f32) {
    if let Some((x, top)) = caret_spot(page, c) {
        let x = origin.x + x - 0.6;
        let line = Path::line(Point::new(x, origin.y + top - 1.5), Point::new(x, origin.y + top + 5.5));
        frame.stroke(&line, Stroke::default().with_width(0.28 * px).with_color(CARET));
        let tick = |y: f32| Path::line(Point::new(x - 0.5, y), Point::new(x + 0.5, y));
        frame.stroke(&tick(origin.y + top - 1.5), Stroke::default().with_width(0.2 * px).with_color(CARET));
        frame.stroke(&tick(origin.y + top + 5.5), Stroke::default().with_width(0.2 * px).with_color(CARET));
    }
}

/// Where a popover points, in desk coordinates: under item `id` if it is
/// on the page, else under the staff at `at` (on the `active` pages).
pub fn anchor(pages: &[DisplayList], origins: &[Vector], active: &std::ops::Range<usize>, id: Option<u64>, at: Option<Caret>) -> Option<Point> {
    let font = MusicFont::bravura();
    if let Some(id) = id.filter(|i| *i != 0) {
        for (p, o) in pages.iter().zip(origins) {
            if let Some(item) = p.items.iter().find(|i| i.id() == id) {
                let r = item.bounds(font);
                return Some(Point::new(o.x + (r[0] + r[2]) / 2.0, o.y + r[3]));
            }
        }
    }
    let c = at?;
    pages.iter().zip(origins).skip(active.start).take(active.len()).find_map(|(p, o)| caret_spot(p, c).map(|(x, top)| Point::new(o.x + x, o.y + top + 5.0)))
}

/// `px`: pixels per staff space. iced transforms a path's points but not its
/// stroke width, so stroke widths are given in pixels.
fn draw_page(frame: &mut Frame, page: &DisplayList, origin: Vector, selected: &std::collections::HashSet<u64>, px: f32) {
    frame.with_save(|f| {
        f.translate(origin);
        // Paper with a soft edge.
        f.fill_rectangle(Point::new(0.5, 0.6), Size::new(page.width, page.height), Color::from_rgba(0.0, 0.0, 0.0, 0.35));
        f.fill_rectangle(Point::ORIGIN, Size::new(page.width, page.height), Color::WHITE);
        for st in &page.staves {
            let lines = Path::new(|b| {
                for k in 0..5 {
                    b.move_to(Point::new(st.left, st.top + k as f32));
                    b.line_to(Point::new(st.right, st.top + k as f32));
                }
            });
            f.stroke(&lines, Stroke::default().with_width(st.line_width * px).with_color(INK));
        }
        for item in &page.items {
            let id = item.id();
            let color = if id != 0 && selected.contains(&id) { SELECTED } else { INK };
            match item {
                Item::Glyph { glyph, at, scale, .. } => {
                    if let Some(p) = glyph_path(*glyph) {
                        f.with_save(|g| {
                            g.translate(Vector::new(at.x, at.y));
                            if (*scale - 1.0).abs() > 1e-6 {
                                g.scale(*scale);
                            }
                            g.fill(&p, color);
                        });
                    }
                }
                Item::Line { from, to, width, .. } => {
                    let p = Path::line(Point::new(from.x, from.y), Point::new(to.x, to.y));
                    f.stroke(&p, Stroke::default().with_width(*width * px).with_color(color));
                }
                Item::Quad { p, .. } => {
                    let path = Path::new(|b| {
                        b.move_to(Point::new(p[0].x, p[0].y));
                        for c in &p[1..] {
                            b.line_to(Point::new(c.x, c.y));
                        }
                        b.close();
                    });
                    f.fill(&path, color);
                }
                Item::Crescent { from, c1, c2, to, d1, d2, .. } => {
                    let q = |v: &corinzia_layout::display::Point| Point::new(v.x, v.y);
                    let path = Path::new(|b| {
                        b.move_to(q(from));
                        b.bezier_curve_to(q(c1), q(c2), q(to));
                        b.bezier_curve_to(q(d2), q(d1), q(from));
                        b.close();
                    });
                    f.fill(&path, color);
                }
                Item::Text { text, at, size, italic, bold, anchor, .. } => draw_text(f, text, *at, *size, *italic, *bold, *anchor),
            }
        }
    });
}

/// Text as vector paths (shaped by cosmic-text), drawn at 10× and scaled down
/// so small sizes keep their outlines.
fn draw_text(f: &mut Frame, content: &str, at: corinzia_layout::display::Point, size: f32, italic: bool, bold: bool, anchor: u8) {
    const K: f32 = 10.0;
    let font = iced::Font {
        family: iced::font::Family::Serif,
        weight: if bold { iced::font::Weight::Bold } else { iced::font::Weight::Normal },
        style: if italic { iced::font::Style::Italic } else { iced::font::Style::Normal },
        ..iced::Font::DEFAULT
    };
    let text = Text {
        content: content.to_string(),
        position: Point::ORIGIN,
        size: iced::Pixels(size * K),
        font,
        align_x: match anchor {
            1 => iced::alignment::Horizontal::Center.into(),
            2 => iced::alignment::Horizontal::Right.into(),
            _ => iced::alignment::Horizontal::Left.into(),
        },
        align_y: iced::alignment::Vertical::Bottom,
        color: INK,
        ..Text::default()
    };
    f.with_save(|g| {
        // Baseline: the bottom of the line box sits ~0.25 em below it.
        g.translate(Vector::new(at.x, at.y + size * 0.25));
        g.scale(1.0 / K);
        text.draw_with(|path, color| g.fill(&path, color));
    });
}
