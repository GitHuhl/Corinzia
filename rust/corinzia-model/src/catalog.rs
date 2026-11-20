//! The instrument catalogue: what an instrument *is*, in one place.
//!
//! Each [`Kind`] gathers what the program needs to know about an
//! instrument: its family (for brackets), its name, abbreviation, clef and
//! transposition when it is added to a score, the General MIDI program the
//! built-in instruments play it with, whether strings may play as a
//! section, its players' timing feel, and the drum map or tablature tuning
//! it starts with.
//!
//! An [`Instrument`](crate::Instrument) remembers its kind (saved with the
//! project), so renaming a staff changes none of this. The kind is found
//! from a name once: when the instrument is added, imported, or an older
//! project without kinds is opened ([`infer`]).

use crate::model::{DrumSound, Interval, Notehead, Pitch, BASS_TUNING, GUITAR_TUNING};

/// The orchestral families: staves of one family next to each other share
/// a bracket. Keyboards, harps and guitars belong to none.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    Woodwinds,
    Brass,
    Percussion,
    Strings,
    Voices,
}

impl Family {
    pub fn name(self) -> &'static str {
        match self {
            Family::Woodwinds => "woodwinds",
            Family::Brass => "brass",
            Family::Percussion => "percussion",
            Family::Strings => "strings",
            Family::Voices => "voices",
        }
    }
}

/// How a kind's players place notes against the beat (the numbers are in
/// `corinzia_perform::timing`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Feel {
    Strings,
    Woodwinds,
    Brass,
    Voices,
    Keyboard,
    Plucked,
    Percussion,
    /// A drum kit: placed per sound.
    Kit,
}

/// The drum map a kind starts with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Drums {
    None,
    Kit,
    Snare,
    BassDrum,
    Cymbals,
}

/// The tablature a fretted kind starts with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tuning {
    None,
    Guitar,
    Bass,
}

#[derive(Debug, PartialEq)]
pub struct Kind {
    /// Saved with the project: never change one.
    pub id: &'static str,
    /// The name and abbreviation an instrument of this kind gets when added.
    pub name: &'static str,
    pub abbr: &'static str,
    /// Names typed (lower case) that mean exactly this kind: "fl", "horn in f".
    pub aliases: &'static [&'static str],
    /// Words (or phrases, with a space) that pick this kind out of any
    /// name, as whole words with plural or Italian endings ("violins",
    /// "violoncelli"): "lute" is not in "flute", nor "harp" in
    /// "harpsichord". The kinds are tried in order, the most specific first.
    pub words: &'static [&'static str],
    pub family: Option<Family>,
    /// "treble", "bass", "alto", "treble_8", "bass_8", "percussion";
    /// "treble+bass" for a grand staff.
    pub clef: &'static str,
    /// Written minus sounding.
    pub transposition: Interval,
    /// The General MIDI program and bank of the built-in instruments (bank
    /// 1: their section or alternative sounds, 2: lower voices).
    pub program: u8,
    pub bank: u16,
    /// Bowed strings: they play as a section in an orchestral score (or
    /// when the instrument is marked a section).
    pub section_strings: bool,
    pub feel: Feel,
    pub drums: Drums,
    pub tuning: Tuning,
}

