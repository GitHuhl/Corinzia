//! Byzantine chant in Chrysanthine ("New Method") notation: the model of a
//! chant, its modes and scales, and the melody its neumes spell.
//!
//! A [`Chant`] is a mode signature and a line of [`Element`]s: neumes with
//! their signs and syllables, martyriae and breaks. [`Chant::sing`] reads
//! it as a singer does: each neume moves from the note before, fthoras
//! change the genus, time signs set the beats. The result has each note's
//! pitch in moria, so it can be engraved, checked and played with the
//! intervals of its genus.

pub mod neume;
pub mod scale;

pub use neume::*;
pub use scale::*;

use corinzia_score::Frac;

/// The eight modes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Echos {
    First,
    Second,
    Third,
    Fourth,
    PlagalFirst,
    PlagalSecond,
    /// The grave mode (varys).
    Varys,
    PlagalFourth,
}

impl Echos {
    /// The base note and genus a chant in this mode usually starts from.
    /// The fourth mode is taken in its heirmologic form, from Pa; the third
    /// and grave modes are written here in the diatonic genus until the
    /// enharmonic genus is supported.
    pub fn default_base(self) -> (Note, Genus) {
        use Degree::*;
        match self {
            Echos::First | Echos::PlagalFirst | Echos::Fourth => (Note::new(Pa, 0), Genus::Diatonic),
            Echos::Second => (Note::new(Di, 0), Genus::SoftChromatic),
            Echos::PlagalSecond => (Note::new(Pa, 0), Genus::HardChromatic),
            Echos::Third | Echos::Varys => (Note::new(Ga, 0), Genus::Diatonic),
            Echos::PlagalFourth => (Note::new(Ni, 0), Genus::Diatonic),
        }
    }

    /// The Greek name, as written over a chant: "Ἦχος α΄", "Ἦχος πλ. β΄",
    /// "Ἦχος βαρύς". Corinzia shows these by default.
    pub fn greek(self) -> &'static str {
        match self {
            Echos::First => "Ἦχος α΄",
            Echos::Second => "Ἦχος β΄",
            Echos::Third => "Ἦχος γ΄",
            Echos::Fourth => "Ἦχος δ΄",
            Echos::PlagalFirst => "Ἦχος πλ. α΄",
            Echos::PlagalSecond => "Ἦχος πλ. β΄",
            Echos::Varys => "Ἦχος βαρύς",
            Echos::PlagalFourth => "Ἦχος πλ. δ΄",
        }
    }

    /// The English name.
    pub fn name(self) -> &'static str {
        match self {
            Echos::First => "First mode",
            Echos::Second => "Second mode",
            Echos::Third => "Third mode",
            Echos::Fourth => "Fourth mode",
            Echos::PlagalFirst => "Plagal of the first",
            Echos::PlagalSecond => "Plagal of the second",
            Echos::Varys => "Grave mode",
            Echos::PlagalFourth => "Plagal of the fourth",
        }
    }
}

/// The mode signature at the head of a chant: its mode, and the note and
/// genus the melody starts from.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ModeKey {
    pub echos: Echos,
    pub base: Note,
    pub genus: Genus,
}

impl ModeKey {
    pub fn new(echos: Echos) -> ModeKey {
        let (base, genus) = echos.default_base();
        ModeKey { echos, base, genus }
    }
}

/// One thing on the line of a chant.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Element {
    Neume(Neume),
    /// A martyria: the note the melody stands on here.
    Martyria(Degree),
    /// A line break chosen by hand.
    Break,
}

/// A chant: a title, a mode signature, a tempo and a line of elements.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Chant {
    pub title: String,
    pub mode: ModeKey,
    /// Beats per minute.
    pub tempo: u32,
    /// Sing the ison under the melody: the mode's base note, held, an
    /// octave down.
    pub ison: bool,
    pub elements: Vec<Element>,
}

/// A note as sung: when, how long (in beats), which note and its pitch.
#[derive(Clone, Debug, PartialEq)]
pub struct Sung {
    pub start: Frac,
    pub length: Frac,
    pub note: Note,
    pub genus: Genus,
    /// Moria above the middle Ni.
    pub moria: i32,
    /// The element that sings it.
    pub element: usize,
    /// The syllable started on this note, if any.
    pub lyric: Option<String>,
}

/// Something in the chant that does not add up.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Problem {
    /// A martyria names a note other than the one the melody is on.
    MartyriaDisagrees { element: usize, written: Degree, sung: Degree },
    /// A gorgon with fewer notes before it than it needs.
    GorgonTooEarly { element: usize },
}

impl Chant {
    pub fn new(title: &str, echos: Echos) -> Chant {
        Chant { title: title.into(), mode: ModeKey::new(echos), tempo: 100, ison: true, elements: vec![] }
    }

    pub fn push(&mut self, n: Neume) -> &mut Chant {
        self.elements.push(Element::Neume(n));
        self
    }

