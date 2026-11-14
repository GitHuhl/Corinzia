//! Corinzia: a music notation editor and CLAP DAW (iced).
//!
//! `corinzia [file.project] [--view page|panorama] [--keys "n c d"]
//! [--screenshot out.png] [--export out.pdf|mid|musicxml] [--part N]`
//! (in `--keys`, `type:TEXT` and `enter` fill and submit an open popover)
//!
//! The score is engraved by the native engraver (corinzia-layout) and drawn
//! as vector geometry; playback runs through corinzia-audio.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod arrange;
mod browser;
mod cli;
mod demo;
mod dialogs;
mod editor;
mod fader;
mod files;
mod keys;
mod midi_in;
mod mix;
mod panels;
mod playback;
mod prefs;
mod presets;
mod roll;
mod score_view;
mod theme;

use app::*;
use iced::Size;
use cli::*;

static LIVE_AUDIO: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The score's text font (serif) and its bold cut next to it (Foo.ttf /
/// Foo-Bold.ttf), from CORINZIA_TEXT_FONT (set by the dev shell and the
/// package wrapper). The engraver measures text with the fonts that draw
/// it; the returned bytes are for the window.
fn load_text_fonts() -> Vec<Vec<u8>> {
    // Else fonts/DejaVuSerif.ttf next to the executable (the Windows zip).
    let beside = || std::env::current_exe().ok()?.parent().map(|d| d.join("fonts").join("DejaVuSerif.ttf")).filter(|p| p.is_file());
    let Some(path) = std::env::var("CORINZIA_TEXT_FONT").ok().or_else(|| beside().map(|p| p.to_string_lossy().into_owned())) else { return vec![] };
    let bold = std::fs::read(path.replace(".ttf", "-Bold.ttf").replace(".otf", "-Bold.otf")).ok();
    let Ok(regular) = std::fs::read(&path) else { return vec![] };
    corinzia_layout::display::set_text_font(regular.clone(), bold.clone());
    std::iter::once(regular).chain(bold).collect()
}

fn main() -> iced::Result {
    // The plugin scanner's child process: one bundle, its plugins as JSON.
    let raw: Vec<String> = std::env::args().collect();
    if raw.get(1).map(String::as_str) == Some("--scan-plugin") {
        match raw.get(2).map(|b| corinzia_audio::scan::bundle_json(std::path::Path::new(b))) {
            Some(Ok(json)) => {
                println!("{json}");
                std::process::exit(0);
            }
            Some(Err(e)) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
            None => std::process::exit(2),
        }
    }
    let a = args();
    if let Some(out) = a.export.clone() {
        // Headless: open, apply scripted keys, export, exit.
        load_text_fonts();
        let app = App::from_args(a);
        let format = match out.extension().and_then(|e| e.to_str()) {
            Some("pdf") => files::Format::Pdf,
            Some("mid" | "midi") => files::Format::Midi,
            Some("musicxml" | "xml") => files::Format::MusicXml,
            Some("wav") => files::Format::Audio,
            _ => {
                eprintln!("--export: use a .pdf, .mid, .musicxml or .wav file name");
                std::process::exit(2);
            }
        };
        if format == files::Format::Audio {
            let folder = app.path.as_deref().and_then(|p| p.parent());
            let done = corinzia_audio::bounce::Bounce::new(&app.editor.project, app.editor.movement, app.playback.sample_rate, 1024, folder, out.clone()).and_then(|mut b| {
                for w in &b.warnings {
                    eprintln!("corinzia: {w}");
                }
                while !b.step(10.0) {}
                b.finish()
            });
            match done {
                Ok(s) => println!("exported {} ({:.1} s, peak {:.1} dBFS)", out.display(), s.seconds, s.peak_db),
                Err(e) => {
                    eprintln!("export failed: {e}");
                    std::process::exit(1);
                }
            }
            return Ok(());
        }
        match files::export_to(&app.editor.project, app.editor.movement, format, app.editor.transposed, &out) {
            Ok(()) => println!("exported {}", out.display()),
            Err(e) => {
                eprintln!("export failed: {e}");
                std::process::exit(1);
            }
        }
        return Ok(());
    }
    let mut app = iced::application(App::boot, App::update, App::view)
        .title(App::title)
        .subscription(App::subscription)
        .theme(|_: &App| theme::theme())
        .window_size(Size::new(1280.0, 860.0))
        .antialiasing(true)
        .font(corinzia_layout::font::FONT_DATA);
    LIVE_AUDIO.store(true, std::sync::atomic::Ordering::Relaxed);
    for font in load_text_fonts() {
        app = app.font(font);
    }
    // A packaged interface font (fonts/DejaVuSans.ttf next to the
    // executable, as in the Windows zip) replaces the system's sans-serif.
    let ui_font = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join("fonts")));
    if let Some(dir) = ui_font.filter(|d| d.join("DejaVuSans.ttf").is_file()) {
        for name in ["DejaVuSans.ttf", "DejaVuSans-Bold.ttf"] {
            if let Ok(bytes) = std::fs::read(dir.join(name)) {
                app = app.font(bytes);
            }
        }
        app = app.default_font(iced::Font::with_name("DejaVu Sans"));
    }
    app.run()
}
