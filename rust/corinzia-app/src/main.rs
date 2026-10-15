//! Corinzia: a music notation editor and CLAP DAW (iced).
//!
//! `corinzia [file.project] [--view page|panorama] [--keys "n c d"]
//! [--screenshot out.png] [--export out.pdf|mid|musicxml] [--part N]`
//! (in `--keys`, `type:TEXT` and `enter` fill and submit an open popover)
//!
//! The score is engraved by the native engraver (corinzia-layout) and drawn
//! as vector geometry; playback runs through corinzia-audio.

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod arrange;
mod demo;
mod editor;
mod fader;
mod files;
mod keys;
mod mix;
mod playback;
mod panels;
mod score_view;
mod theme;

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
    composer: String,
    instruments: String,
    key: String,
    time: String,
    tempo: String,
    bars: String,
    error: Option<String>,
}

#[derive(Debug, Clone, Copy)]
enum NewField {
    Title,
    Composer,
    Instruments,
    Key,
    Time,
    Tempo,
    Bars,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Score,
    Mix,
}

/// Live audio output (PipeWire) for the windowed app; tests and --export run without.
/// Saving projects (Save, Save As, autosave). Off while Corinzia is a
/// preview not ready for real-world projects: the button is grayed out and
/// the shortcuts explain; export still works.
const SAVING_ENABLED: bool = false;

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
    /// What waits for "discard unsaved changes?" to be answered.
    confirm: Option<Box<Message>>,
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
    /// The window's size (logical pixels), for keeping the selection in view.
    window: Size,
    /// The keyboard shortcut overview (F1).
    help: bool,
    /// See [`SAVING_ENABLED`] (tests turn it on).
    saving_enabled: bool,
    /// The Export menu is open.
    export_menu: bool,
    /// The part shown (a player), or the full score.
    part: Option<corinzia_model::Id>,
    /// The lane each engraved staff shows.
    staff_lanes: Vec<usize>,
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
    /// The answer to "discard unsaved changes?".
    ConfirmDiscard(bool),
    /// Runs the message if there are no unsaved changes, or the user agrees to drop them.
    Guarded(Box<Message>),
    Proceed(Box<Message>),
    Open,
    Opened(Option<PathBuf>),
    /// A file dropped on the window.
    Dropped(PathBuf),
    ExportMenu(bool),
    /// Show a player's part (None: the full score).
    SetPart(Option<corinzia_model::Id>),
    Resized(Size),
    SaveTo(Option<PathBuf>),
    Export(files::Format),
    ExportTo(files::Format, Option<PathBuf>),
    Frame,
    Autosaved(u64, Result<(), String>),
    Shot(iced::window::Screenshot),
}

const POPOVER_INPUT: &str = "popover-input";
const NEW_TITLE: &str = "new-score-title";

struct Args {
    file: Option<PathBuf>,
    view: View,
    screenshot: Option<PathBuf>,
    keys: Vec<String>,
    export: Option<PathBuf>,
    /// Show player N's part (from 1).
    part: Option<usize>,
}

