//! The arrange view (Mix tab): one lane per DAW track over time, notes as
//! blocks placed by pitch within the lane (a mini piano roll), a bar ruler
//! that locates the playhead, selection shared with the score.

use corinzia_audio::media::AudioData;
use corinzia_audio::snapshot::Section;
use corinzia_model::{Id, Project};
use std::sync::Arc;
use iced::mouse;
use iced::widget::canvas::{self, Event, Frame, Geometry, Path, Stroke, Text};
use iced::{Color, Point, Rectangle, Renderer, Size, Theme};

pub const RULER: f32 = 24.0;
pub const HEADER: f32 = 150.0;

#[derive(Clone, Debug, Default)]
pub struct Lane {
    pub track: Id,
    pub name: String,
    /// An audio track's lane: clips, no notes.
    pub audio: bool,
    pub color: Color,
    pub folder: String,
    pub min_key: i32,
    pub max_key: i32,
}

#[derive(Clone, Copy, Debug)]
pub struct Block {
    pub lane: usize,
    pub start_q: f64,
    pub length_q: f64,
    pub key: i32,
    pub element: Id,
}

/// A row of the arrange view: a folder's header, or a track's lane.
#[derive(Clone, Debug, PartialEq)]
pub enum Row {
    Folder { name: String, collapsed: bool, tracks: usize },
    Lane(usize),
}

#[derive(Clone, Debug, Default)]
pub struct Content {
    pub lanes: Vec<Lane>,
    /// The rows shown, top to bottom, and each lane's row (none: folded away).
    pub rows: Vec<Row>,
    pub row_of: Vec<Option<usize>>,
    pub blocks: Vec<Block>,
    pub bars: Vec<f64>, // bar line quarters, with the final barline
    pub length_q: f64,
}

/// The current movement's notes, per track that sounds them.
pub fn content(project: &Project, movement: usize) -> Content {
    let mut c = Content { lanes: project.tracks.iter().map(|t| Lane { track: t.id, name: t.name.clone(), audio: t.audio, color: crate::mix::track_color(project, t.id), folder: t.folder.clone(), min_key: i32::MAX, max_key: i32::MIN }).collect(), ..Default::default() };
    let Some(m) = project.movements.get(movement) else { return c };
    let q = |f: corinzia_score::Frac| f.num as f64 * 4.0 / f.den as f64;
    for lane in &m.lanes {
        for v in &lane.voices {
            for e in v.events.iter().filter(|e| !e.is_rest()) {
                let Some(track) = lane.instrument_at(e.start).and_then(|i| project.instrument(i)).and_then(|i| i.track) else { continue };
                let Some(li) = c.lanes.iter().position(|l| l.track == track) else { continue };
                for h in &e.heads {
                    let key = h.pitch.midi_key();
                    c.lanes[li].min_key = c.lanes[li].min_key.min(key);
                    c.lanes[li].max_key = c.lanes[li].max_key.max(key);
                    c.blocks.push(Block { lane: li, start_q: q(e.start), length_q: q(e.duration), key, element: h.id });
                }
            }
        }
    }
    for l in c.lanes.iter_mut() {
        if l.min_key > l.max_key {
            (l.min_key, l.max_key) = (60, 72);
        }
        // At least an octave of room, centred.
        let pad = (12 - (l.max_key - l.min_key)).max(0);
        l.min_key -= pad / 2 + 1;
        l.max_key += pad - pad / 2 + 1;
    }
    c.bars = corinzia_edit::bar_starts(m).into_iter().map(q).collect();
    c.length_q = q(m.length);
    c.bars.push(c.length_q);
    c
}

