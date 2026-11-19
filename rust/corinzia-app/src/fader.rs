//! Mixer controls drawn on a canvas: the gain fader (a console fader law
//! with a dB scale) and the pan bar. Drag to move (Shift for fine), click
//! the track to jump there, double-click to reset (0 dB, centre), wheel to
//! nudge. Each gesture ends with `MixMsg::Released`: one undo step.

use crate::mix::MixMsg;
use corinzia_model::Id;
use iced::mouse;
use iced::widget::canvas::{self, Event, Frame, Geometry, Path, Stroke, Text};
use iced::{keyboard, Color, Point, Rectangle, Renderer, Size, Theme};
use std::time::{Duration, Instant};

/// The fader's bottom: silence.
pub const MIN_DB: f64 = corinzia_audio::snapshot::MIN_DB;
pub const MAX_DB: f64 = 12.0;

/// The fader law: (dB, position 0..1), piecewise linear, as on a mixing
/// console: unity three quarters up, the lower range compressed.
const LAW: [(f64, f64); 8] = [(MIN_DB, 0.0), (-48.0, 0.06), (-36.0, 0.15), (-24.0, 0.3), (-12.0, 0.5), (-6.0, 0.625), (0.0, 0.75), (MAX_DB, 1.0)];

pub fn db_to_pos(db: f64) -> f64 {
    if db <= MIN_DB {
        return 0.0;
    }
    for w in LAW.windows(2) {
        let ((d0, p0), (d1, p1)) = (w[0], w[1]);
        if db <= d1 {
            return p0 + (db - d0) / (d1 - d0) * (p1 - p0);
        }
    }
    1.0
}

pub fn pos_to_db(pos: f64) -> f64 {
    let pos = pos.clamp(0.0, 1.0);
    for w in LAW.windows(2) {
        let ((d0, p0), (d1, p1)) = (w[0], w[1]);
        if pos <= p1 {
            return d0 + (pos - p0) / (p1 - p0) * (d1 - d0);
        }
    }
    MAX_DB
}

/// Values a control lands on: 0.1 dB steps (0.5 below -24 dB).
fn snap_db(db: f64) -> f64 {
    if db <= MIN_DB + 0.05 {
        MIN_DB
    } else if db < -24.0 {
        (db * 2.0).round() / 2.0
    } else {
        (db * 10.0).round() / 10.0
    }
}

const DOUBLE_CLICK: Duration = Duration::from_millis(350);
const CAP_H: f32 = 26.0;
const CAP_W: f32 = 18.0;
const MARGIN: f32 = 12.0; // above and below the travel

#[derive(Default)]
pub struct State {
    /// While dragging: the cursor's y and the position at the press.
    drag: Option<(f32, f64)>,
    last_click: Option<Instant>,
    fine: bool,
}

/// The gain fader of track `track`, at `db`.
pub struct Fader {
    pub track: Id,
    pub db: f64,
}

impl Fader {
    fn travel(bounds: Rectangle) -> (f32, f32) {
        (MARGIN, bounds.height - MARGIN) // top (max) to bottom (min), local y
    }
    fn y_of(&self, bounds: Rectangle, pos: f64) -> f32 {
        let (t, b) = Self::travel(bounds);
        b - (pos as f32) * (b - t)
    }
    fn pos_at(bounds: Rectangle, y: f32) -> f64 {
        let (t, b) = Self::travel(bounds);
        ((b - y) / (b - t)) as f64
    }
}

impl canvas::Program<MixMsg> for Fader {
    type State = State;

