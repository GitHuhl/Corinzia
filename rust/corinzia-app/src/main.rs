//! Corinzia's iced front end.
//!
//! `corinzia-iced [file.project] [--view page|panorama] [--screenshot out.png]`
//!
//! The score is engraved by the native engraver (corinzia-layout) and drawn
//! as vector geometry. This is the start of the Rust application that will
//! replace the Qt one: it reads and writes the same .project files.

mod demo;
mod score_view;

use corinzia_layout::{layout_line, layout_pages, DisplayList, Options, PageSpec};
use corinzia_model::Project;
use iced::widget::{button, canvas, column, container, row, text, Space};
use iced::{Element, Length, Size, Subscription, Task, Theme, Vector};
use score_view::{ScoreView, ViewEvent};
use std::path::PathBuf;
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum View {
    Page,
    Panorama,
}

struct App {
    project: Project,
    path: Option<PathBuf>,
    movement: usize,
    view: View,
    pages: Vec<DisplayList>,
    engrave_ms: f64,
    zoom: f32,
    pan: Vector,
    selection: Option<u64>,
    cache: canvas::Cache,
    status: String,
    screenshot: Option<PathBuf>,
    frames: u32,
}

#[derive(Debug, Clone)]
enum Message {
    SetView(View),
    ZoomBy(f32),
    Canvas(ViewEvent),
    Save,
    Frame,
    Shot(iced::window::Screenshot),
}

struct Args {
    file: Option<PathBuf>,
    view: View,
    screenshot: Option<PathBuf>,
}

fn args() -> Args {
    let mut a = Args { file: None, view: View::Page, screenshot: None };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--view" => a.view = if it.next().as_deref() == Some("panorama") { View::Panorama } else { View::Page },
            "--screenshot" => a.screenshot = it.next().map(PathBuf::from),
            f => a.file = Some(PathBuf::from(f)),
        }
    }
    a
}

impl App {
    fn boot() -> (App, Task<Message>) {
        let a = args();
        let (project, status) = match &a.file {
            Some(f) => match corinzia_model::store::load(f) {
                Ok(p) => (p, format!("Opened {}", f.display())),
                Err(e) => (demo::project(), format!("Could not open {}: {e} — showing the demo", f.display())),
            },
            None => (demo::project(), "Demo project · ctrl+wheel zooms, drag pans, click selects".to_string()),
        };
        let mut app = App::new(project, a.file, a.view, status);
        app.screenshot = a.screenshot;
        (app, Task::none())
    }

    fn new(project: Project, path: Option<PathBuf>, view: View, status: String) -> App {
        let mut app = App {
            project,
            path,
            movement: 0,
            view,
            pages: vec![],
            engrave_ms: 0.0,
            zoom: 1.0,
            pan: Vector::new(24.0, 24.0),
            selection: None,
            cache: canvas::Cache::new(),
            status,
            screenshot: None,
            frames: 0,
        };
        app.engrave();
        app
    }

    fn title(&self) -> String {
        let name = if self.project.title.is_empty() { "Untitled" } else { &self.project.title };
        format!("{name} — Corinzia")
    }