/// Brings the lanes' names, colours and folders up to date and lays out
/// the rows: a folder's tracks together under its header (where the first
/// of them is), folded away when `collapsed` names it.
pub fn layout_rows(c: &mut Content, project: &Project, collapsed: &std::collections::HashSet<String>) {
    for l in c.lanes.iter_mut() {
        if let Some(t) = project.track(l.track) {
            l.name = t.name.clone();
            l.folder = t.folder.clone();
            l.color = crate::mix::track_color(project, t.id);
        }
    }
    c.rows.clear();
    c.row_of = vec![None; c.lanes.len()];
    let mut done: Vec<&str> = Vec::new();
    for (i, l) in c.lanes.iter().enumerate() {
        if l.folder.is_empty() {
            c.row_of[i] = Some(c.rows.len());
            c.rows.push(Row::Lane(i));
            continue;
        }
        if done.contains(&l.folder.as_str()) {
            continue;
        }
        done.push(&l.folder);
        let members: Vec<usize> = (0..c.lanes.len()).filter(|&j| c.lanes[j].folder == l.folder).collect();
        let shut = collapsed.contains(&l.folder);
        c.rows.push(Row::Folder { name: l.folder.clone(), collapsed: shut, tracks: members.len() });
        if !shut {
            for j in members {
                c.row_of[j] = Some(c.rows.len());
                c.rows.push(Row::Lane(j));
            }
        }
    }
}

// ---------------------------------------------------------------- clips --

/// Seconds after a movement's start, as written quarters of the movement
/// (the first pass through repeats; past the end, at the last tempo).
pub fn quarter_of(sec: Option<&Section>, seconds: f64) -> f64 {
    let Some(sec) = sec else { return seconds * 2.0 };
    let (perf_end, written_end) = ends(sec);
    let p = sec.tempo.quarter_at(seconds.max(0.0));
    if p >= perf_end {
        return written_end + (p - perf_end);
    }
    sec.written_quarter(p)
}

/// Written quarters of a movement -> seconds after its start.
pub fn seconds_of(sec: Option<&Section>, q: f64) -> f64 {
    let Some(sec) = sec else { return q / 2.0 };
    let (perf_end, written_end) = ends(sec);
    let p = if q >= written_end { perf_end + (q - written_end) } else { sec.performed_from_written(q.max(0.0)) };
    sec.tempo.seconds_at(p)
}

fn ends(sec: &Section) -> (f64, f64) {
    let perf = sec.spans.last().map(|s| s.performed_q + s.length_q).unwrap_or(0.0);
    let written = sec.spans.iter().map(|s| s.written_q + s.length_q).fold(0.0, f64::max);
    (perf, written)
}

/// A clip as drawn on its lane.
#[derive(Clone, Debug)]
pub struct ClipBlock {
    pub lane: usize,
    pub track: Id,
    pub clip: Id,
    pub name: String,
    pub start_q: f64,
    pub end_q: f64,
    /// Where the fade in ends and the fade out starts.
    pub fade_in_q: f64,
    pub fade_out_q: f64,
    pub muted: bool,
    /// The sound (once decoded) and the part of it played, in seconds.
    pub data: Option<Arc<AudioData>>,
    pub offset: f64,
    pub length: f64,
}

/// A clip's length in seconds: as set, or the rest of its file.
pub fn clip_length(c: &corinzia_model::AudioClip, data: Option<&AudioData>) -> f64 {
    let file = data.map(|d| (d.seconds() - c.offset).max(0.0));
    match (c.length > 0.0, file) {
        (true, Some(f)) => c.length.min(f),
        (true, None) => c.length,
        (false, f) => f.unwrap_or(4.0), // not decoded yet: a placeholder
    }
}

/// The current movement's clips, per audio track.
pub fn clips(project: &Project, movement: usize, sec: Option<&Section>, media: &corinzia_audio::media::Media) -> Vec<ClipBlock> {
    let Some(m) = project.movements.get(movement) else { return vec![] };
    let mut out = Vec::new();
    for (lane, t) in project.tracks.iter().enumerate().filter(|(_, t)| t.audio) {
        for c in t.clips.iter().filter(|c| c.movement == m.id) {
            let data = media.loaded(&c.file);
            let length = clip_length(c, data.as_deref());
            let q = |s: f64| quarter_of(sec, s);
            out.push(ClipBlock {
                lane,
                track: t.id,
                clip: c.id,
                name: c.name.clone(),
                start_q: q(c.at),
                end_q: q(c.at + length),
                fade_in_q: q(c.at + c.fade_in.min(length)),
                fade_out_q: q(c.at + length - c.fade_out.min(length)),
                muted: c.mute,
                data,
                offset: c.offset,
                length,
            });
        }
    }
    out
}

/// What a drag on a clip changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipEdit {
    Move,
    TrimStart,
    TrimEnd,
    FadeIn,
    FadeOut,
}

