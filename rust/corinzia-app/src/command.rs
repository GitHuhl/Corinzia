//! Text commands: every edit and query of a score as a line of text, with a
//! JSON answer, for scripts and AI agents (`--do`, `--script`, `--stdin`).
//!
//! Two general commands reach everything the interface can do:
//! * `act <action>` runs any keyboard or menu action, written as JSON
//!   (`act "Undo"`, `act {"Note":{"step":0,"chord":false}}`); `help actions`
//!   lists them;
//! * `popover <kind> <text>` types into any popover (`popover Key Eb`).
//!
//! Readable commands cover the common edits (`note`, `key`, `select bar`…),
//! and `get …` queries answer with JSON. `help` lists them all.

use crate::app::App;
use crate::editor::PopoverKind;
use crate::files;
use crate::keys::Action;
use corinzia_edit as edit;
use serde_json::{json, Value};
use std::path::PathBuf;

/// The readable commands: (usage, what it does).
pub(crate) const HELP: &[(&str, &str)] = &[
    ("help [actions|popovers]", "This list, the actions `act` takes, or the popover kinds"),
    ("act <action as JSON>", "Any keyboard or menu action: act \"Undo\", act {\"Duration\":2}"),
    ("popover <kind> <text>", "Type into a popover: popover Tempo \"Allegro 132\""),
    ("get project|selection|notes [bar A[-B]] [staff S]|chant|motifs|status", "Queries, answered with JSON"),
    ("open <file> | save [file] | export <file.pdf|mid|musicxml|wav>", "Files"),
    ("undo | redo", "Undo or redo the last edit"),
    ("goto <bar>|<mark>", "Go to a bar (goto 25) or a rehearsal mark (goto B): the bar selected on the caret's staff"),
    ("phrasing on|off", "Playback with the phrasing rules (phrase arches, final ritardando, accents, players' timing) or as written"),
    ("movement <n> | chant <n|off>", "Show movement n or chant n (from 1)"),
    ("select all | select none | select bar <a>[-<b>] [staff <s>[-<t>]]", "Select (bars as numbered in the score, staves from 1)"),
    ("input on|off", "Note input"),
    ("duration whole|half|quarter|eighth|16th|32nd|64th | dot", "The duration (of the selection, or for input)"),
    ("note <A-G> [chord] | rest", "Enter a note (at the caret, in note input) or a rest"),
    ("pitch <A-G> | sharp | flat | natural", "Change the selected notes"),
    ("key|time|tempo|clef|barline|text|technique|expression|dynamic|ending|rehearsal|ornament|tuplet|instrument|transpose|lyrics|chord <text>", "The popover of that name: key Eb, time 6/8, transpose \"up M2\""),
    ("title|composer <text>", "The score's title or composer"),
    ("motif add \"<name>\" [as <role>,<role>…] [tags <tag>,<tag>…] | motif delete <n>", "The passage selected on one staff into the motif bank, named and tagged (roles: melody, countermelody, bass, inner voice, pad, accompaniment, arpeggio, ostinato, riff, hook, call, response, fill, groove, pedal, intro, cadence, transition); or remove motif n (from 1). get motifs lists the bank"),
    ("panel notes|structure|motifs | tab score|mix|explore", "The side panel's page, or the tab shown"),
    ("neume <name> | mode <text> | syllables <text>", "In a chant: a neume at the caret (neume Oligon), the mode (mode \"πλ. β΄\"), syllables from the caret"),
];

/// A command's arguments: words, with "double quotes" keeping spaces.
fn words(s: &str) -> Vec<String> {
    let mut out = vec![];
    let mut cur = String::new();
    let (mut quoted, mut any) = (false, false);
    for c in s.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                any = true;
            }
            c if c.is_whitespace() && !quoted => {
                if any || !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
                any = false;
            }
            c => cur.push(c),
        }
    }
    if any || !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// "3" or "3-5" (from 1).
fn span(s: &str) -> Result<(usize, usize), String> {
    let n = |t: &str| t.trim().parse::<usize>().map_err(|_| format!("Not a number: {t}"));
    match s.split_once('-') {
        Some((a, b)) => Ok((n(a)?, n(b)?)),
        None => Ok((n(s)?, n(s)?)),
    }
}

