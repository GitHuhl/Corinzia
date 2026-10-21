//! Corinzia's look: dark gray chrome, a reddish-orange accent, white text
//! and a white page on a dark desk. Every widget style the app uses lives
//! here, so the palette is changed in one place.

use iced::widget::{button, container, text};
use iced::{border, Background, Border, Color, Shadow, Theme, Vector};

const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::from_rgb8(r, g, b)
}

/// A colour scheme: every colour the app's chrome uses.
pub struct Palette {
    /// The window behind everything (title and status bars).
    pub base: Color,
    /// Panels and the toolbar.
    pub surface: Color,
    /// Controls at rest.
    pub raised: Color,
    /// Controls under the pointer.
    pub hover: Color,
    /// Hairlines between areas.
    pub line: Color,
    /// Around the page in the score view.
    pub desk: Color,
    pub text: Color,
    pub dim: Color,
    pub faint: Color,
    /// The accent: active tools, the primary action, the caret.
    pub accent: Color,
    pub accent_hover: Color,
    pub accent_pressed: Color,
    pub danger: Color,
    pub success: Color,
}

pub const DARK: Palette = Palette {
    base: rgb(0x1b, 0x1c, 0x1f),
    surface: rgb(0x24, 0x26, 0x2a),
    raised: rgb(0x31, 0x34, 0x39),
    hover: rgb(0x3d, 0x41, 0x47),
    line: rgb(0x3a, 0x3d, 0x43),
    desk: rgb(0x2c, 0x2e, 0x33),
    text: rgb(0xf2, 0xf2, 0xf0),
    dim: rgb(0xa2, 0xa6, 0xad),
    faint: rgb(0x6c, 0x70, 0x77),
    accent: rgb(0xe8, 0x57, 0x2a),
    accent_hover: rgb(0xf2, 0x6c, 0x40),
    accent_pressed: rgb(0xc8, 0x47, 0x1f),
    danger: rgb(0xff, 0x6b, 0x5e),
    success: rgb(0x5c, 0xc2, 0x7f),
};

pub const LIGHT: Palette = Palette {
    base: rgb(0xdc, 0xdc, 0xd8),
    surface: rgb(0xec, 0xec, 0xe9),
    raised: rgb(0xfa, 0xfa, 0xf8),
    hover: rgb(0xe0, 0xe0, 0xdb),
    line: rgb(0xc4, 0xc5, 0xc0),
    desk: rgb(0x9c, 0x9f, 0xa4),
    text: rgb(0x1b, 0x1c, 0x1f),
    dim: rgb(0x52, 0x56, 0x5c),
    faint: rgb(0x86, 0x8a, 0x90),
    accent: rgb(0xc8, 0x47, 0x1f),
    accent_hover: rgb(0xdc, 0x57, 0x2a),
    accent_pressed: rgb(0xa8, 0x3a, 0x18),
    danger: rgb(0xc0, 0x30, 0x24),
    success: rgb(0x2a, 0x8a, 0x4c),
};

/// Black and white with a yellow accent, for low vision.
pub const HIGH_CONTRAST: Palette = Palette {
    base: rgb(0x00, 0x00, 0x00),
    surface: rgb(0x00, 0x00, 0x00),
    raised: rgb(0x1e, 0x1e, 0x1e),
    hover: rgb(0x3a, 0x3a, 0x3a),
    line: rgb(0xff, 0xff, 0xff),
    desk: rgb(0x40, 0x40, 0x40),
    text: rgb(0xff, 0xff, 0xff),
    dim: rgb(0xff, 0xff, 0xff),
    faint: rgb(0xd0, 0xd0, 0xd0),
    accent: rgb(0xff, 0xc4, 0x00),
    accent_hover: rgb(0xff, 0xd8, 0x4a),
    accent_pressed: rgb(0xe0, 0xa8, 0x00),
    danger: rgb(0xff, 0x60, 0x60),
    success: rgb(0x60, 0xff, 0x90),
};

/// The schemes by name, as Preferences offers them.
pub const THEMES: [(&str, &Palette); 3] = [("Dark", &DARK), ("Light", &LIGHT), ("High contrast", &HIGH_CONTRAST)];

static CURRENT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Switches the scheme (by its name in [`THEMES`]; unknown names: dark).
pub fn set(name: &str) {
    let i = THEMES.iter().position(|t| t.0.eq_ignore_ascii_case(name)).unwrap_or(0);
    CURRENT.store(i, std::sync::atomic::Ordering::Relaxed);
}

pub fn palette() -> &'static Palette {
    THEMES[CURRENT.load(std::sync::atomic::Ordering::Relaxed)].1
}


