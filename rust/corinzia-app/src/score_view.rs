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

#[derive(Debug, Clone)]
pub enum ViewEvent {
    Select(Option<u64>),
    Pan(Vector),
    Zoom { factor: f32, around: Point },
}

pub struct ScoreView<'a> {
    pub pages: &'a [DisplayList],
    pub vertical: bool,
    pub zoom: f32,
    pub pan: Vector, // screen pixels
    pub selection: Option<u64>,
    pub cache: &'a Cache,
}

#[derive(Default)]
pub struct Interaction {
    drag_from: Option<Point>,
    moved: bool,
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
                state.drag_from = Some(p);
                state.moved = false;
                Some(canvas::Action::capture())
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let (from, p) = (state.drag_from?, pos?);
                let d = p - from;
                if state.moved || d.x.abs() + d.y.abs() > 3.0 {
                    state.moved = true;
                    state.drag_from = Some(p);
                    return Some(canvas::Action::publish(ViewEvent::Pan(d)).and_capture());
                }
                None
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                let from = state.drag_from.take()?;
                if state.moved {
                    return Some(canvas::Action::capture());
                }
                let origins = page_origins(self.pages, self.vertical);
                let hit = hit_test(self.pages, &origins, self.to_desk(from));
                Some(canvas::Action::publish(ViewEvent::Select(hit)).and_capture())
            }
            Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                let p = pos?;
                let (dx, dy) = match delta {
                    mouse::ScrollDelta::Lines { x, y } => (x * 40.0, y * 40.0),
                    mouse::ScrollDelta::Pixels { x, y } => (*x, *y),
                };
                // Ctrl+wheel zooms around the cursor; the wheel alone scrolls
                // (down the pages, or through time in Panorama).
                let ev = if state.modifiers.control() {
                    ViewEvent::Zoom { factor: (1.0f32 + dy / 400.0).clamp(0.5, 2.0), around: p }
                } else if self.vertical {
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

    fn draw(&self, _state: &Interaction, renderer: &Renderer, _theme: &Theme, bounds: Rectangle, _cursor: mouse::Cursor) -> Vec<Geometry> {
        let geometry = self.cache.draw(renderer, bounds.size(), |frame| {
            frame.fill_rectangle(Point::ORIGIN, bounds.size(), DESK);
            frame.translate(self.pan);
            frame.scale(self.scale());
            for (page, origin) in self.pages.iter().zip(page_origins(self.pages, self.vertical)) {
                draw_page(frame, page, origin, self.selection, self.scale());
            }
        });
        vec![geometry]
    }

    fn mouse_interaction(&self, state: &Interaction, _bounds: Rectangle, _cursor: mouse::Cursor) -> mouse::Interaction {
        if state.moved { mouse::Interaction::Grabbing } else { mouse::Interaction::default() }
    }
}

/// `px`: pixels per staff space. iced transforms a path's points but not its
/// stroke width, so stroke widths are given in pixels.
fn draw_page(frame: &mut Frame, page: &DisplayList, origin: Vector, selection: Option<u64>, px: f32) {
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
            let color = if selection.is_some() && item.id() == selection.unwrap_or(0) { SELECTED } else { INK };
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
