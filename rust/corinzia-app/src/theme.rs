//! Corinzia's look: dark gray chrome, a reddish-orange accent, white text
//! and a white page on a dark desk. Every widget style the app uses lives
//! here, so the palette is changed in one place.

use iced::widget::{button, container, text};
use iced::{border, Background, Border, Color, Shadow, Theme, Vector};

const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::from_rgb8(r, g, b)
}

/// The window behind everything (title and status bars).
pub const BASE: Color = rgb(0x1b, 0x1c, 0x1f);
/// Panels and the toolbar.
pub const SURFACE: Color = rgb(0x24, 0x26, 0x2a);
/// Controls at rest.
pub const RAISED: Color = rgb(0x31, 0x34, 0x39);
/// Controls under the pointer.
pub const HOVER: Color = rgb(0x3d, 0x41, 0x47);
/// Hairlines between areas.
pub const LINE: Color = rgb(0x3a, 0x3d, 0x43);
/// Around the page in the score view.
pub const DESK: Color = rgb(0x2c, 0x2e, 0x33);
pub const TEXT: Color = rgb(0xf2, 0xf2, 0xf0);
pub const DIM: Color = rgb(0xa2, 0xa6, 0xad);
pub const FAINT: Color = rgb(0x6c, 0x70, 0x77);
/// The accent: active tools, the primary action, the caret.
pub const ACCENT: Color = rgb(0xe8, 0x57, 0x2a);
pub const ACCENT_HOVER: Color = rgb(0xf2, 0x6c, 0x40);
pub const ACCENT_PRESSED: Color = rgb(0xc8, 0x47, 0x1f);
pub const DANGER: Color = rgb(0xff, 0x6b, 0x5e);
pub const SUCCESS: Color = rgb(0x5c, 0xc2, 0x7f);

pub fn theme() -> Theme {
    Theme::custom(
        "Corinzia",
        iced::theme::Palette { background: SURFACE, text: TEXT, primary: ACCENT, success: SUCCESS, warning: rgb(0xf0, 0xb4, 0x3c), danger: DANGER },
    )
}

const RADIUS: f32 = 4.0;

fn filled(bg: Color, fg: Color) -> button::Style {
    button::Style { background: Some(Background::Color(bg)), text_color: fg, border: border::rounded(RADIUS), shadow: Shadow::default(), snap: true }
}

/// A tool or command: raised gray, lighter under the pointer; accent when
/// `active` (the current duration, a toggled mode).
pub fn tool(active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| match (active, status) {
        (_, button::Status::Disabled) => filled(RAISED, FAINT),
        (true, button::Status::Hovered) => filled(ACCENT_HOVER, TEXT),
        (true, button::Status::Pressed) => filled(ACCENT_PRESSED, TEXT),
        (true, _) => filled(ACCENT, TEXT),
        (false, button::Status::Hovered) => filled(HOVER, TEXT),
        (false, button::Status::Pressed) => filled(ACCENT_PRESSED, TEXT),
        (false, _) => filled(RAISED, TEXT),
    }
}

/// A borderless toolbar button: no background until hovered.
pub fn flat(active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let fg = if matches!(status, button::Status::Disabled) { FAINT } else if active { ACCENT } else { TEXT };
        let bg = match status {
            button::Status::Hovered => Some(Background::Color(HOVER)),
            button::Status::Pressed => Some(Background::Color(RAISED)),
            _ => None,
        };
        button::Style { background: bg, text_color: fg, border: border::rounded(RADIUS), shadow: Shadow::default(), snap: true }
    }
}

/// The primary action (Play, Create): solid accent.
pub fn primary(_: &Theme, status: button::Status) -> button::Style {
    match status {
        button::Status::Hovered => filled(ACCENT_HOVER, TEXT),
        button::Status::Pressed => filled(ACCENT_PRESSED, TEXT),
        button::Status::Disabled => filled(RAISED, FAINT),
        button::Status::Active => filled(ACCENT, TEXT),
    }
}

/// One option of a segmented control (Score | Mix, Page | Panorama): the
/// chosen one is lit, the others sit flush in the control's trough.
pub fn segment(on: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let bg = match (on, status) {
            (true, _) => Some(Background::Color(HOVER)),
            (false, button::Status::Hovered) => Some(Background::Color(RAISED)),
            _ => None,
        };
        button::Style {
            background: bg,
            text_color: if on { TEXT } else { DIM },
            border: Border { color: if on { ACCENT } else { Color::TRANSPARENT }, width: if on { 1.0 } else { 0.0 }, radius: 3.0.into() },
            shadow: Shadow::default(),
            snap: true,
        }
    }
}

/// The trough a segmented control sits in.
pub fn trough(_: &Theme) -> container::Style {
    container::Style { background: Some(Background::Color(BASE)), border: Border { color: LINE, width: 1.0, radius: RADIUS.into() }, ..container::Style::default() }
}

/// The application bar along the top.
pub fn bar(_: &Theme) -> container::Style {
    container::Style {
        text_color: Some(TEXT),
        background: Some(Background::Color(BASE)),
        border: Border { color: LINE, width: 0.0, radius: 0.0.into() },
        ..container::Style::default()
    }
}

/// The status line along the bottom.
pub fn status(_: &Theme) -> container::Style {
    container::Style { text_color: Some(DIM), background: Some(Background::Color(BASE)), ..container::Style::default() }
}

/// A side panel or a mixer area.
pub fn panel(_: &Theme) -> container::Style {
    container::Style { text_color: Some(TEXT), background: Some(Background::Color(SURFACE)), ..container::Style::default() }
}

/// A floating card (popovers, dialogs), lifted off the desk.
pub fn card(_: &Theme) -> container::Style {
    container::Style {
        text_color: Some(TEXT),
        background: Some(Background::Color(SURFACE)),
        border: Border { color: LINE, width: 1.0, radius: 6.0.into() },
        shadow: Shadow { color: Color::from_rgba(0.0, 0.0, 0.0, 0.45), offset: Vector::new(0.0, 6.0), blur_radius: 18.0 },
        snap: true,
    }
}

/// A mixer strip; `selected` outlines it in the accent.
pub fn strip(selected: bool) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        text_color: Some(TEXT),
        background: Some(Background::Color(RAISED)),
        border: Border { color: if selected { ACCENT } else { LINE }, width: 1.0, radius: RADIUS.into() },
        ..container::Style::default()
    }
}

pub fn tooltip(_: &Theme) -> container::Style {
    container::Style {
        text_color: Some(TEXT),
        background: Some(Background::Color(BASE)),
        border: Border { color: LINE, width: 1.0, radius: 3.0.into() },
        ..container::Style::default()
    }
}

/// A thin vertical rule between toolbar groups.
pub fn rule(_: &Theme) -> container::Style {
    container::Style { background: Some(Background::Color(LINE)), ..container::Style::default() }
}

/// A one-pixel hairline below or above a bar.
pub fn hairline(_: &Theme) -> container::Style {
    container::Style { background: Some(Background::Color(LINE)), ..container::Style::default() }
}

pub fn dim(_: &Theme) -> text::Style {
    text::Style { color: Some(DIM) }
}

pub fn faint(_: &Theme) -> text::Style {
    text::Style { color: Some(FAINT) }
}

pub fn accent(_: &Theme) -> text::Style {
    text::Style { color: Some(ACCENT) }
}

pub fn danger(_: &Theme) -> text::Style {
    text::Style { color: Some(DANGER) }
}
