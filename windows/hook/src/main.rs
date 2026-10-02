//! coucou-hook — the relay Claude Code runs on every hook event.
//!
//! Reads the hook JSON on stdin, adds a little terminal context, and hands it to
//! Coucou over the named pipe `\\.\pipe\coucou-<sid>` (Windows) or the Unix
//! socket `$XDG_RUNTIME_DIR/coucou.sock` (Linux).
//!
//! Hard rule (docs/CLAUDE.md): **never block Claude Code.**
//! * If the pipe does not exist — Coucou is closed — we exit 0 immediately with
//!   nothing on stdout, and the session carries on untouched.
//! * Every step runs under a deadline enforced by the main thread, so a pipe that
//!   accepts the connection and then stops reading cannot wedge the session
//!   either: we abandon the worker and exit.
//! * Only `PermissionRequest` waits for an answer, because approving from the
//!   island is the whole point. No answer means empty stdout, and Claude Code
//!   asks in the terminal exactly as if Coucou were not installed.
//!
//! Usage: `coucou-hook <EventName>` (the name is also read from the JSON).

use std::io::{Read, Write};
use std::sync::mpsc;
use std::time::Duration;

/// Budget for getting a pipe connection. Beyond this Claude Code wins, always.
const CONNECT_TIMEOUT: Duration = Duration::from_millis(300);
/// Whole-run budget for an event nobody waits on: connect and write, no more.
const FIRE_AND_FORGET_BUDGET: Duration = Duration::from_secs(2);
/// How long a permission prompt may stay on screen before the terminal takes over.
const DECISION_BUDGET: Duration = Duration::from_secs(110);

/// Fields that are pointless to forward and can be enormous (a whole file read,
/// a full command output). The island never shows them.
const DROPPED_FIELDS: &[&str] = &["tool_response", "transcript_path"];
/// Longest string forwarded for any single field; the island truncates to far
/// less than this anyway.
const MAX_FIELD_LEN: usize = 2_000;

#[cfg(windows)]
mod win;
#[cfg(windows)]
use win::connect;

#[cfg(target_os = "linux")]
mod unix;
#[cfg(target_os = "linux")]
use unix::connect;

fn main() {
    let Some((payload, event)) = read_event() else { std::process::exit(0) };

    let waits_for_answer = event == "PermissionRequest";
    let budget = if waits_for_answer { DECISION_BUDGET } else { FIRE_AND_FORGET_BUDGET };

    // The worker owns every blocking call. If it overruns the budget we simply
    // stop listening and exit: the process dying takes the pipe handle with it.
    // (No catch_unwind here — the release profile is panic = "abort", so it would
    // be dead code. `talk` is written to have nothing to panic on instead.)
    let (tx, rx) = mpsc::channel::<Option<String>>();
    std::thread::spawn(move || {
        let _ = tx.send(talk(&payload, waits_for_answer));
    });

    if let Ok(Some(decision)) = rx.recv_timeout(budget) {
        if let Some(json) = decision_json(&decision) {
            let mut out = std::io::stdout();
            let _ = writeln!(out, "{json}");
            let _ = out.flush();
        }
    }
    // Nothing printed: Claude Code asks in the terminal, as if we were not here.
    std::process::exit(0);
}

/// The documented PermissionRequest output. Anything we do not recognise prints
/// nothing at all rather than guessing — silence is the safe answer.
/// See https://code.claude.com/docs/en/hooks
fn decision_json(decision: &str) -> Option<String> {
    let behavior = match decision.trim() {
        // "always" still answers a plain allow; remembering it is the island's
        // business, not Claude Code's.
        "allow" | "always" => r#"{"behavior":"allow"}"#.to_string(),
        "deny" => r#"{"behavior":"deny","message":"Denied from Coucou"}"#.to_string(),
        _ => return None,
    };
    Some(format!(
        r#"{{"hookSpecificOutput":{{"hookEventName":"PermissionRequest","decision":{behavior}}}}}"#
    ))
}

