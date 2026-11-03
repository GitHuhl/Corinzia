//! The application: its state (App), its messages, and the work shared by
//! update and view. Each area of App lives in a module of its own.

use crate::{arrange, browser, demo, editor, files, keys, midi_in, mix, panels, playback, prefs, presets, roll, score_view, theme};
use crate::cli::*;
use crate::dialogs::*;
use crate::LIVE_AUDIO;
use corinzia_layout::{layout_line, layout_pages, DisplayList};
use corinzia_model::{Movement, Project};
use corinzia_score::Frac;
use editor::{Editor, PopoverKind};
use iced::widget::{button, canvas, container, row, stack, text, text_input, Space};
use iced::{Element, Length, Size, Subscription, Task, Theme, Vector};
use keys::Action;
use panels::Panel;
use score_view::{Caret, Highlight, ScoreView, StaffPoint, ViewEvent};
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Instant;

mod actions;
mod document;
mod midi_keys;
mod mixing;
mod piano_roll;
mod update;
mod view;

use midi_keys::*;

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum View {
    Page,
    Panorama,
}

pub(crate) struct Popover {
    pub(crate) kind: PopoverKind,
    pub(crate) input: String,
    pub(crate) error: Option<String>,
    /// The item on the page being retyped (opened by a double-click).
    pub(crate) target: Option<u64>,
    /// The point on the canvas it opens by: what was clicked, else the
    /// selection or the caret.
    pub(crate) anchor: Option<iced::Point>,
}

/// The piano roll's track, notes (as built for a music revision), selection,
/// view, and while a drag is under way the tweaks as they were.
pub(crate) struct RollState {
    pub(crate) content: roll::Roll,
    pub(crate) built: u64,
    pub(crate) movement: usize,
    pub(crate) selected: Vec<corinzia_model::Id>,
    pub(crate) ppq: f32,
    pub(crate) scroll_x: f32,
    pub(crate) top_key: i32,
    pub(crate) origin: Vec<corinzia_model::Tweak>,
    pub(crate) dragged: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tab {
    Score,
    Mix,
}

/// Live audio output (PipeWire) for the windowed app; tests and --export run without.
/// Saving projects (Save, Save As, autosave). Off while Corinzia is a
/// preview not ready for real-world projects: the button is grayed out and
/// the shortcuts explain; export still works.
pub(crate) const SAVING_ENABLED: bool = false;

pub(crate) struct App {
    pub(crate) editor: Editor,
    pub(crate) playback: playback::Playback,
    pub(crate) tab: Tab,
    pub(crate) strip: Option<corinzia_model::Id>, // the Mix tab's selected strip
    pub(crate) fx: Option<corinzia_model::Id>,    // and its selected effect (audio node)
    /// The presets of the plugin shown in the Mix tab, and the name being typed.
    pub(crate) preset_names: Vec<String>,
    pub(crate) preset_name: String,
    /// The automation each track's arrange lane shows (none: notes only),
    /// and the curves drawn for them.
    pub(crate) auto_shown: std::collections::HashMap<corinzia_model::Id, corinzia_model::AutoTarget>,
    pub(crate) arrange_curves: Vec<Option<arrange::Curve>>,
    pub(crate) arrange_clips: Vec<arrange::ClipBlock>,
    /// The piano roll open in the Mix tab.
    pub(crate) roll: Option<RollState>,
    /// An audio export under way.
    pub(crate) bounce: Option<corinzia_audio::bounce::Bounce>,
    /// Folders folded away in the arrange view.
    pub(crate) folded: HashSet<String>,
    /// The clip selected in the arrange view (track, clip), and while it is
    /// dragged, the clip as it was (its length made explicit).
    pub(crate) selected_clip: Option<(corinzia_model::Id, corinzia_model::Id)>,
    pub(crate) clip_origin: Option<corinzia_model::AudioClip>,
    pub(crate) clip_dragged: bool,
    /// Ctrl was held at the press: the first drag step makes the copy.
    pub(crate) clip_copy: bool,
    pub(crate) arrange: arrange::Content,
    pub(crate) arrange_cache: canvas::Cache,
    pub(crate) arrange_ppq: f32,
    pub(crate) arrange_scroll: f32,
    /// Autosave (projects with a file): the revision on disk, and when the
    /// next coalesced save is due.
    pub(crate) saved_revision: u64,
    pub(crate) autosave_at: Option<std::time::Instant>,
    pub(crate) saving: bool,
    pub(crate) undo_tip: String,
    pub(crate) new_score: Option<NewScore>,
    pub(crate) players: Option<PlayersDialog>,
    pub(crate) layout: Option<LayoutDialog>,
    /// The preferences in use, and the card that edits them.
    pub(crate) prefs: prefs::Prefs,
    pub(crate) prefs_dialog: Option<PrefsDialog>,
    /// The MIDI keyboard, when one is chosen, and the keys held on it.
    pub(crate) midi: Option<midi_in::Input>,
    pub(crate) midi_held: Vec<u8>,
    /// Corinzia's own file browser, when the system's dialog cannot open.
    pub(crate) browser: Option<browser::Browser>,
    /// What waits for "discard unsaved changes?" to be answered.
    pub(crate) confirm: Option<Box<Message>>,
    pub(crate) path: Option<PathBuf>,
    pub(crate) view: View,
    pub(crate) pages: Vec<DisplayList>,
    pub(crate) engraved: Option<u64>, // editor revision of `pages`
    pub(crate) engrave_ms: f64,
    pub(crate) zoom: f32,
    pub(crate) pan: Vector,
    pub(crate) selected: HashSet<u64>,
    pub(crate) cache: canvas::Cache,
    pub(crate) panel: Panel,
    pub(crate) popover: Option<Popover>,
    pub(crate) screenshot: Option<PathBuf>,
    pub(crate) frames: u32,
    /// The window's size (logical pixels), for keeping the selection in view.
    pub(crate) window: Size,
    /// The keyboard shortcut overview (F1).
    pub(crate) help: bool,
    /// See [`SAVING_ENABLED`] (tests turn it on).
    pub(crate) saving_enabled: bool,
    /// The Export menu is open.
    pub(crate) export_menu: bool,
    /// The part shown (a player), or the full score.
    pub(crate) part: Option<corinzia_model::Id>,
    /// The lane each engraved staff shows.
    pub(crate) staff_lanes: Vec<usize>,
    /// Every movement on the pages, one after another (Page view).
    pub(crate) all_movements: bool,
    /// The movement each page shows.
    pub(crate) page_movements: Vec<usize>,
    /// The movement edited when the view was last brought up to date.
    pub(crate) shown_movement: usize,
}

#[derive(Debug, Clone)]
pub(crate) enum Message {
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
    Roll(roll::RollEvent),
    /// Close the piano roll; reset the selected notes' tweaks.
    RollClose,
    RollReset,
}

pub(crate) const POPOVER_INPUT: &str = "popover-input";
pub(crate) const NEW_TITLE: &str = "new-score-title";
pub(crate) const PLAYERS_ADD: &str = "players-add";

/// The diatonic number (C4 = 28) of a staff's top line for a clef.
pub(crate) fn top_line(clef: &str) -> i32 {
    match clef {
        "bass" => 26,      // A3
        "alto" => 32,      // G4
        "tenor" => 30,     // E4
        "treble_8" => 31,  // F4
        "bass_8" => 19,    // A2
        _ => 38,           // treble, percussion: F5
    }
}

pub(crate) fn clef_at(project: &Project, m: &Movement, lane: usize, at: Frac) -> String {
    let l = &m.lanes[lane];
    if let Some(c) = l.clefs.iter().filter(|c| c.at <= at).last() {
        return c.clef.clone();
    }
    l.instrument_at(at).and_then(|i| project.instrument(i)).map(|i| i.clef.clone()).unwrap_or_else(|| "treble".into())
}

pub(crate) fn quarters(f: Frac) -> f64 {
    f.num as f64 * 4.0 / f.den as f64
}

impl App {
    pub(crate) fn boot() -> (App, Task<Message>) {
        (App::from_args(args()), Task::none())
    }

