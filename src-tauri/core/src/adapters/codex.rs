//! Codex CLI adapter.
//!
//! Codex writes rollout transcripts under `%USERPROFILE%\.codex\sessions\`, and
//! usefully embeds the server's own rate-limit snapshot in its `token_count`
//! events. That means we get real percentages and reset windows without any
//! extra authentication — we just read the newest snapshot it recorded.
//!
//! Multiple profiles are supported the way Codex itself does it: `~/.codex` plus
//! any `~/.codex-<slug>` sibling directories.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde_json::Value as Json;

use crate::adapters::{home_dir, newest_files, parse_timestamp, project_label, tail_lines};
use crate::model::{
    Activity, Health, ProviderId, ProviderSnapshot, Session, UsageUnit, UsageWindow,
};

const MAX_TRANSCRIPTS: usize = 10;
const TRANSCRIPT_TAIL_BYTES: u64 = 256 * 1024;
const SESSION_WINDOW_HOURS: i64 = 12;
const GENERATING_WITHIN_SECS: i64 = 25;
const DONE_WITHIN_MINS: i64 = 10;

/// A Codex profile directory (`~/.codex`, `~/.codex-work`, ...).
#[derive(Debug, Clone)]
pub struct CodexProfile {
    pub root: PathBuf,
    /// `None` for the default profile.
    pub name: Option<String>,
}

impl CodexProfile {
    pub fn sessions_dir(&self) -> PathBuf {
        self.root.join("sessions")
    }
}

/// Find every Codex profile under `home`.
pub fn discover_profiles(home: &Path) -> Vec<CodexProfile> {
    let mut out = Vec::new();

    let default = home.join(".codex");
    if default.is_dir() {
        out.push(CodexProfile {
            root: default,
            name: None,
        });
    }

    if let Ok(entries) = std::fs::read_dir(home) {
        for entry in entries.filter_map(|e| e.ok()) {
            let name = entry.file_name().to_string_lossy().into_owned();
            if let Some(slug) = name.strip_prefix(".codex-") {
                if entry.path().is_dir() && !slug.is_empty() {
                    out.push(CodexProfile {
                        root: entry.path(),
                        name: Some(slug.to_string()),
                    });
                }
            }
        }
    }

    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Turn a rate-limit window's length in minutes into a short label.
fn window_label(minutes: Option<i64>) -> String {
    match minutes {
        Some(m) if m % (60 * 24) == 0 => format!("{}d", m / (60 * 24)),
        Some(m) if m % 60 == 0 => format!("{}h", m / 60),
        Some(m) => format!("{m}m"),
        None => "window".to_string(),
    }
}

/// Parse the `rate_limits` object Codex records verbatim from the API.
///
/// Shape: `{"primary": {"used_percent": 12.3, "window_minutes": 300,
/// "resets_in_seconds": 900}, "secondary": {...}}`.
pub fn parse_rate_limits(value: &Json, now: DateTime<Utc>) -> Vec<UsageWindow> {
    let Some(map) = value.as_object() else {
        return Vec::new();
    };

    let mut windows: Vec<(i64, UsageWindow)> = Vec::new();

    for (key, entry) in map {
        let Some(obj) = entry.as_object() else {
            continue;
        };

        let pct = obj
            .get("used_percent")
            .or_else(|| obj.get("usedPercent"))
            .and_then(Json::as_f64);
        let Some(pct) = pct else { continue };

        let minutes = obj
            .get("window_minutes")
            .or_else(|| obj.get("windowMinutes"))
            .and_then(Json::as_i64);

        let resets_at = obj
            .get("resets_in_seconds")
            .or_else(|| obj.get("resetsInSeconds"))
            .and_then(Json::as_i64)
            .map(|secs| now + chrono::Duration::seconds(secs))
            .or_else(|| {
                obj.get("resets_at")
                    .or_else(|| obj.get("resetsAt"))
                    .and_then(parse_timestamp)
            });

        windows.push((
            // Sort by window length so the short window leads; unknown last.
            minutes.unwrap_or(i64::MAX),
            UsageWindow::new(key.clone(), window_label(minutes))
                .with_pct(pct as f32)
                .with_reset(resets_at),
        ));
    }

    windows.sort_by_key(|(minutes, _)| *minutes);
    windows.into_iter().map(|(_, w)| w).collect()
}

/// What one rollout transcript tells us.
#[derive(Debug, Clone, PartialEq)]
pub struct RolloutSummary {
    pub id: String,
    pub project: Option<String>,
    pub model: Option<String>,
    pub total_tokens: Option<u64>,
    pub last_activity: Option<DateTime<Utc>>,
    pub activity: Activity,
    pub rate_limits: Vec<UsageWindow>,
}

/// Read a rollout transcript's tail: the newest rate-limit snapshot, the token
/// total, and whether the agent is mid-flight or waiting on an approval.
pub fn summarise_rollout(path: &Path, now: DateTime<Utc>) -> Option<RolloutSummary> {
    let lines = tail_lines(path, TRANSCRIPT_TAIL_BYTES).ok()?;
    if lines.is_empty() {
        return None;
    }

    let mut last_activity = None;
    let mut rate_limits = Vec::new();
    let mut total_tokens = None;
    let mut model = None;
    let mut project = None;
    // Tracks an approval prompt: a tool call with no matching output after it.
    let mut pending_call = false;

    for line in &lines {
        let Ok(json) = serde_json::from_str::<Json>(line) else {
            continue;
        };

        if let Some(ts) = json.get("timestamp").and_then(parse_timestamp) {
            last_activity = Some(ts);
        }

        if let Some(limits) = json.get("rate_limits") {
            let parsed = parse_rate_limits(limits, now);
            if !parsed.is_empty() {
                rate_limits = parsed; // keep only the newest snapshot
            }
        }

        if let Some(info) = json.get("info") {
            if let Some(total) = info
                .get("total_token_usage")
                .and_then(|u| u.get("total_tokens"))
                .and_then(Json::as_u64)
            {
                total_tokens = Some(total);
            }
            if let Some(m) = info.get("model").and_then(Json::as_str) {
                model = Some(m.to_string());
            }
        }

        // Session metadata line, written when the session starts.
        if let Some(payload) = json.get("payload").or(Some(&json)) {
            if let Some(cwd) = payload.get("cwd").and_then(Json::as_str) {
                project = Some(project_label(cwd));
            }
            if let Some(m) = payload.get("model").and_then(Json::as_str) {
                model.get_or_insert_with(|| m.to_string());
            }
        }

        match json.get("type").and_then(Json::as_str) {
            Some("function_call") | Some("local_shell_call") => pending_call = true,
            Some("function_call_output") | Some("local_shell_call_output") => pending_call = false,
            _ => {}
        }
    }

    let activity = classify(pending_call, last_activity, now);

    Some(RolloutSummary {
        id: path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "session".into()),
        project,
        model,
        total_tokens,
        last_activity,
        activity,
        rate_limits,
    })
}

