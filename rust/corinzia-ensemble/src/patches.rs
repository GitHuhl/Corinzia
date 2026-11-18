//! The instruments: what each sounds like, how loud it is, and which one a
//! staff's instrument name plays.
//!
//! Loudness is calibrated: `level_db` is how loud the instrument is at mf
//! next to a violin section at mf (0 dB), and `range_db` how far it goes
//! from ppp to fff. Each sound is measured once when the plugin starts
//! (see [`trims`]), so with every fader at 0 dB a score is balanced the
//! way an orchestra is: brass over woodwinds at ff, a solo flute under a
//! string section, a clarinet that can fade to almost nothing.

use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Model {
    /// Bowed strings: sawtooth through a body, bow noise.
    Bowed,
    /// A reed: pulse of this width (0.5 a square, the clarinet's odd
    /// harmonics); 0 a sawtooth (saxophones).
    Reed(f32),
    Flute,
    Brass,
    /// Voices through vowel formants.
    Choir,
    /// A plucked string (guitar, harp, pizzicato): `brightness` 0..1.
    Pluck(f32),
    Piano,
    Organ,
    /// Struck bars and bells: the modes (frequency ratio, level, decay
    /// factor) in [`MODES`].
    Mallet(usize),
    Timpani,
    /// General MIDI drum keys.
    Kit,
    /// A soft synthesizer pad: anything else.
    Pad,
}

#[derive(Clone, Copy, Debug)]
pub struct Patch {
    pub name: &'static str,
    pub model: Model,
    /// Loudness at mf against a violin section at mf, and the ppp..fff span.
    pub level_db: f32,
    pub range_db: f32,
    /// Players in unison (a section) and their spread in cents.
    pub unison: u8,
    pub spread: f32,
    /// The low-pass cutoff in multiples of the pitch at ppp and at fff.
    pub dark: f32,
    pub bright: f32,
    /// Body or vowel resonances: (Hz, Q, level).
    pub formants: [(f32, f32, f32); 3],
    /// Attack at mf and release, in seconds.
    pub attack: f32,
    pub release: f32,
    /// Vibrato depth (semitones) and rate (Hz).
    pub vibrato: f32,
    pub vib_rate: f32,
    /// Breath or bow noise.
    pub noise: f32,
    /// Slurred notes join (one player moving), see CC68.
    pub legato: bool,
    /// How long a struck or plucked sound rings (60 dB) at middle C.
    pub ring: f32,
    /// The key the instrument is measured at.
    pub ref_key: u8,
}

const NONE: (f32, f32, f32) = (0.0, 1.0, 0.0);

const fn base(name: &'static str, model: Model, level_db: f32, range_db: f32, ref_key: u8) -> Patch {
    Patch {
        name,
        model,
        level_db,
        range_db,
        unison: 1,
        spread: 0.0,
        dark: 3.0,
        bright: 12.0,
        formants: [NONE; 3],
        attack: 0.03,
        release: 0.12,
        vibrato: 0.0,
        vib_rate: 5.5,
        noise: 0.0,
        legato: false,
        ring: 2.0,
        ref_key,
    }
}

/// A bowed string. `attack`: the bow's 10-90 % rise at mf (measured:
/// quicker on the violin, slower on the double bass); `vibrato`: its
/// depth in semitones (a section's players each a little different).
const fn strings(name: &'static str, level_db: f32, unison: u8, body: [(f32, f32, f32); 3], ref_key: u8, attack: f32, vibrato: f32) -> Patch {
    let mut p = base(name, Model::Bowed, level_db, 30.0, ref_key);
    p.unison = unison;
    p.spread = if unison > 1 { 7.0 } else { 0.0 };
    p.dark = 2.5;
    p.bright = 16.0;
    p.formants = body;
    p.attack = attack;
    // The string rings on a little once the bow leaves it.
    p.release = 0.35;
    p.vibrato = vibrato;
    p.vib_rate = 5.8;
    p.noise = 0.035;
    p.legato = true;
    p
}

