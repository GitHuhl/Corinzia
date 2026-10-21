//! Ported from tests/test_edit.cpp and tests/test_structure.cpp.

use corinzia_edit::*;
use corinzia_model::*;
use corinzia_score::score::{art, span};
use corinzia_score::Frac;
use std::sync::Arc;

fn f(n: i64, d: i64) -> Frac {
    Frac::new(n, d)
}
fn p(step: i8, alter: i8, octave: i8) -> Pitch {
    Pitch { step, alter, octave }
}
fn bar(ids: &mut Ids) -> Vec<Event> {
    // c'4 d'4 e'2
    vec![
        make_event(ids, f(0, 1), f(1, 4), &[p(0, 0, 4)], 0),
        make_event(ids, f(1, 4), f(1, 4), &[p(1, 0, 4)], 0),
        make_event(ids, f(1, 2), f(1, 2), &[p(2, 0, 4)], 0),
    ]
}
fn contiguous(v: &[Event]) -> bool {
    let mut at = Frac::ZERO;
    for e in v {
        if e.start != at {
            return false;
        }
        at = e.end();
    }
    true
}
fn two_bars(ids: &mut Ids) -> Vec<Event> {
    // c4 d4 e4 f4 | g2 a2
    let mut v: Vec<Event> = (0..4).map(|i| make_event(ids, f(i, 4), f(1, 4), &[p(i as i8, 0, 4)], 0)).collect();
    v.push(make_event(ids, f(1, 1), f(1, 2), &[p(4, 0, 4)], 0));
    v.push(make_event(ids, f(3, 2), f(1, 2), &[p(5, 0, 4)], 0));
    v
}

#[test]
fn durations() {
    let d = |base, dots| DurationSpec { base, dots }.value();
    assert_eq!(d(BaseDuration::Quarter, 0), f(1, 4));
    assert_eq!(d(BaseDuration::Quarter, 1), f(3, 8));
    assert_eq!(d(BaseDuration::Half, 2), f(7, 8));
    assert_eq!(d(BaseDuration::SixtyFourth, 0), f(1, 64));
}

#[test]
fn pitch_spelling_and_octave() {
    assert_eq!(key_alter(1, 3), 1);
    assert_eq!(key_alter(1, 0), 0);
    assert!(key_alter(-2, 6) == -1 && key_alter(-2, 2) == -1 && key_alter(-2, 5) == 0);
    let b4 = p(6, 0, 4);
    assert_eq!(nearest_pitch(0, None, 0, b4).octave, 5);
    assert_eq!(nearest_pitch(4, None, 0, b4).octave, 4);
    let fs = nearest_pitch(3, None, 1, b4);
    assert!(fs.alter == 1 && fs.octave == 4);
    assert_eq!(nearest_pitch(3, Some(0), 1, b4).alter, 0);
}

#[test]
fn overwrite_insert_truncates_and_fills() {
    let mut ids = Ids(1000);
    let v = bar(&mut ids);
    let ev = make_event(&mut ids, f(1, 4), f(3, 8), &[p(4, 0, 4)], 0);
    let head = ev.heads[0].id;
    let r = insert_event(&v, ev, true, &mut ids);
    let out = &r.events;
    assert!(contiguous(out));
    assert_eq!(out.len(), 3);
    assert!(out[1].heads[0].pitch.step == 4 && out[1].duration == f(3, 8));
    assert!(out[2].kind == EventKind::Rest && out[2].start == f(5, 8) && out[2].duration == f(3, 8));
    assert!(r.caret == f(5, 8) && r.focus == Some(head));
}

#[test]
fn insert_after_end_fills_gap_in_voice1_only() {
    let mut ids = Ids(1000);
    let v = bar(&mut ids);
    let e = make_event(&mut ids, f(3, 2), f(1, 4), &[p(0, 0, 5)], 0);
    let r1 = insert_event(&v, e.clone(), true, &mut ids);
    assert!(contiguous(&r1.events) && r1.events.len() == 5 && r1.from == f(1, 1));
    let r2 = insert_event(&v, e, false, &mut ids);
    assert!(r2.events.len() == 4 && !contiguous(&r2.events));
}

#[test]
fn chords_and_delete() {
    let mut ids = Ids(1000);
    let v = bar(&mut ids);
    let c = add_to_chord(&v, Frac::ZERO, p(2, 0, 4), &mut ids).unwrap();
    assert!(c.events[0].heads.len() == 2 && c.events[0].kind == EventKind::Chord);
    assert!(add_to_chord(&c.events, Frac::ZERO, p(2, 0, 4), &mut ids).is_none());
    assert!(add_to_chord(&v, f(1, 8), p(2, 0, 4), &mut ids).is_none());
    let upper = c.events[0].heads[1].id;
    let d1 = delete_element(&c.events, upper, true, &mut ids).unwrap();
    assert!(d1.events[0].heads.len() == 1 && d1.events[0].kind == EventKind::Note);
    let d2 = delete_element(&d1.events, d1.events[0].heads[0].id, true, &mut ids).unwrap();
    assert!(d2.events[0].kind == EventKind::Rest && contiguous(&d2.events));
    let d3 = delete_element(&v, v[1].id, false, &mut ids).unwrap();
    assert_eq!(d3.events.len(), 2);
}

#[test]
fn modify_and_lookup() {
    let mut ids = Ids(1000);
    let v = bar(&mut ids);
    let m = modify_event(&v, v[2].heads[0].id, |e, h| {
        e.articulations |= art::ACCENT;
        e.heads[h.unwrap()].tie = true;
    })
    .unwrap();
    assert!(m.events[2].articulations & art::ACCENT != 0 && m.events[2].heads[0].tie);
    assert_eq!(event_at(&v, f(5, 8)).unwrap().id, v[2].id);
    assert_eq!(previous_pitch(&v, f(1, 2)).unwrap().step, 1);
    assert_eq!(voice_end(&v), f(1, 1));
    let (e, h) = find_event(&v, v[1].heads[0].id).unwrap();
    assert!(e.id == v[1].id && h == Some(0));
}

