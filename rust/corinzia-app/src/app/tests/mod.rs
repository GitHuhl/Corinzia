//! The app's tests, driving App through its messages; shared helpers here.

use super::*;
use crate::fader;
use corinzia_layout::display::Item;
use crate::editor::SlurHandle;
use corinzia_model::dynamic_index;

mod dialogs;
mod document;
mod mixing;
mod playback;
mod score;

fn app(view: View) -> App {
    App::new(demo::project(), None, view, String::new())
}

/// Screen position (in the canvas) of the notehead with this id.
fn head_on_screen(a: &App, id: u64) -> iced::Point {
    let origins = score_view::page_origins(&a.pages, a.view == View::Page);
    for (p, o) in a.pages.iter().zip(&origins) {
        for it in &p.items {
            if let Item::Glyph { id: gid, .. } = it {
                if *gid == id {
                    let r = it.bounds(corinzia_layout::font::Font::bravura());
                    let px = a.zoom * score_view::BASE_PX;
                    return iced::Point::new(((r[0] + r[2]) / 2.0 + o.x) * px + a.pan.x, ((r[1] + r[3]) / 2.0 + o.y) * px + a.pan.y);
                }
            }
        }
    }
    panic!("head {id} not engraved");
}

/// The desk point (staff spaces) in the middle of the first item with this id.
fn item_on_desk(a: &App, id: u64) -> iced::Point {
    let origins = score_view::page_origins(&a.pages, a.view == View::Page);
    for (p, o) in a.pages.iter().zip(&origins) {
        if let Some(it) = p.items.iter().find(|it| it.id() == id) {
            let r = it.bounds(corinzia_layout::font::Font::bravura());
            return iced::Point::new((r[0] + r[2]) / 2.0 + o.x, (r[1] + r[3]) / 2.0 + o.y);
        }
    }
    panic!("item {id} not engraved");
}

/// Clicks what is at the middle of item `id`, `n` times (2 = a double-click).
fn click_item(a: &mut App, id: u64, n: usize) {
    let origins = score_view::page_origins(&a.pages, a.view == View::Page);
    let hit = score_view::hit_test(&a.pages, &origins, item_on_desk(a, id));
    assert_eq!(hit, Some(id), "item {id} is a click target");
    let _ = a.update(Message::Canvas(ViewEvent::Click { hit, at: None, shift: false }));
    if n == 2 {
        let _ = a.update(Message::Canvas(ViewEvent::DoubleClick { hit }));
    }
}

fn retype(a: &mut App, text: &str) {
    let _ = a.update(Message::Popover(PopoverMsg::Input(text.into())));
    let _ = a.update(Message::Popover(PopoverMsg::Submit));
    assert!(a.popover.is_none(), "accepted: {:?}", a.popover.as_ref().and_then(|p| p.error.clone()));
    a.sync();
}

/// Right-clicks the desk point `at` (staff spaces).
fn right_click(a: &mut App, at: iced::Point) {
    let origins = score_view::page_origins(&a.pages, a.view == View::Page);
    let target = score_view::context_target(&a.pages, &origins, at).expect("a clef, signature or barline there");
    let px = a.zoom * score_view::BASE_PX;
    let screen = iced::Point::new(at.x * px + a.pan.x, at.y * px + a.pan.y);
    let _ = a.update(Message::Canvas(ViewEvent::Context { target, at: score_view::locate(&a.pages, &origins, at), screen }));
    assert_eq!(a.popover.as_ref().and_then(|p| p.anchor), Some(screen), "the popover opens by the click");
}

/// Desk points of the marks without an id matching `pick` on page 1.
fn marks(a: &App, pick: impl Fn(&Item) -> bool) -> Vec<iced::Point> {
    let font = corinzia_layout::font::Font::bravura();
    a.pages[0].items.iter().filter(|i| i.id() == 0 && pick(i)).map(|i| {
        let r = i.bounds(font);
        iced::Point::new((r[0] + r[2]) / 2.0, (r[1] + r[3]) / 2.0)
    }).collect()
}

fn key(a: &mut App, token: &str) {
    let (k, m) = parse_key_token(token).unwrap();
    let _ = a.update(Message::Key(iced::keyboard::Event::KeyPressed {
        key: k.clone(),
        modified_key: k,
        physical_key: iced::keyboard::key::Physical::Unidentified(iced::keyboard::key::NativeCode::Unidentified),
        location: iced::keyboard::Location::Standard,
        modifiers: m,
        text: None,
        repeat: false,
    }));
}

fn render(a: &mut App, frames: usize) -> f32 {
    let (l, _) = a.playback.offline.as_mut().expect("offline engine").render_offline(frames);
    (l.iter().map(|v| v * v).sum::<f32>() / frames as f32).sqrt()
}

fn type_into_popover(a: &mut App, text: &str) {
    let mut buf = String::new();
    for c in text.chars() {
        buf.push(c);
        let _ = a.update(Message::Popover(PopoverMsg::Input(buf.clone())));
        buf = a.popover.as_ref().map(|p| p.input.clone()).unwrap_or_default();
    }
}