const fn wind(name: &'static str, model: Model, level_db: f32, range_db: f32, formants: [(f32, f32, f32); 3], ref_key: u8) -> Patch {
    let mut p = base(name, model, level_db, range_db, ref_key);
    p.dark = 2.0;
    p.bright = 10.0;
    p.formants = formants;
    p.attack = 0.035;
    p.release = 0.08;
    p.noise = 0.02;
    p.legato = true;
    p
}

const fn with_vibrato(mut p: Patch, depth: f32, rate: f32) -> Patch {
    p.vibrato = depth;
    p.vib_rate = rate;
    p
}

const fn tone(mut p: Patch, dark: f32, bright: f32) -> Patch {
    p.dark = dark;
    p.bright = bright;
    p
}

const fn timing(mut p: Patch, attack: f32, release: f32, ring: f32) -> Patch {
    p.attack = attack;
    p.release = release;
    p.ring = ring;
    p
}

const fn section(mut p: Patch, unison: u8, spread: f32) -> Patch {
    p.unison = unison;
    p.spread = spread;
    p
}

/// Struck modes: (frequency ratio, level, decay relative to the ring).
pub const MODES: [&[(f32, f32, f32)]; 7] = [
    &[(1.0, 1.0, 1.0), (3.99, 0.35, 0.25), (10.65, 0.12, 0.08)],                                       // marimba
    &[(1.0, 1.0, 1.0), (3.0, 0.5, 0.4), (6.1, 0.25, 0.2), (9.8, 0.1, 0.1)],                            // xylophone
    &[(1.0, 1.0, 1.0), (3.98, 0.3, 0.45), (9.92, 0.1, 0.2)],                                           // vibraphone
    &[(1.0, 1.0, 1.0), (2.76, 0.5, 0.7), (5.4, 0.3, 0.5), (8.93, 0.15, 0.35)],                         // glockenspiel
    &[(1.0, 1.0, 1.0), (2.0, 0.25, 0.5), (3.0, 0.12, 0.3), (4.0, 0.05, 0.2)],                          // celesta
    &[(0.5, 0.5, 1.4), (1.0, 1.0, 1.0), (1.19, 0.7, 0.8), (1.5, 0.6, 0.7), (2.0, 0.5, 0.5), (2.5, 0.35, 0.4), (3.0, 0.2, 0.3)], // tubular bells
    &[(1.0, 1.0, 1.0), (2.32, 0.5, 0.6), (4.25, 0.35, 0.5), (6.63, 0.25, 0.35), (9.38, 0.15, 0.25)],   // steel drum / generic bell
];

