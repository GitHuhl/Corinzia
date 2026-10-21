//! The arrange view (Mix tab): one lane per DAW track over time, notes as
//! blocks placed by pitch within the lane (a mini piano roll), a bar ruler
//! that locates the playhead, selection shared with the score.

use corinzia_model::{Id, Project};
use iced::mouse;
use iced::widget::canvas::{self, Event, Frame, Geometry, Path, Stroke, Text};
use iced::{Color, Point, Rectangle, Renderer, Size, Theme};

pub const RULER: f32 = 24.0;
pub const HEADER: f32 = 150.0;

#[derive(Clone, Debug, Default)]
pub struct Lane {
    pub track: Id,
    pub name: String,
    pub min_key: i32,
    pub max_key: i32,
}

#[derive(Clone, Copy, Debug)]
pub struct Block {
    pub lane: usize,
    pub start_q: f64,
    pub length_q: f64,
    pub key: i32,
    pub element: Id,
}

#[derive(Clone, Debug, Default)]
pub struct Content {
    pub lanes: Vec<Lane>,
    pub blocks: Vec<Block>,
    pub bars: Vec<f64>, // bar line quarters, with the final barline
    pub length_q: f64,
}

/// The current movement's notes, per track that sounds them.
pub fn content(project: &Project, movement: usize) -> Content {
    let mut c = Content { lanes: project.tracks.iter().map(|t| Lane { track: t.id, name: t.name.clone(), min_key: i32::MAX, max_key: i32::MIN }).collect(), ..Default::default() };
    let Some(m) = project.movements.get(movement) else { return c };
    let q = |f: corinzia_score::Frac| f.num as f64 * 4.0 / f.den as f64;
    for lane in &m.lanes {
        for v in &lane.voices {
            for e in v.events.iter().filter(|e| !e.is_rest()) {
                let Some(track) = lane.instrument_at(e.start).and_then(|i| project.instrument(i)).and_then(|i| i.track) else { continue };
                let Some(li) = c.lanes.iter().position(|l| l.track == track) else { continue };
                for h in &e.heads {
                    let key = h.pitch.midi_key();
                    c.lanes[li].min_key = c.lanes[li].min_key.min(key);
                    c.lanes[li].max_key = c.lanes[li].max_key.max(key);
                    c.blocks.push(Block { lane: li, start_q: q(e.start), length_q: q(e.duration), key, element: h.id });
                }
            }
        }
    }
    for l in c.lanes.iter_mut() {
        if l.min_key > l.max_key {
            (l.min_key, l.max_key) = (60, 72);
        }
        // At least an octave of room, centred.
        let pad = (12 - (l.max_key - l.min_key)).max(0);
        l.min_key -= pad / 2 + 1;
        l.max_key += pad - pad / 2 + 1;
    }
    c.bars = corinzia_edit::bar_starts(m).into_iter().map(q).collect();
    c.length_q = q(m.length);
    c.bars.push(c.length_q);
    c
}

#[derive(Debug, Clone)]
pub enum ArrangeEvent {
    Select(Option<Id>),
    Locate(f64), // quarters
    Scroll(f32),
    Zoom(f32),
    /// Automation in a lane: a new point, a point dragged, a point deleted,
    /// the drag over.
    AutoAdd { lane: usize, q: f64, value: f64 },
    AutoMove { lane: usize, index: usize, q: f64, value: f64 },
    AutoDelete { lane: usize, index: usize },
    AutoRelease,
}

/// A lane's automation as shown: what it drives, its range, its points
/// (quarters, value).
#[derive(Clone, Debug, PartialEq)]
pub struct Curve {
    pub label: String,
    pub min: f64,
    pub max: f64,
    pub points: Vec<(f64, f64)>,
}

/// The canvas's own state: modifiers held, and the point being dragged.
#[derive(Default)]
pub struct State {
    mods: iced::keyboard::Modifiers,
    drag: Option<(usize, usize)>,
}