#[test]
fn range_erase_slice_paste() {
    let mut ids = Ids(1000);
    let v = two_bars(&mut ids);
    let e = erase_range(&v, f(1, 4), f(3, 4), true, &mut ids);
    assert!(contiguous(&e.events) && e.events.len() == 5);
    assert!(e.events[1].kind == EventKind::Rest && e.events[1].duration == f(1, 2));
    let e2 = erase_range(&v, f(3, 4), f(5, 4), true, &mut ids);
    assert!(contiguous(&e2.events));
    assert_eq!(voice_end(&e2.events), f(2, 1));
    let clip = slice_range(&v, f(1, 2), f(1, 1));
    assert!(clip.len() == 2 && clip[0].start == Frac::ZERO && clip[1].start == f(1, 4));
    let pr = paste_events(&v, &clip, f(1, 1), f(1, 2), true, &mut ids);
    assert!(contiguous(&pr.events));
    let pasted = event_at(&pr.events, f(1, 1)).unwrap();
    assert!(pasted.heads[0].pitch.step == 2 && pasted.id != clip[0].id && pasted.heads[0].id != clip[0].heads[0].id);
    assert_eq!(event_at(&pr.events, f(3, 2)).unwrap().heads[0].pitch.step, 5);
    let holey = vec![make_event(&mut ids, f(1, 4), f(1, 4), &[p(6, 0, 4)], 0)];
    let p2 = paste_events(&v, &holey, Frac::ZERO, f(1, 2), true, &mut ids);
    assert!(contiguous(&p2.events) && p2.events[0].kind == EventKind::Rest);
}

#[test]
fn range_transpose_apply_and_spans() {
    let mut ids = Ids(1000);
    let v = two_bars(&mut ids);
    let t = transpose_range(&v, Frac::ZERO, f(1, 1), 2).unwrap();
    assert!(t.events[0].heads[0].pitch.step == 2 && t.events[4].heads[0].pitch.step == 4);
    let a = apply_to_range(&v, Frac::ZERO, f(1, 2), |e| e.articulations |= art::STACCATO).unwrap();
    assert!(a.events[1].articulations & art::STACCATO != 0 && a.events[2].articulations & art::STACCATO == 0);
    let h = mark_span(&v, Frac::ZERO, f(2, 1), span::CRESC_START, span::HAIRPIN_END).unwrap();
    assert!(h.events[0].spans & span::CRESC_START != 0);
    assert!(mark_span(&v, Frac::ZERO, f(1, 4), span::CRESC_START, span::HAIRPIN_END).is_none(), "one note");
    let d = mark_span(&h.events, Frac::ZERO, f(2, 1), span::DIM_START, span::HAIRPIN_END).unwrap();
    assert!(d.events[0].spans & span::DIM_START != 0 && d.events[0].spans & span::CRESC_START == 0);
}

// ------------------------------------------------------------ structure --
fn header() -> Movement {
    Movement {
        id: 1,
        ordinal: 0,
        title: String::new(),
        length: f(8, 1),
        revision: 0,
        tempo: vec![Tempo { at: Frac::ZERO, bpm: 120.0, ramp: false, text: String::new() }],
        meter: vec![Meter { at: Frac::ZERO, num: 4, den: 4, pickup: false, beams: [0; 8] }],
        keys: vec![Key { at: Frac::ZERO, fifths: 0, minor: false }],
        barlines: vec![],
        rehearsal: vec![],
        navigation: vec![],
        slurs: vec![],
        endings: vec![],
        lanes: vec![Lane { player: 1, changes: vec![], clefs: vec![], texts: vec![], chords: vec![], lines: vec![], voices: vec![Voice { id: 2, index: 1, events: Arc::new(vec![]), tuplets: vec![] }] }],
    }
}

#[test]
fn parse_key_cases() {
    assert_eq!(parse_key("Eb"), Some(KeySig { fifths: -3, minor: false }));
    assert_eq!(parse_key("f#"), Some(KeySig { fifths: 3, minor: true }));
    assert_eq!(parse_key("Bbm"), Some(KeySig { fifths: -5, minor: true }));
    assert_eq!(parse_key("A minor"), Some(KeySig { fifths: 0, minor: true }));
    assert_eq!(parse_key("D major"), Some(KeySig { fifths: 2, minor: false }));
    assert_eq!(parse_key("E♭").unwrap().fifths, -3);
    assert!(parse_key("H").is_none() && parse_key("").is_none() && parse_key("cbm").is_none());
}

#[test]
fn parse_time_clef_barline() {
    assert_eq!(parse_time("6/8"), Some((6, 8)));
    assert_eq!(parse_time(" 3/4 "), Some((3, 4)));
    assert_eq!(parse_time("C"), Some((4, 4)));
    assert_eq!(parse_time("C|"), Some((2, 2)));
    assert!(parse_time("3/5").is_none() && parse_time("x/4").is_none() && parse_time("0/4").is_none());
    assert_eq!(parse_clef("Bass"), Some("bass"));
    assert_eq!(parse_clef("c"), Some("alto"));
    assert_eq!(parse_clef("g8"), Some("treble_8"));
    assert!(parse_clef("soprano-ish").is_none());
    assert_eq!(parse_barline("||"), Some(BarKind::Double));
    assert_eq!(parse_barline("final"), Some(BarKind::Final));
    assert_eq!(parse_barline("|:"), Some(BarKind::RepeatStart));
    assert_eq!(parse_barline(":|"), Some(BarKind::RepeatEnd));
    assert_eq!(parse_barline(":|:"), Some(BarKind::RepeatBoth));
    assert!(parse_barline("|||").is_none());
}

#[test]
fn parse_tempo_cases() {
    let t = |s: &str| parse_tempo(s).map(|t| (t.text, t.bpm));
    assert_eq!(t("Allegro 132"), Some(("Allegro".into(), 132.0)));
    assert_eq!(t("q=96"), Some(("".into(), 96.0)));
    assert_eq!(t("72"), Some(("".into(), 72.0)));
    assert_eq!(t("Adagio"), Some(("Adagio".into(), 70.0)));
    assert_eq!(t("Andante = 84"), Some(("Andante".into(), 84.0)));
    assert!(t("Whatever").is_none() && t("q=2").is_none());
}

