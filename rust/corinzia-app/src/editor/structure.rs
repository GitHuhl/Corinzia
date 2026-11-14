//! Structure: the layout, explode/reduce, popovers (key, time, tempo, clef, …), bars, endings, lyrics and chord symbols.

use super::*;

impl Editor {
    // ------------------------------------------------------------- layout --
    /// The score's and the parts' layouts and the page texts (subtitle,
    /// lyricist, arranger, copyright), as one undo step.
    pub fn set_layout(&mut self, layout: Layout, parts: Layout, texts: [String; 4]) {
        let p = &self.project;
        let unchanged = p.layout == layout && p.part_layout == parts && [&p.subtitle, &p.lyricist, &p.arranger, &p.copyright].iter().zip(&texts).all(|(a, b)| *a == b);
        if unchanged {
            self.status = "No changes to the layout".into();
            return;
        }
        let before = self.snapshot();
        let p = &mut self.project;
        p.layout = layout;
        p.part_layout = parts;
        let [subtitle, lyricist, arranger, copyright] = texts;
        (p.subtitle, p.lyricist, p.arranger, p.copyright) = (subtitle, lyricist, arranger, copyright);
        self.commit(before, "Layout");
        self.status = "Layout changed".into();
    }

    /// A system break (or page break) before the selected bar, or the
    /// caret's; in `part`'s part, or in the full score. Again removes it;
    /// the other kind there turns it into this one.
    pub fn toggle_break(&mut self, page: bool, part: Option<Id>) -> Result<(), String> {
        let m = self.movement().ok_or("No movement")?;
        let t = match (self.range, self.selection.and_then(|id| self.event_of(id))) {
            (Some(r), _) => r.from,
            (None, Some((_, e))) => e.start,
            _ => self.caret,
        };
        let bar = edit::bar_start_at_or_before(m, t.min(m.length));
        if bar == Frac::ZERO {
            return Err("A break goes before a bar after the first: select a later bar".into());
        }
        let n = edit::bar_number(m, bar).0;
        let what = if page { "Page break" } else { "System break" };
        let before = self.snapshot();
        let id = self.project.allocate_id();
        let m = &mut self.project.movements[self.movement];
        let label = match m.breaks.iter().position(|b| b.at == bar && b.part == part) {
            Some(i) if m.breaks[i].page == page => {
                m.breaks.remove(i);
                format!("{what} removed")
            }
            Some(i) => {
                m.breaks[i].page = page;
                what.to_string()
            }
            None => {
                m.breaks.push(Break { id, at: bar, page, part });
                m.breaks.sort_by_key(|b| b.at);
                what.to_string()
            }
        };
        self.commit(before, &label);
        let scope = if part.is_some() { " in the part" } else { "" };
        self.status = format!("{label} before bar {n}{scope}");
        Ok(())
    }

    // ------------------------------------------------------ explode/reduce --
    /// The passage's staves, its voice-1 index on each, checked for
    /// explode/reduce: two staves or more, no tuplets.
    pub(super) fn staves_passage(&self, what: &str) -> Result<(Range, Vec<usize>), String> {
        let r = self.range.filter(|r| r.lane_to > r.lane_from).ok_or_else(|| format!("{what}: select a passage across two or more staves"))?;
        let m = self.movement().ok_or("No movement")?;
        let mut voice1 = Vec::new();
        for li in r.lane_from..=r.lane_to {
            let l = m.lanes.get(li).ok_or("No such staff")?;
            if l.voices.iter().any(|v| v.tuplets.iter().any(|t| t.start < r.to && t.end() > r.from)) {
                return Err(format!("{what} does not handle tuplets yet"));
            }
            voice1.push(l.voices.iter().position(|v| v.index == 1).ok_or("A staff has no voice 1")?);
        }
        Ok((r, voice1))
    }