/// Every sound, in the order of [`PATCH_NAMES`]' indices.
pub static PATCHES: [Patch; COUNT] = [
    // 0..=4: strings, sections
    strings("Violins", 0.0, 6, [(450.0, 1.2, 0.6), (2800.0, 1.5, 0.35), (5500.0, 2.0, 0.12)], 76, 0.07, 0.16),
    strings("Violas", -2.0, 6, [(350.0, 1.2, 0.7), (1800.0, 1.4, 0.3), (4200.0, 2.0, 0.1)], 64, 0.08, 0.16),
    strings("Cellos", 0.0, 6, [(220.0, 1.2, 0.8), (1300.0, 1.3, 0.3), (3200.0, 2.0, 0.1)], 50, 0.16, 0.15),
    strings("Basses", -1.0, 6, [(110.0, 1.0, 0.8), (800.0, 1.2, 0.25), (2000.0, 2.0, 0.08)], 38, 0.3, 0.1),
    strings("Strings", 0.0, 6, [(400.0, 1.0, 0.6), (2400.0, 1.4, 0.3), (5000.0, 2.0, 0.1)], 62, 0.09, 0.15),
    // 5..=8: strings, solo
    strings("Violin", -8.0, 1, [(450.0, 1.5, 0.8), (2900.0, 2.0, 0.45), (5500.0, 2.5, 0.15)], 76, 0.07, 0.22),
    strings("Viola", -9.0, 1, [(350.0, 1.5, 0.8), (1900.0, 1.8, 0.4), (4200.0, 2.5, 0.12)], 64, 0.08, 0.21),
    strings("Cello", -7.0, 1, [(220.0, 1.5, 0.9), (1400.0, 1.6, 0.35), (3200.0, 2.5, 0.12)], 50, 0.16, 0.2),
    strings("Double bass", -8.0, 1, [(110.0, 1.2, 0.9), (850.0, 1.5, 0.3), (2000.0, 2.5, 0.1)], 38, 0.3, 0.14),
    // 9..=11: plucked
    timing(base("Pizzicato strings", Model::Pluck(0.45), -4.0, 26.0, 62), 0.002, 0.15, 0.9),
    timing(base("Harp", Model::Pluck(0.55), -6.0, 28.0, 60), 0.002, 1.2, 4.0),
    timing(base("Guitar", Model::Pluck(0.6), -8.0, 26.0, 55), 0.002, 0.3, 3.0),
    // 12..=21: woodwinds
    with_vibrato(wind("Piccolo", Model::Flute, -2.0, 22.0, [NONE; 3], 86), 0.12, 5.2),
    with_vibrato(wind("Flute", Model::Flute, -3.0, 24.0, [NONE; 3], 74), 0.14, 5.0),
    with_vibrato(wind("Oboe", Model::Reed(0.18), -4.0, 20.0, [(1100.0, 3.0, 0.9), (2900.0, 4.0, 0.5), (4400.0, 4.0, 0.2)], 69), 0.12, 5.4),
    with_vibrato(wind("Cor anglais", Model::Reed(0.2), -5.0, 20.0, [(950.0, 3.0, 0.9), (2300.0, 3.0, 0.4), (3800.0, 4.0, 0.15)], 62), 0.11, 5.2),
    tone(wind("Clarinet", Model::Reed(0.5), -5.0, 36.0, [(1500.0, 1.5, 0.3), (3000.0, 2.0, 0.15), NONE], 65), 1.6, 9.0),
    tone(wind("Bass clarinet", Model::Reed(0.5), -6.0, 30.0, [(700.0, 1.5, 0.3), (1800.0, 2.0, 0.15), NONE], 46), 1.6, 8.0),
    wind("Bassoon", Model::Reed(0.3), -6.0, 24.0, [(450.0, 2.5, 0.9), (1150.0, 3.0, 0.5), (2600.0, 3.0, 0.15)], 50),
    wind("Contrabassoon", Model::Reed(0.3), -7.0, 22.0, [(250.0, 2.0, 0.9), (700.0, 2.5, 0.4), NONE], 34),
    with_vibrato(wind("Saxophone", Model::Reed(0.0), -1.0, 28.0, [(600.0, 2.0, 0.8), (1700.0, 2.5, 0.5), (3200.0, 3.0, 0.2)], 63), 0.12, 5.0),
    with_vibrato(wind("Recorder", Model::Flute, -9.0, 10.0, [NONE; 3], 79), 0.04, 5.0),
    // 22..=27: brass
    tone(timing(wind("Horn", Model::Brass, 0.0, 34.0, [(400.0, 1.5, 0.5), (1200.0, 2.0, 0.25), NONE], 60), 0.04, 0.15, 2.0), 1.2, 7.0),
    tone(timing(wind("Trumpet", Model::Brass, 2.0, 36.0, [(1300.0, 1.5, 0.5), (2600.0, 2.0, 0.3), NONE], 72), 0.025, 0.1, 2.0), 1.6, 16.0),
    tone(timing(wind("Trombone", Model::Brass, 2.0, 38.0, [(550.0, 1.5, 0.5), (1400.0, 2.0, 0.3), NONE], 53), 0.035, 0.12, 2.0), 1.4, 14.0),
    tone(timing(wind("Tuba", Model::Brass, 1.0, 32.0, [(230.0, 1.5, 0.6), (600.0, 2.0, 0.3), NONE], 38), 0.045, 0.15, 2.0), 1.3, 9.0),
    tone(timing(wind("Muted trumpet", Model::Brass, -6.0, 24.0, [(1800.0, 4.0, 1.2), (4000.0, 4.0, 0.5), NONE], 72), 0.025, 0.1, 2.0), 3.0, 14.0),
    section(tone(timing(wind("Brass", Model::Brass, 3.0, 36.0, [(500.0, 1.5, 0.5), (1500.0, 2.0, 0.3), NONE], 60), 0.04, 0.15, 2.0), 1.4, 12.0), 3, 6.0),
    // 28..=31: voices
    section(with_vibrato(timing(wind("Choir", Model::Choir, 0.0, 30.0, [(700.0, 8.0, 1.0), (1150.0, 9.0, 0.5), (2800.0, 10.0, 0.25)], 62), 0.12, 0.25, 2.0), 0.15, 5.3), 4, 10.0),
    with_vibrato(timing(wind("Voice (female)", Model::Choir, -4.0, 28.0, [(800.0, 8.0, 1.0), (1150.0, 9.0, 0.5), (2900.0, 10.0, 0.3)], 72), 0.08, 0.2, 2.0), 0.3, 5.6),
    with_vibrato(timing(wind("Voice (male)", Model::Choir, -4.0, 28.0, [(650.0, 8.0, 1.0), (1080.0, 9.0, 0.5), (2650.0, 10.0, 0.3)], 55), 0.08, 0.2, 2.0), 0.28, 5.4),
    with_vibrato(timing(wind("Voice (oo)", Model::Choir, -6.0, 24.0, [(300.0, 6.0, 1.0), (870.0, 8.0, 0.3), (2250.0, 10.0, 0.1)], 62), 0.1, 0.25, 2.0), 0.15, 5.3),
    // 32..=35: keyboards
    timing(base("Piano", Model::Piano, 0.0, 40.0, 60), 0.001, 0.25, 12.0),
    timing(base("Harpsichord", Model::Pluck(0.95), -6.0, 4.0, 60), 0.001, 0.12, 6.0),
    timing(base("Organ", Model::Organ, -2.0, 8.0, 60), 0.012, 0.08, 2.0),
    timing(base("Accordion", Model::Reed(0.25), -6.0, 22.0, 60), 0.03, 0.08, 2.0),
    // 36..=42: tuned percussion
    timing(base("Celesta", Model::Mallet(4), -9.0, 20.0, 72), 0.001, 0.4, 2.0),
    timing(base("Glockenspiel", Model::Mallet(3), -4.0, 30.0, 84), 0.001, 1.0, 4.0),
    timing(base("Vibraphone", Model::Mallet(2), -5.0, 28.0, 65), 0.001, 0.6, 6.0),
    timing(base("Marimba", Model::Mallet(0), -4.0, 30.0, 60), 0.001, 0.3, 1.2),
    timing(base("Xylophone", Model::Mallet(1), -2.0, 30.0, 77), 0.001, 0.15, 0.7),
    timing(base("Tubular bells", Model::Mallet(5), -3.0, 30.0, 65), 0.001, 2.5, 8.0),
    timing(base("Steel drum", Model::Mallet(6), -5.0, 26.0, 65), 0.001, 0.4, 1.5),
    // 43..=46: percussion and the rest
    timing(base("Timpani", Model::Timpani, 0.0, 36.0, 45), 0.001, 0.6, 2.2),
    timing(base("Drum kit", Model::Kit, 0.0, 30.0, 38), 0.001, 0.1, 1.0),
    timing(tone(base("Synth pad", Model::Pad, -4.0, 24.0, 60), 1.5, 6.0), 0.15, 0.4, 2.0),
    timing(tone(base("Sine", Model::Pad, -6.0, 24.0, 60), 1.0, 1.0), 0.006, 0.15, 2.0),
];