/// Reads stdin and returns the payload to forward plus the event name.
fn read_event() -> Option<(String, String)> {
    let mut raw = Vec::new();
    if std::io::stdin().read_to_end(&mut raw).is_err() || raw.is_empty() {
        return None;
    }
    // Some shells hand us a UTF-8 BOM; serde_json would choke on it.
    if raw.starts_with(&[0xEF, 0xBB, 0xBF]) {
        raw.drain(..3);
    }

    let mut payload = serde_json::from_slice::<serde_json::Value>(&raw).ok()?;
    let map = payload.as_object_mut()?;

    // Parse argv: "coucou-hook.exe [--agent <name>] [<EventName>]"
    // --agent tags the payload with coucou_agent so the app routes to the right pill.
    // Absent or invalid names are validated and discarded by the app, not here.
    let mut agent = String::new();
    let mut arg_event = String::new();
    {
        let mut it = std::env::args().skip(1);
        while let Some(arg) = it.next() {
            if arg == "--agent" {
                agent = it.next().unwrap_or_default();
            } else if arg_event.is_empty() {
                arg_event = arg;
            }
        }
    }
    // Which agent this hook was installed for. Absent means Claude Code,
    // so existing hook commands keep working unchanged.
    if !agent.is_empty() {
        map.insert("coucou_agent".into(), serde_json::Value::String(agent));
    }
    let raw_event = map
        .get("hook_event_name")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .filter(|s| !s.is_empty())
        .unwrap_or(arg_event);
    // Gemini CLI and Antigravity name their events and tool fields their own way.
    // Claude Code's names are not in the table, so they pass through untouched.
    let event = normalize_event(&raw_event).to_string();
    map.insert("hook_event_name".into(), serde_json::Value::String(event.clone()));
    normalize_tool_fields(map);

    for field in DROPPED_FIELDS {
        map.remove(*field);
    }

    let cwd_missing = map
        .get("cwd")
        .and_then(|v| v.as_str())
        .map(str::is_empty)
        .unwrap_or(true);
    if cwd_missing {
        // Antigravity reports its open folders instead of a cwd.
        let workspace = map
            .get("workspacePaths")
            .and_then(|v| v.as_array())
            .and_then(|a| a.first())
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        if let Some(path) = workspace {
            map.insert("cwd".into(), serde_json::Value::String(path));
        } else if let Ok(cwd) = std::env::current_dir() {
            map.insert(
                "cwd".into(),
                serde_json::Value::String(cwd.to_string_lossy().to_string()),
            );
        }
    }

    // Which terminal the session runs in. Unlike macOS, Coucou here accepts
    // events from every terminal, so this is context only — never a filter.
    for (key, var) in [
        ("term_program", "TERM_PROGRAM"),
        ("wt_session", "WT_SESSION"),
        ("term_session_id", "TERM_SESSION_ID"),
        ("vscode_pid", "VSCODE_PID"),
        ("session_pid", "CLAUDE_CODE_SSE_PORT"),
    ] {
        if !map.contains_key(key) {
            let value = std::env::var(var).unwrap_or_default();
            map.insert(key.into(), serde_json::Value::String(value));
        }
    }

    truncate_strings(&mut payload);

    let mut line = payload.to_string();
    line.push('\n');
    Some((line, event))
}

/// Gemini CLI and Antigravity (agy) event names → the names the island speaks.
/// Same table as the macOS relay (HookServer.swift, `normalize_event`).
fn normalize_event(name: &str) -> &str {
    match name {
        "BeforeTool" | "BeforeToolSelection" => "PreToolUse",
        "AfterTool" | "AfterModel" => "PostToolUse",
        "BeforeAgent" | "PreInvocation" => "UserPromptSubmit",
        "AfterAgent" => "Stop",
        "PostInvocation" => "PostToolUse",
        "startup" => "SessionStart",
        "exit" => "SessionEnd",
        other => other,
    }
}

/// Antigravity carries the tool as `toolCall.{name,args}` with its own argument
/// names, and the conversation as `conversationId`. The island reads
/// `tool_name`, `tool_input` and `session_id`, as Claude Code sends them.
/// Does nothing when `tool_name` is already there, i.e. for Claude Code.
fn normalize_tool_fields(map: &mut serde_json::Map<String, serde_json::Value>) {
    use serde_json::Value;
    if map.contains_key("tool_name") {
        return;
    }
    let text = |v: Option<&Value>| v.and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string);
    let tool = map.get("toolCall").and_then(Value::as_object).cloned().unwrap_or_default();

    if let Some(name) = text(tool.get("name")).or_else(|| text(map.get("tool"))) {
        map.insert("tool_name".into(), Value::String(name));
    }
    if !map.contains_key("tool_input") {
        if let Some(args) = tool.get("args").and_then(Value::as_object) {
            let mut flat = args.clone();
            for (from, to) in [
                ("CommandLine", "command"),
                ("FilePath", "file_path"),
                ("Path", "path"),
                ("Url", "url"),
                ("Query", "query"),
                ("Pattern", "pattern"),
            ] {
                if let Some(v) = flat.get(from).cloned() {
                    flat.insert(to.into(), v);
                }
            }
            map.insert("tool_input".into(), Value::Object(flat));
        }
    }
    if !map.contains_key("session_id") {
        let id = ["conversationId", "conversation_id", "sessionId", "GEMINI_SESSION_ID"]
            .iter()
            .find_map(|k| text(map.get(*k)))
            .or_else(|| std::env::var("GEMINI_SESSION_ID").ok().filter(|s| !s.is_empty()));
        if let Some(id) = id {
            map.insert("session_id".into(), Value::String(id));
        }
    }
}