    /// Spreads the chords of a staff in the passage over all its staves,
    /// top note to the top staff. One undo step.
    pub fn explode(&mut self) -> Result<(), String> {
        let (r, voice1) = self.staves_passage("Explode")?;
        let m = self.movement().unwrap();
        let has_chords = |li: usize| m.lanes[li].voices[voice1[li - r.lane_from]].events.iter().any(|e| e.start >= r.from && e.start < r.to && e.heads.len() > 1);
        let source = (r.lane_from..=r.lane_to).find(|&li| has_chords(li)).ok_or("Explode: no chords in the passage")?;
        let src = m.lanes[source].voices[voice1[source - r.lane_from]].events.clone();
        let parts = edit::staves::explode(&src, r.from, r.to, r.lane_to - r.lane_from + 1);
        self.transact("Explode", |m, ids| {
            for (j, clip) in parts.iter().enumerate() {
                let (li, vi) = (r.lane_from + j, voice1[j]);
                let res = edit::paste_events(&m.lanes[li].voices[vi].events, clip, r.from, r.to - r.from, true, ids);
                set_events(m, li, vi, &res);
            }
            true
        });
        self.status = format!("Exploded onto {} staves", parts.len());
        Ok(())
    }

    /// Combines the passage's staves into chords on one: an empty staff of
    /// the passage if there is one, else the top one. One undo step.
    pub fn reduce(&mut self) -> Result<(), String> {
        let (r, voice1) = self.staves_passage("Reduce")?;
        let m = self.movement().unwrap();
        let empty = |li: usize| m.lanes[li].voices.iter().all(|v| v.events.iter().all(|e| e.start < r.from || e.start >= r.to || e.is_rest()));
        let dest = (r.lane_from..=r.lane_to).find(|&li| empty(li)).unwrap_or(r.lane_from);
        let sources: Vec<std::sync::Arc<Vec<Event>>> = (r.lane_from..=r.lane_to).filter(|&li| li != dest || !empty(li)).flat_map(|li| m.lanes[li].voices.iter().map(|v| v.events.clone())).collect();
        let lists: Vec<&[Event]> = sources.iter().map(|v| v.as_slice()).collect();
        let clip = edit::staves::reduce(&lists, r.from, r.to);
        if clip.iter().all(|e| e.is_rest()) {
            return Err("Reduce: no notes in the passage".into());
        }
        let vi = voice1[dest - r.lane_from];
        self.transact("Reduce", |m, ids| {
            let res = edit::paste_events(&m.lanes[dest].voices[vi].events, &clip, r.from, r.to - r.from, true, ids);
            set_events(m, dest, vi, &res);
            true
        });
        let name = self.movement().and_then(|m| m.lanes[dest].changes.first()).and_then(|c| self.project.instrument(c.instrument)).map(|i| i.name.clone()).unwrap_or_default();
        self.status = format!("Reduced onto {name}");
        Ok(())
    }

    // ---------------------------------------------------------- structure --
    /// Where a popover acts: the passage start, else the selected element,
    /// else the caret. (lane, position)
    pub fn structure_target(&self) -> Option<(usize, Frac)> {
        let m = self.movement()?;
        if let Some(r) = self.range {
            return Some((r.lane_from, r.from));
        }
        if let Some((loc, e)) = self.selection.and_then(|s| self.event_of(s)) {
            return Some((loc.lane, e.start));
        }
        Some((self.caret_lane.min(m.lanes.len().saturating_sub(1)), self.caret.min(m.length)))
    }

    /// Points the popovers at a clef, key or time signature or barline
    /// right-clicked at `pos` on a lane's staff: the caret moves to the
    /// change drawn there (or the bar it opens), and nothing stays selected.
    pub fn target_mark(&mut self, lane: usize, pos: Frac, kind: PopoverKind) {
        let Some(m) = self.movement() else { return };
        if lane >= m.lanes.len() {
            return;
        }
        // Signatures are drawn just before the bar they open.
        let ahead = (pos + Frac::new(1, 16)).min(m.length);
        let at = match kind {
            // A clef change stands just before its note; a clef at a
            // system's start belongs to the bar there.
            PopoverKind::Clef => m.lanes[lane].clefs.iter().map(|c| c.at).filter(|&at| at >= pos - Frac::new(1, 16) && at <= pos + Frac::new(1, 2)).min().unwrap_or_else(|| edit::bar_start_at_or_before(m, ahead)),
            PopoverKind::Barline => edit::nearest_barline(m, pos),
            _ => edit::bar_start_at_or_before(m, ahead),
        };
        self.clear_selection();
        self.caret_lane = lane;
        self.caret = at;
    }