pub struct ArrangeView<'a> {
    pub content: &'a Content,
    pub pixels_per_quarter: f32,
    pub scroll_x: f32,
    pub lane_height: f32,
    pub selected: &'a std::collections::HashSet<u64>,
    pub playhead: Option<f64>,
    pub cache: &'a canvas::Cache,
    /// Per lane: its automation, when shown (clicks there edit it).
    pub curves: &'a [Option<Curve>],
}

const PALETTE: [Color; 6] = [
    Color::from_rgb(0.25, 0.47, 0.85),
    Color::from_rgb(0.85, 0.45, 0.20),
    Color::from_rgb(0.30, 0.65, 0.40),
    Color::from_rgb(0.65, 0.35, 0.75),
    Color::from_rgb(0.80, 0.30, 0.40),
    Color::from_rgb(0.20, 0.65, 0.70),
];

impl ArrangeView<'_> {
    fn x(&self, q: f64) -> f32 {
        HEADER + q as f32 * self.pixels_per_quarter - self.scroll_x
    }
    fn quarter_at(&self, x: f32) -> f64 {
        ((x - HEADER + self.scroll_x) / self.pixels_per_quarter).max(0.0) as f64
    }
    /// A lane's curve area: (top, height).
    fn curve_band(&self, lane: usize) -> (f32, f32) {
        (RULER + lane as f32 * self.lane_height + 6.0, self.lane_height - 12.0)
    }
    fn curve_y(&self, lane: usize, c: &Curve, v: f64) -> f32 {
        let (top, h) = self.curve_band(lane);
        let k = if c.max > c.min { ((v - c.min) / (c.max - c.min)).clamp(0.0, 1.0) } else { 0.5 };
        top + (1.0 - k as f32) * h
    }
    fn curve_value(&self, lane: usize, c: &Curve, y: f32) -> f64 {
        let (top, h) = self.curve_band(lane);
        let k = (1.0 - ((y - top) / h.max(1.0))).clamp(0.0, 1.0) as f64;
        c.min + k * (c.max - c.min)
    }
    /// A position snapped to the sixteenth of a quarter.
    fn snapped(&self, x: f32) -> f64 {
        (self.quarter_at(x) * 16.0).round() / 16.0
    }
    fn lane_at(&self, y: f32) -> Option<usize> {
        let i = ((y - RULER) / self.lane_height).floor();
        (i >= 0.0 && (i as usize) < self.content.lanes.len()).then_some(i as usize)
    }
    /// The point of a lane's curve under (x, y), if any.
    fn point_at(&self, lane: usize, c: &Curve, p: Point) -> Option<usize> {
        c.points.iter().position(|&(q, v)| (self.x(q) - p.x).abs() < 7.0 && (self.curve_y(lane, c, v) - p.y).abs() < 7.0)
    }

    pub fn block_rect(&self, b: &Block) -> Rectangle {
        let lane = &self.content.lanes[b.lane];
        let span = (lane.max_key - lane.min_key).max(1) as f32;
        let h = (self.lane_height - 8.0) / span;
        let y = RULER + b.lane as f32 * self.lane_height + 4.0 + (lane.max_key - b.key) as f32 * h - h / 2.0;
        Rectangle::new(Point::new(self.x(b.start_q), y), Size::new((b.length_q as f32 * self.pixels_per_quarter - 1.0).max(2.0), h.max(3.0)))
    }
}

