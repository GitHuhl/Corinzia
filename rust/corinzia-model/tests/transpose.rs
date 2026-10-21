use corinzia_model::notation::{score_as, transpose_chord_text};
use corinzia_model::*;
use corinzia_score::Frac;
use std::sync::Arc;

fn p(step: i8, alter: i8, octave: i8) -> Pitch {
    Pitch { step, alter, octave }
}

#[test]
fn intervals_spell_and_move_keys() {
    let m2 = Interval::new(1, 2);
    assert_eq!(p(0, 0, 4).transposed(m2), p(1, 0, 4)); // C -> D
    assert_eq!(p(2, 0, 4).transposed(m2), p(3, 1, 4)); // E -> F#
    assert_eq!(p(6, -1, 4).transposed(m2), p(0, 0, 5)); // Bb -> C
    assert_eq!(p(1, 0, 5).transposed(m2.inverse()), p(0, 0, 5));
    assert_eq!(p(0, 0, 4).transposed(Interval::new(4, 7)), p(4, 0, 4)); // up a P5: G
    assert_eq!(p(0, 0, 4).transposed(Interval::new(8, 14)), p(1, 0, 5)); // up a M9: D5
    assert_eq!(m2.fifths(), 2);
    assert_eq!(Interval::new(4, 7).fifths(), 1);
    assert_eq!(Interval::new(5, 9).fifths(), 3); // alto sax: M6
    assert_eq!(Interval::new(2, 3).fifths(), -3); // A clarinet: m3
    assert_eq!(shift_key(-2, 2), 0); // B-flat major for a B-flat instrument: C
    assert_eq!(shift_key(7, 2), -3); // C# major up a M2: E-flat, not D#
    assert_eq!(transpose_chord_text("Bb7/D", m2), "C7/E");
    assert_eq!(transpose_chord_text("F#m7b5", m2), "G#m7b5");
    assert_eq!(transpose_chord_text("Cmaj7", Interval::new(5, 9)), "Amaj7");
}

#[test]
fn transposed_view_writes_pitches_and_keys() {
    let mut proj = Project { next_id: 100, ..Default::default() };
    proj.players.push(Player { id: 1, name: "Clarinet".into(), instruments: vec![2] });
    proj.instruments.push(Instrument { id: 2, player: 1, name: "Clarinet in B♭".into(), engraver_name: "Cl.".into(), transposition: Interval::new(1, 2), clef: "treble".into(), track: None, drums: vec![], tuning: vec![] });
    let note = Event { id: 10, kind: EventKind::Note, start: Frac::ZERO, duration: Frac::int(1), heads: vec![Head { id: 11, pitch: p(6, -1, 4), tie: false, velocity: 0 }], articulations: 0, dynamic: None, spans: 0, lyrics: vec![], graces: vec![], fingering: vec![] };
    let m = Movement {
        id: 3,
        ordinal: 0,
        title: String::new(),
        length: Frac::int(1),
        revision: 0,
        tempo: vec![],
        meter: vec![Meter { at: Frac::ZERO, num: 4, den: 4, pickup: false, beams: [0; 8] }],
        keys: vec![Key { at: Frac::ZERO, fifths: -2, minor: false }],
        barlines: vec![],
        rehearsal: vec![],
        navigation: vec![],
        slurs: vec![],
        endings: vec![],
        lanes: vec![Lane {
            player: 1,
            changes: vec![InstrumentChange { id: 4, at: Frac::ZERO, instrument: 2 }],
            clefs: vec![],
            texts: vec![],
            chords: vec![ChordSymbol { id: 5, at: Frac::ZERO, text: "Bb".into() }],
            lines: vec![],
            voices: vec![Voice { id: 6, index: 1, events: Arc::new(vec![note]), tuplets: vec![] }],
        }], breaks: vec![], cross: vec![],
    };
    let concert = score_as(&proj, &m, false);
    let written = score_as(&proj, &m, true);
    let pitch = |s: &corinzia_score::score::Score| { let q = &s.parts[0].voices[0].events[0].pitches[0]; (q.step, q.alter, q.octave) };
    assert_eq!(pitch(&concert), (6, -1, 4)); // sounding B-flat
    assert_eq!(pitch(&written), (0, 0, 5)); // written C
    assert_eq!((concert.parts[0].key_shift, written.parts[0].key_shift), (0, 2));
    assert_eq!(written.parts[0].chords[0].text, "C");
}

#[test]
fn instrument_families_for_brackets() {
    use corinzia_model::notation::family;
    for (name, fam) in [
        ("Flute", Some("woodwinds")),
        ("Bass Clarinet in Bb", Some("woodwinds")),
        ("English Horn", Some("woodwinds")),
        ("Horn in F", Some("brass")),
        ("Bass Trombone", Some("brass")),
        ("Timpani", Some("percussion")),
        ("Bass Drum", Some("percussion")),
        ("Violin I", Some("strings")),
        ("Violoncello", Some("strings")),
        ("Double Bass", Some("strings")),
        ("Soprano", Some("voices")),
        ("Bass", Some("voices")),
        ("Alto Saxophone", Some("woodwinds")),
        ("Piano", None),
        ("Harp", None),
    ] {
        assert_eq!(family(name), fam, "{name}");
    }
}