    /// "Bar 5" for the popover title.
    pub fn popover_position(&self) -> String {
        match (self.structure_target(), self.movement()) {
            (Some((_, at)), Some(m)) => format!("Bar {}", edit::bar_number(m, at).0),
            _ => String::new(),
        }
    }

    // ------------------------------------------------------ lyrics, chords --
    /// The note lyrics go on: the selected note, else the first note at the
    /// popover target in voice 1 of its staff.
    pub fn lyric_target(&self) -> Option<Id> {
        if let Some((_, e)) = self.selection.and_then(|s| self.event_of(s)) {
            if !e.is_rest() {
                return self.selection;
            }
        }
        let (lane, at) = self.structure_target()?;
        let v = self.movement()?.lanes.get(lane)?.voices.iter().find(|v| v.index == 1)?;
        v.events.iter().find(|e| e.start >= at && !e.is_rest()).map(|e| e.heads[0].id)
    }

    /// Writes a syllable on the target note and moves to the next note.
    /// Empty text just moves on (a melisma). Returns the next note.
    pub fn enter_lyric(&mut self, verse: u8, text: &str, hyphen: bool, extend: bool) -> Result<Option<Id>, String> {
        let target = self.lyric_target().ok_or("Select a note for the lyrics")?;
        let loc = self.locate(target).ok_or("Select a note for the lyrics")?;
        if !text.is_empty() {
            let label = format!("Lyric \"{text}\"");
            self.transact(&label, |m, _| match edit::set_lyric(&m.lanes[loc.lane].voices[loc.voice].events.clone(), target, verse, text, hyphen, extend) {
                Some(r) => {
                    set_events(m, loc.lane, loc.voice, &r);
                    true
                }
                None => false,
            });
        }
        let m = self.movement().ok_or("No movement")?;
        let next = edit::next_note(&m.lanes[loc.lane].voices[loc.voice].events, target).map(|e| e.heads[0].id);
        self.selection = next.or(Some(target));
        self.range = None;
        Ok(next)
    }

    /// Moves the popover target to the next event (note or rest) on the staff.
    pub fn advance_target(&mut self) {
        let Some((lane, at)) = self.structure_target() else { return };
        let Some(v) = self.movement().and_then(|m| m.lanes.get(lane)).and_then(|l| l.voices.iter().find(|v| v.index == 1)) else { return };
        if let Some(e) = v.events.iter().find(|e| e.start > at) {
            let id = e.heads.first().map(|h| h.id).unwrap_or(e.id);
            self.selection = Some(id);
            self.range = None;
        }
    }

    /// Whole bars covering the passage, else the bar at `at`.
    pub(super) fn selected_bars(&self, m: &Movement, at: Frac) -> (Frac, Frac) {
        let from = edit::bar_start_at_or_before(m, self.range.map(|r| r.from).unwrap_or(at));
        let mut to = from + edit::bar_length(m, from);
        if let Some(r) = self.range {
            if r.to > to {
                to = edit::bar_start_at_or_before(m, r.to);
                if to < r.to {
                    to = to + edit::bar_length(m, to);
                }
            }
        }
        (from, to.min(m.length))
    }