#[derive(Debug, Clone)]
pub enum ArrangeEvent {
    Select(Option<Id>),
    Locate(f64), // quarters
    Scroll(f32),
    Zoom(f32),
    /// Automation in a lane: a new point, a point dragged, a point deleted,
    /// the drag over.
    AutoAdd { lane: usize, q: f64, value: f64 },
    AutoMove { lane: usize, index: usize, q: f64, value: f64 },
    AutoDelete { lane: usize, index: usize },
    AutoRelease,
    /// A clip pressed (selected; `copy`: Ctrl held, a copy is dragged),
    /// dragged (`q`: the new start, edge or fade point, in quarters), let go.
    ClipPress { track: Id, clip: Id, copy: bool },
    ClipDrag { edit: ClipEdit, q: f64 },
    ClipRelease,
    /// A folder's header clicked: fold it away, or open it.
    ToggleFolder(String),
}

/// A lane's automation as shown: what it drives, its range, its points
/// (quarters, value).
#[derive(Clone, Debug, PartialEq)]
pub struct Curve {
    pub label: String,
    pub min: f64,
    pub max: f64,
    pub points: Vec<(f64, f64)>,
}

/// The canvas's own state: modifiers held, and the point being dragged.
#[derive(Default)]
pub struct State {
    mods: iced::keyboard::Modifiers,
    drag: Option<(usize, usize)>,
    /// A clip being dragged: what changes, and the pointer's distance in
    /// quarters from the clip's start when pressed (for moves).
    clip_drag: Option<(ClipEdit, f64)>,
}

pub struct ArrangeView<'a> {
    pub content: &'a Content,
    pub pixels_per_quarter: f32,
    pub scroll_x: f32,
    pub lane_height: f32,
    pub selected: &'a std::collections::HashSet<u64>,
    pub playhead: Option<f64>,
    pub cache: &'a canvas::Cache,
    /// Per lane: its automation, when shown (clicks there edit it).
    pub curves: &'a [Option<Curve>],
    pub clips: &'a [ClipBlock],
    pub selected_clip: Option<Id>,
}

const PALETTE: [Color; 6] = [
    Color::from_rgb(0.25, 0.47, 0.85),
    Color::from_rgb(0.85, 0.45, 0.20),
    Color::from_rgb(0.30, 0.65, 0.40),
    Color::from_rgb(0.65, 0.35, 0.75),
    Color::from_rgb(0.80, 0.30, 0.40),
    Color::from_rgb(0.20, 0.65, 0.70),
];