    /// The ison's note and pitch (moria above the middle Ni): the mode's
    /// base, an octave down, in the mode's genus.
    pub fn ison_note(&self) -> (Note, i32) {
        let n = self.mode.base.step(-7);
        (n, self.mode.genus.moria(n))
    }

    /// The melody, note by note, and anything that does not add up.
    pub fn sing(&self) -> (Vec<Sung>, Vec<Problem>) {
        let mut out: Vec<Sung> = vec![];
        let mut problems = vec![];
        let mut note = self.mode.base;
        let mut genus = self.mode.genus;
        for (i, el) in self.elements.iter().enumerate() {
            let n = match el {
                Element::Neume(n) => n,
                Element::Martyria(d) => {
                    if *d != note.degree {
                        problems.push(Problem::MartyriaDisagrees { element: i, written: *d, sung: note.degree });
                    }
                    continue;
                }
                Element::Break => continue,
            };
            let Some(q) = n.quantity else { continue };
            let first = out.len();
            for (k, step) in q.steps().iter().enumerate() {
                note = note.step(*step);
                if k == 0 {
                    if let Some(f) = n.fthora {
                        // The fthora names the note: the nearest such note.
                        let up = (f.degree.index() - note.degree.index()).rem_euclid(7);
                        note = note.step(if up > 3 { up - 7 } else { up });
                        genus = f.genus;
                    }
                }
                let start = out.last().map(|s: &Sung| s.start + s.length).unwrap_or(Frac::ZERO);
                let lyric = (k == 0 && !n.lyric.is_empty()).then(|| n.lyric.clone());
                out.push(Sung { start, length: Frac::int(1), note, genus, moria: genus.moria(note), element: i, lyric });
            }
            match n.time {
                Some(TimeSign::Klasma) => lengthen(&mut out, 1),
                Some(TimeSign::Dipli) => lengthen(&mut out, 2),
                Some(TimeSign::Tripli) => lengthen(&mut out, 3),
                Some(t @ (TimeSign::Gorgon | TimeSign::Digorgon | TimeSign::Trigorgon)) => {
                    let before = match t {
                        TimeSign::Gorgon => 1,
                        TimeSign::Digorgon => 2,
                        _ => 3,
                    };
                    if first < before {
                        problems.push(Problem::GorgonTooEarly { element: i });
                    } else {
                        // The notes before and this neume's first share one beat.
                        let from = first - before;
                        let share = Frac::new(1, before as i64 + 1);
                        let start = out[from].start;
                        for (j, s) in out[from..=first].iter_mut().enumerate() {
                            s.start = start + share * Frac::int(j as i64);
                            s.length = share;
                        }
                        // The neume's other notes follow on.
                        for j in first + 1..out.len() {
                            out[j].start = out[j - 1].start + out[j - 1].length;
                        }
                    }
                }
                None => {}
            }
        }
        (out, problems)
    }
}

/// Lengthens the last note by `beats`.
fn lengthen(out: &mut [Sung], beats: i64) {
    if let Some(s) = out.last_mut() {
        s.length = s.length + Frac::int(beats);
    }
}

/// A short chant that shows the notation at work: "Κύριε ἐλέησον" three
/// times in the first mode, with time signs, a gorgon, a melisma, a fthora
/// to the hard chromatic and back, and martyriae.
pub fn demo() -> Chant {
    use Quantity::*;
    let mut c = Chant::new("Κύριε ἐλέησον", Echos::First);
    let n = Neume::new;
    let kyrie = |c: &mut Chant, last: Quantity| {
        c.push(n(Ison).lyric("Κύ-"));
        c.push(n(Oligon).lyric("ρι-"));
        c.push(n(Apostrofos).lyric("ε").time(TimeSign::Klasma));
        c.push(n(Ison).lyric("ἐ-"));
        c.push(n(Oligon).lyric("λέ-"));
        c.push(n(Kentimata).time(TimeSign::Gorgon));
        c.push(n(Apostrofos).lyric("η-"));
        c.push(n(Yporroi).lyric("σον"));
        c.push(n(last).time(TimeSign::Dipli));
    };
    kyrie(&mut c, Oligon);
    c.elements.push(Element::Martyria(Degree::Pa));
    c.push(n(Ison).lyric("Κύ-"));
    c.push(n(OligonKentimaAbove).lyric("ρι-").fthora(Genus::HardChromatic, Degree::Di));
    c.push(n(Apostrofos).lyric("ε").time(TimeSign::Klasma));
    c.push(n(Apostrofos).lyric("ἐ-"));
    c.push(n(Apostrofos).lyric("λέ-").fthora(Genus::Diatonic, Degree::Pa));
    c.push(n(Ison).lyric("η-"));
    c.push(n(Petasti).lyric("σον").time(TimeSign::Klasma));
    c.push(n(Apostrofos));
    c.push(n(Apostrofos).time(TimeSign::Gorgon));
    c.push(n(Oligon).time(TimeSign::Dipli));
    c.elements.push(Element::Martyria(Degree::Pa));
    c.elements.push(Element::Break);
    kyrie(&mut c, Oligon);
    c.elements.push(Element::Martyria(Degree::Pa));
    c
}
