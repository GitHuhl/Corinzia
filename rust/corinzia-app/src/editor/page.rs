//! Page items: titles, tempo marks, texts and other items edited on the page.

use super::*;

impl Editor {
    // --------------------------------------------------------- page items --
    // Titles, tempo marks, texts, chord symbols, dynamics and lyrics: the
    // things on the page that are words. A click selects them; a
    // double-click opens their popover with their text to retype.

    /// What editing `id` opens, and the text it starts with.
    pub fn item_text(&self, id: Id) -> Option<(PopoverKind, String)> {
        let m = self.movement()?;
        match id {
            ids::PROJECT_TITLE => return Some((PopoverKind::Title, self.project.title.clone())),
            ids::MOVEMENT_TITLE => return Some((PopoverKind::MovementTitle, m.title.clone())),
            ids::COMPOSER => return Some((PopoverKind::Composer, self.project.composer.clone())),
            _ => {}
        }
        if let Some(i) = ids::as_tempo(id) {
            let t = m.tempo.get(i)?;
            let bpm = t.bpm.round() as i64;
            return Some((PopoverKind::Tempo, if t.text.is_empty() { format!("q={bpm}") } else { format!("{} {bpm}", t.text) }));
        }
        if let Some(ev) = ids::as_dynamic(id) {
            let (_, e) = self.event_of(ev)?;
            return Some((PopoverKind::Dynamic, e.dynamic.and_then(|d| DYNAMICS.get(d as usize)).map(|d| d.to_string()).unwrap_or_default()));
        }
        if let Some((ev, verse)) = ids::as_lyric(id) {
            let (_, e) = self.event_of(ev)?;
            let l = e.lyrics.iter().find(|l| l.verse == verse)?;
            return Some((PopoverKind::Lyrics, format!("{}{}", l.text, if l.hyphen { "-" } else if l.extend { "_" } else { "" })));
        }
        for l in &m.lanes {
            if let Some(t) = l.texts.iter().find(|t| t.id == id) {
                return Some((if t.kind == TextKind::Expression { PopoverKind::Expression } else { PopoverKind::Technique }, edit::shown_text(t)));
            }
            if let Some(c) = l.chords.iter().find(|c| c.id == id) {
                return Some((PopoverKind::Chord, c.text.clone()));
            }
        }
        None
    }

    /// Replaces the text of the item `id` (see `item_text`); empty text
    /// removes a tempo mark, text, chord symbol, dynamic or lyric.
    pub fn edit_item(&mut self, id: Id, input: &str) -> Result<(), String> {
        let text = input.trim().to_string();
        let Some((kind, _)) = self.item_text(id) else { return Err("It is no longer there".into()) };
        match kind {
            PopoverKind::Title | PopoverKind::Composer => self.set_header(kind, &text),
            PopoverKind::MovementTitle => self.rename_movement(&text),
            PopoverKind::Dynamic => {
                let mark = parse_dynamic(&text)?;
                self.set_dynamic_of(ids::as_dynamic(id).unwrap(), mark);
            }
            PopoverKind::Lyrics => {
                let (ev, verse) = ids::as_lyric(id).unwrap();
                self.select(Some(ev));
                self.lyric_verse = verse;
                if text.is_empty() {
                    self.edit_selected("Lyric", |evs, id, _, _| edit::modify_event(evs, id, |e, _| e.lyrics.retain(|l| l.verse != verse)));
                } else {
                    self.apply_popover(PopoverKind::Lyrics, &text)?;
                }
            }
            PopoverKind::Tempo => {
                let i = ids::as_tempo(id).unwrap();
                if text.is_empty() {
                    self.delete_page_item(id);
                    return Ok(());
                }
                let t = edit::parse_tempo(&text).ok_or("Tempo not understood: try \"Allegro\", \"Andante 84\" or \"q=96\"")?;
                self.transact("Tempo", |m, _| {
                    let Some(old) = m.tempo.get_mut(i) else { return false };
                    old.text = t.text.clone();
                    old.bpm = t.bpm;
                    true
                });
            }
            PopoverKind::Expression | PopoverKind::Technique | PopoverKind::Chord => {
                if text.is_empty() {
                    self.selection = Some(id);
                    self.delete_structure_item();
                    return Ok(());
                }
                let chord = if kind == PopoverKind::Chord {
                    Some(edit::parse_chord(&text).ok_or("Chord symbol not understood: try C, F#m7, Bbmaj7, Dsus4, G7/B")?)
                } else {
                    None
                };
                let label = if chord.is_some() { "Chord symbol" } else { "Text" };
                self.transact(label, |m, _| {
                    let mut found = false;
                    for l in m.lanes.iter_mut() {
                        for t in l.texts.iter_mut().filter(|t| t.id == id) {
                            t.text = text.clone();
                            found = true;
                        }
                        for c in l.chords.iter_mut().filter(|c| c.id == id) {
                            c.text = chord.clone().unwrap_or_default();
                            found = true;
                        }
                    }
                    found
                });
            }
            _ => return Err("Not editable".into()),
        }
        self.status = format!("{} changed", kind.title());
        Ok(())
    }

