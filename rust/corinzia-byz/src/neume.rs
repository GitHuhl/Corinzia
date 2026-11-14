//! Neumes: what each sign means for the melody and for time.
//!
//! A quantity neume moves the voice by steps of the scale from the note
//! before it; some sing two or three notes. Time signs lengthen a note
//! (klasma, dipli, tripli) or share one beat among several (gorgon,
//! digorgon, trigorgon). A fthora changes the genus from its note on, and
//! a martyria names the note the melody has reached.

use crate::scale::{Degree, Genus};

/// The body of a neume and the notes it sings, as steps from the note
/// before it (each step relative to the previous note).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Quantity {
    /// The same note again.
    Ison,
    /// One step up.
    Oligon,
    /// One step up, sung with an accent.
    Petasti,
    /// One step up, light (only after another note).
    Kentimata,
    /// Two steps up.
    Kentima,
    /// Three steps up: oligon with kentima above.
    OligonKentimaAbove,
    /// Four steps up: oligon with ypsili at its right end.
    OligonYpsiliRight,
    /// Five steps up: oligon with ypsili at its left end.
    OligonYpsiliLeft,
    /// One step down.
    Apostrofos,
    /// Two steps down.
    Elafron,
    /// Three steps down: elafron with apostrofos.
    ElafronApostrofos,
    /// Four steps down.
    Chamili,
    /// Two notes, each one step down.
    Yporroi,
    /// Two notes, each one step up: kentimata under oligon.
    OligonKentimataBelow,
    /// Two notes: the same note, then one step up (oligon with ison and
    /// kentimata).
    OligonIsonKentimata,
}

impl Quantity {
    /// The steps of each note the neume sings, from the note before it.
    pub fn steps(self) -> &'static [i32] {
        use Quantity::*;
        match self {
            Ison => &[0],
            Oligon | Petasti | Kentimata => &[1],
            Kentima => &[2],
            OligonKentimaAbove => &[3],
            OligonYpsiliRight => &[4],
            OligonYpsiliLeft => &[5],
            Apostrofos => &[-1],
            Elafron => &[-2],
            ElafronApostrofos => &[-3],
            Chamili => &[-4],
            Yporroi => &[-1, -1],
            OligonKentimataBelow => &[1, 1],
            OligonIsonKentimata => &[0, 1],
        }
    }
}

/// A time sign on a neume.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum TimeSign {
    /// One beat more (on the neume's last note).
    Klasma,
    /// Two beats more.
    Dipli,
    /// Three beats more.
    Tripli,
    /// The note before and this neume's first note share one beat.
    Gorgon,
    /// The two notes before and this one share one beat.
    Digorgon,
    /// The three notes before and this one share one beat.
    Trigorgon,
}

/// A fthora: from this neume's (first) note on, the melody is sung in
/// `genus`, and that note is `degree`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct Fthora {
    pub genus: Genus,
    pub degree: Degree,
}

/// A neume with its signs and the syllable sung on it.
#[derive(Clone, Debug, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct Neume {
    pub quantity: Option<Quantity>,
    pub time: Option<TimeSign>,
    pub fthora: Option<Fthora>,
    /// The syllable under it; empty when the syllable before continues.
    pub lyric: String,
}

impl Neume {
    pub fn new(q: Quantity) -> Neume {
        Neume { quantity: Some(q), ..Default::default() }
    }

    pub fn time(mut self, t: TimeSign) -> Neume {
        self.time = Some(t);
        self
    }

    pub fn fthora(mut self, genus: Genus, degree: Degree) -> Neume {
        self.fthora = Some(Fthora { genus, degree });
        self
    }

    pub fn lyric(mut self, s: &str) -> Neume {
        self.lyric = s.into();
        self
    }
}
