//! Corinzia: a music notation editor and CLAP DAW (iced).
//!
//! `corinzia [file.project] [--view page|panorama] [--keys "n c d"]
//! [--screenshot out.png] [--export out.pdf|mid|musicxml]`
//!
//! The score is engraved by the native engraver (corinzia-layout) and drawn
//! as vector geometry; playback runs through corinzia-audio.

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod arrange;
mod demo;
mod editor;
mod files;
mod keys;
mod mix;
mod playback;
mod panels;
mod score_view;

use corinzia_layout::{layout_line, layout_pages, DisplayList, Options, PageSpec};
use corinzia_model::{Movement, Project};
use corinzia_score::Frac;
use editor::{Editor, PopoverKind};
use iced::widget::{button, canvas, column, container, row, stack, text, text_input, Space};
use iced::{Element, Length, Size, Subscription, Task, Theme, Vector};
use keys::Action;
use panels::Panel;
use score_view::{Caret, Highlight, ScoreView, StaffPoint, ViewEvent};
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum View {
    Page,
    Panorama,
}

struct Popover {
    kind: PopoverKind,
    input: String,
    error: Option<String>,
}

/// The "New score" card.
#[derive(Default)]
struct NewScore {
    title: String,
    instruments: String,
    bars: String,
    error: Option<String>,
}

#[derive(Debug, Clone, Copy)]
enum NewField {
    Title,
    Instruments,
    Bars,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Score,
    Mix,
}

/// Live audio output (PipeWire) for the windowed app; tests and --export run without.
static LIVE_AUDIO: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

struct App {
    editor: Editor,
    playback: playback::Playback,
    tab: Tab,
    strip: Option<corinzia_model::Id>, // the Mix tab's selected strip
    arrange: arrange::Content,
    arrange_cache: canvas::Cache,
    arrange_ppq: f32,
    arrange_scroll: f32,
    /// Autosave (projects with a file): the revision on disk, and when the
    /// next coalesced save is due.
    saved_revision: u64,
    autosave_at: Option<std::time::Instant>,
    saving: bool,
    undo_tip: String,
    new_score: Option<NewScore>,
    path: Option<PathBuf>,
    view: View,
    pages: Vec<DisplayList>,
    engraved: Option<u64>, // editor revision of `pages`
    engrave_ms: f64,
    zoom: f32,
    pan: Vector,
    selected: HashSet<u64>,
    cache: canvas::Cache,
    panel: Panel,
    popover: Option<Popover>,
    screenshot: Option<PathBuf>,
    frames: u32,
}

#[derive(Debug, Clone)]
enum Message {
    SetView(View),
    SetPanel(Panel),
    Mix(mix::MixMsg),
    Arrange(arrange::ArrangeEvent),
    ZoomBy(f32),
    Canvas(ViewEvent),
    Key(iced::keyboard::Event),
    Act(Action),
    PopoverInput(String),
    PopoverSubmit,
    PopoverClose,
    NewInput(NewField, String),
    NewCreate,
    NewCancel,
    /// Runs the message if there are no unsaved changes, or the user agrees to drop them.
    Guarded(Box<Message>),
    Proceed(Box<Message>),
    Open,
    Opened(Option<PathBuf>),
    SaveTo(Option<PathBuf>),
    Export(files::Format),
    ExportTo(files::Format, Option<PathBuf>),
    Frame,
    Autosaved(u64, Result<(), String>),
    Shot(iced::window::Screenshot),
}

const POPOVER_INPUT: &str = "popover-input";

struct Args {
    file: Option<PathBuf>,
    view: View,
    screenshot: Option<PathBuf>,
    keys: Vec<String>,
    export: Option<PathBuf>,
}

fn args() -> Args {
    let mut a = Args { file: None, view: View::Page, screenshot: None, keys: vec![], export: None };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--view" => a.view = if it.next().as_deref() == Some("panorama") { View::Panorama } else { View::Page },
            "--screenshot" => a.screenshot = it.next().map(PathBuf::from),
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
fn parse_key_token(t: &str) -> Option<(iced::keyboard::Key, iced::keyboard::Modifiers)> {
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
        c => Key::Character(c.into()),
    };
    Some((key, mods))
}

/// The diatonic number (C4 = 28) of a staff's top line for a clef.
fn top_line(clef: &str) -> i32 {
    match clef {
        "bass" => 26,      // A3
        "alto" => 32,      // G4
        "tenor" => 30,     // E4
        "treble_8" => 31,  // F4
        "bass_8" => 19,    // A2
        _ => 38,           // treble, percussion: F5
    }
}

fn clef_at(project: &Project, m: &Movement, lane: usize, at: Frac) -> String {
    let l = &m.lanes[lane];
    if let Some(c) = l.clefs.iter().filter(|c| c.at <= at).last() {
        return c.clef.clone();
    }
    l.instrument_at(at).and_then(|i| project.instrument(i)).map(|i| i.clef.clone()).unwrap_or_else(|| "treble".into())
}

fn quarters(f: Frac) -> f64 {
    f.num as f64 * 4.0 / f.den as f64
}

impl App {
    fn boot() -> (App, Task<Message>) {
        (App::from_args(args()), Task::none())
    }

    /// The app as the command line asks for it (file, view, scripted keys);
    /// no window is needed, so `--export` runs headless.
    fn from_args(a: Args) -> App {
        let (project, status) = match &a.file {
            Some(f) => match corinzia_model::store::load(f) {
                Ok(p) => (p, format!("Opened {}", f.display())),
                Err(e) => (demo::project(), format!("Could not open {}: {e} — showing the demo", f.display())),
            },
            None => (demo::project(), "Demo project · N starts note input · drag selects · right-drag pans · Ctrl+wheel zooms".to_string()),
        };
        let mut app = App::new(project, a.file, a.view, status);
        app.screenshot = a.screenshot;
        for t in &a.keys {
            match parse_key_token(t) {
                Some((key, mods)) => app.press(&key, mods).into_iter().for_each(drop),
                None => eprintln!("--keys: cannot parse {t:?}"),
            }
        }
        app.sync();
        app
    }

    fn new(project: Project, path: Option<PathBuf>, view: View, status: String) -> App {
        let live = LIVE_AUDIO.load(std::sync::atomic::Ordering::Relaxed);
        Self::with_playback(project, path, view, status, playback::Playback::new(live))
    }

