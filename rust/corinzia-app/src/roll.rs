//! The piano roll (Mix tab, a double-click on a track's lane): one track's
//! notes over the movement, as they play. Pitch and rhythm are the score's;
//! what is edited here is how each note is played, kept apart from the
//! notation (`corinzia_model::Tweak`): drag a note to play it earlier or
//! later, its right edge to hold it longer or shorter, and the velocity
//! lane below for how hard it is played.

use corinzia_model::{Id, Project, Tweak};
use iced::mouse;
use iced::widget::canvas::{self, Event, Frame, Geometry, Path, Stroke, Text};
use iced::{Color, Point, Rectangle, Renderer, Size, Theme};

pub const KEYS: f32 = 52.0;
pub const RULER: f32 = 20.0;
pub const VELOCITY: f32 = 84.0;
pub const ROW: f32 = 9.0;

/// A note as written and as played without tweaks.
#[derive(Clone, Debug, PartialEq)]
pub struct RollNote {
    pub head: Id,
    pub key: i32,
    /// Written quarters: where the note starts (its first pass), and how
    /// long it sounds (its written length after the gate).
    pub start: f64,
    pub length: f64,
    pub velocity: f64,
}

#[derive(Clone, Debug, Default)]
pub struct Roll {
    pub track: Id,
    pub name: String,
    pub notes: Vec<RollNote>,
    pub bars: Vec<f64>,
    pub length_q: f64,
    pub high: i32,
}

/// A track's notes in a movement, as the performance plays them before
/// any tweak (each note once: its first pass through repeats).
pub fn content(project: &Project, movement: usize, track: Id) -> Roll {
    let name = project.track(track).map(|t| t.name.clone()).unwrap_or_default();
    let Some(m) = project.movements.get(movement) else { return Roll { track, name, ..Default::default() } };
    let plain = corinzia_model::Movement { tweaks: vec![], ..m.clone() };
    let perf = corinzia_perform::perform(&plain);
    let q = |f: corinzia_score::Frac| f.num as f64 * 4.0 / f.den as f64;
    let mut notes: Vec<RollNote> = Vec::new();
    for n in &perf.notes {
        if project.instrument(n.instrument).and_then(|i| i.track) != Some(track) || notes.iter().any(|x| x.head == n.element) {
            continue;
        }
        let length = (n.full_off_q - n.on_q) * n.gate;
        notes.push(RollNote { head: n.element, key: project.sounding_key(n.instrument, n.key), start: perf.written_quarter(n.on_q), length, velocity: n.velocity as f64 });
    }
    notes.sort_by(|a, b| a.start.total_cmp(&b.start).then(a.key.cmp(&b.key)));
    let (lo, hi) = notes.iter().fold((i32::MAX, i32::MIN), |(l, h), n| (l.min(n.key), h.max(n.key)));
    let high = if lo > hi { 84 } else { (hi + 6).min(127) };
    let mut bars: Vec<f64> = corinzia_edit::bar_starts(m).into_iter().map(q).collect();
    bars.push(q(m.length));
    Roll { track, name, notes, bars, length_q: q(m.length), high }
}

/// A note as shown: where it plays (written quarters), how long, how hard.
pub fn played(n: &RollNote, t: Option<&Tweak>) -> (f64, f64, f64) {
    let shift = t.map_or(0.0, |t| t.shift);
    (n.start + shift, t.and_then(|t| t.length).unwrap_or(n.length), t.and_then(|t| t.velocity).unwrap_or(n.velocity))
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Drag {
    /// Played earlier or later.
    Shift,
    /// Held longer or shorter.
    Length,
}

#[derive(Debug, Clone)]
pub enum RollEvent {
    /// A note clicked (`add`: Shift, into the selection); none: cleared.
    Select { head: Option<Id>, add: bool },
    /// A drag on the selection, `dq` quarters from where it was pressed.
    Drag { drag: Drag, dq: f64 },
    /// A note's velocity set in the lane below (0..1).
    Velocity { head: Id, velocity: f64 },
    Release,
    Scroll { dx: f32, dy: f32 },
    Zoom(f32),
}

#[derive(Default)]
pub struct State {
    mods: iced::keyboard::Modifiers,
    /// What is being dragged, from where (x).
    drag: Option<(Drag, f32)>,
    /// Painting velocities.
    painting: bool,
}

pub struct RollView<'a> {
    pub roll: &'a Roll,
    pub tweaks: &'a [Tweak],
    pub selected: &'a [Id],
    pub ppq: f32,
    pub scroll_x: f32,
    /// The top row's key.
    pub top_key: i32,
    pub playhead: Option<f64>,
}