    fn update(&self, state: &mut State, event: &Event, bounds: Rectangle, cursor: mouse::Cursor) -> Option<canvas::Action<MixMsg>> {
        let gain = |db: f64| MixMsg::Gain(self.track, snap_db(db.clamp(MIN_DB, MAX_DB)));
        match event {
            Event::Keyboard(keyboard::Event::ModifiersChanged(m)) => {
                state.fine = m.shift();
                None
            }
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let p = cursor.position_in(bounds)?;
                let now = Instant::now();
                if state.last_click.is_some_and(|t| now - t < DOUBLE_CLICK) {
                    state.last_click = None;
                    state.drag = None;
                    return Some(canvas::Action::publish(MixMsg::Gain(self.track, 0.0)).and_capture());
                }
                state.last_click = Some(now);
                let pos = db_to_pos(self.db);
                let cap = self.y_of(bounds, pos);
                if (p.y - cap).abs() <= CAP_H / 2.0 + 2.0 {
                    // Grabbed the cap: it follows the cursor from here.
                    state.drag = Some((p.y, pos));
                    Some(canvas::Action::capture())
                } else {
                    // On the track: the cap jumps there.
                    let to = Self::pos_at(bounds, p.y);
                    state.drag = Some((p.y, to));
                    Some(canvas::Action::publish(gain(pos_to_db(to))).and_capture())
                }
            }
            Event::Mouse(mouse::Event::CursorMoved { position }) => {
                let (y0, p0) = state.drag?;
                let (t, b) = Self::travel(bounds);
                let k = if state.fine { 0.15 } else { 1.0 };
                let pos = p0 + ((y0 - (position.y - bounds.y)) / (b - t) * k) as f64;
                Some(canvas::Action::publish(gain(pos_to_db(pos))).and_capture())
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                state.drag.take()?;
                Some(canvas::Action::publish(MixMsg::Released).and_capture())
            }
            Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                cursor.position_in(bounds)?;
                let lines = match delta {
                    mouse::ScrollDelta::Lines { y, .. } => *y,
                    mouse::ScrollDelta::Pixels { y, .. } => *y / 30.0,
                };
                let step = if state.fine { 0.1 } else { 0.5 };
                let from = if self.db <= MIN_DB { -60.0 } else { self.db };
                Some(canvas::Action::publish(gain(from + lines as f64 * step)).and_capture())
            }
            _ => None,
        }
    }

    fn draw(&self, state: &State, renderer: &Renderer, _theme: &Theme, bounds: Rectangle, cursor: mouse::Cursor) -> Vec<Geometry> {
        let mut f = Frame::new(renderer, bounds.size());
        let (t, b) = Self::travel(bounds);
        let pal = crate::theme::palette();
        // The scale on the left, the groove and its cap on the right.
        let cx = bounds.width - CAP_W / 2.0 - 1.0;
        f.fill(&Path::rectangle(Point::new(cx - 1.5, t), Size::new(3.0, b - t)), pal.base);
        f.stroke(&Path::rectangle(Point::new(cx - 1.5, t), Size::new(3.0, b - t)), Stroke::default().with_width(1.0).with_color(pal.line));
        for (db, label) in [(12.0, "+12"), (6.0, "+6"), (0.0, "0"), (-6.0, "-6"), (-12.0, "-12"), (-24.0, "-24"), (-36.0, "-36"), (-48.0, "-48"), (MIN_DB, "-∞")] {
            let y = self.y_of(bounds, db_to_pos(db));
            let unity = db == 0.0;
            f.stroke(
                &Path::line(Point::new(cx - CAP_W / 2.0 - 4.0, y), Point::new(cx - CAP_W / 2.0 - 1.0, y)),
                Stroke::default().with_width(1.0).with_color(if unity { pal.dim } else { pal.faint }),
            );
            f.fill_text(Text {
                content: label.into(),
                position: Point::new(cx - CAP_W / 2.0 - 6.0, y),
                color: if unity { pal.dim } else { pal.faint },
                size: 9.0.into(),
                align_x: iced::alignment::Horizontal::Right.into(),
                align_y: iced::alignment::Vertical::Center,
                ..Text::default()
            });
        }
        let y = self.y_of(bounds, db_to_pos(self.db));
        let hot = state.drag.is_some() || cursor.position_in(bounds).is_some_and(|p| (p.y - y).abs() <= CAP_H / 2.0 + 2.0);
        let cap = Path::rounded_rectangle(Point::new(cx - CAP_W / 2.0, y - CAP_H / 2.0), Size::new(CAP_W, CAP_H), 2.0.into());
        f.fill(&cap, if hot { Color::from_rgb8(0x6a, 0x6e, 0x76) } else { Color::from_rgb8(0x52, 0x56, 0x5d) });
        f.stroke(&cap, Stroke::default().with_width(1.0).with_color(Color::from_rgb8(0x14, 0x15, 0x17)));
        for dy in [-4.0, 4.0] {
            f.stroke(&Path::line(Point::new(cx - CAP_W / 2.0 + 3.0, y + dy), Point::new(cx + CAP_W / 2.0 - 3.0, y + dy)), Stroke::default().with_width(1.0).with_color(Color::from_rgb8(0x3c, 0x3f, 0x45)));
        }
        f.stroke(&Path::line(Point::new(cx - CAP_W / 2.0 + 2.0, y), Point::new(cx + CAP_W / 2.0 - 2.0, y)), Stroke::default().with_width(1.5).with_color(Color::WHITE));
        vec![f.into_geometry()]
    }

    fn mouse_interaction(&self, state: &State, bounds: Rectangle, cursor: mouse::Cursor) -> mouse::Interaction {
        if state.drag.is_some() {
            mouse::Interaction::Grabbing
        } else if cursor.is_over(bounds) {
            mouse::Interaction::Grab
        } else {
            mouse::Interaction::default()
        }
    }
}