impl canvas::Program<ArrangeEvent> for ArrangeView<'_> {
    type State = State;

    fn update(&self, state: &mut Self::State, event: &Event, bounds: Rectangle, cursor: mouse::Cursor) -> Option<canvas::Action<ArrangeEvent>> {
        let pos = cursor.position_in(bounds);
        let mods = &mut state.mods;
        match event {
            Event::Mouse(mouse::Event::CursorMoved { .. }) if state.drag.is_some() => {
                let (lane, index) = state.drag?;
                let p = pos?;
                let c = self.curves.get(lane)?.as_ref()?;
                let ev = ArrangeEvent::AutoMove { lane, index, q: self.snapped(p.x.max(HEADER)), value: self.curve_value(lane, c, p.y) };
                return Some(canvas::Action::publish(ev).and_capture());
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) if state.drag.is_some() => {
                state.drag = None;
                return Some(canvas::Action::publish(ArrangeEvent::AutoRelease).and_capture());
            }
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right)) => {
                let p = pos.filter(|p| p.x > HEADER)?;
                let lane = self.lane_at(p.y)?;
                let c = self.curves.get(lane)?.as_ref()?;
                let index = self.point_at(lane, c, p)?;
                return Some(canvas::Action::publish(ArrangeEvent::AutoDelete { lane, index }).and_capture());
            }
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let p = pos?;
                if p.y < RULER && p.x > HEADER {
                    return Some(canvas::Action::publish(ArrangeEvent::Locate(self.quarter_at(p.x))).and_capture());
                }
                // A lane showing automation: grab a point, or add one.
                if let Some((lane, c)) = self.lane_at(p.y).filter(|_| p.x > HEADER).and_then(|l| Some((l, self.curves.get(l)?.as_ref()?))) {
                    if let Some(index) = self.point_at(lane, c, p) {
                        state.drag = Some((lane, index));
                        return Some(canvas::Action::capture());
                    }
                    let q = self.snapped(p.x);
                    let index = c.points.partition_point(|x| x.0 <= q);
                    state.drag = Some((lane, index));
                    return Some(canvas::Action::publish(ArrangeEvent::AutoAdd { lane, q, value: self.curve_value(lane, c, p.y) }).and_capture());
                }
                let hit = self.content.blocks.iter().rev().find(|b| self.block_rect(b).expand(1.5).contains(p)).map(|b| b.element);
                Some(canvas::Action::publish(ArrangeEvent::Select(hit)).and_capture())
            }
            Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                pos?;
                let (dx, dy) = match delta {
                    mouse::ScrollDelta::Lines { x, y } => (x * 40.0, y * 40.0),
                    mouse::ScrollDelta::Pixels { x, y } => (*x, *y),
                };
                let ev = if mods.control() { ArrangeEvent::Zoom((1.0 + dy / 400.0).clamp(0.5, 2.0)) } else { ArrangeEvent::Scroll(-(dx + dy)) };
                Some(canvas::Action::publish(ev).and_capture())
            }
            Event::Keyboard(iced::keyboard::Event::ModifiersChanged(m)) => {
                *mods = *m;
                None
            }
            _ => None,
        }
    }

    fn draw(&self, _state: &Self::State, renderer: &Renderer, _theme: &Theme, bounds: Rectangle, _cursor: mouse::Cursor) -> Vec<Geometry> {
        let c = self.content;
        let geometry = self.cache.draw(renderer, bounds.size(), |f: &mut Frame| {
            f.fill_rectangle(Point::ORIGIN, bounds.size(), crate::theme::palette().desk);
            f.fill_rectangle(Point::ORIGIN, Size::new(bounds.width, RULER), crate::theme::palette().base);
            // Lanes.
            for (i, _) in c.lanes.iter().enumerate() {
                let y = RULER + i as f32 * self.lane_height;
                if i % 2 == 1 {
                    f.fill_rectangle(Point::new(HEADER, y), Size::new(bounds.width - HEADER, self.lane_height), crate::theme::palette().surface);
                }
                f.stroke(&Path::line(Point::new(0.0, y + self.lane_height), Point::new(bounds.width, y + self.lane_height)), Stroke::default().with_width(1.0).with_color(crate::theme::palette().line));
            }
            // Bar grid and numbers.
            for (n, &bq) in c.bars.iter().enumerate() {
                let x = self.x(bq);
                if x < HEADER || x > bounds.width {
                    continue;
                }
                f.stroke(&Path::line(Point::new(x, RULER - 8.0), Point::new(x, RULER + c.lanes.len() as f32 * self.lane_height)), Stroke::default().with_width(1.0).with_color(crate::theme::palette().line));
                if n + 1 < c.bars.len() {
                    f.fill_text(Text { content: format!("{}", n + 1), position: Point::new(x + 3.0, 4.0), size: iced::Pixels(11.0), color: crate::theme::palette().dim, ..Text::default() });
                }
            }
            // Note blocks.
            for b in &c.blocks {
                let r = self.block_rect(b);
                if r.x + r.width < HEADER || r.x > bounds.width {
                    continue;
                }
                let color = if self.selected.contains(&b.element) { crate::theme::palette().accent } else { PALETTE[b.lane % PALETTE.len()] };
                f.fill_rectangle(Point::new(r.x.max(HEADER), r.y), Size::new(r.width - (HEADER - r.x).max(0.0), r.height), color);
            }
            // Track headers over everything that scrolled under them.
            f.fill_rectangle(Point::ORIGIN, Size::new(HEADER, bounds.height), crate::theme::palette().surface);
            for (i, l) in c.lanes.iter().enumerate() {
                let y = RULER + i as f32 * self.lane_height;
                f.fill_rectangle(Point::new(0.0, y), Size::new(5.0, self.lane_height), PALETTE[i % PALETTE.len()]);
                f.fill_text(Text { content: l.name.clone(), position: Point::new(12.0, y + 8.0), size: iced::Pixels(13.0), ..Text::default() });
            }
            f.fill_text(Text { content: "Tracks".into(), position: Point::new(8.0, 5.0), size: iced::Pixels(12.0), color: crate::theme::palette().dim, ..Text::default() });
        });
        let mut overlay = Frame::new(renderer, bounds.size());
        // Automation over the lanes that show it.
        for (lane, c) in self.curves.iter().enumerate().filter_map(|(i, c)| Some((i, c.as_ref()?))) {
            let (top, h) = self.curve_band(lane);
            overlay.fill_rectangle(Point::new(HEADER, top), Size::new(bounds.width - HEADER, h), Color { a: 0.12, ..crate::theme::palette().accent });
            let mut pts: Vec<Point> = c.points.iter().map(|&(q, v)| Point::new(self.x(q).max(HEADER), self.curve_y(lane, c, v))).collect();
            if let (Some(first), Some(last)) = (pts.first().copied(), pts.last().copied()) {
                pts.insert(0, Point::new(HEADER, first.y));
                pts.push(Point::new(bounds.width, last.y));
                let path = Path::new(|b| {
                    b.move_to(pts[0]);
                    for p in &pts[1..] {
                        b.line_to(*p);
                    }
                });
                overlay.stroke(&path, Stroke::default().with_width(2.0).with_color(crate::theme::palette().accent));
                for p in &pts[1..pts.len() - 1] {
                    if p.x > HEADER {
                        overlay.fill(&Path::circle(*p, 4.0), crate::theme::palette().accent);
                    }
                }
            }
            overlay.fill_text(Text { content: c.label.clone(), position: Point::new(12.0, RULER + lane as f32 * self.lane_height + 26.0), size: iced::Pixels(11.0), color: crate::theme::palette().accent, ..Text::default() });
        }
        if let Some(q) = self.playhead {
            let x = self.x(q);
            if x >= HEADER {
                overlay.stroke(&Path::line(Point::new(x, 0.0), Point::new(x, bounds.height)), Stroke::default().with_width(2.0).with_color(crate::theme::palette().success));
            }
        }
        vec![geometry, overlay.into_geometry()]
    }
}

/// Keeps the scroll inside the content.
pub fn clamp_scroll(scroll: f32, content: &Content, ppq: f32) -> f32 {
    scroll.clamp(0.0, (content.length_q as f32 * ppq - 200.0).max(0.0))
}