    /// Applies a popover; `Err` carries the hint to show.
    pub fn apply_popover(&mut self, kind: PopoverKind, input: &str) -> Result<(), String> {
        let text = input.trim().to_string();
        if kind == PopoverKind::Fingering {
            let fingers = edit::parse_fingering(&text).ok_or("Fingering not understood: 1, 2… or \"1 3 5\" for a chord")?;
            if self.range.is_none() && self.selection.is_none() {
                return Err("Select a note first".into());
            }
            self.edit_selected("Fingering", |evs, id, _, _| edit::modify_event(evs, id, |e, _| e.fingering = fingers.clone()));
            return Ok(());
        }
        // Bowings in the technique popover are marks on the notes.
        if kind == PopoverKind::Technique {
            if let Some(bit) = edit::parse_bowing(&text) {
                if self.range.is_none() && self.selection.is_none() {
                    return Err("Select notes first".into());
                }
                self.toggle_articulation(bit);
                return Ok(());
            }
        }
        if kind == PopoverKind::Instrument {
            return self.change_instrument(&text);
        }
        if kind == PopoverKind::Transpose {
            return self.transpose(&text);
        }
        if kind == PopoverKind::Filter {
            return self.filter(&text);
        }
        if kind == PopoverKind::MovementTitle {
            self.rename_movement(&text);
            return Ok(());
        }
        if kind == PopoverKind::Title || kind == PopoverKind::Composer {
            self.set_header(kind, &text);
            return Ok(());
        }
        if kind == PopoverKind::Dynamic {
            let mark = parse_dynamic(&text)?;
            if self.range.is_none() && self.selection.is_none() {
                return Err("Select a note first".into());
            }
            self.set_dynamic(mark);
            return Ok(());
        }
        if kind == PopoverKind::Tuplet {
            let (a, n) = edit::tuplets::parse_tuplet(&text).ok_or("Tuplet not understood: try 3, 5, 5:4 or 7:8")?;
            self.apply_tuplet(a, n);
            return Ok(());
        }
        if kind == PopoverKind::Ornament {
            let bit = edit::parse_ornament(&text).ok_or("Ornament not understood: tr, mordent, turn, trem, arp, breath, caesura, fermata")?;
            if self.range.is_none() && self.selection.is_none() {
                return Err("Select notes first".into());
            }
            self.toggle_articulation(bit);
            return Ok(());
        }
        let (lane, target) = self.structure_target().ok_or("No movement")?;
        let m = self.movement().ok_or("No movement")?;
        if m.lanes.is_empty() {
            return Err("No staves".into());
        }
        let bar = edit::bar_start_at_or_before(m, target);
        let player = m.lanes[lane].player;
        let bars = self.selected_bars(m, target);
        let mut at = target;
        let label: String;
        let change: Box<dyn FnOnce(&mut Movement, &mut Ids)> = match kind {
            PopoverKind::Key => {
                let k = edit::parse_key(&text).ok_or("Key not understood: try C, Eb, f# (minor) or \"a minor\"")?;
                at = bar;
                label = "Key signature".into();
                Box::new(move |m, _| edit::set_key(m, bar, k))
            }
            PopoverKind::Time if edit::parse_pickup(&text, Frac::new(1, 4)).is_some() => {
                let beat = m.meter.iter().find(|x| !x.pickup).map(|x| Frac::new(1, x.den)).unwrap_or(Frac::new(1, 4));
                let len = edit::parse_pickup(&text, beat).flatten();
                at = Frac::ZERO;
                label = if len.is_some() { "Pickup bar".into() } else { "Remove pickup".into() };
                Box::new(move |m, _| edit::set_pickup(m, len))
            }
            PopoverKind::Time => {
                let (n, d, beams) = edit::parse_time_grouped(&text).ok_or("Time signature not understood: try 3/4, 6/8, C, C|, \"7/8 2+2+3\" or \"pickup 1/4\"")?;
                at = bar;
                label = "Time signature".into();
                Box::new(move |m, _| {
                    edit::set_meter_grouped(m, bar, n, d, beams);
                    // Keep the movement a whole number of bars in the new meter.
                    let last = edit::bar_start_at_or_before(m, m.length);
                    if last != m.length {
                        m.length = last + edit::bar_length(m, last);
                    }
                })
            }
            PopoverKind::Clef | PopoverKind::Technique if line_text(kind, &text).is_some() => {
                // Octave lines and pedal over the passage, the selected note, or the bar.
                let (l0, l1, from, to) = match (self.range, self.selection.and_then(|id| self.event_of(id))) {
                    (Some(r), _) => (r.lane_from, r.lane_to, r.from, r.to),
                    (None, Some((loc, e))) => (loc.lane, loc.lane, e.start, e.end()),
                    _ => (lane, lane, bars.0, bars.1),
                };
                at = from;
                let line = line_text(kind, &text).unwrap();
                label = match line {
                    Some(LineKind::Pedal) => "Pedal".into(),
                    Some(k) => format!("{} line", k.code()),
                    None if kind == PopoverKind::Technique => "Pedal off".into(),
                    None => "Loco".into(),
                };
                Box::new(move |m, ids| {
                    for li in l0..=l1.min(m.lanes.len().saturating_sub(1)) {
                        match line {
                            Some(k) => edit::set_line(m, li, from, to, k, ids),
                            None => edit::remove_lines(m, li, from, to, kind == PopoverKind::Technique, ids),
                        }
                    }
                })
            }
            PopoverKind::Clef => {
                let c = edit::parse_clef(&text).ok_or("Clef not understood: treble, bass, alto, tenor, treble_8, bass_8")?;
                label = "Clef".into();
                Box::new(move |m, ids| edit::set_clef(m, player, target, c, ids))
            }
            PopoverKind::Barline => {
                let b = edit::parse_barline(&text).ok_or("Barline not understood: ||  |.  |:  :|  :|:")?;
                at = edit::nearest_barline(m, target);
                if at == Frac::ZERO && b != BarKind::RepeatStart {
                    at = (bar + edit::bar_length(m, bar)).min(m.length);
                }
                label = "Barline".into();
                let at = at;
                Box::new(move |m, ids| edit::set_barline(m, at, b, ids))
            }
            PopoverKind::Tempo => {
                let t = edit::parse_tempo(&text).ok_or("Tempo not understood: try \"Allegro\", \"Andante 84\" or \"q=96\"")?;
                label = "Tempo".into();
                Box::new(move |m, _| edit::set_tempo(m, target, &t))
            }
            PopoverKind::Expression | PopoverKind::Technique => {
                if text.is_empty() {
                    return Err("Type some text first".into());
                }
                let k = if kind == PopoverKind::Expression { TextKind::Expression } else { TextKind::Technique };
                label = if k == TextKind::Expression { "Expression text" } else { "Technique text" }.into();
                Box::new(move |m, ids| edit::add_text(m, player, target, &text, k, ids))
            }
            PopoverKind::Ending if edit::parse_nav(&text).is_some() => {
                let kind = edit::parse_nav(&text).unwrap();
                // Signs open their bar; words close it.
                let pos = if kind.at_bar_start() { bars.0 } else { bars.1 };
                at = pos;
                let k = kind;
                label = match kind {
                    Nav::Segno => "Segno".into(),
                    Nav::Coda => "Coda".into(),
                    k => k.text().to_string(),
                };
                Box::new(move |m, ids| edit::toggle_nav(m, pos, k, ids))
            }
            PopoverKind::Ending => {
                let passes = edit::parse_ending(&text).ok_or("Not understood: endings 1, 2, 1-3; or segno, coda, to coda, fine, D.C. al Fine, D.S. al Coda")?;
                at = bars.0;
                label = format!("Ending {}", edit::ending_label(passes));
                Box::new(move |m, ids| edit::set_ending(m, bars.0, bars.1, passes, ids))
            }
            PopoverKind::Rehearsal => {
                at = bar;
                label = if m.rehearsal.iter().any(|r| r.at == bar) { "Remove rehearsal mark" } else { "Rehearsal mark" }.into();
                Box::new(move |m, ids| edit::toggle_rehearsal_mark(m, bar, ids))
            }
            PopoverKind::Chord => {
                let c = edit::parse_chord(&text).ok_or("Chord symbol not understood: try C, F#m7, Bbmaj7, Dsus4, G7/B")?;
                label = format!("Chord symbol {c}");
                Box::new(move |m, ids| edit::set_chord(m, player, target, &c, ids))
            }
            PopoverKind::Lyrics => {
                let verse = self.lyric_verse;
                let (word, hyphen, extend) = match text.strip_suffix('-').map(|w| (w, true, false)).or(text.strip_suffix('_').map(|w| (w, false, true))) {
                    Some(x) => x,
                    None => (text.as_str(), false, false),
                };
                return self.enter_lyric(verse, word.trim_end(), hyphen, extend).map(|_| ());
            }
            PopoverKind::Ornament | PopoverKind::Tuplet | PopoverKind::MovementTitle | PopoverKind::Fingering | PopoverKind::Dynamic | PopoverKind::Title | PopoverKind::Composer | PopoverKind::Instrument | PopoverKind::Transpose | PopoverKind::Filter => unreachable!(),
        };
        self.transact(&label, |m, ids| {
            change(m, ids);
            true
        });
        let m = self.movement().unwrap();
        self.status = format!("{label} · bar {}", edit::bar_number(m, at.min(m.length)).0);
        Ok(())
    }

