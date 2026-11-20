//! Text commands for the Mix tab: strips, plugins, effects, buses, sends,
//! automation and audio tracks, by name. `mix <message>` runs any Mix tab
//! message as JSON for everything else.
//!
//! A strip is named by its track's or bus's name (any case), `master`, or a
//! track's number from 1.

use crate::app::App;
use crate::editor::MASTER;
use crate::mix::MixMsg;
use corinzia_edit as edit;
use corinzia_model::{AutoTarget, Id};
use serde_json::{json, Value};

/// The Mix tab's commands, for `help`.
pub(crate) const HELP: &[(&str, &str)] = &[
    ("get mixer", "Every strip: tracks, buses and the master, with their instruments, effects, levels, outputs and sends"),
    ("get plugins [instruments|effects] | get params <strip> <effect n>", "The plugins installed, and an effect's parameters"),
    ("gain <strip> <dB> | pan <strip> <-1..1, L30, R30, C> | mute|solo <strip> [on|off]", "A strip's fader, pan, mute and solo"),
    ("plugin <track> <plugin>", "The track's instrument plugin (\"Corinzia Ensemble\" is the built-in one)"),
    ("effect add <strip> <plugin> | effect remove|bypass <strip> <n> | effect move <strip> <n> up|down", "A strip's effects, numbered from 1"),
    ("param <strip> <effect n> <parameter> <value>", "An effect's parameter, by name or id"),
    ("bus add <name> | bus remove <name> | output <strip> <master|bus>", "Buses and where strips go"),
    ("send <track> <bus> [<dB>] [pre|post] | send <track> <bus> off", "A send to a bus"),
    ("program <track> <label> | color <track> <#rrggbb|none> | folder <track> <name>", "A track's MIDI program, colour and folder"),
    ("automate <strip> gain|pan|bend|cc <n>|send <bus>|param <effect n> <parameter> at <bar>[:<beat>] <value> | automate <…> clear", "Automation points (bar as numbered, beat from 1)"),
    ("audio track <name> | audio add <track> <file>", "An audio track; a sound file at the caret (select a bar first)"),
    ("mix <message as JSON>", "Any Mix tab message: mix {\"Gain\":[12,-6.0]} (strips by id: see get mixer)"),
];

impl App {
    /// Strip by name: a track, a bus, `master`, or a track's number.
    fn strip_named(&self, name: &str) -> Result<Id, String> {
        let p = &self.editor.project;
        if name.eq_ignore_ascii_case("master") {
            return Ok(MASTER);
        }
        if let Ok(n) = name.parse::<usize>() {
            return p.tracks.get(n.wrapping_sub(1)).map(|t| t.id).ok_or(format!("No track {n}: there are {}", p.tracks.len()));
        }
        p.tracks
            .iter()
            .find(|t| t.name.eq_ignore_ascii_case(name))
            .map(|t| t.id)
            .or_else(|| p.buses.iter().find(|b| b.name.eq_ignore_ascii_case(name)).map(|b| b.id))
            .ok_or_else(|| {
                let names: Vec<&str> = p.tracks.iter().map(|t| t.name.as_str()).chain(p.buses.iter().map(|b| b.name.as_str())).collect();
                format!("No strip {name}: one of {}, master", names.join(", "))
            })
    }

    fn track_named(&self, name: &str) -> Result<Id, String> {
        let id = self.strip_named(name)?;
        self.editor.project.track(id).map(|t| t.id).ok_or(format!("{name} is not a track"))
    }

    fn bus_named(&self, name: &str) -> Result<Id, String> {
        let p = &self.editor.project;
        p.buses.iter().find(|b| b.name.eq_ignore_ascii_case(name)).map(|b| b.id).ok_or_else(|| format!("No bus {name}: one of {}", p.buses.iter().map(|b| b.name.as_str()).collect::<Vec<_>>().join(", ")))
    }

    /// A strip's effect chain.
    fn inserts_of(&self, strip: Id) -> Vec<Id> {
        let p = &self.editor.project;
        if strip == MASTER {
            p.master.inserts.clone()
        } else if let Some(t) = p.track(strip) {
            t.inserts.clone()
        } else {
            p.bus(strip).map(|b| b.inserts.clone()).unwrap_or_default()
        }
    }

