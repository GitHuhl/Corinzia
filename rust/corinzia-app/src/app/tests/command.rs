//! Text commands (`--do`, `--script`, `--stdin`).

use super::*;
use serde_json::Value;

fn ok(a: &mut App, line: &str) -> Value {
    a.command(line).unwrap_or_else(|e| panic!("{line}: {e}"))
}

#[test]
fn queries_answer_with_json() {
    let mut a = app(View::Page);
    let p = ok(&mut a, "get project");
    assert_eq!(p["title"], "Serenade (demo)");
    assert_eq!(p["players"][0]["name"], "Player 1");
    assert_eq!(p["chants"][0]["mode"], "Ἦχος α΄");
    let notes = ok(&mut a, "get notes bar 1 staff 1");
    let first = &notes.as_array().unwrap()[0];
    assert_eq!((first["bar"].as_u64(), first["beat"].as_f64(), first["staff"].as_u64()), (Some(1), Some(1.0), Some(1)));
    assert!(notes.as_array().unwrap().iter().all(|n| n["bar"] == 1 && n["staff"] == 1));
    assert!(ok(&mut a, "help").as_array().unwrap().len() > 10);
    assert!(a.command("get nothing").is_err());
}

#[test]
fn edits_by_name_by_action_and_by_popover() {
    let mut a = app(View::Page);
    let before = ok(&mut a, "get notes bar 1 staff 1");
    let pitch = |v: &Value| v.as_array().unwrap().iter().find(|n| n["rest"] == false).unwrap()["pitches"][0].clone();
    ok(&mut a, "select bar 1 staff 1");
    assert_eq!(ok(&mut a, "get selection")["passage"]["bars"], serde_json::json!([1, 1]));
    ok(&mut a, "transpose \"up P8\"");
    let up = ok(&mut a, "get notes bar 1 staff 1");
    let (p0, p1) = (pitch(&before), pitch(&up));
    assert_eq!(p0.as_str().unwrap()[..p0.as_str().unwrap().len() - 1], p1.as_str().unwrap()[..p1.as_str().unwrap().len() - 1], "the same letter");
    assert_ne!(p0, p1, "an octave up");
    ok(&mut a, "undo");
    assert_eq!(pitch(&ok(&mut a, "get notes bar 1 staff 1")), p0);
    // Any action as JSON, any popover by kind.
    ok(&mut a, "act \"Redo\"");
    assert_eq!(pitch(&ok(&mut a, "get notes bar 1 staff 1")), p1);
    ok(&mut a, "popover Title \"Night Music\"");
    assert_eq!(a.editor.project.title, "Night Music");
    ok(&mut a, "composer Anon.");
    assert_eq!(a.editor.project.composer, "Anon.");
    // Errors say what went wrong, and change nothing.
    assert!(a.command("select bar 99").unwrap_err().contains("No bar 99"));
    assert!(a.command("act \"NoSuchThing\"").is_err());
    assert!(a.command("key Q#").is_err());
    assert!(a.command("frobnicate").unwrap_err().contains("Unknown command"));
}