impl ArrangeView<'_> {
    fn x(&self, q: f64) -> f32 {
        HEADER + q as f32 * self.pixels_per_quarter - self.scroll_x
    }
    fn quarter_at(&self, x: f32) -> f64 {
        ((x - HEADER + self.scroll_x) / self.pixels_per_quarter).max(0.0) as f64
    }
    /// The top of a lane's row; folded away: far above the view.
    fn lane_y(&self, lane: usize) -> f32 {
        match self.content.row_of.get(lane).copied().flatten() {
            Some(r) => RULER + r as f32 * self.lane_height,
            None => -1.0e6,
        }
    }
    fn row_at(&self, y: f32) -> Option<&Row> {
        let i = ((y - RULER) / self.lane_height).floor();
        if i < 0.0 {
            return None;
        }
        self.content.rows.get(i as usize)
    }
    /// A lane's curve area: (top, height).
    fn curve_band(&self, lane: usize) -> (f32, f32) {
        (self.lane_y(lane) + 6.0, self.lane_height - 12.0)
    }
    fn curve_y(&self, lane: usize, c: &Curve, v: f64) -> f32 {
        let (top, h) = self.curve_band(lane);
        let k = if c.max > c.min { ((v - c.min) / (c.max - c.min)).clamp(0.0, 1.0) } else { 0.5 };
        top + (1.0 - k as f32) * h
    }
    fn curve_value(&self, lane: usize, c: &Curve, y: f32) -> f64 {
        let (top, h) = self.curve_band(lane);
        let k = (1.0 - ((y - top) / h.max(1.0))).clamp(0.0, 1.0) as f64;
        c.min + k * (c.max - c.min)
    }
    /// A position snapped to the sixteenth of a quarter.
    fn snapped(&self, x: f32) -> f64 {
        (self.quarter_at(x) * 16.0).round() / 16.0
    }
    fn lane_at(&self, y: f32) -> Option<usize> {
        match self.row_at(y)? {
            Row::Lane(l) => Some(*l),
            Row::Folder { .. } => None,
        }
    }
    /// The point of a lane's curve under (x, y), if any.
    fn point_at(&self, lane: usize, c: &Curve, p: Point) -> Option<usize> {
        c.points.iter().position(|&(q, v)| (self.x(q) - p.x).abs() < 7.0 && (self.curve_y(lane, c, v) - p.y).abs() < 7.0)
    }

    pub fn clip_rect(&self, c: &ClipBlock) -> Rectangle {
        let y = self.lane_y(c.lane) + 3.0;
        Rectangle::new(Point::new(self.x(c.start_q), y), Size::new(((c.end_q - c.start_q) as f32 * self.pixels_per_quarter).max(4.0), self.lane_height - 6.0))
    }

    /// The clip under a point and what dragging there does: the top
    /// corners fade, the edges trim, the body moves.
    pub fn clip_at(&self, p: Point) -> Option<(&ClipBlock, ClipEdit)> {
        let c = self.clips.iter().rev().find(|c| self.clip_rect(c).contains(p) && p.x > HEADER)?;
        let r = self.clip_rect(c);
        let edge = 6.0f32.min(r.width / 4.0);
        let corner = 12.0f32.min(r.width / 3.0);
        let edit = if p.y < r.y + 10.0 && p.x < r.x + corner {
            ClipEdit::FadeIn
        } else if p.y < r.y + 10.0 && p.x > r.x + r.width - corner {
            ClipEdit::FadeOut
        } else if p.x < r.x + edge {
            ClipEdit::TrimStart
        } else if p.x > r.x + r.width - edge {
            ClipEdit::TrimEnd
        } else {
            ClipEdit::Move
        };
        Some((c, edit))
    }

    /// Quarters snapped to the beat (quarter notes), unless Shift is held.
    fn beat(&self, q: f64, free: bool) -> f64 {
        if free { q.max(0.0) } else { q.round().max(0.0) }
    }

    pub fn block_rect(&self, b: &Block) -> Rectangle {
        let lane = &self.content.lanes[b.lane];
        let span = (lane.max_key - lane.min_key).max(1) as f32;
        let h = (self.lane_height - 8.0) / span;
        let y = self.lane_y(b.lane) + 4.0 + (lane.max_key - b.key) as f32 * h - h / 2.0;
        Rectangle::new(Point::new(self.x(b.start_q), y), Size::new((b.length_q as f32 * self.pixels_per_quarter - 1.0).max(2.0), h.max(3.0)))
    }
}