// One row of the table: the fields every kind sets.
#[allow(clippy::too_many_arguments)]
const fn k(id: &'static str, name: &'static str, abbr: &'static str, aliases: &'static [&'static str], words: &'static [&'static str], family: Option<Family>, clef: &'static str, program: u8, feel: Feel) -> Kind {
    Kind { id, name, abbr, aliases, words, family, clef, transposition: Interval::new(0, 0), program, bank: 0, section_strings: false, feel, drums: Drums::None, tuning: Tuning::None }
}

use Family::*;
const WW: Option<Family> = Some(Woodwinds);
const BR: Option<Family> = Some(Brass);
const PE: Option<Family> = Some(Percussion);
const ST: Option<Family> = Some(Strings);
const VO: Option<Family> = Some(Voices);

const fn transposing(kind: Kind, steps: i32, semitones: i32) -> Kind {
    Kind { transposition: Interval::new(steps, semitones), ..kind }
}
const fn bank(kind: Kind, bank: u16) -> Kind {
    Kind { bank, ..kind }
}
const fn strings(kind: Kind) -> Kind {
    Kind { section_strings: true, ..kind }
}
const fn drums(kind: Kind, drums: Drums) -> Kind {
    Kind { drums, ..kind }
}
const fn tuned(kind: Kind, tuning: Tuning) -> Kind {
    Kind { tuning, ..kind }
}

/// Every kind, the most specific first (a name is matched in this order).
pub static KINDS: &[Kind] = &[
    // Strings.
    k("string-ensemble", "Strings", "Str.", &[], &["string ensemble", "strings"], ST, "treble", 48, Feel::Strings),
    k("pizzicato-strings", "Pizzicato strings", "Pizz.", &[], &["pizz"], ST, "treble", 45, Feel::Strings),
    strings(k("double-bass", "Double bass", "D.B.", &["double bass", "contrabass", "db", "cb"], &["double bass", "contrabass", "string bass", "kontrabass", "basses"], ST, "bass_8", 43, Feel::Strings)),
    tuned(k("bass-guitar", "Bass guitar", "B. Gtr.", &["bass guitar", "electric bass", "e. bass"], &["bass guitar", "electric bass"], None, "bass_8", 33, Feel::Plucked), Tuning::Bass),
    strings(k("cello", "Cello", "Vc.", &["cello", "violoncello", "vc"], &["violoncell", "cello", "celli"], ST, "bass", 42, Feel::Strings)),
    strings(k("viola", "Viola", "Vla.", &["viola", "vla"], &["viola"], ST, "alto", 41, Feel::Strings)),
    strings(k("violin", "Violin", "Vln.", &["violin", "vln", "violin i", "violin ii"], &["violin", "fiddle", "violon", "geige"], ST, "treble", 40, Feel::Strings)),
    // Plucked.
    k("harp", "Harp", "Hp.", &["harp"], &["harp", "arpa", "harfe"], None, "treble+bass", 46, Feel::Plucked),
    tuned(k("electric-guitar", "Electric guitar", "E. Gtr.", &["electric guitar", "e. gtr", "e-guitar"], &["electric guitar"], None, "treble_8", 24, Feel::Plucked), Tuning::Guitar),
    tuned(k("guitar-tab", "Guitar (tab)", "Gtr.", &["guitar tab", "guitar with tab", "guitar (tab)"], &["guitar tab", "guitar (tab)", "guitar with tab", "tab guitar"], None, "treble_8", 24, Feel::Plucked), Tuning::Guitar),
    k("guitar", "Guitar", "Gtr.", &["guitar", "gtr"], &["guitar", "lute", "mandolin", "banjo", "ukulele", "theorbo"], None, "treble_8", 24, Feel::Plucked),
    // Woodwinds.
    k("piccolo", "Piccolo", "Picc.", &["piccolo"], &["piccolo", "ottavino"], WW, "treble", 72, Feel::Woodwinds),
    k("recorder", "Recorder", "Rec.", &[], &["recorder", "blockflöte"], WW, "treble", 74, Feel::Woodwinds),
    k("flute", "Flute", "Fl.", &["flute", "fl"], &["flute", "flauto", "flöte", "flûte"], WW, "treble", 73, Feel::Woodwinds),
    transposing(k("cor-anglais", "Cor anglais", "C. a.", &["cor anglais", "english horn"], &["english horn", "cor anglais", "corno inglese", "englischhorn"], WW, "treble", 69, Feel::Woodwinds), 4, 7),
    k("oboe", "Oboe", "Ob.", &["oboe", "ob"], &["oboe", "hautbois"], WW, "treble", 68, Feel::Woodwinds),
    bank(transposing(k("bass-clarinet", "Bass clarinet", "B. Cl.", &["bass clarinet"], &["bass clarinet", "clarinetto basso", "bassklarinette"], WW, "bass", 71, Feel::Woodwinds), 1, 2), 1),
    transposing(k("clarinet-a", "Clarinet in A", "Cl.", &["clarinet in a", "a clarinet"], &["clarinet in a"], WW, "treble", 71, Feel::Woodwinds), 2, 3),
    transposing(k("clarinet", "Clarinet in B♭", "Cl.", &["clarinet", "cl", "clarinet in bb", "bb clarinet"], &["clarinet", "clarinetto", "klarinette", "clarinette"], WW, "treble", 71, Feel::Woodwinds), 1, 2),
    bank(k("contrabassoon", "Contrabassoon", "Cbsn.", &["contrabassoon"], &["contrabassoon", "contrafagotto", "kontrafagott"], WW, "bass_8", 70, Feel::Woodwinds), 1),
    k("bassoon", "Bassoon", "Bsn.", &["bassoon", "bsn"], &["bassoon", "fagott", "basson"], WW, "bass", 70, Feel::Woodwinds),
    transposing(k("soprano-saxophone", "Soprano saxophone", "S. Sax.", &["soprano saxophone", "soprano sax"], &["soprano sax"], WW, "treble", 65, Feel::Woodwinds), 1, 2),
    transposing(k("tenor-saxophone", "Tenor saxophone", "T. Sax.", &["tenor saxophone", "tenor sax"], &["tenor sax"], WW, "treble", 65, Feel::Woodwinds), 8, 14),
    transposing(k("baritone-saxophone", "Baritone saxophone", "Bar. Sax.", &["baritone saxophone", "baritone sax", "bari sax"], &["baritone sax", "bari sax"], WW, "bass", 65, Feel::Woodwinds), 12, 21),
    transposing(k("alto-saxophone", "Alto saxophone", "A. Sax.", &["saxophone", "alto saxophone", "alto sax", "sax"], &["sax", "saxophone"], WW, "treble", 65, Feel::Woodwinds), 5, 9),
    // Brass.
    transposing(k("horn", "Horn in F", "Hn.", &["horn", "french horn", "hn", "horn in f"], &["horn", "corno", "horn in f"], BR, "treble", 60, Feel::Brass), 4, 7),
    k("trumpet-c", "Trumpet in C", "Tpt.", &["trumpet in c", "c trumpet"], &["trumpet in c"], BR, "treble", 56, Feel::Brass),
    transposing(k("trumpet", "Trumpet in B♭", "Tpt.", &["trumpet", "tpt", "trumpet in bb", "bb trumpet"], &["trumpet", "cornet", "flugelhorn", "flügelhorn", "tromba", "trompete", "trompette"], BR, "treble", 56, Feel::Brass), 1, 2),
    k("bass-trombone", "Bass trombone", "B. Tbn.", &["bass trombone"], &["bass trombone"], BR, "bass", 57, Feel::Brass),
    k("trombone", "Trombone", "Tbn.", &["trombone", "tbn"], &["trombone", "posaune"], BR, "bass", 57, Feel::Brass),
    k("tuba", "Tuba", "Tba.", &["tuba"], &["tuba", "euphonium", "cimbasso"], BR, "bass", 58, Feel::Brass),
    // Percussion.
    drums(k("drum-set", "Drum set", "Dr.", &["drum set", "drums", "drum kit", "kit"], &["drum set", "drum kit", "drumset", "drums", "kit"], PE, "percussion", 0, Feel::Kit), Drums::Kit),
    drums(k("snare-drum", "Snare drum", "S.D.", &["snare drum", "snare"], &["snare", "caisse claire"], PE, "percussion", 0, Feel::Percussion), Drums::Snare),
    drums(k("bass-drum", "Bass drum", "B.D.", &["bass drum"], &["bass drum", "gran cassa", "grosse caisse"], PE, "percussion", 0, Feel::Percussion), Drums::BassDrum),
    drums(k("cymbals", "Cymbals", "Cym.", &["cymbals", "cymbal"], &["cymbal", "piatti"], PE, "percussion", 0, Feel::Percussion), Drums::Cymbals),
    k("timpani", "Timpani", "Timp.", &["timpani", "timp"], &["timpani", "timpano", "kettle", "pauken", "pauke"], PE, "bass", 47, Feel::Percussion),
    k("glockenspiel", "Glockenspiel", "Glk.", &[], &["glockenspiel", "glock"], PE, "treble", 9, Feel::Percussion),
    k("vibraphone", "Vibraphone", "Vib.", &[], &["vibraphone", "vibes", "vibrafono"], PE, "treble", 11, Feel::Percussion),
    k("marimba", "Marimba", "Mar.", &[], &["marimba"], PE, "treble+bass", 12, Feel::Percussion),
    k("xylophone", "Xylophone", "Xyl.", &[], &["xylophone", "xylo"], PE, "treble", 13, Feel::Percussion),
    k("tubular-bells", "Tubular bells", "T. B.", &[], &["tubular bell", "chimes", "bells"], PE, "treble", 14, Feel::Percussion),
    k("steel-drum", "Steel drum", "St. Dr.", &[], &["steel"], PE, "treble", 114, Feel::Percussion),
    k("percussion", "Percussion", "Perc.", &["percussion", "perc"], &["percussion", "perc", "drum", "triangle", "tam-tam", "gong", "tambourine", "woodblock"], PE, "percussion", 0, Feel::Percussion),
    // Voices.
    bank(k("choir", "Choir", "Ch.", &[], &["choir", "chorus", "satb", "coro", "chor"], VO, "treble", 52, Feel::Voices), 1),
    k("soprano", "Soprano", "S.", &["soprano", "s"], &["soprano"], VO, "treble", 53, Feel::Voices),
    k("alto", "Alto", "A.", &["alto", "a", "mezzo"], &["alto", "mezzo", "contralto"], VO, "treble", 53, Feel::Voices),
    bank(k("tenor", "Tenor", "T.", &["tenor", "t"], &["tenor"], VO, "treble_8", 53, Feel::Voices), 2),
    bank(k("baritone", "Baritone", "Bar.", &["baritone"], &["baritone"], VO, "bass", 53, Feel::Voices), 2),
    bank(k("bass-voice", "Bass", "B.", &["bass", "b"], &["bass voice", "basso", "bass"], VO, "bass", 53, Feel::Voices), 2),
    k("voice", "Voice", "V.", &[], &["voice", "treble", "chant", "cantor"], VO, "treble", 53, Feel::Voices),
    // Keyboards.
    k("harpsichord", "Harpsichord", "Hpd.", &[], &["harpsichord", "cembalo", "clavecin"], None, "treble+bass", 6, Feel::Keyboard),
    k("organ", "Organ", "Org.", &["organ", "org"], &["organ", "orgel", "organo"], None, "treble+bass", 19, Feel::Keyboard),
    k("accordion", "Accordion", "Acc.", &[], &["accordion", "bandoneon", "akkordeon"], None, "treble+bass", 21, Feel::Keyboard),
    k("celesta", "Celesta", "Cel.", &["celesta"], &["celesta", "celeste"], None, "treble+bass", 8, Feel::Keyboard),
    k("piano", "Piano", "Pno.", &["piano", "pno"], &["piano", "keyboard", "pianoforte", "fortepiano", "klavier"], None, "treble+bass", 0, Feel::Keyboard),
    k("synth", "Synthesizer", "Syn.", &[], &["synth", "synthesizer", "synthesiser"], None, "treble+bass", 88, Feel::Keyboard),
];

/// The kind with this id (as saved).
pub fn kind(id: &str) -> Option<&'static Kind> {
    KINDS.iter().find(|k| k.id == id)
}

/// A name without its player number ("Violin 2", "Horn 3").
fn base(name: &str) -> &str {
    let name = name.trim();
    match name.rsplit_once(' ') {
        Some((b, n)) if n.chars().all(|c| c.is_ascii_digit()) && !b.trim().is_empty() => b.trim(),
        _ => name,
    }
}

/// The kind a name means exactly: one of its aliases ("fl", "Horn in F",
/// "Violin 2"). Used when an instrument is typed, so its clef and
/// transposition are the kind's.
pub fn lookup(name: &str) -> Option<&'static Kind> {
    let key = base(name).to_lowercase();
    KINDS.iter().find(|k| k.aliases.contains(&key.as_str()))
}

