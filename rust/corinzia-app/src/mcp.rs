//! `corinzia --mcp [file]`: a Model Context Protocol server on standard
//! input and output (JSON-RPC 2.0, one message per line), so AI agents can
//! open, query, edit, save and export scores. Its one tool, `corinzia`,
//! runs a text command (see `command.rs`) and answers with its JSON.

use crate::app::App;
use crate::command::HELP;
use serde_json::{json, Value};
use std::io::{BufRead, Write};

const PROTOCOL: &str = "2025-06-18";

fn tool() -> Value {
    let commands: String = HELP.iter().map(|(u, w)| format!("\n- {u}: {w}")).collect();
    json!({
        "name": "corinzia",
        "description": format!(
            "Runs one Corinzia command on the open score and returns its JSON answer. \
             Edits apply to the selection or caret, like the keyboard; start with `get project` and `get notes`. \
             Commands:{commands}"
        ),
        "inputSchema": {
            "type": "object",
            "properties": {"command": {"type": "string", "description": "One command line, e.g. `select bar 3-4 staff 1` or `transpose \"up M2\"`"}},
            "required": ["command"]
        }
    })
}

/// The answer to one request (`None` for a notification).
pub(crate) fn handle(app: &mut App, msg: &Value) -> Option<Value> {
    let id = msg.get("id").cloned()?;
    let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
    let reply = |result: Value| Some(json!({"jsonrpc": "2.0", "id": id, "result": result}));
    let error = |code: i64, message: &str| Some(json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}));
    match method {
        "initialize" => reply(json!({
            "protocolVersion": msg["params"]["protocolVersion"].as_str().unwrap_or(PROTOCOL),
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "corinzia", "version": env!("CARGO_PKG_VERSION")},
            "instructions": "Corinzia is a music notation program and DAW. The `corinzia` tool runs commands on the open score; `help` lists them.",
        })),
        "ping" => reply(json!({})),
        "tools/list" => reply(json!({"tools": [tool()]})),
        "tools/call" => {
            if msg["params"]["name"] != "corinzia" {
                return error(-32602, "Unknown tool: the one tool is `corinzia`");
            }
            let Some(command) = msg["params"]["arguments"]["command"].as_str() else {
                return error(-32602, "`command` (a string) is required");
            };
            let (text, failed) = match app.command(command) {
                Ok(v) => (v.to_string(), false),
                Err(e) => (e, true),
            };
            reply(json!({"content": [{"type": "text", "text": text}], "isError": failed}))
        }
        _ => error(-32601, &format!("Method not found: {method}")),
    }
}

/// Serves until standard input closes.
pub(crate) fn serve(mut app: App) {
    let stdin = std::io::stdin();
    let mut out = std::io::stdout().lock();
    for line in stdin.lock().lines().map_while(Result::ok) {
        if line.trim().is_empty() {
            continue;
        }
        let answer = match serde_json::from_str::<Value>(&line) {
            Ok(msg) => handle(&mut app, &msg),
            Err(e) => Some(json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": format!("Parse error: {e}")}})),
        };
        if let Some(a) = answer {
            let _ = writeln!(out, "{a}");
            let _ = out.flush();
        }
    }
}