pub const COUNT: usize = 47;
pub const KIT: usize = 44;
pub const SINE: usize = 46;

/// Bank 1: string sections instead of the General MIDI soloists.
pub const SECTION_BANK: u16 = 1;
/// Bank 128 (MSB 1, LSB 0): the drum kit, as General MIDI level 2 has it.
pub const DRUM_BANK: u16 = 128;

/// The sound a General MIDI program (0-based) and bank select.
pub fn patch_for(program: u8, bank: u16) -> usize {
    match (program, bank) {
        (71, 1) => return 17,
        (70, 1) => return 19,
        (53, 2) => return 30,
        _ => {}
    }
    if bank == DRUM_BANK {
        return KIT;
    }
    if bank == SECTION_BANK {
        match program {
            40 => return 0,
            41 => return 1,
            42 => return 2,
            43 => return 3,
            52..=54 => return 28,
            _ => {}
        }
    }
    match program {
        0..=5 => 32,
        6 => 33,
        7 => 33,
        8 => 36,
        9 => 37,
        10 => 36,
        11 => 38,
        12 => 39,
        13 => 40,
        14 => 41,
        15 => 37,
        16..=20 => 34,
        21..=23 => 35,
        24..=31 => 11,
        32..=39 => 11,
        40 => 5,
        41 => 6,
        42 => 7,
        43 => 8,
        44 | 48 | 49 | 50 | 51 => 4,
        45 => 9,
        46 => 10,
        47 => 43,
        52 => 28,
        53 => 29,
        54 => 31,
        55 => 45,
        56 => 23,
        57 => 24,
        58 => 25,
        59 => 26,
        60 => 22,
        61..=63 => 27,
        64..=67 => 20,
        68 => 14,
        69 => 15,
        70 => 18,
        71 => 16,
        72 => 12,
        73 => 13,
        74..=78 => 21,
        79 => SINE,
        114 => 42,
        118 | 119 => 43,
        _ => 45,
    }
}