/// The kind a name most likely means: an exact alias, else the first kind
/// (the most specific) with one of its words in the name. None for a name
/// the catalogue does not know.
pub fn infer(name: &str) -> Option<&'static Kind> {
    if let Some(k) = lookup(name) {
        return Some(k);
    }
    let n = name.to_lowercase();
    let words: Vec<&str> = n.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
    let has = |w: &str| {
        if w.contains(' ') || w.contains('-') {
            return n.contains(w);
        }
        words.iter().any(|word| word.strip_prefix(w).is_some_and(|rest| ["", "s", "es", "i", "o", "e", "a"].contains(&rest)))
    };
    KINDS.iter().find(|k| k.words.iter().any(|w| has(w)))
}

/// Whether a name says one player or a section: "Violin solo" (one),
/// "Violins", "Celli", "Strings section" (a section); None: either, as the
/// score suggests.
pub fn section_of(name: &str) -> Option<bool> {
    let n = name.to_lowercase();
    if n.contains("solo") {
        return Some(false);
    }
    let words: Vec<&str> = n.split(|c: char| !c.is_alphanumeric()).collect();
    let plural = ["violins", "violas", "cellos", "celli", "violoncelli", "basses", "contrabasses", "section"];
    words.iter().any(|w| plural.contains(w)).then_some(true)
}