    /// Effect `n` (from 1) of a strip: its node.
    fn effect_of(&self, strip: Id, n: &str) -> Result<Id, String> {
        let chain = self.inserts_of(strip);
        let k: usize = n.parse().map_err(|_| format!("An effect's number, from 1: not {n}"))?;
        chain.get(k.wrapping_sub(1)).copied().ok_or(format!("No effect {k}: the strip has {}", chain.len()))
    }

    /// A plugin installed, by name (any case): its index in the scan.
    fn plugin_named(&mut self, name: &str, instrument: bool) -> Result<usize, String> {
        if self.playback.plugins.is_empty() {
            self.playback.scan();
        }
        let list = &self.playback.plugins;
        list.iter()
            .position(|p| p.name.eq_ignore_ascii_case(name) && p.is_instrument() == instrument)
            .or_else(|| list.iter().position(|p| p.name.eq_ignore_ascii_case(name)))
            .ok_or_else(|| {
                let names: Vec<&str> = list.iter().filter(|p| p.is_instrument() == instrument).map(|p| p.name.as_str()).collect();
                format!("No plugin {name}: one of {}", names.join(", "))
            })
    }

    /// A parameter of a node's plugin, by name (any case) or id.
    fn param_named(&self, node: Id, name: &str) -> Result<(u32, f64, f64), String> {
        let pl = self.playback.pool.effect(node).ok_or("The effect is not loaded")?;
        let params = pl.params();
        params
            .iter()
            .find(|p| p.name.eq_ignore_ascii_case(name) || p.id.to_string() == name)
            .map(|p| (p.id, p.min, p.max))
            .ok_or_else(|| format!("No parameter {name}: one of {}", params.iter().map(|p| p.name.as_str()).collect::<Vec<_>>().join(", ")))
    }

    fn mix_now(&mut self, m: MixMsg) {
        self.mix(m);
        self.editor.end_gesture();
        self.sync();
    }