/// What the built-in instruments play for a staff's instrument: the
/// General MIDI program and bank. `orchestral`: strings play as sections
/// (the score has a double bass or plural string names), else as soloists.
pub fn program_for(name: &str, drums: bool, orchestral: bool) -> (u8, u16) {
    let n = name.to_lowercase();
    if drums {
        return (0, DRUM_BANK);
    }
    // Whole words, with plural or Italian endings ("violins", "violoncelli"):
    // "lute" is not in "flute", nor "harp" in "harpsichord", nor
    // "contrabass" in "contrabassoon".
    let words: Vec<&str> = n.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
    let has = |w: &[&str]| {
        w.iter().any(|x| {
            if x.contains(' ') {
                return n.contains(x);
            }
            words.iter().any(|word| word.strip_prefix(x).is_some_and(|rest| ["", "s", "es", "i", "o", "e", "a"].contains(&rest)))
        })
    };
    let solo = n.contains("solo");
    let plural = has(&["violins", "violas", "cellos", "celli", "violoncelli", "basses", "contrabasses", "section"]);
    let strings_bank = if !solo && (plural || orchestral) { SECTION_BANK } else { 0 };
    // Most specific first.
    let table: &[(&[&str], u8, u16)] = &[
        (&["string ensemble", "strings"], 48, 0),
        (&["pizz"], 45, 0),
        (&["double bass", "contrabass", "string bass", "kontrabass"], 43, strings_bank),
        (&["bass guitar", "electric bass"], 33, 0),
        (&["violoncell", "cello", "celli"], 42, strings_bank),
        (&["viola"], 41, strings_bank),
        (&["violin", "fiddle"], 40, strings_bank),
        (&["harp"], 46, 0),
        (&["guitar", "lute", "mandolin", "banjo", "ukulele", "theorbo"], 24, 0),
        (&["piccolo"], 72, 0),
        (&["recorder"], 74, 0),
        (&["flute", "flauto"], 73, 0),
        (&["english horn", "cor anglais", "corno inglese"], 69, 0),
        (&["oboe"], 68, 0),
        (&["bass clarinet"], 71, 1),
        (&["clarinet"], 71, 0),
        (&["contrabassoon", "contrafagotto"], 70, 1),
        (&["bassoon", "fagott"], 70, 0),
        (&["sax", "saxophone"], 65, 0),
        (&["horn", "corno"], 60, 0),
        (&["trumpet", "cornet", "flugelhorn", "tromba"], 56, 0),
        (&["trombone"], 57, 0),
        (&["tuba", "euphonium", "cimbasso"], 58, 0),
        (&["choir", "chorus", "satb", "coro"], 52, SECTION_BANK),
        (&["soprano", "alto", "mezzo", "contralto", "treble", "voice", "chant", "cantor"], 53, 0),
        (&["tenor", "baritone", "bass voice", "basso"], 53, 2),
        (&["harpsichord", "cembalo"], 6, 0),
        (&["organ"], 19, 0),
        (&["accordion", "bandoneon"], 21, 0),
        (&["celesta"], 8, 0),
        (&["glockenspiel", "glock"], 9, 0),
        (&["vibraphone", "vibes"], 11, 0),
        (&["marimba"], 12, 0),
        (&["xylophone"], 13, 0),
        (&["tubular bell", "chimes", "bells"], 14, 0),
        (&["steel"], 114, 0),
        (&["timpani", "kettle"], 47, 0),
        (&["piano", "keyboard", "pianoforte", "fortepiano"], 0, 0),
        (&["synth", "synthesizer", "synthesiser"], 88, 0),
    ];
    for (words, program, bank) in table {
        if has(words) {
            return (*program, *bank);
        }
    }
    (0, 0) // unknown: the piano
}