/// Caps every string in the payload. A single Write can carry a whole file.
fn truncate_strings(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::String(s) => {
            if s.len() > MAX_FIELD_LEN {
                // Cut on a char boundary; a lone byte index can split UTF-8.
                let mut end = MAX_FIELD_LEN;
                while end > 0 && !s.is_char_boundary(end) {
                    end -= 1;
                }
                s.truncate(end);
                s.push('…');
            }
        }
        serde_json::Value::Array(items) => items.iter_mut().for_each(truncate_strings),
        serde_json::Value::Object(map) => map.values_mut().for_each(truncate_strings),
        _ => {}
    }
}

/// Connect, send, and — for a permission request — wait for the island's word.
fn talk(payload: &str, waits_for_answer: bool) -> Option<String> {
    let mut pipe = connect()?;

    if pipe.write_all(payload.as_bytes()).is_err() {
        return None;
    }
    let _ = pipe.flush();

    if !waits_for_answer {
        return None;
    }

    let mut buf = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        match pipe.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if buf.contains(&b'\n') {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    let answer = String::from_utf8_lossy(&buf).trim().to_string();
    (!answer.is_empty()).then_some(answer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decision_json_matches_the_documented_shape() {
        assert_eq!(
            decision_json("allow").unwrap(),
            r#"{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"allow"}}}"#
        );
        assert_eq!(
            decision_json("deny").unwrap(),
            r#"{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"deny","message":"Denied from Coucou"}}}"#
        );
        // "always" is an island concept; Claude Code just gets an allow.
        assert!(decision_json("always").unwrap().contains(r#""behavior":"allow""#));
    }

    #[test]
    fn anything_unrecognised_prints_nothing() {
        assert!(decision_json("").is_none());
        assert!(decision_json("maybe").is_none());
        // The shape the app used to send must not be mistaken for a decision.
        assert!(decision_json(r#"{"permissionDecision":"allow"}"#).is_none());
    }

    #[test]
    fn antigravity_and_gemini_events_get_the_islands_names() {
        assert_eq!(normalize_event("PreInvocation"), "UserPromptSubmit");
        assert_eq!(normalize_event("PostInvocation"), "PostToolUse");
        assert_eq!(normalize_event("BeforeTool"), "PreToolUse");
        assert_eq!(normalize_event("AfterAgent"), "Stop");
        // Claude Code's own names must never be touched.
        for own in ["PreToolUse", "PermissionRequest", "SessionStart", "Stop", "SubagentStop"] {
            assert_eq!(normalize_event(own), own);
        }
    }

    #[test]
    fn an_antigravity_tool_call_becomes_a_claude_shaped_one() {
        let mut v = serde_json::json!({
            "toolCall": { "name": "run_command", "args": { "CommandLine": "npm test", "Cwd": "D:\\x" } },
            "conversationId": "c-1"
        });
        normalize_tool_fields(v.as_object_mut().unwrap());
        assert_eq!(v["tool_name"], "run_command");
        assert_eq!(v["tool_input"]["command"], "npm test");
        // The original argument stays too; only aliases are added.
        assert_eq!(v["tool_input"]["CommandLine"], "npm test");
        assert_eq!(v["session_id"], "c-1");
    }

    #[test]
    fn a_claude_code_payload_is_left_exactly_as_it_was() {
        let original = serde_json::json!({
            "tool_name": "Bash", "tool_input": { "command": "ls" }, "session_id": "s", "conversationId": "other"
        });
        let mut v = original.clone();
        normalize_tool_fields(v.as_object_mut().unwrap());
        assert_eq!(v, original);
    }

    #[test]
    fn long_strings_are_cut_on_a_char_boundary() {
        let mut v = serde_json::json!({ "tool_input": { "content": "é".repeat(4000) } });
        truncate_strings(&mut v);
        let s = v["tool_input"]["content"].as_str().unwrap();
        assert!(s.len() <= MAX_FIELD_LEN + 4);
        assert!(s.ends_with('…'));
    }
}