    /// The app as the command line asks for it (file, view, scripted keys);
    /// no window is needed, so `--export` runs headless.
    pub(crate) fn from_args(a: Args) -> App {
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

    pub(crate) fn new(project: Project, path: Option<PathBuf>, view: View, status: String) -> App {
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
    pub(crate) fn with_playback(project: Project, path: Option<PathBuf>, view: View, status: String, mut playback: playback::Playback) -> App {
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
            roll: None,
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

    pub(crate) fn title(&self) -> String {
        let p = &self.editor.project;
        let name = if p.title.is_empty() { "Untitled" } else { &p.title };
        format!("{}{name} — Corinzia", if self.editor.modified { "• " } else { "" })
    }

    /// The pages showing movement `mi`.
    pub(crate) fn pages_of(&self, mi: usize) -> std::ops::Range<usize> {
        let first = self.page_movements.iter().position(|&m| m == mi).unwrap_or(0);
        let n = self.page_movements.iter().filter(|&&m| m == mi).count();
        first..first + n
    }

    /// Native engraving of the current movement (milliseconds).
    pub(crate) fn engrave(&mut self) {
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

    /// Brings the engraving and the selection colours up to date.
    pub(crate) fn sync(&mut self) {
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
        self.sync_roll();
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

    /// The score canvas's size: the window less the toolbar, side panel and
    /// status bar (and their hairlines).
    pub(crate) fn viewport(&self) -> Size {
        Size::new((self.window.width - panels::WIDTH - 1.0).max(100.0), (self.window.height - 47.0 - 27.0).max(100.0))
    }

    /// Pans so that the selection (or the caret's note) is in view.
    pub(crate) fn reveal(&mut self) {
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
    pub(crate) fn show_movement_pages(&mut self) {
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
    pub(crate) fn fit_width(&mut self) {
        let Some(page) = self.pages.first() else { return };
        let vp = self.viewport();
        self.zoom = ((vp.width - 48.0) / (page.width * score_view::BASE_PX)).clamp(0.25, 8.0);
        self.pan = Vector::new(24.0, self.pan.y.min(24.0));
    }

    pub(crate) fn subscription(&self) -> Subscription<Message> {
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

/// Ticks for plugin editors (their timers, descriptors and windows), about
/// 60 a second, whatever the main window is doing. The thread ends when the
/// subscription is dropped.
pub(crate) fn plugin_clock() -> impl iced::futures::Stream<Item = Message> {
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
pub(crate) fn segmented(opts: Vec<(&'static str, bool, Message)>) -> Element<'static, Message> {
    let mut r = row![].spacing(2);
    for (label, on, m) in opts {
        r = r.push(button(text(label).size(13)).padding([3, 12]).style(theme::segment(on)).on_press(m));
    }
    container(r).padding(2).style(theme::trough).into()
}