    /// A project in the window, keeping the audio output and plugins.
    fn with_playback(project: Project, path: Option<PathBuf>, view: View, status: String, mut playback: playback::Playback) -> App {
        playback.stop();
        let mut editor = Editor::new(project);
        editor.status = status; // an audio problem shows on the right of the status bar
        let mut app = App {
            editor,
            playback,
            tab: Tab::Score,
            strip: None,
            arrange: arrange::Content::default(),
            arrange_cache: canvas::Cache::new(),
            arrange_ppq: 40.0,
            arrange_scroll: 0.0,
            saved_revision: 0,
            autosave_at: None,
            saving: false,
            undo_tip: String::new(),
            new_score: None,
            path,
            view,
            pages: vec![],
            engraved: None,
            engrave_ms: 0.0,
            zoom: 0.9, // an A4 page fits the default window
            pan: Vector::new(24.0, 24.0),
            selected: HashSet::new(),
            cache: canvas::Cache::new(),
            panel: Panel::Notes,
            popover: None,
            screenshot: None,
            frames: 0,
        };
        app.sync();
        app
    }

    fn title(&self) -> String {
        let p = &self.editor.project;
        let name = if p.title.is_empty() { "Untitled" } else { &p.title };
        format!("{}{name} — Corinzia", if self.editor.modified { "• " } else { "" })
    }

    /// Native engraving of the current movement (milliseconds).
    fn engrave(&mut self) {
        self.engraved = Some(self.editor.music_revision);
        let Some(m) = self.editor.movement() else {
            self.pages.clear();
            return;
        };
        let t = Instant::now();
        let score = corinzia_model::notation::score(&self.editor.project, m);
        let opt = Options::default();
        self.pages = match self.view {
            View::Page => layout_pages(&score, &opt, &PageSpec::default()),
            View::Panorama => {
                let mut l = layout_line(&score, &opt);
                // A panorama strip: the system plus room for marks above and below.
                let pad = 8.0;
                l = DisplayList {
                    width: l.width,
                    height: l.height + 2.0 * pad,
                    items: l.items.into_iter().map(|i| i.translated(0.0, pad)).collect(),
                    staves: l
                        .staves
                        .into_iter()
                        .map(|mut s| {
                            s.top += pad;
                            s
                        })
                        .collect(),
                    columns: l.columns,
                };
                vec![l]
            }
        };
        self.engrave_ms = t.elapsed().as_secs_f64() * 1000.0;
        self.arrange = arrange::content(&self.editor.project, self.editor.movement);
    }

    /// Brings the engraving and the selection colours up to date.
    fn sync(&mut self) {
        if self.engraved != Some(self.editor.music_revision) {
            self.engrave();
        }
        self.playback.sync(&self.editor.project, self.editor.music_revision, self.editor.revision);
        if self.path.is_some() && self.editor.revision != self.saved_revision && self.autosave_at.is_none() && !self.saving {
            self.autosave_at = Some(std::time::Instant::now() + std::time::Duration::from_millis(800));
        }
        let mut sel = HashSet::new();
        if let Some(id) = self.editor.selection {
            sel.insert(id);
        }
        if let (Some(r), Some(m)) = (self.editor.range, self.editor.movement()) {
            for l in m.lanes.iter().skip(r.lane_from).take(r.lane_to + 1 - r.lane_from) {
                for v in &l.voices {
                    for e in v.events.iter().filter(|e| e.start >= r.from && e.start < r.to) {
                        sel.insert(e.id);
                        sel.extend(e.heads.iter().map(|h| h.id));
                    }
                }
            }
        }
        self.selected = sel;
        self.undo_tip = self.editor.undo_label().map(|l| format!("Undo {l} (Ctrl+Z)")).unwrap_or_else(|| "Nothing to undo".into());
        self.cache.clear();
        self.arrange_cache.clear();
    }

    fn open_popover(&mut self, kind: PopoverKind) -> Task<Message> {
        self.popover = Some(Popover { kind, input: String::new(), error: None });
        iced::widget::operation::focus(POPOVER_INPUT)
    }

    /// A key press from the score keyboard.
    fn press(&mut self, key: &iced::keyboard::Key, mods: iced::keyboard::Modifiers) -> Option<Task<Message>> {
        let action = keys::map(key, mods, self.editor.input_mode)?;
        Some(self.act(action))
    }

    /// Plays the selected note (just entered or clicked) on its instrument.
    fn audition(&mut self) {
        let ed = &self.editor;
        let (Some(id), Some(m)) = (ed.selection, ed.movement()) else { return };
        let Some(loc) = ed.locate(id) else { return };
        let lane = &m.lanes[loc.lane];
        let Some((e, head)) = corinzia_edit::find_event(&lane.voices[loc.voice].events, id) else { return };
        let keys: Vec<i32> = match head {
            Some(h) => vec![e.heads[h].pitch.midi_key()],
            None => e.heads.iter().map(|h| h.pitch.midi_key()).collect(),
        };
        let Some(track) = lane.instrument_at(e.start).and_then(|i| ed.project.instrument(i)).and_then(|i| i.track) else { return };
        let channel = ed.project.track(track).map(|t| t.midi_channel as i16).unwrap_or(0);
        self.sync(); // the plan knows a new track before we preview on it
        self.playback.preview(track, channel, &keys);
    }

    fn play_stop(&mut self) {
        if self.playback.is_playing() {
            self.playback.stop();
            return;
        }
        // From the selection, else the caret.
        let ed = &self.editor;
        let (Some(m), Some((_, at))) = (ed.movement(), ed.structure_target()) else { return };
        self.playback.play_from(m.id, quarters(at));
    }

    fn act(&mut self, a: Action) -> Task<Message> {
        let audition = matches!(a, Action::Note { .. } | Action::Transpose(_));
        let task = self.act_inner(a);
        if audition && self.playback.problem.is_none() && !self.playback.is_playing() {
            self.audition();
        }
        task
    }