    pub fn add_first_second_endings(&mut self) {
        let Some((_, at)) = self.structure_target() else { return };
        let m = self.movement().unwrap();
        let (from, to) = self.selected_bars(m, at);
        // 1st ending = the selected bars, closed by an end repeat; 2nd = as
        // many bars after it, as far as the movement goes.
        let to2 = (to + (to - from)).min(m.length);
        if to >= to2 {
            self.status = "No room for a 2nd ending after the selection".into();
            return;
        }
        let (b0, b1) = (edit::bar_number(m, from).0, edit::bar_number(m, to2 - Frac::new(1, 1024)).0);
        self.transact("1st and 2nd endings", |m, ids| {
            edit::set_ending(m, from, to, 1, ids);
            edit::set_ending(m, to, to2, 2, ids);
            edit::set_barline(m, to, BarKind::RepeatEnd, ids);
            true
        });
        self.status = format!("1st and 2nd endings · bars {b0}–{b1}");
    }

    pub fn insert_bars(&mut self, count: i64) {
        let Some((_, target)) = self.structure_target() else { return };
        if count < 1 {
            return;
        }
        let m = self.movement().unwrap();
        let at = edit::bar_start_at_or_before(m, target);
        let len = edit::bar_length(m, at) * Frac::int(count);
        let bar = edit::bar_number(m, at).0;
        self.transact("Insert bars", |m, _| {
            for l in m.lanes.iter_mut() {
                for v in l.voices.iter_mut() {
                    v.events = Arc::new(edit::insert_span_events(&v.events, at, len));
                    v.tuplets = edit::tuplets::shift(&v.tuplets, at, len);
                }
            }
            edit::insert_span(m, at, len);
            true
        });
        self.status = format!("Inserted {count} bar{} before bar {bar}", if count == 1 { "" } else { "s" });
    }