impl RollView<'_> {
    fn tweak(&self, head: Id) -> Option<&Tweak> {
        self.tweaks.iter().find(|t| t.head == head)
    }
    pub fn x(&self, q: f64) -> f32 {
        KEYS + q as f32 * self.ppq - self.scroll_x
    }
    fn y_of(&self, key: i32) -> f32 {
        RULER + (self.top_key - key) as f32 * ROW
    }
    pub fn note_rect(&self, n: &RollNote) -> Rectangle {
        let (start, len, _) = played(n, self.tweak(n.head));
        Rectangle::new(Point::new(self.x(start), self.y_of(n.key)), Size::new((len as f32 * self.ppq).max(3.0), ROW - 1.0))
    }
    /// The note under a point, and whether at its right edge.
    pub fn note_at(&self, p: Point) -> Option<(&RollNote, bool)> {
        let n = self.roll.notes.iter().rev().find(|n| self.note_rect(n).expand(1.0).contains(p))?;
        let r = self.note_rect(n);
        Some((n, p.x > r.x + r.width - 5.0f32.min(r.width / 3.0)))
    }
    /// The note whose start is nearest `x` in the velocity lane (the
    /// selected ones first, so a chord's notes can be set one by one).
    fn velocity_note(&self, x: f32) -> Option<&RollNote> {
        let near = |n: &&RollNote| (self.x(played(n, self.tweak(n.head)).0) - x).abs();
        let pick = |only_selected: bool| self.roll.notes.iter().filter(|n| !only_selected || self.selected.contains(&n.head)).filter(|n| near(n) < 6.0).min_by(|a, b| near(a).total_cmp(&near(b)));
        pick(true).or_else(|| pick(false))
    }
    fn lane_top(bounds: Rectangle) -> f32 {
        bounds.height - VELOCITY
    }
}