    fn act_inner(&mut self, a: Action) -> Task<Message> {
        let ed = &mut self.editor;
        ed.status.clear(); // the action reports its own outcome
        match a {
            Action::PlayStop => self.play_stop(),
            Action::Lyrics(v) => {
                ed.lyric_verse = v;
                let t = self.open_popover(PopoverKind::Lyrics);
                self.sync();
                return t;
            }
            Action::ShowScore => self.tab = Tab::Score,
            Action::ShowMix => {
                self.tab = Tab::Mix;
                ed.set_input_mode(false); // the keyboard follows the visible tab
                if self.playback.plugins.is_empty() {
                    self.playback.scan();
                }
            }
            Action::ToggleInput => ed.set_input_mode(!ed.input_mode),
            Action::SetInput(on) => ed.set_input_mode(on),
            Action::Escape => {
                if self.popover.take().is_some() {
                } else if ed.input_mode {
                    ed.set_input_mode(false)
                } else {
                    ed.clear_selection()
                }
            }
            Action::Note { step, chord } => ed.enter_note(step, chord),
            Action::Rest => ed.enter_rest(),
            Action::Duration(d) => ed.spec.base = corinzia_edit::BaseDuration::ALL[d.min(6) as usize],
            Action::Dots => ed.cycle_dots(),
            Action::Triplet => ed.spec.triplet = !ed.spec.triplet,
            Action::Accidental(x) => ed.toggle_accidental(x),
            Action::ChordMode => ed.chord_mode = !ed.chord_mode,
            Action::Tie => ed.toggle_tie(),
            Action::Voice => ed.cycle_voice(),
            Action::SetVoice(v) => ed.set_voice(v),
            Action::Articulation(f) => ed.toggle_articulation(f),
            Action::Dynamic(d) => ed.set_dynamic(d),
            Action::Slur => ed.add_slur(),
            Action::Hairpin { crescendo } => ed.add_hairpin(crescendo),
            Action::Repeat => ed.repeat_selection(),
            Action::CaretBy(d) => ed.move_caret(d),
            Action::CaretStaff(d) => ed.move_caret_staff(d),
            Action::Transpose(s) => ed.transpose_selection(s),
            Action::Backspace => ed.backspace(),
            Action::Delete => ed.delete_selection(),
            Action::Copy => ed.copy(),
            Action::Cut => ed.cut(),
            Action::Paste => ed.paste(),
            Action::SelectAll => ed.select_all(),
            Action::InsertBar => ed.insert_bars(1),
            Action::DeleteBars => ed.delete_bars(),
            Action::FirstSecondEndings => ed.add_first_second_endings(),
            Action::Undo => ed.undo(),
            Action::Redo => ed.redo(),
            Action::Save if self.path.is_some() => self.save(self.path.clone().unwrap()),
            Action::Save | Action::SaveAs => return self.ask_save_path(),
            Action::Open => return Task::done(Message::Guarded(Box::new(Message::Open))),
            Action::New => {
                self.new_score = Some(NewScore { bars: "32".into(), ..Default::default() });
            }
            Action::Popover(kind) => {
                let t = self.open_popover(kind);
                self.sync();
                return t;
            }
            Action::Rehearsal => {
                let _ = ed.apply_popover(PopoverKind::Rehearsal, "");
            }
            Action::ZoomIn => self.zoom = (self.zoom * 1.25).min(8.0),
            Action::ZoomOut => self.zoom = (self.zoom / 1.25).max(0.25),
        }
        self.sync();
        Task::none()
    }

    fn ask_save_path(&self) -> Task<Message> {
        let name = files::sibling(self.path.as_deref(), &self.editor.project.title, "project");
        let dir = self.path.as_ref().and_then(|p| p.parent().map(PathBuf::from));
        Task::perform(
            async move {
                let mut d = rfd::AsyncFileDialog::new().set_title("Save project").add_filter("Corinzia project", &["project"]).set_file_name(name.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default());
                if let Some(dir) = dir {
                    d = d.set_directory(dir);
                }
                d.save_file().await.map(|h| h.path().to_path_buf())
            },
            Message::SaveTo,
        )
    }

    fn open(&mut self, path: PathBuf) {
        match corinzia_model::store::load(&path) {
            Ok(p) => {
                let (view, pb) = (self.view, std::mem::replace(&mut self.playback, playback::Playback::new(false)));
                *self = App::with_playback(p, Some(path.clone()), view, format!("Opened {}", path.display()), pb);
            }
            Err(e) => self.editor.status = format!("Could not open {}: {e}", path.display()),
        }
    }

    fn create_new(&mut self) {
        let Some(n) = self.new_score.as_mut() else { return };
        let bars = n.bars.trim().parse::<i64>().unwrap_or(32).clamp(1, 2000);
        let title = if n.title.trim().is_empty() { "Untitled" } else { n.title.trim() };
        match files::new_project(title, &n.instruments, bars) {
            Ok(p) => {
                let (view, pb) = (self.view, std::mem::replace(&mut self.playback, playback::Playback::new(false)));
                *self = App::with_playback(p, None, view, "New score · N starts note input".into(), pb);
                self.editor.set_input_mode(true);
            }
            Err(e) => n.error = Some(e),
        }
    }

    /// Starts a background save when one is due.
    fn autosave(&mut self) -> Option<Task<Message>> {
        let due = self.autosave_at.is_some_and(|t| std::time::Instant::now() >= t);
        let path = self.path.clone().filter(|_| due && !self.saving)?;
        self.autosave_at = None;
        self.saving = true;
        self.playback.pool.store_states(&mut self.editor.project);
        let (project, rev) = (self.editor.project.clone(), self.editor.revision);
        Some(Task::perform(async move { corinzia_model::store::save(&project, &path).map_err(|e| e.to_string()) }, move |r| Message::Autosaved(rev, r)))
    }

    fn save(&mut self, path: PathBuf) {
        // Plugin state chunks go into the file with the music.
        self.playback.pool.store_states(&mut self.editor.project);
        self.editor.status = match corinzia_model::store::save(&self.editor.project, &path) {
            Ok(()) => {
                self.path = Some(path.clone());
                self.saved_revision = self.editor.revision;
                self.autosave_at = None;
                self.editor.saved();
                format!("Saved {}", path.display())
            }
            Err(e) => format!("Save failed: {e}"),
        };
    }

    fn mix(&mut self, m: mix::MixMsg) {
        use mix::MixMsg::*;
        match m {
            Select(t) => self.strip = Some(t),
            Gain(t, db) => {
                self.editor.set_track(t, "Gain", true, |x| x.gain_db = (db * 2.0).round() / 2.0);
                self.playback.live_mix(&self.editor.project, t);
            }
            Pan(t, pan) => {
                self.editor.set_track(t, "Pan", true, |x| x.pan = if pan.abs() < 0.03 { 0.0 } else { pan });
                self.playback.live_mix(&self.editor.project, t);
            }
            Released => {}
            Mute(t) => self.editor.set_track(t, "Mute", false, |x| x.mute = !x.mute),
            Solo(t) => self.editor.set_track(t, "Solo", false, |x| x.solo = !x.solo),
            Assign(t, i) => {
                let Some(info) = self.playback.plugins.get(i).cloned() else { return };
                if info.bundle.as_os_str() == corinzia_audio::host::BUILTIN {
                    self.editor.set_track_plugin(t, None);
                } else {
                    let bundle = info.bundle.to_string_lossy().into_owned();
                    self.editor.set_track_plugin(t, Some((&info.name, &bundle, &info.id)));
                }
            }
            Rescan => self.playback.scan(),
        }
    }

