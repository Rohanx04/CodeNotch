//! codenotch-hook — the relay Claude Code runs on every hook event.
//!
//! Reads the hook JSON on stdin, trims it, and hands it to CodeNotch over the
//! named pipe `\\.\pipe\codenotch-<sid>`.
//!
//! Hard rule: **never block Claude Code.**
//! * If the pipe does not exist — CodeNotch is closed — we exit 0 immediately
//!   with nothing on stdout, and the session carries on untouched.
//! * Every step runs under a deadline enforced by the main thread, so a pipe
//!   that accepts the connection and then stops reading cannot wedge the
//!   session either: we abandon the worker and exit.
//! * Only `PermissionRequest` waits for an answer, because approving from the
//!   notch is the whole point. No answer means empty stdout, and Claude Code
//!   asks in the terminal exactly as if CodeNotch were not installed.
//!
//! Usage: `codenotch-hook <EventName>` (the name is also read from the JSON).
//!
//! Adapted from the MIT-licensed relay in Coucou (Copyright (c) 2026 Louis
//! Raillé, https://github.com/Louis-CFM/coucou).

use std::io::{Read, Write};
use std::sync::mpsc;
use std::time::Duration;

/// Whole-run budget for an event nobody waits on: connect and write, no more.
const FIRE_AND_FORGET_BUDGET: Duration = Duration::from_secs(2);
/// How long a permission prompt may stay on the notch before the terminal
/// takes over. CodeNotch answers inside this (it gives up at 108 s), and the
/// hook's own timeout in settings.json is 120 s, so the order is always
/// CodeNotch → relay → Claude Code.
const DECISION_BUDGET: Duration = Duration::from_secs(110);

/// Fields that are pointless to forward and can be enormous (a whole file
/// read, a full command output). The notch never shows them.
const DROPPED_FIELDS: &[&str] = &["tool_response", "transcript_path", "last_assistant_message"];
/// Longest string forwarded for any single field; the notch truncates to far
/// less than this anyway.
const MAX_FIELD_LEN: usize = 2_000;

#[cfg(windows)]
mod win;