/// Each sound's calibration: the gain that brings its measured loudness at
/// mf to `level_db`. Measured once, at a fixed sample rate.
pub fn trims() -> &'static [f32; COUNT] {
    static TRIMS: OnceLock<[f32; COUNT]> = OnceLock::new();
    TRIMS.get_or_init(|| {
        let mut t = [1.0; COUNT];
        for (i, p) in PATCHES.iter().enumerate() {
            let measured = crate::voice::measure(i, 48000.0);
            let target = REFERENCE_RMS * crate::dsp::db(p.level_db);
            t[i] = if measured > 1e-9 { target / measured } else { 1.0 };
        }
        t
    })
}

/// The loudness of a violin section at mf (RMS, weighted as `measure` does).
pub const REFERENCE_RMS: f32 = 0.012;

use crate::voice::{Exciter, Spectrum};

const fn spec(tilt_pp: f32, tilt_ff: f32, peaks: [(f32, f32, f32); 3], even: (f32, f32), rise: f32, noise_db: f32, exciter: Exciter, physical: f32) -> Option<Spectrum> {
    Some(Spectrum { tilt_pp, tilt_ff, peaks, even_db: even.0, even_below: even.1, rise, noise_db, exciter, physical, body: None })
}

/// A bowed string on a measured body ([`crate::body`]): its noise part
/// through `peaks` at `noise_db`.
const fn bowed(body: u8, peaks: [(f32, f32, f32); 3], noise_db: f32) -> Option<Spectrum> {
    Some(Spectrum { tilt_pp: 0.0, tilt_ff: 0.0, peaks, even_db: 0.0, even_below: 0.0, rise: 0.15, noise_db, exciter: Exciter::Bow, physical: 0.1, body: Some(body) })
}

const NO_EVEN: (f32, f32) = (0.0, 0.0);
const P0: (f32, f32, f32) = (1000.0, 1.0, 0.0);

