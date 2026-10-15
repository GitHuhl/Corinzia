//! Corinzia's own file browser, for when the system's file dialog cannot
//! open (on Linux it goes through the xdg-desktop-portal, which some
//! desktops lack): folders and matching files, a path field that takes a
//! typed path, and Open or Export.

use crate::files::Format;
use crate::theme;
use iced::widget::{button, column, container, row, scrollable, text, text_input};
use iced::{Element, Length};
use std::path::{Path, PathBuf};

pub const NAME_INPUT: &str = "browser-name";

/// What the browser is choosing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Purpose {
    Open,
    Export(Format),
}

#[derive(Debug, Clone)]
pub enum BrowserMsg {
    /// Into a sub-folder (or "..").
    Into(String),
    /// A file clicked: picked, or (clicked again) chosen.
    Pick(String),
    Name(String),
    Accept,
    Cancel,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub name: String,
    pub folder: bool,
}

#[derive(Debug, Clone)]
pub struct Browser {
    pub purpose: Purpose,
    pub dir: PathBuf,
    pub entries: Vec<Entry>,
    /// The file name, or a path typed in.
    pub name: String,
    pub error: Option<String>,
}

/// The browser's answer to a message.
#[derive(Debug, PartialEq)]
pub enum Outcome {
    Stay,
    Cancel,
    Chosen(PathBuf),
}

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).or_else(|| std::env::current_dir().ok()).unwrap_or_else(|| PathBuf::from("/"))
}

impl Browser {
    pub fn new(purpose: Purpose, dir: Option<PathBuf>, name: String) -> Browser {
        let dir = dir.filter(|d| d.is_dir()).unwrap_or_else(home);
        let mut b = Browser { purpose, dir, entries: vec![], name, error: None };
        b.refresh();
        b
    }