    pub fn delete_bars(&mut self) {
        let Some((_, target)) = self.structure_target() else { return };
        let m = self.movement().unwrap();
        let (from, to) = self.selected_bars(m, target);
        if from >= to || to - from >= m.length {
            self.status = "A movement needs at least one bar".into();
            return;
        }
        let (b0, b1) = (edit::bar_number(m, from).0, edit::bar_number(m, to - Frac::new(1, 1024)).0);
        self.transact("Delete bars", |m, _| {
            for l in m.lanes.iter_mut() {
                for v in l.voices.iter_mut() {
                    v.events = Arc::new(edit::delete_span_events(&v.events, from, to));
                    v.tuplets = edit::tuplets::cut(&v.tuplets, from, to);
                }
            }
            edit::delete_span(m, from, to);
            true
        });
        self.range = None;
        self.selection = None;
        let len = self.movement().unwrap().length;
        self.caret = from.min(len);
        self.status = if b0 == b1 { format!("Deleted bar {b0}") } else { format!("Deleted bars {b0}–{b1}") };
    }

    /// A selected barline, mark, ending, clef change or text: removed.
    pub fn delete_structure_item(&mut self) -> bool {
        let Some(id) = self.selection else { return false };
        if self.delete_page_item(id) {
            return true;
        }
        let Some(what) = self.movement().and_then(|m| edit::describe_item(m, id)) else { return false };
        self.transact(&format!("Delete {what}"), |m, _| edit::remove_item(m, id).is_some());
        self.selection = None;
        self.status = format!("Deleted {what}");
        true
    }
}
