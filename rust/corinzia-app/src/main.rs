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
mod browser;
mod demo;
mod editor;
mod fader;
mod files;
mod keys;
mod midi_in;
mod mix;
mod playback;
mod prefs;
mod presets;
mod panels;
mod score_view;
mod theme;

use corinzia_layout::{layout_line, layout_pages, DisplayList};
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
    /// The item on the page being retyped (opened by a double-click).
    target: Option<u64>,
    /// The point on the canvas it opens by: what was clicked, else the
    /// selection or the caret.
    anchor: Option<iced::Point>,
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

/// The Players card: the instrumentation being edited, applied as a whole.
struct PlayersDialog {
    rows: Vec<PlayerRow>,
    /// Instruments to add ("Flute, 2 Violin").
    add: String,
    error: Option<String>,
}

struct PlayerRow {
    /// The player in the score, or None for one to add.
    player: Option<corinzia_model::Id>,
    /// Its instruments as typed: the first, then doublings.
    instruments: String,
    detail: String,
}

#[derive(Debug, Clone)]
enum PlayersMsg {
    Open,
    Move(usize, isize),
    Remove(usize),
    /// A row's instruments retyped.
    Edit(usize, String),
    AddInput(String),
    Add,
    /// One of the quick-add buttons.
    Quick(&'static str),
    Apply,
    Cancel,
}

/// The Layout card: the score's and the parts' page layouts as typed,
/// and the page texts.
struct LayoutDialog {
    parts: bool,
    fields: [LayoutFields; 2], // score, parts
    texts: [String; 4],        // subtitle, lyricist, arranger, copyright
    error: Option<String>,
}

#[derive(Clone)]
struct LayoutFields {
    size: String, // a name from PAGE_SIZES, or "Custom"
    width: String,
    height: String,
    landscape: bool,
    staff: String,
    margin_y: String,
    margin_x: String,
    spacing: String, // percent
    staff_distance: String,
    system_gap: String,
    bars: String,
    page_numbers: bool,
    condense: bool,
}

impl LayoutFields {
    fn of(l: &corinzia_model::Layout) -> LayoutFields {
        let n = |v: f64| {
            let s = format!("{v:.2}");
            s.trim_end_matches('0').trim_end_matches('.').to_string()
        };
        let size = corinzia_model::PAGE_SIZES.iter().find(|(_, w, h)| (w - l.page_width_mm).abs() < 0.05 && (h - l.page_height_mm).abs() < 0.05).map(|p| p.0).unwrap_or("Custom");
        LayoutFields {
            size: size.to_string(),
            width: n(l.page_width_mm),
            height: n(l.page_height_mm),
            landscape: l.landscape,
            staff: n(l.staff_mm),
            margin_y: n(l.margin_y_mm),
            margin_x: n(l.margin_x_mm),
            spacing: n(l.spacing * 100.0),
            staff_distance: n(l.staff_distance),
            system_gap: n(l.system_gap),
            bars: if l.bars_per_system == 0 { String::new() } else { l.bars_per_system.to_string() },
            page_numbers: l.page_numbers,
            condense: l.condense,
        }
    }

    /// The layout typed, or what is wrong with it.
    fn layout(&self) -> Result<corinzia_model::Layout, String> {
        let num = |s: &str, what: &str, lo: f64, hi: f64| -> Result<f64, String> {
            s.trim().replace(',', ".").parse::<f64>().ok().filter(|v| (lo..=hi).contains(v)).ok_or(format!("{what}: a number from {lo} to {hi}"))
        };
        Ok(corinzia_model::Layout {
            page_width_mm: num(&self.width, "Page width (mm)", 50.0, 1000.0)?,
            page_height_mm: num(&self.height, "Page height (mm)", 50.0, 1000.0)?,
            landscape: self.landscape,
            staff_mm: num(&self.staff, "Staff size (mm)", 3.0, 12.0)?,
            margin_y_mm: num(&self.margin_y, "Top and bottom margins (mm)", 0.0, 60.0)?,
            margin_x_mm: num(&self.margin_x, "Side margins (mm)", 0.0, 60.0)?,
            spacing: num(&self.spacing, "Note spacing (%)", 50.0, 250.0)? / 100.0,
            staff_distance: num(&self.staff_distance, "Staff distance (spaces)", 5.0, 30.0)?,
            system_gap: num(&self.system_gap, "System gap (spaces)", 0.0, 40.0)?,
            bars_per_system: if self.bars.trim().is_empty() { 0 } else { num(&self.bars, "Bars per system", 1.0, 64.0)? as u32 },
            page_numbers: self.page_numbers,
            condense: self.condense,
        })
    }
}

/// The Preferences card: a copy being edited, the plugin folders as typed,
/// the output devices found.
struct PrefsDialog {
    draft: prefs::Prefs,
    paths: String,
    devices: Vec<(String, String)>,
    midi_ports: Vec<(String, String)>,
    error: Option<String>,
}

#[derive(Debug, Clone)]
enum PrefsMsg {
    Open,
    Theme(String),
    Device(String),
    Rate(u32),
    Buffer(u32),
    Midi(String),
    Paths(String),
    DefaultPaths(bool),
    Cancel,
    Apply,
}

#[derive(Debug, Clone)]
enum LayoutMsg {
    Tab(bool),
    Size(String),
    Field(LayoutField, String),
    Landscape(bool),
    PageNumbers(bool),
    Condense(bool),
    Text(usize, String),
    Defaults,
    Apply,
    Cancel,
}

#[derive(Debug, Clone, Copy)]
enum LayoutField {
    Width,
    Height,
    Staff,
    MarginY,
    MarginX,
    Spacing,
    StaffDistance,
    SystemGap,
    Bars,
}

/// The quick-add buttons of the Players card.
const QUICK_ADD: [&str; 16] = ["Flute", "Oboe", "Clarinet", "Bassoon", "Horn", "Trumpet", "Trombone", "Piano", "Soprano", "Alto", "Tenor", "Bass", "Violin", "Viola", "Cello", "Double bass"];

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
    fx: Option<corinzia_model::Id>,    // and its selected effect (audio node)
    /// The presets of the plugin shown in the Mix tab, and the name being typed.
    preset_names: Vec<String>,
    preset_name: String,
    /// The automation each track's arrange lane shows (none: notes only),
    /// and the curves drawn for them.
    auto_shown: std::collections::HashMap<corinzia_model::Id, corinzia_model::AutoTarget>,
    arrange_curves: Vec<Option<arrange::Curve>>,
    arrange_clips: Vec<arrange::ClipBlock>,
    /// An audio export under way.
    bounce: Option<corinzia_audio::bounce::Bounce>,
    /// Folders folded away in the arrange view.
    folded: HashSet<String>,
    /// The clip selected in the arrange view (track, clip), and while it is
    /// dragged, the clip as it was (its length made explicit).
    selected_clip: Option<(corinzia_model::Id, corinzia_model::Id)>,
    clip_origin: Option<corinzia_model::AudioClip>,
    clip_dragged: bool,
    /// Ctrl was held at the press: the first drag step makes the copy.
    clip_copy: bool,
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
    players: Option<PlayersDialog>,
    layout: Option<LayoutDialog>,
    /// The preferences in use, and the card that edits them.
    prefs: prefs::Prefs,
    prefs_dialog: Option<PrefsDialog>,
    /// The MIDI keyboard, when one is chosen, and the keys held on it.
    midi: Option<midi_in::Input>,
    midi_held: Vec<u8>,
    /// Corinzia's own file browser, when the system's dialog cannot open.
    browser: Option<browser::Browser>,
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
    /// Every movement on the pages, one after another (Page view).
    all_movements: bool,
    /// The movement each page shows.
    page_movements: Vec<usize>,
    /// The movement edited when the view was last brought up to date.
    shown_movement: usize,
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
    Players(PlayersMsg),
    Layout(LayoutMsg),
    Prefs(PrefsMsg),
    /// Read the MIDI keyboard.
    MidiPoll,
    /// The answer to "discard unsaved changes?".
    ConfirmDiscard(bool),
    /// Runs the message if there are no unsaved changes, or the user agrees to drop them.
    Guarded(Box<Message>),
    Proceed(Box<Message>),
    Open,
    /// Open, showing MusicXML files only.
    Import,
    Opened(Option<PathBuf>),
    /// The system file dialog's answer; `failed`: it could not open.
    Picked { purpose: browser::Purpose, path: Option<PathBuf>, failed: bool },
    /// A sound file chosen for an audio track.
    AudioChosen(corinzia_model::Id, Option<PathBuf>),
    Browser(browser::BrowserMsg),
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
    /// While plugin editors are open: their timers, descriptors and windows.
    PluginTick,
    Autosaved(u64, Result<(), String>),
    Shot(iced::window::Screenshot),
}

