use super::*;
use crate::fader;
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
fn the_piano_roll_tweaks_how_notes_play_and_leaves_the_score_alone() {
    use roll::{Drag, RollEvent as R};
    let mut a = app(View::Page);
    let _ = a.act(Action::ShowMix);
    let track = a.editor.project.tracks[0].id;
    let _ = a.update(Message::Arrange(arrange::ArrangeEvent::OpenRoll(track)));
    let notes = a.roll.as_ref().unwrap().content.notes.clone();
    assert!(notes.len() > 4, "the track's notes");
    let score_before = format!("{:?}", a.editor.project.movements[0].lanes);
    let (h0, h1) = (notes[0].head, notes[1].head);
    let ev = |a: &mut App, e: R| {
        let _ = a.update(Message::Roll(e));
    };
    // Two notes selected, dragged a sixteenth later: one undo step.
    ev(&mut a, R::Select { head: Some(h0), add: false });
    ev(&mut a, R::Select { head: Some(h1), add: true });
    let music = a.editor.music_revision;
    ev(&mut a, R::Drag { drag: Drag::Shift, dq: 0.125 });
    ev(&mut a, R::Drag { drag: Drag::Shift, dq: 0.25 });
    assert_eq!(a.editor.music_revision, music, "playback catches up on release");
    ev(&mut a, R::Release);
    assert!(a.editor.music_revision > music);
    assert_eq!(a.editor.tweak_of(h0).unwrap().shift, 0.25);
    assert_eq!(a.editor.tweak_of(h1).unwrap().shift, 0.25);
    a.editor.undo();
    assert!(a.editor.tweak_of(h0).is_none(), "one undo puts both back");
    a.editor.redo();
    // Longer, from the right edge; softer, from the velocity lane.
    ev(&mut a, R::Select { head: Some(h0), add: false });
    ev(&mut a, R::Drag { drag: Drag::Length, dq: 1.0 });
    ev(&mut a, R::Release);
    assert!((a.editor.tweak_of(h0).unwrap().length.unwrap() - (notes[0].length + 1.0)).abs() < 1e-9);
    ev(&mut a, R::Velocity { head: h0, velocity: 0.2 });
    ev(&mut a, R::Release);
    assert!((a.editor.tweak_of(h0).unwrap().velocity.unwrap() - 0.2).abs() < 0.01);
    // The score is as it was; the performance is not.
    assert_eq!(format!("{:?}", a.editor.project.movements[0].lanes), score_before);
    let perf = corinzia_perform::perform(&a.editor.project.movements[0]);
    let n0 = perf.notes.iter().find(|n| n.element == h0).unwrap();
    assert!((n0.velocity - 0.2).abs() < 0.01);
    a.sync();
    let r = a.roll.as_ref().unwrap();
    assert_eq!(r.content.notes[0].start, notes[0].start, "the roll keeps the written place; the tweak moves it");
    // R resets the selection: played as written again.
    let _ = a.update(Message::RollReset);
    assert!(a.editor.tweak_of(h0).is_none() && a.editor.tweak_of(h1).is_some());
    let _ = a.update(Message::RollClose);
    assert!(a.roll.is_none());
}

#[test]
fn a_roll_note_is_grabbed_by_its_body_or_its_right_edge() {
    let r = roll::Roll { track: 1, name: "F".into(), notes: vec![roll::RollNote { head: 9, key: 60, start: 2.0, length: 1.0, velocity: 0.5 }], bars: vec![0.0, 4.0], length_q: 4.0, high: 72 };
    let tweaks = [corinzia_model::Tweak { head: 9, velocity: None, shift: 0.5, length: Some(2.0) }];
    let v = roll::RollView { roll: &r, tweaks: &tweaks, selected: &[], ppq: 40.0, scroll_x: 0.0, top_key: 72, playhead: None };
    let rect = v.note_rect(&r.notes[0]);
    assert_eq!((rect.x, rect.width), (v.x(2.5), 80.0), "where and how long it plays");
    let mid = rect.y + rect.height / 2.0;
    assert_eq!(v.note_at(iced::Point::new(rect.x + 20.0, mid)).map(|x| x.1), Some(false));
    assert_eq!(v.note_at(iced::Point::new(rect.x + rect.width - 2.0, mid)).map(|x| x.1), Some(true));
    assert!(v.note_at(iced::Point::new(rect.x - 10.0, mid)).is_none());
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
