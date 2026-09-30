//! Installing CodeNotch's Claude Code hooks into `~/.claude/settings.json`.
//!
//! This is the one place CodeNotch writes to another tool's state, so it is
//! opt-in and held to strict rules:
//!
//! - **Nothing is written without being shown first.** [`HookInstaller::preview`]
//!   returns the exact diff and the backup path; [`HookInstaller::write`] only
//!   applies it after the user confirms.
//! - **What was shown is what gets written.** The preview carries a fingerprint
//!   of the bytes it was computed from, and the write refuses if the file has
//!   changed since -- another tool, the user's editor -- rather than silently
//!   reverting somebody else's edit.
//! - **Everything else is left alone.** The merge adds CodeNotch's entries and
//!   touches no other key and no other tool's hook; uninstalling removes only
//!   CodeNotch's entries, so install-then-uninstall gives back the original.
//! - **An unreadable file is an error, never an empty object.** Treating a
//!   locked or malformed `settings.json` as `{}` would write a file containing
//!   nothing but our hooks over the user's settings.
//! - **A dated backup is taken before every write**, and the write goes to a
//!   temporary file renamed over the original, so a crash leaves it intact.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Local};
use serde::Serialize;
use serde_json::{json, Map, Value};

/// Every hook event CodeNotch listens to, with the timeout (seconds) written
/// beside it. `PermissionRequest` waits for a human, so it gets the relay's
/// decision budget plus headroom; the rest are fire-and-forget.
pub const HOOK_EVENTS: &[(&str, u64)] = &[
    ("SessionStart", 10),
    ("SessionEnd", 10),
    ("UserPromptSubmit", 10),
    ("PreToolUse", 10),
    ("PostToolUse", 10),
    ("PostToolUseFailure", 10),
    ("PermissionRequest", 120),
    ("Notification", 10),
    ("Stop", 10),
    ("StopFailure", 10),
    ("SubagentStart", 10),
    ("SubagentStop", 10),
];

/// Identifies a CodeNotch entry: the relay's executable name.
pub const MARKER: &str = "codenotch-hook";

/// Whether the hooks are in place, for the settings panel.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HookStatus {
    pub installed: bool,
    pub settings_path: String,
    pub hook_path: String,
    /// The relay executable exists where the hooks point.
    pub hook_ready: bool,
}

/// What an install or uninstall would change, before anything is written.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HookPreview {
    pub install: bool,
    pub diff: String,
    pub backup: String,
    pub settings_path: String,
    /// Identifies the bytes this diff was computed from; handed back to
    /// [`HookInstaller::write`] so only what the user looked at is applied.
    pub fingerprint: String,
}

/// Reads and writes one `settings.json` on behalf of one relay executable.
#[derive(Debug, Clone)]
pub struct HookInstaller {
    settings_path: PathBuf,
    hook_exe: PathBuf,
}

impl HookInstaller {
    pub fn new(settings_path: impl Into<PathBuf>, hook_exe: impl Into<PathBuf>) -> Self {
        Self {
            settings_path: settings_path.into(),
            hook_exe: hook_exe.into(),
        }
    }

    /// `~/.claude/settings.json`, the file Claude Code reads its hooks from.
    pub fn default_settings_path() -> Option<PathBuf> {
        dirs::home_dir().map(|h| h.join(".claude").join("settings.json"))
    }

    pub fn settings_path(&self) -> &Path {
        &self.settings_path
    }

    /// The command written for one event: the quoted relay path in forward
    /// slashes plus the event name. On Windows Claude Code runs hook commands
    /// through Git Bash, where backslashes are escapes and anything leaning on
    /// PowerShell or cmd syntax breaks.
    pub fn hook_command(&self, event: &str) -> String {
        let exe = self.hook_exe.to_string_lossy().replace('\\', "/");
        format!("\"{exe}\" {event}")
    }