    /// A Mix tab command, if `verb` is one: `None` when it is not.
    pub(crate) fn mix_command(&mut self, verb: &str, args: &[String], rest: &str) -> Option<Result<Value, String>> {
        let arg = |i: usize| args.get(i).map(String::as_str).unwrap_or("");
        let run = |app: &mut App| -> Result<Value, String> {
            match verb {
                "mix" => {
                    let m: MixMsg = serde_json::from_str(rest).map_err(|e| format!("Not a Mix tab message ({e})"))?;
                    app.mix_now(m);
                }
                "gain" => {
                    let s = app.strip_named(arg(0))?;
                    let db: f64 = arg(1).trim_end_matches("dB").trim_end_matches("db").parse().map_err(|_| "gain <strip> <dB>")?;
                    app.mix_now(MixMsg::Gain(s, db.clamp(-60.0, 12.0)));
                }
                "pan" => {
                    let s = app.strip_named(arg(0))?;
                    let t = arg(1).to_ascii_uppercase();
                    let pan = match t.as_str() {
                        "C" | "CENTER" | "CENTRE" => 0.0,
                        _ if t.starts_with('L') => -t[1..].parse::<f64>().map_err(|_| "pan: L30")? / 100.0,
                        _ if t.starts_with('R') => t[1..].parse::<f64>().map_err(|_| "pan: R30")? / 100.0,
                        _ => t.parse::<f64>().map_err(|_| "pan <strip> <-1..1, L30, R30, C>")?,
                    };
                    app.mix_now(MixMsg::Pan(s, pan.clamp(-1.0, 1.0)));
                }
                "mute" | "solo" => {
                    let s = app.strip_named(arg(0))?;
                    let p = &app.editor.project;
                    let now = match (p.track(s), p.bus(s), verb) {
                        (Some(t), _, "mute") => t.mute,
                        (Some(t), _, _) => t.solo,
                        (_, Some(b), "mute") => b.mute,
                        (_, Some(b), _) => b.solo,
                        _ => return Err(format!("The master has no {verb}")),
                    };
                    let want = match arg(1) {
                        "on" => true,
                        "off" => false,
                        _ => !now,
                    };
                    if want != now {
                        app.mix_now(if verb == "mute" { MixMsg::Mute(s) } else { MixMsg::Solo(s) });
                    }
                }
                "plugin" => {
                    let t = app.track_named(arg(0))?;
                    let i = app.plugin_named(&args[1..].join(" "), true)?;
                    app.mix_now(MixMsg::Assign(t, i));
                }
                "effect" => {
                    let s = app.strip_named(arg(1))?;
                    match arg(0) {
                        "add" => {
                            let i = app.plugin_named(&args[2..].join(" "), false)?;
                            app.mix_now(MixMsg::AddEffect(s, i));
                        }
                        "remove" => {
                            let n = app.effect_of(s, arg(2))?;
                            app.mix_now(MixMsg::RemoveEffect(s, n));
                        }
                        "bypass" => {
                            let n = app.effect_of(s, arg(2))?;
                            app.mix_now(MixMsg::Bypass(n));
                        }
                        "move" => {
                            let n = app.effect_of(s, arg(2))?;
                            let by = match arg(3) {
                                "up" => -1,
                                "down" => 1,
                                _ => return Err("effect move <strip> <n> up|down".into()),
                            };
                            app.mix_now(MixMsg::MoveEffect(s, n, by));
                        }
                        _ => return Err("effect add|remove|bypass|move …".into()),
                    }
                }
                "param" => {
                    let s = app.strip_named(arg(0))?;
                    let n = app.effect_of(s, arg(1))?;
                    app.sync(); // the effect is loaded with the plan
                    let (id, min, max) = app.param_named(n, arg(2))?;
                    let v: f64 = arg(3).parse().map_err(|_| "param <strip> <effect n> <parameter> <value>")?;
                    app.mix_now(MixMsg::Param(n, id, v.clamp(min, max)));
                }
                "bus" => match arg(0) {
                    "add" => {
                        let name = args[1..].join(" ");
                        let id = app.editor.add_bus(&name);
                        app.mix.strip = Some(id);
                        app.sync();
                        return Ok(json!({"bus": id}));
                    }
                    "remove" => {
                        let b = app.bus_named(&args[1..].join(" "))?;
                        app.editor.remove_bus(b);
                        app.sync();
                    }
                    _ => return Err("bus add <name> | bus remove <name>".into()),
                },
                "output" => {
                    let s = app.strip_named(arg(0))?;
                    let to = args[1..].join(" ");
                    let label = if to.eq_ignore_ascii_case("master") { "Master".to_string() } else { app.editor.project.bus(app.bus_named(&to)?).map(|b| b.name.clone()).unwrap_or_default() };
                    if app.editor.project.bus(s).is_some() {
                        app.mix_now(MixMsg::BusOutput(s, label));
                    } else {
                        app.mix_now(MixMsg::Output(s, label));
                    }
                }
                "send" => {
                    let t = app.track_named(arg(0))?;
                    let b = app.bus_named(arg(1))?;
                    let index = |app: &App| app.editor.project.track(t).and_then(|x| x.sends.iter().position(|s| s.bus == b));
                    if arg(2) == "off" {
                        if index(app).is_some() {
                            app.mix_now(MixMsg::ToggleSend(t, b));
                        }
                    } else {
                        if index(app).is_none() {
                            app.mix_now(MixMsg::ToggleSend(t, b));
                        }
                        let i = index(app).ok_or("The send could not be made")?;
                        for w in &args[2..] {
                            match w.as_str() {
                                "pre" | "post" => {
                                    let pre = app.editor.project.track(t).map_or(false, |x| x.sends[i].pre);
                                    if pre != (w == "pre") {
                                        app.mix_now(MixMsg::SendPre(t, i));
                                    }
                                }
                                db => {
                                    let db: f64 = db.trim_end_matches("dB").parse().map_err(|_| "send <track> <bus> [<dB>] [pre|post]")?;
                                    app.mix_now(MixMsg::SendLevel(t, i, db.clamp(-60.0, 12.0)));
                                }
                            }
                        }
                    }
                }
                "program" => {
                    let t = app.track_named(arg(0))?;
                    app.mix_now(MixMsg::Program(t, args[1..].join(" ")));
                }
                "color" | "colour" => {
                    let t = app.track_named(arg(0))?;
                    let c = match arg(1) {
                        "none" => None,
                        hex => Some(u32::from_str_radix(hex.trim_start_matches('#'), 16).map_err(|_| "color <track> <#rrggbb|none>")?),
                    };
                    app.mix_now(MixMsg::Color(t, c));
                }
                "folder" => {
                    let t = app.track_named(arg(0))?;
                    app.mix_now(MixMsg::Folder(t, args[1..].join(" ")));
                }
                "automate" => return app.automate(args),
                "audio" => match arg(0) {
                    "track" => {
                        let name = if args.len() > 1 { args[1..].join(" ") } else { format!("Audio {}", app.editor.project.tracks.iter().filter(|t| t.audio).count() + 1) };
                        let id = app.editor.add_audio_track(&name);
                        app.sync();
                        return Ok(json!({"track": id}));
                    }
                    "add" => {
                        let t = app.track_named(arg(1))?;
                        let file = std::path::PathBuf::from(args[2..].join(" "));
                        let clips = |app: &App| app.editor.project.track(t).map_or(0, |x| x.clips.len());
                        let before = clips(app);
                        app.add_audio_file(Some(t), &file, None);
                        app.sync();
                        if clips(app) == before {
                            return Err(app.editor.status.clone());
                        }
                    }
                    _ => return Err("audio track <name> | audio add <track> <file>".into()),
                },
                _ => unreachable!(),
            }
            Ok(Value::Null)
        };
        match verb {
            "mix" | "plugin" | "gain" | "pan" | "mute" | "solo" | "effect" | "param" | "bus" | "output" | "send" | "program" | "color" | "colour" | "folder" | "automate" | "audio" => Some(run(self)),
            _ => None,
        }
    }

