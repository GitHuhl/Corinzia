//! Keyboard and menu actions, playback start/stop and popovers.

use super::*;

impl App {
    pub(crate) fn act(&mut self, a: Action) -> Task<Message> {
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
    pub(crate) fn slur_action(&mut self, a: &Action) -> bool {
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

    pub(crate) fn act_inner(&mut self, a: Action) -> Task<Message> {
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

    /// Plays the selected note (just entered or clicked) on its instrument.
    pub(crate) fn audition(&mut self) {
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

    pub(crate) fn play_stop(&mut self) {
        if self.playback.is_playing() {
            self.playback.stop();
            return;
        }
        // From the selection, else the caret.
        let ed = &self.editor;
        let (Some(m), Some((_, at))) = (ed.movement(), ed.structure_target()) else { return };
        self.playback.play_from(m.id, quarters(at));
    }

    pub(crate) fn open_popover(&mut self, kind: PopoverKind) -> Task<Message> {
        let anchor = self.popover_anchor();
        self.popover = Some(Popover { kind, input: String::new(), error: None, target: None, anchor });
        iced::widget::operation::focus(POPOVER_INPUT)
    }

    /// Where on the canvas the popover target is: the selected element,
    /// else the passage start or the caret on its staff.
    pub(crate) fn popover_anchor(&self) -> Option<iced::Point> {
        let ed = &self.editor;
        let origins = score_view::page_origins(&self.pages, self.view == View::Page);
        let at = ed.structure_target().and_then(|(lane, pos)| Some(Caret { part: self.staff_lanes.iter().position(|&l| l == lane)?, quarter: quarters(pos) }));
        let desk = score_view::anchor(&self.pages, &origins, &self.pages_of(ed.movement), ed.selection, at)?;
        let px = self.zoom * score_view::BASE_PX;
        Some(iced::Point::new(desk.x * px + self.pan.x, desk.y * px + self.pan.y))
    }

    /// A double-click on the page: a title, tempo mark, text, chord symbol,
    /// dynamic or lyric opens its popover with its text to retype.
    pub(crate) fn edit_on_page(&mut self, id: u64) -> Task<Message> {
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
}
