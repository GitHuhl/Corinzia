//! Players and their instruments: adding, removing, reordering and instrument changes.

use super::*;

impl Editor {
    // ------------------------------------------------------------ players --
    /// Sets the instrumentation: the players in `list`, in its order, each
    /// with its instruments (the first, then doublings). Players left out go
    /// with their staves (in every movement), instruments and tracks; new
    /// ones get staves of whole-bar rests in every movement. A kept player's
    /// instruments are matched by name: a name still listed keeps its
    /// instrument; a new name takes the place of one no longer listed (the
    /// instrument changes, its music and track stay); any others are added
    /// as doublings with tracks of their own, and instruments left over go.
    /// One undo step.
    pub fn set_players(&mut self, list: &[PlayerSlot]) -> Result<(), String> {
        if list.is_empty() {
            return Err("A score needs at least one player".into());
        }
        if list.iter().any(|s| match s {
            PlayerSlot::Keep { instruments, .. } | PlayerSlot::New { instruments } => instruments.is_empty(),
        }) {
            return Err("Each player needs an instrument".into());
        }
        let before = self.snapshot();
        let p = &mut self.project;
        let kept: Vec<Id> = list.iter().filter_map(|s| if let PlayerSlot::Keep { id, .. } = s { Some(*id) } else { None }).collect();
        // Removed players and their instruments.
        let gone: Vec<Id> = p.players.iter().map(|pl| pl.id).filter(|id| !kept.contains(id)).collect();
        p.instruments.retain(|i| !gone.contains(&i.player));
        p.players.retain(|pl| !gone.contains(&pl.id));
        for m in p.movements.iter_mut() {
            m.lanes.retain(|l| !gone.contains(&l.player));
        }
        // Kept players' instruments.
        let mut changed = 0;
        for slot in list {
            if let PlayerSlot::Keep { id, instruments } = slot {
                changed += reconcile_instruments(p, *id, instruments);
            }
        }
        // New players.
        let mut order = Vec::new();
        for slot in list {
            let id = match slot {
                PlayerSlot::Keep { id, .. } => *id,
                PlayerSlot::New { instruments } => {
                    let player = p.allocate_id();
                    let ids: Vec<Id> = instruments.iter().map(|spec| add_instrument(p, player, spec)).collect();
                    p.players.push(Player { id: player, name: instruments[0].name.clone(), instruments: ids.clone() });
                    let clefs: Vec<String> = instruments[0].clef.split('+').map(str::to_string).collect();
                    for mi in 0..p.movements.len() {
                        let bars: Vec<(Frac, Frac)> = {
                            let m = &p.movements[mi];
                            edit::bar_starts(m).into_iter().map(|b| (b, edit::bar_length(m, b).min(m.length - b))).collect()
                        };
                        for (si, c) in clefs.iter().enumerate() {
                            let rests: Vec<Event> = bars
                                .iter()
                                .map(|&(start, duration)| Event { id: p.allocate_id(), kind: EventKind::Rest, start, duration, heads: vec![], articulations: 0, dynamic: None, spans: 0, lyrics: vec![], graces: vec![], fingering: vec![] })
                                .collect();
                            let (change, voice) = (p.allocate_id(), p.allocate_id());
                            let clef_changes = if si == 0 { vec![] } else { vec![ClefChange { id: p.allocate_id(), at: Frac::ZERO, clef: c.clone() }] };
                            p.movements[mi].lanes.push(Lane {
                                player,
                                changes: vec![InstrumentChange { id: change, at: Frac::ZERO, instrument: ids[0] }],
                                clefs: clef_changes,
                                texts: vec![],
                                chords: vec![],
                                lines: vec![],
                                voices: vec![Voice { id: voice, index: 1, events: Arc::new(rests), tuplets: vec![] }],
                            });
                        }
                    }
                    player
                }
            };
            order.push(id);
        }
        drop_unused_tracks(p);
        // The list's order: players, their staves (a player's own staves
        // keep theirs), and the mixer's tracks.
        let rank = |player: Id| order.iter().position(|x| *x == player).unwrap_or(usize::MAX);
        p.players.sort_by_key(|pl| rank(pl.id));
        for m in p.movements.iter_mut() {
            m.lanes.sort_by_key(|l| rank(l.player)); // stable
        }
        let mut track_rank: std::collections::HashMap<Id, (usize, usize)> = std::collections::HashMap::new();
        for pl in &p.players {
            for (k, i) in pl.instruments.iter().enumerate() {
                if let Some(t) = p.instruments.iter().find(|x| x.id == *i).and_then(|x| x.track) {
                    let r = (rank(pl.id), k);
                    let e = track_rank.entry(t).or_insert(r);
                    *e = (*e).min(r);
                }
            }
        }
        p.tracks.sort_by_key(|t| track_rank.get(&t.id).copied().unwrap_or((usize::MAX, 0)));
        let (added, removed) = (list.len() - kept.len(), gone.len());
        let same_order = before.players.iter().map(|pl| pl.id).eq(self.project.players.iter().map(|pl| pl.id));
        if added == 0 && removed == 0 && changed == 0 && same_order {
            self.project = before;
            self.status = "No changes to the players".into();
            return Ok(());
        }
        // Nothing to point at in staves that may be gone.
        self.selection = None;
        self.range = None;
        self.caret = Frac::ZERO;
        self.caret_lane = 0;
        let plural = |n: usize| if n == 1 { "" } else { "s" };
        let mut parts = Vec::new();
        if added > 0 {
            parts.push(format!("{added} player{} added", plural(added)));
        }
        if removed > 0 {
            parts.push(format!("{removed} player{} removed", plural(removed)));
        }
        if changed > 0 {
            parts.push(format!("{changed} instrument{} changed", plural(changed)));
        }
        self.status = if parts.is_empty() { "Players updated".into() } else { parts.join(", ") };
        self.commit(before, "Players");
        Ok(())
    }