    /// Native engraving of the current movement (milliseconds).
    fn engrave(&mut self) {
        let Some(m) = self.project.movements.get(self.movement) else {
            self.pages.clear();
            return;
        };
        let t = Instant::now();
        let score = corinzia_model::notation::score(&self.project, m);
        let opt = Options::default();
        self.pages = match self.view {
            View::Page => layout_pages(&score, &opt, &PageSpec::default()),
            View::Panorama => {
                let mut l = layout_line(&score, &opt);
                // A panorama strip: the system plus room for marks above and below.
                let pad = 8.0;
                l = DisplayList { width: l.width, height: l.height + 2.0 * pad, items: l.items.into_iter().map(|i| i.translated(0.0, pad)).collect(), staves: l.staves.into_iter().map(|mut s| { s.top += pad; s }).collect(), columns: l.columns };
                vec![l]
            }
        };
        self.engrave_ms = t.elapsed().as_secs_f64() * 1000.0;
        self.cache.clear();
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::SetView(v) => {
                self.view = v;
                self.pan = Vector::new(24.0, 24.0);
                self.engrave();
            }
            Message::ZoomBy(f) => {
                self.zoom = (self.zoom * f).clamp(0.25, 8.0);
                self.cache.clear();
            }
            Message::Canvas(ViewEvent::Pan(d)) => {
                self.pan = self.pan + d;
                self.cache.clear();
            }
            Message::Canvas(ViewEvent::Zoom { factor, around }) => {
                let z = (self.zoom * factor).clamp(0.25, 8.0);
                let k = z / self.zoom;
                // Keep the point under the cursor fixed.
                self.pan = Vector::new(around.x - (around.x - self.pan.x) * k, around.y - (around.y - self.pan.y) * k);
                self.zoom = z;
                self.cache.clear();
            }
            Message::Canvas(ViewEvent::Select(id)) => {
                self.selection = id;
                self.status = match id.and_then(|id| self.describe(id)) {
                    Some(d) => d,
                    None => String::new(),
                };
                self.cache.clear();
            }
            Message::Save => {
                let path = self.path.clone().unwrap_or_else(|| PathBuf::from("untitled.project"));
                self.status = match corinzia_model::store::save(&self.project, &path) {
                    Ok(()) => {
                        self.path = Some(path.clone());
                        format!("Saved {}", path.display())
                    }
                    Err(e) => format!("Save failed: {e}"),
                };
            }
            Message::Frame => {
                self.frames += 1;
                if self.frames == 3 && self.screenshot.is_some() {
                    return iced::window::oldest().and_then(iced::window::screenshot).map(Message::Shot);
                }
            }
            Message::Shot(shot) => {
                if let Some(path) = self.screenshot.take() {
                    match save_png(&path, &shot) {
                        Ok(()) => println!("screenshot {} ({}x{})", path.display(), shot.size.width, shot.size.height),
                        Err(e) => eprintln!("screenshot failed: {e}"),
                    }
                }
                return iced::exit();
            }
        }
        Task::none()
    }

    /// "C5 · Flute" for a notehead, "Rest · Cello" for a rest.
    fn describe(&self, id: u64) -> Option<String> {
        let m = self.project.movements.get(self.movement)?;
        for lane in &m.lanes {
            for v in &lane.voices {
                for e in v.events.iter() {
                    let inst = lane.instrument_at(e.start).and_then(|i| self.project.instrument(i)).map(|i| i.name.clone()).unwrap_or_default();
                    if e.id == id {
                        return Some(format!("Rest · {inst}"));
                    }
                    if let Some(h) = e.heads.iter().find(|h| h.id == id) {
                        const NAMES: [&str; 7] = ["C", "D", "E", "F", "G", "A", "B"];
                        let acc = match h.pitch.alter {
                            -2 => "𝄫",
                            -1 => "♭",
                            1 => "♯",
                            2 => "𝄪",
                            _ => "",
                        };
                        return Some(format!("{}{}{} · {inst}", NAMES[h.pitch.step.clamp(0, 6) as usize], acc, h.pitch.octave));
                    }
                }
            }
        }
        None
    }

    fn view(&self) -> Element<'_, Message> {
        let tab = |label: &'static str, v: View| {
            let b = button(text(label).size(14)).padding([4, 12]).on_press(Message::SetView(v));
            if self.view == v { b.style(button::primary) } else { b.style(button::secondary) }
        };
        let toolbar = row![
            tab("Page", View::Page),
            tab("Panorama", View::Panorama),
            Space::new().width(16),
            button(text("−").size(14)).padding([4, 10]).style(button::secondary).on_press(Message::ZoomBy(1.0 / 1.25)),
            text(format!("{:.0} %", self.zoom * 100.0)).size(14),
            button(text("+").size(14)).padding([4, 10]).style(button::secondary).on_press(Message::ZoomBy(1.25)),
            Space::new().width(Length::Fill),
            button(text("Save").size(14)).padding([4, 12]).style(button::secondary).on_press(Message::Save),
        ]
        .spacing(6)
        .padding(8)
        .align_y(iced::Alignment::Center);
        let score = canvas(ScoreView {
            pages: &self.pages,
            vertical: self.view == View::Page,
            zoom: self.zoom,
            pan: self.pan,
            selection: self.selection,
            cache: &self.cache,
        })
        .width(Length::Fill)
        .height(Length::Fill);
        let status = row![
            text(&self.status).size(13),
            Space::new().width(Length::Fill),
            text(format!("Engraved in {:.1} ms · {} page{}", self.engrave_ms, self.pages.len(), if self.pages.len() == 1 { "" } else { "s" })).size(13),
        ]
        .padding([4, 10]);
        column![toolbar, container(Element::from(score).map(Message::Canvas)).width(Length::Fill).height(Length::Fill), status].into()
    }

    fn subscription(&self) -> Subscription<Message> {
        if self.screenshot.is_some() { iced::window::frames().map(|_| Message::Frame) } else { Subscription::none() }
    }
}

