//! view(): the toolbar, tabs, overlays and status bar.

use super::*;
use iced::widget::column;


/// The status bar's movement: its title between previous and next.
pub(crate) fn movement_picker(ed: &editor::Editor, title: String) -> Element<'static, Message> {
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
pub(crate) fn grid_picker(ed: &editor::Editor) -> Element<'static, Message> {
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

impl App {
    pub(crate) fn view(&self) -> Element<'_, Message> {
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
            desk = desk.push(container(iced::widget::opaque(container(new_score_card(n)).padding(20).style(theme::card))).center(Length::Fill));
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
                let top: Element<'_, Message> = match &self.roll {
                    Some(r) => {
                        let tweaks = ed.movement().map(|m| m.tweaks.as_slice()).unwrap_or(&[]);
                        let view = canvas(roll::RollView { roll: &r.content, tweaks, selected: &r.selected, ppq: r.ppq, scroll_x: r.scroll_x, top_key: r.top_key, playhead }).width(Length::Fill).height(Length::Fill);
                        let what = if r.selected.is_empty() { "all notes".to_string() } else { format!("{} selected", r.selected.len()) };
                        column![
                            row![
                                text(format!("Piano roll · {}", r.content.name)).size(13).font(bold),
                                text("drag a note: earlier or later (Shift: by sixteenths) · its right edge: longer or shorter · the lane below: velocity · pitch and rhythm stay the score's").size(11).style(theme::dim),
                                panels::spacer(),
                                panels::tip(button(text(format!("Reset {what}")).size(12)).padding([3, 8]).style(theme::tool(false)).on_press(Message::RollReset), "Play as written again (R)"),
                                panels::tip(button(text("✕").size(12)).padding([3, 8]).style(theme::flat(false)).on_press(Message::RollClose), "Back to the arrange view (Escape)"),
                            ]
                            .spacing(10)
                            .padding([4, 10])
                            .align_y(iced::Alignment::Center),
                            Element::from(view).map(Message::Roll),
                        ]
                        .height(Length::Fill)
                        .into()
                    }
                    None => Element::from(canvas(arrange::ArrangeView {
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
                .height(Length::Fill))
                    .map(Message::Arrange),
                };
                column![
                    top,
                    hline(),
                    container(mix::view(&ed.project, &self.playback, self.strip, self.fx, mix::Presets { names: &self.preset_names, name: &self.preset_name }, &self.auto_shown, self.selected_clip).map(Message::Mix)).height(420).width(Length::Fill).style(theme::panel),
                ]
                .height(Length::Fill)
                .into()
            }
        };
        column![toolbar, hline(), body, hline(), status].into()
    }

    /// Full score, or a player's part.
    pub(crate) fn part_picker(&self) -> Element<'_, Message> {
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
}