    /// `automate <strip> <target…> at <bar>[:<beat>] <value>` or `… clear`.
    fn automate(&mut self, args: &[String]) -> Result<Value, String> {
        let arg = |i: usize| args.get(i).map(String::as_str).unwrap_or("");
        let s = self.strip_named(arg(0))?;
        let (target, used) = match arg(1) {
            "gain" => (AutoTarget::Gain(s), 2),
            "pan" => (AutoTarget::Pan(s), 2),
            "bend" => (AutoTarget::PitchBend(self.track_named(arg(0))?), 2),
            "cc" => (AutoTarget::Cc { track: self.track_named(arg(0))?, cc: arg(2).parse().map_err(|_| "automate <track> cc <0-127> …")? }, 3),
            "send" => (AutoTarget::Send { track: self.track_named(arg(0))?, bus: self.bus_named(arg(2))? }, 3),
            "param" => {
                let node = self.effect_of(s, arg(2))?;
                self.sync();
                (AutoTarget::Param { node, param: self.param_named(node, arg(3))?.0 }, 4)
            }
            _ => return Err("automate <strip> gain|pan|bend|cc <n>|send <bus>|param <effect n> <parameter> …".into()),
        };
        if arg(used) == "clear" {
            while self.editor.auto_lane(target).is_some_and(|l| !l.points.is_empty()) {
                self.editor.delete_auto_point(target, 0);
            }
            self.sync();
            return Ok(Value::Null);
        }
        if arg(used) != "at" {
            return Err("… at <bar>[:<beat>] <value> | … clear".into());
        }
        let m = self.editor.movement().ok_or("No movement")?;
        let (bar, beat) = match arg(used + 1).split_once(':') {
            Some((b, q)) => (b.parse::<usize>().map_err(|_| "at <bar>[:<beat>]")?, q.parse::<f64>().map_err(|_| "at <bar>:<beat>")?),
            None => (arg(used + 1).parse::<usize>().map_err(|_| "at <bar>[:<beat>]")?, 1.0),
        };
        let starts = edit::bar_starts(m);
        let base = if edit::pickup(m).is_some() { 0 } else { 1 };
        let start = bar.checked_sub(base).and_then(|i| starts.get(i)).copied().ok_or(format!("No bar {bar}"))?;
        let at = start + corinzia_score::Frac::new(((beat - 1.0).max(0.0) * 64.0).round() as i64, 256);
        let value: f64 = arg(used + 2).trim_end_matches("dB").parse().map_err(|_| "… at <bar>[:<beat>] <value>")?;
        let index = self.editor.add_auto_point(target, at, value).ok_or("The point could not be added")?;
        self.sync();
        Ok(json!({"point": index}))
    }

