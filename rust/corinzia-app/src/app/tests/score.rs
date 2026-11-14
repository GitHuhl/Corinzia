//! Editing on the page: clicks, popovers, slurs, selection, parts, condensing, transposing, filters, lyrics.

use super::*;

#[test]
fn engraves_pages_and_panorama() {
    let a = app(View::Page);
    assert_eq!(a.pages.len(), 1);
    assert!(a.pages[0].height > a.pages[0].width, "A4 portrait");
    let b = app(View::Panorama);
    assert_eq!(b.pages.len(), 1);
    assert!(b.pages[0].width > b.pages[0].height * 3.0, "one long system");
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
    let _ = a.update(Message::Popover(PopoverMsg::Input("x".into())));
    let _ = a.update(Message::Popover(PopoverMsg::Submit));
    assert!(a.popover.as_ref().unwrap().error.is_some(), "bad input keeps the popover open with a hint");
    let _ = a.update(Message::Popover(PopoverMsg::Input("D".into())));
    let _ = a.update(Message::Popover(PopoverMsg::Submit));
    assert!(a.popover.is_none());
    assert_eq!(a.editor.project.movements[0].keys[0].fifths, 2);
    key(&mut a, "esc");
    assert!(a.editor.range.is_none());
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
}

#[test]
fn filters_and_select_similar_pick_elements_to_edit() {
    let mut a = app(View::Page);
    let notes = |a: &App, lane: usize| a.editor.project.movements[0].lanes[lane].voices[0].events.iter().filter(|e| !e.is_rest()).cloned().collect::<Vec<_>>();
    let high: Vec<u64> = notes(&a, 0).iter().filter(|e| e.heads[0].pitch.midi_key() > 72).map(|e| e.heads[0].id).collect();
    assert!(high.len() > 3);
    // Ctrl+F "above C5": the flute's high notes, highlighted.
    let _ = a.update(Message::Act(Action::Popover(PopoverKind::Filter)));
    let _ = a.update(Message::Popover(PopoverMsg::Input("above C5".into())));
    let _ = a.update(Message::Popover(PopoverMsg::Submit));
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
    let _ = a.update(Message::Popover(PopoverMsg::Close));
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
    let _ = a.update(Message::Popover(PopoverMsg::Close));
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