/// A pan knob: an arc from the top (centre) towards the side panned to;
/// drag up or down (Shift: fine), wheel, double-click to centre.
pub struct PanKnob {
    pub track: Id,
    pub pan: f64,
}

impl canvas::Program<MixMsg> for PanKnob {
    type State = State;

    fn update(&self, state: &mut State, event: &Event, bounds: Rectangle, cursor: mouse::Cursor) -> Option<canvas::Action<MixMsg>> {
        let pan = |v: f64| {
            let v = v.clamp(-1.0, 1.0);
            MixMsg::Pan(self.track, if v.abs() < 0.02 { 0.0 } else { (v * 100.0).round() / 100.0 })
        };
        match event {
            Event::Keyboard(keyboard::Event::ModifiersChanged(m)) => {
                state.fine = m.shift();
                None
            }
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let p = cursor.position_in(bounds)?;
                let now = Instant::now();
                if state.last_click.is_some_and(|t| now - t < DOUBLE_CLICK) {
                    state.last_click = None;
                    state.drag = None;
                    return Some(canvas::Action::publish(MixMsg::Pan(self.track, 0.0)).and_capture());
                }
                state.last_click = Some(now);
                state.drag = Some((p.y, self.pan));
                Some(canvas::Action::capture())
            }
            Event::Mouse(mouse::Event::CursorMoved { position }) => {
                let (y0, v0) = state.drag?;
                let k = if state.fine { 0.002 } else { 0.012 };
                Some(canvas::Action::publish(pan(v0 + ((y0 - (position.y - bounds.y)) * k) as f64)).and_capture())
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                state.drag.take()?;
                Some(canvas::Action::publish(MixMsg::Released).and_capture())
            }
            Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                cursor.position_in(bounds)?;
                let lines = match delta {
                    mouse::ScrollDelta::Lines { y, .. } => *y,
                    mouse::ScrollDelta::Pixels { y, .. } => *y / 30.0,
                };
                Some(canvas::Action::publish(pan(self.pan + lines as f64 * if state.fine { 0.01 } else { 0.05 })).and_capture())
            }
            _ => None,
        }
    }

    fn draw(&self, _state: &State, renderer: &Renderer, _theme: &Theme, bounds: Rectangle, _cursor: mouse::Cursor) -> Vec<Geometry> {
        use std::f32::consts::PI;
        let mut f = Frame::new(renderer, bounds.size());
        let pal = crate::theme::palette();
        let c = Point::new(bounds.width / 2.0, bounds.height / 2.0);
        let r = (bounds.width.min(bounds.height) / 2.0 - 2.0).max(4.0);
        // 135° each way from the top.
        let angle = |v: f64| -PI / 2.0 + v as f32 * PI * 0.75;
        let arc = |from: f32, to: f32| Path::new(|b| b.arc(canvas::path::Arc { center: c, radius: r, start_angle: iced::Radians(from.min(to)), end_angle: iced::Radians(from.max(to)) }));
        f.stroke(&arc(angle(-1.0), angle(1.0)), Stroke::default().with_width(3.0).with_color(pal.base));
        if self.pan.abs() > 0.004 {
            f.stroke(&arc(angle(0.0), angle(self.pan)), Stroke::default().with_width(3.0).with_color(Color::from_rgb8(0x6f, 0xb3, 0xe8)));
        }
        f.fill(&Path::circle(c, r - 3.0), Color::from_rgb8(0x4a, 0x4e, 0x55));
        let a = angle(self.pan);
        f.stroke(&Path::line(c, Point::new(c.x + a.cos() * (r - 4.0), c.y + a.sin() * (r - 4.0))), Stroke::default().with_width(1.5).with_color(Color::WHITE));
        vec![f.into_geometry()]
    }

    fn mouse_interaction(&self, state: &State, bounds: Rectangle, cursor: mouse::Cursor) -> mouse::Interaction {
        if state.drag.is_some() {
            mouse::Interaction::ResizingVertically
        } else if cursor.is_over(bounds) {
            mouse::Interaction::Pointer
        } else {
            mouse::Interaction::default()
        }
    }
}

