use corinzia_byz::*;
use corinzia_score::Frac;
use Degree::*;
use Quantity::*;

fn labels(s: &[Sung]) -> Vec<String> {
    s.iter().map(|s| s.note.label()).collect()
}

#[test]
fn genera_fill_the_octave_with_the_commissions_intervals() {
    assert_eq!(Genus::Diatonic.intervals(), [12, 10, 8, 12, 12, 10, 8]);
    // Soft chromatic from Di: 8 14 8, then 12 (Ni Pa), then 8 14 8.
    let p = Genus::SoftChromatic.positions();
    assert_eq!([p[5] - p[4], p[6] - p[5], 72 - p[6]], [8, 14, 8]);
    // Hard chromatic from Pa: 6 20 4, 12, 6 20 4.
    let p = Genus::HardChromatic.positions();
    assert_eq!([p[2] - p[1], p[3] - p[2], p[4] - p[3], p[5] - p[4], p[6] - p[5], 72 + p[0] - p[6], p[1] - p[0]], [6, 20, 4, 12, 6, 20, 4]);
    for g in [Genus::Diatonic, Genus::SoftChromatic, Genus::HardChromatic] {
        assert_eq!(g.intervals().iter().sum::<i32>(), 72);
        assert_eq!(g.moria(Note::new(Pa, 0)), 12, "Pa stays put");
        assert_eq!(g.moria(Note::new(Di, 0)), 42, "Di stays put");
    }
}

#[test]
fn notes_step_across_octaves() {
    assert_eq!(Note::new(Zo, 0).step(1), Note::new(Ni, 1));
    assert_eq!(Note::new(Ni, 0).step(-1), Note::new(Zo, -1));
    assert_eq!(Note::new(Ni, 1).label(), "Ni΄");
    assert_eq!(Note::new(Ke, -1).label(), "Ke͵");
    assert_eq!(Genus::Diatonic.moria(Note::new(Ni, 1)), 72);
}

#[test]
fn neumes_move_from_the_note_before() {
    let mut c = Chant::new("Steps", Echos::PlagalFourth); // from Ni
    for q in [Ison, Oligon, Kentima, Apostrofos, OligonYpsiliRight, Chamili, Yporroi, OligonKentimataBelow, Elafron] {
        c.push(Neume::new(q));
    }
    let (sung, problems) = c.sing();
    assert!(problems.is_empty());
    assert_eq!(labels(&sung), ["Ni", "Pa", "Ga", "Vou", "Zo", "Vou", "Pa", "Ni", "Pa", "Vou", "Ni"]);
    assert!(sung.iter().all(|s| s.length == Frac::int(1)));
    assert_eq!(sung.last().unwrap().start, Frac::int(10));
}

#[test]
fn time_signs_lengthen_and_gorgons_share_a_beat() {
    let mut c = Chant::new("Time", Echos::First); // from Pa
    c.push(Neume::new(Ison).lyric("Κύ").time(TimeSign::Klasma));
    c.push(Neume::new(Oligon).lyric("ρι"));
    c.push(Neume::new(Apostrofos).time(TimeSign::Gorgon));
    c.push(Neume::new(Apostrofos).lyric("ε").time(TimeSign::Dipli));
    let (sung, _) = c.sing();
    let times: Vec<(Frac, Frac)> = sung.iter().map(|s| (s.start, s.length)).collect();
    assert_eq!(
        times,
        [(Frac::ZERO, Frac::int(2)), (Frac::int(2), Frac::new(1, 2)), (Frac::new(5, 2), Frac::new(1, 2)), (Frac::int(3), Frac::int(3))]
    );
    assert_eq!(labels(&sung), ["Pa", "Vou", "Pa", "Ni"]);
    assert_eq!(sung[0].lyric.as_deref(), Some("Κύ"));
    assert_eq!(sung[2].lyric, None);
}

#[test]
fn a_digorgon_puts_three_notes_in_one_beat() {
    let mut c = Chant::new("Digorgon", Echos::First);
    c.push(Neume::new(Ison));
    c.push(Neume::new(Oligon));
    c.push(Neume::new(Oligon).time(TimeSign::Digorgon));
    let (sung, problems) = c.sing();
    assert!(problems.is_empty());
    assert!(sung.iter().all(|s| s.length == Frac::new(1, 3)));
    assert_eq!(sung[2].start, Frac::new(2, 3));

    let mut early = Chant::new("Too early", Echos::First);
    early.push(Neume::new(Oligon).time(TimeSign::Digorgon));
    assert_eq!(early.sing().1, [Problem::GorgonTooEarly { element: 0 }]);
}

#[test]
fn the_second_mode_sings_soft_chromatic_intervals() {
    let mut c = Chant::new("Second", Echos::Second); // from Di, soft chromatic
    for q in [Ison, Oligon, Oligon, Oligon] {
        c.push(Neume::new(q));
    }
    let (sung, _) = c.sing();
    assert_eq!(labels(&sung), ["Di", "Ke", "Zo", "Ni΄"]);
    let moria: Vec<i32> = sung.iter().map(|s| s.moria).collect();
    assert_eq!(moria, [42, 50, 64, 72]);
}

#[test]
fn a_fthora_changes_the_genus_and_names_the_note() {
    // Diatonic from Pa, up to Di; a hard chromatic fthora on Di.
    let mut c = Chant::new("Fthora", Echos::First);
    c.push(Neume::new(Ison));
    c.push(Neume::new(OligonKentimaAbove).fthora(Genus::HardChromatic, Di));
    c.push(Neume::new(Oligon));
    c.push(Neume::new(Oligon));
    let (sung, _) = c.sing();
    assert_eq!(labels(&sung), ["Pa", "Di", "Ke", "Zo"]);
    assert_eq!(sung[1].genus, Genus::HardChromatic);
    // Ke and Zo are now 12 and 6 moria apart, not 12 and 10.
    assert_eq!(sung[3].moria - sung[2].moria, 6);
}

#[test]
fn martyriae_are_checked_against_the_melody() {
    let mut c = Chant::new("Martyria", Echos::First);
    c.push(Neume::new(Ison));
    c.elements.push(Element::Martyria(Pa));
    c.push(Neume::new(Oligon));
    c.elements.push(Element::Martyria(Ga));
    let (_, problems) = c.sing();
    assert_eq!(problems, [Problem::MartyriaDisagrees { element: 3, written: Ga, sung: Vou }]);
}

#[test]
fn pitches_map_to_midi_keys_and_cents() {
    let r = Reference::default(); // Ni = D4
    assert_eq!(r.key_and_cents(0), (62, 0.0));
    assert_eq!(r.key_and_cents(12), (64, 0.0)); // diatonic Pa: E4
    // Diatonic Vou is 22 moria up: 3⅔ semitones, F#4 a third flat.
    let (key, cents) = r.key_and_cents(22);
    assert_eq!(key, 66);
    assert!((cents + 33.33).abs() < 0.01, "{cents}");
}

#[test]
fn names_are_greek_first() {
    assert_eq!(Echos::PlagalSecond.greek(), "Ἦχος πλ. β΄");
    assert_eq!(Echos::Varys.greek(), "Ἦχος βαρύς");
    assert_eq!(Degree::Vou.greek(), "Βου");
}