impl canvas::Program<ArrangeEvent> for ArrangeView<'_> {
    type State = State;

    fn update(&self, state: &mut Self::State, event: &Event, bounds: Rectangle, cursor: mouse::Cursor) -> Option<canvas::Action<ArrangeEvent>> {
        let pos = cursor.position_in(bounds);
        let mods = &mut state.mods;
        match event {
            Event::Mouse(mouse::Event::CursorMoved { .. }) if state.drag.is_some() => {
                let (lane, index) = state.drag?;
                let p = pos?;
                let c = self.curves.get(lane)?.as_ref()?;
                let ev = ArrangeEvent::AutoMove { lane, index, q: self.snapped(p.x.max(HEADER)), value: self.curve_value(lane, c, p.y) };
                Some(canvas::Action::publish(ev).and_capture())
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) if state.clip_drag.is_some() => {
                let (edit, grab) = state.clip_drag?;
                let p = pos?;
                let at = self.quarter_at(p.x.max(HEADER));
                let free = mods.shift();
                let q = match edit {
                    ClipEdit::Move => self.beat(at - grab, free),
                    ClipEdit::TrimStart | ClipEdit::TrimEnd => self.beat(at, free),
                    ClipEdit::FadeIn | ClipEdit::FadeOut => at,
                };
                Some(canvas::Action::publish(ArrangeEvent::ClipDrag { edit, q }).and_capture())
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) if state.clip_drag.is_some() => {
                state.clip_drag = None;
                Some(canvas::Action::publish(ArrangeEvent::ClipRelease).and_capture())
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) if state.drag.is_some() => {
                state.drag = None;
                Some(canvas::Action::publish(ArrangeEvent::AutoRelease).and_capture())
            }
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right)) => {
                let p = pos.filter(|p| p.x > HEADER)?;
                let lane = self.lane_at(p.y)?;
                let c = self.curves.get(lane)?.as_ref()?;
                let index = self.point_at(lane, c, p)?;
                Some(canvas::Action::publish(ArrangeEvent::AutoDelete { lane, index }).and_capture())
            }
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let p = pos?;
                if p.y < RULER && p.x > HEADER {
                    return Some(canvas::Action::publish(ArrangeEvent::Locate(self.quarter_at(p.x))).and_capture());
                }
                if let Some(Row::Folder { name, .. }) = self.row_at(p.y) {
                    return Some(canvas::Action::publish(ArrangeEvent::ToggleFolder(name.clone())).and_capture());
                }
                // A lane showing automation: grab a point, or add one.
                if let Some((lane, c)) = self.lane_at(p.y).filter(|_| p.x > HEADER).and_then(|l| Some((l, self.curves.get(l)?.as_ref()?))) {
                    if let Some(index) = self.point_at(lane, c, p) {
                        state.drag = Some((lane, index));
                        return Some(canvas::Action::capture());
                    }
                    let q = self.snapped(p.x);
                    let index = c.points.partition_point(|x| x.0 <= q);
                    state.drag = Some((lane, index));
                    return Some(canvas::Action::publish(ArrangeEvent::AutoAdd { lane, q, value: self.curve_value(lane, c, p.y) }).and_capture());
                }
                if let Some((c, edit)) = self.clip_at(p) {
                    state.clip_drag = Some((edit, self.quarter_at(p.x) - c.start_q));
                    return Some(canvas::Action::publish(ArrangeEvent::ClipPress { track: c.track, clip: c.clip, copy: mods.control() && edit == ClipEdit::Move }).and_capture());
                }
                let hit = self.content.blocks.iter().rev().find(|b| self.block_rect(b).expand(1.5).contains(p)).map(|b| b.element);
                Some(canvas::Action::publish(ArrangeEvent::Select(hit)).and_capture())
            }
            Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                pos?;
                let (dx, dy) = match delta {
                    mouse::ScrollDelta::Lines { x, y } => (x * 40.0, y * 40.0),
                    mouse::ScrollDelta::Pixels { x, y } => (*x, *y),
                };
                let ev = if mods.control() { ArrangeEvent::Zoom((1.0 + dy / 400.0).clamp(0.5, 2.0)) } else { ArrangeEvent::Scroll(-(dx + dy)) };
                Some(canvas::Action::publish(ev).and_capture())
            }
            Event::Keyboard(iced::keyboard::Event::ModifiersChanged(m)) => {
                *mods = *m;
                None
            }
            _ => None,
        }
    }

    fn draw(&self, _state: &Self::State, renderer: &Renderer, _theme: &Theme, bounds: Rectangle, _cursor: mouse::Cursor) -> Vec<Geometry> {
        let c = self.content;
        let geometry = self.cache.draw(renderer, bounds.size(), |f: &mut Frame| {
            f.fill_rectangle(Point::ORIGIN, bounds.size(), crate::theme::palette().desk);
            f.fill_rectangle(Point::ORIGIN, Size::new(bounds.width, RULER), crate::theme::palette().base);
            // Rows: lanes, and folders' headers.
            for (i, row) in c.rows.iter().enumerate() {
                let y = RULER + i as f32 * self.lane_height;
                if matches!(row, Row::Folder { .. }) {
                    f.fill_rectangle(Point::new(HEADER, y), Size::new(bounds.width - HEADER, self.lane_height), crate::theme::palette().base);
                } else if i % 2 == 1 {
                    f.fill_rectangle(Point::new(HEADER, y), Size::new(bounds.width - HEADER, self.lane_height), crate::theme::palette().surface);
                }
                f.stroke(&Path::line(Point::new(0.0, y + self.lane_height), Point::new(bounds.width, y + self.lane_height)), Stroke::default().with_width(1.0).with_color(crate::theme::palette().line));
            }
            // Bar grid and numbers.
            for (n, &bq) in c.bars.iter().enumerate() {
                let x = self.x(bq);
                if x < HEADER || x > bounds.width {
                    continue;
                }
                f.stroke(&Path::line(Point::new(x, RULER - 8.0), Point::new(x, RULER + c.rows.len() as f32 * self.lane_height)), Stroke::default().with_width(1.0).with_color(crate::theme::palette().line));
                if n + 1 < c.bars.len() {
                    f.fill_text(Text { content: format!("{}", n + 1), position: Point::new(x + 3.0, 4.0), size: iced::Pixels(11.0), color: crate::theme::palette().dim, ..Text::default() });
                }
            }
            // Note blocks.
            for b in c.blocks.iter().filter(|b| c.row_of.get(b.lane).copied().flatten().is_some()) {
                let r = self.block_rect(b);
                if r.x + r.width < HEADER || r.x > bounds.width {
                    continue;
                }
                let color = if self.selected.contains(&b.element) { crate::theme::palette().accent } else { c.lanes[b.lane].color };
                f.fill_rectangle(Point::new(r.x.max(HEADER), r.y), Size::new(r.width - (HEADER - r.x).max(0.0), r.height), color);
            }
            // Clips, with their waveform and fades.
            for clip in self.clips.iter().filter(|x| c.row_of.get(x.lane).copied().flatten().is_some()) {
                draw_clip(f, self, clip, bounds.width);
            }
            // Track headers over everything that scrolled under them.
            f.fill_rectangle(Point::ORIGIN, Size::new(HEADER, bounds.height), crate::theme::palette().surface);
            for (i, row) in c.rows.iter().enumerate() {
                let y = RULER + i as f32 * self.lane_height;
                let l = match row {
                    Row::Folder { name, collapsed, tracks } => {
                        let label = format!("{} {name}  ({tracks})", if *collapsed { "▸" } else { "▾" });
                        f.fill_text(Text { content: label, position: Point::new(8.0, y + 8.0), size: iced::Pixels(13.0), font: iced::Font { weight: iced::font::Weight::Bold, ..iced::Font::DEFAULT }, ..Text::default() });
                        continue;
                    }
                    Row::Lane(l) => &c.lanes[*l],
                };
                let indent = if l.folder.is_empty() { 0.0 } else { 10.0 };
                f.fill_rectangle(Point::new(indent, y), Size::new(5.0, self.lane_height), l.color);
                f.fill_text(Text { content: l.name.clone(), position: Point::new(12.0 + indent, y + 8.0), size: iced::Pixels(13.0), ..Text::default() });
                if l.audio {
                    f.fill_text(Text { content: "Audio".into(), position: Point::new(12.0 + indent, y + 26.0), size: iced::Pixels(11.0), color: crate::theme::palette().dim, ..Text::default() });
                }
            }
            f.fill_text(Text { content: "Tracks".into(), position: Point::new(8.0, 5.0), size: iced::Pixels(12.0), color: crate::theme::palette().dim, ..Text::default() });
        });
        let mut overlay = Frame::new(renderer, bounds.size());
        // Automation over the lanes that show it.
        for (lane, c) in self.curves.iter().enumerate().filter_map(|(i, c)| Some((i, c.as_ref()?))).filter(|(i, _)| self.content.row_of.get(*i).copied().flatten().is_some()) {
            let (top, h) = self.curve_band(lane);
            overlay.fill_rectangle(Point::new(HEADER, top), Size::new(bounds.width - HEADER, h), Color { a: 0.12, ..crate::theme::palette().accent });
            let mut pts: Vec<Point> = c.points.iter().map(|&(q, v)| Point::new(self.x(q).max(HEADER), self.curve_y(lane, c, v))).collect();
            if let (Some(first), Some(last)) = (pts.first().copied(), pts.last().copied()) {
                pts.insert(0, Point::new(HEADER, first.y));
                pts.push(Point::new(bounds.width, last.y));
                let path = Path::new(|b| {
                    b.move_to(pts[0]);
                    for p in &pts[1..] {
                        b.line_to(*p);
                    }
                });
                overlay.stroke(&path, Stroke::default().with_width(2.0).with_color(crate::theme::palette().accent));
                for p in &pts[1..pts.len() - 1] {
                    if p.x > HEADER {
                        overlay.fill(&Path::circle(*p, 4.0), crate::theme::palette().accent);
                    }
                }
            }
            overlay.fill_text(Text { content: c.label.clone(), position: Point::new(12.0, self.lane_y(lane) + 26.0), size: iced::Pixels(11.0), color: crate::theme::palette().accent, ..Text::default() });
        }
        if let Some(q) = self.playhead {
            let x = self.x(q);
            if x >= HEADER {
                overlay.stroke(&Path::line(Point::new(x, 0.0), Point::new(x, bounds.height)), Stroke::default().with_width(2.0).with_color(crate::theme::palette().success));
            }
        }
        vec![geometry, overlay.into_geometry()]
    }
}