#[test]
fn bar_positions() {
    let mut h = header();
    h.meter.push(Meter { at: f(2, 1), num: 3, den: 4, pickup: false, beams: [0; 8] });
    assert_eq!(bar_start_at_or_before(&h, f(3, 2)), f(1, 1));
    assert_eq!(bar_start_at_or_before(&h, f(2, 1)), f(2, 1));
    assert_eq!(bar_start_at_or_before(&h, f(23, 8)), f(11, 4));
    assert_eq!(bar_length(&h, f(3, 1)), f(3, 4));
    assert_eq!(nearest_barline(&h, f(9, 8)), f(1, 1));
    assert_eq!(nearest_barline(&h, f(15, 8)), f(2, 1));
    assert_eq!(nearest_barline(&h, f(63, 8)), f(8, 1));
    assert_eq!(bar_number(&h, f(9, 4)), (3, f(1, 4)));
    assert_eq!(round_up_to_bar(&h, f(33, 4)), f(35, 4));
}

#[test]
fn pickup_bars() {
    let q = f(1, 4);
    assert_eq!(parse_pickup("pickup 1/4", q), Some(Some(f(1, 4))));
    assert_eq!(parse_pickup("Pickup 2", f(1, 8)), Some(Some(f(1, 4))));
    assert_eq!(parse_pickup("no pickup", q), Some(None));
    assert_eq!(parse_pickup("3/4", q), None);
    assert_eq!(parse_pickup("pickup 0/4", q), None);

    let mut h = header();
    let n = |at: Frac| Event { id: 9, kind: EventKind::Rest, start: at, duration: f(1, 4), heads: vec![], articulations: 0, dynamic: None, spans: 0, lyrics: vec![], graces: vec![], fingering: vec![] };
    Arc::make_mut(&mut h.lanes[0].voices[0].events).push(n(f(1, 1)));
    set_pickup(&mut h, Some(f(1, 4)));
    assert_eq!(pickup(&h), Some(f(1, 4)));
    assert_eq!(h.length, f(33, 4));
    assert_eq!(h.lanes[0].voices[0].events[0].start, f(5, 4));
    assert_eq!(bar_starts(&h)[..3], [Frac::ZERO, f(1, 4), f(5, 4)]);
    assert_eq!(bar_number(&h, f(1, 8)), (0, f(1, 8)));
    assert_eq!(bar_number(&h, f(5, 4)), (2, Frac::ZERO));
    // A time signature on the pickup applies from bar 1.
    set_meter(&mut h, Frac::ZERO, 3, 4);
    assert_eq!(h.meter.len(), 2);
    assert_eq!((h.meter[1].at, h.meter[1].num), (f(1, 4), 3));
    // A longer pickup, then none.
    set_pickup(&mut h, Some(f(1, 2)));
    assert_eq!(pickup(&h), Some(f(1, 2)));
    assert_eq!((h.meter[1].at, h.meter[1].num), (f(1, 2), 3));
    assert_eq!(h.lanes[0].voices[0].events[0].start, f(3, 2));
    set_pickup(&mut h, None);
    assert_eq!(pickup(&h), None);
    assert_eq!(h.meter.len(), 1);
    assert_eq!((h.meter[0].at, h.meter[0].num), (Frac::ZERO, 3));
    assert_eq!(h.length, f(8, 1));
    assert_eq!(h.lanes[0].voices[0].events[0].start, f(1, 1));
}

#[test]
fn header_edits() {
    let mut ids = Ids(5000);
    let mut h = header();
    set_key(&mut h, f(1, 1), KeySig { fifths: 2, minor: false });
    set_key(&mut h, f(1, 1), KeySig { fifths: -1, minor: false });
    assert!(h.keys.len() == 2 && h.keys[1].fifths == -1);
    set_meter(&mut h, f(2, 1), 6, 8);
    assert!(h.meter.len() == 2 && h.meter[1].num == 6);
    set_tempo(&mut h, Frac::ZERO, &TempoSpec { text: "Lento".into(), bpm: 56.0 });
    assert!(h.tempo.len() == 1 && h.tempo[0].text == "Lento" && h.tempo[0].bpm == 56.0);
    set_barline(&mut h, f(4, 1), BarKind::Double, &mut ids);
    set_barline(&mut h, f(4, 1), BarKind::RepeatEnd, &mut ids);
    assert!(h.barlines.len() == 1 && h.barlines[0].kind == BarKind::RepeatEnd);
    toggle_rehearsal_mark(&mut h, f(3, 1), &mut ids);
    assert_eq!(h.rehearsal.len(), 1);
    toggle_rehearsal_mark(&mut h, f(3, 1), &mut ids);
    assert!(h.rehearsal.is_empty());
    set_clef(&mut h, 1, f(1, 1), "bass", &mut ids);
    assert!(h.lanes[0].clefs.len() == 1 && h.lanes[0].clefs[0].clef == "bass");
    add_text(&mut h, 1, f(2, 1), "dolce", TextKind::Expression, &mut ids);
    add_text(&mut h, 1, f(1, 1), "pizz.", TextKind::Technique, &mut ids);
    assert!(h.lanes[0].texts.len() == 2 && h.lanes[0].texts[0].text == "pizz.");
}

#[test]
fn insert_delete_header() {
    let mut ids = Ids(5000);
    let mut h = header();
    set_key(&mut h, f(2, 1), KeySig { fifths: 1, minor: false });
    set_barline(&mut h, f(4, 1), BarKind::Double, &mut ids);
    set_clef(&mut h, 1, f(1, 1), "bass", &mut ids);
    insert_span(&mut h, f(1, 1), f(2, 1));
    assert_eq!(h.length, f(10, 1));
    assert!(h.keys[0].at == Frac::ZERO && h.keys[1].at == f(4, 1));
    assert_eq!(h.barlines[0].at, f(6, 1));
    assert_eq!(h.lanes[0].clefs[0].at, f(3, 1));
    assert_eq!(h.meter[0].at, Frac::ZERO);
    delete_span(&mut h, Frac::ZERO, f(4, 1));
    assert_eq!(h.length, f(6, 1));
    assert!(h.keys.len() == 1 && h.keys[0].at == Frac::ZERO && h.keys[0].fifths == 1);
    assert!(h.lanes[0].clefs.is_empty());
    assert_eq!(h.barlines[0].at, f(2, 1));
}