/// A stereo level meter beside a fader, on the fader's scale (so 0 dB
/// lines up): peaks held with a decay, green to the fader's −6 dB, amber
/// to 0 dB, red over; a clip light on top that a click puts out.
pub struct Meter {
    pub id: Id,
    pub level: (f32, f32),
    pub clipped: bool,
}

impl canvas::Program<MixMsg> for Meter {
    type State = ();

    fn update(&self, _: &mut (), event: &Event, bounds: Rectangle, cursor: mouse::Cursor) -> Option<canvas::Action<MixMsg>> {
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) if self.clipped && cursor.position_in(bounds).is_some_and(|p| p.y < MARGIN) => {
                Some(canvas::Action::publish(MixMsg::ClearClip(self.id)).and_capture())
            }
            _ => None,
        }
    }

    fn draw(&self, _: &(), renderer: &Renderer, _theme: &Theme, bounds: Rectangle, _cursor: mouse::Cursor) -> Vec<Geometry> {
        let mut f = Frame::new(renderer, bounds.size());
        let (top, bottom) = (MARGIN, bounds.height - MARGIN);
        let y = |pos: f64| bottom - pos as f32 * (bottom - top);
        let w = (bounds.width - 1.0) / 2.0;
        let well = crate::theme::palette().line;
        let (green, amber, red) = (Color::from_rgb(0.30, 0.72, 0.42), Color::from_rgb(0.92, 0.70, 0.20), Color::from_rgb(0.88, 0.26, 0.24));
        for (k, &v) in [self.level.0, self.level.1].iter().enumerate() {
            let x = k as f32 * (w + 1.0);
            f.fill_rectangle(Point::new(x, top), Size::new(w, bottom - top), Color { a: 0.35, ..well });
            if v <= 1e-6 {
                continue;
            }
            let db = 20.0 * (v as f64).log10();
            let at = y(db_to_pos(db.min(MAX_DB)));
            // Coloured by zone: up to −6 dB, to 0 dB, over.
            for (from, to, c) in [(MIN_DB, -6.0, green), (-6.0, 0.0, amber), (0.0, MAX_DB, red)] {
                let (y0, y1) = (y(db_to_pos(from)), y(db_to_pos(to)).max(at));
                if y1 < y0 {
                    f.fill_rectangle(Point::new(x, y1), Size::new(w, y0 - y1), c);
                }
            }
        }
        let light = if self.clipped { red } else { Color { a: 0.35, ..well } };
        f.fill_rectangle(Point::new(0.0, 0.0), Size::new(bounds.width, MARGIN - 3.0), light);
        vec![f.into_geometry()]
    }

    fn mouse_interaction(&self, _: &(), bounds: Rectangle, cursor: mouse::Cursor) -> mouse::Interaction {
        if self.clipped && cursor.position_in(bounds).is_some_and(|p| p.y < MARGIN) {
            mouse::Interaction::Pointer
        } else {
            mouse::Interaction::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fader_law_round_trips_and_puts_unity_three_quarters_up() {
        assert_eq!(db_to_pos(0.0), 0.75);
        assert_eq!(db_to_pos(MIN_DB), 0.0);
        assert_eq!(db_to_pos(MAX_DB), 1.0);
        for db in [-59.0, -40.0, -18.0, -3.0, 0.0, 4.5, 12.0] {
            assert!((pos_to_db(db_to_pos(db)) - db).abs() < 1e-9, "{db}");
        }
        // More travel per dB near unity than near the bottom.
        assert!(db_to_pos(0.0) - db_to_pos(-6.0) > db_to_pos(-42.0) - db_to_pos(-48.0));
        assert_eq!(snap_db(-60.02), MIN_DB);
        assert_eq!(snap_db(-3.04), -3.0);
    }
}
