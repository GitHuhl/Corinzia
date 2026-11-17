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