#[test]
fn insert_delete_events() {
    let mut ids = Ids(5000);
    let v = vec![
        make_event(&mut ids, Frac::ZERO, f(1, 2), &[p(0, 0, 4)], 0),
        make_event(&mut ids, f(1, 2), f(1, 1), &[p(1, 0, 4)], 0),
        make_event(&mut ids, f(2, 1), f(1, 1), &[p(2, 0, 4)], 0),
    ];
    let ins = insert_span_events(&v, f(1, 1), f(1, 1));
    assert!(ins[0].start == Frac::ZERO && ins[1].start == f(1, 2) && ins[2].start == f(3, 1));
    let del = delete_span_events(&v, f(1, 1), f(2, 1));
    assert_eq!(del.len(), 3);
    assert_eq!(del[1].duration, f(1, 2));
    assert!(!del[1].heads[0].tie);
    assert_eq!(del[2].start, f(1, 1));
    let del2 = delete_span_events(&v, f(1, 1), f(3, 1));
    assert!(del2.len() == 2 && del2[1].duration == f(1, 2));
}

#[test]
fn remove_items_by_id() {
    let mut ids = Ids(5000);
    let mut h = header();
    set_barline(&mut h, f(1, 1), BarKind::Final, &mut ids);
    toggle_rehearsal_mark(&mut h, f(1, 1), &mut ids);
    add_text(&mut h, 1, Frac::ZERO, "dolce", TextKind::Expression, &mut ids);
    let (text, mark) = (h.lanes[0].texts[0].id, h.rehearsal[0].id);
    assert_eq!(describe_item(&h, mark).as_deref(), Some("Rehearsal mark"));
    assert_eq!(remove_item(&mut h, text).as_deref(), Some("Expression \"dolce\""));
    assert!(h.lanes[0].texts.is_empty() && h.rehearsal.len() == 1 && h.barlines.len() == 1);
    assert!(remove_item(&mut h, text).is_none());
    let b = h.barlines[0].id;
    assert_eq!(remove_item(&mut h, b).as_deref(), Some("Final barline"));
    assert!(h.barlines.is_empty());
}

#[test]
fn text_items_follow_their_bars() {
    let mut ids = Ids(5000);
    let mut h = header();
    add_text(&mut h, 1, Frac::ZERO, "dolce", TextKind::Expression, &mut ids);
    add_text(&mut h, 1, f(1, 1), "pizz.", TextKind::Technique, &mut ids);
    add_text(&mut h, 1, f(1, 1), "sul pont.", TextKind::Technique, &mut ids);
    set_barline(&mut h, f(1, 1), BarKind::RepeatStart, &mut ids);
    insert_span(&mut h, Frac::ZERO, f(1, 1));
    assert!(h.lanes[0].texts[0].at == f(1, 1) && h.lanes[0].texts[1].at == f(2, 1));
    assert_eq!(h.barlines[0].at, f(2, 1));
    assert_eq!(h.keys[0].at, Frac::ZERO);
    delete_span(&mut h, f(1, 1), f(2, 1));
    assert!(h.lanes[0].texts.len() == 2 && h.lanes[0].texts[0].at == f(1, 1) && h.lanes[0].texts[1].at == f(1, 1));
}

#[test]
fn delete_bars_keeps_barlines_between_bars() {
    let mut ids = Ids(5000);
    let mut h = header();
    set_barline(&mut h, f(2, 1), BarKind::RepeatEnd, &mut ids);
    set_barline(&mut h, f(3, 1), BarKind::Double, &mut ids);
    set_barline(&mut h, f(8, 1), BarKind::Final, &mut ids);
    delete_span(&mut h, f(2, 1), f(4, 1));
    assert!(h.barlines.len() == 2 && h.barlines[0].at == f(2, 1) && h.barlines[0].kind == BarKind::RepeatEnd);
    assert!(h.barlines[1].at == f(6, 1) && h.barlines[1].kind == BarKind::Final);
    delete_span(&mut h, f(5, 1), f(6, 1));
    assert!(h.length == f(5, 1) && h.barlines.last().unwrap().at == f(5, 1) && h.barlines.last().unwrap().kind == BarKind::Final);
}

#[test]
fn ornaments() {
    assert_eq!(parse_ornament("tr"), Some(art::TRILL));
    assert_eq!(parse_ornament(" Mordent "), Some(art::MORDENT));
    assert_eq!(parse_ornament("//"), Some(art::CAESURA));
    assert_eq!(parse_ornament(","), Some(art::BREATH));
    assert_eq!(parse_ornament("arp"), Some(art::ARPEGGIO));
    assert!(parse_ornament("glissando").is_none());
}

#[test]
fn endings() {
    assert_eq!(parse_ending("1"), Some(1));
    assert_eq!(parse_ending("2."), Some(2));
    assert_eq!(parse_ending("1, 2"), Some(3));
    assert_eq!(parse_ending("1-3"), Some(7));
    assert_eq!(parse_ending("1.2."), Some(3));
    for bad in ["0", "12", "x", "3-1", "1-"] {
        assert!(parse_ending(bad).is_none(), "{bad}");
    }
    assert_eq!(ending_label(1), "1.");
    assert_eq!(ending_label(3), "1., 2.");
    assert_eq!(ending_label(7), "1.–3.");
    assert_eq!(ending_label(5), "1., 3.");
    let mut ids = Ids(5000);
    let mut h = header();
    set_ending(&mut h, f(3, 1), f(4, 1), 1, &mut ids);
    set_ending(&mut h, f(4, 1), f(5, 1), 2, &mut ids);
    set_ending(&mut h, f(3, 1), f(4, 1), 3, &mut ids);
    assert!(h.endings.len() == 2 && h.endings[0].passes == 3 && h.endings[1].from == f(4, 1));
    insert_span(&mut h, f(7, 2), f(1, 1));
    assert!(h.endings[0].from == f(3, 1) && h.endings[0].to == f(5, 1) && h.endings[1].from == f(5, 1));
    delete_span(&mut h, f(4, 1), f(6, 1));
    assert!(h.endings.len() == 1 && h.endings[0].to == f(4, 1));
    assert_eq!(describe_item(&h, h.endings[0].id).as_deref(), Some("Ending 1., 2."));
}