/// The clef for an instrument the catalogue does not know, from its name.
pub fn guess_clef(name: &str) -> &'static str {
    let n = name.to_lowercase();
    if ["bass", "cello", "tuba", "bassoon", "trombone", "baritone", "contra", "timpani"].iter().any(|w| n.contains(w)) {
        "bass"
    } else if n.contains("viola") {
        "alto"
    } else if ["keyboard", "synth", "piano", "harpsichord", "marimba"].iter().any(|w| n.contains(w)) {
        "treble+bass"
    } else if ["drum", "perc", "kit"].iter().any(|w| n.contains(w)) {
        "percussion"
    } else {
        "treble"
    }
}

impl Kind {
    /// The drum sounds an instrument of this kind starts with: a drum set's
    /// (the usual positions, General MIDI keys), a single drum's, or none.
    pub fn default_drums(&self) -> Vec<DrumSound> {
        let p = |step: i8, octave: i8| Pitch { step, alter: 0, octave };
        let s = |name: &str, position: Pitch, key: u8, head: Notehead| DrumSound { name: name.into(), position, key, head };
        match self.drums {
            Drums::Kit => vec![
                s("Bass drum", p(3, 4), 36, Notehead::Normal),
                s("Floor tom", p(5, 4), 41, Notehead::Normal),
                s("Snare drum", p(0, 5), 38, Notehead::Normal),
                s("Low tom", p(1, 5), 45, Notehead::Normal),
                s("High tom", p(2, 5), 48, Notehead::Normal),
                s("Ride cymbal", p(3, 5), 51, Notehead::X),
                s("Hi-hat", p(4, 5), 42, Notehead::X),
                s("Open hi-hat", p(5, 5), 46, Notehead::CircleX),
                s("Crash cymbal", p(6, 5), 49, Notehead::X),
                s("Hi-hat pedal", p(1, 4), 44, Notehead::X),
                s("Side stick", p(4, 4), 37, Notehead::X),
                s("Cowbell", p(2, 4), 56, Notehead::Triangle),
            ],
            Drums::Snare => vec![s("Snare drum", p(0, 5), 38, Notehead::Normal)],
            Drums::BassDrum => vec![s("Bass drum", p(0, 5), 36, Notehead::Normal)],
            Drums::Cymbals => vec![s("Crash cymbal", p(0, 5), 49, Notehead::X)],
            Drums::None => vec![],
        }
    }