/// The spectral envelopes of the strings, woodwinds and brass (the
/// hybrid sounds; others have none). Peaks after the published
/// resonance regions of each instrument (body and bridge resonances of
/// the strings, the woodwinds' formant regions, the brass bells).
static SPECTRA: [Option<Spectrum>; COUNT] = {
    let mut s: [Option<Spectrum>; COUNT] = [None; COUNT];
    let violin = [(280.0, 0.4, 4.0), (480.0, 0.5, 3.0), (2800.0, 0.6, 5.0)];
    let viola = [(230.0, 0.4, 4.0), (400.0, 0.5, 3.0), (2200.0, 0.6, 4.0)];
    let cello = [(100.0, 0.4, 3.0), (220.0, 0.5, 4.0), (1800.0, 0.6, 3.0)];
    let bass = [(60.0, 0.4, 3.0), (130.0, 0.5, 3.0), (900.0, 0.6, 2.0)];
    use crate::body::{BASS, CELLO, VIOLA, VIOLIN};
    // Noise levels matched to the recordings' bow noise between the
    // harmonics (about -37 dB on the violin, -36 cello, -40 bass).
    s[0] = bowed(VIOLIN, violin, -24.0);
    s[1] = bowed(VIOLA, viola, -23.0);
    s[2] = bowed(CELLO, cello, -21.0);
    s[3] = bowed(BASS, bass, -22.0);
    s[4] = bowed(VIOLA, [(250.0, 0.6, 3.0), (480.0, 0.6, 2.0), (2500.0, 0.7, 4.0)], -23.0);
    s[5] = bowed(VIOLIN, violin, -24.0);
    s[6] = bowed(VIOLA, viola, -23.0);
    s[7] = bowed(CELLO, cello, -21.0);
    s[8] = bowed(BASS, bass, -22.0);
    s[12] = spec(-18.0, -9.0, [(1500.0, 1.0, 1.0), P0, P0], NO_EVEN, 0.3, -24.0, Exciter::Jet, 0.2);
    s[13] = spec(-16.0, -8.0, [(1500.0, 1.0, 2.0), P0, P0], NO_EVEN, 0.3, -22.0, Exciter::Jet, 0.2);
    s[14] = spec(-5.0, -2.0, [(1100.0, 0.35, 10.0), (2900.0, 0.3, 6.0), (4400.0, 0.3, 2.0)], NO_EVEN, 0.2, -32.0, Exciter::Reed(0.18), 0.3);
    s[15] = spec(-5.5, -2.5, [(950.0, 0.35, 10.0), (2300.0, 0.3, 5.0), (3800.0, 0.3, 1.5)], NO_EVEN, 0.2, -32.0, Exciter::Reed(0.2), 0.3);
    s[16] = spec(-10.0, -5.0, [(1500.0, 0.6, 2.0), P0, P0], (-18.0, 1800.0), 0.2, -34.0, Exciter::Reed(0.5), 0.3);
    s[17] = spec(-10.0, -5.0, [(700.0, 0.6, 2.0), P0, P0], (-15.0, 900.0), 0.2, -34.0, Exciter::Reed(0.5), 0.3);
    s[18] = spec(-7.0, -3.0, [(480.0, 0.4, 9.0), (1150.0, 0.35, 5.0), (2600.0, 0.4, 1.0)], NO_EVEN, 0.2, -32.0, Exciter::Reed(0.3), 0.3);
    s[19] = spec(-7.5, -3.5, [(250.0, 0.4, 8.0), (700.0, 0.4, 4.0), P0], NO_EVEN, 0.2, -32.0, Exciter::Reed(0.3), 0.3);
    s[20] = spec(-6.0, -2.5, [(600.0, 0.5, 6.0), (1700.0, 0.5, 6.0), (3200.0, 0.4, 2.0)], NO_EVEN, 0.3, -30.0, Exciter::Reed(0.0), 0.3);
    s[21] = spec(-18.0, -12.0, [(2000.0, 1.0, 1.0), P0, P0], NO_EVEN, 0.2, -24.0, Exciter::Jet, 0.2);
    s[22] = spec(-14.0, -3.5, [(380.0, 0.5, 6.0), (1200.0, 0.5, 2.0), P0], NO_EVEN, 0.8, -40.0, Exciter::Lip, 0.15);
    s[23] = spec(-12.0, -1.5, [(1300.0, 0.5, 6.0), (2600.0, 0.5, 3.0), P0], NO_EVEN, 0.9, -40.0, Exciter::Lip, 0.15);
    s[24] = spec(-13.0, -2.0, [(550.0, 0.5, 6.0), (1400.0, 0.5, 3.0), P0], NO_EVEN, 0.9, -40.0, Exciter::Lip, 0.15);
    s[25] = spec(-14.0, -4.0, [(230.0, 0.5, 6.0), (600.0, 0.5, 2.0), P0], NO_EVEN, 0.8, -40.0, Exciter::Lip, 0.15);
    s[26] = spec(-8.0, -3.0, [(1800.0, 0.3, 12.0), (4000.0, 0.3, 5.0), (300.0, 0.6, -12.0)], NO_EVEN, 0.6, -36.0, Exciter::Lip, 0.3);
    s[27] = spec(-13.0, -2.0, [(500.0, 0.5, 5.0), (1500.0, 0.5, 3.0), P0], NO_EVEN, 0.8, -40.0, Exciter::Lip, 0.15);
    s
};

/// A sound's spectral envelope, if it is one of the hybrids.
pub fn spectrum(patch: usize) -> Option<&'static Spectrum> {
    SPECTRA.get(patch).and_then(|s| s.as_ref())
}