#[test]
fn chord_symbols() {
    assert_eq!(parse_chord("C").as_deref(), Some("C"));
    assert_eq!(parse_chord("f#m7").as_deref(), Some("F♯m7"));
    assert_eq!(parse_chord("Bbmaj7").as_deref(), Some("B♭maj7"));
    assert_eq!(parse_chord("Eb7(#9)/G").as_deref(), Some("E♭7(♯9)/G"));
    assert_eq!(parse_chord("C#m7b5/E").as_deref(), Some("C♯m7♭5/E"));
    assert_eq!(parse_chord("Bdim7").as_deref(), Some("B°7"));
    assert_eq!(parse_chord("Cø").as_deref(), Some("Cø"));
    assert_eq!(parse_chord("Gsus4").as_deref(), Some("Gsus4"));
    assert_eq!(parse_chord("C6/9").as_deref(), Some("C6/9"));
    assert_eq!(parse_chord("Dadd9").as_deref(), Some("Dadd9"));
    for bad in ["H7", "Cxyz", "", "C/H", "7"] {
        assert!(parse_chord(bad).is_none(), "{bad}");
    }
    let mut ids = Ids(5000);
    let mut h = header();
    set_chord(&mut h, 1, f(1, 2), "F♯m7", &mut ids);
    set_chord(&mut h, 1, f(1, 2), "D", &mut ids); // replaces
    set_chord(&mut h, 1, f(3, 1), "G7", &mut ids);
    assert_eq!(h.lanes[0].chords.len(), 2);
    let id = h.lanes[0].chords[0].id;
    assert_eq!(describe_item(&h, id).as_deref(), Some("Chord symbol D"));
    insert_span(&mut h, Frac::ZERO, f(1, 1));
    assert_eq!(h.lanes[0].chords[1].at, f(4, 1));
    delete_span(&mut h, f(1, 1), f(2, 1));
    assert_eq!(h.lanes[0].chords.len(), 1, "the chord in the deleted bar goes");
    let g7 = h.lanes[0].chords[0].id;
    assert_eq!(remove_item(&mut h, g7).as_deref(), Some("Chord symbol G7"));
}

#[test]
fn lyrics_on_notes() {
    let mut ids = Ids(1000);
    let mut v = two_bars(&mut ids);
    // Tie d4 into e4 (same pitch for the test): e4 is a continuation.
    v[1].heads[0].tie = true;
    v[2].heads[0].pitch = v[1].heads[0].pitch;
    let r = set_lyric(&v, v[0].heads[0].id, 1, "Glo", true, false).unwrap();
    assert_eq!(r.events[0].lyrics[0].text, "Glo");
    assert!(r.events[0].lyrics[0].hyphen);
    assert_eq!(next_note(&r.events, v[0].heads[0].id).unwrap().id, v[1].id);
    assert_eq!(next_note(&r.events, v[1].heads[0].id).unwrap().id, v[3].id, "skips the tie continuation");
    let r2 = set_lyric(&r.events, v[0].id, 2, "Ah", false, false).unwrap();
    assert_eq!(r2.events[0].lyrics.iter().map(|l| l.verse).collect::<Vec<_>>(), vec![1, 2]);
    let r3 = set_lyric(&r2.events, v[0].id, 1, "", false, false).unwrap();
    assert_eq!(r3.events[0].lyrics.len(), 1, "empty text removes the syllable");
    let rest = make_event(&mut ids, f(2, 1), f(1, 4), &[], 0);
    assert!(set_lyric(&[rest.clone()], rest.id, 1, "x", false, false).is_none(), "no lyrics on rests");
}

// --------------------------------------------------------------- tuplets --
#[test]
fn tuplet_popover_and_ratios() {
    use corinzia_edit::tuplets::*;
    assert_eq!(parse_tuplet("3"), Some((3, 2)));
    assert_eq!(parse_tuplet("5"), Some((5, 4)));
    assert_eq!(parse_tuplet("7"), Some((7, 4)));
    assert_eq!(parse_tuplet("9"), Some((9, 8)));
    assert_eq!(parse_tuplet("2"), Some((2, 3)));
    assert_eq!(parse_tuplet(" 7:8 "), Some((7, 8)));
    assert!(parse_tuplet("x").is_none() && parse_tuplet("1").is_none() && parse_tuplet("4:4").is_none());
}