    /// The tablature tuning an instrument of this kind starts with.
    pub fn default_tuning(&self) -> Vec<u8> {
        match self.tuning {
            Tuning::Guitar => GUITAR_TUNING.to_vec(),
            Tuning::Bass => BASS_TUNING.to_vec(),
            Tuning::None => vec![],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_found() {
        for (i, a) in KINDS.iter().enumerate() {
            assert!(KINDS[i + 1..].iter().all(|b| b.id != a.id), "{} twice", a.id);
            assert_eq!(kind(a.id).map(|k| k.id), Some(a.id));
        }
    }

    #[test]
    fn names_find_their_kinds() {
        let id = |n: &str| infer(n).map(|k| k.id);
        for (name, kind) in [
            ("Flute", "flute"),
            ("Flauto 1", "flute"),
            ("Violin I", "violin"),
            ("Violins", "violin"),
            ("Violoncello", "cello"),
            ("Celli", "cello"),
            ("Contrabass", "double-bass"),
            ("Basses", "double-bass"),
            ("Contrabassoon", "contrabassoon"),
            ("Bass Clarinet in Bb", "bass-clarinet"),
            ("Clarinet in A", "clarinet-a"),
            ("English Horn", "cor-anglais"),
            ("Horn in F", "horn"),
            ("Corno 2", "horn"),
            ("Flugelhorn", "trumpet"),
            ("Bass Trombone", "bass-trombone"),
            ("Bass Drum", "bass-drum"),
            ("Drum Set", "drum-set"),
            ("Bass", "bass-voice"),
            ("Bass guitar", "bass-guitar"),
            ("Alto Saxophone", "alto-saxophone"),
            ("Harpsichord", "harpsichord"),
            ("Harp", "harp"),
            ("Electric Guitar", "electric-guitar"),
            ("Pauken", "timpani"),
            ("Solo Violin", "violin"),
        ] {
            assert_eq!(id(name), Some(kind), "{name}");
        }
        assert_eq!(id("Theremin"), None);
        assert_eq!(lookup("Clarinet in Eb"), None, "no transposition borrowed from a near name");
        assert_eq!((section_of("Violins"), section_of("Solo violin"), section_of("Violin I")), (Some(true), Some(false), None));
    }
}