    /// The project's title or composer.
    pub fn set_header(&mut self, kind: PopoverKind, text: &str) {
        let text = text.trim().to_string();
        fn field(p: &mut Project, kind: PopoverKind) -> &mut String {
            if kind == PopoverKind::Title { &mut p.title } else { &mut p.composer }
        }
        if *field(&mut self.project, kind) == text {
            return;
        }
        let before = self.snapshot();
        *field(&mut self.project, kind) = text;
        self.commit(before, kind.title());
    }

    pub(super) fn set_dynamic_of(&mut self, event: Id, mark: Option<u8>) {
        let Some((loc, _)) = self.event_of(event) else { return };
        let before = self.snapshot();
        let m = &mut self.project.movements[self.movement];
        let v = &mut m.lanes[loc.lane].voices[loc.voice];
        let mut evs = (*v.events).clone();
        let Some(e) = evs.iter_mut().find(|e| e.id == event) else { return };
        if e.dynamic == mark {
            return;
        }
        e.dynamic = mark;
        v.events = Arc::new(evs);
        self.commit(before, if mark.is_some() { "Dynamic" } else { "Remove dynamic" });
        if mark.is_none() && self.selection == Some(ids::dynamic(event)) {
            self.selection = None;
        }
    }

    /// Deletes a dynamic, tempo mark, lyric or the title or composer.
    pub(super) fn delete_page_item(&mut self, id: Id) -> bool {
        if let Some(ev) = ids::as_dynamic(id) {
            self.set_dynamic_of(ev, None);
            self.status = "Deleted dynamic".into();
            return true;
        }
        if let Some((ev, verse)) = ids::as_lyric(id) {
            self.select(Some(ev));
            self.edit_selected("Delete lyric", |evs, id, _, _| edit::modify_event(evs, id, |e, _| e.lyrics.retain(|l| l.verse != verse)));
            self.status = "Deleted lyric".into();
            return true;
        }
        if let Some(i) = ids::as_tempo(id) {
            // The first tempo stays (playback needs one); it loses its words.
            self.transact("Delete tempo mark", |m, _| {
                if i >= m.tempo.len() {
                    return false;
                }
                if m.tempo[i].at == Frac::ZERO {
                    m.tempo[i].text.clear();
                } else {
                    m.tempo.remove(i);
                }
                true
            });
            self.selection = None;
            self.status = "Deleted tempo mark".into();
            return true;
        }
        match id {
            ids::PROJECT_TITLE => self.set_header(PopoverKind::Title, ""),
            ids::COMPOSER => self.set_header(PopoverKind::Composer, ""),
            ids::MOVEMENT_TITLE => self.rename_movement(""),
            _ => return false,
        }
        self.selection = None;
        true
    }

    pub(super) fn describe_page_item(&self, id: Id) -> Option<String> {
        let (kind, text) = self.item_text(id)?;
        let what = match kind {
            PopoverKind::Expression | PopoverKind::Technique | PopoverKind::Chord => return None, // edit::describe_item
            PopoverKind::Lyrics => format!("Lyric \"{text}\" · verse {}", ids::as_lyric(id)?.1),
            PopoverKind::Tempo => format!("Tempo {text}"),
            k => format!("{} {text}", k.title()),
        };
        Some(format!("{what} · double-click to edit"))
    }
}
