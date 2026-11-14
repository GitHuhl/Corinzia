//! Byzantine chants: showing one, and writing in it at the caret.
//!
//! While a chant is shown (`chant`), the caret (`chant_caret`) stands
//! between its elements: neumes and martyriae go in at the caret, and the
//! signs (time signs, fthoras, syllables) go on the neume just before it.
//! Every change is one undo step, like the rest of the editor.

use super::*;
use corinzia_byz::{Chant, Echos, Element, Fthora, Genus, Neume, Quantity, TimeSign};

impl Editor {
    /// The chant shown, if any.
    pub fn shown_chant(&self) -> Option<&Chant> {
        self.chant.and_then(|i| self.project.chants.get(i))
    }

    /// Shows chant `i` (its end), or the movements again (`None`).
    pub fn show_chant(&mut self, i: Option<usize>) {
        self.chant = i.filter(|i| *i < self.project.chants.len());
        self.chant_caret = self.shown_chant().map_or(0, |c| c.elements.len());
        self.input_mode = false;
        self.selection = None;
        self.range = None;
        self.revision += 1;
        self.music_revision += 1;
        if let Some(c) = self.shown_chant() {
            self.status = format!("{} · {}", c.title, c.mode.echos.greek());
        }
    }

    /// A new, empty chant in a mode, after the others; shown.
    pub fn add_chant(&mut self, echos: Echos) {
        let before = self.snapshot();
        let n = self.project.chants.len() + 1;
        self.project.chants.push(Chant::new(&format!("Chant {n}"), echos));
        self.commit(before, "New chant");
        self.show_chant(Some(self.project.chants.len() - 1));
    }

    /// Deletes the chant shown.
    pub fn delete_chant(&mut self) {
        let Some(i) = self.chant.filter(|i| *i < self.project.chants.len()) else { return };
        let before = self.snapshot();
        self.project.chants.remove(i);
        self.commit(before, "Delete chant");
        self.show_chant(None);
    }

    /// A change to the chant shown, as one undo step. `f` says whether it
    /// changed anything.
    fn chant_edit(&mut self, label: &str, f: impl FnOnce(&mut Chant, &mut usize) -> bool) -> bool {
        let Some(i) = self.chant.filter(|i| *i < self.project.chants.len()) else { return false };
        let before = self.snapshot();
        let mut caret = self.chant_caret.min(self.project.chants[i].elements.len());
        if !f(&mut self.project.chants[i], &mut caret) {
            return false;
        }
        self.chant_caret = caret;
        self.commit(before, label);
        true
    }

