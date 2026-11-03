//! Files: the browser, saving, autosave and audio export.

use super::*;

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