const POPOVER_INPUT: &str = "popover-input";
const NEW_TITLE: &str = "new-score-title";
const PLAYERS_ADD: &str = "players-add";

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
        let prefs = if cfg!(test) { prefs::Prefs::default() } else { prefs::Prefs::load() };
        theme::set(&prefs.theme);
        let pb = playback::Playback::new(live, &prefs);
        let mut app = Self::with_playback(project, path, view, status, pb);
        app.open_midi(&prefs.midi_input);
        app.prefs = prefs;
        app
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
            fx: None,
            preset_names: Vec::new(),
            preset_name: String::new(),
            auto_shown: Default::default(),
            arrange_curves: Vec::new(),
            arrange_clips: Vec::new(),
            bounce: None,
            folded: HashSet::new(),
            selected_clip: None,
            clip_origin: None,
            clip_dragged: false,
            clip_copy: false,
            arrange: arrange::Content::default(),
            arrange_cache: canvas::Cache::new(),
            arrange_ppq: 40.0,
            arrange_scroll: 0.0,
            saved_revision: 0,
            autosave_at: None,
            saving: false,
            undo_tip: String::new(),
            new_score: None,
            players: None,
            layout: None,
            prefs: prefs::Prefs::default(),
            prefs_dialog: None,
            midi: None,
            midi_held: Vec::new(),
            all_movements: false,
            page_movements: Vec::new(),
            shown_movement: 0,
            browser: None,
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
        .width(130)
        .into()
    }

    /// The pages showing movement `mi`.
    fn pages_of(&self, mi: usize) -> std::ops::Range<usize> {
        let first = self.page_movements.iter().position(|&m| m == mi).unwrap_or(0);
        let n = self.page_movements.iter().filter(|&&m| m == mi).count();
        first..first + n
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
        let project = &self.editor.project;
        let score_of = |mi: usize| {
            let m = &project.movements[mi];
            match part {
                Some(p) => {
                    let s = corinzia_perform::part_score(project, m, p);
                    let lanes = (0..m.lanes.len()).filter(|&li| m.lanes[li].player == p).collect();
                    ((s, lanes), files::page_setup(&project.part_layout, true))
                }
                None => {
                    let mut s = corinzia_perform::score_as(project, m, self.editor.transposed);
                    let lanes = if project.layout.condense { corinzia_model::notation::condense(&mut s) } else { (0..s.parts.len()).collect() };
                    ((s, lanes), files::page_setup(&project.layout, false))
                }
            }
        };
        let ((score, lanes), (opt, spec)) = score_of(self.editor.movement);
        self.staff_lanes = lanes;
        self.page_movements.clear();
        self.pages = match self.view {
            // Every movement, as printed: each from a new page, the work's
            // header on the first, page numbers running on.
            View::Page if self.all_movements => {
                let mut pages = Vec::new();
                for mi in 0..project.movements.len() {
                    if part.is_some_and(|p| !project.movements[mi].lanes.iter().any(|l| l.player == p)) {
                        continue;
                    }
                    let ((mut score, _), (opt, spec)) = score_of(mi);
                    if mi > 0 {
                        for t in [&mut score.work_title, &mut score.composer, &mut score.subtitle, &mut score.lyricist, &mut score.arranger, &mut score.copyright] {
                            t.clear();
                        }
                    }
                    let spec = corinzia_layout::PageSpec { first_page: pages.len() + 1, ..spec };
                    let these = layout_pages(&score, &opt, &spec);
                    self.page_movements.extend(std::iter::repeat_n(mi, these.len()));
                    pages.extend(these);
                }
                pages
            }
            View::Page => layout_pages(&score, &opt, &spec),
            View::Panorama => {
                let mut l = layout_line(&score, &opt);
                // A panorama strip: the system plus room for marks above and below.
                let pad = 8.0;
                l = DisplayList {
                    space_pt: l.space_pt,
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
        if self.page_movements.len() != self.pages.len() {
            self.page_movements = vec![self.editor.movement; self.pages.len()];
        }
        self.engrave_ms = t.elapsed().as_secs_f64() * 1000.0;
        self.arrange = arrange::content(&self.editor.project, self.editor.movement);
    }

    /// The automation curve shown on a track's arrange lane.
    fn curve(&self, track: corinzia_model::Id) -> Option<arrange::Curve> {
        let target = *self.auto_shown.get(&track)?;
        let (label, min, max) = mix::auto_options(&self.editor.project, &self.playback, track).into_iter().find(|o| o.1 == Some(target)).map(|o| (o.0, o.2, o.3))?;
        let q = |f: Frac| f.num as f64 * 4.0 / f.den as f64;
        let lane = self.editor.auto_lane(target);
        let points = lane.map(|l| l.points.iter().map(|p| (q(p.at), p.value)).collect()).unwrap_or_default();
        let holds = lane.map(|l| l.points.iter().map(|p| p.hold).collect()).unwrap_or_default();
        Some(arrange::Curve { label, min, max, points, holds })
    }

    /// Brings the engraving and the selection colours up to date.
    fn sync(&mut self) {
        if self.engraved != Some(self.editor.music_revision) {
            self.engrave();
        }
        self.arrange_curves = self.arrange.lanes.iter().map(|l| self.curve(l.track)).collect();
        // Another movement edited, its pages out of sight: go to its first.
        if self.shown_movement != self.editor.movement {
            self.shown_movement = self.editor.movement;
            self.show_movement_pages();
        }
        self.playback.sync(&self.editor.project, self.editor.music_revision, self.editor.revision, self.path.as_deref().and_then(|p| p.parent()));
        arrange::layout_rows(&mut self.arrange, &self.editor.project, &self.folded);
        self.arrange_clips = arrange::clips(&self.editor.project, self.editor.movement, self.movement_section(), &self.playback.pool.media);
        if self.selected_clip.is_some_and(|(t, c)| self.editor.clip(t, c).is_none()) {
            self.selected_clip = None;
        }
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
                        sel.extend(e.dynamic.map(|_| corinzia_score::score::ids::dynamic(e.id)));
                    }
                }
            }
        }
        sel.extend(self.editor.selected_slur());
        sel.extend(self.editor.picked.iter().copied());
        self.selected = sel;
        self.undo_tip = self.editor.undo_label().map(|l| format!("Undo {l} (Ctrl+Z)")).unwrap_or_else(|| "Nothing to undo".into());
        self.cache.clear();
        self.arrange_cache.clear();
    }

    /// Starts an audio export of the movement edited: its own plugins,
    /// restored from the live ones' current settings.
    fn start_bounce(&mut self, path: PathBuf) {
        let mut project = self.editor.project.clone();
        self.playback.pool.store_states(&mut project);
        let folder = self.path.as_deref().and_then(|p| p.parent());
        match corinzia_audio::bounce::Bounce::new(&project, self.editor.movement, self.playback.sample_rate, self.playback.max_frames(), folder, path) {
            Ok(b) => {
                self.editor.status = "Exporting audio… (Escape stops)".into();
                self.bounce = Some(b);
            }
            Err(e) => self.editor.status = format!("Export failed: {e}"),
        }
    }

    /// Renders some of the audio export (about 30 ms' work a frame).
    fn bounce_step(&mut self) {
        let Some(b) = self.bounce.as_mut() else { return };
        let started = Instant::now();
        let mut done = false;
        while !done && started.elapsed() < std::time::Duration::from_millis(30) {
            done = b.step(0.1);
        }
        if !done {
            self.editor.status = format!("Exporting audio… {:.0}% (Escape stops)", b.progress() * 100.0);
            return;
        }
        let b = self.bounce.take().unwrap();
        let (path, warnings) = (b.path.clone(), b.warnings.clone());
        self.editor.status = match b.finish() {
            Ok(s) => {
                let mut msg = format!("Exported {} ({:.1} s, peak {:.1} dBFS)", path.display(), s.seconds, s.peak_db);
                if s.clipped > 0 {
                    msg += &format!(" — {} samples clipped: lower the master or add a limiter", s.clipped);
                }
                if let Some(w) = warnings.first() {
                    msg += &format!(" — {w}");
                }
                msg
            }
            Err(e) => format!("Export failed: {e}"),
        };
    }

    /// Places a sound file on an audio track (`None`, or not an audio
    /// track: a new one named after the file) at the caret of the movement edited. A file in the
    /// project's folder is kept by its relative path.
    fn add_audio_file(&mut self, track: Option<corinzia_model::Id>, path: &std::path::Path) {
        let Some(movement) = self.editor.movement().map(|m| m.id) else { return };
        let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Audio".into());
        let folder = self.path.as_deref().and_then(|p| p.parent());
        let file = match folder.and_then(|f| path.strip_prefix(f).ok()) {
            Some(rel) => rel.to_string_lossy().into_owned(),
            None => path.to_string_lossy().into_owned(),
        };
        // Decoded now, so a file Corinzia cannot read says so at once.
        if let Err(e) = self.playback.pool.media.get(&self.playback.pool.media.resolve(&file).to_string_lossy()) {
            self.editor.status = e;
            return;
        }
        let at = arrange::seconds_of(self.movement_section(), quarters(self.editor.caret));
        let track = match track.filter(|t| self.editor.project.track(*t).is_some_and(|t| t.audio)) {
            Some(t) => t,
            None => self.editor.add_audio_track(&name),
        };
        if let Some(id) = self.editor.add_clip(track, corinzia_model::AudioClip::new(0, &name, &file, movement, at)) {
            self.selected_clip = Some((track, id));
            self.strip = Some(track);
        }
        self.sync();
    }

    /// Changes the selected clip; `live`: a slider or typing (one undo step
    /// until let go; playback catches up then).
    fn edit_selected_clip(&mut self, label: &str, live: bool, f: impl FnOnce(&mut corinzia_model::AudioClip)) {
        if let Some((t, c)) = self.selected_clip {
            self.editor.edit_clip(t, c, label, live, f);
            self.clip_dragged |= live;
        }
    }

    /// The playback timeline's section of the movement edited (its tempo:
    /// seconds to quarters).
    fn movement_section(&self) -> Option<&corinzia_audio::snapshot::Section> {
        let id = self.editor.movement()?.id;
        self.playback.timeline.sections.iter().find(|s| s.movement == id)
    }

    /// A drag step on the selected clip: `q` (quarters) is its new start,
    /// edge, or the end of a fade, applied to the clip as it was pressed.
    fn drag_clip(&mut self, edit: arrange::ClipEdit, q: f64) {
        use arrange::ClipEdit;
        const SHORTEST: f64 = 0.01;
        let (Some((track, mut id)), Some(o)) = (self.selected_clip, self.clip_origin.clone()) else { return };
        if std::mem::take(&mut self.clip_copy) {
            // Ctrl+drag: the copy moves, the original stays.
            let Some(copy) = self.editor.duplicate_clip(track, id) else { return };
            id = copy;
            self.selected_clip = Some((track, id));
            self.editor.end_gesture();
        }
        let s = arrange::seconds_of(self.movement_section(), q);
        let file = self.playback.pool.media.loaded(&o.file).map(|d| d.seconds()).unwrap_or(f64::INFINITY);
        let label = match edit {
            ClipEdit::Move => "Move audio",
            ClipEdit::TrimStart | ClipEdit::TrimEnd => "Trim audio",
            ClipEdit::FadeIn | ClipEdit::FadeOut => "Fade audio",
        };
        self.clip_dragged = true;
        self.editor.edit_clip(track, id, label, true, |c| match edit {
            ClipEdit::Move => c.at = s.max(0.0),
            ClipEdit::TrimStart => {
                // The start moves; the sound stays where it was.
                let at = s.clamp((o.at - o.offset).max(0.0), o.at + o.length - SHORTEST);
                let d = at - o.at;
                (c.at, c.offset, c.length) = (at, o.offset + d, o.length - d);
                c.fade_in = o.fade_in.min(c.length);
                c.fade_out = o.fade_out.min(c.length - c.fade_in);
            }
            ClipEdit::TrimEnd => {
                c.length = (s - o.at).clamp(SHORTEST, (file - o.offset).max(SHORTEST));
                c.fade_out = o.fade_out.min(c.length);
                c.fade_in = o.fade_in.min(c.length - c.fade_out);
            }
            ClipEdit::FadeIn => c.fade_in = (s - o.at).clamp(0.0, o.length - o.fade_out),
            ClipEdit::FadeOut => c.fade_out = (o.at + o.length - s).clamp(0.0, o.length - o.fade_in),
        });
    }

    /// Asks for a file to open (`musicxml`: MusicXML only) with the
    /// system's dialog, else Corinzia's browser.
    fn pick_file(&mut self, musicxml: bool) -> Task<Message> {
        self.pick_file_for(browser::Purpose::Open, musicxml)
    }

    fn pick_file_for(&mut self, purpose: browser::Purpose, musicxml: bool) -> Task<Message> {
        if std::env::var("CORINZIA_FILE_DIALOG").is_ok_and(|v| v == "builtin") {
            return self.open_browser(purpose);
        }
        let dir = self.path.as_ref().and_then(|p| p.parent().map(PathBuf::from));
        let mut d = rfd::AsyncFileDialog::new();
        if let Some(dir) = dir {
            d = d.set_directory(dir);
        }
        let d = match purpose {
            browser::Purpose::Open if musicxml => d.set_title("Import MusicXML").add_filter("MusicXML", &["musicxml", "xml", "mxl"]),
            browser::Purpose::Open => d
                .set_title("Open")
                .add_filter("Scores (project, MusicXML)", &["project", "musicxml", "xml", "mxl"])
                .add_filter("Corinzia project", &["project"])
                .add_filter("MusicXML", &["musicxml", "xml", "mxl"]),
            browser::Purpose::Audio(_) => d.set_title("Add a sound file").add_filter("Sound files", &browser::AUDIO_EXTENSIONS),
            browser::Purpose::Export(format) => {
                let name = files::sibling(self.path.as_deref(), &self.editor.project.title, format.extension());
                d.set_title(format!("Export {}", format.label()))
                    .add_filter(format.label(), &[format.extension()])
                    .set_file_name(name.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default())
            }
        };
        Task::perform(
            async move {
                let started = Instant::now();
                let path = match purpose {
                    browser::Purpose::Open | browser::Purpose::Audio(_) => d.pick_file().await,
                    browser::Purpose::Export(_) => d.save_file().await,
                }
                .map(|h| h.path().to_path_buf());
                // Nobody cancels a dialog this fast: it never opened.
                let failed = path.is_none() && started.elapsed() < std::time::Duration::from_millis(600);
                (path, failed)
            },
            move |(path, failed)| Message::Picked { purpose, path, failed },
        )
    }

    fn open_browser(&mut self, purpose: browser::Purpose) -> Task<Message> {
        let dir = self.path.as_ref().and_then(|p| p.parent().map(PathBuf::from));
        let name = match purpose {
            browser::Purpose::Open | browser::Purpose::Audio(_) => String::new(),
            browser::Purpose::Export(f) => files::sibling(self.path.as_deref(), &self.editor.project.title, f.extension()).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        };
        self.browser = Some(browser::Browser::new(purpose, dir, name));
        iced::widget::operation::focus(browser::NAME_INPUT)
    }

    fn open_popover(&mut self, kind: PopoverKind) -> Task<Message> {
        let anchor = self.popover_anchor();
        self.popover = Some(Popover { kind, input: String::new(), error: None, target: None, anchor });
        iced::widget::operation::focus(POPOVER_INPUT)
    }

    /// Where on the canvas the popover target is: the selected element,
    /// else the passage start or the caret on its staff.
    fn popover_anchor(&self) -> Option<iced::Point> {
        let ed = &self.editor;
        let origins = score_view::page_origins(&self.pages, self.view == View::Page);
        let at = ed.structure_target().and_then(|(lane, pos)| Some(Caret { part: self.staff_lanes.iter().position(|&l| l == lane)?, quarter: quarters(pos) }));
        let desk = score_view::anchor(&self.pages, &origins, &self.pages_of(ed.movement), ed.selection, at)?;
        let px = self.zoom * score_view::BASE_PX;
        Some(iced::Point::new(desk.x * px + self.pan.x, desk.y * px + self.pan.y))
    }

    /// A double-click on the page: a title, tempo mark, text, chord symbol,
    /// dynamic or lyric opens its popover with its text to retype.
    fn edit_on_page(&mut self, id: u64) -> Task<Message> {
        use corinzia_score::score::ids;
        // The page texts are edited in the Layout card.
        if [ids::SUBTITLE, ids::LYRICIST, ids::ARRANGER, ids::COPYRIGHT].contains(&id) {
            return self.act(Action::Layout);
        }
        let Some((kind, text)) = self.editor.item_text(id) else { return Task::none() };
        // Lyrics carry on from the note as when typed (space: the next note).
        let target = if let Some((ev, verse)) = corinzia_score::score::ids::as_lyric(id) {
            self.editor.select(Some(ev));
            self.editor.lyric_verse = verse;
            None
        } else {
            self.editor.select(Some(id));
            Some(id)
        };
        self.sync();
        let anchor = self.popover_anchor();
        self.popover = Some(Popover { kind, input: text, error: None, target, anchor });
        iced::widget::operation::focus(POPOVER_INPUT)
    }

    /// A key press from the score keyboard.
    fn press(&mut self, key: &iced::keyboard::Key, mods: iced::keyboard::Modifiers) -> Option<Task<Message>> {
        use iced::keyboard::{key::Named, Key};
        // The selected clip in the arrange view: Delete removes it.
        if let (Tab::Mix, Some((track, clip)), Key::Named(Named::Delete | Named::Backspace)) = (self.tab, self.selected_clip, key) {
            self.editor.delete_clip(track, clip);
            self.selected_clip = None;
            return Some(Task::none());
        }
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
        let Some(inst) = lane.instrument_at(e.start) else { return };
        // A drum set plays the sound mapped to the staff position.
        let keys: Vec<i32> = match head {
            Some(h) => vec![e.heads[h].pitch.midi_key()],
            None => e.heads.iter().map(|h| h.pitch.midi_key()).collect(),
        }
        .into_iter()
        .map(|k| ed.project.sounding_key(inst, k))
        .collect();
        let Some(track) = ed.project.instrument(inst).and_then(|i| i.track) else { return };
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

    /// Scrolls to the current movement's first page if none of its pages
    /// is in view (several movements shown).
    fn show_movement_pages(&mut self) {
        let pages = self.pages_of(self.editor.movement);
        if !self.all_movements || pages.is_empty() {
            return;
        }
        let origins = score_view::page_origins(&self.pages, self.view == View::Page);
        let px = self.zoom * score_view::BASE_PX;
        let vp = self.viewport();
        let visible = pages.clone().any(|k| {
            let (y0, y1) = (origins[k].y * px + self.pan.y, (origins[k].y + self.pages[k].height) * px + self.pan.y);
            y1 > 0.0 && y0 < vp.height
        });
        if !visible {
            self.pan.y = 24.0 - origins[pages.start].y * px;
        }
    }

    /// Ctrl+0: the page fills the canvas's width.
    fn fit_width(&mut self) {
        let Some(page) = self.pages.first() else { return };
        let vp = self.viewport();
        self.zoom = ((vp.width - 48.0) / (page.width * score_view::BASE_PX)).clamp(0.25, 8.0);
        self.pan = Vector::new(24.0, self.pan.y.min(24.0));
    }

    fn act(&mut self, a: Action) -> Task<Message> {
        if self.slur_action(&a) {
            self.sync();
            return Task::none();
        }
        let audition = matches!(a, Action::Note { .. } | Action::Pitch(_) | Action::Nudge(_) | Action::Accidental(_) | Action::Transpose(_) | Action::Step { .. } | Action::Edge { .. } | Action::Staff(_));
        let reveal = matches!(
            a,
            Action::Note { .. } | Action::Rest | Action::Step { .. } | Action::Edge { .. } | Action::Staff(_) | Action::Extend(_) | Action::CaretBy(_) | Action::CaretStaff(_) | Action::Paste | Action::PasteCue
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

    /// Keys on a selected slur: Shift+←→ move its end a note, Alt+←→ its
    /// start, X flips it, Delete removes it, Escape deselects.
    fn slur_action(&mut self, a: &Action) -> bool {
        let Some(id) = self.editor.selected_slur() else {
            if matches!(a, Action::Flip) {
                self.editor.status = "Select a slur first (click its curve)".into();
                return true;
            }
            return false;
        };
        let above = score_view::slur_above(&self.pages, id).unwrap_or(true);
        let ed = &mut self.editor;
        match a {
            Action::Extend(d) => ed.step_slur(editor::SlurHandle::End, *d),
            Action::Nudge(d) => ed.step_slur(editor::SlurHandle::Start, *d),
            Action::Flip => ed.flip_slur(above),
            Action::Delete | Action::Backspace => ed.delete_slur(),
            Action::Escape => ed.clear_selection(),
            _ => return false,
        }
        true
    }

    fn act_inner(&mut self, a: Action) -> Task<Message> {
        let ed = &mut self.editor;
        ed.status.clear(); // the action reports its own outcome
        match a {
            Action::PlayStop => self.play_stop(),
            Action::Cycle => {
                // A passage selected: cycle it; else the cycle on or off.
                match (ed.range, ed.movement().map(|m| m.id)) {
                    (Some(r), Some(m)) => {
                        self.playback.set_cycle(Some((m, quarters(r.from), quarters(r.to))));
                        ed.status = format!("Cycling bars {}–{}", corinzia_edit::bar_number(ed.movement().unwrap(), r.from).0, corinzia_edit::bar_number(ed.movement().unwrap(), r.to - Frac::new(1, 64)).0);
                    }
                    _ if self.playback.cycle.is_none() => ed.status = "Select a passage to cycle (or Shift+drag along the arrange view's ruler)".into(),
                    _ => self.playback.toggle_cycle(),
                }
            }
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
            Action::Flip => {} // a slur's (slur_action)
            Action::Extend(d) => ed.extend_step(d),
            Action::Edge { last } => ed.select_edge(last),
            Action::Staff(d) => ed.select_staff(d),
            Action::Transpose(s) => ed.transpose_selection(s),
            Action::Backspace => ed.backspace(),
            Action::Delete => ed.delete_selection(),
            Action::Copy => ed.copy(),
            Action::Cut => ed.cut(),
            Action::Paste => ed.paste(),
            Action::PasteCue => ed.paste_cue(),
            Action::Tab => ed.toggle_tab(),
            Action::CrossStaff(d) => ed.cross_staff(d),
            Action::SelectAll => ed.select_all(),
            Action::Beam(mode) => ed.set_beam(mode),
            Action::Break { page } => {
                // In the part shown, or the full score.
                if let Err(e) = ed.toggle_break(page, self.part) {
                    ed.status = e;
                }
            }
            Action::AllMovements => {
                self.all_movements = !self.all_movements;
                if self.all_movements && self.view != View::Page {
                    self.view = View::Page;
                }
                self.engraved = None;
                ed.status = if self.all_movements { "All movements, one after another" } else { "One movement at a time" }.into();
            }
            Action::Preferences => self.prefs_msg(PrefsMsg::Open),
            Action::Layout => {
                let p = &ed.project;
                self.layout = Some(LayoutDialog {
                    parts: self.part.is_some(),
                    fields: [LayoutFields::of(&p.layout), LayoutFields::of(&p.part_layout)],
                    texts: [p.subtitle.clone(), p.lyricist.clone(), p.arranger.clone(), p.copyright.clone()],
                    error: None,
                });
            }
            Action::Explode | Action::Reduce => {
                let r = if a == Action::Explode { ed.explode() } else { ed.reduce() };
                if let Err(e) = r {
                    ed.status = e;
                }
            }
            Action::SelectSimilar => {
                if let Err(e) = ed.select_similar() {
                    ed.status = e;
                }
            }
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
                let (view, pb) = (self.view, std::mem::replace(&mut self.playback, playback::Playback::new(false, &prefs::Prefs::default())));
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
                let (view, pb) = (self.view, std::mem::replace(&mut self.playback, playback::Playback::new(false, &prefs::Prefs::default())));
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

    /// The plugin a preset target names, and its project node (where its state is kept).
    fn preset_plugin(&self, t: mix::Target) -> Option<(std::sync::Arc<corinzia_audio::host::Plugin>, Option<corinzia_model::Id>)> {
        match t {
            mix::Target::Effect(n) => self.playback.pool.effect(n).cloned().map(|p| (p, Some(n))),
            mix::Target::Track(id) => self.playback.pool.plugin_of_track(id).cloned().map(|p| (p, self.editor.project.track(id).and_then(|x| x.instrument_node))),
        }
    }

    /// The presets of the plugin the Mix tab shows.
    fn refresh_presets(&mut self) {
        let target = match (self.fx, self.strip) {
            (Some(n), _) => Some(mix::Target::Effect(n)),
            (None, Some(t)) if self.editor.project.track(t).is_some() => Some(mix::Target::Track(t)),
            _ => None,
        };
        self.preset_names = match (target.and_then(|t| self.preset_plugin(t)), Self::preset_base()) {
            (Some((p, _)), Some(base)) => presets::list(&base, &p.info.id),
            _ => vec![],
        };
    }

    fn preset_base() -> Option<PathBuf> {
        if cfg!(test) {
            Some(std::env::temp_dir().join(format!("corinzia-test-presets-{}", std::process::id())))
        } else {
            presets::base()
        }
    }

    fn mix(&mut self, m: mix::MixMsg) {
        self.mix_msg(m);
        self.refresh_presets();
    }

    fn mix_msg(&mut self, m: mix::MixMsg) {
        use mix::MixMsg::*;
        match m {
            Select(t) => self.strip = Some(t),
            Editor(t) => {
                self.strip = Some(t);
                self.toggle_editor(t);
            }
            Gain(t, db) if t == editor::MASTER => {
                if self.editor.project.master.gain_db != db {
                    self.editor.set_master_gain(db);
                    self.playback.live_levels(&self.editor.project);
                }
            }
            Gain(b, db) if self.editor.project.bus(b).is_some() => {
                if self.editor.project.bus(b).is_some_and(|x| x.gain_db != db) {
                    self.editor.set_bus(b, "Bus gain", true, |x| x.gain_db = db);
                    self.playback.live_levels(&self.editor.project);
                }
            }
            Pan(b, pan) if self.editor.project.bus(b).is_some() => {
                if self.editor.project.bus(b).is_some_and(|x| x.pan != pan) {
                    self.editor.set_bus(b, "Bus pan", true, |x| x.pan = pan);
                    self.playback.live_levels(&self.editor.project);
                }
            }
            Mute(b) if self.editor.project.bus(b).is_some() => self.editor.set_bus(b, "Mute", false, |x| x.mute = !x.mute),
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
            AddEffect(owner, i) => {
                let Some(info) = self.playback.plugins.get(i).cloned() else { return };
                let bundle = info.bundle.to_string_lossy().into_owned();
                self.fx = self.editor.add_insert(owner, &info.name, &bundle, &info.id);
            }
            SelectEffect(n) => self.fx = Some(n),
            EffectEditor(n) => {
                self.fx = Some(n);
                if let Some(pl) = self.playback.pool.effect(n) {
                    if pl.editor_is_open() {
                        pl.close_editor();
                    } else {
                        let title = self.editor.project.node(n).map(|x| x.name.clone()).unwrap_or_default();
                        if let Err(e) = pl.open_editor(&title) {
                            self.editor.status = e;
                        }
                    }
                }
            }
            RemoveEffect(owner, n) => {
                if let Some(pl) = self.playback.pool.effect(n) {
                    pl.close_editor();
                }
                self.editor.remove_insert(owner, n);
                if self.fx == Some(n) {
                    self.fx = None;
                }
            }
            MoveEffect(owner, n, by) => self.editor.move_insert(owner, n, by),
            Bypass(n) => self.editor.toggle_bypass(n),
            Param(n, id, v) => {
                if let Some(pl) = self.playback.pool.effect(n) {
                    pl.set_param(id, v);
                    // Kept with the project (saved, and restored on the next load).
                    if let Some(state) = pl.save_state().filter(|_| !pl.is_processing()) {
                        self.editor.project.plugin_state.insert(n, std::sync::Arc::new(state));
                    }
                    self.editor.modified = true;
                }
            }
            AddBus => {
                let id = self.editor.add_bus("");
                self.strip = Some(id);
            }
            AddAudioTrack => {
                let n = self.editor.project.tracks.iter().filter(|t| t.audio).count() + 1;
                let id = self.editor.add_audio_track(&format!("Audio {n}"));
                self.strip = Some(id);
            }
            RemoveTrack(t) => {
                self.editor.remove_audio_track(t);
                if self.strip == Some(t) {
                    self.strip = None;
                }
            }
            AddAudio(_) => {} // a file picker: in `update`
            ClipName(n) => self.edit_selected_clip("Rename audio", true, move |c| c.name = n),
            ClipGain(db) => self.edit_selected_clip("Audio gain", true, move |c| c.gain_db = db),
            ClipMute => self.edit_selected_clip("Mute audio", false, |c| c.mute = !c.mute),
            ClipCue(label) => {
                let player = self.editor.project.players.iter().find(|p| p.name == label).map(|p| p.id);
                self.edit_selected_clip("Audio cue", false, move |c| c.cue = player);
            }
            ClipCueLine => self.edit_selected_clip("Audio cue line", false, |c| c.cue_line = !c.cue_line),
            ClearClip(id) => self.playback.clear_clip(id),
            Share(t, host) => {
                self.editor.share_instrument(t, host);
            }
            ClipRemove => {
                if let Some((t, c)) = self.selected_clip.take() {
                    self.editor.delete_clip(t, c);
                }
            }
            RemoveBus(b) => self.editor.remove_bus(b),
            Output(t, label) => {
                let bus = self.editor.project.buses.iter().find(|b| b.name == label).map(|b| b.id);
                self.editor.set_output(t, bus);
            }
            ToggleSend(t, b) => self.editor.toggle_send(t, b),
            PresetName(n) => self.preset_name = n,
            SavePreset(target) => {
                let (Some((pl, _)), Some(base)) = (self.preset_plugin(target), Self::preset_base()) else { return };
                let Some(state) = pl.save_state() else {
                    self.editor.status = format!("{} cannot save its state", pl.info.name);
                    return;
                };
                match presets::save(&base, &pl.info.id, &self.preset_name, &state) {
                    Ok(name) => {
                        self.editor.status = format!("Preset {name} saved for {}", pl.info.name);
                        self.preset_name.clear();
                    }
                    Err(e) => self.editor.status = e,
                }
            }
            LoadPreset(target, name) => {
                let (Some((pl, node)), Some(base)) = (self.preset_plugin(target), Self::preset_base()) else { return };
                match presets::load(&base, &pl.info.id, &name) {
                    Ok(state) if pl.load_state(&state) => {
                        if let Some(n) = node {
                            self.editor.project.plugin_state.insert(n, std::sync::Arc::new(state));
                            self.editor.modified = true;
                        }
                        self.editor.status = format!("{}: {name}", pl.info.name);
                    }
                    Ok(_) => self.editor.status = format!("{} did not accept the preset {name}", pl.info.name),
                    Err(e) => self.editor.status = e,
                }
            }
            DeletePreset(target, name) => {
                let (Some((pl, _)), Some(base)) = (self.preset_plugin(target), Self::preset_base()) else { return };
                if let Err(e) = presets::delete(&base, &pl.info.id, &name) {
                    self.editor.status = e;
                }
            }
            AutoShow(t, label) => {
                match mix::auto_options(&self.editor.project, &self.playback, t).into_iter().find(|o| o.0 == label).and_then(|o| o.1) {
                    Some(target) => {
                        self.auto_shown.insert(t, target);
                        self.editor.status = "Automation: click to add a point, drag to move, right-click to delete".into();
                    }
                    None => {
                        self.auto_shown.remove(&t);
                    }
                }
            }
            DynamicsCc(t, label) => {
                let cc = match label.as_str() {
                    l if l.starts_with("CC1 ") => Some(1),
                    l if l.starts_with("CC11") => Some(11),
                    _ => None,
                };
                self.editor.set_track(t, "Dynamics controller", false, |x| x.dynamics_cc = cc);
            }
            TechAdd(t) => self.editor.set_track(t, "Playing technique", false, |x| {
                x.techniques.push(corinzia_model::TechniqueSwitch { technique: if x.techniques.is_empty() { "ord.".into() } else { "pizz.".into() }, action: corinzia_model::SwitchAction::Keyswitch(24 + x.techniques.len() as u8) })
            }),
            TechRemove(t, i) => self.editor.set_track(t, "Playing technique", false, |x| {
                if i < x.techniques.len() {
                    x.techniques.remove(i);
                }
            }),
            TechEdit(t, i, field, v) => self.editor.set_track(t, "Playing technique", false, |x| {
                use corinzia_model::SwitchAction::*;
                let Some(line) = x.techniques.get_mut(i) else { return };
                let num = |s: &str| s.trim_start_matches("CC").split_whitespace().next().and_then(|n| n.parse::<u8>().ok());
                match field {
                    mix::TechField::Technique => line.technique = v,
                    mix::TechField::Kind => {
                        line.action = match v.as_str() {
                            "Program" => Program(0),
                            "Controller" => Cc(32, 0),
                            "Channel" => Channel(0),
                            _ => Keyswitch(24),
                        }
                    }
                    mix::TechField::Value => {
                        line.action = match line.action {
                            Keyswitch(k) => Keyswitch((0..128u8).find(|&n| mix::key_name(n) == v).unwrap_or(k)),
                            Program(p) => Program(presets::program_of(&v).unwrap_or(p)),
                            Cc(c, w) => Cc(num(&v).unwrap_or(c).min(127), w),
                            Channel(c) => Channel(num(&v).map(|n| n.clamp(1, 16) - 1).unwrap_or(c)),
                        }
                    }
                    mix::TechField::Value2 => {
                        if let Cc(c, w) = line.action {
                            line.action = Cc(c, num(&v).unwrap_or(w).min(127));
                        }
                    }
                }
            }),
            Program(t, label) => {
                let program = presets::program_of(&label);
                self.editor.set_track(t, "MIDI program", false, |x| x.program = program);
            }
            SendLevel(t, i, db) => {
                if self.editor.project.track(t).and_then(|x| x.sends.get(i)).is_some_and(|s| s.gain_db != db) {
                    self.editor.set_send(t, i, db);
                    self.playback.live_levels(&self.editor.project);
                }
            }
            // The gesture is over: the next one is its own undo step.
            Released => {
                self.editor.end_gesture();
                // A clip's gain or name changed live: playback catches up.
                if std::mem::take(&mut self.clip_dragged) {
                    self.editor.music_changed();
                }
            }
            Solo(t) if self.editor.project.bus(t).is_some() => self.editor.set_bus(t, "Solo", false, |b| b.solo = !b.solo),
            Mute(t) => self.editor.set_track(t, "Mute", false, |x| x.mute = !x.mute),
            Solo(t) => self.editor.set_track(t, "Solo", false, |x| x.solo = !x.solo),
            SendPre(t, i) => self.editor.toggle_send_pre(t, i),
            BusOutput(b, label) => {
                let to = self.editor.project.buses.iter().find(|x| x.name == label && x.id != b).map(|x| x.id);
                self.editor.set_bus_output(b, to);
            }
            Color(t, c) => self.editor.set_track(t, "Track colour", false, |x| x.color = c),
            Folder(t, f) => self.editor.set_track(t, "Folder", true, |x| x.folder = f.trim_start().to_string()),
            Assign(t, i) => {
                let Some(info) = self.playback.plugins.get(i).cloned() else { return };
                if info.bundle.as_os_str() == corinzia_audio::host::BUILTIN {
                    self.editor.set_track_plugin(t, None);
                } else {
                    let bundle = info.bundle.to_string_lossy().into_owned();
                    self.editor.set_track_plugin(t, Some((&info.name, &bundle, &info.id)));
                }
            }
            Rescan => self.playback.rescan(),
        }
    }

    fn layout_msg(&mut self, m: LayoutMsg) {
        let Some(d) = self.layout.as_mut() else { return };
        d.error = None;
        let k = usize::from(d.parts);
        let f = &mut d.fields[k];
        match m {
            LayoutMsg::Tab(parts) => d.parts = parts,
            LayoutMsg::Size(name) => {
                if let Some((_, w, h)) = corinzia_model::PAGE_SIZES.iter().find(|p| p.0 == name) {
                    (f.width, f.height) = (w.to_string(), h.to_string());
                }
                f.size = name;
            }
            LayoutMsg::Field(which, v) => {
                let slot = match which {
                    LayoutField::Width | LayoutField::Height => {
                        f.size = "Custom".into();
                        if matches!(which, LayoutField::Width) { &mut f.width } else { &mut f.height }
                    }
                    LayoutField::Staff => &mut f.staff,
                    LayoutField::MarginY => &mut f.margin_y,
                    LayoutField::MarginX => &mut f.margin_x,
                    LayoutField::Spacing => &mut f.spacing,
                    LayoutField::StaffDistance => &mut f.staff_distance,
                    LayoutField::SystemGap => &mut f.system_gap,
                    LayoutField::Bars => &mut f.bars,
                };
                *slot = v;
            }
            LayoutMsg::Landscape(on) => f.landscape = on,
            LayoutMsg::PageNumbers(on) => f.page_numbers = on,
            LayoutMsg::Condense(on) => f.condense = on,
            LayoutMsg::Text(i, v) => d.texts[i] = v,
            LayoutMsg::Defaults => *f = LayoutFields::of(&if d.parts { corinzia_model::Layout::parts() } else { corinzia_model::Layout::default() }),
            LayoutMsg::Cancel => self.layout = None,
            LayoutMsg::Apply => {
                let (score, parts) = match (d.fields[0].layout(), d.fields[1].layout()) {
                    (Ok(a), Ok(b)) => (a, b),
                    (Err(e), _) => {
                        d.error = Some(format!("Score: {e}"));
                        return;
                    }
                    (_, Err(e)) => {
                        d.error = Some(format!("Parts: {e}"));
                        return;
                    }
                };
                let texts = d.texts.clone();
                self.layout = None;
                self.editor.set_layout(score, parts, texts);
            }
        }
    }

    /// Opens a MIDI input port (none for an empty id).
    fn open_midi(&mut self, id: &str) {
        self.midi = None;
        self.midi_held.clear();
        if id.is_empty() {
            return;
        }
        match midi_in::Input::open(id) {
            Ok(i) => self.midi = Some(i),
            Err(e) => self.editor.status = format!("MIDI input: {e}"),
        }
    }

    /// Keys from the MIDI keyboard: in note input they are entered (keys
    /// held together make a chord); they always sound on the staff's track.
    fn midi_poll(&mut self) {
        let Some(input) = &self.midi else { return };
        let msgs: Vec<midi_in::MidiMsg> = input.rx.try_iter().collect();
        for m in msgs {
            match m {
                midi_in::MidiMsg::On { key, .. } => {
                    if self.editor.input_mode && self.tab == Tab::Score {
                        let chord = !self.midi_held.is_empty();
                        self.editor.enter_midi_key(key, chord);
                    }
                    self.midi_held.push(key);
                    self.sound_key(key);
                }
                midi_in::MidiMsg::Off { key } => self.midi_held.retain(|k| *k != key),
            }
        }
    }

    /// Plays a key on the instrument of the caret's staff.
    fn sound_key(&mut self, key: u8) {
        let ed = &self.editor;
        let Some(m) = ed.movement() else { return };
        let Some(inst) = m.lanes.get(ed.caret_lane).and_then(|l| l.instrument_at(ed.caret)) else { return };
        let Some(track) = ed.project.instrument(inst).and_then(|i| i.track) else { return };
        let channel = ed.project.track(track).map(|t| t.midi_channel as i16).unwrap_or(0);
        self.sync();
        self.playback.preview(track, channel, &[key as i32]);
    }

    /// The Preferences card: edit a copy, apply it (saved; the audio output
    /// restarts when the device, rate or buffer change).
    fn prefs_msg(&mut self, m: PrefsMsg) {
        if let PrefsMsg::Open = m {
            self.prefs_dialog = Some(PrefsDialog { paths: self.prefs.plugin_paths.join("; "), draft: self.prefs.clone(), devices: playback::Playback::devices(), midi_ports: midi_in::ports(), error: None });
            return;
        }
        let Some(d) = self.prefs_dialog.as_mut() else { return };
        d.error = None;
        match m {
            PrefsMsg::Open => {}
            PrefsMsg::Theme(t) => {
                d.draft.theme = t;
                theme::set(&d.draft.theme); // seen at once
            }
            PrefsMsg::Device(label) => d.draft.output_device = d.devices.iter().find(|x| x.1 == label).map(|x| x.0.clone()).unwrap_or_default(),
            PrefsMsg::Rate(r) => d.draft.sample_rate = r,
            PrefsMsg::Midi(label) => d.draft.midi_input = d.midi_ports.iter().find(|x| x.1 == label).map(|x| x.0.clone()).unwrap_or_default(),
            PrefsMsg::Buffer(b) => d.draft.buffer = b,
            PrefsMsg::Paths(t) => d.paths = t,
            PrefsMsg::DefaultPaths(on) => d.draft.default_plugin_paths = on,
            PrefsMsg::Cancel => {
                theme::set(&self.prefs.theme);
                self.prefs_dialog = None;
            }
            PrefsMsg::Apply => {
                let mut next = d.draft.clone();
                next.plugin_paths = d.paths.split(';').map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect();
                let next = next.sane();
                if !cfg!(test) {
                    if let Err(e) = next.save() {
                        d.error = Some(format!("Could not save the preferences: {e}"));
                        return;
                    }
                }
                self.prefs_dialog = None;
                theme::set(&next.theme);
                if self.prefs.audio_differs(&next) {
                    // A new engine at the new rate: plugins come back with their state.
                    self.playback.stop();
                    self.playback.pool.store_states(&mut self.editor.project);
                    let live = LIVE_AUDIO.load(std::sync::atomic::Ordering::Relaxed);
                    let plugins = std::mem::take(&mut self.playback.plugins);
                    self.playback = playback::Playback::new(live, &next);
                    self.playback.plugins = plugins;
                    self.playback.sync(&self.editor.project, self.editor.music_revision, self.editor.revision, self.path.as_deref().and_then(|p| p.parent()));
                }
                if next.midi_input != self.prefs.midi_input || (self.midi.is_none() && !next.midi_input.is_empty()) {
                    self.open_midi(&next.midi_input);
                }
                let paths = next.plugin_search_paths();
                if paths != self.playback.search_paths {
                    self.playback.search_paths = paths;
                    self.playback.scan();
                }
                self.editor.status = match &self.playback.problem {
                    Some(p) => format!("Preferences saved · {p}"),
                    None => format!("Preferences saved · {} Hz, {} frames", next.sample_rate, next.buffer),
                };
                self.prefs = next;
            }
        }
    }

    /// The Players card: open it on the score's players, edit the list,
    /// apply it as one undo step.
    fn players_msg(&mut self, m: PlayersMsg) {
        if let PlayersMsg::Open = m {
            let p = &self.editor.project;
            let lanes = self.editor.movement().map(|m| m.lanes.clone()).unwrap_or_default();
            let rows = p
                .players
                .iter()
                .map(|pl| {
                    // "Name / Abbr." (the abbreviation names later systems).
                    let instruments: Vec<String> = pl
                        .instruments
                        .iter()
                        .filter_map(|i| p.instrument(*i))
                        .map(|i| if i.engraver_name.is_empty() { i.name.clone() } else { format!("{} / {}", i.name, i.engraver_name) })
                        .collect();
                    let staves = lanes.iter().filter(|l| l.player == pl.id).count();
                    let detail = if staves == 1 { "1 staff".to_string() } else { format!("{staves} staves") };
                    PlayerRow { player: Some(pl.id), instruments: instruments.join(", "), detail }
                })
                .collect();
            self.players = Some(PlayersDialog { rows, add: String::new(), error: None });
            self.help = false;
            self.export_menu = false;
            return;
        }
        let Some(d) = self.players.as_mut() else { return };
        d.error = None;
        let add = |d: &mut PlayersDialog, text: &str| match files::parse_instruments(text) {
            Ok(list) => {
                for (name, _, clef) in list {
                    let staves = clef.split('+').count();
                    let detail = format!("new · {}", if staves == 1 { "1 staff".to_string() } else { format!("{staves} staves") });
                    d.rows.push(PlayerRow { player: None, instruments: name, detail });
                }
                true
            }
            Err(e) => {
                d.error = Some(e);
                false
            }
        };
        match m {
            PlayersMsg::Open => {}
            PlayersMsg::Move(i, by) => {
                let j = i as isize + by;
                if i < d.rows.len() && j >= 0 && (j as usize) < d.rows.len() {
                    d.rows.swap(i, j as usize);
                }
            }
            PlayersMsg::Remove(i) => {
                if i < d.rows.len() {
                    d.rows.remove(i);
                }
            }
            PlayersMsg::Edit(i, v) => {
                if let Some(r) = d.rows.get_mut(i) {
                    r.instruments = v;
                }
            }
            PlayersMsg::AddInput(v) => d.add = v,
            PlayersMsg::Add => {
                let text = d.add.clone();
                if add(d, &text) {
                    d.add.clear();
                }
            }
            PlayersMsg::Quick(name) => {
                add(d, name);
            }
            PlayersMsg::Cancel => self.players = None,
            PlayersMsg::Apply => {
                let mut slots = Vec::new();
                for (k, r) in d.rows.iter().enumerate() {
                    match files::parse_instrument_list(&r.instruments) {
                        Ok(instruments) => slots.push(match r.player {
                            Some(id) => editor::PlayerSlot::Keep { id, instruments },
                            None => editor::PlayerSlot::New { instruments },
                        }),
                        Err(e) => {
                            d.error = Some(format!("Player {}: {e}", k + 1));
                            return;
                        }
                    }
                }
                match self.editor.set_players(&slots) {
                    Ok(()) => {
                        self.players = None;
                        // A part whose player went shows the full score.
                        if self.part.is_some_and(|p| self.editor.project.player(p).is_none()) {
                            self.part = None;
                        }
                    }
                    Err(e) => d.error = Some(e),
                }
            }
        }
    }

    /// Shows a track's plugin editor, or closes it when open.
    fn toggle_editor(&mut self, track: corinzia_model::Id) {
        if self.playback.pool.editor_is_open(track) {
            self.playback.pool.close_editor(track);
            return;
        }
        let Some(t) = self.editor.project.track(track) else { return };
        let title = format!("{} — {}", t.name, self.playback.plugin_name(track));
        if let Err(e) = self.playback.pool.open_editor(track, &title) {
            self.editor.status = e;
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
        // A slur's curve selects the slur.
        if let Some(id) = hit.filter(|_| !ed.input_mode) {
            if ed.select_slur(id) {
                return;
            }
        }
        match (hit, shift) {
            (Some(id), true) => ed.select_elements(&[id], true),
            (Some(id), false) => ed.select(Some(id)),
            (None, _) => {
                if let (Some(p), true) = (at, ed.input_mode) {
                    ed.caret_lane = self.staff_lanes.get(p.part).copied().unwrap_or(p.part);
                }
                // Empty space inside a bar's staff selects the bar (Shift
                // extends the passage); the paper around it deselects.
                if let Some(p) = at.filter(|p| !ed.input_mode && (-1..=9).contains(&p.step_below_top)) {
                    let lane = self.staff_lanes.get(p.part).copied().unwrap_or(p.part);
                    let pos = Frac::new((p.quarter * 1024.0).round() as i64, 4096);
                    ed.select_bar(lane, pos, shift);
                    return;
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
            Message::Mix(mix::MixMsg::AddAudio(t)) => return self.pick_file_for(browser::Purpose::Audio(t), false),
            Message::Mix(m) => self.mix(m),
            Message::Arrange(e) => match e {
                arrange::ArrangeEvent::ClipPress { track, clip, copy } => {
                    let id = clip;
                    self.selected_clip = Some((track, id));
                    self.clip_dragged = false;
                    self.clip_copy = copy;
                    // Its length made explicit: trims and fades measure from it.
                    self.clip_origin = self.editor.clip(track, id).cloned().map(|mut c| {
                        c.length = arrange::clip_length(&c, self.playback.pool.media.loaded(&c.file).as_deref());
                        c
                    });
                }
                arrange::ArrangeEvent::ClipDrag { edit, q } => self.drag_clip(edit, q),
                arrange::ArrangeEvent::Cycle { from, to } => {
                    if let Some(m) = self.editor.movement().map(|m| m.id) {
                        self.playback.set_cycle(Some((m, from, to)));
                    }
                }
                arrange::ArrangeEvent::ToggleFolder(name) => {
                    if !self.folded.remove(&name) {
                        self.folded.insert(name);
                    }
                }
                arrange::ArrangeEvent::ClipRelease => {
                    self.editor.end_gesture();
                    if std::mem::take(&mut self.clip_dragged) {
                        self.editor.music_changed();
                    }
                    self.clip_origin = None;
                }
                arrange::ArrangeEvent::Select(id) => {
                    self.selected_clip = None;
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
                arrange::ArrangeEvent::AutoAdd { lane, q, value } => {
                    if let Some(t) = self.arrange.lanes.get(lane).and_then(|l| self.auto_shown.get(&l.track)).copied() {
                        let at = Frac::new((q * 16.0).round() as i64, 64);
                        self.editor.add_auto_point(t, at, value);
                    }
                }
                arrange::ArrangeEvent::AutoMove { lane, index, q, value } => {
                    if let Some(t) = self.arrange.lanes.get(lane).and_then(|l| self.auto_shown.get(&l.track)).copied() {
                        let at = Frac::new((q * 16.0).round() as i64, 64);
                        self.editor.move_auto_point(t, index, at, value);
                    }
                }
                arrange::ArrangeEvent::AutoDelete { lane, index } => {
                    if let Some(t) = self.arrange.lanes.get(lane).and_then(|l| self.auto_shown.get(&l.track)).copied() {
                        self.editor.delete_auto_point(t, index);
                    }
                }
                arrange::ArrangeEvent::AutoRelease => self.editor.end_gesture(),
                arrange::ArrangeEvent::AutoHold { lane, index } => {
                    if let Some(t) = self.arrange.lanes.get(lane).and_then(|l| self.auto_shown.get(&l.track)).copied() {
                        self.editor.toggle_auto_hold(t, index);
                    }
                }
            },
            Message::ZoomBy(f) => self.zoom = (self.zoom * f).clamp(0.25, 8.0),
            Message::Resized(size) => self.window = size,
            Message::ExportMenu(open) => self.export_menu = open,
            Message::SetPart(p) => {
                self.part = p;
                self.engraved = None;
            }
            Message::Dropped(path) if path.extension().and_then(|e| e.to_str()).is_some_and(|e| browser::AUDIO_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str())) => {
                // A sound file: onto the selected audio track, else a new one.
                let track = self.strip.filter(|t| self.editor.project.track(*t).is_some_and(|t| t.audio));
                self.add_audio_file(track, &path);
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
                // Another movement's page: it becomes the one edited.
                let clicked = at.map(|p| self.page_movements.get(p.page).copied()).unwrap_or_else(|| hit.and_then(|id| self.editor.movement_of(id)));
                if let Some(mi) = clicked.filter(|mi| *mi != self.editor.movement) {
                    self.editor.switch_movement(mi);
                    self.sync();
                }
                self.click(hit, at, shift)
            }
            Message::Canvas(ViewEvent::Marquee { ids, shift }) => self.editor.select_elements(&ids, shift),
            Message::Canvas(ViewEvent::SlurDrag { handle, at }) => {
                self.editor.drag_slur(handle, Frac::new((at.quarter * 1024.0).round() as i64, 4096));
            }
            Message::Canvas(ViewEvent::SlurDragEnd) => self.editor.end_gesture(),
            Message::Canvas(ViewEvent::Context { target, at, screen }) => {
                self.export_menu = false;
                self.editor.status.clear();
                let Some(p) = at else { return Task::none() };
                if let Some(mi) = self.page_movements.get(p.page).copied().filter(|mi| *mi != self.editor.movement) {
                    self.editor.switch_movement(mi);
                    self.sync();
                }
                let kind = match target {
                    score_view::ContextTarget::Clef => PopoverKind::Clef,
                    score_view::ContextTarget::Key => PopoverKind::Key,
                    score_view::ContextTarget::Time => PopoverKind::Time,
                    score_view::ContextTarget::Barline => PopoverKind::Barline,
                };
                let lane = self.staff_lanes.get(p.part).copied().unwrap_or(p.part);
                self.editor.target_mark(lane, Frac::new((p.quarter * 1024.0).round() as i64, 4096), kind);
                self.sync();
                self.popover = Some(Popover { kind, input: String::new(), error: None, target: None, anchor: Some(screen) });
                return iced::widget::operation::focus(POPOVER_INPUT);
            }
            Message::Canvas(ViewEvent::DoubleClick { hit }) => {
                if let Some(id) = hit.filter(|_| !self.editor.input_mode) {
                    return self.edit_on_page(id);
                }
            }
            Message::Key(iced::keyboard::Event::KeyPressed { key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape), .. }) if self.bounce.is_some() => {
                self.bounce = None;
                self.editor.status = "Audio export stopped".into();
            }
            Message::Key(iced::keyboard::Event::KeyPressed { key, modifiers, .. }) => {
                if self.popover.is_some() || self.new_score.is_some() || self.players.is_some() || self.layout.is_some() || self.prefs_dialog.is_some() || self.confirm.is_some() || self.browser.is_some() {
                    if matches!(key, iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape)) {
                        if self.confirm.take().is_none() && self.browser.take().is_none() {
                            self.popover = None;
                            self.new_score = None;
                            self.players = None;
                            self.layout = None;
                            self.prefs_dialog = None;
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
                let commits = p.target.is_none()
                    && match p.kind {
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
                    let done = match p.target {
                        Some(id) => self.editor.edit_item(id, &p.input),
                        None => self.editor.apply_popover(p.kind, &p.input),
                    };
                    match done {
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
            Message::Players(m) => self.players_msg(m),
            Message::Layout(m) => self.layout_msg(m),
            Message::Prefs(m) => self.prefs_msg(m),
            Message::MidiPoll => self.midi_poll(),
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
                Message::Open => return self.pick_file(false),
                Message::Import => return self.pick_file(true),
                Message::NewCreate => self.create_new(),
                other => return Task::done(other),
            },
            Message::Open => return Task::done(Message::Guarded(Box::new(Message::Open))),
            Message::Import => return Task::done(Message::Guarded(Box::new(Message::Import))),
            Message::Picked { purpose, path, failed } => {
                if failed {
                    // No system dialog here (no xdg-desktop-portal file
                    // chooser): Corinzia's own browser instead.
                    return self.open_browser(purpose);
                }
                return Task::done(match purpose {
                    browser::Purpose::Open => Message::Opened(path),
                    browser::Purpose::Export(f) => Message::ExportTo(f, path),
                    browser::Purpose::Audio(t) => Message::AudioChosen(t, path),
                });
            }
            Message::Browser(msg) => {
                let Some(b) = self.browser.as_mut() else { return Task::none() };
                let purpose = b.purpose;
                match b.update(msg) {
                    browser::Outcome::Stay => {}
                    browser::Outcome::Cancel => self.browser = None,
                    browser::Outcome::Chosen(path) => {
                        self.browser = None;
                        return Task::done(match purpose {
                            browser::Purpose::Open => Message::Opened(Some(path)),
                            browser::Purpose::Export(f) => Message::ExportTo(f, Some(path)),
                            browser::Purpose::Audio(t) => Message::AudioChosen(t, Some(path)),
                        });
                    }
                }
            }
            Message::Opened(None) | Message::SaveTo(None) | Message::ExportTo(_, None) | Message::AudioChosen(_, None) => return Task::none(),
            Message::AudioChosen(track, Some(path)) => self.add_audio_file(Some(track), &path),
            Message::Opened(Some(path)) => self.open(path),
            Message::SaveTo(Some(path)) => {
                let path = if path.extension().is_none() { path.with_extension("project") } else { path };
                self.save(path);
            }
            Message::Export(format) => {
                self.export_menu = false;
                return self.pick_file_for(browser::Purpose::Export(format), false);
            }
            Message::ExportTo(files::Format::Audio, Some(path)) => {
                let path = if path.extension().is_none() { path.with_extension("wav") } else { path };
                self.start_bounce(path);
            }
            Message::ExportTo(format, Some(path)) => {
                let path = if path.extension().is_none() { path.with_extension(format.extension()) } else { path };
                self.editor.status = match files::export_to(&self.editor.project, self.editor.movement, format, self.editor.transposed, &path) {
                    Ok(()) => format!("Exported {} to {}", format.label(), path.display()),
                    Err(e) => format!("Export failed: {e}"),
                };
            }
            Message::PluginTick => {
                self.playback.tick();
                return Task::none();
            }
            Message::Frame => {
                self.playback.tick();
                self.bounce_step();
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
        let flat = |label: &'static str, enabled: bool, m: Message| button(text(label).size(13)).padding([5, 7]).style(theme::flat(false)).on_press_maybe(enabled.then_some(m));
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
            panels::tip(
                button(text("⟲").size(14)).padding([4, 8]).style(theme::tool(self.playback.cycling)).on_press(Message::Act(Action::Cycle)),
                "Cycle: the selected passage, or the cycle on or off (Alt+L; Shift+drag along the arrange view's ruler to set it)",
            ),
            panels::tip(flat("↶", ed.can_undo(), Message::Act(Action::Undo)), self.undo_tip.as_str()),
            panels::tip(flat("↷", ed.can_redo(), Message::Act(Action::Redo)), "Redo (Ctrl+Shift+Z)"),
        ]
        .spacing(6)
        .align_y(iced::Alignment::Center);
        if self.tab == Tab::Score {
            toolbar = toolbar
                .push(rule())
                .push(panels::tip(
                    button(text("✎  Note input").size(13)).padding([5, 10]).style(theme::tool(ed.input_mode)).on_press(Message::Act(Action::SetInput(!ed.input_mode))),
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
            .push(panels::tip(flat("⚙", true, Message::Prefs(PrefsMsg::Open)), "Preferences: theme, audio output, plugin folders (Ctrl+,)"))
            .push(panels::tip(flat("Players", true, Message::Players(PlayersMsg::Open)), "Add, remove or reorder players (the instrumentation)"))
            .push(panels::tip(flat("New", true, Message::Act(Action::New)), "New score (Ctrl+N)"))
            .push(panels::tip(flat("Open", true, Message::Act(Action::Open)), "Open a project or MusicXML file (Ctrl+O)"))
            .push(panels::tip(flat("Import", true, Message::Import), "Import MusicXML (.musicxml, .xml, .mxl) as a new score"))
            .push(panels::tip(
                flat("Save", self.saving_enabled, Message::Act(Action::Save)),
                if self.saving_enabled { "Save (Ctrl+S)" } else { "Saving is turned off in this preview build" },
            ))
            .push(button(text("Export ▾").size(13)).padding([5, 7]).style(theme::flat(self.export_menu)).on_press(Message::ExportMenu(!self.export_menu)));
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
            playhead: self.playback.playhead().and_then(|(id, q)| {
                let mi = ed.project.movements.iter().position(|m| m.id == id)?;
                (self.all_movements || mi == ed.movement).then(|| (q, self.pages_of(mi)))
            }),
            active: self.pages_of(ed.movement),
            slur: ed.selected_slur(),
            ghost: ed.input_mode.then(|| {
                use corinzia_edit::BaseDuration as B;
                use corinzia_layout::font::Glyph;
                let glyph = match ed.spec.base {
                    B::Whole => Glyph::NoteheadWhole,
                    B::Half => Glyph::NoteheadHalf,
                    _ => Glyph::NoteheadBlack,
                };
                let grid = corinzia_edit::DurationSpec { dots: 0, ..ed.spec }.value();
                score_view::Ghost { glyph: glyph.codepoint(), dots: ed.spec.dots, grid: quarters(grid) }
            }),
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
            let card = iced::widget::opaque(container(card).padding(12).style(theme::card));
            desk = desk.push(match p.anchor {
                // Just below what it is about (above it near the bottom),
                // kept inside the window.
                Some(a) => {
                    let (w, h) = (344.0, 130.0);
                    let room = self.window.height - 46.0 - 60.0;
                    let x = (a.x - 40.0).clamp(8.0, (self.window.width - w - 8.0).max(8.0));
                    let y = if a.y + 14.0 + h < room { a.y + 14.0 } else { (a.y - h - 40.0).max(8.0) };
                    container(card).padding(iced::Padding { top: y.max(8.0), left: x, right: 0.0, bottom: 0.0 }).width(Length::Fill).height(Length::Fill)
                }
                None => container(card).padding(16).width(Length::Fill).align_x(iced::Alignment::Center),
            });
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

        if let Some(d) = &self.prefs_dialog {
            desk = desk.push(container(iced::widget::opaque(container(prefs_card(d)).padding(20).style(theme::card))).padding([40, 16]).width(Length::Fill).align_x(iced::Alignment::Center));
        }
        if let Some(d) = &self.layout {
            desk = desk.push(container(iced::widget::opaque(container(layout_card(d)).padding(20).style(theme::card))).padding([40, 16]).width(Length::Fill).align_x(iced::Alignment::Center));
        }
        if let Some(d) = &self.players {
            // Anchored at the top, so its controls stay put as the list grows.
            desk = desk.push(container(iced::widget::opaque(container(players_card(d)).padding(20).style(theme::card))).padding([40, 16]).width(Length::Fill).align_x(iced::Alignment::Center));
        }
        if self.help {
            desk = desk.push(container(iced::widget::opaque(help_card())).center(Length::Fill));
        }
        if let Some(b) = &self.browser {
            let card = b.view().map(Message::Browser);
            desk = desk.push(container(iced::widget::opaque(container(card).padding(20).style(theme::card))).center(Length::Fill));
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
                    item("Audio (WAV)…", "the movement as it plays, mixed", files::Format::Audio),
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
        let side = container(column![tabs, panels::panel(ed, self.panel, self.all_movements).map(Message::Act)]).width(panels::WIDTH).height(Length::Fill).style(theme::panel);
        let vline = || container(Space::new()).width(1).height(Length::Fill).style(theme::hairline);
        let hline = || container(Space::new()).height(1).width(Length::Fill).style(theme::hairline);

        let left = if !ed.status.is_empty() { ed.status.clone() } else { ed.selection_label() };
        let middle = if ed.input_mode { ed.caret_label() } else if ed.status.is_empty() { String::new() } else { ed.selection_label() };
        let status = container(
            row![
                // The messages give way (clipped) to the controls on the right.
                container(
                    row![
                        text(left).size(12).wrapping(text::Wrapping::None).style(|_: &Theme| text::Style { color: Some(theme::palette().text) }),
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
                    curves: &self.arrange_curves,
                    clips: &self.arrange_clips,
                    selected_clip: self.selected_clip.map(|s| s.1),
                    cycle: self.playback.cycle.filter(|c| Some(c.0) == ed.movement().map(|m| m.id)).map(|c| (c.1, c.2, self.playback.cycling)),
                })
                .width(Length::Fill)
                .height(Length::Fill);
                column![
                    Element::from(arr).map(Message::Arrange),
                    hline(),
                    container(mix::view(&ed.project, &self.playback, self.strip, self.fx, mix::Presets { names: &self.preset_names, name: &self.preset_name }, &self.auto_shown, self.selected_clip).map(Message::Mix)).height(420).width(Length::Fill).style(theme::panel),
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
        let keys = if self.playback.editors_busy() { Subscription::batch([keys, Subscription::run(plugin_clock)]) } else { keys };
        let keys = if self.midi.is_some() { Subscription::batch([keys, Subscription::run(midi_clock)]) } else { keys };
        if self.screenshot.is_some() || self.playback.busy() || self.autosave_at.is_some() || self.bounce.is_some() {
            Subscription::batch([keys, iced::window::frames().map(|_| Message::Frame)])
        } else {
            keys
        }
    }
}

/// Ticks to read the MIDI keyboard (every 8 ms) while one is open.
fn midi_clock() -> impl iced::futures::Stream<Item = Message> {
    let (mut tx, rx) = iced::futures::channel::mpsc::channel(1);
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_millis(8));
        if let Err(e) = tx.try_send(Message::MidiPoll) {
            if e.is_disconnected() {
                break;
            }
        }
    });
    rx
}

/// Ticks for plugin editors (their timers, descriptors and windows), about
/// 60 a second, whatever the main window is doing. The thread ends when the
/// subscription is dropped.
fn plugin_clock() -> impl iced::futures::Stream<Item = Message> {
    let (mut tx, rx) = iced::futures::channel::mpsc::channel(1);
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_millis(16));
        if let Err(e) = tx.try_send(Message::PluginTick) {
            if e.is_disconnected() {
                break;
            }
        }
    });
    rx
}

/// A segmented control: one button per choice, the chosen one lit.
fn segmented(opts: Vec<(&'static str, bool, Message)>) -> Element<'static, Message> {
    let mut r = row![].spacing(2);
    for (label, on, m) in opts {
        r = r.push(button(text(label).size(13)).padding([3, 12]).style(theme::segment(on)).on_press(m));
    }
    container(r).padding(2).style(theme::trough).into()
}

/// The Layout card: the page of the score or of the parts, and the texts.
fn layout_card(d: &LayoutDialog) -> Element<'_, Message> {
    let bold = iced::Font { weight: iced::font::Weight::Bold, ..iced::Font::DEFAULT };
    let f = &d.fields[usize::from(d.parts)];
    let lm = |m: LayoutMsg| Message::Layout(m);
    let field = |label: &'static str, value: &str, which: LayoutField| {
        column![text(label).size(12).style(theme::dim), text_input("", value).on_input(move |v| lm(LayoutMsg::Field(which, v))).on_submit(lm(LayoutMsg::Apply)).padding(5).size(14)].spacing(3).width(Length::Fill)
    };
    let toggle = |label: &'static str, on: bool, m: LayoutMsg| button(text(label).size(13)).padding([5, 10]).style(theme::tool(on)).on_press(lm(m));
    let mut sizes: Vec<String> = corinzia_model::PAGE_SIZES.iter().map(|p| p.0.to_string()).collect();
    sizes.push("Custom".into());
    let size = iced::widget::pick_list(sizes, Some(f.size.clone()), |s| Message::Layout(LayoutMsg::Size(s))).text_size(13).padding([5, 9]).width(Length::Fill);
    let texts = ["Subtitle", "Lyricist", "Arranger", "Copyright"];
    let text_fields = column(texts.iter().enumerate().map(|(i, label)| {
        row![text(*label).size(12).style(theme::dim).width(80), text_input("", &d.texts[i]).on_input(move |v| lm(LayoutMsg::Text(i, v))).on_submit(lm(LayoutMsg::Apply)).padding(5).size(14)].spacing(8).align_y(iced::Alignment::Center).into()
    }))
    .spacing(6);
    let mut card = column![
        row![text("Layout").size(18).font(bold), panels::spacer(), segmented(vec![("Score", !d.parts, lm(LayoutMsg::Tab(false))), ("Parts", d.parts, lm(LayoutMsg::Tab(true)))])].align_y(iced::Alignment::Center),
        row![column![text("Page").size(12).style(theme::dim), size].spacing(3).width(Length::Fill), field("Width (mm)", &f.width, LayoutField::Width), field("Height (mm)", &f.height, LayoutField::Height)].spacing(10),
        row![toggle("Portrait", !f.landscape, LayoutMsg::Landscape(false)), toggle("Landscape", f.landscape, LayoutMsg::Landscape(true)), panels::spacer(), toggle("Page numbers", f.page_numbers, LayoutMsg::PageNumbers(!f.page_numbers)), { let e: iced::Element<'_, Message> = if d.parts { panels::spacer().into() } else { toggle("Condense", f.condense, LayoutMsg::Condense(!f.condense)).into() }; e }].spacing(6),
        row![field("Staff size (mm)", &f.staff, LayoutField::Staff), field("Margins top/bottom (mm)", &f.margin_y, LayoutField::MarginY), field("Margins sides (mm)", &f.margin_x, LayoutField::MarginX)].spacing(10),
        row![field("Note spacing (%)", &f.spacing, LayoutField::Spacing), field("Staff distance (spaces)", &f.staff_distance, LayoutField::StaffDistance), field("System gap (spaces)", &f.system_gap, LayoutField::SystemGap), field("Bars per system", &f.bars, LayoutField::Bars)].spacing(10),
        text("Breaks: Enter puts a system break before the selected bar, Ctrl+Enter a page break (in the part shown, or the full score).").size(11).style(theme::dim),
        text("Page texts").size(12).style(theme::dim),
        text_fields,
    ]
    .spacing(10)
    .width(600);
    if let Some(e) = &d.error {
        card = card.push(text(e).size(12).style(theme::danger));
    }
    card.push(
        row![
            button(text(if d.parts { "Parts defaults" } else { "Score defaults" }).size(13)).padding([6, 14]).style(theme::tool(false)).on_press(lm(LayoutMsg::Defaults)),
            panels::spacer(),
            button(text("Cancel").size(13)).padding([6, 14]).style(theme::tool(false)).on_press(lm(LayoutMsg::Cancel)),
            button(text("Apply").size(13)).padding([6, 14]).style(theme::primary).on_press(lm(LayoutMsg::Apply)),
        ]
        .spacing(8),
    )
    .into()
}

/// The Preferences card.
fn prefs_card(d: &PrefsDialog) -> Element<'_, Message> {
    let bold = iced::Font { weight: iced::font::Weight::Bold, ..iced::Font::DEFAULT };
    let pm = |m: PrefsMsg| Message::Prefs(m);
    let label = |t: &'static str| text(t).size(12).style(theme::dim);
    let themes: Vec<String> = theme::THEMES.iter().map(|t| t.0.to_string()).collect();
    let mut devices: Vec<String> = vec!["System default".into()];
    devices.extend(d.devices.iter().map(|x| x.1.clone()));
    let device = d.devices.iter().find(|x| x.0 == d.draft.output_device).map(|x| x.1.clone()).unwrap_or_else(|| if d.draft.output_device.is_empty() { "System default".into() } else { d.draft.output_device.clone() });
    let pick = |opts: Vec<String>, cur: String, f: fn(String) -> PrefsMsg| iced::widget::pick_list(opts, Some(cur), move |v| pm(f(v))).text_size(13).padding([5, 9]).width(Length::Fill);
    let mut midi_ports: Vec<String> = vec!["None".into()];
    midi_ports.extend(d.midi_ports.iter().map(|x| x.1.clone()));
    let midi_port = d.midi_ports.iter().find(|x| x.0 == d.draft.midi_input).map(|x| x.1.clone()).unwrap_or_else(|| if d.draft.midi_input.is_empty() { "None".into() } else { d.draft.midi_input.clone() });
    let rates: Vec<String> = prefs::SAMPLE_RATES.iter().map(|r| r.to_string()).collect();
    let buffers: Vec<String> = prefs::BUFFERS.iter().map(|r| r.to_string()).collect();
    let mut card = column![
        text("Preferences").size(18).font(bold),
        column![label("Theme"), pick(themes, d.draft.theme.clone(), PrefsMsg::Theme)].spacing(3),
        column![label("Audio output"), pick(devices, device, PrefsMsg::Device)].spacing(3),
        column![label("MIDI keyboard (note input)"), pick(midi_ports, midi_port, PrefsMsg::Midi)].spacing(3),
        row![
            column![label("Sample rate (Hz)"), pick(rates, d.draft.sample_rate.to_string(), |v| PrefsMsg::Rate(v.parse().unwrap_or(48000)))].spacing(3).width(Length::Fill),
            column![label("Buffer (frames)"), pick(buffers, d.draft.buffer.to_string(), |v| PrefsMsg::Buffer(v.parse().unwrap_or(2048)))].spacing(3).width(Length::Fill),
        ]
        .spacing(10),
        column![
            label("More plugin folders (separated by ;)"),
            text_input("/path/to/plugins", &d.paths).on_input(move |v| pm(PrefsMsg::Paths(v))).padding(5).size(14),
            button(text("Also search the usual folders (and CLAP_PATH, VST3_PATH)").size(13)).padding([5, 10]).style(theme::tool(d.draft.default_plugin_paths)).on_press(pm(PrefsMsg::DefaultPaths(!d.draft.default_plugin_paths))),
        ]
        .spacing(5),
        text("Changing the output, rate or buffer restarts the audio; plugins keep their settings.").size(11).style(theme::dim),
    ]
    .spacing(10)
    .width(520);
    if let Some(e) = &d.error {
        card = card.push(text(e).size(12).style(theme::danger));
    }
    card.push(
        row![
            panels::spacer(),
            button(text("Cancel").size(13)).padding([6, 14]).style(theme::tool(false)).on_press(pm(PrefsMsg::Cancel)),
            button(text("Apply").size(13)).padding([6, 14]).style(theme::primary).on_press(pm(PrefsMsg::Apply)),
        ]
        .spacing(8),
    )
    .into()
}

/// The Players card: the list (reorder, remove), and instruments to add.
fn players_card(d: &PlayersDialog) -> Element<'_, Message> {
    let bold = iced::Font { weight: iced::font::Weight::Bold, ..iced::Font::DEFAULT };
    let small = |label: &'static str, hint: &'static str, m: Option<PlayersMsg>| {
        let mut b = button(text(label).size(12)).padding([2, 7]).style(theme::flat(false));
        if let Some(m) = m {
            b = b.on_press(Message::Players(m));
        }
        panels::tip(b, hint)
    };
    let n = d.rows.len();
    let rows = column(d.rows.iter().enumerate().map(|(i, r)| {
        row![
            text(format!("{}", i + 1)).size(12).style(theme::dim).width(22),
            column![
                text_input("Instrument, then doublings: Flute, Piccolo", &r.instruments).on_input(move |v| Message::Players(PlayersMsg::Edit(i, v))).on_submit(Message::Players(PlayersMsg::Apply)).padding([3, 6]).size(14),
                text(&r.detail).size(11).style(theme::dim)
            ]
            .spacing(2)
            .width(Length::Fill),
            small("↑", "Move up", (i > 0).then_some(PlayersMsg::Move(i, -1))),
            small("↓", "Move down", (i + 1 < n).then_some(PlayersMsg::Move(i, 1))),
            small("✕", "Remove the player and its music", (n > 1).then_some(PlayersMsg::Remove(i))),
        ]
        .spacing(4)
        .align_y(iced::Alignment::Center)
        .into()
    }))
    .spacing(6);
    let quick = iced::widget::Row::with_children(QUICK_ADD.iter().map(|name| button(text(*name).size(12)).padding([3, 8]).style(theme::segment(false)).on_press(Message::Players(PlayersMsg::Quick(name))).into())).spacing(4).wrap();
    // Add first, the list below: the buttons stay put while the list grows.
    let mut card = column![
        text("Players").size(18).font(bold),
        text("Add").size(12).style(theme::dim),
        quick,
        row![
            text_input("Instruments to add: \"Flute, 2 Violin\"; any name", &d.add).id(PLAYERS_ADD).on_input(|v| Message::Players(PlayersMsg::AddInput(v))).on_submit(Message::Players(PlayersMsg::Add)).padding(6).size(14),
            button(text("Add").size(13)).padding([6, 14]).style(theme::tool(false)).on_press(Message::Players(PlayersMsg::Add)),
        ]
        .spacing(8),
    ]
    .spacing(10)
    .width(480);
    if let Some(e) = &d.error {
        card = card.push(text(e).size(12).style(theme::danger));
    }
    card.push(text("In the score, top to bottom. Retype an instrument to change it, or its short name after \"/\"; add doublings after a comma (Flute, Piccolo). Removing a player removes its music.").size(12).style(theme::dim))
        .push(iced::widget::scrollable(rows).height(Length::Fixed((n.max(1) as f32 * 52.0).min(400.0))))
        .push(
        row![
            panels::spacer(),
            button(text("Cancel").size(13)).padding([6, 14]).style(theme::tool(false)).on_press(Message::Players(PlayersMsg::Cancel)),
            button(text("Apply").size(13)).padding([6, 14]).style(theme::primary).on_press(Message::Players(PlayersMsg::Apply)),
        ]
        .spacing(8),
    )
    .into()
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
                ("Click its curve", "Select a slur; drag an end to a note"),
                ("Shift+← →  Alt+← →", "Slur's end, start by a note"),
                ("X", "Flip a slur"),
                ("Double-click", "Retype a title, tempo, text, dynamic…"),
                ("H  Shift+H", "Crescendo, diminuendo"),
                ("Shift+K  M  T", "Key, time, tempo"),
                ("Shift+C", "Clef; 8va, 8vb, loco"),
                ("Shift+I", "Instrument change (a doubling)"),
                ("Ctrl+Shift+T", "Transpose: up M2, to Eb…"),
                ("Ctrl+F", "Select only: top, voice 2, above C5, F#…"),
                ("Enter  Ctrl+Enter", "System break, page break before the bar"),
                ("Ctrl+Shift+A", "Select similar"),
                ("Shift+B  R", "Barline; ending, segno, D.C.…"),
                ("Shift+X  P  O", "Expression, technique (ped), ornament"),
                ("Shift+D", "Dynamic: pp, mf, sfz, fp…"),
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

#[cfg(test)]
mod tests {
    use super::*;
    use corinzia_layout::display::Item;
    use crate::editor::SlurHandle;
    use corinzia_model::dynamic_index;

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
        let _ = a.update(Message::PopoverInput(text.into()));
        let _ = a.update(Message::PopoverSubmit);
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

    #[test]
    fn clefs_keys_and_barlines_change_by_right_click() {
        use corinzia_layout::font::Glyph;
        let mut a = app(View::Page);
        // A barline inside the first system (not its first bar's start).
        let left = a.pages[0].staves[0].left;
        let bar = *marks(&a, |i| matches!(i, Item::Line { from, to, .. } if (from.x - to.x).abs() < 1e-3 && (from.y - to.y).abs() >= 3.9 && from.x > left + 5.0)).first().unwrap();
        let before = a.editor.movement().unwrap().barlines.len();
        right_click(&mut a, bar);
        assert_eq!(a.popover.as_ref().unwrap().kind, PopoverKind::Barline);
        retype(&mut a, "||");
        assert_eq!(a.editor.movement().unwrap().barlines.len(), before + 1);

        // The first staff's clef: the clef at the start becomes bass.
        let clef = *marks(&a, |i| matches!(i, Item::Glyph { glyph, .. } if [Glyph::GClef, Glyph::FClef, Glyph::CClef].iter().any(|g| g.codepoint() == *glyph))).first().unwrap();
        right_click(&mut a, clef);
        assert_eq!(a.popover.as_ref().unwrap().kind, PopoverKind::Clef);
        retype(&mut a, "bass");
        let m = a.editor.movement().unwrap();
        assert!(m.lanes.iter().any(|l| l.clefs.iter().any(|c| c.clef == "bass" && c.at == Frac::ZERO)), "{:?}", m.lanes.iter().map(|l| &l.clefs).collect::<Vec<_>>());

        // A key signature's accidental: the key at the start changes.
        a.editor.clear_selection();
        a.editor.caret = Frac::ZERO;
        let _ = a.open_popover(PopoverKind::Key);
        retype(&mut a, "D");
        let sharp = *marks(&a, |i| matches!(i, Item::Glyph { glyph, .. } if *glyph == Glyph::Sharp.codepoint())).first().unwrap();
        right_click(&mut a, sharp);
        assert_eq!(a.popover.as_ref().unwrap().kind, PopoverKind::Key);
        retype(&mut a, "Eb");
        let m = a.editor.movement().unwrap();
        assert_eq!((m.keys.len(), m.keys[0].fifths), (1, -3));
    }

    #[test]
    fn the_note_input_shadow_is_where_a_click_writes() {
        let mut a = app(View::Page);
        let _ = a.act(Action::SetInput(true));
        assert!(a.editor.input_mode);
        let g = score_view::Ghost { glyph: corinzia_layout::font::Glyph::NoteheadBlack.codepoint(), dots: 0, grid: 1.0 };
        let origins = score_view::page_origins(&a.pages, true);
        // Over the first staff, a little into its second bar, a space above the top line.
        let st = a.pages[0].staves[0].clone();
        let col = a.pages[0].columns.iter().filter(|c| c.system == st.system && c.quarter >= 4.0).nth(1).unwrap().clone();
        let desk = iced::Point::new(origins[0].x + col.x + 0.3, origins[0].y + st.top - 1.0);
        let (spot, ..) = score_view::ghost_spot(&a.pages, &origins, &a.pages_of(a.editor.movement), desk, g).expect("a shadow");
        let at = score_view::locate(&a.pages, &origins, desk);
        let _ = a.update(Message::Canvas(ViewEvent::Click { hit: None, at, shift: false }));
        a.sync();
        let id = a.editor.selection.expect("a note written");
        let head = a.pages.iter().zip(&origins).find_map(|(p, o)| p.items.iter().find_map(|i| match i {
            Item::Glyph { id: g, at, .. } if *g == id => Some(iced::Point::new(at.x + o.x, at.y + o.y)),
            _ => None,
        })).expect("engraved");
        assert!(head.distance(spot) < 0.05, "shadow {spot:?}, note {head:?}");
    }

    #[test]
    fn popovers_open_by_the_caret() {
        let mut a = app(View::Page);
        a.editor.clear_selection();
        a.editor.caret = Frac::int(2);
        let _ = a.open_popover(PopoverKind::Tempo);
        let first = a.popover.as_ref().and_then(|p| p.anchor).expect("anchored");
        a.editor.caret = Frac::int(1);
        let _ = a.open_popover(PopoverKind::Tempo);
        let earlier = a.popover.as_ref().and_then(|p| p.anchor).unwrap();
        assert!(earlier.x < first.x || earlier.y < first.y, "{earlier:?} before {first:?}");
    }

    #[test]
    fn words_on_the_page_are_double_clicked_and_retyped() {
        use corinzia_score::score::ids;
        let mut a = app(View::Page);
        let m = |a: &App| a.editor.project.movements[0].clone();
        // The title, the movement's title and the composer.
        click_item(&mut a, ids::PROJECT_TITLE, 2);
        assert_eq!(a.popover.as_ref().map(|p| (p.kind, p.input.clone())), Some((PopoverKind::Title, "Serenade (demo)".into())));
        retype(&mut a, "Nocturne");
        assert_eq!(a.editor.project.title, "Nocturne");
        click_item(&mut a, ids::MOVEMENT_TITLE, 2);
        retype(&mut a, "II. Lento");
        assert_eq!(m(&a).title, "II. Lento");
        click_item(&mut a, ids::COMPOSER, 2);
        retype(&mut a, "Anon.");
        assert_eq!(a.editor.project.composer, "Anon.");
        // The tempo mark.
        click_item(&mut a, ids::tempo(0), 2);
        assert_eq!(a.popover.as_ref().unwrap().input, "Andante 84");
        retype(&mut a, "Allegro 132");
        assert_eq!((m(&a).tempo[0].text.as_str(), m(&a).tempo[0].bpm), ("Allegro", 132.0));
        // Expression text.
        click_item(&mut a, 50, 2);
        assert_eq!(a.popover.as_ref().map(|p| p.kind), Some(PopoverKind::Expression));
        retype(&mut a, "espress.");
        assert_eq!(m(&a).lanes[0].texts[0].text, "espress.");
        // A dynamic: a click selects it, a double-click retypes it, and
        // Delete removes it.
        let first = m(&a).lanes[0].voices[0].events[0].clone();
        let d = ids::dynamic(first.id);
        click_item(&mut a, d, 1);
        assert_eq!(a.editor.selection, Some(d));
        assert!(a.selected.contains(&d), "drawn selected");
        assert!(a.editor.selection_label().starts_with("Dynamic mp"), "{}", a.editor.selection_label());
        click_item(&mut a, d, 2);
        retype(&mut a, "sfz");
        assert_eq!(m(&a).lanes[0].voices[0].events[0].dynamic, dynamic_index("sfz"));
        assert!(a.pages[0].items.iter().any(|it| matches!(it, Item::Glyph { glyph, id, .. } if *id == d && *glyph == corinzia_layout::font::Glyph::DynamicZ.codepoint())));
        a.editor.select(Some(d));
        let _ = a.act(Action::Delete);
        assert_eq!(m(&a).lanes[0].voices[0].events[0].dynamic, None);
        // Shift+D types one on the selected note.
        a.editor.select(Some(first.heads[0].id));
        let _ = a.act(Action::Popover(PopoverKind::Dynamic));
        retype(&mut a, "fp");
        assert_eq!(m(&a).lanes[0].voices[0].events[0].dynamic, dynamic_index("fp"));
        assert!(a.editor.apply_popover(PopoverKind::Dynamic, "loud").is_err());
        // Every change is one undo step.
        a.editor.undo();
        assert_eq!(m(&a).lanes[0].voices[0].events[0].dynamic, None);
    }

    #[test]
    fn without_a_system_file_dialog_corinzias_browser_opens() {
        let mut a = app(View::Page);
        a.editor.modified = true;
        // Open asks first; Discard goes on to the file dialog.
        let _ = a.update(Message::Guarded(Box::new(Message::Open)));
        assert!(a.confirm.is_some());
        let _ = a.update(Message::ConfirmDiscard(true));
        assert!(a.confirm.is_none());
        // The system dialog could not open: the browser takes its place.
        let _ = a.update(Message::Picked { purpose: browser::Purpose::Open, path: None, failed: true });
        assert!(a.browser.is_some());
        let _ = a.update(Message::Browser(browser::BrowserMsg::Cancel));
        assert!(a.browser.is_none());
        // A dialog the user cancelled stays cancelled.
        let _ = a.update(Message::Picked { purpose: browser::Purpose::Open, path: None, failed: false });
        assert!(a.browser.is_none());
        // Exports too, with the file name filled in; Escape closes it.
        let _ = a.update(Message::Picked { purpose: browser::Purpose::Export(files::Format::Pdf), path: None, failed: true });
        assert!(a.browser.as_ref().unwrap().name.ends_with(".pdf"));
        let esc = iced::keyboard::Event::KeyPressed {
            key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape),
            modified_key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape),
            physical_key: iced::keyboard::key::Physical::Unidentified(iced::keyboard::key::NativeCode::Unidentified),
            location: iced::keyboard::Location::Standard,
            modifiers: Default::default(),
            text: None,
            repeat: false,
        };
        let _ = a.update(Message::Key(esc));
        assert!(a.browser.is_none());
    }

    #[test]
    fn escape_and_blank_paper_deselect() {
        let mut a = app(View::Page);
        let slur = a.editor.project.movements[0].slurs[0].id;
        // A slur, then Escape.
        assert!(a.editor.select_slur(slur));
        let _ = a.act(Action::Escape);
        assert_eq!(a.editor.selected_slur(), None);
        a.sync();
        assert!(a.selected.is_empty());
        // A slur, then a click on the paper (no staff under it).
        assert!(a.editor.select_slur(slur));
        let _ = a.update(Message::Canvas(ViewEvent::Click { hit: None, at: None, shift: false }));
        assert_eq!(a.editor.selected_slur(), None);
        // A note, then a click on the paper just above the staff.
        let head = a.editor.project.movements[0].lanes[0].voices[0].events[0].heads[0].id;
        a.editor.select(Some(head));
        let above = score_view::StaffPoint { page: 0, part: 0, quarter: 0.5, step_below_top: -6 };
        let _ = a.update(Message::Canvas(ViewEvent::Click { hit: None, at: Some(above), shift: false }));
        assert!(a.editor.selection.is_none() && a.editor.range.is_none());
        // A dynamic, then Escape.
        let d = corinzia_score::score::ids::dynamic(a.editor.project.movements[0].lanes[0].voices[0].events[0].id);
        a.editor.select(Some(d));
        let _ = a.act(Action::Escape);
        assert!(a.editor.selection.is_none());
    }

    #[test]
    fn slurs_are_selected_and_moved_to_other_notes_with_the_mouse() {
        let mut a = app(View::Page);
        let events = |a: &App| a.editor.project.movements[0].lanes[0].voices[0].events.clone();
        let notes = |a: &App| events(a).iter().filter(|e| !e.heads.is_empty()).cloned().collect::<Vec<_>>();
        // Only the flute's first two notes slurred.
        a.editor.project.movements[0].slurs.clear();
        let n = notes(&a);
        a.editor.select(Some(n[0].heads[0].id));
        let _ = a.act(Action::Slur);
        let slur = a.editor.project.movements[0].slurs[0].id;
        let slur_of = |a: &App| {
            let n = notes(a);
            let s = a.editor.project.movements[0].slurs.iter().find(|s| s.id == slur)?;
            Some((n.iter().position(|e| e.id == s.from)?, n.iter().position(|e| e.id == s.to)?))
        };
        assert_eq!(slur_of(&a), Some((0, 1)));
        a.editor.clear_selection();
        a.sync();
        // A click on its curve selects it (not its note), with a handle at each end.
        let origins = score_view::page_origins(&a.pages, true);
        let handles = score_view::slur_handles(&a.pages, &origins, slur);
        assert_eq!(handles.len(), 2, "start and end handles");
        let curve = iced::Point::new((handles[0].1.x + handles[1].1.x) / 2.0, (handles[0].1.y + handles[1].1.y) / 2.0);
        let on_curve = (0..12).map(|k| iced::Point::new(curve.x, curve.y - 1.5 + 0.25 * k as f32)).find(|p| score_view::hit_test(&a.pages, &origins, *p) == Some(slur));
        assert!(on_curve.is_some(), "the curve is a hit target");
        let _ = a.update(Message::Canvas(ViewEvent::Click { hit: Some(slur), at: None, shift: false }));
        assert_eq!(a.editor.selected_slur(), Some(slur));
        assert!(a.selected.contains(&slur), "drawn selected");
        // Dragging the end handle over to the fourth note: the end snaps from
        // note to note, and the whole drag is one undo step.
        let q = |e: &corinzia_model::Event| score_view::StaffPoint { page: 0, part: 0, quarter: quarters(e.start), step_below_top: 4 };
        for k in [2, 3] {
            let _ = a.update(Message::Canvas(ViewEvent::SlurDrag { handle: SlurHandle::End, at: q(&n[k]) }));
        }
        let _ = a.update(Message::Canvas(ViewEvent::SlurDragEnd));
        assert_eq!(slur_of(&a), Some((0, 3)));
        assert_eq!(a.editor.undo_label().as_deref(), Some("Move slur"));
        a.editor.undo();
        assert_eq!(slur_of(&a), Some((0, 1)), "one drag, one undo step");
        a.editor.redo();
        // The start dragged onto the second note; the slur stays selected.
        let _ = a.update(Message::Canvas(ViewEvent::Click { hit: Some(slur), at: None, shift: false }));
        let _ = a.update(Message::Canvas(ViewEvent::SlurDrag { handle: SlurHandle::Start, at: q(&n[1]) }));
        let _ = a.update(Message::Canvas(ViewEvent::SlurDragEnd));
        assert_eq!(slur_of(&a), Some((1, 3)));
        assert_eq!(a.editor.selected_slur(), Some(slur));
        // A start can't pass its end.
        let _ = a.update(Message::Canvas(ViewEvent::SlurDrag { handle: SlurHandle::Start, at: q(&n[6]) }));
        let _ = a.update(Message::Canvas(ViewEvent::SlurDragEnd));
        assert_eq!(slur_of(&a), Some((2, 3)));
        // Keys: Shift+→ lengthens it by a note, Alt+← moves its start back.
        let _ = a.act(Action::Extend(1));
        let _ = a.act(Action::Nudge(-1));
        assert_eq!(slur_of(&a), Some((1, 4)));
        // X flips it; Delete removes it and leaves the notes.
        let above = score_view::slur_above(&a.pages, slur).unwrap();
        let _ = a.act(Action::Flip);
        assert_eq!(score_view::slur_above(&a.pages, slur), Some(!above));
        let count = events(&a).len();
        let _ = a.act(Action::Delete);
        assert_eq!(slur_of(&a), None);
        assert_eq!(events(&a).len(), count);
        assert_eq!(a.editor.selected_slur(), None);
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
    fn preferences_change_the_theme_and_restart_audio_at_a_new_rate() {
        let p = files::new_project("P", "Flute", 2).unwrap();
        let mut a = App::new(p, None, View::Page, String::new());
        let _ = a.update(Message::Act(Action::Preferences));
        assert!(a.prefs_dialog.is_some());
        let _ = a.update(Message::Prefs(PrefsMsg::Theme("Light".into())));
        assert_eq!(theme::palette().text, theme::LIGHT.text, "seen at once");
        let _ = a.update(Message::Prefs(PrefsMsg::Cancel));
        assert_eq!(theme::palette().text, theme::DARK.text, "cancel goes back");
        let _ = a.update(Message::Prefs(PrefsMsg::Open));
        let _ = a.update(Message::Prefs(PrefsMsg::Theme("High contrast".into())));
        let _ = a.update(Message::Prefs(PrefsMsg::Rate(96000)));
        let _ = a.update(Message::Prefs(PrefsMsg::Buffer(512)));
        let _ = a.update(Message::Prefs(PrefsMsg::Paths("/opt/a; /opt/b".into())));
        let _ = a.update(Message::Prefs(PrefsMsg::Apply));
        assert!(a.prefs_dialog.is_none());
        assert_eq!((a.prefs.sample_rate, a.prefs.buffer, a.prefs.plugin_paths.len()), (96000, 512, 2));
        assert_eq!(a.playback.sample_rate, 96000.0, "a new engine");
        assert!(a.playback.search_paths[0].ends_with("opt/a"));
        assert_eq!(a.editor.status, "Preferences saved · 96000 Hz, 512 frames");
        theme::set("Dark");
    }

    #[test]
    fn condensing_puts_two_flutes_on_one_staff() {
        let p = files::new_project("Duo", "Flute 1, Flute 2, Cello", 2).unwrap();
        let mut a = App::new(p, None, View::Page, String::new());
        // Bar 1 in unison, bar 2 apart; the cello rests.
        for (lane, notes) in [(0, [0, 1, 2, 3, 4, 4, 4, 4]), (1, [0, 1, 2, 3, 2, 2, 2, 2])] {
            a.editor.select(None);
            a.editor.set_input_mode(true);
            a.editor.caret_lane = lane;
            a.editor.caret = corinzia_score::Frac::ZERO;
            for s in notes {
                a.editor.enter_note(s, false);
            }
            a.editor.set_input_mode(false);
        }
        let mut l = a.editor.project.layout.clone();
        l.condense = true;
        let parts = a.editor.project.part_layout.clone();
        a.editor.set_layout(l, parts, Default::default());
        a.sync();
        assert_eq!(a.staff_lanes, [0, 2], "the flutes on staff 0, the cello on 1");
        let m = a.editor.movement().unwrap();
        let mut s = corinzia_model::notation::score(&a.editor.project, m);
        corinzia_model::notation::condense(&mut s);
        assert_eq!(s.parts[0].name, "Flute 1.2");
        assert!(s.parts[0].texts.iter().any(|t| t.text == "a 2" && t.at.is_zero()));
        let second = s.parts[0].voices.iter().find(|v| v.index == 2).unwrap();
        assert!(second.events.iter().all(|e| e.start >= corinzia_score::Frac::int(1)), "bar 1 written once");
        assert_eq!(second.events.iter().filter(|e| !e.pitches.is_empty()).count(), 4);
        // Parts are never condensed.
        let _ = a.update(Message::SetPart(Some(a.editor.project.players[1].id)));
        a.sync();
        assert_eq!(a.staff_lanes, [1]);
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
        a.click(None, Some(StaffPoint { page: 0, part: 0, quarter: 0.0, step_below_top: 4 }), false);
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
    fn a_click_in_empty_bar_space_selects_the_bar() {
        let mut a = app(View::Page);
        a.sync();
        // Between notes of bar 2 on the cello's staff, on its middle line.
        a.click(None, Some(StaffPoint { page: 0, part: 1, quarter: 5.5, step_below_top: 4 }), false);
        let r = a.editor.range.expect("a bar selected");
        assert_eq!((r.lane_from, r.lane_to, r.from, r.to), (1, 1, Frac::int(1), Frac::int(2)));
        // Shift-click in bar 4 of the flute: bars 2-4 on both staves.
        a.click(None, Some(StaffPoint { page: 0, part: 0, quarter: 13.0, step_below_top: 2 }), true);
        let r = a.editor.range.unwrap();
        assert_eq!((r.lane_from, r.lane_to, r.from, r.to), (0, 1, Frac::int(1), Frac::int(4)));
        // Far from any staff: the selection clears.
        a.click(None, Some(StaffPoint { page: 0, part: 0, quarter: 13.0, step_below_top: 30 }), false);
        assert!(a.editor.range.is_none());
        // In note input a click still enters a note.
        a.editor.set_input_mode(true);
        let rev = a.editor.music_revision;
        a.click(None, Some(StaffPoint { page: 0, part: 0, quarter: 1.0, step_below_top: 4 }), false);
        assert!(a.editor.range.is_none() && a.editor.selection.is_some() && a.editor.music_revision > rev);
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
    fn the_players_card_adds_removes_and_reorders_players() {
        let mut a = app(View::Page);
        let players = |a: &App| a.editor.project.players.iter().map(|p| a.editor.project.instrument(p.instruments[0]).unwrap().name.clone()).collect::<Vec<_>>();
        assert_eq!(players(&a), ["Flute", "Cello"]);
        let flute = a.editor.project.players[0].id;
        let pl = |m: PlayersMsg| Message::Players(m);
        let _ = a.update(pl(PlayersMsg::Open));
        assert_eq!(a.players.as_ref().unwrap().rows.len(), 2);
        // Escape and Cancel change nothing.
        let _ = a.update(pl(PlayersMsg::Cancel));
        assert!(a.players.is_none());
        let _ = a.update(pl(PlayersMsg::Open));
        // Add a piano (a grand staff) and two violins; a bad list says why.
        let _ = a.update(pl(PlayersMsg::Quick("Piano")));
        let _ = a.update(pl(PlayersMsg::AddInput(" , ".into())));
        let _ = a.update(pl(PlayersMsg::Add));
        assert!(a.players.as_ref().unwrap().error.is_some());
        let _ = a.update(pl(PlayersMsg::AddInput("2 Violin".into())));
        let _ = a.update(pl(PlayersMsg::Add));
        let d = a.players.as_ref().unwrap();
        assert!(d.error.is_none() && d.add.is_empty());
        assert_eq!(d.rows.iter().map(|r| r.instruments.as_str()).collect::<Vec<_>>(), ["Flute / Fl.", "Cello / Vc.", "Piano", "Violin 1", "Violin 2"]);
        // Remove the flute, put the piano first.
        let _ = a.update(pl(PlayersMsg::Remove(0)));
        let _ = a.update(pl(PlayersMsg::Move(1, -1)));
        let _ = a.update(pl(PlayersMsg::Move(0, -1))); // already first: nothing
        let tracks_before = a.editor.project.tracks.len();
        let _ = a.update(pl(PlayersMsg::Apply));
        assert!(a.players.is_none());
        assert_eq!(players(&a), ["Piano", "Cello", "Violin 1", "Violin 2"]);
        assert!(a.editor.project.player(flute).is_none());
        assert_eq!(a.editor.project.tracks.len(), tracks_before - 1 + 3);
        assert_eq!(a.editor.project.tracks.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), ["Piano", "Cello", "Violin 1", "Violin 2"], "mixer in score order");
        let m = &a.editor.project.movements[0];
        // Piano: two staves; every staff has music or rests through the movement.
        assert_eq!(m.lanes.len(), 5);
        assert_eq!(m.lanes[0].player, m.lanes[1].player);
        assert_eq!(m.lanes[1].clefs[0].clef, "bass");
        for l in &m.lanes {
            assert_eq!(corinzia_edit::voice_end(&l.voices[0].events), m.length);
        }
        assert_eq!(a.editor.status, "3 players added, 1 player removed");
        // The new instruments play: on their own MIDI channels.
        let mut channels: Vec<u8> = a.editor.project.tracks.iter().map(|t| t.midi_channel).collect();
        channels.sort();
        channels.dedup();
        assert_eq!(channels.len(), 4);
        // One undo step brings the flute back.
        a.editor.undo();
        assert_eq!(players(&a), ["Flute", "Cello"]);
        // Removing every player is not allowed.
        let _ = a.update(pl(PlayersMsg::Open));
        let _ = a.update(pl(PlayersMsg::Remove(0)));
        let _ = a.update(pl(PlayersMsg::Remove(0)));
        let _ = a.update(pl(PlayersMsg::Apply));
        assert_eq!(a.players.as_ref().unwrap().error.as_deref(), Some("A score needs at least one player"));
        assert_eq!(players(&a), ["Flute", "Cello"]);
    }

    #[test]
    fn players_change_instrument_and_take_up_doublings() {
        let mut a = app(View::Page);
        let pl = |m: PlayersMsg| Message::Players(m);
        let flute = a.editor.project.players[0].id;
        let flute_inst = a.editor.project.players[0].instruments[0];
        let notes_before = a.editor.project.movements[0].lanes[0].voices[0].events.clone();
        let track = a.editor.project.instrument(flute_inst).unwrap().track;
        // Applying the list as it is changes nothing, not even the undo list.
        let _ = a.update(pl(PlayersMsg::Open));
        let _ = a.update(pl(PlayersMsg::Apply));
        assert_eq!(a.editor.status, "No changes to the players");
        assert!(!a.editor.can_undo());
        // The flute player becomes an oboist who doubles cor anglais.
        let _ = a.update(pl(PlayersMsg::Open));
        let _ = a.update(pl(PlayersMsg::Edit(0, "Oboe, Cor anglais".into())));
        let _ = a.update(pl(PlayersMsg::Apply));
        assert!(a.players.is_none(), "{:?}", a.players.as_ref().and_then(|d| d.error.clone()));
        let p = &a.editor.project;
        let player = p.player(flute).unwrap();
        assert_eq!(player.instruments.len(), 2);
        let oboe = p.instrument(player.instruments[0]).unwrap();
        assert_eq!((oboe.id, oboe.name.as_str(), oboe.engraver_name.as_str()), (flute_inst, "Oboe", "Ob."), "the instrument changes in place");
        assert_eq!(oboe.track, track, "keeping its track");
        assert_eq!(p.track(track.unwrap()).unwrap().name, "Oboe");
        let ca = p.instrument(player.instruments[1]).unwrap();
        assert_eq!((ca.name.as_str(), ca.transposition), ("Cor anglais", corinzia_model::Interval::new(4, 7)));
        assert!(ca.track.is_some() && ca.track != track, "a doubling sounds on its own track");
        assert!(std::sync::Arc::ptr_eq(&a.editor.project.movements[0].lanes[0].voices[0].events, &notes_before), "the music stays");
        assert_eq!(a.editor.status, "2 instruments changed");
        assert_eq!(a.editor.project.tracks.len(), 3);
        // Dropping the doubling takes its track.
        let _ = a.update(pl(PlayersMsg::Open));
        assert_eq!(a.players.as_ref().unwrap().rows[0].instruments, "Oboe / Ob., Cor anglais / C. a.");
        let _ = a.update(pl(PlayersMsg::Edit(0, "Oboe".into())));
        let _ = a.update(pl(PlayersMsg::Apply));
        assert_eq!(a.editor.project.player(flute).unwrap().instruments.len(), 1);
        assert_eq!(a.editor.project.tracks.len(), 2);
        // A new short name for the staff's later systems.
        let _ = a.update(pl(PlayersMsg::Open));
        let _ = a.update(pl(PlayersMsg::Edit(0, "Oboe / Hb.".into())));
        let _ = a.update(pl(PlayersMsg::Apply));
        assert_eq!(a.editor.project.instrument(flute_inst).unwrap().engraver_name, "Hb.");
        assert_eq!(a.editor.status, "1 instrument changed");
        // A row without an instrument is refused.
        let _ = a.update(pl(PlayersMsg::Open));
        let _ = a.update(pl(PlayersMsg::Edit(1, " ".into())));
        let _ = a.update(pl(PlayersMsg::Apply));
        assert_eq!(a.players.as_ref().unwrap().error.as_deref(), Some("Player 2: Each player needs an instrument"));
    }

    #[test]
    fn an_instrument_change_switches_sound_clef_and_written_key() {
        let mut a = app(View::Page);
        let flute = a.editor.project.players[0].id;
        // Bar 3 of the flute: change to clarinet (added as a doubling).
        let m = a.editor.movement().unwrap();
        let bar3 = corinzia_edit::bar_starts(m)[2];
        let first_in_bar3 = m.lanes[0].voices[0].events.iter().find(|e| e.start >= bar3 && !e.is_rest()).unwrap().heads[0].id;
        a.editor.select(Some(first_in_bar3));
        a.editor.apply_popover(PopoverKind::Instrument, "clarinet").unwrap();
        let p = &a.editor.project;
        let insts = &p.player(flute).unwrap().instruments;
        assert_eq!(insts.len(), 2);
        let clarinet = p.instrument(insts[1]).unwrap();
        assert_eq!(clarinet.name, "Clarinet in B♭");
        let lane = &p.movements[0].lanes[0];
        assert_eq!(lane.changes.len(), 2);
        assert_eq!((lane.changes[1].at, lane.changes[1].instrument), (bar3, clarinet.id));
        assert_eq!(a.editor.status, "Clarinet in B♭ · bar 3");
        assert!(lane.clefs.is_empty(), "both treble: no clef change");
        // Playback: notes from bar 3 sound on the clarinet's track.
        let perf = corinzia_perform::perform(&p.movements[0]);
        let flute_notes: Vec<_> = perf.notes.iter().filter(|n| insts.contains(&n.instrument)).collect();
        assert!(flute_notes.iter().filter(|n| n.on_q < 8.0).all(|n| n.instrument == insts[0]));
        assert!(flute_notes.iter().filter(|n| n.on_q >= 8.0).all(|n| n.instrument == clarinet.id));
        assert!(flute_notes.iter().any(|n| n.on_q >= 8.0));
        // Written pitch: the clarinet's bars show its key (D major in C).
        let s = corinzia_model::notation::score_as(p, &p.movements[0], true);
        assert_eq!(s.parts[0].key_shift, 0);
        assert_eq!(s.parts[0].key_shifts, vec![(bar3, 2)]);
        a.editor.transposed = true;
        a.engrave();
        assert!(!a.pages.is_empty());
        // Back to the flute later, then to a bass-clef instrument: the clef follows.
        a.editor.select(None);
        a.editor.range = None;
        a.editor.caret_lane = 0;
        a.editor.caret = corinzia_edit::bar_starts(a.editor.movement().unwrap())[5];
        a.editor.apply_popover(PopoverKind::Instrument, "Bass clarinet").unwrap();
        let lane = &a.editor.project.movements[0].lanes[0];
        assert_eq!(lane.clefs.len(), 1);
        assert_eq!(lane.clefs[0].clef, "bass");
        // Empty text removes the change at the caret.
        a.editor.apply_popover(PopoverKind::Instrument, "").unwrap();
        assert_eq!(a.editor.project.movements[0].lanes[0].changes.len(), 2);
        assert!(a.editor.apply_popover(PopoverKind::Instrument, "").is_err());
        // One undo step each.
        a.editor.undo();
        a.editor.undo();
        a.editor.undo();
        assert_eq!(a.editor.project.player(flute).unwrap().instruments.len(), 1);
    }

    #[test]
    fn transposing_the_score_a_passage_or_to_a_key() {
        let mut a = app(View::Page);
        let pitch = |a: &App, lane: usize, k: usize| a.editor.project.movements[0].lanes[lane].voices[0].events.iter().filter(|e| !e.is_rest()).nth(k).unwrap().heads[0].pitch;
        let name = |p: corinzia_model::Pitch| editor::pitch_name(p);
        assert_eq!(name(pitch(&a, 0, 0)), "E5");
        // A chord symbol goes along.
        a.editor.caret_lane = 0;
        a.editor.caret = corinzia_score::Frac::ZERO;
        a.editor.apply_popover(PopoverKind::Chord, "C").unwrap();
        // The whole score up a major second: D major, F sharp, chord D.
        a.editor.apply_popover(PopoverKind::Transpose, "up M2").unwrap();
        assert_eq!(a.editor.status, "The score transposed up a major 2nd");
        let m = &a.editor.project.movements[0];
        assert_eq!(m.keys[0].fifths, 2);
        assert_eq!(name(pitch(&a, 0, 0)), "F♯5");
        assert_eq!(name(pitch(&a, 1, 0)), "D3");
        assert_eq!(m.lanes[0].chords[0].text, "D");
        a.editor.undo();
        assert_eq!(a.editor.project.movements[0].keys[0].fifths, 0, "one undo step");
        // To E-flat: the nearer way is up a minor third.
        a.editor.apply_popover(PopoverKind::Transpose, "to Eb").unwrap();
        assert_eq!(a.editor.project.movements[0].keys[0].fifths, -3);
        assert_eq!(name(pitch(&a, 0, 0)), "G5");
        assert!(a.editor.apply_popover(PopoverKind::Transpose, "to Eb").is_err(), "already there");
        a.editor.undo();
        // A passage: the flute's first bar only, keys untouched.
        let id = a.editor.project.movements[0].lanes[0].voices[0].events[0].heads[0].id;
        a.editor.select(Some(id));
        a.editor.select_bar(0, corinzia_score::Frac::ZERO, false);
        a.editor.apply_popover(PopoverKind::Transpose, "down P8").unwrap();
        assert_eq!(a.editor.status, "The passage transposed down an octave");
        assert_eq!(name(pitch(&a, 0, 0)), "E4");
        assert_eq!(a.editor.project.movements[0].keys[0].fifths, 0);
        assert_eq!(name(pitch(&a, 1, 0)), "C3", "other staves stay");
        let later = a.editor.project.movements[0].lanes[0].voices[0].events.iter().find(|e| e.start >= corinzia_score::Frac::int(1) && !e.is_rest()).unwrap();
        assert_eq!(name(later.heads[0].pitch), "E5", "bar 2 stays");
        assert!(a.editor.apply_popover(PopoverKind::Transpose, "sideways").is_err());
        assert_eq!(editor::interval_name(corinzia_model::Interval::new(3, 6)), "up an augmented 4th");
        assert_eq!(editor::interval_name(corinzia_model::Interval::new(-2, -3)), "down a minor 3rd");
    }

    #[test]
    fn filters_and_select_similar_pick_elements_to_edit() {
        let mut a = app(View::Page);
        let notes = |a: &App, lane: usize| a.editor.project.movements[0].lanes[lane].voices[0].events.iter().filter(|e| !e.is_rest()).cloned().collect::<Vec<_>>();
        let high: Vec<u64> = notes(&a, 0).iter().filter(|e| e.heads[0].pitch.midi_key() > 72).map(|e| e.heads[0].id).collect();
        assert!(high.len() > 3);
        // Ctrl+F "above C5": the flute's high notes, highlighted.
        let _ = a.update(Message::Act(Action::Popover(PopoverKind::Filter)));
        let _ = a.update(Message::PopoverInput("above C5".into()));
        let _ = a.update(Message::PopoverSubmit);
        assert_eq!(a.editor.picked, high);
        assert_eq!(a.editor.status, format!("{} notes picked in the movement", high.len()));
        assert!(high.iter().all(|id| a.selected.contains(id)));
        // Edits apply to each: staccato, then up a step.
        a.editor.toggle_articulation(corinzia_score::score::art::STACCATO);
        let stacc = |a: &App| notes(a, 0).iter().filter(|e| e.articulations & corinzia_score::score::art::STACCATO != 0).count();
        assert_eq!(stacc(&a), high.len());
        let before: Vec<i32> = notes(&a, 0).iter().map(|e| e.heads[0].pitch.midi_key()).collect();
        a.editor.transpose_selection(1);
        let after: Vec<i32> = notes(&a, 0).iter().map(|e| e.heads[0].pitch.midi_key()).collect();
        assert_eq!(before.iter().zip(&after).filter(|(b, a)| b != a).count(), high.len(), "only the picked notes moved");
        a.editor.undo();
        a.editor.undo();
        assert_eq!(stacc(&a), 0);
        // Narrowing: among those picked, only E.
        a.editor.picked = high.clone();
        a.editor.filter("E").unwrap();
        assert!(a.editor.picked.len() < high.len() && !a.editor.picked.is_empty());
        // Delete takes the picked notes (rests in voice 1), one undo step.
        let n_rests = |a: &App| a.editor.project.movements[0].lanes[0].voices[0].events.iter().filter(|e| e.is_rest()).count();
        let (rests, picked) = (n_rests(&a), a.editor.picked.len());
        a.editor.delete_selection();
        assert!(n_rests(&a) > rests);
        assert_eq!(notes(&a, 0).len(), notes(&app(View::Page), 0).len() - picked);
        a.editor.undo();
        // Select similar from a dynamic: the same mark everywhere (mp: one).
        let first = notes(&a, 0)[0].id;
        a.editor.select(Some(corinzia_score::score::ids::dynamic(first)));
        let _ = a.update(Message::Act(Action::SelectSimilar));
        assert_eq!(a.editor.picked, vec![corinzia_score::score::ids::dynamic(first)]);
        // From a note: every note on its staff; a mark applies to all.
        a.editor.select(Some(notes(&a, 1)[0].heads[0].id));
        let _ = a.update(Message::Act(Action::SelectSimilar));
        assert_eq!(a.editor.picked.len(), notes(&a, 1).iter().map(|e| e.heads.len()).sum::<usize>());
        a.editor.set_dynamic(corinzia_model::dynamic_index("ff"));
        assert!(notes(&a, 1).iter().all(|e| e.dynamic == corinzia_model::dynamic_index("ff")));
        // Nothing found says so; Escape lets go.
        assert_eq!(a.editor.filter("dynamics sfz"), Err("No dynamics in the movement".into()));
        let _ = a.update(Message::Act(Action::Escape));
        assert!(a.editor.picked.is_empty());
    }

    #[test]
    fn explode_and_reduce_move_music_between_staves() {
        use corinzia_model::{Event, EventKind, Head, Pitch};
        use corinzia_score::Frac;
        let project = files::new_project("T", "Violin, Viola, Cello, Piano", 2).unwrap();
        let mut ed = editor::Editor::new(project);
        // Lanes: violin, viola, cello, piano right hand, piano left hand.
        let chord = |id: u64, k: i64| Event {
            id,
            kind: EventKind::Chord,
            start: Frac::new(k, 4),
            duration: Frac::new(1, 4),
            heads: [(0, 4), (2, 4), (4, 4), (0, 5)].iter().enumerate().map(|(n, &(step, octave))| Head { id: id * 10 + n as u64, pitch: Pitch { step, alter: 0, octave }, tie: false, velocity: 0 }).collect(),
            articulations: 0,
            dynamic: None,
            spans: 0,
            lyrics: vec![],
            graces: vec![],
            fingering: vec![],
        };
        let mut rh: Vec<Event> = (0..4).map(|k| chord(9000 + k as u64, k)).collect();
        rh.push(Event { id: 9100, kind: EventKind::Rest, start: Frac::int(1), duration: Frac::int(1), heads: vec![], articulations: 0, dynamic: None, spans: 0, lyrics: vec![], graces: vec![], fingering: vec![] });
        ed.project.next_id = 100_000;
        ed.project.movements[0].lanes[3].voices[0].events = std::sync::Arc::new(rh);
        let top = |ed: &editor::Editor, lane: usize| ed.project.movements[0].lanes[lane].voices[0].events.iter().filter(|e| !e.is_rest()).map(|e| e.heads.iter().map(|h| editor::pitch_name(h.pitch)).collect::<Vec<_>>().join("-")).collect::<Vec<_>>();
        // Nothing selected, or one staff: says what to do.
        assert!(ed.explode().unwrap_err().contains("two or more staves"));
        // The chords onto violin, viola, cello and the right hand.
        ed.range = Some(editor::Range { lane_from: 0, lane_to: 3, from: Frac::ZERO, to: Frac::int(1) });
        ed.explode().unwrap();
        assert_eq!(top(&ed, 0), ["C5"; 4]);
        assert_eq!(top(&ed, 1), ["G4"; 4]);
        assert_eq!(top(&ed, 2), ["E4"; 4]);
        assert_eq!(top(&ed, 3), ["C4"; 4]);
        assert_eq!(ed.status, "Exploded onto 4 staves");
        // Reduce all five: onto the empty left hand, as the chords again.
        ed.range = Some(editor::Range { lane_from: 0, lane_to: 4, from: Frac::ZERO, to: Frac::int(1) });
        ed.reduce().unwrap();
        assert_eq!(top(&ed, 4), ["C4-E4-G4-C5"; 4]);
        assert_eq!(ed.status, "Reduced onto Piano");
        // Ids stay unique.
        let mut ids: Vec<u64> = ed.project.movements[0].lanes.iter().flat_map(|l| l.voices[0].events.iter().flat_map(|e| std::iter::once(e.id).chain(e.heads.iter().map(|h| h.id)))).collect();
        let n = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), n);
        ed.undo();
        ed.undo();
        assert_eq!(top(&ed, 3), ["C4-E4-G4-C5"; 4], "one undo step each");
        assert!(top(&ed, 0).is_empty());
    }

    #[test]
    fn beams_by_hand_and_by_the_meter() {
        use corinzia_score::score::span;
        let mut a = app(View::Page);
        // The flute's first bar: four eighths after the opening quarter.
        let evs = a.editor.project.movements[0].lanes[0].voices[0].events.clone();
        let eighth = evs.iter().find(|e| e.duration == corinzia_score::Frac::new(1, 8)).unwrap().heads[0].id;
        a.editor.select(Some(eighth));
        let _ = a.update(Message::Act(Action::Beam(span::NO_BEAM)));
        let e = |a: &App| a.editor.project.movements[0].lanes[0].voices[0].events.iter().find(|e| e.heads.first().is_some_and(|h| h.id == eighth)).unwrap().spans;
        assert_eq!(e(&a) & span::BEAMS, span::NO_BEAM);
        assert_eq!(a.editor.status, "No beam");
        let _ = a.update(Message::Act(Action::Beam(span::BEAM_BREAK)));
        assert_eq!(e(&a) & span::BEAMS, span::BEAM_BREAK, "one mode at a time");
        let _ = a.update(Message::Act(Action::Beam(0)));
        assert_eq!(e(&a) & span::BEAMS, 0);
        // A grouped meter from the Time popover, kept in the movement.
        a.editor.select(None);
        a.editor.caret = corinzia_score::Frac::ZERO;
        a.editor.apply_popover(PopoverKind::Time, "7/8 3+2+2").unwrap();
        let m = &a.editor.project.movements[0].meter[0];
        assert_eq!((m.num, m.den, m.beams[..3].to_vec()), (7, 8, vec![3, 2, 2]));
        assert!(a.editor.apply_popover(PopoverKind::Time, "7/8 3+3").is_err());
        a.engrave();
        assert!(!a.pages.is_empty());
    }

    #[test]
    fn layout_card_page_texts_and_breaks() {
        let mut a = app(View::Page);
        let lm = |m: LayoutMsg| Message::Layout(m);
        let _ = a.update(Message::Act(Action::Layout));
        assert_eq!(a.layout.as_ref().unwrap().fields[0].size, "A4");
        // Letter, landscape, a 7 mm staff, four bars a system, page texts.
        let _ = a.update(lm(LayoutMsg::Size("Letter".into())));
        let _ = a.update(lm(LayoutMsg::Landscape(true)));
        let _ = a.update(lm(LayoutMsg::Field(LayoutField::Staff, "7".into())));
        let _ = a.update(lm(LayoutMsg::Field(LayoutField::Bars, "4".into())));
        let _ = a.update(lm(LayoutMsg::Text(0, "for flute and cello".into())));
        let _ = a.update(lm(LayoutMsg::Text(3, "© 2026".into())));
        // The parts keep their own.
        let _ = a.update(lm(LayoutMsg::Tab(true)));
        assert_eq!(a.layout.as_ref().unwrap().fields[1].staff, "7");
        let _ = a.update(lm(LayoutMsg::Field(LayoutField::Spacing, "300".into())));
        let _ = a.update(lm(LayoutMsg::Apply));
        assert_eq!(a.layout.as_ref().unwrap().error.as_deref(), Some("Parts: Note spacing (%): a number from 50 to 250"));
        let _ = a.update(lm(LayoutMsg::Field(LayoutField::Spacing, "120".into())));
        let _ = a.update(lm(LayoutMsg::Apply));
        assert!(a.layout.is_none());
        let l = &a.editor.project.layout;
        assert_eq!((l.page_width_mm, l.landscape, l.staff_mm, l.bars_per_system), (215.9, true, 7.0, 4));
        assert_eq!(a.editor.project.part_layout.spacing, 1.2);
        assert_eq!(a.editor.project.subtitle, "for flute and cello");
        // The page: 279.4 mm wide in 1.75 mm spaces, printed at 4.96 pt.
        a.engrave();
        assert!((a.pages[0].width - 279.4 / 1.75).abs() < 0.01);
        assert!((a.pages[0].space_pt - 1.75 * 72.0 / 25.4).abs() < 0.01);
        let texts: Vec<String> = a.pages[0].items.iter().filter_map(|i| if let Item::Text { text, .. } = i { Some(text.clone()) } else { None }).collect();
        assert!(texts.contains(&"for flute and cello".into()) && texts.contains(&"© 2026".into()));
        // Four bars, then the next four: two systems.
        let systems = |a: &App| a.pages.iter().map(|p| p.staves.iter().map(|s| s.system).max().map_or(0, |m| m + 1)).sum::<u32>();
        assert_eq!(systems(&a), 2);
        a.editor.undo();
        assert_eq!(a.editor.project.layout, corinzia_model::Layout::default(), "one undo step");
        // A system break before bar 3 (Enter on a note there).
        let bar3 = corinzia_edit::bar_starts(a.editor.movement().unwrap())[2];
        let note = a.editor.movement().unwrap().lanes[0].voices[0].events.iter().find(|e| e.start >= bar3 && !e.is_rest()).unwrap().heads[0].id;
        a.editor.select(Some(note));
        let _ = a.update(Message::Act(Action::Break { page: false }));
        assert_eq!(a.editor.status, "System break before bar 3");
        a.engrave();
        let starts: Vec<f64> = a.pages[0].columns.iter().filter(|c| c.system == 1).map(|c| c.quarter).collect();
        assert_eq!(starts.iter().cloned().fold(f64::MAX, f64::min), 8.0, "the second system starts at bar 3");
        // Ctrl+Enter there turns it into a page break; Enter again removes it.
        let _ = a.update(Message::Act(Action::Break { page: true }));
        a.engrave();
        assert_eq!(a.pages.len(), 2);
        let _ = a.update(Message::Act(Action::Break { page: true }));
        assert_eq!(a.editor.status, "Page break removed before bar 3");
        assert!(a.editor.movement().unwrap().breaks.is_empty());
        // In a part, the break is the part's alone.
        let cello = a.editor.project.players[1].id;
        let _ = a.update(Message::SetPart(Some(cello)));
        a.editor.select(Some(note));
        let _ = a.update(Message::Act(Action::Break { page: false }));
        let b = a.editor.movement().unwrap().breaks[0];
        assert_eq!(b.part, Some(cello));
        assert!(corinzia_model::notation::score(&a.editor.project, a.editor.movement().unwrap()).breaks.is_empty());
        // Not before the first bar.
        a.editor.select(None);
        a.editor.caret = corinzia_score::Frac::ZERO;
        assert!(a.editor.toggle_break(false, None).is_err());
    }

    #[test]
    fn all_movements_one_after_another_in_the_view() {
        let mut a = app(View::Page);
        a.editor.add_movement();
        a.editor.rename_movement("II. Allegro");
        a.editor.switch_movement(0);
        a.sync();
        let one = a.pages.len();
        let _ = a.update(Message::Act(Action::AllMovements));
        a.sync();
        assert!(a.pages.len() > one, "both movements' pages");
        assert_eq!(a.page_movements.first(), Some(&0));
        assert_eq!(a.page_movements.last(), Some(&1));
        let second = a.page_movements.iter().position(|&m| m == 1).unwrap();
        assert_eq!(a.pages_of(1), second..a.pages.len());
        // The second movement's page is headed by its own title, not the work's.
        let texts = |p: &DisplayList| p.items.iter().filter_map(|i| if let Item::Text { text, .. } = i { Some(text.clone()) } else { None }).collect::<Vec<_>>();
        assert!(texts(&a.pages[second]).contains(&"II. Allegro".to_string()));
        assert!(!texts(&a.pages[second]).contains(&"Serenade (demo)".to_string()));
        // A click on it (empty paper, its first staff) edits that movement.
        let _ = a.update(Message::Canvas(ViewEvent::Click { hit: None, at: Some(StaffPoint { page: second, part: 0, quarter: 0.0, step_below_top: 4 }), shift: false }));
        assert_eq!(a.editor.movement, 1);
        // A click on a note of the first movement takes the editing back.
        let first_note = a.editor.project.movements[0].lanes[0].voices[0].events.iter().find(|e| !e.is_rest()).unwrap().heads[0].id;
        let _ = a.update(Message::Canvas(ViewEvent::Click { hit: Some(first_note), at: None, shift: false }));
        assert_eq!(a.editor.movement, 0);
        assert_eq!(a.editor.selection, Some(first_note));
        // Off again: one movement.
        let _ = a.update(Message::Act(Action::AllMovements));
        a.sync();
        assert_eq!(a.pages.len(), one);
    }

    #[test]
    fn a_shown_part_whose_player_goes_shows_the_full_score() {
        let mut a = app(View::Page);
        let cello = a.editor.project.players[1].id;
        let _ = a.update(Message::SetPart(Some(cello)));
        let _ = a.update(Message::Players(PlayersMsg::Open));
        let _ = a.update(Message::Players(PlayersMsg::Remove(1)));
        let _ = a.update(Message::Players(PlayersMsg::Apply));
        assert_eq!(a.part, None);
        assert_eq!(a.editor.project.movements[0].lanes.len(), 1);
        a.engrave();
        assert!(!a.pages.is_empty());
    }

    #[test]
    fn effects_buses_and_sends_from_the_mix_tab() {
        let mut a = app(View::Page);
        let _ = a.update(Message::Act(Action::ShowMix));
        let mx = |m: mix::MixMsg| Message::Mix(m);
        let flute = a.editor.project.tracks[0].id;
        let index = |a: &App, id: &str| a.playback.plugins.iter().position(|p| p.id == id).unwrap();
        // A gain insert on the flute, its level from the parameter list.
        let gain = index(&a, "org.corinzia.gain");
        let _ = a.update(mx(mix::MixMsg::AddEffect(flute, gain)));
        let node = a.fx.expect("the new effect is selected");
        assert_eq!(a.editor.project.tracks[0].inserts, [node]);
        a.sync();
        assert!(a.playback.pool.effect(node).is_some(), "loaded with the new plan");
        let _ = a.update(mx(mix::MixMsg::Param(node, 0, -60.0)));
        assert_eq!(a.playback.pool.effect(node).unwrap().param_value(0), Some(-60.0));
        // A bus with a reverb; the flute sends to it.
        let _ = a.update(mx(mix::MixMsg::AddBus));
        let bus = a.editor.project.buses[0].id;
        assert_eq!((a.strip, a.editor.project.buses[0].name.as_str()), (Some(bus), "Bus 1"));
        let reverb = index(&a, "org.corinzia.reverb");
        let _ = a.update(mx(mix::MixMsg::AddEffect(bus, reverb)));
        let _ = a.update(mx(mix::MixMsg::ToggleSend(flute, bus)));
        let _ = a.update(mx(mix::MixMsg::SendLevel(flute, 0, -3.0)));
        let _ = a.update(mx(mix::MixMsg::Released));
        assert_eq!(a.editor.project.tracks[0].sends, [corinzia_model::SendLevel { bus, gain_db: -3.0, pre: false }]);
        // The cello into the bus; the master down 6 dB.
        let cello = a.editor.project.tracks[1].id;
        let _ = a.update(mx(mix::MixMsg::Output(cello, "Bus 1".into())));
        assert_eq!(a.editor.project.tracks[1].output_bus, Some(bus));
        let _ = a.update(mx(mix::MixMsg::Gain(editor::MASTER, -6.0)));
        assert_eq!(a.editor.project.master.gain_db, -6.0);
        a.sync();
        let _ = a.update(Message::Act(Action::PlayStop));
        assert!(render(&mut a, 9600) > 0.001, "heard through the bus");
        // Bypass and removal; one undo brings the bus back.
        let _ = a.update(mx(mix::MixMsg::Bypass(node)));
        assert!(a.editor.project.node(node).unwrap().bypassed);
        let _ = a.update(mx(mix::MixMsg::RemoveBus(bus)));
        assert!(a.editor.project.buses.is_empty() && a.editor.project.tracks[0].sends.is_empty() && a.editor.project.tracks[1].output_bus.is_none());
        a.editor.undo();
        assert_eq!(a.editor.project.buses.len(), 1);
        a.sync();
    }

    #[test]
    fn presets_and_programs_from_the_mix_tab() {
        let mut a = app(View::Page);
        let _ = a.update(Message::Act(Action::ShowMix));
        let mx = |m: mix::MixMsg| Message::Mix(m);
        let flute = a.editor.project.tracks[0].id;
        let reverb = a.playback.plugins.iter().position(|p| p.id == "org.corinzia.reverb").unwrap();
        let _ = a.update(mx(mix::MixMsg::AddEffect(flute, reverb)));
        a.sync();
        let node = a.fx.unwrap();
        let _ = a.update(mx(mix::MixMsg::Param(node, 0, 0.9)));
        let _ = a.update(mx(mix::MixMsg::PresetName("Wet".into())));
        let _ = a.update(mx(mix::MixMsg::SavePreset(mix::Target::Effect(node))));
        assert_eq!(a.preset_names, ["Wet"], "{}", a.editor.status);
        let _ = a.update(mx(mix::MixMsg::Param(node, 0, 0.1)));
        let _ = a.update(mx(mix::MixMsg::LoadPreset(mix::Target::Effect(node), "Wet".into())));
        assert_eq!(a.playback.pool.effect(node).unwrap().param_value(0), Some(0.9));
        assert!(a.editor.project.plugin_state.contains_key(&node), "kept with the project");
        let _ = a.update(mx(mix::MixMsg::DeletePreset(mix::Target::Effect(node), "Wet".into())));
        assert!(a.preset_names.is_empty());
        // A program for the flute: in the plan.
        let _ = a.update(mx(mix::MixMsg::Program(flute, "74 Flute".into())));
        assert_eq!(a.editor.project.tracks[0].program, Some(73));
        a.sync();
        let _ = a.update(mx(mix::MixMsg::Program(flute, "None".into())));
        assert_eq!(a.editor.project.tracks[0].program, None);
    }

    #[test]
    fn audio_clips_are_added_moved_trimmed_faded_copied_and_deleted() {
        use arrange::{ArrangeEvent as E, ClipEdit};
        let mut a = app(View::Page);
        let _ = a.update(Message::Act(Action::ShowMix));
        let rate = a.playback.sample_rate;
        // Ten seconds of sound, already decoded under its name.
        a.playback.pool.media.insert("tone.wav", corinzia_audio::media::AudioData::from_channels(vec![vec![0.5; (rate * 10.0) as usize]], rate));
        a.editor.caret = Frac::int(1); // bar 2 in 4/4: four quarters in
        let _ = a.update(Message::AudioChosen(0, Some("tone.wav".into()))); // no such track: a new one
        let (track, clip) = a.selected_clip.expect("a clip, selected");
        let t = a.editor.project.track(track).unwrap();
        assert!(t.audio && t.name == "tone" && t.clips.len() == 1);
        let sec = |a: &App, q: f64| arrange::seconds_of(a.movement_section(), q);
        let c = |a: &App| a.editor.clip(track, clip).unwrap().clone();
        assert!((c(&a).at - sec(&a, 4.0)).abs() < 1e-9, "at the caret");
        // Drawn on its lane, ten seconds long.
        a.sync();
        let b = a.arrange_clips.iter().find(|b| b.clip == clip).unwrap();
        assert!((b.start_q - 4.0).abs() < 1e-6 && (b.length - 10.0).abs() < 1e-9);
        assert!(a.arrange.lanes[b.lane].audio);

        let ar = |a: &mut App, e: E| {
            let _ = a.update(Message::Arrange(e));
        };
        // Move it to quarter 8: one undo step for the whole drag; playback
        // rebuilt once, when let go.
        let music = a.editor.music_revision;
        ar(&mut a, E::ClipPress { track, clip, copy: false });
        ar(&mut a, E::ClipDrag { edit: ClipEdit::Move, q: 6.0 });
        ar(&mut a, E::ClipDrag { edit: ClipEdit::Move, q: 8.0 });
        assert_eq!(a.editor.music_revision, music, "not while dragging");
        ar(&mut a, E::ClipRelease);
        assert!(a.editor.music_revision > music);
        assert!((c(&a).at - sec(&a, 8.0)).abs() < 1e-9);
        a.editor.undo();
        assert!((c(&a).at - sec(&a, 4.0)).abs() < 1e-9, "one undo puts it back");
        a.editor.redo();

        // Trim the front by a quarter: the sound stays where it was.
        let before = c(&a);
        ar(&mut a, E::ClipPress { track, clip, copy: false });
        ar(&mut a, E::ClipDrag { edit: ClipEdit::TrimStart, q: 9.0 });
        ar(&mut a, E::ClipRelease);
        let d = sec(&a, 9.0) - before.at;
        let now = c(&a);
        assert!((now.at - sec(&a, 9.0)).abs() < 1e-9 && (now.offset - d).abs() < 1e-9 && (now.length - (10.0 - d)).abs() < 1e-9);
        // Trimming the end past the file stops at the file's end.
        ar(&mut a, E::ClipPress { track, clip, copy: false });
        ar(&mut a, E::ClipDrag { edit: ClipEdit::TrimEnd, q: 400.0 });
        ar(&mut a, E::ClipRelease);
        assert!((c(&a).length - (10.0 - d)).abs() < 1e-9);
        // Fades from the corners.
        ar(&mut a, E::ClipPress { track, clip, copy: false });
        ar(&mut a, E::ClipDrag { edit: ClipEdit::FadeIn, q: 10.0 });
        ar(&mut a, E::ClipRelease);
        assert!((c(&a).fade_in - (sec(&a, 10.0) - now.at)).abs() < 1e-9);

        // Ctrl+drag: a copy moves, the original stays.
        let at = c(&a).at;
        ar(&mut a, E::ClipPress { track, clip, copy: true });
        ar(&mut a, E::ClipRelease);
        assert_eq!(a.editor.project.track(track).unwrap().clips.len(), 1, "a click alone copies nothing");
        ar(&mut a, E::ClipPress { track, clip, copy: true });
        ar(&mut a, E::ClipDrag { edit: ClipEdit::Move, q: 20.0 });
        ar(&mut a, E::ClipRelease);
        let clips = &a.editor.project.track(track).unwrap().clips;
        assert_eq!(clips.len(), 2);
        assert!(clips.iter().any(|x| x.id == clip && x.at == at));
        let copy = a.selected_clip.unwrap().1;
        assert_ne!(copy, clip);

        // Delete removes the selected clip.
        let _ = a.update(Message::Key(iced::keyboard::Event::KeyPressed {
            key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Delete),
            modified_key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Delete),
            physical_key: iced::keyboard::key::Physical::Unidentified(iced::keyboard::key::NativeCode::Unidentified),
            location: iced::keyboard::Location::Standard,
            modifiers: Default::default(),
            text: None,
            repeat: false,
        }));
        assert_eq!(a.editor.project.track(track).unwrap().clips.len(), 1);
        assert!(a.selected_clip.is_none());

        // The plan plays it: a node with the clip.
        a.sync();
        assert!(a.playback.clip_errors.is_empty(), "{:?}", a.playback.clip_errors);
    }

    #[test]
    fn a_clip_cued_on_a_staff_is_engraved_there() {
        let mut a = app(View::Page);
        let rate = a.playback.sample_rate;
        a.playback.pool.media.insert("rain.wav", corinzia_audio::media::AudioData::from_channels(vec![vec![0.1; (rate * 3.0) as usize]], rate));
        a.editor.caret = Frac::new(1, 2);
        let _ = a.update(Message::AudioChosen(0, Some("rain.wav".into())));
        let (_, clip) = a.selected_clip.unwrap();
        let drawn = |a: &App| a.pages.iter().flat_map(|p| p.items.iter()).filter(|i| i.id() == clip).cloned().collect::<Vec<Item>>();
        a.sync();
        assert!(drawn(&a).is_empty(), "not cued: not in the score");
        let player = a.editor.project.players[1].name.clone();
        let _ = a.update(Message::Mix(mix::MixMsg::ClipCue(player)));
        a.sync();
        let items = drawn(&a);
        assert!(items.iter().any(|i| matches!(i, Item::Text { text, .. } if text == "rain ▸")), "{items:?}");
        assert!(!items.iter().any(|i| matches!(i, Item::Line { .. })), "no line yet");
        let _ = a.update(Message::Mix(mix::MixMsg::ClipCueLine));
        a.sync();
        assert!(drawn(&a).iter().filter(|i| matches!(i, Item::Line { .. })).count() >= 2, "a line and its hook");
        // On the second player's staff: below the first staff's top.
        let first = a.pages[0].staves.iter().find(|s| s.part == 0).unwrap().top;
        let y = drawn(&a).iter().find_map(|i| match i { Item::Text { at, .. } => Some(at.y), _ => None }).unwrap();
        assert!(y > first + 4.0, "{y} under the first staff ({first})");
    }

    #[test]
    fn buses_route_into_buses_without_loops_and_folders_fold_in_the_arrange_view() {
        let mut a = app(View::Page);
        let mx = |a: &mut App, m: mix::MixMsg| {
            let _ = a.update(Message::Mix(m));
        };
        mx(&mut a, mix::MixMsg::AddBus);
        mx(&mut a, mix::MixMsg::AddBus);
        let (b1, b2) = (a.editor.project.buses[0].id, a.editor.project.buses[1].id);
        let name = |a: &App, b: u64| a.editor.project.bus(b).unwrap().name.clone();
        let n2 = name(&a, b2);
        mx(&mut a, mix::MixMsg::BusOutput(b1, n2));
        assert_eq!(a.editor.project.bus(b1).unwrap().output, Some(b2));
        let n1 = name(&a, b1);
        mx(&mut a, mix::MixMsg::BusOutput(b2, n1));
        assert_eq!(a.editor.project.bus(b2).unwrap().output, None, "a loop is refused");
        assert!(a.editor.status.contains("cannot"));
        mx(&mut a, mix::MixMsg::Solo(b1));
        assert!(a.editor.project.bus(b1).unwrap().solo);
        // Removing a bus: what went into it goes to the master.
        mx(&mut a, mix::MixMsg::RemoveBus(b2));
        assert_eq!(a.editor.project.bus(b1).unwrap().output, None);

        // Folders: the first and third tracks in "Winds", shown together under its header.
        mx(&mut a, mix::MixMsg::AddAudioTrack);
        let ids: Vec<u64> = a.editor.project.tracks.iter().map(|t| t.id).collect();
        assert!(ids.len() >= 3);
        mx(&mut a, mix::MixMsg::Folder(ids[0], "Winds".into()));
        mx(&mut a, mix::MixMsg::Folder(ids[2], "Winds".into()));
        mx(&mut a, mix::MixMsg::Color(ids[1], Some(mix::TRACK_COLORS[6])));
        a.sync();
        use arrange::Row;
        assert_eq!(a.arrange.rows[..4], [Row::Folder { name: "Winds".into(), collapsed: false, tracks: 2 }, Row::Lane(0), Row::Lane(2), Row::Lane(1)]);
        assert_eq!(a.arrange.lanes[1].color, mix::color_of(mix::TRACK_COLORS[6]));
        let _ = a.update(Message::Arrange(arrange::ArrangeEvent::ToggleFolder("Winds".into())));
        a.sync();
        assert_eq!(a.arrange.rows[..2], [Row::Folder { name: "Winds".into(), collapsed: true, tracks: 2 }, Row::Lane(1)]);
        assert_eq!((a.arrange.row_of[0], a.arrange.row_of[2], a.arrange.row_of[1]), (None, None, Some(1)));
    }

    #[test]
    fn the_cycle_is_set_from_a_passage_or_the_ruler_and_toggled() {
        let mut a = app(View::Page);
        let _ = a.act(Action::Cycle);
        assert!(a.playback.cycle.is_none() && a.editor.status.contains("Select a passage"));
        // Bars 2 and 3 (4/4): quarters 4 to 12.
        let lane = 0;
        a.editor.select_bar(lane, Frac::int(1), false);
        a.editor.select_bar(lane, Frac::int(2), true);
        let _ = a.act(Action::Cycle);
        let m = a.editor.movement().unwrap().id;
        assert_eq!(a.playback.cycle, Some((m, 4.0, 12.0)));
        assert!(a.playback.cycling && a.editor.status.contains("bars 2–3"), "{}", a.editor.status);
        // Alt+L with nothing selected: off, and on again.
        a.editor.clear_selection();
        let alt_l = keys::map(&iced::keyboard::Key::Character("l".into()), iced::keyboard::Modifiers::ALT, false);
        assert_eq!(alt_l, Some(Action::Cycle));
        let _ = a.act(Action::Cycle);
        assert!(!a.playback.cycling);
        let _ = a.act(Action::Cycle);
        assert!(a.playback.cycling);
        // Shift+drag on the arrange view's ruler sets it anew.
        let _ = a.update(Message::Arrange(arrange::ArrangeEvent::Cycle { from: 2.0, to: 6.0 }));
        assert_eq!(a.playback.cycle, Some((m, 2.0, 6.0)));
    }

    #[test]
    fn audio_is_exported_over_several_frames_and_can_be_stopped() {
        let dir = std::env::temp_dir().join(format!("corinzia-app-bounce-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut a = app(View::Page);
        let path = dir.join("mix");
        let _ = a.update(Message::ExportTo(files::Format::Audio, Some(path.clone())));
        assert!(a.bounce.is_some() && a.editor.status.starts_with("Exporting audio"));
        let mut frames = 0;
        while a.bounce.is_some() {
            let _ = a.update(Message::Frame);
            frames += 1;
            assert!(frames < 10_000);
        }
        assert!(a.editor.status.starts_with("Exported"), "{}", a.editor.status);
        let wav = path.with_extension("wav");
        assert!(std::fs::metadata(&wav).unwrap().len() > 44);
        // Stopped half way: no file.
        let other = dir.join("stopped.wav");
        let _ = a.update(Message::ExportTo(files::Format::Audio, Some(other.clone())));
        let _ = a.update(Message::Frame);
        let _ = a.update(Message::Key(iced::keyboard::Event::KeyPressed {
            key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape),
            modified_key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape),
            physical_key: iced::keyboard::key::Physical::Unidentified(iced::keyboard::key::NativeCode::Unidentified),
            location: iced::keyboard::Location::Standard,
            modifiers: Default::default(),
            text: None,
            repeat: false,
        }));
        assert!(a.bounce.is_none() && !other.exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn strips_meter_their_levels_and_latch_a_clip() {
        let mut a = app(View::Page);
        a.sync();
        a.playback.play_from(a.editor.project.movements[0].id, 0.0);
        render(&mut a, 24000);
        a.playback.tick();
        let first = a.editor.project.tracks[0].id;
        let master = corinzia_model::MASTER;
        assert!(a.playback.level(master).0 > 0.01 && a.playback.level(first).0 > 0.0);
        assert!(!a.playback.clipped(master));
        // Stopped: the levels fall away over the frames that follow.
        a.playback.stop();
        render(&mut a, 256);
        for _ in 0..200 {
            render(&mut a, 256);
            a.playback.tick();
        }
        assert_eq!(a.playback.level(master), (0.0, 0.0));
        assert!(!a.playback.busy(), "no more frames needed");
        // Too loud: the master's light latches until clicked.
        let _ = a.update(Message::Mix(mix::MixMsg::Gain(first, 12.0)));
        let _ = a.update(Message::Mix(mix::MixMsg::Gain(master, 12.0)));
        a.sync();
        a.playback.play_from(a.editor.project.movements[0].id, 0.0);
        render(&mut a, 24000);
        a.playback.tick();
        assert!(a.playback.clipped(master));
        let _ = a.update(Message::Mix(mix::MixMsg::ClearClip(master)));
        assert!(!a.playback.clipped(master));
    }

    #[test]
    fn a_track_shares_another_tracks_instrument_on_a_free_channel() {
        let mut a = app(View::Page);
        let (t0, t1) = (a.editor.project.tracks[0].id, a.editor.project.tracks[1].id);
        a.editor.set_track_plugin(t0, Some(("Corinzia Sine", corinzia_audio::host::BUILTIN, "org.corinzia.sine")));
        a.editor.set_track_plugin(t1, Some(("Corinzia Sine", corinzia_audio::host::BUILTIN, "org.corinzia.sine")));
        let own = a.editor.project.track(t1).unwrap().instrument_node.unwrap();
        let ch0 = a.editor.project.track(t0).unwrap().midi_channel;
        let _ = a.update(Message::Mix(mix::MixMsg::Share(t1, t0)));
        let (n0, n1) = (a.editor.project.track(t0).unwrap().instrument_node, a.editor.project.track(t1).unwrap().instrument_node);
        assert_eq!(n0, n1, "one plugin node");
        assert!(a.editor.project.node(own).is_none(), "its own plugin went");
        assert_ne!(a.editor.project.track(t1).unwrap().midi_channel, ch0, "on another channel");
        a.sync();
        assert!(std::sync::Arc::ptr_eq(a.playback.pool.plugin_of_track(t0).unwrap(), a.playback.pool.plugin_of_track(t1).unwrap()), "one instance");
        a.playback.play_from(a.editor.project.movements[0].id, 0.0);
        assert!(render(&mut a, 24000) > 0.001, "it plays");
        a.editor.undo();
        a.sync();
        assert!(!std::sync::Arc::ptr_eq(a.playback.pool.plugin_of_track(t0).unwrap(), a.playback.pool.plugin_of_track(t1).unwrap()));
    }

    #[test]
    fn a_clip_is_grabbed_by_its_body_edges_and_corners() {
        let content = arrange::Content { lanes: vec![arrange::Lane { track: 1, name: "A".into(), audio: true, color: iced::Color::WHITE, folder: String::new(), mixer: false, min_key: 60, max_key: 72 }], rows: vec![arrange::Row::Lane(0)], row_of: vec![Some(0)], length_q: 16.0, ..Default::default() };
        let block = arrange::ClipBlock { lane: 0, track: 1, clip: 2, name: "x".into(), start_q: 2.0, end_q: 6.0, fade_in_q: 2.0, fade_out_q: 6.0, muted: false, data: None, offset: 0.0, length: 2.0 };
        let cache = canvas::Cache::new();
        let sel = HashSet::new();
        let clips = [block];
        let v = arrange::ArrangeView { content: &content, pixels_per_quarter: 40.0, scroll_x: 0.0, lane_height: 56.0, selected: &sel, playhead: None, cache: &cache, curves: &[None], clips: &clips, selected_clip: None, cycle: None };
        let r = v.clip_rect(&clips[0]);
        let at = |x: f32, y: f32| v.clip_at(iced::Point::new(x, y)).map(|(_, e)| e);
        let mid_y = r.y + r.height / 2.0;
        assert_eq!(at(r.x + r.width / 2.0, mid_y), Some(arrange::ClipEdit::Move));
        assert_eq!(at(r.x + 2.0, mid_y), Some(arrange::ClipEdit::TrimStart));
        assert_eq!(at(r.x + r.width - 2.0, mid_y), Some(arrange::ClipEdit::TrimEnd));
        assert_eq!(at(r.x + 4.0, r.y + 3.0), Some(arrange::ClipEdit::FadeIn));
        assert_eq!(at(r.x + r.width - 4.0, r.y + 3.0), Some(arrange::ClipEdit::FadeOut));
        assert_eq!(at(r.x - 10.0, mid_y), None);
    }

    #[test]
    fn automation_lanes_in_the_arrange_view() {
        let mut a = app(View::Page);
        let _ = a.update(Message::Act(Action::ShowMix));
        let flute = a.editor.project.tracks[0].id;
        let mx = |m: mix::MixMsg| Message::Mix(m);
        let _ = a.update(mx(mix::MixMsg::AutoShow(flute, "Gain".into())));
        a.sync();
        assert_eq!(a.arrange_curves[0].as_ref().map(|c| (c.min, c.max, c.points.len())), Some((fader::MIN_DB, 6.0, 0)));
        let ar = |e: arrange::ArrangeEvent| Message::Arrange(e);
        let _ = a.update(ar(arrange::ArrangeEvent::AutoAdd { lane: 0, q: 0.0, value: 0.0 }));
        let _ = a.update(ar(arrange::ArrangeEvent::AutoAdd { lane: 0, q: 8.0, value: -20.0 }));
        // A drag: one undo step.
        let _ = a.update(ar(arrange::ArrangeEvent::AutoMove { lane: 0, index: 1, q: 6.0, value: -24.0 }));
        let _ = a.update(ar(arrange::ArrangeEvent::AutoMove { lane: 0, index: 1, q: 7.0, value: -30.0 }));
        let _ = a.update(ar(arrange::ArrangeEvent::AutoRelease));
        a.sync();
        let lane = a.editor.auto_lane(corinzia_model::AutoTarget::Gain(flute)).unwrap().clone();
        assert_eq!(lane.points.iter().map(|p| (p.at, p.value)).collect::<Vec<_>>(), [(Frac::ZERO, 0.0), (Frac::new(7, 4), -30.0)]);
        a.editor.undo();
        assert_eq!(a.editor.auto_lane(corinzia_model::AutoTarget::Gain(flute)).unwrap().points[1].value, -20.0);
        let _ = a.update(ar(arrange::ArrangeEvent::AutoDelete { lane: 0, index: 1 }));
        let _ = a.update(ar(arrange::ArrangeEvent::AutoDelete { lane: 0, index: 0 }));
        assert!(a.editor.auto_lane(corinzia_model::AutoTarget::Gain(flute)).is_none(), "an empty lane goes");
        // Plugin parameters are offered too; dynamics as CC11.
        let gain = a.playback.plugins.iter().position(|p| p.id == "org.corinzia.gain").unwrap();
        let _ = a.update(mx(mix::MixMsg::AddEffect(flute, gain)));
        a.sync();
        let labels: Vec<String> = mix::auto_options(&a.editor.project, &a.playback, flute).into_iter().map(|o| o.0).collect();
        assert!(labels.contains(&"Corinzia Gain: Gain".to_string()), "{labels:?}");
        let _ = a.update(mx(mix::MixMsg::DynamicsCc(flute, mix::DYNAMICS_CC[2].into())));
        assert_eq!(a.editor.project.tracks[0].dynamics_cc, Some(11));
        let _ = a.update(mx(mix::MixMsg::AutoShow(flute, "Off".into())));
        a.sync();
        assert!(a.arrange_curves[0].is_none());
    }

    #[test]
    fn expression_maps_from_the_mix_tab() {
        let mut a = app(View::Page);
        let _ = a.update(Message::Act(Action::ShowMix));
        let flute = a.editor.project.tracks[0].id;
        let mx = |m: mix::MixMsg| Message::Mix(m);
        let _ = a.update(mx(mix::MixMsg::Select(flute)));
        let _ = a.update(mx(mix::MixMsg::TechAdd(flute)));
        let _ = a.update(mx(mix::MixMsg::TechAdd(flute)));
        let tech = |a: &App| a.editor.project.tracks[0].techniques.clone();
        assert_eq!(tech(&a).iter().map(|t| t.technique.as_str()).collect::<Vec<_>>(), ["ord.", "pizz."]);
        use corinzia_model::SwitchAction::*;
        let _ = a.update(mx(mix::MixMsg::TechEdit(flute, 1, mix::TechField::Kind, "Program".into())));
        let _ = a.update(mx(mix::MixMsg::TechEdit(flute, 1, mix::TechField::Value, "46 Pizzicato Strings".into())));
        let _ = a.update(mx(mix::MixMsg::TechEdit(flute, 0, mix::TechField::Value, "D1".into())));
        let _ = a.update(mx(mix::MixMsg::TechEdit(flute, 0, mix::TechField::Kind, "Controller".into())));
        let _ = a.update(mx(mix::MixMsg::TechEdit(flute, 0, mix::TechField::Value, "CC58".into())));
        let _ = a.update(mx(mix::MixMsg::TechEdit(flute, 0, mix::TechField::Value2, "64".into())));
        assert_eq!(tech(&a).iter().map(|t| t.action).collect::<Vec<_>>(), [Cc(58, 64), Program(45)]);
        let _ = a.update(mx(mix::MixMsg::TechEdit(flute, 1, mix::TechField::Kind, "Channel".into())));
        let _ = a.update(mx(mix::MixMsg::TechEdit(flute, 1, mix::TechField::Value, "5".into())));
        assert_eq!(tech(&a)[1].action, Channel(4));
        let _ = a.update(mx(mix::MixMsg::TechRemove(flute, 0)));
        assert_eq!(tech(&a).len(), 1);
        a.sync();
        assert_eq!(mix::key_name(60), "C4");
    }

    #[test]
    fn a_midi_keyboard_enters_notes_and_chords() {
        let mut a = app(View::Page);
        a.editor.select(None);
        a.editor.set_input_mode(true);
        a.editor.caret_lane = 0;
        a.editor.caret = Frac::ZERO;
        let first = a.editor.movement().unwrap().lanes[0].voices[0].events.len();
        // C E G held together (a chord), let go; then D alone.
        let path = std::env::temp_dir().join(format!("corinzia-midi-{}", std::process::id()));
        std::fs::write(&path, [0x90, 72, 100, 76, 90, 79, 80, 0x80, 72, 0, 76, 0, 79, 0, 0x90, 74, 70, 74, 0]).unwrap();
        a.open_midi(&path.to_string_lossy());
        assert!(a.midi.is_some());
        let mut tries = 0;
        while a.editor.movement().unwrap().lanes[0].voices[0].events.iter().filter(|e| e.heads.len() == 3).count() == 0 || a.midi_held.len() > 0 {
            std::thread::sleep(std::time::Duration::from_millis(10));
            a.midi_poll();
            tries += 1;
            assert!(tries < 200, "the keys arrive");
        }
        std::thread::sleep(std::time::Duration::from_millis(30));
        a.midi_poll();
        let evs = a.editor.movement().unwrap().lanes[0].voices[0].events.clone();
        let names = |e: &corinzia_model::Event| e.heads.iter().map(|h| editor::pitch_name(h.pitch)).collect::<Vec<_>>().join("+");
        assert_eq!(names(&evs[0]), "C5+E5+G5");
        assert_eq!(names(&evs[1]), "D5");
        assert!(evs.len() >= first.min(2));
        // Spelling in the key: C# in D major, Db in F major, Bb in F major.
        assert_eq!(editor::pitch_name(editor::spell_key(61, 2)), "C♯4");
        assert_eq!(editor::pitch_name(editor::spell_key(61, -1)), "D♭4");
        assert_eq!(editor::pitch_name(editor::spell_key(70, -1)), "B♭4");
        assert_eq!(editor::pitch_name(editor::spell_key(60, -7)), "C4", "not in C-flat major: natural");
        a.open_midi("");
        assert!(a.midi.is_none());
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn a_strip_click_shows_the_plugin_editor() {
        let mut a = app(View::Page);
        let _ = a.update(Message::Act(Action::ShowMix));
        let track = a.editor.project.tracks[0].id;
        // The built-in instrument has no editor: the strip says so.
        let _ = a.update(Message::Mix(mix::MixMsg::Editor(track)));
        assert_eq!(a.strip, Some(track), "the click selects the strip");
        assert!(!a.playback.pool.editor_is_open(track));
        assert_eq!(a.editor.status, "Corinzia Sine has no editor of its own");
        assert!(!a.playback.editors_busy());
        // Selecting by name leaves editors alone.
        let cello = a.editor.project.tracks[1].id;
        let _ = a.update(Message::Mix(mix::MixMsg::Select(cello)));
        assert_eq!(a.strip, Some(cello));
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