    pub fn status(&self) -> HookStatus {
        // Read-only and never loud: an unreadable file just reads as "not
        // installed" here; anything that writes surfaces the real error.
        let current = self.read().unwrap_or_else(|_| json!({}));
        let installed = current
            .get("hooks")
            .and_then(Value::as_object)
            .map(|hooks| {
                hooks
                    .values()
                    .filter_map(Value::as_array)
                    .flatten()
                    .any(entry_is_ours)
            })
            .unwrap_or(false);
        HookStatus {
            installed,
            settings_path: self.settings_path.to_string_lossy().to_string(),
            hook_path: self.hook_exe.to_string_lossy().to_string(),
            hook_ready: self.hook_exe.exists(),
        }
    }

    /// The diff an install (or uninstall) would make, without writing.
    pub fn preview(&self, install: bool, now: DateTime<Local>) -> Result<HookPreview, String> {
        let current = self.read()?;
        let next = self.next(&current, install);
        Ok(HookPreview {
            install,
            diff: unified_diff(&pretty(&current), &pretty(&next)),
            backup: self.backup_path(now).to_string_lossy().to_string(),
            settings_path: self.settings_path.to_string_lossy().to_string(),
            fingerprint: self.current_fingerprint(),
        })
    }

    /// Apply a previewed change. Returns the backup's path (empty when there
    /// was no file to back up).
    pub fn write(
        &self,
        install: bool,
        fingerprint: &str,
        now: DateTime<Local>,
    ) -> Result<String, String> {
        let path = &self.settings_path;
        let dir = path.parent().unwrap_or(Path::new("."));
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;

        // Read before backing up: an unreadable file aborts before anything is
        // touched.
        let current = self.read()?;
        if self.current_fingerprint() != fingerprint {
            return Err(format!(
                "{} changed since the preview. Nothing was written — review the new diff.",
                path.display()
            ));
        }

        let backup = if path.exists() {
            let backup = self.backup_path(now);
            std::fs::copy(path, &backup).map_err(|e| format!("backup failed: {e}"))?;
            backup.to_string_lossy().to_string()
        } else {
            String::new()
        };

        let mut text = pretty(&self.next(&current, install));
        text.push('\n');

        let temp = path.with_extension(format!("json.codenotch-{}", std::process::id()));
        std::fs::write(&temp, text.as_bytes()).map_err(|e| format!("write failed: {e}"))?;
        if let Err(err) = std::fs::rename(&temp, path) {
            let _ = std::fs::remove_file(&temp);
            return Err(format!("write failed: {err}"));
        }
        Ok(backup)
    }

    fn next(&self, current: &Value, install: bool) -> Value {
        if install {
            merged(current, |event| self.hook_command(event))
        } else {
            without_ours(current)
        }
    }