fn main() {
    let Some((payload, event)) = read_event() else {
        std::process::exit(0)
    };

    let waits_for_answer = event == "PermissionRequest";
    let budget = if waits_for_answer {
        DECISION_BUDGET
    } else {
        FIRE_AND_FORGET_BUDGET
    };

    // The worker owns every blocking call. If it overruns the budget we stop
    // listening and exit: the process dying takes the pipe handle with it.
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

/// The documented `PermissionRequest` output. Anything we do not recognise
/// prints nothing at all rather than guessing — silence is the safe answer.
/// See https://code.claude.com/docs/en/hooks
fn decision_json(decision: &str) -> Option<String> {
    let behavior = match decision.trim() {
        "allow" => r#"{"behavior":"allow"}"#,
        "deny" => r#"{"behavior":"deny","message":"Denied from CodeNotch"}"#,
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
    prepare(raw, std::env::args().nth(1).unwrap_or_default())
}

/// Parse, trim and annotate one payload. Split from `read_event` for tests.
fn prepare(mut raw: Vec<u8>, arg_event: String) -> Option<(String, String)> {
    // Some shells hand us a UTF-8 BOM; serde_json would choke on it.
    if raw.starts_with(&[0xEF, 0xBB, 0xBF]) {
        raw.drain(..3);
    }

    let mut payload = serde_json::from_slice::<serde_json::Value>(&raw).ok()?;
    let map = payload.as_object_mut()?;

    // The hook command passes the event as argv[1]; the JSON usually carries
    // it too. Trust argv when the JSON is missing it.
    let event = map
        .get("hook_event_name")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .filter(|s| !s.is_empty())
        .unwrap_or(arg_event);
    if event.is_empty() {
        return None;
    }
    map.insert(
        "hook_event_name".into(),
        serde_json::Value::String(event.clone()),
    );

    for field in DROPPED_FIELDS {
        map.remove(*field);
    }

    let cwd_missing = map
        .get("cwd")
        .and_then(|v| v.as_str())
        .is_none_or(str::is_empty);
    if cwd_missing {
        if let Ok(cwd) = std::env::current_dir() {
            map.insert(
                "cwd".into(),
                serde_json::Value::String(cwd.to_string_lossy().to_string()),
            );
        }
    }

    truncate_strings(&mut payload);

    let mut line = payload.to_string();
    line.push('\n');
    Some((line, event))
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

/// Connect, send, and — for a permission request — wait for the notch's word.
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

/// Opens the pipe. Retries only while the server is busy: any other error
/// means there is nothing to talk to, and waiting would only delay Claude Code.
#[cfg(windows)]
fn connect() -> Option<std::fs::File> {
    use std::os::windows::io::AsRawHandle;
    use std::time::Instant;

    /// Budget for getting a connection. Beyond this Claude Code wins, always.
    const CONNECT_TIMEOUT: Duration = Duration::from_millis(300);
    /// `ERROR_PIPE_BUSY` — every instance is serving someone else right now.
    /// The one error worth retrying: the server exists and a slot will free up.
    const ERROR_PIPE_BUSY: i32 = 231;

    let path = win::pipe_path();
    let deadline = Instant::now() + CONNECT_TIMEOUT;
    loop {
        match std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
        {
            Ok(file) => {
                let handle = windows::Win32::Foundation::HANDLE(file.as_raw_handle());
                // Somebody else's server on our pipe name gets nothing from us.
                return win::pipe_server_is_same_user(handle).then_some(file);
            }
            Err(err) => {
                if err.raw_os_error() != Some(ERROR_PIPE_BUSY) || Instant::now() >= deadline {
                    return None;
                }
                std::thread::sleep(Duration::from_millis(15));
            }
        }
    }
}

/// CodeNotch only runs on Windows; anywhere else there is never a pipe, so the
/// relay exits at once — which is also what keeps it testable off-Windows.
#[cfg(not(windows))]
fn connect() -> Option<std::fs::File> {
    None
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
            decision_json("deny\n").unwrap(),
            r#"{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"deny","message":"Denied from CodeNotch"}}}"#
        );
    }

    #[test]
    fn anything_unrecognised_prints_nothing() {
        assert!(decision_json("").is_none());
        assert!(decision_json("maybe").is_none());
        assert!(decision_json(r#"{"permissionDecision":"allow"}"#).is_none());
    }

    #[test]
    fn long_strings_are_cut_on_a_char_boundary() {
        let mut v = serde_json::json!({ "tool_input": { "content": "é".repeat(4000) } });
        truncate_strings(&mut v);
        let s = v["tool_input"]["content"].as_str().unwrap();
        assert!(s.len() <= MAX_FIELD_LEN + 4);
        assert!(s.ends_with('…'));
    }

    #[test]
    fn payloads_are_trimmed_and_named() {
        let raw = br#"{"session_id":"s","cwd":"/w","tool_response":"huge","transcript_path":"/t"}"#;
        let (line, event) = prepare(raw.to_vec(), "PostToolUse".into()).unwrap();
        assert_eq!(event, "PostToolUse");
        let v: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(v["hook_event_name"], "PostToolUse");
        assert!(v.get("tool_response").is_none());
        assert!(v.get("transcript_path").is_none());
        assert_eq!(v["cwd"], "/w");
        assert!(line.ends_with('\n'), "the pipe reads up to a newline");
    }

    #[test]
    fn the_json_event_name_wins_and_a_bom_is_tolerated() {
        let mut raw = vec![0xEF, 0xBB, 0xBF];
        raw.extend_from_slice(br#"{"hook_event_name":"Stop"}"#);
        let (_, event) = prepare(raw, "Other".into()).unwrap();
        assert_eq!(event, "Stop");
    }

    #[test]
    fn nothing_usable_means_nothing_sent() {
        assert!(prepare(b"not json".to_vec(), "Stop".into()).is_none());
        assert!(prepare(b"[1]".to_vec(), "Stop".into()).is_none());
        assert!(prepare(b"{}".to_vec(), String::new()).is_none());
    }

    #[test]
    fn without_codenotch_running_the_relay_answers_nothing() {
        // On a machine with no pipe (and on every non-Windows host) the relay
        // must fall straight through, including for a permission request.
        #[cfg(not(windows))]
        assert!(talk("{}\n", true).is_none());
    }
}