#[test]
fn notes_are_entered_and_files_saved_and_reopened() {
    let dir = std::env::temp_dir().join(format!("corinzia-cmd-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("s.project");
    let mut a = app(View::Page);
    ok(&mut a, "select bar 2 staff 2");
    ok(&mut a, "input on");
    ok(&mut a, "duration half");
    ok(&mut a, "note C");
    ok(&mut a, "note E chord");
    let notes = ok(&mut a, "get notes bar 2 staff 2");
    let chord = notes.as_array().unwrap().iter().find(|n| n["pitches"].as_array().unwrap().len() == 2).expect("a chord");
    assert_eq!(chord["quarters"], 2.0);
    ok(&mut a, &format!("save {}", path.display()));
    let mut b = App::new(corinzia_model::store::load(&path).unwrap(), Some(path.clone()), View::Page, String::new());
    assert_eq!(ok(&mut b, "get notes bar 2 staff 2"), notes);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn chants_are_written_by_command() {
    let mut a = app(View::Page);
    assert!(a.command("neume Oligon").unwrap_err().contains("chant 1"));
    ok(&mut a, "chant 1");
    let before = ok(&mut a, "get chant")["elements"].as_array().unwrap().len();
    ok(&mut a, "neume oligon");
    ok(&mut a, "neume Apostrofos");
    let c = ok(&mut a, "get chant");
    assert_eq!(c["elements"].as_array().unwrap().len(), before + 2);
    assert_eq!(c["elements"][before]["Neume"]["quantity"], "Oligon");
    ok(&mut a, "mode \"πλ. β΄\"");
    assert_eq!(ok(&mut a, "get chant")["mode"], "Ἦχος πλ. β΄");
    ok(&mut a, "chant off");
    assert!(a.editor.chant.is_none());
}

#[test]
fn the_mcp_server_lists_its_tool_and_runs_commands() {
    let mut a = app(View::Page);
    let call = |a: &mut App, msg: Value| crate::mcp::handle(a, &msg);
    let init = call(&mut a, serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18"}})).unwrap();
    assert_eq!(init["result"]["serverInfo"]["name"], "corinzia");
    assert!(call(&mut a, serde_json::json!({"jsonrpc": "2.0", "method": "notifications/initialized"})).is_none(), "notifications get no answer");
    let tools = call(&mut a, serde_json::json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"})).unwrap();
    assert_eq!(tools["result"]["tools"][0]["name"], "corinzia");
    assert!(tools["result"]["tools"][0]["description"].as_str().unwrap().contains("select bar"));
    let run = |a: &mut App, c: &str| call(a, serde_json::json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "corinzia", "arguments": {"command": c}}})).unwrap();
    let r = run(&mut a, "title \"From an agent\"");
    assert_eq!(r["result"]["isError"], false);
    assert_eq!(a.editor.project.title, "From an agent");
    let got: Value = serde_json::from_str(run(&mut a, "get project")["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(got["title"], "From an agent");
    let bad = run(&mut a, "select bar 77");
    assert_eq!(bad["result"]["isError"], true);
    assert!(bad["result"]["content"][0]["text"].as_str().unwrap().contains("No bar 77"));
    let unknown = call(&mut a, serde_json::json!({"jsonrpc": "2.0", "id": 9, "method": "nope"})).unwrap();
    assert_eq!(unknown["error"]["code"], -32601);
}

#[test]
fn the_mixer_is_set_up_by_command() {
    let mut a = app(View::Page);
    let mixer = |a: &mut App| ok(a, "get mixer");
    ok(&mut a, "bus add Strings");
    ok(&mut a, "output cello strings");
    ok(&mut a, "send flute strings -12 pre");
    ok(&mut a, "gain 1 -6");
    ok(&mut a, "pan cello L30");
    ok(&mut a, "mute flute on");
    ok(&mut a, "mute flute on"); // already: stays on
    let m = mixer(&mut a);
    let (flute, cello) = (&m["tracks"][0], &m["tracks"][1]);
    assert_eq!((flute["gain_db"].as_f64(), flute["mute"].as_bool()), (Some(-6.0), Some(true)));
    assert_eq!(flute["sends"][0], serde_json::json!({"bus": "Strings", "db": -12.0, "pre": true}));
    assert_eq!((cello["output"].as_str(), cello["pan"].as_f64()), (Some("Strings"), Some(-0.3)));
    // Effects by plugin name, their parameters by name.
    ok(&mut a, "effect add strings Corinzia Reverb");
    ok(&mut a, "effect add strings Corinzia Gain");
    ok(&mut a, "effect move strings 2 up");
    let m = mixer(&mut a);
    assert_eq!(m["buses"][0]["effects"][0]["name"], "Corinzia Gain");
    let params = ok(&mut a, "get params strings 2");
    assert!(params.as_array().unwrap().iter().any(|p| p["name"] == "Mix"));
    ok(&mut a, "param strings 2 Mix 0.4");
    let mix = ok(&mut a, "get params strings 2").as_array().unwrap().iter().find(|p| p["name"] == "Mix").unwrap()["value"].as_f64().unwrap();
    assert!((mix - 0.4).abs() < 1e-6, "{mix}");
    ok(&mut a, "effect bypass strings 1");
    assert_eq!(mixer(&mut a)["buses"][0]["effects"][0]["bypassed"], true);
    // Automation at bars and beats.
    assert_eq!(ok(&mut a, "automate flute gain at 2 -3"), serde_json::json!({"point": 0}));
    ok(&mut a, "automate flute gain at 3:3 -9");
    let lane = a.editor.auto_lane(corinzia_model::AutoTarget::Gain(a.editor.project.tracks[0].id)).expect("a lane");
    assert_eq!(lane.points.len(), 2);
    assert_eq!(lane.points[1].at, corinzia_score::Frac::new(5, 2), "bar 3, beat 3 of 4/4: two and a half whole notes in");
    ok(&mut a, "automate flute gain clear");
    assert!(a.editor.auto_lane(corinzia_model::AutoTarget::Gain(a.editor.project.tracks[0].id)).is_none_or(|l| l.points.is_empty()));
    // Errors name what is there.
    assert!(a.command("gain oboe 0").unwrap_err().contains("Flute"));
    assert!(a.command("effect add strings No Such Plugin").unwrap_err().contains("Corinzia Reverb"));
    assert!(a.command("send flute nowhere").unwrap_err().contains("Strings"));
    // Everything is undoable, one step each.
    ok(&mut a, "undo");
    assert!(a.editor.auto_lane(corinzia_model::AutoTarget::Gain(a.editor.project.tracks[0].id)).is_some_and(|l| !l.points.is_empty()));
}
