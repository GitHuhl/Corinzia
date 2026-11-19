//! Opening, saving, autosaving and exporting files, and the file dialogs.

use super::*;

/// What to do with a file chosen for a purpose (in the system dialog or
/// Corinzia's browser).
pub(crate) fn chosen(purpose: browser::Purpose, path: Option<PathBuf>) -> Message {
    match purpose {
        browser::Purpose::Open => Message::Opened(path),
        browser::Purpose::Export(f) => Message::ExportTo(f, path),
        browser::Purpose::Audio(t) => Message::AudioChosen(t, path),
    }
}

impl App {
    /// Asks for a file to open (`musicxml`: MusicXML only) with the
    /// system's dialog, else Corinzia's browser.
    pub(crate) fn pick_file(&mut self, musicxml: bool) -> Task<Message> {
        self.pick_file_for(browser::Purpose::Open, musicxml)
    }

    pub(crate) fn pick_file_for(&mut self, purpose: browser::Purpose, musicxml: bool) -> Task<Message> {
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

    pub(crate) fn open_browser(&mut self, purpose: browser::Purpose) -> Task<Message> {
        let dir = self.path.as_ref().and_then(|p| p.parent().map(PathBuf::from));
        let name = match purpose {
            browser::Purpose::Open | browser::Purpose::Audio(_) => String::new(),
            browser::Purpose::Export(f) => files::sibling(self.path.as_deref(), &self.editor.project.title, f.extension()).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        };
        self.browser = Some(browser::Browser::new(purpose, dir, name));
        iced::widget::operation::focus(browser::NAME_INPUT)
    }

    pub(crate) fn ask_save_path(&self) -> Task<Message> {
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

    pub(crate) fn open(&mut self, path: PathBuf) {
        match files::load(&path) {
            Ok((p, kept, status)) => {
                let (view, pb) = (self.view, std::mem::replace(&mut self.playback, playback::Playback::new(false, &prefs::Prefs::default())));
                let prefs = self.prefs.clone();
                *self = App::with_playback(p, kept, view, status, pb);
                self.prefs = prefs;
                self.remember(&path);
            }
            Err(e) => self.editor.status = format!("Could not open {}: {e}", path.display()),
        }
    }

    /// A score into the recent list (kept with the preferences).
    pub(crate) fn remember(&mut self, path: &std::path::Path) {
        self.prefs.remember(path);
        self.save_prefs();
    }

    /// Starts a background save when one is due.
    pub(crate) fn autosave(&mut self) -> Option<Task<Message>> {
        let due = self.autosave_at.is_some_and(|t| std::time::Instant::now() >= t);
        let path = self.path.clone().filter(|_| due && !self.saving)?;
        self.autosave_at = None;
        self.saving = true;
        self.playback.pool.store_states(&mut self.editor.project);
        let (project, rev) = (self.editor.project.clone(), self.editor.revision);
        Some(Task::perform(async move { corinzia_model::store::save(&project, &path).map_err(|e| e.to_string()) }, move |r| Message::Autosaved(rev, r)))
    }

    pub(crate) fn save(&mut self, path: PathBuf) {
        // Plugin state chunks go into the file with the music.
        self.playback.pool.store_states(&mut self.editor.project);
        self.editor.status = match corinzia_model::store::save(&self.editor.project, &path) {
            Ok(()) => {
                self.path = Some(path.clone());
                self.remember(&path);
                self.saved_revision = self.editor.revision;
                self.autosave_at = None;
                self.editor.saved();
                format!("Saved {}", path.display())
            }
            Err(e) => format!("Save failed: {e}"),
        };
    }

    /// Starts an audio export of the movement edited: its own plugins,
    /// restored from the live ones' current settings.
    pub(crate) fn start_bounce(&mut self, path: PathBuf) {
        self.stems.clear();
        self.stems_done = None;
        self.bounce_project(self.editor.project.clone(), path);
    }

    /// Starts exporting each track on its own: "name - Track.wav" beside
    /// `path`, one after another (tracks muted in the mix left out).
    pub(crate) fn start_stems(&mut self, path: PathBuf) {
        let tracks: Vec<(corinzia_model::Id, PathBuf)> = self.editor.project.tracks.iter().filter(|t| !t.mute).map(|t| (t.id, files::stem_path(&path, &t.name))).collect();
        if tracks.is_empty() {
            self.editor.status = "No tracks to export".into();
            return;
        }
        self.stems_done = Some((tracks.len(), path.parent().map(Into::into).unwrap_or_default()));
        self.stems = tracks.into();
        self.next_stem();
    }

    /// The next stem: the project with every other track muted.
    fn next_stem(&mut self) {
        let Some((track, path)) = self.stems.pop_front() else { return };
        let mut project = self.editor.project.clone();
        for t in project.tracks.iter_mut() {
            t.mute = t.id != track;
            t.solo = false;
        }
        self.bounce_project(project, path);
    }

    fn bounce_project(&mut self, mut project: corinzia_model::Project, path: PathBuf) {
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
    pub(crate) fn bounce_step(&mut self) {
        let Some(b) = self.bounce.as_mut() else { return };
        let started = Instant::now();
        let mut done = false;
        while !done && started.elapsed() < std::time::Duration::from_millis(30) {
            done = b.step(0.1);
        }
        if !done {
            self.editor.status = match self.stems_done {
                Some((n, _)) => format!("Exporting stem {} of {n}… {:.0}% (Escape stops)", n - self.stems.len(), b.progress() * 100.0),
                None => format!("Exporting audio… {:.0}% (Escape stops)", b.progress() * 100.0),
            };
            return;
        }
        let b = self.bounce.take().unwrap();
        let (path, warnings) = (b.path.clone(), b.warnings.clone());
        self.editor.status = match b.finish() {
            Ok(s) => {
                let mut msg = format!("Exported {} ({:.1} s, {:.1} LUFS, true peak {:.1} dBTP)", path.display(), s.seconds, s.lufs, s.true_peak_db);
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
        // Stems: the next, or all done.
        if self.editor.status.starts_with("Exported") {
            if !self.stems.is_empty() {
                self.next_stem();
            } else if let Some((n, folder)) = self.stems_done.take() {
                self.editor.status = format!("Exported {n} stems to {}", if folder.as_os_str().is_empty() { ".".into() } else { folder.display().to_string() });
            }
        } else {
            self.stems.clear();
            self.stems_done = None;
        }
    }

    /// Exports the movement or the work to a file (audio goes through
    /// `start_bounce`).
    pub(crate) fn export_to(&mut self, format: files::Format, path: PathBuf) {
        let path = if path.extension().is_none() { path.with_extension(format.extension()) } else { path };
        // A chant shown: its pages.
        if let (Some(c), files::Format::Pdf) = (self.editor.shown_chant(), format) {
            self.editor.status = match std::fs::write(&path, files::chant_pdf(c)) {
                Ok(()) => format!("Exported the chant to {}", path.display()),
                Err(e) => format!("Export failed: {e}"),
            };
            return;
        }
        self.editor.status = match files::export_to(&self.editor.project, self.editor.movement, format, self.editor.transposed, &path) {
            Ok(()) => format!("Exported {} to {}", format.label(), path.display()),
            Err(e) => format!("Export failed: {e}"),
        };
    }
}