/// A clip: a block in the lane's colour (grey when muted), its waveform,
/// its fades as lines from the top corners, its name.
fn draw_clip(f: &mut Frame, v: &ArrangeView, c: &ClipBlock, width: f32) {
    let r = v.clip_rect(c);
    if r.x + r.width < HEADER || r.x > width {
        return;
    }
    let base = if c.muted { Color::from_rgb(0.55, 0.55, 0.58) } else { v.content.lanes.get(c.lane).map(|l| l.color).unwrap_or(PALETTE[0]) };
    let selected = v.selected_clip == Some(c.clip);
    let (x0, x1) = (r.x.max(HEADER), (r.x + r.width).min(width));
    f.fill_rectangle(Point::new(x0, r.y), Size::new(x1 - x0, r.height), Color { a: 0.35, ..base });
    let wave = Color { a: 0.9, ..base };
    if let Some(d) = c.data.as_ref().filter(|d| !d.peaks.is_empty()) {
        let mid = r.y + r.height / 2.0 + 6.0;
        let half = (r.height - 16.0) / 2.0;
        let px = r.width / c.length.max(1e-6) as f32; // pixels per second, along the clip
        let mut x = x0.floor();
        while x < x1 {
            // The peaks under this pixel column.
            let (s0, s1) = (((x - r.x) / px) as f64 + c.offset, ((x + 1.0 - r.x) / px) as f64 + c.offset);
            let (k0, k1) = ((s0 * d.sample_rate) as usize / corinzia_audio::media::PEAK_FRAMES, ((s1 * d.sample_rate) as usize / corinzia_audio::media::PEAK_FRAMES + 1).min(d.peaks.len()));
            if k0 < k1 {
                let (lo, hi) = d.peaks[k0..k1].iter().fold((0.0f32, 0.0f32), |a, p| (a.0.min(p.0), a.1.max(p.1)));
                f.fill_rectangle(Point::new(x, mid - hi.min(1.0) * half), Size::new(1.0, ((hi - lo).min(2.0) * half).max(1.0)), wave);
            }
            x += 1.0;
        }
    }
    // Fades: from the bottom corner to the top where they end.
    let fade = Stroke::default().with_width(1.5).with_color(Color::WHITE);
    if c.fade_in_q > c.start_q {
        f.stroke(&Path::line(Point::new(r.x, r.y + r.height), Point::new(v.x(c.fade_in_q), r.y)), fade);
    }
    if c.fade_out_q < c.end_q {
        f.stroke(&Path::line(Point::new(v.x(c.fade_out_q), r.y), Point::new(r.x + r.width, r.y + r.height)), fade);
    }
    f.stroke(&Path::rectangle(Point::new(x0, r.y), Size::new(x1 - x0, r.height)), Stroke::default().with_width(if selected { 2.0 } else { 1.0 }).with_color(if selected { crate::theme::palette().accent } else { base }));
    if r.x >= HEADER - 1.0 {
        f.fill_text(Text { content: c.name.clone(), position: Point::new(r.x + 4.0, r.y + 2.0), size: iced::Pixels(11.0), ..Text::default() });
    }
}

/// Keeps the scroll inside the content.
pub fn clamp_scroll(scroll: f32, content: &Content, ppq: f32) -> f32 {
    scroll.clamp(0.0, (content.length_q as f32 * ppq - 200.0).max(0.0))
}