    /// A click on the score: select, or (input mode, empty paper) enter a note.
    fn click(&mut self, hit: Option<u64>, at: Option<StaffPoint>, shift: bool) {
        let ed = &mut self.editor;
        if ed.input_mode && (hit.is_none() || shift) {
            if let (Some(p), Some(m)) = (at, ed.movement()) {
                if p.part < m.lanes.len() {
                    let pos = Frac::new((p.quarter * 1024.0).round() as i64, 4096);
                    let diatonic = top_line(&clef_at(&ed.project, m, p.part, pos)) - p.step_below_top;
                    ed.place_note_at(p.part, p.quarter, diatonic, shift);
                    self.audition();
                    return;
                }
            }
        }
        match (hit, shift) {
            (Some(id), true) => ed.select_elements(&[id], true),
            (Some(id), false) => ed.select(Some(id)),
            (None, _) => {
                if let (Some(p), true) = (at, ed.input_mode) {
                    ed.caret_lane = p.part;
                }
                ed.clear_selection()
            }
        }
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::SetView(v) => {
                self.view = v;
                self.pan = Vector::new(24.0, 24.0);
                self.engraved = None;
            }
            Message::SetPanel(p) => self.panel = p,
            Message::Mix(m) => self.mix(m),
            Message::Arrange(e) => match e {
                arrange::ArrangeEvent::Select(id) => {
                    self.editor.select(id);
                    if id.is_some() {
                        self.audition();
                    }
                }
                arrange::ArrangeEvent::Locate(q) => {
                    let at = Frac::new((q * 1024.0).round() as i64, 4096);
                    self.editor.clear_selection();
                    self.editor.caret = at.min(self.editor.movement().map(|m| m.length).unwrap_or(at));
                    if self.playback.is_playing() {
                        if let Some(m) = self.editor.movement().map(|m| m.id) {
                            self.playback.play_from(m, q);
                        }
                    }
                }
                arrange::ArrangeEvent::Scroll(d) => self.arrange_scroll = arrange::clamp_scroll(self.arrange_scroll + d, &self.arrange, self.arrange_ppq),
                arrange::ArrangeEvent::Zoom(k) => self.arrange_ppq = (self.arrange_ppq * k).clamp(6.0, 200.0),
            },
            Message::ZoomBy(f) => self.zoom = (self.zoom * f).clamp(0.25, 8.0),
            Message::Canvas(ViewEvent::Pan(d)) => self.pan = self.pan + d,
            Message::Canvas(ViewEvent::Zoom { factor, around }) => {
                let z = (self.zoom * factor).clamp(0.25, 8.0);
                let k = z / self.zoom;
                // Keep the point under the cursor fixed.
                self.pan = Vector::new(around.x - (around.x - self.pan.x) * k, around.y - (around.y - self.pan.y) * k);
                self.zoom = z;
            }
            Message::Canvas(ViewEvent::Click { hit, at, shift }) => {
                self.editor.status.clear();
                self.click(hit, at, shift)
            }
            Message::Canvas(ViewEvent::Marquee { ids, shift }) => self.editor.select_elements(&ids, shift),
            Message::Key(iced::keyboard::Event::KeyPressed { key, modifiers, .. }) => {
                if self.popover.is_some() || self.new_score.is_some() {
                    if matches!(key, iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape)) {
                        self.popover = None;
                        self.new_score = None;
                    }
                } else if let Some(t) = self.press(&key, modifiers) {
                    return t;
                }
            }
            Message::Key(_) => return Task::none(),
            Message::Act(a) => return self.act(a),
            Message::PopoverInput(s) => {
                let Some(p) = self.popover.as_mut() else { return Task::none() };
                p.error = None;
                // Lyrics and chord symbols are entered as a stream: space (or,
                // for lyrics, a hyphen or an extender) commits and moves on.
                let last = s.chars().last();
                let commits = match p.kind {
                    PopoverKind::Lyrics => matches!(last, Some(' ' | '-' | '_')),
                    PopoverKind::Chord => last == Some(' '),
                    _ => false,
                };
                if !commits {
                    p.input = s;
                    return Task::none();
                }
                let word = s[..s.len() - 1].trim().to_string();
                let kind = p.kind;
                let result = match kind {
                    PopoverKind::Lyrics => self.editor.enter_lyric(self.editor.lyric_verse, &word, last == Some('-'), last == Some('_')).map(|_| ()),
                    _ if word.is_empty() => {
                        self.editor.advance_target();
                        Ok(())
                    }
                    _ => self.editor.apply_popover(kind, &word).map(|_| self.editor.advance_target()),
                };
                let p = self.popover.as_mut().unwrap();
                match result {
                    Ok(()) => p.input.clear(),
                    Err(e) => {
                        p.input = word;
                        p.error = Some(e);
                    }
                }
                self.sync();
                if self.playback.problem.is_none() && !self.playback.is_playing() {
                    self.audition();
                }
                return Task::none();
            }
            Message::PopoverSubmit => {
                if let Some(p) = self.popover.as_mut() {
                    match self.editor.apply_popover(p.kind, &p.input) {
                        Ok(()) => self.popover = None,
                        Err(e) => p.error = Some(e),
                    }
                }
            }
            Message::PopoverClose => self.popover = None,
            Message::NewInput(field, v) => {
                if let Some(n) = self.new_score.as_mut() {
                    match field {
                        NewField::Title => n.title = v,
                        NewField::Instruments => n.instruments = v,
                        NewField::Bars => n.bars = v,
                    }
                    n.error = None;
                }
                return Task::none();
            }
            Message::NewCancel => self.new_score = None,
            Message::NewCreate => return Task::done(Message::Guarded(Box::new(Message::Proceed(Box::new(Message::NewCreate))))),
            Message::Guarded(next) => {
                if !self.editor.modified {
                    return Task::done(Message::Proceed(next));
                }
                return Task::perform(
                    async {
                        rfd::AsyncMessageDialog::new()
                            .set_title("Unsaved changes")
                            .set_description("The score has unsaved changes. Discard them?")
                            .set_buttons(rfd::MessageButtons::OkCancelCustom("Discard".into(), "Cancel".into()))
                            .show()
                            .await
                    },
                    move |r| match r {
                        rfd::MessageDialogResult::Custom(c) if c == "Discard" => Message::Proceed(next.clone()),
                        rfd::MessageDialogResult::Ok | rfd::MessageDialogResult::Yes => Message::Proceed(next.clone()),
                        _ => Message::PopoverClose,
                    },
                );
            }
            Message::Proceed(next) => match *next {
                Message::Open => {
                    let dir = self.path.as_ref().and_then(|p| p.parent().map(PathBuf::from));
                    return Task::perform(
                        async move {
                            let mut d = rfd::AsyncFileDialog::new().set_title("Open project").add_filter("Corinzia project", &["project"]);
                            if let Some(dir) = dir {
                                d = d.set_directory(dir);
                            }
                            d.pick_file().await.map(|h| h.path().to_path_buf())
                        },
                        Message::Opened,
                    );
                }
                Message::NewCreate => self.create_new(),
                other => return Task::done(other),
            },
            Message::Open => return Task::done(Message::Guarded(Box::new(Message::Open))),
            Message::Opened(None) | Message::SaveTo(None) | Message::ExportTo(_, None) => return Task::none(),
            Message::Opened(Some(path)) => self.open(path),
            Message::SaveTo(Some(path)) => {
                let path = if path.extension().is_none() { path.with_extension("project") } else { path };
                self.save(path);
            }
            Message::Export(format) => {
                let name = files::sibling(self.path.as_deref(), &self.editor.project.title, format.extension());
                let dir = self.path.as_ref().and_then(|p| p.parent().map(PathBuf::from));
                return Task::perform(
                    async move {
                        let mut d = rfd::AsyncFileDialog::new()
                            .set_title(format!("Export {}", format.label()))
                            .add_filter(format.label(), &[format.extension()])
                            .set_file_name(name.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default());
                        if let Some(dir) = dir {
                            d = d.set_directory(dir);
                        }
                        d.save_file().await.map(|h| h.path().to_path_buf())
                    },
                    move |p| Message::ExportTo(format, p),
                );
            }
            Message::ExportTo(format, Some(path)) => {
                let path = if path.extension().is_none() { path.with_extension(format.extension()) } else { path };
                self.editor.status = match files::export_to(&self.editor.project, self.editor.movement, format, &path) {
                    Ok(()) => format!("Exported {} to {}", format.label(), path.display()),
                    Err(e) => format!("Export failed: {e}"),
                };
            }
            Message::Frame => {
                self.playback.tick();
                if let Some(t) = self.autosave() {
                    return t;
                }
                self.frames += 1;
                if self.frames == 3 && self.screenshot.is_some() {
                    return iced::window::oldest().and_then(iced::window::screenshot).map(Message::Shot);
                }
                return Task::none();
            }
            Message::Autosaved(rev, result) => {
                self.saving = false;
                match result {
                    Ok(()) => {
                        self.saved_revision = rev;
                        if self.editor.revision == rev {
                            self.editor.saved();
                        }
                    }
                    Err(e) => self.editor.status = format!("Autosave failed: {e}"),
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
        self.sync();
        Task::none()
    }

    fn view(&self) -> Element<'_, Message> {
        let ed = &self.editor;
        let seg = |label: &'static str, on: bool, m: Message| {
            let b = button(text(label).size(14)).padding([4, 12]).on_press(m);
            if on { b.style(button::primary) } else { b.style(button::secondary) }
        };
        let act = |label: &'static str, enabled: bool, a: Action| {
            button(text(label).size(14)).padding([4, 10]).style(button::secondary).on_press_maybe(enabled.then_some(Message::Act(a)))
        };
        let small = |label: &'static str, m: Message| button(text(label).size(13)).padding([4, 8]).style(button::secondary).on_press(m);
        let mut toolbar = row![
            seg("Score", self.tab == Tab::Score, Message::Act(Action::ShowScore)),
            seg("Mix", self.tab == Tab::Mix, Message::Act(Action::ShowMix)),
            Space::new().width(8),
            button(text(if self.playback.is_playing() { "■ Stop" } else { "▶ Play" }).size(14)).padding([4, 12]).style(button::success).on_press(Message::Act(Action::PlayStop)),
            Space::new().width(8),
            act("New", true, Action::New),
            act("Open", true, Action::Open),
            act("Save", true, Action::Save),
            Space::new().width(8),
            text("Export").size(13).style(text::secondary),
            small("PDF", Message::Export(files::Format::Pdf)),
            small("MIDI", Message::Export(files::Format::Midi)),
            small("MusicXML", Message::Export(files::Format::MusicXml)),
            Space::new().width(8),
            panels::tip(act("↶", ed.can_undo(), Action::Undo), self.undo_tip.as_str()),
            panels::tip(act("↷", ed.can_redo(), Action::Redo), "Redo (Ctrl+Shift+Z)"),
        ]
        .spacing(6)
        .padding(8)
        .align_y(iced::Alignment::Center);
        if self.tab == Tab::Score {
            toolbar = toolbar
                .push(Space::new().width(8))
                .push(seg(if ed.input_mode { "✎ Input on" } else { "✎ Input" }, ed.input_mode, Message::Act(Action::SetInput(!ed.input_mode))))
                .push(Space::new().width(8))
                .push(seg("Page", self.view == View::Page, Message::SetView(View::Page)))
                .push(seg("Panorama", self.view == View::Panorama, Message::SetView(View::Panorama)))
                .push(Space::new().width(8))
                .push(small("−", Message::ZoomBy(1.0 / 1.25)))
                .push(text(format!("{:.0} %", self.zoom * 100.0)).size(13))
                .push(small("+", Message::ZoomBy(1.25)));
        }
        let toolbar = toolbar.push(panels::spacer()).push(text(ed.movement().map(|m| m.title.clone()).unwrap_or_default()).size(14));

        let caret = ed.input_mode.then(|| Caret { part: ed.caret_lane, quarter: quarters(ed.caret) });
        let highlight = ed.range.map(|r| Highlight { part_from: r.lane_from, part_to: r.lane_to, start: quarters(r.from), end: quarters(r.to) });
        let score = canvas(ScoreView {
            pages: &self.pages,
            vertical: self.view == View::Page,
            zoom: self.zoom,
            pan: self.pan,
            selected: &self.selected,
            caret,
            highlight,
            playhead: self.playback.playhead().filter(|(m, _)| Some(*m) == ed.movement().map(|m| m.id)).map(|(_, q)| q),
            cache: &self.cache,
        })
        .width(Length::Fill)
        .height(Length::Fill);
        let mut desk = stack![container(Element::from(score).map(Message::Canvas)).width(Length::Fill).height(Length::Fill)];
        if let Some(p) = &self.popover {
            let mut card = column![
                row![
                    text(match p.kind {
                        PopoverKind::Lyrics => format!("Lyrics · verse {} · {}", ed.lyric_verse, ed.popover_position()),
                        _ => format!("{} · {}", p.kind.title(), ed.popover_position()),
                    })
                    .size(13),
                    panels::spacer(), button(text("✕").size(12)).padding([1, 6]).style(button::text).on_press(Message::PopoverClose)]
                    .align_y(iced::Alignment::Center),
                text_input(p.kind.placeholder(), &p.input).id(POPOVER_INPUT).on_input(Message::PopoverInput).on_submit(Message::PopoverSubmit).padding(6).size(15),
            ]
            .spacing(6)
            .width(320);
            if let Some(e) = &p.error {
                card = card.push(text(e).size(12).style(text::danger));
            }
            desk = desk.push(container(iced::widget::opaque(container(card).padding(10).style(container::bordered_box))).padding(16).width(Length::Fill).align_x(iced::Alignment::Center));
        }

        if let Some(n) = &self.new_score {
            let field = |label: &'static str, hint: &'static str, value: &str, f: NewField| {
                column![text(label).size(12).style(text::secondary), text_input(hint, value).on_input(move |v| Message::NewInput(f, v)).on_submit(Message::NewCreate).padding(6).size(14)].spacing(3)
            };
            let mut card = column![
                text("New score").size(18),
                field("Title", "Untitled", &n.title, NewField::Title),
                field("Instruments (comma separated; \"2 Violin\" for two)", "Flute, Clarinet, Violin, Cello", &n.instruments, NewField::Instruments),
                field("Bars", "32", &n.bars, NewField::Bars),
            ]
            .spacing(10)
            .width(420);
            if let Some(e) = &n.error {
                card = card.push(text(e).size(12).style(text::danger));
            }
            card = card.push(
                row![panels::spacer(), button(text("Cancel").size(14)).style(button::secondary).on_press(Message::NewCancel), button(text("Create").size(14)).style(button::primary).on_press(Message::NewCreate)].spacing(8),
            );
            desk = desk.push(container(iced::widget::opaque(container(card).padding(16).style(container::bordered_box))).center(Length::Fill));
        }

        let tabs = row![seg("Notes", self.panel == Panel::Notes, Message::SetPanel(Panel::Notes)), seg("Structure", self.panel == Panel::Structure, Message::SetPanel(Panel::Structure))].spacing(4).padding([0, 8]);
        let side = column![tabs, panels::panel(ed, self.panel).map(Message::Act)].spacing(6).width(panels::WIDTH);

        let left = if !ed.status.is_empty() { ed.status.clone() } else { ed.selection_label() };
        let middle = if ed.input_mode { ed.caret_label() } else if ed.status.is_empty() { String::new() } else { ed.selection_label() };
        let status = row![
            text(left).size(13),
            Space::new().width(24),
            text(middle).size(13).style(text::secondary),
            panels::spacer(),
            text(self.playback.problem.clone().unwrap_or_default()).size(13).style(text::danger),
            Space::new().width(16),
            text(format!("Engraved in {:.1} ms · {} page{}", self.engrave_ms, self.pages.len(), if self.pages.len() == 1 { "" } else { "s" })).size(13),
        ]
        .padding([4, 10]);
        let body: Element<'_, Message> = match self.tab {
            Tab::Score => row![side, desk].height(Length::Fill).into(),
            Tab::Mix => {
                let playhead = self.playback.playhead().filter(|(m, _)| Some(*m) == ed.movement().map(|m| m.id)).map(|(_, q)| q);
                let arr = canvas(arrange::ArrangeView {
                    content: &self.arrange,
                    pixels_per_quarter: self.arrange_ppq,
                    scroll_x: self.arrange_scroll,
                    lane_height: 56.0,
                    selected: &self.selected,
                    playhead,
                    cache: &self.arrange_cache,
                })
                .width(Length::Fill)
                .height(Length::Fill);
                column![
                    Element::from(arr).map(Message::Arrange),
                    container(mix::view(&ed.project, &self.playback, self.strip).map(Message::Mix)).height(360),
                ]
                .height(Length::Fill)
                .into()
            }
        };
        column![toolbar, body, status].into()
    }

    fn subscription(&self) -> Subscription<Message> {
        let keys = iced::keyboard::listen().map(Message::Key);
        if self.screenshot.is_some() || self.playback.busy() || self.autosave_at.is_some() {
            Subscription::batch([keys, iced::window::frames().map(|_| Message::Frame)])
        } else {
            keys
        }
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
    let a = args();
    if let Some(out) = a.export.clone() {
        // Headless: open, apply scripted keys, export, exit.
        load_text_fonts();
        let app = App::from_args(a);
        let format = match out.extension().and_then(|e| e.to_str()) {
            Some("pdf") => files::Format::Pdf,
            Some("mid" | "midi") => files::Format::Midi,
            Some("musicxml" | "xml") => files::Format::MusicXml,
            _ => {
                eprintln!("--export: use a .pdf, .mid or .musicxml file name");
                std::process::exit(2);
            }
        };
        match files::export_to(&app.editor.project, app.editor.movement, format, &out) {
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
        .theme(|_: &App| Theme::Light)
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
        assert!(b.pages[0].width > b.pages[0].height * 3.0, "one long system");
    }

    #[test]
    fn clicking_a_notehead_selects_it() {
        let mut a = app(View::Page);
        let first = a.editor.project.movements[0].lanes[0].voices[0].events[0].heads[0].id; // E5 in the flute
        let at = head_on_screen(&a, first);
        let px = a.zoom * score_view::BASE_PX;
        let desk = iced::Point::new((at.x - a.pan.x) / px, (at.y - a.pan.y) / px);
        let origins = score_view::page_origins(&a.pages, true);
        assert_eq!(score_view::hit_test(&a.pages, &origins, desk), Some(first));
        let _ = a.update(Message::Canvas(ViewEvent::Click { hit: Some(first), at: None, shift: false }));
        a.editor.status.clear();
        assert_eq!(a.editor.selection_label(), "E5 · Flute · bar 1");
        assert!(a.selected.contains(&first));
        // Empty paper selects nothing.
        assert_eq!(score_view::hit_test(&a.pages, &origins, iced::Point::new(2.0, 2.0)), None);
    }

    #[test]
    fn zoom_keeps_the_point_under_the_cursor() {
        let mut a = app(View::Page);
        let id = a.editor.project.movements[0].lanes[1].voices[0].events[2].heads[0].id;
        let before = head_on_screen(&a, id);
        let z0 = a.zoom;
        let _ = a.update(Message::Canvas(ViewEvent::Zoom { factor: 1.6, around: before }));
        let after = head_on_screen(&a, id);
        assert!((after.x - before.x).abs() < 0.5 && (after.y - before.y).abs() < 0.5, "{before:?} -> {after:?}");
        assert!((a.zoom - z0 * 1.6).abs() < 1e-6);
    }

    #[test]
    fn saves_a_project_both_apps_can_open() {
        let dir = std::env::temp_dir().join(format!("corinzia-app-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("demo.project");
        let mut a = app(View::Page);
        a.path = Some(path.clone());
        let _ = a.update(Message::Act(Action::Save));
        assert!(a.editor.status.starts_with("Saved"), "{}", a.editor.status);
        assert!(!a.editor.modified);
        let p = corinzia_model::store::load(&path).unwrap();
        assert_eq!(p.movements[0].lanes.len(), 2);
        assert_eq!(*p.movements[0].lanes[0].voices[0].events, *a.editor.project.movements[0].lanes[0].voices[0].events);
        std::fs::remove_dir_all(&dir).ok();
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

    #[test]
    fn typing_in_input_mode_re_engraves() {
        let mut a = App::new(demo::blank("T", &[("Flute", "Fl.", "treble")], 2), None, View::Page, String::new());
        let before = a.engraved;
        for t in ["n", "4", "c", "d", "e", "shift+c"] {
            key(&mut a, t);
        }
        assert!(a.editor.input_mode);
        assert_ne!(a.engraved, before, "re-engraved after the edit");
        let ev = &a.editor.project.movements[0].lanes[0].voices[0].events;
        assert_eq!(ev.iter().filter(|e| !e.is_rest()).count(), 3);
        assert_eq!(ev[2].heads.len(), 2, "Shift+C added to the chord");
        key(&mut a, "ctrl+z");
        assert_eq!(a.editor.project.movements[0].lanes[0].voices[0].events[2].heads.len(), 1);
    }

    #[test]
    fn clicking_a_staff_in_input_mode_enters_a_note() {
        let mut a = App::new(demo::blank("T", &[("Flute", "Fl.", "treble")], 2), None, View::Page, String::new());
        key(&mut a, "n");
        let origins = score_view::page_origins(&a.pages, true);
        let st = a.pages[0].staves[0].clone();
        // Second line from the top of a treble staff is D5; one beat in.
        let col = a.pages[0].columns.iter().find(|c| c.system == st.system && c.quarter == 0.0).unwrap();
        let x = col.x + 0.5;
        let at = score_view::locate(&a.pages, &origins, iced::Point::new(x, st.top + 1.0)).unwrap();
        assert_eq!((at.part, at.step_below_top), (0, 2));
        let _ = a.update(Message::Canvas(ViewEvent::Click { hit: None, at: Some(at), shift: false }));
        let ev = &a.editor.project.movements[0].lanes[0].voices[0].events;
        assert_eq!(editor::pitch_name(ev[0].heads[0].pitch), "D5");
    }

    #[test]
    fn marquee_selects_a_passage_and_popovers_apply() {
        let mut a = app(View::Page);
        let origins = score_view::page_origins(&a.pages, true);
        let ids = score_view::ids_in(&a.pages, &origins, iced::Point::new(0.0, 0.0), iced::Point::new(1000.0, 1000.0));
        let _ = a.update(Message::Canvas(ViewEvent::Marquee { ids, shift: false }));
        let r = a.editor.range.unwrap();
        assert_eq!((r.lane_from, r.lane_to), (0, 1));
        assert!(a.selected.len() > 20);
        let _ = a.update(Message::Act(Action::Popover(PopoverKind::Key)));
        assert!(a.popover.is_some());
        let _ = a.update(Message::PopoverInput("x".into()));
        let _ = a.update(Message::PopoverSubmit);
        assert!(a.popover.as_ref().unwrap().error.is_some(), "bad input keeps the popover open with a hint");
        let _ = a.update(Message::PopoverInput("D".into()));
        let _ = a.update(Message::PopoverSubmit);
        assert!(a.popover.is_none());
        assert_eq!(a.editor.project.movements[0].keys[0].fifths, 2);
        key(&mut a, "esc");
        assert!(a.editor.range.is_none());
    }

    #[test]
    fn new_score_from_an_instrument_list() {
        let mut a = app(View::Page);
        let _ = a.update(Message::Act(Action::New));
        let _ = a.update(Message::NewInput(NewField::Instruments, "Flute, kazoo".into()));
        a.create_new();
        assert_eq!(a.new_score.as_ref().unwrap().error.as_deref(), Some("Unknown instrument: kazoo"));
        let _ = a.update(Message::NewInput(NewField::Instruments, "2 Violin, Viola, Cello".into()));
        let _ = a.update(Message::NewInput(NewField::Title, "Quartet".into()));
        let _ = a.update(Message::NewInput(NewField::Bars, "12".into()));
        a.create_new();
        assert!(a.new_score.is_none());
        assert_eq!(a.editor.project.title, "Quartet");
        let m = &a.editor.project.movements[0];
        assert_eq!((m.lanes.len(), m.length), (4, Frac::int(12)));
        assert!(a.editor.input_mode, "a new score starts in note input");
        assert_eq!(a.pages[0].staves.iter().filter(|s| s.system == 0).count(), 4);
    }

    fn render(a: &mut App, frames: usize) -> f32 {
        let (l, _) = a.playback.offline.as_mut().expect("offline engine").render_offline(frames);
        (l.iter().map(|v| v * v).sum::<f32>() / frames as f32).sqrt()
    }

    #[test]
    fn plays_the_score_and_follows_the_playhead() {
        let mut a = app(View::Page);
        let _ = a.update(Message::Act(Action::PlayStop));
        assert!(render(&mut a, 4800) > 0.005, "the flute and cello sound");
        assert!(a.playback.is_playing());
        // Andante 84: a quarter is 60/84 s; after 1.6 s (76 800 samples) the
        // playhead is 1.6 · 84/60 = 2.24 quarters in.
        render(&mut a, 72000);
        let (m, q) = a.playback.playhead().unwrap();
        assert_eq!(m, a.editor.project.movements[0].id);
        assert!((q - 2.24).abs() < 0.01, "playhead at {q}");
        let _ = a.update(Message::Act(Action::PlayStop));
        render(&mut a, 9600); // the 150 ms release
        assert!(!a.playback.is_playing());
        assert!(render(&mut a, 4800) < 1e-4, "silent once stopped and released");
        // Play from a selected note in bar 5.
        let id = a.editor.project.movements[0].lanes[1].voices[0].events[8].heads[0].id;
        let _ = a.update(Message::Canvas(ViewEvent::Click { hit: Some(id), at: None, shift: false }));
        let _ = a.update(Message::Act(Action::PlayStop));
        render(&mut a, 256);
        let (_, q) = a.playback.playhead().unwrap();
        assert!((q - 16.0).abs() < 0.1, "from bar 5: {q}");
    }

    #[test]
    fn mixer_edits_are_live_and_undoable() {
        let mut a = app(View::Page);
        let _ = a.update(Message::Act(Action::ShowMix));
        assert!(a.playback.plugins.iter().any(|p| p.id == "org.corinzia.sine"));
        let (engraved, track) = (a.engraved, a.editor.project.tracks[0].id);
        for db in [-1.0, -2.5, -4.0] {
            let _ = a.update(Message::Mix(mix::MixMsg::Gain(track, db)));
        }
        let _ = a.update(Message::Mix(mix::MixMsg::Released));
        assert_eq!(a.editor.project.tracks[0].gain_db, -4.0);
        assert_eq!(a.engraved, engraved, "mixer edits do not re-engrave");
        a.editor.undo();
        assert_eq!(a.editor.project.tracks[0].gain_db, 0.0, "one drag, one undo step");
        // Solo the cello: the flute is silent.
        let cello = a.editor.project.tracks[1].id;
        let _ = a.update(Message::Mix(mix::MixMsg::Solo(cello)));
        assert!(a.editor.project.tracks[1].solo);
        // Choose the built-in instrument explicitly: no node needed.
        let sine = a.playback.plugins.iter().position(|p| p.id == "org.corinzia.sine").unwrap();
        let _ = a.update(Message::Mix(mix::MixMsg::Assign(track, sine)));
        assert_eq!(a.editor.project.tracks[0].instrument_node, None);
        assert_eq!(a.playback.plugin_name(track), "Corinzia Sine");
    }

    #[test]
    fn entering_a_note_auditions_it() {
        let mut a = App::new(demo::blank("T", &[("Flute", "Fl.", "treble")], 2), None, View::Page, String::new());
        render(&mut a, 1024);
        assert!(render(&mut a, 1024) < 1e-6);
        key(&mut a, "n");
        key(&mut a, "c");
        assert!(render(&mut a, 2048) > 0.005, "the new note sounds");
        assert!(!a.playback.is_playing(), "without starting playback");
    }

    #[test]
    fn autosaves_projects_with_a_file() {
        let dir = std::env::temp_dir().join(format!("corinzia-autosave-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.project");
        corinzia_model::store::save(&demo::project(), &path).unwrap();
        let mut a = App::new(corinzia_model::store::load(&path).unwrap(), Some(path.clone()), View::Page, String::new());
        assert!(a.autosave_at.is_none(), "nothing to save after opening");
        key(&mut a, "ctrl+a");
        key(&mut a, "[");
        assert!(a.editor.modified && a.autosave_at.is_some());
        a.autosave_at = Some(std::time::Instant::now()); // due now
        assert!(a.autosave().is_some() && a.saving);
        // The task's work, as the executor would run it:
        corinzia_model::store::save(&a.editor.project, &path).unwrap();
        let rev = a.editor.revision;
        let _ = a.update(Message::Autosaved(rev, Ok(())));
        assert!(!a.editor.modified && !a.saving);
        let p = corinzia_model::store::load(&path).unwrap();
        assert!(p.movements[0].lanes[0].voices[0].events[0].articulations != 0, "the staccato reached the file");
        std::fs::remove_dir_all(&dir).ok();
    }

    fn type_into_popover(a: &mut App, text: &str) {
        let mut buf = String::new();
        for c in text.chars() {
            buf.push(c);
            let _ = a.update(Message::PopoverInput(buf.clone()));
            buf = a.popover.as_ref().map(|p| p.input.clone()).unwrap_or_default();
        }
    }

    #[test]
    fn lyrics_and_chords_are_typed_as_a_stream() {
        let mut a = app(View::Page);
        let flute = a.editor.project.movements[0].lanes[0].voices[0].events.clone();
        let _ = a.update(Message::Canvas(ViewEvent::Click { hit: Some(flute[0].heads[0].id), at: None, shift: false }));
        key(&mut a, "ctrl+l");
        assert_eq!(a.popover.as_ref().unwrap().kind, PopoverKind::Lyrics);
        type_into_popover(&mut a, "Glo-ri-a in ");
        let ev = &a.editor.project.movements[0].lanes[0].voices[0].events;
        let syl: Vec<(String, bool)> = ev.iter().take(4).map(|e| e.lyrics.first().map(|l| (l.text.clone(), l.hyphen)).unwrap_or_default()).collect();
        assert_eq!(syl, vec![("Glo".into(), true), ("ri".into(), true), ("a".into(), false), ("in".into(), false)]);
        assert_eq!(a.editor.selection, Some(ev[4].heads[0].id), "the popover moved on to the fifth note");
        let _ = a.update(Message::PopoverClose);
        // Chord symbols on the cello from its first note, moving on with space.
        let cello = a.editor.project.movements[0].lanes[1].voices[0].events.clone();
        let _ = a.update(Message::Canvas(ViewEvent::Click { hit: Some(cello[0].heads[0].id), at: None, shift: false }));
        key(&mut a, "ctrl+k");
        type_into_popover(&mut a, "C G7/b "); // a lower-case bass letter is fine
        type_into_popover(&mut a, "Hm ");
        assert!(a.popover.as_ref().unwrap().error.as_deref().is_some_and(|e| e.starts_with("Chord symbol not understood")));
        assert_eq!(a.popover.as_ref().unwrap().input, "Hm", "the bad input stays for correction");
        let chords: Vec<String> = a.editor.movement().unwrap().lanes[1].chords.iter().map(|c| c.text.clone()).collect();
        assert_eq!(chords, vec!["C", "G7/B"]);
        let _ = a.update(Message::PopoverClose);
        // A chord symbol is clickable: select it and delete it.
        let id = a.editor.movement().unwrap().lanes[1].chords[1].id;
        let origins = score_view::page_origins(&a.pages, true);
        let item = a.pages[0].items.iter().find(|i| matches!(i, Item::Text { id: t, .. } if *t == id)).unwrap();
        let r = item.bounds(corinzia_layout::font::Font::bravura());
        let at = iced::Point::new((r[0] + r[2]) / 2.0 + origins[0].x, (r[1] + r[3]) / 2.0 + origins[0].y);
        assert_eq!(score_view::hit_test(&a.pages, &origins, at), Some(id));
        let _ = a.update(Message::Canvas(ViewEvent::Click { hit: Some(id), at: None, shift: false }));
        key(&mut a, "del");
        assert_eq!(a.editor.movement().unwrap().lanes[1].chords.len(), 1);
        // Both reach MusicXML.
        let xml = String::from_utf8(files::export(&a.editor.project, 0, files::Format::MusicXml).unwrap()).unwrap();
        assert!(xml.contains("<lyric number=\"1\"><syllabic>begin</syllabic><text>Glo</text></lyric>"));
        assert!(xml.contains("<syllabic>end</syllabic><text>a</text>"));
        assert!(xml.contains("<harmony><root><root-step>C</root-step></root><kind text=\"\">major</kind></harmony>"));
        if let Ok(dir) = std::env::var("CORINZIA_EXPORT_DIR") {
            std::fs::write(std::path::Path::new(&dir).join("lyrics.musicxml"), &xml).unwrap();
            std::fs::write(std::path::Path::new(&dir).join("lyrics.pdf"), files::export(&a.editor.project, 0, files::Format::Pdf).unwrap()).unwrap();
        }
    }
}