    fn extensions(&self) -> &'static [&'static str] {
        match self.purpose {
            Purpose::Open => &["project", "musicxml", "xml", "mxl"],
            Purpose::Export(Format::Pdf | Format::Parts) => &["pdf"],
            Purpose::Export(Format::Midi) => &["mid", "midi"],
            Purpose::Export(Format::MusicXml) => &["musicxml", "xml"],
        }
    }

    /// Lists the folder: sub-folders, then the files that fit, by name;
    /// hidden ones left out.
    pub fn refresh(&mut self) {
        let exts = self.extensions();
        let mut entries: Vec<Entry> = match std::fs::read_dir(&self.dir) {
            Ok(rd) => rd
                .filter_map(|e| e.ok())
                .filter_map(|e| {
                    let name = e.file_name().to_string_lossy().into_owned();
                    if name.starts_with('.') {
                        return None;
                    }
                    let folder = e.path().is_dir();
                    let fits = Path::new(&name).extension().and_then(|x| x.to_str()).is_some_and(|x| exts.contains(&x.to_ascii_lowercase().as_str()));
                    (folder || fits).then_some(Entry { name, folder })
                })
                .collect(),
            Err(e) => {
                self.error = Some(format!("{}: {e}", self.dir.display()));
                vec![]
            }
        };
        entries.sort_by(|a, b| b.folder.cmp(&a.folder).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
        self.entries = entries;
    }

    fn go(&mut self, dir: PathBuf) {
        self.dir = dir;
        self.error = None;
        if self.purpose == Purpose::Open {
            self.name.clear();
        }
        self.refresh();
    }

    /// A typed name as a path: absolute, from home (`~/`), or in the folder.
    fn typed(&self) -> PathBuf {
        let t = self.name.trim();
        if let Some(rest) = t.strip_prefix("~/") {
            home().join(rest)
        } else if t == "~" {
            home()
        } else {
            self.dir.join(t)
        }
    }

    pub fn update(&mut self, msg: BrowserMsg) -> Outcome {
        match msg {
            BrowserMsg::Into(name) => {
                let d = if name == ".." { self.dir.parent().map(PathBuf::from).unwrap_or_else(|| self.dir.clone()) } else { self.dir.join(name) };
                self.go(d);
            }
            BrowserMsg::Pick(name) => {
                if self.name == name {
                    return self.update(BrowserMsg::Accept);
                }
                self.name = name;
                self.error = None;
            }
            BrowserMsg::Name(n) => {
                self.name = n;
                self.error = None;
            }
            BrowserMsg::Cancel => return Outcome::Cancel,
            BrowserMsg::Accept => {
                if self.name.trim().is_empty() {
                    self.error = Some("Choose a file".into());
                    return Outcome::Stay;
                }
                let p = self.typed();
                if p.is_dir() {
                    self.go(p);
                    return Outcome::Stay;
                }
                match self.purpose {
                    Purpose::Open if !p.is_file() => self.error = Some(format!("No file {}", p.display())),
                    Purpose::Export(_) if !p.parent().is_some_and(|d| d.is_dir()) => self.error = Some(format!("No folder {}", p.parent().unwrap_or(&p).display())),
                    _ => return Outcome::Chosen(p),
                }
            }
        }
        Outcome::Stay
    }

    pub fn view(&self) -> Element<'_, BrowserMsg> {
        let bold = iced::Font { weight: iced::font::Weight::Bold, ..iced::Font::DEFAULT };
        let (title, verb) = match self.purpose {
            Purpose::Open => ("Open a score".to_string(), "Open"),
            Purpose::Export(f) => (format!("Export {}", f.label()), "Export"),
        };
        let mut list = column![].spacing(1);
        if self.dir.parent().is_some() {
            list = list.push(button(text("⬑  ..").size(13)).width(Length::Fill).padding([3, 8]).style(theme::flat(false)).on_press(BrowserMsg::Into("..".into())));
        }
        for e in &self.entries {
            let (label, msg) = if e.folder { (format!("▸  {}/", e.name), BrowserMsg::Into(e.name.clone())) } else { (format!("    {}", e.name), BrowserMsg::Pick(e.name.clone())) };
            list = list.push(button(text(label).size(13)).width(Length::Fill).padding([3, 8]).style(theme::flat(!e.folder && e.name == self.name)).on_press(msg));
        }
        if self.entries.is_empty() {
            list = list.push(text("No folders or scores here").size(12).style(theme::faint));
        }
        let mut card = column![
            text(title).size(18).font(bold),
            text(self.dir.display().to_string()).size(12).style(theme::dim),
            container(scrollable(list).height(300)).padding(4).style(theme::trough),
            text_input(if self.purpose == Purpose::Open { "a file name, or a path" } else { "file name" }, &self.name)
                .id(NAME_INPUT)
                .on_input(BrowserMsg::Name)
                .on_submit(BrowserMsg::Accept)
                .padding(6)
                .size(14),
        ]
        .spacing(10)
        .width(560);
        if let Some(e) = &self.error {
            card = card.push(text(e).size(12).style(theme::danger));
        }
        card.push(
            row![
                text("The system file dialog could not open; this is Corinzia's.").size(11).style(theme::faint).width(Length::Fill),
                button(text("Cancel").size(13)).padding([6, 14]).style(theme::tool(false)).on_press(BrowserMsg::Cancel),
                button(text(verb).size(13)).padding([6, 14]).style(theme::primary).on_press(BrowserMsg::Accept),
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center),
        )
        .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walks_folders_and_picks_scores() {
        let root = std::env::temp_dir().join(format!("corinzia-browser-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("Songs")).unwrap();
        std::fs::create_dir_all(root.join(".hidden")).unwrap();
        for f in ["b.musicxml", "a.project", "notes.txt", "Songs/c.mxl"] {
            std::fs::write(root.join(f), b"x").unwrap();
        }
        let mut b = Browser::new(Purpose::Open, Some(root.clone()), String::new());
        let names: Vec<_> = b.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["Songs", "a.project", "b.musicxml"], "folders first; other files and hidden ones left out");
        assert_eq!(b.update(BrowserMsg::Accept), Outcome::Stay);
        assert!(b.error.is_some(), "nothing chosen yet");
        b.update(BrowserMsg::Into("Songs".into()));
        assert_eq!(b.entries, [Entry { name: "c.mxl".into(), folder: false }]);
        // A click picks; a second click on it opens.
        assert_eq!(b.update(BrowserMsg::Pick("c.mxl".into())), Outcome::Stay);
        assert_eq!(b.update(BrowserMsg::Pick("c.mxl".into())), Outcome::Chosen(root.join("Songs/c.mxl")));
        b.update(BrowserMsg::Into("..".into()));
        assert_eq!(b.dir, root);
        // A typed path: a folder goes there, a file opens, a missing one says so.
        b.update(BrowserMsg::Name(root.join("Songs").display().to_string()));
        assert_eq!(b.update(BrowserMsg::Accept), Outcome::Stay);
        assert_eq!(b.dir, root.join("Songs"));
        b.update(BrowserMsg::Name("nope.project".into()));
        assert_eq!(b.update(BrowserMsg::Accept), Outcome::Stay);
        assert!(b.error.as_deref().unwrap().starts_with("No file"));
        // Exports take a new name.
        let mut e = Browser::new(Purpose::Export(Format::Pdf), Some(root.clone()), "out.pdf".into());
        assert_eq!(e.update(BrowserMsg::Accept), Outcome::Chosen(root.join("out.pdf")));
        let _ = std::fs::remove_dir_all(&root);
    }
}