    /// An instrument change at the caret, the selection's start or the
    /// selected bar, on its player's staves: to an instrument the player
    /// holds, or one added as a doubling (with a track). The clef follows
    /// the instrument. Empty text removes the change there.
    pub fn change_instrument(&mut self, text: &str) -> Result<(), String> {
        let (lane, target) = self.structure_target().ok_or("No movement")?;
        let m = self.movement().ok_or("No movement")?;
        let player = m.lanes.get(lane).ok_or("No staves")?.player;
        let mi = self.movement;
        let before = self.snapshot();
        if text.trim().is_empty() {
            let mut removed = false;
            for l in self.project.movements[mi].lanes.iter_mut().filter(|l| l.player == player) {
                let n = l.changes.len();
                l.changes.retain(|c| c.at == Frac::ZERO || c.at != target);
                removed |= l.changes.len() != n;
            }
            if !removed {
                return Err("No instrument change here".into());
            }
            self.commit(before, "Remove instrument change");
            self.status = "Instrument change removed".into();
            return Ok(());
        }
        let spec = crate::files::instrument_spec(text).ok_or("Name an instrument")?;
        let p = &mut self.project;
        let held = p.player(player).map(|pl| pl.instruments.clone()).unwrap_or_default();
        let inst = match held.iter().copied().find(|i| p.instrument(*i).is_some_and(|x| x.name.eq_ignore_ascii_case(&spec.name))) {
            Some(i) => i,
            None => {
                let i = add_instrument(p, player, &spec);
                if let Some(pl) = p.players.iter_mut().find(|pl| pl.id == player) {
                    pl.instruments.push(i);
                }
                i
            }
        };
        let name = p.instrument(inst).map(|i| i.name.clone()).unwrap_or_default();
        let new_clef = p.instrument(inst).map(|i| i.clef.split('+').next().unwrap_or("treble").to_string()).unwrap_or_default();
        let first_clef = |p: &Project, l: &Lane| l.changes.first().and_then(|c| p.instrument(c.instrument)).map(|i| i.clef.split('+').next().unwrap_or("treble").to_string()).unwrap_or_default();
        let lanes: Vec<usize> = p.movements[mi].lanes.iter().enumerate().filter(|(_, l)| l.player == player).map(|(i, _)| i).collect();
        // The player's first staff: the clef in force here, before the change.
        let first_lane = lanes[0];
        let clef_before = {
            let l = &p.movements[mi].lanes[first_lane];
            l.clefs.iter().rfind(|c| c.at <= target).map(|c| c.clef.clone()).unwrap_or_else(|| first_clef(p, l))
        };
        let mut ids = Ids(p.next_id);
        for &li in &lanes {
            let l = &mut p.movements[mi].lanes[li];
            if target == Frac::ZERO {
                match l.changes.first_mut() {
                    Some(c) => c.instrument = inst,
                    None => l.changes.push(InstrumentChange { id: ids.next(), at: Frac::ZERO, instrument: inst }),
                }
            } else {
                l.changes.retain(|c| c.at != target);
                l.changes.push(InstrumentChange { id: ids.next(), at: target, instrument: inst });
                l.changes.sort_by_key(|c| c.at);
            }
        }
        if target > Frac::ZERO && new_clef != clef_before && !new_clef.is_empty() {
            let l = &mut p.movements[mi].lanes[first_lane];
            l.clefs.retain(|c| c.at != target);
            l.clefs.push(ClefChange { id: ids.next(), at: target, clef: new_clef });
            l.clefs.sort_by_key(|c| c.at);
        }
        p.next_id = ids.0;
        self.commit(before, "Instrument change");
        let m = self.movement().unwrap();
        self.status = format!("{name} · bar {}", edit::bar_number(m, target.min(m.length)).0);
        Ok(())
    }

    /// Marks the project saved (the app calls this after writing the file).
    pub fn saved(&mut self) {
        self.modified = false;
    }
}