    /// The neume just before the caret.
    fn neume_before_caret<'c>(c: &'c mut Chant, caret: usize) -> Option<&'c mut Neume> {
        match c.elements[..caret].iter_mut().next_back() {
            Some(Element::Neume(n)) => Some(n),
            _ => None,
        }
    }

    /// Writes a neume at the caret.
    pub fn chant_neume(&mut self, q: Quantity) {
        self.chant_edit("Neume", |c, caret| {
            c.elements.insert(*caret, Element::Neume(Neume::new(q)));
            *caret += 1;
            true
        });
    }

    /// Writes a martyria at the caret, naming the note the melody is on.
    pub fn chant_martyria(&mut self) {
        let Some(c) = self.shown_chant() else { return };
        let at = self.chant_caret;
        let (sung, _) = c.sing();
        let degree = sung.iter().rev().find(|s| s.element < at).map_or(c.mode.base.degree, |s| s.note.degree);
        self.chant_edit("Martyria", |c, caret| {
            c.elements.insert(*caret, Element::Martyria(degree));
            *caret += 1;
            true
        });
    }

    /// Ends the line at the caret (or joins it again).
    pub fn chant_break(&mut self) {
        self.chant_edit("Line break", |c, caret| {
            if *caret > 0 && c.elements[*caret - 1] == Element::Break {
                c.elements.remove(*caret - 1);
                *caret -= 1;
            } else {
                c.elements.insert(*caret, Element::Break);
                *caret += 1;
            }
            true
        });
    }

    /// Deletes the element before the caret (`forward`: after it).
    pub fn chant_delete(&mut self, forward: bool) {
        self.chant_edit("Delete", |c, caret| {
            if forward && *caret < c.elements.len() {
                c.elements.remove(*caret);
                true
            } else if !forward && *caret > 0 {
                c.elements.remove(*caret - 1);
                *caret -= 1;
                true
            } else {
                false
            }
        });
    }

    /// Moves the caret by elements.
    pub fn chant_move(&mut self, by: i64) {
        let Some(c) = self.shown_chant() else { return };
        self.chant_caret = (self.chant_caret as i64 + by).clamp(0, c.elements.len() as i64) as usize;
        self.revision += 1;
    }

    /// Puts the caret after element `i` (a click on it).
    pub fn chant_caret_after(&mut self, i: usize) {
        let Some(c) = self.shown_chant() else { return };
        self.chant_caret = (i + 1).min(c.elements.len());
        self.revision += 1;
    }

    /// Steps the neume's time sign through `cycle` and back to none.
    pub fn chant_time(&mut self, cycle: &[TimeSign]) {
        self.chant_edit("Time sign", |c, caret| {
            let Some(n) = Self::neume_before_caret(c, *caret) else { return false };
            let next = match n.time.and_then(|t| cycle.iter().position(|x| *x == t)) {
                Some(k) => cycle.get(k + 1).copied(),
                None => cycle.first().copied(),
            };
            n.time = next;
            true
        });
    }

    /// Steps the neume's fthora: none, then the diatonic, hard chromatic and
    /// soft chromatic genus on the note it sings, then none again.
    pub fn chant_fthora(&mut self) {
        let Some(c) = self.shown_chant() else { return };
        let at = self.chant_caret;
        let (sung, _) = c.sing();
        let Some(s) = sung.iter().rev().find(|s| s.element + 1 == at) else { return };
        let degree = s.note.degree;
        self.chant_edit("Fthora", |c, caret| {
            let Some(n) = Self::neume_before_caret(c, *caret) else { return false };
            const ORDER: [Genus; 3] = [Genus::Diatonic, Genus::HardChromatic, Genus::SoftChromatic];
            n.fthora = match n.fthora.map(|f| ORDER.iter().position(|g| *g == f.genus)) {
                None => Some(Fthora { genus: ORDER[0], degree }),
                Some(Some(k)) if k + 1 < ORDER.len() => Some(Fthora { genus: ORDER[k + 1], degree }),
                _ => None,
            };
            true
        });
    }

    /// Sets the syllable on the neume before the caret (a trailing `-`: the
    /// word goes on).
    pub fn chant_lyric(&mut self, text: &str) -> Result<(), String> {
        let text = text.trim().to_string();
        let done = self.chant_edit("Syllable", |c, caret| {
            let Some(n) = Self::neume_before_caret(c, *caret) else { return false };
            n.lyric = text;
            true
        });
        if done {
            Ok(())
        } else {
            Err("Put the caret after a neume first".into())
        }
    }

    /// Typing syllables as a stream: the syllable goes on the neume after
    /// the caret, and the caret moves past it to the next.
    pub fn chant_lyric_next(&mut self, text: &str) -> Result<(), String> {
        let Some(c) = self.shown_chant() else { return Err("No chant shown".into()) };
        let Some(next) = c.elements[self.chant_caret.min(c.elements.len())..].iter().position(|e| matches!(e, Element::Neume(_))) else {
            return Err("No neume after the caret".into());
        };
        self.chant_caret += next + 1;
        self.chant_lyric(text)
    }

    /// The mode signature, typed: "1"–"8" (5–8 the plagal modes, 7 the
    /// grave), or "α΄", "πλ. β΄", "βαρύς", "plagal 2"…
    pub fn chant_mode(&mut self, text: &str) -> Result<(), String> {
        let echos = parse_echos(text).ok_or_else(|| format!("Mode not understood: {text}"))?;
        self.chant_edit("Mode", |c, _| {
            c.mode = corinzia_byz::ModeKey::new(echos);
            true
        });
        Ok(())
    }

    pub fn chant_title(&mut self, text: &str) {
        let text = text.trim().to_string();
        self.chant_edit("Chant title", |c, _| {
            c.title = text;
            true
        });
    }

    /// "Neume 4 of 20 · Πα · diatonic" for the status bar.
    pub fn chant_label(&self) -> Option<String> {
        let c = self.shown_chant()?;
        let (sung, problems) = c.sing();
        let at = self.chant_caret;
        let last = sung.iter().rev().find(|s| s.element < at);
        let (note, genus) = last.map_or((c.mode.base, c.mode.genus), |s| (s.note, s.genus));
        let genus = match genus {
            Genus::Diatonic => "διατονικό",
            Genus::HardChromatic => "σκληρό χρωματικό",
            Genus::SoftChromatic => "μαλακό χρωματικό",
        };
        let mut label = format!("{} · {at} of {} · {} · {genus}", c.mode.echos.greek(), c.elements.len(), note.degree.greek());
        if let Some(p) = problems.first() {
            label += &match p {
                corinzia_byz::Problem::MartyriaDisagrees { element, written, sung } => {
                    format!(" · martyria {} says {}, the melody is on {}", element + 1, written.greek(), sung.greek())
                }
                corinzia_byz::Problem::GorgonTooEarly { element } => format!(" · gorgon {} has too few notes before it", element + 1),
            };
        }
        Some(label)
    }
}

/// A mode as typed.
pub fn parse_echos(text: &str) -> Option<Echos> {
    let t = text.trim().to_lowercase().replace(['΄', '\'', '.', 'ς'], "").replace("ήχος", "").replace("ηχος", "").replace("ἦχο", "");
    let t = t.trim();
    let plagal = t.starts_with("πλ") || t.starts_with("pl");
    let rest = t.trim_start_matches("plagal").trim_start_matches("pl").trim_start_matches("πλ").trim();
    let n = match rest {
        "1" | "α" | "a" | "first" => 1,
        "2" | "β" | "b" | "second" => 2,
        "3" | "γ" | "c" | "third" => 3,
        "4" | "δ" | "d" | "fourth" => 4,
        "5" => 5,
        "6" => 6,
        "7" | "βαρυ" | "βαρύ" | "varys" | "grave" => 7,
        "8" => 8,
        _ => return None,
    };
    let n = if plagal && n <= 4 { n + 4 } else { n };
    Some(match n {
        1 => Echos::First,
        2 => Echos::Second,
        3 => Echos::Third,
        4 => Echos::Fourth,
        5 => Echos::PlagalFirst,
        6 => Echos::PlagalSecond,
        7 => Echos::Varys,
        _ => Echos::PlagalFourth,
    })
}