#[test]
fn tuplets_follow_edits() {
    use corinzia_edit::tuplets::*;
    use corinzia_model::Tuplet;
    let mut ids = Ids(100);
    // A bar of quarters C D E F.
    let mut evs = Vec::new();
    for (i, s) in [0, 1, 2, 3].iter().enumerate() {
        evs.push(make_event(&mut ids, f(i as i64, 4), f(1, 4), &[Pitch { step: *s, alter: 0, octave: 5 }], 0));
    }
    // A triplet of eighths made from D: D keeps its pitch, a rest fills the rest.
    let d = evs[1].clone();
    let (r, ts) = create(&evs, &[], f(1, 4), f(1, 8), 3, 2, Some(&d), true, &mut ids).unwrap();
    assert_eq!(ts, vec![Tuplet { start: f(1, 4), length: f(1, 4), actual: 3, normal: 2 }]);
    let inside: Vec<(Frac, Frac, bool)> = r.events.iter().filter(|e| ts[0].contains(e.start)).map(|e| (e.start, e.duration, e.is_rest())).collect();
    assert_eq!(inside, [(f(1, 4), f(1, 12), false), (f(1, 3), f(1, 6), true)]);
    assert_eq!(sounding_scale(&ts, f(1, 3)), f(2, 3));
    assert_eq!(sounding_scale(&ts, f(1, 2)), f(1, 1));
    // A note crossing its end removes it; one inside keeps it.
    let crossing = insert_event(&r.events, make_event(&mut ids, f(1, 3), f(1, 4), &[], 0), true, &mut ids);
    assert!(sanitize(&crossing.events, &ts).is_empty());
    let inner = insert_event(&r.events, make_event(&mut ids, f(1, 3), f(1, 12), &[], 0), true, &mut ids);
    assert_eq!(sanitize(&inner.events, &ts), ts);
    // Bars inserted before it move it; a deleted span holding it removes it.
    assert_eq!(shift(&ts, Frac::ZERO, f(1, 1))[0].start, f(5, 4));
    assert!(cut(&ts, Frac::ZERO, f(1, 2)).is_empty());
    assert_eq!(cut(&ts, Frac::ZERO, f(1, 4))[0].start, Frac::ZERO);
    // Copy and paste carry it.
    let clip = slice(&ts, f(1, 4), f(1, 2));
    assert_eq!(paste(&[], &clip, f(2, 1), f(1, 4))[0].start, f(2, 1));
    // An eighth quintuplet inside the triplet would not fit; a nested triplet does.
    assert!(create(&r.events, &ts, f(1, 4), f(1, 8), 5, 4, None, true, &mut ids).is_none());
    let (_, nested) = create(&r.events, &ts, f(1, 4), f(1, 16), 3, 2, None, true, &mut ids).unwrap();
    assert_eq!(nested.len(), 2);
    assert_eq!(nested[1].length, f(1, 12));
}

#[test]
fn navigation_marks_parse_toggle_and_move_with_bars() {
    assert_eq!(parse_nav("Segno"), Some(Nav::Segno));
    assert_eq!(parse_nav("to coda"), Some(Nav::ToCoda));
    assert_eq!(parse_nav("Fine"), Some(Nav::Fine));
    assert_eq!(parse_nav("D.C."), Some(Nav::DaCapo(NavTo::End)));
    assert_eq!(parse_nav("D.C. al Fine"), Some(Nav::DaCapo(NavTo::Fine)));
    assert_eq!(parse_nav("ds al coda"), Some(Nav::DalSegno(NavTo::Coda)));
    assert_eq!(parse_nav("Dal Segno al Fine"), Some(Nav::DalSegno(NavTo::Fine)));
    assert!(parse_nav("1, 2").is_none() && parse_nav("dc al nowhere").is_none());
    let mut ids = Ids(900);
    let mut h = header();
    toggle_nav(&mut h, f(1, 1), Nav::Segno, &mut ids);
    toggle_nav(&mut h, f(3, 1), Nav::DalSegno(NavTo::End), &mut ids);
    assert_eq!(h.navigation.len(), 2);
    // A bar inserted at bar 2 moves the segno (on its bar's start) but not
    // a mark ending bar 1.
    toggle_nav(&mut h, f(1, 1), Nav::Fine, &mut ids);
    insert_span(&mut h, f(1, 1), f(1, 1));
    let at = |h: &Movement, k: Nav| h.navigation.iter().find(|n| n.kind == k).map(|n| n.at);
    assert_eq!(at(&h, Nav::Segno), Some(f(2, 1)));
    assert_eq!(at(&h, Nav::Fine), Some(f(1, 1)));
    assert_eq!(at(&h, Nav::DalSegno(NavTo::End)), Some(f(4, 1)));
    // Deleting that bar again, and toggling the Fine off.
    delete_span(&mut h, f(1, 1), f(2, 1));
    assert_eq!(at(&h, Nav::Segno), Some(f(1, 1)));
    toggle_nav(&mut h, f(1, 1), Nav::Fine, &mut ids);
    assert_eq!(at(&h, Nav::Fine), None);
    // By id: described and removed like other structure items.
    let id = h.navigation[0].id;
    assert_eq!(describe_item(&h, id).as_deref(), Some("Segno"));
    assert!(remove_item(&mut h, id).is_some());
    assert_eq!(h.navigation.len(), 1);
}

#[test]
fn octave_lines_and_pedals_are_set_split_and_moved() {
    assert_eq!(parse_octave_line("8va"), Some(Some(LineKind::Octave(1))));
    assert_eq!(parse_octave_line("15mb"), Some(Some(LineKind::Octave(-2))));
    assert_eq!(parse_octave_line("loco"), Some(None));
    assert_eq!(parse_octave_line("treble"), None);
    assert_eq!(parse_pedal("Ped."), Some(true));
    assert_eq!(parse_pedal("*"), Some(false));
    assert_eq!(parse_pedal("pizz."), None);
    let mut ids = Ids(700);
    let mut h = header();
    set_line(&mut h, 0, Frac::ZERO, f(2, 1), LineKind::Octave(1), &mut ids);
    set_line(&mut h, 0, f(1, 1), f(3, 1), LineKind::Pedal, &mut ids);
    // An 8vb over bar 2 splits the 8va around it; the pedal stays.
    set_line(&mut h, 0, f(1, 1), f(3, 2), LineKind::Octave(-1), &mut ids);
    let spans = |h: &Movement| h.lanes[0].lines.iter().map(|l| (l.from, l.to, l.kind)).collect::<Vec<_>>();
    assert_eq!(
        spans(&h),
        [
            (Frac::ZERO, f(1, 1), LineKind::Octave(1)),
            (f(1, 1), f(3, 1), LineKind::Pedal),
            (f(1, 1), f(3, 2), LineKind::Octave(-1)),
            (f(3, 2), f(2, 1), LineKind::Octave(1)),
        ]
    );
    // Loco over everything removes the octave lines only.
    remove_lines(&mut h, 0, Frac::ZERO, f(8, 1), false, &mut ids);
    assert_eq!(spans(&h), [(f(1, 1), f(3, 1), LineKind::Pedal)]);
    // A bar inserted before moves it; one deleted inside shortens it.
    insert_span(&mut h, Frac::ZERO, f(1, 1));
    assert_eq!(spans(&h)[0].0, f(2, 1));
    delete_span(&mut h, f(2, 1), f(3, 1));
    assert_eq!(spans(&h), [(f(2, 1), f(3, 1), LineKind::Pedal)]);
    let id = h.lanes[0].lines[0].id;
    assert_eq!(describe_item(&h, id).as_deref(), Some("Pedal"));
}

