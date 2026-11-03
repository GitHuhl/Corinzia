use corinzia_edit::names::*;
use corinzia_model::{Interval, Pitch};

#[test]
fn interval_names() {
    assert_eq!(interval_name(Interval::new(3, 6)), "up an augmented 4th");
    assert_eq!(interval_name(Interval::new(-2, -3)), "down a minor 3rd");
}

#[test]
fn keys_are_spelled_in_the_key() {
    // C# in D major, Db in F major, Bb in F major.
    assert_eq!(pitch_name(spell_key(61, 2)), "C♯4");
    assert_eq!(pitch_name(spell_key(61, -1)), "D♭4");
    assert_eq!(pitch_name(spell_key(70, -1)), "B♭4");
    assert_eq!(pitch_name(spell_key(60, -7)), "C4", "not in C-flat major: natural");
}

#[test]
fn pitch_names_and_dynamics() {
    assert_eq!(pitch_name(Pitch { step: 6, alter: -1, octave: 3 }), "B♭3");
    assert_eq!(parse_dynamic(" "), Ok(None));
    assert!(parse_dynamic("mf").unwrap().is_some());
    assert!(parse_dynamic("loud").is_err());
}

#[test]
fn top_lines_of_clefs() {
    assert_eq!(top_line("treble"), 38);
    assert_eq!(top_line("bass"), 26);
    assert_eq!(top_line("alto"), 32);
}