    /// `get mixer`, `get plugins`, `get params`.
    pub(crate) fn mix_query(&mut self, args: &[String]) -> Option<Result<Value, String>> {
        let arg = |i: usize| args.get(i).map(String::as_str).unwrap_or("");
        match arg(0) {
            "mixer" => {
                let p = &self.editor.project;
                let fx = |chain: &[Id]| chain.iter().enumerate().map(|(k, n)| {
                    let node = p.node(*n);
                    json!({"number": k + 1, "name": node.map(|x| x.name.clone()), "bypassed": node.is_some_and(|x| x.bypassed), "id": n})
                }).collect::<Vec<_>>();
                let bus_name = |b: Option<Id>| b.and_then(|b| p.bus(b)).map_or("Master".to_string(), |b| b.name.clone());
                Some(Ok(json!({
                    "tracks": p.tracks.iter().enumerate().map(|(i, t)| json!({
                        "number": i + 1, "name": t.name, "id": t.id, "audio": t.audio,
                        "instrument": if t.audio { None } else { Some(t.instrument_node.and_then(|n| p.node(n)).map_or("Corinzia Ensemble".to_string(), |n| n.name.clone())) },
                        "gain_db": t.gain_db, "pan": t.pan, "mute": t.mute, "solo": t.solo,
                        "output": bus_name(t.output_bus),
                        "sends": t.sends.iter().map(|s| json!({"bus": bus_name(Some(s.bus)), "db": s.gain_db, "pre": s.pre})).collect::<Vec<_>>(),
                        "effects": fx(&t.inserts),
                        "program": t.program, "clips": t.clips.len(), "folder": t.folder,
                    })).collect::<Vec<_>>(),
                    "buses": p.buses.iter().map(|b| json!({"name": b.name, "id": b.id, "gain_db": b.gain_db, "pan": b.pan, "mute": b.mute, "solo": b.solo, "output": bus_name(b.output), "effects": fx(&b.inserts)})).collect::<Vec<_>>(),
                    "master": {"gain_db": p.master.gain_db, "effects": fx(&p.master.inserts)},
                    "automation": self.editor.movement().map(|m| m.automation.iter().map(|l| json!({"target": format!("{:?}", l.target), "points": l.points.len()})).collect::<Vec<_>>()),
                })))
            }
            "plugins" => {
                if self.playback.plugins.is_empty() {
                    self.playback.scan();
                }
                let want = arg(1);
                Some(Ok(Value::Array(
                    self.playback
                        .plugins
                        .iter()
                        .filter(|p| match want {
                            "instruments" => p.is_instrument(),
                            "effects" => !p.is_instrument(),
                            _ => true,
                        })
                        .map(|p| json!({"name": p.name, "vendor": p.vendor, "instrument": p.is_instrument(), "format": format!("{:?}", p.format)}))
                        .collect(),
                )))
            }
            "params" => Some((|| {
                let s = self.strip_named(arg(1))?;
                let n = self.effect_of(s, arg(2))?;
                self.sync();
                let pl = self.playback.pool.effect(n).ok_or("The effect is not loaded")?;
                Ok(Value::Array(pl.params().iter().map(|q| json!({"name": q.name, "id": q.id, "min": q.min, "max": q.max, "value": pl.param_value(q.id)})).collect()))
            })()),
            _ => None,
        }
    }
}