#[test]
fn fingering_and_bowing_popovers() {
    assert_eq!(parse_fingering("3"), Some(vec![3]));
    assert_eq!(parse_fingering("1 3 5"), Some(vec![1, 3, 5]));
    assert_eq!(parse_fingering("1,2"), Some(vec![1, 2]));
    assert_eq!(parse_fingering(""), Some(vec![]));
    assert!(parse_fingering("7").is_none() && parse_fingering("x").is_none());
    assert_eq!(parse_bowing("down bow"), Some(art::DOWN_BOW));
    assert_eq!(parse_bowing("upbow"), Some(art::UP_BOW));
    assert_eq!(parse_bowing("pizz."), None);
    assert_eq!(parse_ornament("up bow"), Some(art::UP_BOW));
}

#[test]
fn transposition_intervals_and_keys() {
    use corinzia_edit::{interval_between, key_tonic, parse_interval, parse_transpose, TransposeSpec};
    use corinzia_model::Interval;
    assert_eq!(parse_interval("M2"), Some(Interval::new(1, 2)));
    assert_eq!(parse_interval("up m3"), Some(Interval::new(2, 3)));
    assert_eq!(parse_interval("down P5"), Some(Interval::new(-4, -7)));
    assert_eq!(parse_interval("-M2"), Some(Interval::new(-1, -2)));
    assert_eq!(parse_interval("A4"), Some(Interval::new(3, 6)));
    assert_eq!(parse_interval("d5"), Some(Interval::new(4, 6)));
    assert_eq!(parse_interval("P8"), Some(Interval::new(7, 12)));
    assert_eq!(parse_interval("down an octave"), Some(Interval::new(-7, -12)));
    assert_eq!(parse_interval("M10"), Some(Interval::new(9, 16)));
    assert_eq!(parse_interval("P3"), None, "no perfect third");
    assert_eq!(parse_interval("M5"), None);
    assert_eq!(parse_interval("x"), None);
    // Tonics on the line of fifths.
    assert_eq!(key_tonic(0, false), (0, 0)); // C
    assert_eq!(key_tonic(-3, false), (2, -1)); // E-flat
    assert_eq!(key_tonic(0, true), (5, 0)); // a
    assert_eq!(key_tonic(6, false), (3, 1)); // F-sharp
    assert_eq!(key_tonic(3, true), (3, 1)); // f-sharp minor (A major's relative)
    // The nearer way: C to E-flat is up a minor third; C to A down a minor third.
    assert_eq!(interval_between((0, 0), (2, -1)), Interval::new(2, 3));
    assert_eq!(interval_between((0, 0), (5, 0)), Interval::new(-2, -3));
    assert_eq!(parse_transpose("to Eb"), Some(TransposeSpec::To { step: 2, alter: -1 }));
    assert_eq!(parse_transpose("to f#"), Some(TransposeSpec::To { step: 3, alter: 1 }));
    assert_eq!(parse_transpose("to H"), None);
    assert_eq!(parse_transpose("up M2"), Some(TransposeSpec::By(Interval::new(1, 2))));
}

#[test]
fn filters_pick_notes_by_place_pitch_voice_and_marks() {
    use corinzia_edit::select::{find, parse_query, Pick};
    use corinzia_model::*;
    use corinzia_score::Frac;
    use std::sync::Arc;
    let p = |step, octave| Pitch { step, alter: 0, octave };
    let head = |id, pitch: Pitch, tie| Head { id, pitch, tie, velocity: 0 };
    let ev = |id, start: i64, heads: Vec<Head>, art: u16, dynamic: Option<u8>| Event {
        id,
        kind: if heads.is_empty() { EventKind::Rest } else if heads.len() > 1 { EventKind::Chord } else { EventKind::Note },
        start: Frac::new(start, 4),
        duration: Frac::new(1, 4),
        heads,
        articulations: art,
        dynamic,
        spans: 0,
        lyrics: vec![],
        graces: vec![],
        fingering: vec![],
    };
    // C-E-G chord (staccato, mf), D5 tied, a rest, F#4 in voice 2.
    let v1 = vec![
        ev(1, 0, vec![head(10, p(0, 4), false), head(11, p(2, 4), false), head(12, p(4, 4), false)], corinzia_score::score::art::STACCATO, Some(4)),
        ev(2, 1, vec![head(20, p(1, 5), true)], 0, None),
        ev(3, 2, vec![], 0, None),
        ev(4, 3, vec![head(40, p(0, 5), false)], 0, Some(4)),
    ];
    let v2 = vec![ev(5, 0, vec![head(50, Pitch { step: 3, alter: 1, octave: 4 }, false)], 0, None)];
    let lane = Lane {
        player: 1,
        changes: vec![],
        clefs: vec![],
        texts: vec![TextItem { id: 70, at: Frac::ZERO, text: "dolce".into(), kind: TextKind::Expression }],
        chords: vec![ChordSymbol { id: 80, at: Frac::ZERO, text: "C".into() }],
        lines: vec![],
        voices: vec![Voice { id: 6, index: 1, events: Arc::new(v1), tuplets: vec![] }, Voice { id: 7, index: 2, events: Arc::new(v2), tuplets: vec![] }],
    };
    let m = Movement { id: 1, ordinal: 0, title: String::new(), length: Frac::int(1), revision: 0, tempo: vec![], meter: vec![], keys: vec![], barlines: vec![], rehearsal: vec![], endings: vec![], navigation: vec![], lanes: vec![lane], slurs: vec![] };
    let all = |q: &str| find(&m, 0..=0, Frac::ZERO, Frac::int(1), &parse_query(q).unwrap());
    assert_eq!(all("notes"), [10, 11, 12, 20, 40, 50]);
    assert_eq!(all("top"), [12, 20, 40, 50]);
    assert_eq!(all("bottom"), [10, 20, 40, 50]);
    assert_eq!(all("top voice 1"), [12, 20, 40]);
    assert_eq!(all("chords"), [10, 11, 12]);
    assert_eq!(all("voice 2"), [50]);
    assert_eq!(all("above C5"), [20]);
    assert_eq!(all("below E4"), [10]);
    assert_eq!(all("C"), [10, 40]);
    assert_eq!(all("C5"), [40]);
    assert_eq!(all("f#"), [50]);
    assert_eq!(all("staccato"), [10, 11, 12]);
    assert_eq!(all("stacc top"), [12]);
    assert_eq!(all("tied"), [20]);
    assert_eq!(all("rests"), [3]);
    assert_eq!(all("dynamics"), [corinzia_score::score::ids::dynamic(1), corinzia_score::score::ids::dynamic(4)]);
    assert_eq!(all("mf").len(), 2);
    assert!(all("dynamics ff").is_empty());
    assert_eq!(all("chord symbols"), [80]);
    assert_eq!(all("text dolce"), [70]);
    assert!(all("text espress.").is_empty());
    // Only what starts inside the passage.
    assert_eq!(find(&m, 0..=0, Frac::new(1, 4), Frac::new(3, 4), &parse_query("notes").unwrap()), [20]);
    assert!(parse_query("purple").is_err());
    assert!(parse_query("above C").is_err(), "needs an octave");
    assert_eq!(parse_query("lyrics verse 2").unwrap().pick, Pick::Lyrics(Some(2)));
}