pub fn theme() -> Theme {
    Theme::custom(
        "Corinzia",
        iced::theme::Palette { background: palette().surface, text: palette().text, primary: palette().accent, success: palette().success, warning: rgb(0xf0, 0xb4, 0x3c), danger: palette().danger },
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
        (_, button::Status::Disabled) => filled(palette().raised, palette().faint),
        (true, button::Status::Hovered) => filled(palette().accent_hover, palette().text),
        (true, button::Status::Pressed) => filled(palette().accent_pressed, palette().text),
        (true, _) => filled(palette().accent, palette().text),
        (false, button::Status::Hovered) => filled(palette().hover, palette().text),
        (false, button::Status::Pressed) => filled(palette().accent_pressed, palette().text),
        (false, _) => filled(palette().raised, palette().text),
    }
}

/// A borderless toolbar button: no background until hovered.
pub fn flat(active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let fg = if matches!(status, button::Status::Disabled) { palette().faint } else if active { palette().accent } else { palette().text };
        let bg = match status {
            button::Status::Hovered => Some(Background::Color(palette().hover)),
            button::Status::Pressed => Some(Background::Color(palette().raised)),
            _ => None,
        };
        button::Style { background: bg, text_color: fg, border: border::rounded(RADIUS), shadow: Shadow::default(), snap: true }
    }
}

/// The primary action (Play, Create): solid accent.
pub fn primary(_: &Theme, status: button::Status) -> button::Style {
    match status {
        button::Status::Hovered => filled(palette().accent_hover, palette().text),
        button::Status::Pressed => filled(palette().accent_pressed, palette().text),
        button::Status::Disabled => filled(palette().raised, palette().faint),
        button::Status::Active => filled(palette().accent, palette().text),
    }
}

/// One option of a segmented control (Score | Mix, Page | Panorama): the
/// chosen one is lit, the others sit flush in the control's trough.
pub fn segment(on: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let bg = match (on, status) {
            (true, _) => Some(Background::Color(palette().hover)),
            (false, button::Status::Hovered) => Some(Background::Color(palette().raised)),
            _ => None,
        };
        button::Style {
            background: bg,
            text_color: if on { palette().text } else { palette().dim },
            border: Border { color: if on { palette().accent } else { Color::TRANSPARENT }, width: if on { 1.0 } else { 0.0 }, radius: 3.0.into() },
            shadow: Shadow::default(),
            snap: true,
        }
    }
}

/// The trough a segmented control sits in.
pub fn trough(_: &Theme) -> container::Style {
    container::Style { background: Some(Background::Color(palette().base)), border: Border { color: palette().line, width: 1.0, radius: RADIUS.into() }, ..container::Style::default() }
}

/// The application bar along the top.
pub fn bar(_: &Theme) -> container::Style {
    container::Style {
        text_color: Some(palette().text),
        background: Some(Background::Color(palette().base)),
        border: Border { color: palette().line, width: 0.0, radius: 0.0.into() },
        ..container::Style::default()
    }
}

/// The status line along the bottom.
pub fn status(_: &Theme) -> container::Style {
    container::Style { text_color: Some(palette().dim), background: Some(Background::Color(palette().base)), ..container::Style::default() }
}

/// A side panel or a mixer area.
pub fn panel(_: &Theme) -> container::Style {
    container::Style { text_color: Some(palette().text), background: Some(Background::Color(palette().surface)), ..container::Style::default() }
}

/// A floating card (popovers, dialogs), lifted off the desk.
pub fn card(_: &Theme) -> container::Style {
    container::Style {
        text_color: Some(palette().text),
        background: Some(Background::Color(palette().surface)),
        border: Border { color: palette().line, width: 1.0, radius: 6.0.into() },
        shadow: Shadow { color: Color::from_rgba(0.0, 0.0, 0.0, 0.45), offset: Vector::new(0.0, 6.0), blur_radius: 18.0 },
        snap: true,
    }
}

/// A mixer strip; `selected` outlines it in the accent.
pub fn strip(selected: bool) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        text_color: Some(palette().text),
        background: Some(Background::Color(palette().raised)),
        border: Border { color: if selected { palette().accent } else { palette().line }, width: 1.0, radius: RADIUS.into() },
        ..container::Style::default()
    }
}

pub fn tooltip(_: &Theme) -> container::Style {
    container::Style {
        text_color: Some(palette().text),
        background: Some(Background::Color(palette().base)),
        border: Border { color: palette().line, width: 1.0, radius: 3.0.into() },
        ..container::Style::default()
    }
}

/// A thin vertical rule between toolbar groups.
pub fn rule(_: &Theme) -> container::Style {
    container::Style { background: Some(Background::Color(palette().line)), ..container::Style::default() }
}

/// A one-pixel hairline below or above a bar.
pub fn hairline(_: &Theme) -> container::Style {
    container::Style { background: Some(Background::Color(palette().line)), ..container::Style::default() }
}

pub fn dim(_: &Theme) -> text::Style {
    text::Style { color: Some(palette().dim) }
}

pub fn faint(_: &Theme) -> text::Style {
    text::Style { color: Some(palette().faint) }
}

pub fn accent(_: &Theme) -> text::Style {
    text::Style { color: Some(palette().accent) }
}

pub fn danger(_: &Theme) -> text::Style {
    text::Style { color: Some(palette().danger) }
}
