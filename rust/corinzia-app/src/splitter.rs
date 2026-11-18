//! A divider between two panes that is dragged to resize them.

use iced::mouse;
use iced::widget::canvas::{self, Event, Frame, Geometry};
use iced::{Point, Rectangle, Renderer, Size, Theme};

/// A bar across the space between two panes. `vertical`: a vertical bar
/// (the panes left and right), dragged sideways; else horizontal, dragged
/// up and down. It publishes how far it moved since the last event, then
/// `None` when let go (the time to remember the sizes).
pub struct Splitter {
    pub vertical: bool,
}

#[derive(Default)]
pub struct State {
    last: Option<f32>,
    hover: bool,
}

/// Its thickness.
pub const SIZE: f32 = 6.0;

impl canvas::Program<Option<f32>> for Splitter {
    type State = State;

    fn update(&self, state: &mut State, event: &Event, bounds: Rectangle, cursor: mouse::Cursor) -> Option<canvas::Action<Option<f32>>> {
        let along = |p: Point| if self.vertical { p.x } else { p.y };
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let p = cursor.position_over(bounds)?;
                state.last = Some(along(p));
                Some(canvas::Action::request_redraw().and_capture())
            }
            Event::Mouse(mouse::Event::CursorMoved { position }) => {
                let hover = cursor.is_over(bounds);
                let redraw = hover != state.hover;
                state.hover = hover;
                match state.last {
                    Some(last) => {
                        let now = along(*position);
                        state.last = Some(now);
                        Some(canvas::Action::publish(Some(now - last)).and_capture())
                    }
                    None if redraw => Some(canvas::Action::request_redraw()),
                    None => None,
                }
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) if state.last.is_some() => {
                state.last = None;
                Some(canvas::Action::publish(None).and_capture())
            }
            _ => None,
        }
    }

    fn draw(&self, state: &State, renderer: &Renderer, _theme: &Theme, bounds: Rectangle, _cursor: mouse::Cursor) -> Vec<Geometry> {
        let mut f = Frame::new(renderer, bounds.size());
        let p = crate::theme::palette();
        let active = state.hover || state.last.is_some();
        f.fill_rectangle(Point::ORIGIN, bounds.size(), if active { p.accent } else { p.line });
        // A grip in the middle.
        let c = Point::new(bounds.width / 2.0, bounds.height / 2.0);
        for k in [-6.0f32, 0.0, 6.0] {
            let at = if self.vertical { Point::new(c.x - 1.0, c.y + k - 1.0) } else { Point::new(c.x + k - 1.0, c.y - 1.0) };
            f.fill_rectangle(at, Size::new(2.0, 2.0), if active { p.base } else { p.dim });
        }
        vec![f.into_geometry()]
    }

    fn mouse_interaction(&self, state: &State, bounds: Rectangle, cursor: mouse::Cursor) -> mouse::Interaction {
        if state.last.is_some() || cursor.is_over(bounds) {
            if self.vertical { mouse::Interaction::ResizingHorizontally } else { mouse::Interaction::ResizingVertically }
        } else {
            mouse::Interaction::default()
        }
    }
}

/// A splitter as an element of the given size along its length.
pub fn view<'a, M: 'a>(vertical: bool, on: impl Fn(Option<f32>) -> M + 'a) -> iced::Element<'a, M> {
    let c = iced::widget::canvas(Splitter { vertical });
    let c = if vertical { c.width(SIZE).height(iced::Length::Fill) } else { c.height(SIZE).width(iced::Length::Fill) };
    iced::Element::from(c).map(on)
}
