//! update(): every message, and clicks and keys on the score.

use super::*;

impl App {
    pub(crate) fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::SetView(v) => {
                self.view = v;
                self.pan = Vector::new(24.0, 24.0);
                self.engraved = None;
            }
            Message::SetPanel(p) => self.panel = p,
            Message::Mix(mix::MixMsg::AddAudio(t)) => return self.pick_file_for(browser::Purpose::Audio(t), false),
            Message::Mix(m) => self.mix(m),
            Message::Roll(e) => self.roll_event(e),
            Message::RollClose => self.roll = None,
            Message::RollReset => {
                if let Some(r) = &self.roll {
                    let heads = r.selected.clone();
                    let all = if heads.is_empty() { r.content.notes.iter().map(|n| n.head).collect() } else { heads };
                    self.editor.tweak(&all, "Reset playing", false, |t| *t = corinzia_model::Tweak::new(t.head));
                }
            }
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
                arrange::ArrangeEvent::OpenRoll(track) => self.open_roll(track),
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

    /// A click on the score: select, or (input mode, empty paper) enter a note.
    pub(crate) fn click(&mut self, hit: Option<u64>, at: Option<StaffPoint>, shift: bool) {
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

    /// A key press from the score keyboard.
    pub(crate) fn press(&mut self, key: &iced::keyboard::Key, mods: iced::keyboard::Modifiers) -> Option<Task<Message>> {
        use iced::keyboard::{key::Named, Key};
        // The piano roll: Escape closes it, R resets the selected notes.
        if let (Tab::Mix, true) = (self.tab, self.roll.is_some()) {
            match key {
                Key::Named(Named::Escape) => return Some(Task::done(Message::RollClose)),
                Key::Character(c) if c.as_str() == "r" && !mods.control() => return Some(Task::done(Message::RollReset)),
                _ => {}
            }
        }
        // The selected clip in the arrange view: Delete removes it.
        if let (Tab::Mix, Some((track, clip)), Key::Named(Named::Delete | Named::Backspace)) = (self.tab, self.selected_clip, key) {
            self.editor.delete_clip(track, clip);
            self.selected_clip = None;
            return Some(Task::none());
        }
        let action = keys::map(key, mods, self.editor.input_mode)?;
        Some(self.act(action))
    }
}