fn args() -> Args {
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
        "home" => Key::Named(Named::Home),
        "end" => Key::Named(Named::End),
        "f1" => Key::Named(Named::F1),
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
        let (project, path, status) = match &a.file {
            Some(f) => match files::load(f) {
                Ok(loaded) => loaded,
                Err(e) => (demo::project(), None, format!("Could not open {}: {e} — showing the demo", f.display())),
            },
            None => (demo::project(), None, "Demo project · N starts note input · drag selects · right-drag pans · Ctrl+wheel zooms".to_string()),
        };
        let mut app = App::new(project, path, a.view, status);
        app.screenshot = a.screenshot;
        app.part = a.part.and_then(|n| app.editor.project.players.get(n.wrapping_sub(1))).map(|p| p.id);
        app.engraved = None;
        for t in &a.keys {
            // An open popover takes typed text ("type:5:4") and Enter.
            if app.popover.is_some() {
                if let Some(text) = t.strip_prefix("type:") {
                    let _ = app.update(Message::PopoverInput(text.to_string()));
                    continue;
                }
                if t == "enter" {
                    let _ = app.update(Message::PopoverSubmit);
                    continue;
                }
            }
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
            confirm: None,
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
            window: Size::new(1280.0, 860.0),
            help: false,
            saving_enabled: SAVING_ENABLED,
            export_menu: false,
            part: None,
            staff_lanes: Vec::new(),
        };
        app.sync();
        app
    }

    fn title(&self) -> String {
        let p = &self.editor.project;
        let name = if p.title.is_empty() { "Untitled" } else { &p.title };
        format!("{}{name} — Corinzia", if self.editor.modified { "• " } else { "" })
    }

    /// Full score, or a player's part.
    fn part_picker(&self) -> Element<'_, Message> {
        let ed = &self.editor;
        let mut names = vec!["Full score".to_string()];
        let mut ids = vec![None];
        for pl in &ed.project.players {
            // Named by their first instrument, as the part is.
            let name = pl.instruments.first().and_then(|i| ed.project.instrument(*i)).map(|i| i.name.clone()).unwrap_or_else(|| pl.name.clone());
            names.push(name);
            ids.push(Some(pl.id));
        }
        let current = ids.iter().position(|&i| i == self.part).map(|k| names[k].clone());
        iced::widget::pick_list(names.clone(), current, move |chosen: String| {
            let k = names.iter().position(|n| *n == chosen).unwrap_or(0);
            Message::SetPart(ids[k])
        })
        .text_size(13)
        .padding([5, 9])
        .width(150)
        .into()
    }

    /// Native engraving of the current movement (milliseconds).
    fn engrave(&mut self) {
        self.engraved = Some(self.editor.music_revision);
        let Some(m) = self.editor.movement() else {
            self.pages.clear();
            return;
        };
        let t = Instant::now();
        // A part: the player's staves, as written, with multi-bar rests.
        let part = self.part.filter(|p| m.lanes.iter().any(|l| l.player == *p));
        self.staff_lanes = (0..m.lanes.len()).filter(|&li| part.is_none_or(|p| m.lanes[li].player == p)).collect();
        let (score, opt) = match part {
            Some(p) => (corinzia_model::notation::part_score(&self.editor.project, m, p), Options { multirest: true, names: false, ..Options::default() }),
            None => (corinzia_model::notation::score_as(&self.editor.project, m, self.editor.transposed), Options::default()),
        };
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
        if self.saving_enabled && self.path.is_some() && self.editor.revision != self.saved_revision && self.autosave_at.is_none() && !self.saving {
            self.autosave_at = Some(std::time::Instant::now() + std::time::Duration::from_millis(800));
        }
        let mut sel = HashSet::new();
        if let Some(id) = self.editor.selection {
            sel.insert(id);
            // A selected chord (or note by its event) shows all its heads.
            if let Some(loc) = self.editor.locate(id) {
                let v = &self.editor.movement().unwrap().lanes[loc.lane].voices[loc.voice];
                if let Some((e, None)) = corinzia_edit::find_event(&v.events, id) {
                    sel.extend(e.heads.iter().map(|h| h.id));
                }
            }
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

    /// The score canvas's size: the window less the toolbar, side panel and
    /// status bar (and their hairlines).
    fn viewport(&self) -> Size {
        Size::new((self.window.width - panels::WIDTH - 1.0).max(100.0), (self.window.height - 47.0 - 27.0).max(100.0))
    }

    /// Pans so that the selection (or the caret's note) is in view.
    fn reveal(&mut self) {
        let ed = &self.editor;
        // What to show: the selected element, or the events at the caret.
        let mut ids: Vec<u64> = ed.selection.into_iter().collect();
        if ids.is_empty() {
            if let Some(m) = ed.movement() {
                if let Some(l) = m.lanes.get(ed.caret_lane) {
                    for v in &l.voices {
                        if let Some(e) = v.events.iter().find(|e| e.start <= ed.caret && ed.caret <= e.end()) {
                            ids.push(e.id);
                            ids.extend(e.heads.iter().map(|h| h.id));
                        }
                    }
                }
            }
        }
        if ids.is_empty() || self.pages.is_empty() {
            return;
        }
        let font = corinzia_layout::font::Font::bravura();
        let origins = score_view::page_origins(&self.pages, self.view == View::Page);
        let mut r: Option<[f32; 4]> = None;
        for (p, o) in self.pages.iter().zip(&origins) {
            for it in p.items.iter().filter(|it| ids.contains(&it.id())) {
                let b = it.bounds(font);
                let b = [b[0] + o.x, b[1] + o.y, b[2] + o.x, b[3] + o.y];
                r = Some(match r {
                    None => b,
                    Some(a) => [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])],
                });
            }
        }
        let Some(r) = r else { return };
        let px = self.zoom * score_view::BASE_PX;
        let (x0, y0, x1, y1) = (r[0] * px + self.pan.x, r[1] * px + self.pan.y, r[2] * px + self.pan.x, r[3] * px + self.pan.y);
        let vp = self.viewport();
        let margin = 60.0;
        let shift = |lo: f32, hi: f32, size: f32| -> f32 {
            if hi - lo > size - 2.0 * margin {
                0.0
            } else if lo < margin {
                margin - lo
            } else if hi > size - margin {
                size - margin - hi
            } else {
                0.0
            }
        };
        self.pan = self.pan + Vector::new(shift(x0, x1, vp.width), shift(y0, y1, vp.height));
    }

    /// Ctrl+0: the page fills the canvas's width.
    fn fit_width(&mut self) {
        let Some(page) = self.pages.first() else { return };
        let vp = self.viewport();
        self.zoom = ((vp.width - 48.0) / (page.width * score_view::BASE_PX)).clamp(0.25, 8.0);
        self.pan = Vector::new(24.0, self.pan.y.min(24.0));
    }

    fn act(&mut self, a: Action) -> Task<Message> {
        let audition = matches!(a, Action::Note { .. } | Action::Pitch(_) | Action::Nudge(_) | Action::Accidental(_) | Action::Transpose(_) | Action::Step { .. } | Action::Edge { .. } | Action::Staff(_));
        let reveal = matches!(
            a,
            Action::Note { .. } | Action::Rest | Action::Step { .. } | Action::Edge { .. } | Action::Staff(_) | Action::Extend(_) | Action::CaretBy(_) | Action::CaretStaff(_) | Action::Paste
        );
        let task = self.act_inner(a);
        if reveal {
            self.sync();
            self.reveal();
        }
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
                if self.help || self.export_menu {
                    self.help = false;
                    self.export_menu = false;
                } else if self.popover.take().is_some() {
                } else if ed.input_mode {
                    ed.set_input_mode(false)
                } else {
                    ed.clear_selection()
                }
            }
            Action::Note { step, chord } => ed.enter_note(step, chord),
            Action::Rest => ed.enter_rest(),
            Action::Duration(d) => {
                ed.spec.base = corinzia_edit::BaseDuration::ALL[d.min(6) as usize];
                // Outside note input the selected music takes the duration.
                if !ed.input_mode {
                    ed.set_selection_duration();
                }
            }
            Action::Dots if !ed.input_mode && ed.has_selection() => ed.cycle_selection_dots(),
            Action::Dots => ed.cycle_dots(),
            Action::Pitch(step) => ed.repitch_selection(step),
            Action::Nudge(d) => ed.nudge(d),
            Action::Grid(i) => ed.grid = i.min(editor::GRIDS.len() - 1),
            Action::GridStep(d) => ed.step_grid(d),
            Action::Accidental(x) if !ed.input_mode && ed.has_selection() => ed.set_selection_accidental(x),
            Action::Triplet => ed.toggle_triplets(),
            Action::GraceInput(slash) => ed.toggle_grace_input(slash),
            Action::Movement(i) => ed.switch_movement(i),
            Action::MovementStep(d) => {
                let i = (ed.movement as i64 + d).clamp(0, ed.project.movements.len() as i64 - 1) as usize;
                ed.switch_movement(i);
            }
            Action::AddMovement => ed.add_movement(),
            Action::DeleteMovement => ed.delete_movement(),
            Action::TupletOff => {
                ed.tuplet_input = None;
                ed.status = "Tuplet input off".into();
            }
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
            Action::Step { dir, bar } => ed.select_step(dir, bar),
            Action::FitWidth => self.fit_width(),
            Action::Transposed => {
                ed.transposed = !ed.transposed;
                ed.status = if ed.transposed { "Transposed score: written pitches; letters you type are written pitches" } else { "Concert pitch" }.into();
                self.engraved = None;
            }
            Action::Help => self.help = !self.help,
            Action::Extend(d) => ed.extend_step(d),
            Action::Edge { last } => ed.select_edge(last),
            Action::Staff(d) => ed.select_staff(d),
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
            Action::Save | Action::SaveAs if !self.saving_enabled => {
                ed.status = "Saving is turned off in this preview build · export to PDF, MIDI or MusicXML instead".into();
            }
            Action::Save if self.path.is_some() => self.save(self.path.clone().unwrap()),
            Action::Save | Action::SaveAs => return self.ask_save_path(),
            Action::Open => return Task::done(Message::Guarded(Box::new(Message::Open))),
            Action::New => {
                self.new_score = Some(NewScore { bars: "32".into(), key: "C".into(), time: "4/4".into(), ..Default::default() });
                return iced::widget::operation::focus(NEW_TITLE);
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
        match files::load(&path) {
            Ok((p, kept, status)) => {
                let (view, pb) = (self.view, std::mem::replace(&mut self.playback, playback::Playback::new(false)));
                *self = App::with_playback(p, kept, view, status, pb);
            }
            Err(e) => self.editor.status = format!("Could not open {}: {e}", path.display()),
        }
    }

    fn create_new(&mut self) {
        let Some(n) = self.new_score.as_mut() else { return };
        let bars = n.bars.trim().parse::<i64>().unwrap_or(32).clamp(1, 2000);
        let title = if n.title.trim().is_empty() { "Untitled" } else { n.title.trim() };
        // Key, time (a pickup too: "3/4 pickup 1/4") and tempo, checked first.
        let key = if n.key.trim().is_empty() { None } else {
            match corinzia_edit::parse_key(&n.key) {
                Some(k) => Some(k),
                None => return n.error = Some(format!("Key not understood: {} (try C, Eb, f# minor)", n.key.trim())),
            }
        };
        let (time_text, pickup_text) = match n.time.to_lowercase().split_once("pickup") {
            Some((t, p)) => (t.trim().to_string(), Some(format!("pickup {}", p.trim()))),
            None => (n.time.trim().to_string(), None),
        };
        let time = if time_text.is_empty() { None } else {
            match corinzia_edit::parse_time(&time_text) {
                Some(t) => Some(t),
                None => return n.error = Some(format!("Time not understood: {time_text} (try 3/4, 6/8, C)")),
            }
        };
        let tempo = if n.tempo.trim().is_empty() { None } else {
            match corinzia_edit::parse_tempo(&n.tempo) {
                Some(t) => Some(t),
                None => return n.error = Some(format!("Tempo not understood: {} (try Allegro, Andante 84, q=96)", n.tempo.trim())),
            }
        };
        match files::new_project(title, &n.instruments, bars).map(|mut p| {
            p.composer = n.composer.trim().to_string();
            if let Some(m) = p.movements.first_mut() {
                if let Some(k) = key {
                    corinzia_edit::set_key(m, Frac::ZERO, k);
                }
                if let Some((num, den)) = time {
                    corinzia_edit::set_meter(m, Frac::ZERO, num, den);
                    m.length = Frac::new(num, den) * Frac::int(bars);
                }
                if let Some(t) = &tempo {
                    corinzia_edit::set_tempo(m, Frac::ZERO, t);
                }
                if let Some(len) = pickup_text.as_deref().and_then(|t| corinzia_edit::parse_pickup(t, Frac::new(1, time.map(|t| t.1).unwrap_or(4)))) {
                    corinzia_edit::set_pickup(m, len);
                }
                // Voice 1 is rests to the end in the new meter.
                for l in m.lanes.iter_mut() {
                    for v in l.voices.iter_mut() {
                        v.events = std::sync::Arc::new(Vec::new());
                    }
                }
            }
            p
        }) {
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
                if self.editor.project.track(t).is_some_and(|x| x.gain_db != db) {
                    self.editor.set_track(t, "Gain", true, |x| x.gain_db = db);
                    self.playback.live_mix(&self.editor.project, t);
                }
            }
            Pan(t, pan) => {
                if self.editor.project.track(t).is_some_and(|x| x.pan != pan) {
                    self.editor.set_track(t, "Pan", true, |x| x.pan = pan);
                    self.playback.live_mix(&self.editor.project, t);
                }
            }
            // The gesture is over: the next one is its own undo step.
            Released => self.editor.end_gesture(),
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
                let lane = self.staff_lanes.get(p.part).copied().unwrap_or(p.part);
                if lane < m.lanes.len() {
                    let pos = Frac::new((p.quarter * 1024.0).round() as i64, 4096);
                    let diatonic = top_line(&clef_at(&ed.project, m, lane, pos)) - p.step_below_top;
                    ed.place_note_at(lane, p.quarter, diatonic, shift);
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
                    ed.caret_lane = self.staff_lanes.get(p.part).copied().unwrap_or(p.part);
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
            Message::Resized(size) => self.window = size,
            Message::ExportMenu(open) => self.export_menu = open,
            Message::SetPart(p) => {
                self.part = p;
                self.engraved = None;
            }
            Message::Dropped(path) => {
                if path.extension().is_some_and(|e| e == "project") || files::is_musicxml(&path) {
                    return Task::done(Message::Guarded(Box::new(Message::Opened(Some(path)))));
                }
                self.editor.status = format!("Cannot open {}: drop a .project or MusicXML file", path.display());
            }
            Message::Canvas(ViewEvent::Pan(d)) => self.pan = self.pan + d,
            Message::Canvas(ViewEvent::Zoom { factor, around }) => {
                let z = (self.zoom * factor).clamp(0.25, 8.0);
                let k = z / self.zoom;
                // Keep the point under the cursor fixed.
                self.pan = Vector::new(around.x - (around.x - self.pan.x) * k, around.y - (around.y - self.pan.y) * k);
                self.zoom = z;
            }
            Message::Canvas(ViewEvent::Click { hit, at, shift }) => {
                self.export_menu = false;
                self.editor.status.clear();
                self.click(hit, at, shift)
            }
            Message::Canvas(ViewEvent::Marquee { ids, shift }) => self.editor.select_elements(&ids, shift),
            Message::Key(iced::keyboard::Event::KeyPressed { key, modifiers, .. }) => {
                if self.popover.is_some() || self.new_score.is_some() || self.confirm.is_some() {
                    if matches!(key, iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape)) {
                        if self.confirm.take().is_none() {
                            self.popover = None;
                            self.new_score = None;
                        }
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
                        NewField::Composer => n.composer = v,
                        NewField::Instruments => n.instruments = v,
                        NewField::Key => n.key = v,
                        NewField::Time => n.time = v,
                        NewField::Tempo => n.tempo = v,
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
                // Asked in the window (no system dialog needed).
                self.confirm = Some(next);
            }
            Message::ConfirmDiscard(yes) => {
                if let (Some(next), true) = (self.confirm.take(), yes) {
                    return Task::done(Message::Proceed(next));
                }
            }
            Message::Proceed(next) => match *next {
                Message::Open => {
                    let dir = self.path.as_ref().and_then(|p| p.parent().map(PathBuf::from));
                    return Task::perform(
                        async move {
                            let mut d = rfd::AsyncFileDialog::new()
                                .set_title("Open")
                                .add_filter("Scores (project, MusicXML)", &["project", "musicxml", "xml", "mxl"])
                                .add_filter("Corinzia project", &["project"])
                                .add_filter("MusicXML", &["musicxml", "xml", "mxl"]);
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
                self.export_menu = false;
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
                self.editor.status = match files::export_to(&self.editor.project, self.editor.movement, format, self.editor.transposed, &path) {
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
        let bold = iced::Font { weight: iced::font::Weight::Bold, ..iced::Font::DEFAULT };
        // A segmented control: options side by side in a trough.
        let segmented = |opts: Vec<(&'static str, bool, Message)>| -> Element<'_, Message> {
            let mut r = row![].spacing(2);
            for (label, on, m) in opts {
                r = r.push(button(text(label).size(13)).padding([3, 12]).style(theme::segment(on)).on_press(m));
            }
            container(r).padding(2).style(theme::trough).into()
        };
        let flat = |label: &'static str, enabled: bool, m: Message| button(text(label).size(13)).padding([5, 9]).style(theme::flat(false)).on_press_maybe(enabled.then_some(m));
        let rule = || container(Space::new()).width(1).height(22).style(theme::rule);
        let mut toolbar = row![
            text("Corinzia").size(15).font(bold).style(theme::accent),
            segmented(vec![
                ("Score", self.tab == Tab::Score, Message::Act(Action::ShowScore)),
                ("Mix", self.tab == Tab::Mix, Message::Act(Action::ShowMix)),
            ]),
            rule(),
            panels::tip(
                button(text(if self.playback.is_playing() { "■  Stop" } else { "▶  Play" }).size(13)).padding([5, 14]).style(theme::primary).on_press(Message::Act(Action::PlayStop)),
                "Play from the selection (Space)",
            ),
            panels::tip(flat("↶", ed.can_undo(), Message::Act(Action::Undo)), self.undo_tip.as_str()),
            panels::tip(flat("↷", ed.can_redo(), Message::Act(Action::Redo)), "Redo (Ctrl+Shift+Z)"),
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center);
        if self.tab == Tab::Score {
            toolbar = toolbar
                .push(rule())
                .push(panels::tip(
                    button(text("✎  Note input").size(13)).padding([5, 12]).style(theme::tool(ed.input_mode)).on_press(Message::Act(Action::SetInput(!ed.input_mode))),
                    "Note input (N)",
                ))
                .push(segmented(vec![("Page", self.view == View::Page, Message::SetView(View::Page)), ("Panorama", self.view == View::Panorama, Message::SetView(View::Panorama))]))
                .push(panels::tip(self.part_picker(), "Full score or one player's part (written pitch, multi-bar rests)"))
                .push(panels::tip(
                    button(text("Transposed").size(13)).padding([5, 10]).style(theme::tool(ed.transposed)).on_press(Message::Act(Action::Transposed)),
                    "Transposed score (Ctrl+T); off: concert pitch",
                ))
;
        }
        let title = ed.movement().map(|m| m.title.clone()).unwrap_or_default();
        let toolbar = toolbar
            .push(panels::spacer())
            .push(panels::tip(flat("?", true, Message::Act(Action::Help)), "Keyboard shortcuts (F1)"))
            .push(panels::tip(flat("New", true, Message::Act(Action::New)), "New score (Ctrl+N)"))
            .push(panels::tip(flat("Open", true, Message::Act(Action::Open)), "Open (Ctrl+O)"))
            .push(panels::tip(
                flat("Save", self.saving_enabled, Message::Act(Action::Save)),
                if self.saving_enabled { "Save (Ctrl+S)" } else { "Saving is turned off in this preview build" },
            ))
            .push(button(text("Export ▾").size(13)).padding([5, 9]).style(theme::flat(self.export_menu)).on_press(Message::ExportMenu(!self.export_menu)));
        let toolbar = container(toolbar.padding([0, 12])).height(46).center_y(46).width(Length::Fill).style(theme::bar);

        // Lanes to the staves they are engraved on (a part shows some).
        let staff_of = |lane: usize| self.staff_lanes.iter().position(|&l| l == lane);
        let caret = ed.input_mode.then(|| staff_of(ed.caret_lane)).flatten().map(|s| Caret { part: s, quarter: quarters(ed.caret) });
        let highlight = ed.range.and_then(|r| {
            let shown: Vec<usize> = (r.lane_from..=r.lane_to).filter_map(staff_of).collect();
            Some(Highlight { part_from: *shown.first()?, part_to: *shown.last()?, start: quarters(r.from), end: quarters(r.to) })
        });
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
                    .size(13)
                    .style(theme::dim),
                    panels::spacer(), button(text("✕").size(12)).padding([1, 6]).style(theme::flat(false)).on_press(Message::PopoverClose)]
                    .align_y(iced::Alignment::Center),
                text_input(p.kind.placeholder(), &p.input).id(POPOVER_INPUT).on_input(Message::PopoverInput).on_submit(Message::PopoverSubmit).padding(6).size(15),
            ]
            .spacing(6)
            .width(320);
            if let Some(e) = &p.error {
                card = card.push(text(e).size(12).style(theme::danger));
            }
            desk = desk.push(container(iced::widget::opaque(container(card).padding(12).style(theme::card))).padding(16).width(Length::Fill).align_x(iced::Alignment::Center));
        }

        if let Some(n) = &self.new_score {
            let field = |label: &'static str, hint: &'static str, value: &str, f: NewField| {
                column![text(label).size(12).style(theme::dim), text_input(hint, value).on_input(move |v| Message::NewInput(f, v)).on_submit(Message::NewCreate).padding(6).size(14)].spacing(3)
            };
            let title = column![
                text("Title").size(12).style(theme::dim),
                text_input("Untitled", &n.title).id(NEW_TITLE).on_input(|v| Message::NewInput(NewField::Title, v)).on_submit(Message::NewCreate).padding(6).size(14)
            ]
            .spacing(3);
            // Ready-made ensembles fill in the instruments.
            let templates = iced::widget::Row::with_children(files::TEMPLATES.iter().map(|(label, list)| {
                let on = n.instruments == *list;
                button(text(*label).size(12)).padding([3, 8]).style(theme::segment(on)).on_press(Message::NewInput(NewField::Instruments, list.to_string())).into()
            }))
            .spacing(4)
            .wrap();
            let mut card = column![
                text("New score").size(18).font(bold),
                row![title.width(Length::FillPortion(3)), field("Composer", "", &n.composer, NewField::Composer).width(Length::FillPortion(2))].spacing(10),
                text("Ensemble").size(12).style(theme::dim),
                templates,
                field("Instruments (comma separated; \"2 Violin\" for two; any name)", "Flute, Clarinet, Violin, Cello", &n.instruments, NewField::Instruments),
                row![
                    field("Key", "C", &n.key, NewField::Key).width(Length::Fill),
                    field("Time", "4/4 (3/4 pickup 1/4)", &n.time, NewField::Time).width(Length::Fill),
                    field("Tempo", "Allegro 120", &n.tempo, NewField::Tempo).width(Length::Fill),
                    field("Bars", "32", &n.bars, NewField::Bars).width(Length::Fill),
                ]
                .spacing(10),
            ]
            .spacing(10)
            .width(560);
            if let Some(e) = &n.error {
                card = card.push(text(e).size(12).style(theme::danger));
            }
            card = card.push(
                row![
                    panels::spacer(),
                    button(text("Cancel").size(13)).padding([6, 14]).style(theme::tool(false)).on_press(Message::NewCancel),
                    button(text("Create").size(13)).padding([6, 14]).style(theme::primary).on_press(Message::NewCreate)
                ]
                .spacing(8),
            );
            desk = desk.push(container(iced::widget::opaque(container(card).padding(20).style(theme::card))).center(Length::Fill));
        }

        if self.help {
            desk = desk.push(container(iced::widget::opaque(help_card())).center(Length::Fill));
        }
        if self.confirm.is_some() {
            let card = column![
                text("Discard the changes to this score?").size(16).font(bold),
                text("Saving is not available in this preview, so they cannot be kept; export first if you need them.").size(12).style(theme::dim),
                row![
                    panels::spacer(),
                    button(text("Cancel").size(13)).padding([6, 14]).style(theme::tool(false)).on_press(Message::ConfirmDiscard(false)),
                    button(text("Discard").size(13)).padding([6, 14]).style(theme::primary).on_press(Message::ConfirmDiscard(true)),
                ]
                .spacing(8),
            ]
            .spacing(12)
            .width(420);
            desk = desk.push(container(iced::widget::opaque(container(card).padding(20).style(theme::card))).center(Length::Fill));
        }
        if self.export_menu {
            let item = |label: &'static str, hint: &'static str, f: files::Format| {
                button(column![text(label).size(13), text(hint).size(11).style(theme::dim)].spacing(1)).width(Length::Fill).padding([6, 10]).style(theme::flat(false)).on_press(Message::Export(f))
            };
            let what = if ed.transposed { "as shown (transposed)" } else { "as shown (concert pitch)" };
            let menu = container(
                column![
                    item("PDF…", what, files::Format::Pdf),
                    item("Parts (PDF)…", "each player's part, as written", files::Format::Parts),
                    item("MIDI…", "the performance, track by track", files::Format::Midi),
                    item("MusicXML…", "for other notation programs", files::Format::MusicXml),
                ]
                .spacing(2)
                .width(250),
            )
            .padding(6)
            .style(theme::card);
            desk = desk.push(container(iced::widget::opaque(menu)).padding([6, 10]).width(Length::Fill).align_x(iced::Alignment::End));
        }

        let tabs = container(segmented(vec![
            ("Notes", self.panel == Panel::Notes, Message::SetPanel(Panel::Notes)),
            ("Structure", self.panel == Panel::Structure, Message::SetPanel(Panel::Structure)),
        ]))
        .padding([10, 12]);
        let side = container(column![tabs, panels::panel(ed, self.panel).map(Message::Act)]).width(panels::WIDTH).height(Length::Fill).style(theme::panel);
        let vline = || container(Space::new()).width(1).height(Length::Fill).style(theme::hairline);
        let hline = || container(Space::new()).height(1).width(Length::Fill).style(theme::hairline);

        let left = if !ed.status.is_empty() { ed.status.clone() } else { ed.selection_label() };
        let middle = if ed.input_mode { ed.caret_label() } else if ed.status.is_empty() { String::new() } else { ed.selection_label() };
        let status = container(
            row![
                // The messages give way (clipped) to the controls on the right.
                container(
                    row![
                        text(left).size(12).wrapping(text::Wrapping::None).style(|_: &Theme| text::Style { color: Some(theme::TEXT) }),
                        Space::new().width(24),
                        text(middle).size(12).wrapping(text::Wrapping::None),
                    ]
                )
                .width(Length::Fill)
                .clip(true),
                Space::new().width(12),
                grid_picker(ed),
                Space::new().width(16),
                text(self.playback.problem.clone().unwrap_or_default()).size(12).style(theme::danger),
                Space::new().width(16),
                movement_picker(ed, title),
                Space::new().width(16),
                panels::tip(
                    text(format!("{} page{} · {:.0} ms", self.pages.len(), if self.pages.len() == 1 { "" } else { "s" }, self.engrave_ms)).size(12).style(theme::dim),
                    "Pages, and how long engraving took",
                ),
                Space::new().width(12),
                panels::tip(flat("−", true, Message::ZoomBy(1.0 / 1.25)), "Zoom out (Ctrl+−)"),
                text(format!("{:.0}%", self.zoom * 100.0)).size(12).style(theme::dim).width(38).align_x(iced::Alignment::Center),
                panels::tip(flat("+", true, Message::ZoomBy(1.25)), "Zoom in (Ctrl+=)"),
            ]
            .align_y(iced::Alignment::Center)
            .padding([0, 12]),
        )
        .height(26)
        .center_y(26)
        .width(Length::Fill)
        .style(theme::status);
        let body: Element<'_, Message> = match self.tab {
            Tab::Score => row![side, vline(), desk].height(Length::Fill).into(),
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
                    hline(),
                    container(mix::view(&ed.project, &self.playback, self.strip).map(Message::Mix)).height(360).width(Length::Fill).style(theme::panel),
                ]
                .height(Length::Fill)
                .into()
            }
        };
        column![toolbar, hline(), body, hline(), status].into()
    }

    fn subscription(&self) -> Subscription<Message> {
        let window = iced::event::listen_with(|e, _, _| match e {
            iced::Event::Window(iced::window::Event::FileDropped(p)) => Some(Message::Dropped(p)),
            iced::Event::Window(iced::window::Event::Resized(s)) => Some(Message::Resized(s)),
            _ => None,
        });
        let keys = Subscription::batch([iced::keyboard::listen().map(Message::Key), window]);
        if self.screenshot.is_some() || self.playback.busy() || self.autosave_at.is_some() {
            Subscription::batch([keys, iced::window::frames().map(|_| Message::Frame)])
        } else {
            keys
        }
    }
}

/// The keyboard shortcut overview (F1).
fn help_card() -> Element<'static, Message> {
    const SECTIONS: [(&str, &[(&str, &str)]); 4] = [
        (
            "Note input",
            &[
                ("N", "Note input on / off"),
                ("A – G", "Enter a note"),
                ("Shift+A – G", "Add to the chord"),
                ("1 – 7", "64th … whole note"),
                (".", "Dot, double dot"),
                (";", "Tuplet: 3, 5, 5:4…   : ends it"),
                ("/  ?", "Grace notes (slashed, plain)"),
                ("R  ,", "Rest"),
                ("−  =  0", "Flat, sharp, natural"),
                ("T", "Tie"),
                ("V", "Next voice"),
                ("Q", "Chord mode"),
                ("Backspace", "Delete the last note"),
            ],
        ),
        (
            "Moving around",
            &[
                ("← →", "Previous / next note"),
                ("Ctrl+← →", "Previous / next bar"),
                ("Shift+← →", "Extend the selection"),
                ("Home  End", "First / last note"),
                ("Alt+↑ ↓", "Staff above / below"),
                ("↑ ↓", "Transpose a step"),
                ("Ctrl+↑ ↓", "Transpose an octave"),
                ("Space", "Play / stop"),
                ("Ctrl+= −", "Zoom in / out"),
                ("Ctrl+0", "Fit the page width"),
                ("Ctrl+T", "Transposed / concert pitch"),
                ("Ctrl+PgUp PgDn", "Previous / next movement"),
            ],
        ),
        (
            "Marks and text",
            &[
                ("[  '  ]  \\", "Staccato, accent, tenuto, marcato"),
                ("S", "Slur"),
                ("H  Shift+H", "Crescendo, diminuendo"),
                ("Shift+K  M  T", "Key, time, tempo"),
                ("Shift+C", "Clef; 8va, 8vb, loco"),
                ("Shift+B  R", "Barline; ending, segno, D.C.…"),
                ("Shift+X  P  O", "Expression, technique (ped), ornament"),
                ("Shift+F", "Fingering"),
                ("Shift+A", "Rehearsal mark"),
                ("Ctrl+L", "Lyrics"),
                ("Ctrl+K", "Chord symbols"),
            ],
        ),
        (
            "Editing and files",
            &[
                ("Ctrl+Z  Ctrl+Y", "Undo, redo"),
                ("Ctrl+C  X  V", "Copy, cut, paste"),
                ("Ctrl+A", "Select all"),
                ("Delete", "Delete the selection"),
                ("R", "Repeat the selection"),
                ("1 – 7  .", "Selection's duration, dots"),
                ("A – G", "Selection's pitch"),
                ("Alt+← →", "Move by the rhythmic grid"),
                ("Ctrl+B", "Insert a bar"),
                ("Ctrl+Shift+⌫", "Delete bars"),
                ("Ctrl+N  O  S", "New, open, save"),
                ("Ctrl+1  2", "Score, Mix"),
                ("F1", "This overview"),
            ],
        ),
    ];
    let bold = iced::Font { weight: iced::font::Weight::Bold, ..iced::Font::DEFAULT };
    let mut cols = row![].spacing(20);
    for (title, keys) in SECTIONS {
        let mut c = column![text(title.to_uppercase()).size(10).style(theme::faint)].spacing(5).width(200);
        for (k, what) in keys {
            c = c.push(row![text(*k).size(12).font(bold).style(theme::accent).width(92), text(*what).size(12)].spacing(8));
        }
        cols = cols.push(c);
    }
    let close = button(text("✕").size(12)).padding([1, 6]).style(theme::flat(false)).on_press(Message::Act(Action::Help));
    container(column![row![text("Keyboard shortcuts").size(16).font(bold), panels::spacer(), close], cols].spacing(14)).padding(20).style(theme::card).into()
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
    fn saving_is_off_in_the_preview() {
        let dir = std::env::temp_dir().join(format!("corinzia-nosave-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("demo.project");
        let mut a = app(View::Page);
        a.path = Some(path.clone());
        key(&mut a, "ctrl+a");
        key(&mut a, "[");
        let _ = a.update(Message::Act(Action::Save));
        assert!(a.editor.status.starts_with("Saving is turned off"), "{}", a.editor.status);
        assert!(a.autosave_at.is_none() && !path.exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn saves_a_project_both_apps_can_open() {
        let dir = std::env::temp_dir().join(format!("corinzia-app-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("demo.project");
        let mut a = app(View::Page);
        a.saving_enabled = true;
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
    fn a_part_shows_one_player_as_written_and_maps_staves_to_lanes() {
        let p = files::new_project("Trio", "Flute, Clarinet, Piano", 8).unwrap();
        let mut a = App::new(p, None, View::Page, String::new());
        a.sync();
        assert_eq!(a.staff_lanes, [0, 1, 2, 3]);
        let clarinet = a.editor.project.players[1].id;
        let piano = a.editor.project.players[2].id;
        let _ = a.update(Message::SetPart(Some(piano)));
        a.sync();
        // The piano's two staves, engraved as staves 0 and 1.
        assert_eq!(a.staff_lanes, [2, 3]);
        assert_eq!(a.pages[0].staves.iter().filter(|s| s.system == 0).count(), 2);
        // Eight empty bars: one multi-bar rest, counted 8.
        let eight = corinzia_layout::font::Glyph::TimeSigDigit(8).codepoint();
        assert!(a.pages[0].items.iter().any(|i| matches!(i, corinzia_layout::display::Item::Glyph { glyph, .. } if *glyph == eight)));
        // A click on the part's first staff enters into the piano's right hand.
        a.editor.set_input_mode(true);
        a.click(None, Some(StaffPoint { part: 0, quarter: 0.0, step_below_top: 4 }), false);
        assert_eq!(a.editor.caret_lane, 2);
        assert!(!a.editor.project.movements[0].lanes[2].voices[0].events.iter().all(|e| e.is_rest()));
        // The clarinet's part is written a tone up: concert C is written D.
        let _ = a.update(Message::SetPart(Some(clarinet)));
        a.editor.caret_lane = 1;
        a.editor.caret = Frac::ZERO;
        a.editor.transposed = false;
        a.editor.enter_note(0, false);
        let score = corinzia_model::notation::part_score(&a.editor.project, &a.editor.project.movements[0], clarinet);
        assert_eq!(score.parts.len(), 1);
        assert!(score.instrument.starts_with("Clarinet") && score.instrument == score.parts[0].name);
        assert_eq!(score.parts[0].voices[0].events[0].pitches[0].step, 1, "written D");
        // Parts PDF: every player's part.
        let pages = files::parts_pages(&a.editor.project);
        assert!(pages.len() >= 3);
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
    fn new_score_with_key_time_pickup_tempo_any_instrument_and_an_in_app_confirmation() {
        let mut a = app(View::Page);
        // An edit makes the score "unsaved": Create asks in the window first.
        a.editor.set_input_mode(true);
        a.editor.enter_note(0, false);
        let _ = a.update(Message::Act(Action::New));
        let _ = a.update(Message::NewInput(NewField::Instruments, files::TEMPLATES[2].1.to_string() + ", Kazoo"));
        for (f, v) in [(NewField::Title, "Suite"), (NewField::Composer, "J. Doe"), (NewField::Key, "Eb"), (NewField::Time, "3/4 pickup 1/4"), (NewField::Tempo, "Allegro 132"), (NewField::Bars, "8")] {
            let _ = a.update(Message::NewInput(f, v.into()));
        }
        // Create -> Guarded -> the confirmation card (no system dialog).
        let mut msg = Some(Message::NewCreate);
        while let Some(m) = msg.take() {
            let _ = a.update(m.clone());
            msg = match m {
                Message::NewCreate => Some(Message::Guarded(Box::new(Message::Proceed(Box::new(Message::NewCreate))))),
                _ => None,
            };
        }
        assert!(a.confirm.is_some() && a.new_score.is_some(), "asked first");
        let _ = a.update(Message::ConfirmDiscard(true));
        let _ = a.update(Message::Proceed(Box::new(Message::NewCreate)));
        assert!(a.new_score.is_none());
        let p = &a.editor.project;
        assert_eq!((p.title.as_str(), p.composer.as_str()), ("Suite", "J. Doe"));
        assert_eq!(p.instruments.last().unwrap().name, "Kazoo");
        let m = &p.movements[0];
        assert_eq!(m.lanes.len(), 5);
        assert_eq!(m.keys[0].fifths, -3);
        assert_eq!((m.tempo[0].bpm, m.tempo[0].text.as_str()), (132.0, "Allegro"));
        assert!(m.meter[0].pickup && (m.meter[1].num, m.meter[1].den) == (3, 4));
        assert_eq!(m.length, Frac::new(1, 4) + Frac::new(3, 4) * Frac::int(8));
        assert!(!a.pages.is_empty());
    }

    #[test]
    fn new_score_from_an_instrument_list() {
        let mut a = app(View::Page);
        let _ = a.update(Message::Act(Action::New));
        let _ = a.update(Message::NewInput(NewField::Instruments, "".into()));
        a.create_new();
        assert!(a.new_score.as_ref().unwrap().error.as_deref().unwrap().starts_with("Name at least one instrument"));
        let _ = a.update(Message::NewInput(NewField::Time, "7/5".into()));
        let _ = a.update(Message::NewInput(NewField::Instruments, "Flute".into()));
        a.create_new();
        assert!(a.new_score.as_ref().unwrap().error.as_deref().unwrap().starts_with("Time not understood"));
        let _ = a.update(Message::NewInput(NewField::Time, "4/4".into()));
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
        // A second drag is a step of its own.
        let _ = a.update(Message::Mix(mix::MixMsg::Gain(track, -10.0)));
        let _ = a.update(Message::Mix(mix::MixMsg::Released));
        a.editor.undo();
        assert_eq!(a.editor.project.tracks[0].gain_db, -4.0, "the second drag undone alone");
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
        a.saving_enabled = true;
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
        let xml = String::from_utf8(files::export(&a.editor.project, 0, files::Format::MusicXml, false).unwrap()).unwrap();
        assert!(xml.contains("<lyric number=\"1\"><syllabic>begin</syllabic><text>Glo</text></lyric>"));
        assert!(xml.contains("<syllabic>end</syllabic><text>a</text>"));
        assert!(xml.contains("<harmony><root><root-step>C</root-step></root><kind text=\"\">major</kind></harmony>"));
        if let Ok(dir) = std::env::var("CORINZIA_EXPORT_DIR") {
            std::fs::write(std::path::Path::new(&dir).join("lyrics.musicxml"), &xml).unwrap();
            std::fs::write(std::path::Path::new(&dir).join("lyrics.pdf"), files::export(&a.editor.project, 0, files::Format::Pdf, false).unwrap()).unwrap();
        }
    }
}

/// The status bar's movement: its title between previous and next.
fn movement_picker(ed: &editor::Editor, title: String) -> Element<'static, Message> {
    let n = ed.project.movements.len();
    let step = |label: &'static str, to: Option<usize>| {
        let b = iced::widget::button(text(label).size(13).center()).padding([0, 6]).height(22).style(theme::flat(false));
        match to {
            Some(i) => b.on_press(Message::Act(Action::Movement(i))),
            None => b,
        }
    };
    let label = if n > 1 { format!("{title}  ({}/{n})", ed.movement + 1) } else { title };
    row![
        step("‹", ed.movement.checked_sub(1)),
        text(label).size(12),
        step("›", (ed.movement + 1 < n).then_some(ed.movement + 1)),
    ]
    .spacing(2)
    .align_y(iced::Alignment::Center)
    .into()
}

/// The status bar's rhythmic grid: what Alt+arrows move music by and the
/// caret steps by (Alt+[ and Alt+] change it).
fn grid_picker(ed: &editor::Editor) -> Element<'static, Message> {
    let mut r = row![text("Grid").size(12).style(theme::dim), Space::new().width(6)].align_y(iced::Alignment::Center);
    for (i, (_, _, label)) in editor::GRIDS.iter().enumerate() {
        r = r.push(
            iced::widget::button(text(*label).size(13).center())
                .padding([0, 7])
                .height(22)
                .style(theme::segment(ed.grid == i))
                .on_press(Message::Act(Action::Grid(i))),
        );
    }
    r.into()
}