impl canvas::Program<RollEvent> for RollView<'_> {
    type State = State;

    fn update(&self, state: &mut State, event: &Event, bounds: Rectangle, cursor: mouse::Cursor) -> Option<canvas::Action<RollEvent>> {
        let pos = cursor.position_in(bounds);
        let lane = Self::lane_top(bounds);
        let velocity_at = |p: Point| -> Option<canvas::Action<RollEvent>> {
            let n = self.velocity_note(p.x)?;
            let v = (1.0 - ((p.y - lane - 4.0) / (VELOCITY - 8.0))).clamp(0.0, 1.0) as f64;
            Some(canvas::Action::publish(RollEvent::Velocity { head: n.head, velocity: v }).and_capture())
        };
        match event {
            Event::Keyboard(iced::keyboard::Event::ModifiersChanged(m)) => {
                state.mods = *m;
                None
            }
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let p = pos?;
                if p.x < KEYS {
                    return None;
                }
                if p.y > lane {
                    state.painting = true;
                    return velocity_at(p).or(Some(canvas::Action::capture()));
                }
                match self.note_at(p) {
                    Some((n, edge)) => {
                        state.drag = Some((if edge { Drag::Length } else { Drag::Shift }, p.x));
                        let add = state.mods.shift();
                        // A note already selected keeps the selection (to drag it all).
                        if self.selected.contains(&n.head) && !add {
                            return Some(canvas::Action::capture());
                        }
                        Some(canvas::Action::publish(RollEvent::Select { head: Some(n.head), add }).and_capture())
                    }
                    None => Some(canvas::Action::publish(RollEvent::Select { head: None, add: false }).and_capture()),
                }
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let p = pos?;
                if state.painting {
                    return velocity_at(p);
                }
                let (drag, from) = state.drag?;
                // Fine to the 1/128 of a quarter; Shift: to sixteenths.
                let dq = ((p.x - from) / self.ppq) as f64;
                let dq = if state.mods.shift() { (dq * 4.0).round() / 4.0 } else { (dq * 128.0).round() / 128.0 };
                Some(canvas::Action::publish(RollEvent::Drag { drag, dq }).and_capture())
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) if state.drag.is_some() || state.painting => {
                state.drag = None;
                state.painting = false;
                Some(canvas::Action::publish(RollEvent::Release).and_capture())
            }
            Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                pos?;
                let (dx, dy) = match delta {
                    mouse::ScrollDelta::Lines { x, y } => (x * 40.0, y * 40.0),
                    mouse::ScrollDelta::Pixels { x, y } => (*x, *y),
                };
                let ev = if state.mods.control() {
                    RollEvent::Zoom((1.0 + dy / 400.0).clamp(0.5, 2.0))
                } else if state.mods.shift() {
                    RollEvent::Scroll { dx: -(dx + dy), dy: 0.0 }
                } else {
                    RollEvent::Scroll { dx: -dx, dy: -dy }
                };
                Some(canvas::Action::publish(ev).and_capture())
            }
            _ => None,
        }
    }

    fn draw(&self, _state: &State, renderer: &Renderer, _theme: &Theme, bounds: Rectangle, _cursor: mouse::Cursor) -> Vec<Geometry> {
        let pal = crate::theme::palette();
        let mut f = Frame::new(renderer, bounds.size());
        let lane = Self::lane_top(bounds);
        f.fill_rectangle(Point::ORIGIN, bounds.size(), pal.desk);
        // Key rows: the black keys darker, C named.
        let mut key = self.top_key;
        let mut y = RULER;
        while y < lane {
            let black = matches!(key.rem_euclid(12), 1 | 3 | 6 | 8 | 10);
            if black {
                f.fill_rectangle(Point::new(KEYS, y), Size::new(bounds.width - KEYS, ROW), pal.surface);
            }
            f.fill_rectangle(Point::new(0.0, y), Size::new(KEYS - 2.0, ROW - 1.0), if black { Color::from_rgb(0.18, 0.18, 0.2) } else { Color::from_rgb(0.92, 0.92, 0.9) });
            if key.rem_euclid(12) == 0 {
                f.fill_text(Text { content: format!("C{}", key / 12 - 1), position: Point::new(4.0, y - 1.0), size: iced::Pixels(9.0), color: Color::from_rgb(0.2, 0.2, 0.25), ..Text::default() });
                f.stroke(&Path::line(Point::new(KEYS, y + ROW), Point::new(bounds.width, y + ROW)), Stroke::default().with_width(1.0).with_color(pal.line));
            }
            y += ROW;
            key -= 1;
        }
        // Bars.
        for (i, &b) in self.roll.bars.iter().enumerate() {
            let x = self.x(b);
            if x < KEYS || x > bounds.width {
                continue;
            }
            f.stroke(&Path::line(Point::new(x, RULER - 6.0), Point::new(x, bounds.height)), Stroke::default().with_width(1.0).with_color(pal.line));
            if i + 1 < self.roll.bars.len() {
                f.fill_text(Text { content: format!("{}", i + 1), position: Point::new(x + 3.0, 3.0), size: iced::Pixels(11.0), color: pal.dim, ..Text::default() });
            }
        }
        // Notes: the colour deepens with velocity; tweaked ones outlined.
        for n in &self.roll.notes {
            let r = self.note_rect(n);
            if r.x + r.width < KEYS || r.x > bounds.width || r.y < RULER - ROW || r.y > lane {
                continue;
            }
            let t = self.tweak(n.head);
            let (_, _, v) = played(n, t);
            let selected = self.selected.contains(&n.head);
            let base = if selected { pal.accent } else { Color::from_rgb(0.25, 0.47, 0.85) };
            let x0 = r.x.max(KEYS);
            f.fill_rectangle(Point::new(x0, r.y), Size::new(r.width - (x0 - r.x), r.height), Color { a: 0.35 + 0.65 * v as f32, ..base });
            if t.is_some() {
                f.stroke(&Path::rectangle(Point::new(x0, r.y), Size::new(r.width - (x0 - r.x), r.height)), Stroke::default().with_width(1.5).with_color(Color::WHITE));
            }
        }
        // The velocity lane.
        f.fill_rectangle(Point::new(0.0, lane), Size::new(bounds.width, VELOCITY), pal.base);
        f.fill_text(Text { content: "Velocity".into(), position: Point::new(4.0, lane + 4.0), size: iced::Pixels(10.0), color: pal.dim, ..Text::default() });
        for n in &self.roll.notes {
            let t = self.tweak(n.head);
            let (start, _, v) = played(n, t);
            let x = self.x(start);
            if x < KEYS || x > bounds.width {
                continue;
            }
            let h = (VELOCITY - 8.0) * v as f32;
            let c = if self.selected.contains(&n.head) { pal.accent } else { Color::from_rgb(0.25, 0.47, 0.85) };
            f.fill_rectangle(Point::new(x, lane + 4.0 + (VELOCITY - 8.0) - h), Size::new(3.0, h), c);
        }
        if let Some(q) = self.playhead {
            let x = self.x(q);
            if x >= KEYS {
                f.stroke(&Path::line(Point::new(x, 0.0), Point::new(x, bounds.height)), Stroke::default().with_width(2.0).with_color(pal.success));
            }
        }
        vec![f.into_geometry()]
    }

    fn mouse_interaction(&self, state: &State, bounds: Rectangle, cursor: mouse::Cursor) -> mouse::Interaction {
        match (state.drag, cursor.position_in(bounds).and_then(|p| self.note_at(p))) {
            (Some((Drag::Length, _)), _) | (None, Some((_, true))) => mouse::Interaction::ResizingHorizontally,
            (Some((Drag::Shift, _)), _) => mouse::Interaction::Grabbing,
            (None, Some(_)) => mouse::Interaction::Grab,
            _ => mouse::Interaction::default(),
        }
    }
}