fn save_png(path: &std::path::Path, shot: &iced::window::Screenshot) -> Result<(), Box<dyn std::error::Error>> {
    let file = std::io::BufWriter::new(std::fs::File::create(path)?);
    let mut enc = png::Encoder::new(file, shot.size.width, shot.size.height);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()?.write_image_data(&shot.rgba)?;
    Ok(())
}

fn main() -> iced::Result {
    let mut app = iced::application(App::boot, App::update, App::view)
        .title(App::title)
        .subscription(App::subscription)
        .theme(|_: &App| Theme::Light)
        .window_size(Size::new(1280.0, 860.0))
        .antialiasing(true);
    // A text font for the score (serif); the dev shell and the wrapper point here.
    // A text font for the score (serif) and its bold cut next to it
    // (Foo.ttf / Foo-Bold.ttf); the dev shell and the package wrapper set it.
    if let Ok(path) = std::env::var("CORINZIA_TEXT_FONT") {
        let bold_path = path.replace(".ttf", "-Bold.ttf").replace(".otf", "-Bold.otf");
        let bold = std::fs::read(&bold_path).ok();
        if let Ok(regular) = std::fs::read(&path) {
            // The engraver measures text with the fonts that draw it.
            corinzia_layout::display::set_text_font(regular.clone(), bold.clone());
            app = app.font(regular);
            if let Some(b) = bold {
                app = app.font(b);
            }
        }
    }
    app.run()
}

#[cfg(test)]
mod tests {
    use super::*;
    use corinzia_layout::display::Item;

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

    #[test]
    fn engraves_pages_and_panorama() {
        let a = app(View::Page);
        assert_eq!(a.pages.len(), 1);
        assert!(a.pages[0].height > a.pages[0].width, "A4 portrait");
        let b = app(View::Panorama);
        assert_eq!(b.pages.len(), 1);
        assert!(b.pages[0].width > b.pages[0].height * 4.0, "one long system");
    }

    #[test]
    fn clicking_a_notehead_selects_it() {
        let mut a = app(View::Page);
        let first = a.project.movements[0].lanes[0].voices[0].events[0].heads[0].id; // E5 in the flute
        let at = head_on_screen(&a, first);
        let px = a.zoom * score_view::BASE_PX;
        let desk = iced::Point::new((at.x - a.pan.x) / px, (at.y - a.pan.y) / px);
        let origins = score_view::page_origins(&a.pages, true);
        assert_eq!(score_view::hit_test(&a.pages, &origins, desk), Some(first));
        let _ = a.update(Message::Canvas(ViewEvent::Select(Some(first))));
        assert_eq!(a.status, "E5 · Flute");
        // Empty paper selects nothing.
        assert_eq!(score_view::hit_test(&a.pages, &origins, iced::Point::new(2.0, 2.0)), None);
    }

    #[test]
    fn zoom_keeps_the_point_under_the_cursor() {
        let mut a = app(View::Page);
        let id = a.project.movements[0].lanes[1].voices[0].events[2].heads[0].id;
        let before = head_on_screen(&a, id);
        let _ = a.update(Message::Canvas(ViewEvent::Zoom { factor: 1.6, around: before }));
        let after = head_on_screen(&a, id);
        assert!((after.x - before.x).abs() < 0.5 && (after.y - before.y).abs() < 0.5, "{before:?} -> {after:?}");
        assert!((a.zoom - 1.6).abs() < 1e-6);
    }

    #[test]
    fn saves_a_project_both_apps_can_open() {
        let dir = std::env::temp_dir().join(format!("corinzia-iced-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("demo.project");
        let mut a = app(View::Page);
        a.path = Some(path.clone());
        let _ = a.update(Message::Save);
        assert!(a.status.starts_with("Saved"), "{}", a.status);
        let p = corinzia_model::store::load(&path).unwrap();
        assert_eq!(p.movements[0].lanes.len(), 2);
        assert_eq!(*p.movements[0].lanes[0].voices[0].events, *a.project.movements[0].lanes[0].voices[0].events);
        std::fs::remove_dir_all(&dir).ok();
    }
}
