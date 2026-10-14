//! The score canvas: engraved pages (display lists from corinzia-layout)
//! drawn as vector geometry, so every zoom level is sharp. Glyph outlines
//! come straight from Bravura; text is converted to paths as well. Geometry
//! is cached and rebuilt only when the engraving, zoom, pan or selection
//! changes.

use corinzia_layout::display::{DisplayList, Item};
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
const SELECTED: Color = Color::from_rgb(0.12, 0.42, 0.86);
const PLAYHEAD: Color = Color::from_rgb(0.1, 0.62, 0.3);
const CARET: Color = Color::from_rgb(0.95, 0.45, 0.05);
const DESK: Color = Color::from_rgb(0.84, 0.83, 0.80);

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

/// The element under a point given in staff spaces (desk coordinates).
pub fn hit_test(pages: &[DisplayList], origins: &[Vector], at: Point) -> Option<u64> {
    let font = MusicFont::bravura();
    let mut best: Option<(f32, u64)> = None;
    for (p, o) in pages.iter().zip(origins) {
        let local = Point::new(at.x - o.x, at.y - o.y);
        for item in &p.items {
            let Item::Glyph { glyph, id, .. } = item else { continue };
            if *id == 0 || !is_hit_target(*glyph) {
                continue;
            }
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
    pub part: usize,
    pub quarter: f64,
    pub step_below_top: i32,
}

/// The staff point under a desk position: the nearest staff of the system
/// it falls in, with time interpolated between that system's columns.
pub fn locate(pages: &[DisplayList], origins: &[Vector], at: Point) -> Option<StaffPoint> {
    for (p, o) in pages.iter().zip(origins) {
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
        return Some(StaffPoint { part: st.part, quarter, step_below_top: ((local.y - st.top) * 2.0).round() as i32 });
    }
    None
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
    /// The playback position (written quarters) while playing.
    pub playhead: Option<f64>,
    pub cache: &'a Cache,
}

#[derive(Default)]
pub struct Interaction {
    press: Option<Point>,
    current: Option<Point>,
    marquee: bool,
    panning: Option<Point>,
    modifiers: iced::keyboard::Modifiers,
}

impl ScoreView<'_> {
    fn scale(&self) -> f32 {
        self.zoom * BASE_PX
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
                state.press = Some(p);
                state.current = Some(p);
                state.marquee = false;
                Some(canvas::Action::capture())
            }
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Middle | mouse::Button::Right)) => {
                state.panning = Some(pos?);
                Some(canvas::Action::capture())
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                if let (Some(from), Some(p)) = (state.panning, pos) {
                    state.panning = Some(p);
                    return Some(canvas::Action::publish(ViewEvent::Pan(p - from)).and_capture());
                }
                let (from, p) = (state.press?, pos?);
                let d = p - from;
                if state.marquee || d.x.abs() + d.y.abs() > 4.0 {
                    state.marquee = true;
                    state.current = Some(p);
                    return Some(canvas::Action::request_redraw().and_capture());
                }
                None
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Middle | mouse::Button::Right)) => {
                state.panning.take().map(|_| canvas::Action::capture())
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
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
            frame.fill_rectangle(Point::ORIGIN, bounds.size(), DESK);
            frame.translate(self.pan);
            frame.scale(px);
            for (page, origin) in self.pages.iter().zip(&origins) {
                draw_page(frame, page, *origin, self.selected, px);
                if let Some(h) = self.highlight {
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
                for (page, origin) in self.pages.iter().zip(&origins) {
                    draw_caret(f, page, *origin, c, px);
                }
            }
            if let Some(q) = self.playhead {
                for (page, origin) in self.pages.iter().zip(&origins) {
                    draw_playhead(f, page, *origin, q, px);
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

    fn mouse_interaction(&self, state: &Interaction, _bounds: Rectangle, _cursor: mouse::Cursor) -> mouse::Interaction {
        if state.panning.is_some() {
            mouse::Interaction::Grabbing
        } else if state.marquee {
            mouse::Interaction::Crosshair
        } else {
            mouse::Interaction::default()
        }
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

fn draw_caret(frame: &mut Frame, page: &DisplayList, origin: Vector, c: Caret, px: f32) {
    for st in page.staves.iter().filter(|s| s.part == c.part) {
        let cols: Vec<_> = page.columns.iter().filter(|k| k.system == st.system).collect();
        let (Some(first), Some(last)) = (cols.first(), cols.last()) else { continue };
        // The end of a system belongs to the next one, except at the very end.
        let is_last_system = page.columns.iter().all(|k| k.quarter <= last.quarter + 1e-9);
        if c.quarter < first.quarter - 1e-9 || c.quarter > last.quarter + 1e-9 || (c.quarter >= last.quarter - 1e-9 && !is_last_system && c.quarter > first.quarter) {
            continue;
        }
        let Some(x) = column_x(page, st.system, c.quarter) else { continue };
        let x = origin.x + x - 0.6;
        let line = Path::line(Point::new(x, origin.y + st.top - 1.5), Point::new(x, origin.y + st.top + 5.5));
        frame.stroke(&line, Stroke::default().with_width(0.28 * px).with_color(CARET));
        let tick = |y: f32| Path::line(Point::new(x - 0.5, y), Point::new(x + 0.5, y));
        frame.stroke(&tick(origin.y + st.top - 1.5), Stroke::default().with_width(0.2 * px).with_color(CARET));
        frame.stroke(&tick(origin.y + st.top + 5.5), Stroke::default().with_width(0.2 * px).with_color(CARET));
        return;
    }
}

/// `px`: pixels per staff space. iced transforms a path's points but not its
/// stroke width, so stroke widths are given in pixels.
fn draw_page(frame: &mut Frame, page: &DisplayList, origin: Vector, selected: &std::collections::HashSet<u64>, px: f32) {
    frame.with_save(|f| {
        f.translate(origin);
        // Paper with a soft edge.
        f.fill_rectangle(Point::new(0.35, 0.35), Size::new(page.width, page.height), Color::from_rgba(0.0, 0.0, 0.0, 0.18));
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