/// A name matched to a serde enum variant, ignoring case.
fn variant<T: serde::de::DeserializeOwned>(name: &str, all: &[&str]) -> Option<T> {
    let found = all.iter().find(|v| v.eq_ignore_ascii_case(name))?;
    serde_json::from_value(Value::String(found.to_string())).ok()
}

const POPOVERS: &[&str] = &[
    "Key", "Time", "Clef", "Barline", "Tempo", "Expression", "Technique", "Ending", "Rehearsal", "Ornament", "Lyrics", "Chord", "Tuplet", "MovementTitle", "Fingering", "Dynamic", "Title", "Composer", "Instrument", "Transpose", "Filter", "ChantLyric", "ChantMode", "ChantTitle", "GoTo",
];
const NEUMES: &[&str] = &[
    "Ison", "Oligon", "Petasti", "Kentimata", "Kentima", "OligonKentimaAbove", "OligonYpsiliRight", "OligonYpsiliLeft", "Apostrofos", "Elafron", "ElafronApostrofos", "Chamili", "Yporroi", "OligonKentimataBelow", "OligonIsonKentimata",
];

impl App {
    /// Runs one command line; the answer as JSON (`null` for plain edits).
    pub(crate) fn command(&mut self, line: &str) -> Result<Value, String> {
        let line = line.trim();
        let (verb, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
        let rest = rest.trim();
        let args = words(rest);
        let arg = |i: usize| args.get(i).map(String::as_str).unwrap_or("");
        let text = || args.join(" ");
        let lower = verb.to_lowercase();
        if let Some(r) = self.mix_command(&lower, &args, rest) {
            return r;
        }
        let answer = match lower.as_str() {
            "" => return Ok(Value::Null),
            "help" => match arg(0) {
                "actions" => json!({"actions": "Every variant of Action, as serde JSON: a name (\"Undo\") or {\"Name\": value}. See rust/corinzia-app/src/keys.rs."}),
                "popovers" => json!(POPOVERS),
                _ => json!(HELP.iter().chain(crate::command_mix::HELP).map(|(u, w)| json!({"command": u, "does": w})).collect::<Vec<_>>()),
            },
            "act" => {
                let action: Action = serde_json::from_str(rest).map_err(|e| format!("Not an action ({e}): try act \"Undo\" or act {{\"Duration\":2}}"))?;
                self.run_action(action);
                Value::Null
            }
            "popover" => {
                let kind: PopoverKind = variant(arg(0), POPOVERS).ok_or_else(|| format!("No popover {}: one of {}", arg(0), POPOVERS.join(", ")))?;
                let t = args[1..].join(" ");
                self.apply_popover(kind, &t)?;
                Value::Null
            }
            "get" => return self.query(&args),
            "goto" | "go" => {
                let target = if arg(0) == "to" { arg(1).to_string() } else { text() };
                self.editor.apply_popover(PopoverKind::GoTo, &target)?;
                self.show_caret();
                self.sync();
                Value::Null
            }
            "phrasing" => match arg(0) {
                "on" => self.run_action(Action::Phrasing(true)),
                "off" => self.run_action(Action::Phrasing(false)),
                _ => return Err("phrasing on|off".into()),
            },
            "motif" | "motifs" => match arg(0) {
                "add" => {
                    let mut m = self.editor.motif_from_selection()?;
                    let mut i = 1;
                    if !arg(1).is_empty() && arg(1) != "as" && arg(1) != "tags" {
                        m.name = arg(1).to_string();
                        i = 2;
                    }
                    while i < args.len() {
                        let list = || args.get(i + 1).map(|v| v.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect::<Vec<_>>()).unwrap_or_default();
                        match arg(i) {
                            "as" => {
                                for r in list() {
                                    let role = corinzia_model::motif::Role::parse(&r).ok_or_else(|| format!("No role {r}: see help"))?;
                                    if !m.roles.contains(&role) {
                                        m.roles.push(role);
                                    }
                                }
                            }
                            "tags" => m.tags = list(),
                            other => return Err(format!("Unexpected {other}: motif add \"name\" as melody,call tags lyrical")),
                        }
                        i += 2;
                    }
                    let id = self.editor.save_motif(m);
                    json!({"motif": id})
                }
                "delete" => {
                    let n = span(arg(1))?.0;
                    let id = self.editor.project.motifs.get(n.wrapping_sub(1)).map(|m| m.id).ok_or_else(|| format!("No motif {n}: there are {}", self.editor.project.motifs.len()))?;
                    self.editor.delete_motif(id);
                    Value::Null
                }
                _ => return Err("motif add \"name\" [as roles] [tags tags] | motif delete <n>".into()),
            },
            "panel" => {
                self.panel = match arg(0) {
                    "notes" => crate::panels::Panel::Notes,
                    "structure" => crate::panels::Panel::Structure,
                    "motifs" => crate::panels::Panel::Motifs,
                    _ => return Err("panel notes|structure|motifs".into()),
                };
                Value::Null
            }
            "tab" => match arg(0) {
                "score" => self.run_action(Action::ShowScore),
                "mix" => self.run_action(Action::ShowMix),
                "explore" => self.run_action(Action::ShowExplore),
                _ => return Err("tab score|mix|explore".into()),
            },
            "undo" => self.run_action(Action::Undo),
            "redo" => self.run_action(Action::Redo),
            "movement" => {
                let n = span(arg(0))?.0;
                if n == 0 || n > self.editor.project.movements.len() {
                    return Err(format!("No movement {n}: there are {}", self.editor.project.movements.len()));
                }
                self.run_action(Action::Movement(n - 1))
            }
            "chant" => {
                if arg(0) == "off" {
                    self.run_action(Action::Chant(None))
                } else {
                    let n = span(arg(0))?.0;
                    if n == 0 || n > self.editor.project.chants.len() {
                        return Err(format!("No chant {n}: there are {}", self.editor.project.chants.len()));
                    }
                    self.run_action(Action::Chant(Some(n - 1)))
                }
            }
            "select" => match (arg(0), arg(1)) {
                ("all", _) => self.run_action(Action::SelectAll),
                ("none", _) => {
                    self.editor.clear_selection();
                    self.sync();
                    Value::Null
                }
                ("bar" | "bars", b) => {
                    let (first, last) = span(b)?;
                    let lanes = match (arg(2), arg(3)) {
                        ("staff" | "staves", s) => {
                            let (a, b) = span(s)?;
                            (a.max(1) - 1)..=(b.max(1) - 1)
                        }
                        _ => 0..=self.editor.movement().map_or(0, |m| m.lanes.len().saturating_sub(1)),
                    };
                    self.editor.select_bars(first, last, lanes)?;
                    self.sync();
                    Value::Null
                }
                _ => return Err("select all | select none | select bar <a>[-<b>] [staff <s>[-<t>]]".into()),
            },
            "input" => self.run_action(Action::SetInput(arg(0) != "off")),
            "duration" => {
                const NAMES: [&str; 7] = ["whole", "half", "quarter", "eighth", "16th", "32nd", "64th"];
                let d = NAMES.iter().position(|n| n.eq_ignore_ascii_case(arg(0))).ok_or_else(|| format!("Duration: one of {}", NAMES.join(", ")))?;
                self.run_action(Action::Duration(d as u8))
            }
            "dot" => self.run_action(Action::Dots),
            "rest" => self.run_action(Action::Rest),
            "note" | "pitch" => {
                let step = "CDEFGAB".find(arg(0).to_ascii_uppercase().as_str()).filter(|_| arg(0).len() == 1).ok_or("A letter, A–G")? as i32;
                if verb.eq_ignore_ascii_case("pitch") {
                    self.run_action(Action::Pitch(step))
                } else {
                    if !self.editor.input_mode {
                        self.run_action(Action::SetInput(true));
                    }
                    self.run_action(Action::Note { step, chord: arg(1) == "chord" })
                }
            }
            "sharp" => self.run_action(Action::Accidental(1)),
            "flat" => self.run_action(Action::Accidental(-1)),
            "natural" => self.run_action(Action::Accidental(0)),
            v @ ("key" | "time" | "tempo" | "clef" | "barline" | "text" | "technique" | "expression" | "dynamic" | "ending" | "rehearsal" | "ornament" | "tuplet" | "instrument" | "transpose" | "lyrics" | "chord" | "title" | "composer" | "fingering") => {
                let name = match v {
                    "text" => "Expression",
                    other => other,
                };
                let kind: PopoverKind = variant(name, POPOVERS).ok_or("No such popover")?;
                self.apply_popover(kind, &text())?;
                Value::Null
            }
            "neume" => {
                let q: corinzia_byz::Quantity = variant(arg(0), NEUMES).ok_or_else(|| format!("Neume: one of {}", NEUMES.join(", ")))?;
                if self.editor.chant.is_none() {
                    return Err("Show a chant first: chant 1".into());
                }
                self.run_action(Action::ChantNeume(q))
            }
            "mode" => {
                self.apply_popover(PopoverKind::ChantMode, &text())?;
                Value::Null
            }
            "syllables" => {
                for w in words(rest) {
                    self.editor.chant_lyric_next(&w)?;
                }
                self.sync();
                Value::Null
            }
            "open" => {
                let path = PathBuf::from(text());
                self.open(path);
                if !self.editor.status.starts_with("Opened") && !self.editor.status.starts_with("Imported") {
                    return Err(self.editor.status.clone());
                }
                Value::Null
            }
            "save" => {
                let path = if args.is_empty() { self.path.clone().ok_or("save <file>: the score has no file yet")? } else { PathBuf::from(text()) };
                self.save(path);
                if !self.editor.status.starts_with("Saved") {
                    return Err(self.editor.status.clone());
                }
                Value::Null
            }
            "export" => {
                let path = PathBuf::from(text());
                let format = files::format_of(&path).ok_or("export: a .pdf, .mid, .musicxml or .wav file")?;
                if format == files::Format::Audio {
                    return Err("export: audio is rendered with --export file.wav".into());
                }
                self.export_to(format, path);
                if !self.editor.status.starts_with("Exported") {
                    return Err(self.editor.status.clone());
                }
                Value::Null
            }
            _ => return Err(format!("Unknown command: {verb} (try help)")),
        };
        Ok(answer)
    }

    /// An action as the keyboard runs it, then the views caught up.
    fn run_action(&mut self, a: Action) -> Value {
        let _ = self.act(a);
        self.sync();
        Value::Null
    }

    fn apply_popover(&mut self, kind: PopoverKind, text: &str) -> Result<(), String> {
        self.editor.apply_popover(kind, text)?;
        self.sync();
        Ok(())
    }

    /// `get …`
    fn query(&mut self, args: &[String]) -> Result<Value, String> {
        if let Some(r) = self.mix_query(args) {
            return r;
        }
        let ed = &self.editor;
        let p = &ed.project;
        let arg = |i: usize| args.get(i).map(String::as_str).unwrap_or("");
        Ok(match arg(0) {
            "motifs" => json!(p
                .motifs
                .iter()
                .enumerate()
                .map(|(i, m)| {
                    let facts: serde_json::Map<String, Value> = crate::motif_view::facts(p, m).into_iter().map(|(k, v)| (k.to_lowercase(), json!(v))).collect();
                    json!({"number": i + 1, "name": m.name, "roles": m.roles, "tags": m.tags, "comment": m.comment, "notes": m.notes.len(), "facts": facts})
                })
                .collect::<Vec<_>>()),
            "project" => json!({
                "title": p.title,
                "composer": p.composer,
                "file": self.path,
                "movements": p.movements.iter().enumerate().map(|(i, m)| json!({"number": i + 1, "title": m.title, "bars": edit::bar_starts(m).len(), "shown": i == ed.movement && ed.chant.is_none()})).collect::<Vec<_>>(),
                "players": p.players.iter().map(|pl| json!({"name": pl.name, "instruments": pl.instruments.iter().filter_map(|i| p.instrument(*i)).map(|i| i.name.clone()).collect::<Vec<_>>()})).collect::<Vec<_>>(),
                "chants": p.chants.iter().enumerate().map(|(i, c)| json!({"number": i + 1, "title": c.title, "mode": c.mode.echos.greek(), "shown": ed.chant == Some(i)})).collect::<Vec<_>>(),
                "phrasing": p.phrasing,
                "modified": ed.modified,
            }),
            "status" => json!({"status": ed.status, "at": ed.caret_label(), "input": ed.input_mode, "audio": {"late_blocks": self.playback.late_blocks(), "peak_load": self.playback.peak_load, "buffer": self.playback.max_frames()}}),
            "selection" => {
                let range = ed.range.and_then(|r| {
                    let m = ed.movement()?;
                    Some(json!({"staves": [r.lane_from + 1, r.lane_to + 1], "bars": [edit::bar_number(m, r.from).0, edit::bar_number(m, r.to - corinzia_score::Frac::new(1, 1024)).0]}))
                });
                json!({"at": ed.caret_label(), "element": ed.selection, "passage": range, "picked": ed.picked.len()})
            }
            "notes" => {
                let m = ed.movement().ok_or("No movement")?;
                let mut bars = (0usize, usize::MAX);
                let mut staves = (1usize, usize::MAX);
                let mut i = 1;
                while i < args.len() {
                    match (arg(i), args.get(i + 1)) {
                        ("bar" | "bars", Some(s)) => bars = span(s)?,
                        ("staff" | "staves", Some(s)) => staves = span(s)?,
                        (w, _) => return Err(format!("get notes [bar A[-B]] [staff S[-T]]: not {w}")),
                    }
                    i += 2;
                }
                let mut out = vec![];
                for (li, lane) in m.lanes.iter().enumerate().filter(|(li, _)| (staves.0..=staves.1).contains(&(li + 1))) {
                    let name = p.player(lane.player).map(|pl| pl.name.clone()).unwrap_or_default();
                    for v in &lane.voices {
                        for e in v.events.iter() {
                            let (bar, offset) = edit::bar_number(m, e.start);
                            if !(bars.0..=bars.1).contains(&bar) {
                                continue;
                            }
                            out.push(json!({
                                "staff": li + 1,
                                "player": name,
                                "voice": v.index,
                                "bar": bar,
                                "beat": 1.0 + offset.num as f64 * 4.0 / offset.den as f64,
                                "quarters": e.duration.num as f64 * 4.0 / e.duration.den as f64,
                                "rest": e.is_rest(),
                                "pitches": e.heads.iter().map(|h| edit::names::pitch_name(h.pitch)).collect::<Vec<_>>(),
                                "id": e.id,
                            }));
                        }
                    }
                }
                Value::Array(out)
            }
            "chant" => {
                let c = ed.shown_chant().ok_or("No chant shown: chant 1")?;
                let (sung, problems) = c.sing();
                json!({
                    "title": c.title,
                    "mode": c.mode.echos.greek(),
                    "caret": ed.chant_caret,
                    "elements": c.elements,
                    "sung": sung.iter().map(|s| json!({"element": s.element, "note": s.note.label(), "moria": s.moria, "beat": s.start.num as f64 / s.start.den as f64, "beats": s.length.num as f64 / s.length.den as f64, "syllable": s.lyric})).collect::<Vec<_>>(),
                    "problems": problems.iter().map(|p| format!("{p:?}")).collect::<Vec<_>>(),
                })
            }
            w => return Err(format!("get project|selection|notes|chant|status|mixer|plugins|params: not {w}")),
        })
    }

    /// Runs commands, printing one JSON line each: {"ok":true,"result":…}
    /// or {"ok":false,"error":…}. With `stop`, the first failure ends it.
    /// Whether all succeeded.
    pub(crate) fn run_commands(&mut self, lines: impl IntoIterator<Item = String>, stop: bool) -> bool {
        use std::io::Write;
        let mut ok = true;
        for line in lines {
            let l = line.trim();
            if l.is_empty() || l.starts_with('#') {
                continue;
            }
            let out = match self.command(l) {
                Ok(v) => json!({"ok": true, "command": l, "result": v}),
                Err(e) => {
                    ok = false;
                    json!({"ok": false, "command": l, "error": e})
                }
            };
            println!("{out}");
            let _ = std::io::stdout().flush();
            if !ok && stop {
                break;
            }
        }
        ok
    }
}
