//! The command line: arguments, --keys parsing and saving screenshots.

use crate::app::View;
use std::path::PathBuf;

pub(crate) struct Args {
    pub(crate) file: Option<PathBuf>,
    pub(crate) view: View,
    pub(crate) screenshot: Option<PathBuf>,
    pub(crate) keys: Vec<String>,
    pub(crate) export: Option<PathBuf>,
    /// Show player N's part (from 1).
    pub(crate) part: Option<usize>,
}

pub(crate) fn args() -> Args {
    let mut a = Args { file: None, view: View::Page, screenshot: None, keys: vec![], export: None, part: None };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--view" => a.view = if it.next().as_deref() == Some("panorama") { View::Panorama } else { View::Page },
            "--screenshot" => a.screenshot = it.next().map(PathBuf::from),
            "--part" => a.part = it.next().and_then(|n| n.parse().ok()),
            // Scripted key presses (testing and demos): "n c d shift+e ctrl+z".
            // Export the (edited) movement and exit: format from the extension.
            "--export" => a.export = it.next().map(PathBuf::from),
            "--keys" => a.keys = it.next().unwrap_or_default().split_whitespace().map(String::from).collect(),
            f => a.file = Some(PathBuf::from(f)),
        }
    }
    a
}

/// "ctrl+shift+z", "alt+up", "esc", "c" -> a key and its modifiers.
pub(crate) fn parse_key_token(t: &str) -> Option<(iced::keyboard::Key, iced::keyboard::Modifiers)> {
    use iced::keyboard::{key::Named, Key, Modifiers};
    let mut mods = Modifiers::empty();
    let mut parts: Vec<&str> = t.split('+').collect();
    let name = if t.ends_with("++") { "+" } else { parts.pop()? };
    if t.ends_with("++") {
        parts.pop();
        parts.pop();
    }
    for m in parts {
        mods |= match m {
            "ctrl" => Modifiers::CTRL,
            "shift" => Modifiers::SHIFT,
            "alt" => Modifiers::ALT,
            _ => return None,
        };
    }
    let key = match name {
        "left" => Key::Named(Named::ArrowLeft),
        "right" => Key::Named(Named::ArrowRight),
        "up" => Key::Named(Named::ArrowUp),
        "down" => Key::Named(Named::ArrowDown),
        "esc" => Key::Named(Named::Escape),
        "bs" => Key::Named(Named::Backspace),
        "del" => Key::Named(Named::Delete),
        "space" => Key::Named(Named::Space),
        "home" => Key::Named(Named::Home),
        "end" => Key::Named(Named::End),
        "f1" => Key::Named(Named::F1),
        c => Key::Character(c.into()),
    };
    Some((key, mods))
}

pub(crate) fn save_png(path: &std::path::Path, shot: &iced::window::Screenshot) -> Result<(), Box<dyn std::error::Error>> {
    let file = std::io::BufWriter::new(std::fs::File::create(path)?);
    let mut enc = png::Encoder::new(file, shot.size.width, shot.size.height);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()?.write_image_data(&shot.rgba)?;
    Ok(())
}
