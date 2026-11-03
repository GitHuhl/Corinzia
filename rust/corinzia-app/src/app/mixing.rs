//! The Mix tab: mixer messages, presets, plugin editors, audio clips and automation curves.

use super::*;

impl App {
    /// The automation curve shown on a track's arrange lane.
    pub(crate) fn curve(&self, track: corinzia_model::Id) -> Option<arrange::Curve> {
        let target = *self.auto_shown.get(&track)?;
        let (label, min, max) = mix::auto_options(&self.editor.project, &self.playback, track).into_iter().find(|o| o.1 == Some(target)).map(|o| (o.0, o.2, o.3))?;
        let q = |f: Frac| f.num as f64 * 4.0 / f.den as f64;
        let lane = self.editor.auto_lane(target);
        let points = lane.map(|l| l.points.iter().map(|p| (q(p.at), p.value)).collect()).unwrap_or_default();
        let holds = lane.map(|l| l.points.iter().map(|p| p.hold).collect()).unwrap_or_default();
        Some(arrange::Curve { label, min, max, points, holds })
    }

    pub(crate) fn mix(&mut self, m: mix::MixMsg) {
        self.mix_msg(m);
        self.refresh_presets();
    }

    pub(crate) fn mix_msg(&mut self, m: mix::MixMsg) {
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

    /// The plugin a preset target names, and its project node (where its state is kept).
    pub(crate) fn preset_plugin(&self, t: mix::Target) -> Option<(std::sync::Arc<corinzia_audio::host::Plugin>, Option<corinzia_model::Id>)> {
        match t {
            mix::Target::Effect(n) => self.playback.pool.effect(n).cloned().map(|p| (p, Some(n))),
            mix::Target::Track(id) => self.playback.pool.plugin_of_track(id).cloned().map(|p| (p, self.editor.project.track(id).and_then(|x| x.instrument_node))),
        }
    }

    /// The presets of the plugin the Mix tab shows.
    pub(crate) fn refresh_presets(&mut self) {
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

    pub(crate) fn preset_base() -> Option<PathBuf> {
        if cfg!(test) {
            Some(std::env::temp_dir().join(format!("corinzia-test-presets-{}", std::process::id())))
        } else {
            presets::base()
        }
    }

    /// Shows a track's plugin editor, or closes it when open.
    pub(crate) fn toggle_editor(&mut self, track: corinzia_model::Id) {
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

    /// Places a sound file on an audio track (`None`, or not an audio
    /// track: a new one named after the file) at the caret of the movement edited. A file in the
    /// project's folder is kept by its relative path.
    pub(crate) fn add_audio_file(&mut self, track: Option<corinzia_model::Id>, path: &std::path::Path) {
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
    pub(crate) fn edit_selected_clip(&mut self, label: &str, live: bool, f: impl FnOnce(&mut corinzia_model::AudioClip)) {
        if let Some((t, c)) = self.selected_clip {
            self.editor.edit_clip(t, c, label, live, f);
            self.clip_dragged |= live;
        }
    }

    /// The playback timeline's section of the movement edited (its tempo:
    /// seconds to quarters).
    pub(crate) fn movement_section(&self) -> Option<&corinzia_audio::snapshot::Section> {
        let id = self.editor.movement()?.id;
        self.playback.timeline.sections.iter().find(|s| s.movement == id)
    }

    /// A drag step on the selected clip: `q` (quarters) is its new start,
    /// edge, or the end of a fade, applied to the clip as it was pressed.
    pub(crate) fn drag_clip(&mut self, edit: arrange::ClipEdit, q: f64) {
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
}
