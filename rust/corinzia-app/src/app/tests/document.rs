//! Files: the browser, saving, autosave and audio export.

use super::*;

#[test]
fn without_a_system_file_dialog_corinzias_browser_opens() {
    let mut a = app(View::Page);
    a.editor.modified = true;
    // Open asks first; Discard goes on to the file dialog.
    let _ = a.update(Message::Guarded(Box::new(Message::Open)));
    assert!(a.dialogs.confirm.is_some());
    let _ = a.update(Message::ConfirmDiscard(true));
    assert!(a.dialogs.confirm.is_none());
    // The system dialog could not open: the browser takes its place.
    let _ = a.update(Message::Picked { purpose: browser::Purpose::Open, path: None, failed: true });
    assert!(a.dialogs.browser.is_some());
    let _ = a.update(Message::Browser(browser::BrowserMsg::Cancel));
    assert!(a.dialogs.browser.is_none());
    // A dialog the user cancelled stays cancelled.
    let _ = a.update(Message::Picked { purpose: browser::Purpose::Open, path: None, failed: false });
    assert!(a.dialogs.browser.is_none());
    // Exports too, with the file name filled in; Escape closes it.
    let _ = a.update(Message::Picked { purpose: browser::Purpose::Export(files::Format::Pdf), path: None, failed: true });
    assert!(a.dialogs.browser.as_ref().unwrap().name.ends_with(".pdf"));
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
    assert!(a.dialogs.browser.is_none());
}

#[test]
fn saves_a_project_both_apps_can_open() {
    let dir = std::env::temp_dir().join(format!("corinzia-app-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("demo.project");
    let mut a = app(View::Page);
    a.doc.path = Some(path.clone());
    let _ = a.update(Message::Act(Action::Save));
    assert!(a.editor.status.starts_with("Saved"), "{}", a.editor.status);
    assert!(!a.editor.modified);
    let p = corinzia_model::store::load(&path).unwrap();
    assert_eq!(p.movements[0].lanes.len(), 2);
    assert_eq!(*p.movements[0].lanes[0].voices[0].events, *a.editor.project.movements[0].lanes[0].voices[0].events);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn audio_is_exported_over_several_frames_and_can_be_stopped() {
    let dir = std::env::temp_dir().join(format!("corinzia-app-bounce-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut a = app(View::Page);
    let path = dir.join("mix");
    let _ = a.update(Message::ExportTo(files::Format::Audio, Some(path.clone())));
    assert!(a.exports.bounce.is_some() && a.editor.status.starts_with("Exporting audio"));
    let mut frames = 0;
    while a.exports.bounce.is_some() {
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
    assert!(a.exports.bounce.is_none() && !other.exists());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn each_track_exports_as_a_stem_of_its_own() {
    let dir = std::env::temp_dir().join(format!("corinzia-app-stems-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut a = app(View::Page);
    // Two tracks heard (the muted ones are left out).
    for t in a.editor.project.tracks.iter_mut().skip(2) {
        t.mute = true;
    }
    let names: Vec<String> = a.editor.project.tracks.iter().take(2).map(|t| t.name.clone()).collect();
    let _ = a.update(Message::ExportTo(files::Format::Stems, Some(dir.join("take"))));
    assert!(a.editor.status.starts_with("Exporting"), "{}", a.editor.status);
    let mut frames = 0;
    while a.exports.bounce.is_some() {
        let _ = a.update(Message::Frame);
        frames += 1;
        assert!(frames < 20_000);
    }
    assert!(a.editor.status.starts_with("Exported 2 stems"), "{}", a.editor.status);
    let files: Vec<Vec<u8>> = names.iter().map(|n| std::fs::read(dir.join(format!("take - {n}.wav"))).unwrap()).collect();
    assert!(files.iter().all(|f| f.len() > 44));
    assert_ne!(files[0], files[1], "each its own track");
    // The project itself is as it was: the stems' mutes were on copies.
    assert!(!a.editor.project.tracks[0].mute && !a.editor.project.tracks[1].mute);
    assert_eq!(files::stem_path(std::path::Path::new("/x/mix.wav"), "Violin I/II"), std::path::PathBuf::from("/x/mix - Violin III.wav"));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn autosaves_projects_with_a_file() {
    let dir = std::env::temp_dir().join(format!("corinzia-autosave-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("a.project");
    corinzia_model::store::save(&demo::project(), &path).unwrap();
    let mut a = App::new(corinzia_model::store::load(&path).unwrap(), Some(path.clone()), View::Page, String::new());
    assert!(a.doc.autosave_at.is_none(), "nothing to save after opening");
    key(&mut a, "ctrl+a");
    key(&mut a, "[");
    assert!(a.editor.modified && a.doc.autosave_at.is_some());
    a.doc.autosave_at = Some(std::time::Instant::now()); // due now
    assert!(a.autosave().is_some() && a.doc.saving);
    // The task's work, as the executor would run it:
    corinzia_model::store::save(&a.editor.project, &path).unwrap();
    let rev = a.editor.revision;
    let _ = a.update(Message::Autosaved(rev, Ok(())));
    assert!(!a.editor.modified && !a.doc.saving);
    let p = corinzia_model::store::load(&path).unwrap();
    assert!(p.movements[0].lanes[0].voices[0].events[0].articulations != 0, "the staccato reached the file");
    std::fs::remove_dir_all(&dir).ok();
}