fn classify(
    pending_call: bool,
    last_activity: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> Activity {
    let Some(last) = last_activity else {
        return Activity::Idle;
    };
    let age = now.signed_duration_since(last);
    if age < chrono::Duration::zero() {
        return Activity::Generating;
    }
    if pending_call && age < chrono::Duration::hours(1) {
        return Activity::AwaitingInput;
    }
    if age < chrono::Duration::seconds(GENERATING_WITHIN_SECS) {
        Activity::Generating
    } else if age < chrono::Duration::minutes(DONE_WITHIN_MINS) {
        Activity::Done
    } else {
        Activity::Idle
    }
}

/// Collects Codex usage.
pub struct CodexAdapter {
    home: Option<PathBuf>,
}

impl CodexAdapter {
    pub fn new() -> Self {
        Self { home: home_dir() }
    }

    pub fn with_home(home: PathBuf) -> Self {
        Self { home: Some(home) }
    }

    pub fn collect(&mut self, now: DateTime<Utc>) -> ProviderSnapshot {
        let Some(home) = &self.home else {
            return ProviderSnapshot::degraded(
                ProviderId::Codex,
                Health::Unavailable,
                "no home directory",
            );
        };

        let profiles = discover_profiles(home);
        if profiles.is_empty() {
            return ProviderSnapshot::degraded(
                ProviderId::Codex,
                Health::Unavailable,
                "Codex not found",
            );
        }

        let cutoff = now - chrono::Duration::hours(SESSION_WINDOW_HOURS);
        let mut windows: Vec<UsageWindow> = Vec::new();
        let mut newest_limits_at: Option<DateTime<Utc>> = None;
        let mut sessions = Vec::new();

        for profile in &profiles {
            let transcripts = newest_files(&profile.sessions_dir(), "jsonl", MAX_TRANSCRIPTS);
            for path in transcripts {
                let Some(summary) = summarise_rollout(&path, now) else {
                    continue;
                };

                // Keep the rate limits from whichever transcript is freshest:
                // they're account-wide, so the newest snapshot is the truth.
                if !summary.rate_limits.is_empty() && summary.last_activity > newest_limits_at {
                    newest_limits_at = summary.last_activity;
                    windows = summary.rate_limits.clone();
                }

                if summary.last_activity.is_none_or(|t| t < cutoff) {
                    continue;
                }

                let title = match (&summary.project, &profile.name) {
                    (Some(p), Some(n)) => format!("{p} ({n})"),
                    (Some(p), None) => p.clone(),
                    (None, Some(n)) => format!("Codex ({n})"),
                    (None, None) => "Codex".to_string(),
                };

                sessions.push(Session {
                    id: summary.id,
                    title,
                    cwd: summary.project.clone(),
                    model: summary.model,
                    activity: summary.activity,
                    last_activity: summary.last_activity,
                    tokens: summary.total_tokens,
                    detail: None,
                });
            }
        }

        sessions.sort_by(|a, b| {
            b.activity
                .rank()
                .cmp(&a.activity.rank())
                .then(b.last_activity.cmp(&a.last_activity))
        });
        sessions.truncate(8);

        // A total token count is worth showing even with no rate-limit snapshot,
        // but it's derived locally so it's flagged as an estimate.
        if windows.is_empty() {
            let tokens: u64 = sessions.iter().filter_map(|s| s.tokens).sum();
            if tokens > 0 {
                windows.push(
                    UsageWindow::new("tokens", "Tokens")
                        .with_counts(tokens as f64, None)
                        .with_unit(UsageUnit::Tokens)
                        .estimated(),
                );
            }
        }

        let account = profiles
            .iter()
            .filter_map(|p| p.name.clone())
            .collect::<Vec<_>>()
            .join(", ");

        let mut snap = ProviderSnapshot::new(ProviderId::Codex)
            .with_source("sessions")
            .with_account(Some(account).filter(|s| !s.is_empty()))
            .with_windows(windows)
            .with_sessions(sessions);

        if snap.windows.is_empty() {
            snap.health = Health::Stale;
            snap.detail = Some("No recent Codex activity to read limits from".into());
        }
        snap
    }
}

impl Default for CodexAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    const NOW: &str = "2026-01-01T12:00:00Z";

    #[test]
    fn parses_the_rate_limit_snapshot() {
        let json: Json = serde_json::from_str(
            r#"{
              "primary":   {"used_percent": 12.5, "window_minutes": 300,   "resets_in_seconds": 900},
              "secondary": {"used_percent": 60.0, "window_minutes": 10080, "resets_in_seconds": 3600}
            }"#,
        )
        .unwrap();

        let windows = parse_rate_limits(&json, at(NOW));
        assert_eq!(windows.len(), 2);

        // Shortest window first.
        assert_eq!(windows[0].key, "primary");
        assert_eq!(windows[0].label, "5h");
        assert_eq!(windows[0].used_pct, Some(12.5));
        assert_eq!(windows[0].resets_at, Some(at("2026-01-01T12:15:00Z")));

        assert_eq!(windows[1].label, "7d");
        assert_eq!(windows[1].used_pct, Some(60.0));
    }

    #[test]
    fn window_labels_read_naturally() {
        assert_eq!(window_label(Some(300)), "5h");
        assert_eq!(window_label(Some(10080)), "7d");
        assert_eq!(window_label(Some(45)), "45m");
        assert_eq!(window_label(None), "window");
    }

    #[test]
    fn rate_limits_without_a_percentage_are_skipped() {
        let json: Json = serde_json::from_str(r#"{"primary":{"window_minutes":300}}"#).unwrap();
        assert!(parse_rate_limits(&json, at(NOW)).is_empty());
        assert!(parse_rate_limits(&Json::Null, at(NOW)).is_empty());
    }

    fn write_rollout(dir: &Path, name: &str, lines: &[String]) -> PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        let path = dir.join(name);
        let mut f = std::fs::File::create(&path).unwrap();
        for line in lines {
            writeln!(f, "{line}").unwrap();
        }
        path
    }

    fn ts(offset: i64) -> String {
        (at(NOW) + chrono::Duration::seconds(offset)).to_rfc3339()
    }

    #[test]
    fn reads_the_newest_rate_limits_from_a_rollout() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_rollout(
            dir.path(),
            "rollout-1.jsonl",
            &[
                format!(
                    r#"{{"type":"token_count","timestamp":"{}","rate_limits":{{"primary":{{"used_percent":10,"window_minutes":300,"resets_in_seconds":600}}}}}}"#,
                    ts(-600)
                ),
                format!(
                    r#"{{"type":"token_count","timestamp":"{}","rate_limits":{{"primary":{{"used_percent":33,"window_minutes":300,"resets_in_seconds":300}}}},"info":{{"total_token_usage":{{"total_tokens":4242}},"model":"gpt-5-codex"}}}}"#,
                    ts(-30)
                ),
            ],
        );

        let s = summarise_rollout(&path, at(NOW)).unwrap();
        assert_eq!(s.rate_limits.len(), 1);
        assert_eq!(
            s.rate_limits[0].used_pct,
            Some(33.0),
            "the later snapshot must win"
        );
        assert_eq!(s.total_tokens, Some(4242));
        assert_eq!(s.model.as_deref(), Some("gpt-5-codex"));
    }

    #[test]
    fn detects_a_pending_tool_call_as_waiting_for_approval() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_rollout(
            dir.path(),
            "r.jsonl",
            &[format!(
                r#"{{"type":"function_call","timestamp":"{}","name":"shell"}}"#,
                ts(-40)
            )],
        );
        assert_eq!(
            summarise_rollout(&path, at(NOW)).unwrap().activity,
            Activity::AwaitingInput
        );
    }

    #[test]
    fn a_completed_tool_call_is_not_waiting() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_rollout(
            dir.path(),
            "r.jsonl",
            &[
                format!(r#"{{"type":"function_call","timestamp":"{}"}}"#, ts(-60)),
                format!(
                    r#"{{"type":"function_call_output","timestamp":"{}"}}"#,
                    ts(-55)
                ),
            ],
        );
        assert_eq!(
            summarise_rollout(&path, at(NOW)).unwrap().activity,
            Activity::Done
        );
    }

    #[test]
    fn activity_reflects_recency() {
        let now = at(NOW);
        assert_eq!(
            classify(false, Some(now - chrono::Duration::seconds(3)), now),
            Activity::Generating
        );
        assert_eq!(
            classify(false, Some(now - chrono::Duration::minutes(2)), now),
            Activity::Done
        );
        assert_eq!(
            classify(false, Some(now - chrono::Duration::days(1)), now),
            Activity::Idle
        );
        // A tool call from yesterday isn't a live prompt.
        assert_eq!(
            classify(true, Some(now - chrono::Duration::days(1)), now),
            Activity::Idle
        );
    }

    #[test]
    fn discovers_the_default_and_named_profiles() {
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(home.path().join(".codex/sessions")).unwrap();
        std::fs::create_dir_all(home.path().join(".codex-work/sessions")).unwrap();
        std::fs::create_dir_all(home.path().join(".codex-oss/sessions")).unwrap();
        // Not a profile.
        std::fs::create_dir_all(home.path().join(".codexfoo")).unwrap();

        let profiles = discover_profiles(home.path());
        assert_eq!(profiles.len(), 3);
        assert_eq!(profiles[0].name, None, "default profile sorts first");
        let names: Vec<_> = profiles.iter().filter_map(|p| p.name.clone()).collect();
        assert_eq!(names, vec!["oss", "work"]);
    }

    #[test]
    fn a_missing_install_reports_unavailable() {
        let home = tempfile::tempdir().unwrap();
        let mut adapter = CodexAdapter::with_home(home.path().to_path_buf());
        let snap = adapter.collect(at(NOW));
        assert_eq!(snap.health, Health::Unavailable);
    }

    #[test]
    fn collects_limits_and_sessions_across_profiles() {
        let home = tempfile::tempdir().unwrap();
        let sessions = home.path().join(".codex/sessions/2026/01/01");
        write_rollout(
            &sessions,
            "rollout-a.jsonl",
            &[format!(
                r#"{{"type":"token_count","timestamp":"{}","cwd":"C:\\dev\\api","rate_limits":{{"primary":{{"used_percent":75,"window_minutes":300,"resets_in_seconds":1800}}}},"info":{{"total_token_usage":{{"total_tokens":100}}}}}}"#,
                ts(-10)
            )],
        );

        let mut adapter = CodexAdapter::with_home(home.path().to_path_buf());
        let snap = adapter.collect(at(NOW));

        assert_eq!(snap.health, Health::Ok);
        assert_eq!(snap.windows.len(), 1);
        assert_eq!(snap.windows[0].used_pct, Some(75.0));
        assert!(!snap.windows[0].estimated, "API figures aren't estimates");
        assert_eq!(snap.activity, Activity::Generating);
        assert_eq!(snap.sessions[0].title, "api");
    }

    #[test]
    fn falls_back_to_a_token_estimate_when_no_snapshot_exists() {
        let home = tempfile::tempdir().unwrap();
        write_rollout(
            &home.path().join(".codex/sessions"),
            "r.jsonl",
            &[format!(
                r#"{{"type":"token_count","timestamp":"{}","info":{{"total_token_usage":{{"total_tokens":777}}}}}}"#,
                ts(-30)
            )],
        );

        let mut adapter = CodexAdapter::with_home(home.path().to_path_buf());
        let snap = adapter.collect(at(NOW));
        assert_eq!(snap.windows.len(), 1);
        assert_eq!(snap.windows[0].used, Some(777.0));
        assert!(snap.windows[0].estimated);
        assert_eq!(snap.windows[0].used_pct, None);
    }

    #[test]
    fn an_installed_but_idle_codex_is_marked_stale_not_wrong() {
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(home.path().join(".codex/sessions")).unwrap();

        let mut adapter = CodexAdapter::with_home(home.path().to_path_buf());
        let snap = adapter.collect(at(NOW));
        assert_eq!(snap.health, Health::Stale);
        assert!(snap.windows.is_empty());
        assert!(snap.detail.is_some());
    }
}