#[test]
fn explode_spreads_chords_and_reduce_combines_staves() {
    use corinzia_edit::staves::{explode, reduce};
    use corinzia_model::*;
    use corinzia_score::Frac;
    let p = |step, octave| Pitch { step, alter: 0, octave };
    let h = |pitch| Head { id: 1, pitch, tie: false, velocity: 0 };
    let ev = |start: Frac, dur: Frac, heads: Vec<Head>| Event {
        id: 1,
        kind: if heads.is_empty() { EventKind::Rest } else { EventKind::Note },
        start,
        duration: dur,
        heads,
        articulations: 0,
        dynamic: None,
        spans: 0,
        lyrics: vec![],
        graces: vec![],
        fingering: vec![],
    };
    let q = Frac::new(1, 4);
    // C-E-G-C chord, E-G, a single A, a rest.
    let src = vec![
        ev(Frac::ZERO, q, vec![h(p(0, 4)), h(p(2, 4)), h(p(4, 4)), h(p(0, 5))]),
        ev(q, q, vec![h(p(2, 4)), h(p(4, 4))]),
        ev(q * Frac::int(2), q, vec![h(p(5, 4))]),
        ev(q * Frac::int(3), q, vec![]),
    ];
    let names = |e: &Event| e.heads.iter().map(|h| format!("{}{}", "CDEFGAB".as_bytes()[h.pitch.step as usize] as char, h.pitch.octave)).collect::<Vec<_>>().join("-");
    let three = explode(&src, Frac::ZERO, Frac::int(1), 3);
    assert_eq!(three.len(), 3);
    let col = |k: usize| three.iter().map(|s| names(&s[k])).collect::<Vec<_>>();
    assert_eq!(col(0), ["C5", "G4", "C4-E4"], "more notes than staves: the bottom keeps the rest");
    assert_eq!(col(1), ["G4", "E4", "E4"], "fewer: the lowest is doubled");
    assert_eq!(col(2), ["A4", "A4", "A4"]);
    assert!(three.iter().all(|s| s[3].is_rest()));
    // Reduce: a half note against two quarters; a rest against a note.
    let top = vec![ev(Frac::ZERO, q * Frac::int(2), vec![h(p(4, 5))]), ev(q * Frac::int(2), q * Frac::int(2), vec![])];
    let bottom = vec![ev(Frac::ZERO, q, vec![h(p(0, 4))]), ev(q, q, vec![h(p(4, 5))]), ev(q * Frac::int(2), q, vec![h(p(2, 4))]), ev(q * Frac::int(3), q, vec![])];
    let r = reduce(&[&top, &bottom], Frac::ZERO, Frac::int(1));
    assert_eq!(r.iter().map(names).collect::<Vec<_>>(), ["C4-G5", "G5", "E4", ""]);
    assert!(r[0].heads[1].tie, "the held G is tied over the cut");
    assert!(!r[0].heads[0].tie);
    assert_eq!(r[1].heads.len(), 1, "G in both staves sounds once");
    assert_eq!(r[3].duration, q, "the closing rest");
    assert_eq!(r.iter().map(|e| e.duration).fold(Frac::ZERO, |a, b| a + b), Frac::int(1));
}

#[test]
fn time_signatures_with_beam_groups() {
    use corinzia_edit::parse_time_grouped;
    let g = |v: &[u8]| {
        let mut b = [0u8; 8];
        b[..v.len()].copy_from_slice(v);
        b
    };
    assert_eq!(parse_time_grouped("7/8 2+2+3"), Some((7, 8, g(&[2, 2, 3]))));
    assert_eq!(parse_time_grouped("2+2+3/8"), Some((7, 8, g(&[2, 2, 3]))));
    assert_eq!(parse_time_grouped("(3+2)/8"), Some((5, 8, g(&[3, 2]))));
    assert_eq!(parse_time_grouped("4/4 3+3+2"), None, "groups in quarters must make 4");
    assert_eq!(parse_time_grouped("4/4 1+3"), Some((4, 4, g(&[1, 3]))));
    assert_eq!(parse_time_grouped("6/8"), Some((6, 8, [0; 8])));
    assert_eq!(parse_time_grouped("C"), Some((4, 4, [0; 8])));
    assert_eq!(parse_time_grouped("7/8 2+x"), None);
}
