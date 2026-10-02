// Antigravity (agy) hook installation: %USERPROFILE%\.gemini\config\hooks.json.
//
// Same contract as hooks.rs, and it leans on the same machinery: show the exact
// diff, take a dated backup, merge without touching anything that isn't ours,
// and write only when the file still matches what the user looked at.
//
// Coucou's entries live under a single top-level key, "coucou" — the layout the
// macOS app writes (HookServer.swift), so both platforms read the same file the
// same way. Uninstalling removes that key and nothing else.

use std::path::PathBuf;

use serde_json::{json, Map, Value};

use crate::hooks::{self, HookPreview, HookStatus};
use crate::{platform, settings};

/// The top-level key Coucou's hooks live under.
const KEY: &str = "coucou";
/// Marker that tells our entries from anybody else's.
const MARKER: &str = "coucou-hook";
/// Tells the relay (and then the island) which pill the events belong to.
const AGENT_FLAG: &str = "--agent antigravity ";
/// Seconds; Antigravity takes hook timeouts in seconds, not milliseconds.
const TIMEOUT: u64 = 10;

/// Tool-level events take a matcher group.
const TOOL_EVENTS: &[&str] = &["PreToolUse", "PostToolUse"];
/// Lifecycle events take a bare handler, no matcher.
const LIFECYCLE_EVENTS: &[&str] = &["PreInvocation", "PostInvocation", "Stop"];

pub fn hooks_path() -> PathBuf {
    platform::home_dir().join(".gemini").join("config").join("hooks.json")
}

/// The file as it is. Only "not there" means start from nothing: anything else
/// (a lock, bad JSON) is reported rather than treated as empty and written over.
fn read() -> Result<Value, String> {
    let path = hooks_path();
    match std::fs::read(&path) {
        Ok(bytes) => hooks::parse_settings(&bytes, &path.display().to_string()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(json!({})),
        Err(err) => Err(format!("Can't read {}: {err}", path.display())),
    }
}

fn handler(event: &str) -> Value {
    json!({
        "type": "command",
        "command": hooks::command_line(AGENT_FLAG, event),
        "timeout": TIMEOUT,
    })
}

/// What lives under the "coucou" key.
fn our_group() -> Value {
    let mut group = Map::new();
    for event in TOOL_EVENTS {
        group.insert((*event).into(), json!([{ "matcher": "*", "hooks": [handler(event)] }]));
    }
    for event in LIFECYCLE_EVENTS {
        group.insert((*event).into(), json!([handler(event)]));
    }
    Value::Object(group)
}

fn is_ours(group: &Value) -> bool {
    group.to_string().contains(MARKER)
}

/// The file with Coucou's hooks added; everything else is left exactly as it was.
fn merged(existing: &Value) -> Value {
    let mut root = existing.as_object().cloned().unwrap_or_default();
    root.insert(KEY.into(), our_group());
    Value::Object(root)
}

/// The file with Coucou's key removed — and only if it really is ours.
fn without_ours(existing: &Value) -> Value {
    let mut root = existing.as_object().cloned().unwrap_or_default();
    if root.get(KEY).is_some_and(is_ours) {
        root.remove(KEY);
    }
    Value::Object(root)
}

fn backup_path() -> PathBuf {
    hooks_path().with_file_name(format!("hooks.json.bak-{}", hooks::stamp()))
}

fn current_fingerprint() -> String {
    hooks::fingerprint(&std::fs::read(hooks_path()).unwrap_or_default())
}

// ── Public API ────────────────────────────────────────────────────────────────

pub fn status() -> HookStatus {
    let installed = read().ok().is_some_and(|root| root.get(KEY).is_some_and(is_ours));
    let hook_path = settings::hook_exe_path();
    HookStatus {
        installed,
        settings_path: hooks_path().to_string_lossy().to_string(),
        hook_ready: hook_path.exists(),
        hook_path: hook_path.to_string_lossy().to_string(),
    }
}

pub fn preview(install: bool) -> Result<HookPreview, String> {
    let current = read()?;
    let next = if install { merged(&current) } else { without_ours(&current) };
    Ok(HookPreview {
        diff: hooks::unified_diff(&hooks::pretty(&current), &hooks::pretty(&next)),
        backup: backup_path().to_string_lossy().to_string(),
        settings_path: hooks_path().to_string_lossy().to_string(),
        fingerprint: current_fingerprint(),
    })
}

/// Writes the merged (or cleaned) file after a dated backup. `fingerprint` is the
/// one the preview was computed from: if the file moved since, nothing is written.
pub fn write(install: bool, fingerprint: &str) -> Result<String, String> {
    let path = hooks_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }

    let current = read()?;
    if current_fingerprint() != fingerprint {
        return Err(format!(
            "{} changed since the preview. Nothing was written — review the new diff.",
            path.display()
        ));
    }

    let backup = backup_path();
    let next = if install { merged(&current) } else { without_ours(&current) };
    let mut text = hooks::pretty(&next);
    text.push('\n');
    hooks::commit(&path, &backup, &text)?;
    Ok(backup.to_string_lossy().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_adds_one_group_with_every_event_tagged_for_antigravity() {
        let after = merged(&json!({}));
        let group = after[KEY].as_object().unwrap();
        for event in TOOL_EVENTS.iter().chain(LIFECYCLE_EVENTS) {
            let text = group[*event].to_string();
            assert!(text.contains("--agent antigravity"), "{event} is not tagged");
            assert!(text.contains(MARKER), "{event} does not point at the relay");
            assert!(text.contains(&format!("antigravity {event}")), "{event} lost its name");
        }
        // Tool events carry a matcher; lifecycle events are bare handlers.
        assert_eq!(group["PreToolUse"][0]["matcher"], "*");
        assert!(group["Stop"][0].get("matcher").is_none());
        assert_eq!(group["Stop"][0]["timeout"], TIMEOUT);
    }

    #[test]
    fn install_and_uninstall_leave_everybody_elses_settings_alone() {
        let existing = json!({
            "someone-else": { "PreToolUse": [{ "hooks": [{ "type": "command", "command": "theirs.exe" }] }] },
            "theme": "dark"
        });
        let after = merged(&existing);
        assert_eq!(after["someone-else"], existing["someone-else"]);
        assert_eq!(after["theme"], "dark");
        assert_eq!(without_ours(&after), existing, "uninstall must restore the file as it was");
    }

    #[test]
    fn a_key_called_coucou_that_is_not_ours_is_never_removed() {
        let theirs = json!({ KEY: { "PreToolUse": [{ "hooks": [{ "type": "command", "command": "other.exe" }] }] } });
        assert_eq!(without_ours(&theirs), theirs);
        assert!(!theirs.get(KEY).is_some_and(is_ours));
    }

    #[test]
    fn reinstalling_replaces_our_group_instead_of_stacking_a_second() {
        let once = merged(&json!({}));
        assert_eq!(merged(&once), once);
    }
}