    /// The only error that means "start from nothing" is the file not being
    /// there. A lock, a permission problem or JSON we cannot parse all mean we
    /// do not know what is in it -- and not knowing is not the same as empty.
    fn read(&self) -> Result<Value, String> {
        match std::fs::read(&self.settings_path) {
            Ok(bytes) => parse_settings(&bytes, &self.settings_path.display().to_string()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(json!({})),
            Err(err) => Err(format!(
                "Can't read {}: {err}",
                self.settings_path.display()
            )),
        }
    }

    /// Down to the second: installing then uninstalling in the same minute
    /// must not overwrite the first backup.
    fn backup_path(&self, now: DateTime<Local>) -> PathBuf {
        self.settings_path
            .with_file_name(format!("settings.json.bak-{}", now.format("%Y%m%d-%H%M%S")))
    }

    fn current_fingerprint(&self) -> String {
        match std::fs::read(&self.settings_path) {
            Ok(bytes) => fingerprint(&bytes),
            Err(_) => fingerprint(b""),
        }
    }
}

/// Parse `settings.json` bytes. A UTF-8 BOM (PowerShell's `-Encoding utf8`
/// writes one) is stripped; blank files start from nothing; anything else that
/// is not a JSON object is refused.
pub fn parse_settings(bytes: &[u8], path: &str) -> Result<Value, String> {
    let text = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    if text.iter().all(u8::is_ascii_whitespace) {
        return Ok(json!({}));
    }
    match serde_json::from_slice::<Value>(text) {
        Ok(v) if v.is_object() => Ok(v),
        Ok(_) => Err(format!(
            "{path} isn't a JSON object — CodeNotch won't touch it."
        )),
        Err(err) => Err(format!(
            "{path} isn't valid JSON ({err}). Fix or move it, then try again — CodeNotch won't overwrite it."
        )),
    }
}

fn entry_is_ours(entry: &Value) -> bool {
    entry
        .get("hooks")
        .and_then(Value::as_array)
        .map(|hooks| {
            hooks.iter().any(|h| {
                h.get("command")
                    .and_then(Value::as_str)
                    .is_some_and(|c| c.contains(MARKER))
            })
        })
        .unwrap_or(false)
}

/// `existing` with CodeNotch's hooks added (replacing any older copy of ours).
fn merged(existing: &Value, command: impl Fn(&str) -> String) -> Value {
    let mut root = existing.as_object().cloned().unwrap_or_default();
    let mut hooks = root
        .get("hooks")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_else(Map::new);

    for (event, timeout) in HOOK_EVENTS {
        let mut list = hooks
            .get(*event)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        list.retain(|entry| !entry_is_ours(entry));
        list.push(json!({
            "hooks": [{
                "type": "command",
                "command": command(event),
                "timeout": timeout,
            }]
        }));
        hooks.insert((*event).to_string(), Value::Array(list));
    }

    root.insert("hooks".into(), Value::Object(hooks));
    Value::Object(root)
}

/// `existing` with every CodeNotch entry removed and nothing else changed.
fn without_ours(existing: &Value) -> Value {
    let mut root = existing.as_object().cloned().unwrap_or_default();
    let Some(hooks) = root.get("hooks").and_then(Value::as_object).cloned() else {
        return Value::Object(root);
    };
    let mut out = Map::new();
    for (event, value) in hooks {
        match value.as_array() {
            Some(list) => {
                let kept: Vec<Value> = list.iter().filter(|e| !entry_is_ours(e)).cloned().collect();
                if !kept.is_empty() {
                    out.insert(event, Value::Array(kept));
                }
            }
            None => {
                out.insert(event, value);
            }
        }
    }
    if out.is_empty() {
        root.remove("hooks");
    } else {
        root.insert("hooks".into(), Value::Object(out));
    }
    Value::Object(root)
}

fn pretty(v: &Value) -> String {
    serde_json::to_string_pretty(v).unwrap_or_default()
}

/// FNV-1a: the only question is "is this still the file I showed the user?".
fn fingerprint(bytes: &[u8]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// A line diff with three lines of context. `settings.json` is short, so a
/// plain O(n·m) longest-common-subsequence is the simplest honest diff.
pub fn unified_diff(before: &str, after: &str) -> String {
    let a: Vec<&str> = before.lines().collect();
    let b: Vec<&str> = after.lines().collect();
    let (n, m) = (a.len(), b.len());

    let mut lcs = vec![vec![0usize; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[i][j] = if a[i] == b[j] {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }

    let mut out: Vec<String> = Vec::new();
    let (mut i, mut j) = (0usize, 0usize);
    while i < n && j < m {
        if a[i] == b[j] {
            out.push(format!("  {}", a[i]));
            i += 1;
            j += 1;
        } else if lcs[i + 1][j] >= lcs[i][j + 1] {
            out.push(format!("- {}", a[i]));
            i += 1;
        } else {
            out.push(format!("+ {}", b[j]));
            j += 1;
        }
    }
    out.extend(a[i..].iter().map(|l| format!("- {l}")));
    out.extend(b[j..].iter().map(|l| format!("+ {l}")));

    let changed: Vec<usize> = out
        .iter()
        .enumerate()
        .filter(|(_, l)| l.starts_with('+') || l.starts_with('-'))
        .map(|(i, _)| i)
        .collect();
    if changed.is_empty() {
        return "No change.".into();
    }
    let mut keep = vec![false; out.len()];
    for idx in changed {
        let lo = idx.saturating_sub(3);
        let hi = (idx + 4).min(out.len());
        keep[lo..hi].iter_mut().for_each(|k| *k = true);
    }
    let mut result = String::new();
    let mut gap = false;
    for (idx, line) in out.iter().enumerate() {
        if keep[idx] {
            result.push_str(line);
            result.push('\n');
            gap = false;
        } else if !gap {
            result.push_str("  …\n");
            gap = true;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    const WHERE: &str = "settings.json";

    fn installer(dir: &Path) -> HookInstaller {
        HookInstaller::new(
            dir.join(".claude").join("settings.json"),
            PathBuf::from(r"C:\Users\me\AppData\Local\CodeNotch\bin\codenotch-hook.exe"),
        )
    }

    fn cmd(event: &str) -> String {
        format!("\"codenotch-hook\" {event}")
    }

    #[test]
    fn a_utf8_bom_is_stripped_not_treated_as_corruption() {
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(br#"{"model":"opus","hooks":{}}"#);
        let parsed = parse_settings(&bytes, WHERE).expect("a BOM must not defeat the parser");
        assert_eq!(parsed["model"], "opus");
    }

    #[test]
    fn unreadable_content_is_an_error_never_an_empty_object() {
        for bad in [&b"{ not json"[..], &b"[1,2,3]"[..], &b"\"a string\""[..]] {
            assert!(parse_settings(bad, WHERE).is_err());
        }
    }

    #[test]
    fn empty_and_whitespace_files_start_from_nothing() {
        assert_eq!(parse_settings(b"", WHERE).unwrap(), json!({}));
        assert_eq!(parse_settings(b"  \n\t ", WHERE).unwrap(), json!({}));
    }

    #[test]
    fn the_command_is_quoted_with_forward_slashes() {
        let i = installer(Path::new("/tmp"));
        assert_eq!(
            i.hook_command("Stop"),
            "\"C:/Users/me/AppData/Local/CodeNotch/bin/codenotch-hook.exe\" Stop"
        );
    }

    #[test]
    fn merging_keeps_every_other_setting_and_every_foreign_hook() {
        let existing = json!({
            "model": "claude-opus-5",
            "theme": "dark",
            "enabledPlugins": ["a", "b"],
            "hooks": {
                "PreToolUse": [
                    { "hooks": [{ "type": "command", "command": "someone-elses-tool.exe" }] }
                ],
                "SomeEventWeDoNotTouch": [
                    { "hooks": [{ "type": "command", "command": "keep-me.exe" }] }
                ]
            }
        });

        let after = merged(&existing, cmd);
        assert_eq!(after["model"], "claude-opus-5");
        assert_eq!(after["enabledPlugins"], json!(["a", "b"]));
        let pre = after["hooks"]["PreToolUse"].as_array().unwrap();
        assert!(pre
            .iter()
            .any(|e| e.to_string().contains("someone-elses-tool.exe")));
        assert!(pre.iter().any(entry_is_ours));
        for (event, _) in HOOK_EVENTS {
            assert!(
                after["hooks"][*event]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(entry_is_ours),
                "{event} missing"
            );
        }

        // Removing ours gives back exactly what was there.
        assert_eq!(without_ours(&after), existing);
    }

    #[test]
    fn installing_twice_does_not_duplicate_our_entries() {
        let once = merged(&json!({}), cmd);
        let twice = merged(&once, cmd);
        assert_eq!(once, twice);
    }

    #[test]
    fn the_permission_hook_waits_longer_than_the_rest() {
        let after = merged(&json!({}), cmd);
        assert_eq!(
            after["hooks"]["PermissionRequest"][0]["hooks"][0]["timeout"],
            120
        );
        assert_eq!(after["hooks"]["Stop"][0]["hooks"][0]["timeout"], 10);
    }

    #[test]
    fn a_fingerprint_notices_any_change() {
        assert_eq!(fingerprint(b"{}"), fingerprint(b"{}"));
        assert_ne!(fingerprint(b"{}"), fingerprint(b"{ }"));
        assert_ne!(fingerprint(b""), fingerprint(b"{}"));
    }

    #[test]
    fn a_diff_marks_only_what_changed() {
        assert_eq!(unified_diff("a\nb", "a\nb"), "No change.");
        let d = unified_diff("a\nb\nc", "a\nB\nc");
        assert!(d.contains("- b"));
        assert!(d.contains("+ B"));
        assert!(d.contains("  a"));
    }

    #[test]
    fn writing_backs_up_preserves_and_refuses_a_changed_file() {
        let tmp = tempfile::tempdir().unwrap();
        let i = installer(tmp.path());
        let path = i.settings_path().to_path_buf();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let now = Local::now();

        // A real-shaped file, written the way PowerShell 5 would: UTF-8 with BOM.
        let original = r#"{"model":"claude-opus-5","theme":"dark","tui":{"x":1},"hooks":{"PreToolUse":[{"hooks":[{"type":"command","command":"other-tool.exe"}]}]}}"#;
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(original.as_bytes());
        std::fs::write(&path, &bytes).unwrap();
        assert!(!i.status().installed);

        let plan = i
            .preview(true, now)
            .expect("a BOM must not stop the preview");
        assert!(plan.diff.contains(MARKER), "the diff shows what changes");
        let backup = i.write(true, &plan.fingerprint, now).expect("install");

        // The backup is the original, byte for byte.
        assert_eq!(std::fs::read(&backup).unwrap(), bytes);

        let after: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(after["model"], "claude-opus-5");
        assert_eq!(after["tui"]["x"], 1);
        assert!(after["hooks"]["PreToolUse"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e.to_string().contains("other-tool.exe")));
        assert!(i.status().installed);

        // A file that moved since the preview is refused and left alone.
        let stale = i.preview(false, now).unwrap();
        std::fs::write(&path, br#"{"model":"someone-else-edited-this"}"#).unwrap();
        let err = i.write(false, &stale.fingerprint, now).unwrap_err();
        assert!(err.contains("changed since the preview"), "got: {err}");
        let untouched: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(untouched["model"], "someone-else-edited-this");

        // Content we cannot parse is refused before anything is written.
        std::fs::write(&path, b"{ broken").unwrap();
        assert!(i.preview(true, now).is_err());
        assert!(i.write(true, "whatever", now).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"{ broken");
    }

    #[test]
    fn uninstalling_restores_the_original_settings() {
        let tmp = tempfile::tempdir().unwrap();
        let i = installer(tmp.path());
        let path = i.settings_path().to_path_buf();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, br#"{"model":"opus"}"#).unwrap();
        let now = Local::now();

        let plan = i.preview(true, now).unwrap();
        i.write(true, &plan.fingerprint, now).unwrap();
        let plan = i.preview(false, now).unwrap();
        assert!(!plan.install);
        i.write(false, &plan.fingerprint, now).unwrap();

        let after: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(after, json!({"model": "opus"}));
        assert!(!i.status().installed);
    }

    #[test]
    fn a_missing_file_installs_from_nothing_without_a_backup() {
        let tmp = tempfile::tempdir().unwrap();
        let i = installer(tmp.path());
        let now = Local::now();
        let plan = i.preview(true, now).unwrap();
        let backup = i.write(true, &plan.fingerprint, now).unwrap();
        assert!(backup.is_empty());
        assert!(i.status().installed);
    }
}
