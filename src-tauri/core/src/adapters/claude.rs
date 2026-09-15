//! Claude Code adapter.
//!
//! Primary source is Claude's OAuth usage endpoint, which reports the real
//! rolling-window utilisation. Credentials come from
//! `%USERPROFILE%\.claude\.credentials.json`, or from Windows Credential
//! Manager when the install opted into OS-backed storage.
//!
//! When there is no usable token (or the endpoint is rate limiting us) we fall
//! back to the local session transcripts under `%USERPROFILE%\.claude\projects\`.
//! Those give token counts and, importantly, the live activity state: whether an
//! agent is generating, finished, or parked on a `[y/N]` approval prompt.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde_json::Value as Json;

use crate::adapters::{
    home_dir, newest_files, parse_timestamp, project_label, tail_lines, Backoff,
};
use crate::model::{
    Activity, Health, ProviderId, ProviderSnapshot, Session, UsageUnit, UsageWindow,
};
use crate::secrets::{read_generic_credential, CLAUDE_CREDENTIAL_TARGETS};

/// Anthropic's OAuth usage endpoint.
const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
/// Beta header Claude Code sends with OAuth-authenticated calls.
const OAUTH_BETA: &str = "oauth-2025-04-20";

/// How far back a transcript counts as "the current session window".
const SESSION_WINDOW_HOURS: i64 = 5;
/// Newer than this and we consider the agent to be actively generating.
const GENERATING_WITHIN_SECS: i64 = 25;
/// Newer than this and a finished session is still worth surfacing.
const DONE_WITHIN_MINS: i64 = 10;
/// Only the newest transcripts matter; scanning every project is wasteful.
const MAX_TRANSCRIPTS: usize = 12;
/// Tail window per transcript. Enough for the last few exchanges.
const TRANSCRIPT_TAIL_BYTES: u64 = 256 * 1024;

/// OAuth credentials as Claude Code stores them.
#[derive(Debug, Clone, PartialEq)]
pub struct Credentials {
    pub access_token: String,
    pub expires_at: Option<DateTime<Utc>>,
    pub subscription: Option<String>,
}

impl Credentials {
    pub fn is_expired(&self, now: DateTime<Utc>) -> bool {
        self.expires_at.is_some_and(|e| now >= e)
    }
}

/// Parse `.credentials.json` (or the Credential Manager blob, which is the same
/// JSON).
///
/// Tolerant of both the nested `claudeAiOauth` envelope and a flat object,
/// because the shape has moved between releases.
pub fn parse_credentials(raw: &str) -> Option<Credentials> {
    let root: Json = serde_json::from_str(raw).ok()?;
    let obj = root
        .get("claudeAiOauth")
        .or_else(|| root.get("oauth"))
        .unwrap_or(&root);

    let access_token = obj
        .get("accessToken")
        .or_else(|| obj.get("access_token"))
        .and_then(Json::as_str)
        .filter(|s| !s.is_empty())?
        .to_string();

    Some(Credentials {
        access_token,
        expires_at: obj
            .get("expiresAt")
            .or_else(|| obj.get("expires_at"))
            .and_then(parse_timestamp),
        subscription: obj
            .get("subscriptionType")
            .or_else(|| obj.get("subscription_type"))
            .and_then(Json::as_str)
            .map(str::to_string),
    })
}

/// Turn a `subscriptionType` into something worth putting on screen.
fn plan_label(subscription: Option<&str>) -> Option<String> {
    let s = subscription?;
    Some(match s.to_ascii_lowercase().as_str() {
        "max" => "Max".to_string(),
        "pro" => "Pro".to_string(),
        "team" => "Team".to_string(),
        "enterprise" => "Enterprise".to_string(),
        "free" => "Free".to_string(),
        other => {
            let mut c = other.chars();
            let first = c.next()?;
            first.to_uppercase().collect::<String>() + c.as_str()
        }
    })
}

/// Friendly labels for the windows the endpoint is known to return. Anything
/// unrecognised still renders, using a prettified version of its key.
fn window_label(key: &str) -> String {
    match key {
        "five_hour" => "5h session".to_string(),
        "seven_day" => "7d all models".to_string(),
        "seven_day_opus" => "7d Opus".to_string(),
        "seven_day_oauth_apps" => "7d apps".to_string(),
        other => other
            .split(['_', '-'])
            .filter(|s| !s.is_empty())
            .map(|w| {
                let mut c = w.chars();
                match c.next() {
                    Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                    None => String::new(),
                }
            })
            .collect::<Vec<_>>()
            .join(" "),
    }
}

/// Parse the usage endpoint's body into windows.
///
/// The endpoint is undocumented and its shape has changed more than once, so
/// this walks whatever object it gets and picks up any entry that looks like a
/// window, rather than binding to one exact schema. An unparseable body yields
/// an empty list, which the caller surfaces as an error instead of a number.
pub fn parse_usage(raw: &str) -> Vec<UsageWindow> {
    let Ok(root) = serde_json::from_str::<Json>(raw) else {
        return Vec::new();
    };

    // Accept `{...}`, `{"usage": {...}}` and `{"windows": [...]}`.
    let mut candidates: Vec<(String, &Json)> = Vec::new();
    let container = root.get("usage").unwrap_or(&root);

    if let Some(list) = container.get("windows").and_then(Json::as_array) {
        for (i, item) in list.iter().enumerate() {
            let key = item
                .get("key")
                .or_else(|| item.get("name"))
                .and_then(Json::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| format!("window_{i}"));
            candidates.push((key, item));
        }
    } else if let Some(map) = container.as_object() {
        for (key, value) in map {
            if value.is_object() {
                candidates.push((key.clone(), value));
            }
        }
    }

    let mut windows: Vec<UsageWindow> = candidates
        .into_iter()
        .filter_map(|(key, value)| parse_window(&key, value))
        .collect();

    // Stable, meaningful order: shortest window first is what a user scans for.
    windows.sort_by_key(|w| window_rank(&w.key));
    windows
}

fn window_rank(key: &str) -> u8 {
    match key {
        "five_hour" => 0,
        "seven_day" => 1,
        "seven_day_opus" => 2,
        "seven_day_oauth_apps" => 3,
        _ => 4,
    }
}

fn parse_window(key: &str, value: &Json) -> Option<UsageWindow> {
    let mut window = UsageWindow::new(key, window_label(key));

    let pct = value
        .get("utilization")
        .or_else(|| value.get("used_percent"))
        .or_else(|| value.get("percent"))
        .or_else(|| value.get("percentage"))
        .and_then(Json::as_f64);

    let used = value
        .get("used")
        .or_else(|| value.get("used_tokens"))
        .and_then(Json::as_f64);
    let limit = value
        .get("limit")
        .or_else(|| value.get("total"))
        .and_then(Json::as_f64);

    match (pct, used) {
        (Some(p), _) => {
            window = window.with_pct(p as f32);
            // Keep raw counts alongside a reported percentage when both exist.
            if let Some(u) = used {
                window.used = Some(u);
                window.limit = limit;
            }
        }
        (None, Some(u)) => {
            window = window.with_counts(u, limit).with_unit(UsageUnit::Tokens);
        }
        // Neither a percentage nor a count: nothing to show, so skip it rather
        // than rendering an empty ring.
        (None, None) => return None,
    }

    let resets = value
        .get("resets_at")
        .or_else(|| value.get("reset_at"))
        .or_else(|| value.get("resetsAt"))
        .and_then(parse_timestamp);

    Some(window.with_reset(resets))
}

/// One line of a Claude Code transcript, reduced to what we care about.
#[derive(Debug, Default, Clone)]
struct TranscriptEntry {
    timestamp: Option<DateTime<Utc>>,
    cwd: Option<String>,
    model: Option<String>,
    tokens: u64,
    /// The assistant asked to run a tool.
    has_tool_use: bool,
    /// A tool result came back (so the request was approved and ran).
    has_tool_result: bool,
}

fn parse_entry(line: &str) -> Option<TranscriptEntry> {
    let json: Json = serde_json::from_str(line).ok()?;
    let mut entry = TranscriptEntry {
        timestamp: json.get("timestamp").and_then(parse_timestamp),
        cwd: json.get("cwd").and_then(Json::as_str).map(str::to_string),
        ..Default::default()
    };

    if let Some(message) = json.get("message") {
        entry.model = message
            .get("model")
            .and_then(Json::as_str)
            .map(str::to_string);

        if let Some(usage) = message.get("usage") {
            // Cache reads are charged differently but still count towards the
            // window, so include every bucket the transcript reports.
            for field in [
                "input_tokens",
                "output_tokens",
                "cache_creation_input_tokens",
                "cache_read_input_tokens",
            ] {
                entry.tokens += usage.get(field).and_then(Json::as_u64).unwrap_or(0);
            }
        }

        if let Some(content) = message.get("content").and_then(Json::as_array) {
            for block in content {
                match block.get("type").and_then(Json::as_str) {
                    Some("tool_use") => entry.has_tool_use = true,
                    Some("tool_result") => entry.has_tool_result = true,
                    _ => {}
                }
            }
        }
    }

    Some(entry)
}

/// What a single transcript tells us.
#[derive(Debug, Clone, PartialEq)]
pub struct TranscriptSummary {
    pub session_id: String,
    pub project: String,
    pub cwd: Option<String>,
    pub model: Option<String>,
    pub tokens: u64,
    pub last_activity: Option<DateTime<Utc>>,
    pub activity: Activity,
}

/// Read one transcript's tail and classify what the session is doing.
///
/// The approval heuristic is the interesting part: when the last thing in the
/// transcript is an assistant turn requesting a tool, and no tool result has
/// followed it, Claude Code is sitting at a permission prompt waiting for the
/// user. That is the state worth interrupting someone for.
pub fn summarise_transcript(path: &Path, now: DateTime<Utc>) -> Option<TranscriptSummary> {
    let lines = tail_lines(path, TRANSCRIPT_TAIL_BYTES).ok()?;
    let entries: Vec<TranscriptEntry> = lines.iter().filter_map(|l| parse_entry(l)).collect();
    if entries.is_empty() {
        return None;
    }

    let session_id = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "session".into());

    let cwd = entries.iter().rev().find_map(|e| e.cwd.clone());
    let project = cwd
        .as_deref()
        .map(project_label)
        // Fall back to the encoded directory name Claude Code derives from cwd.
        .or_else(|| {
            path.parent()
                .and_then(|p| p.file_name())
                .map(|s| decode_project_dir(&s.to_string_lossy()))
        })
        .unwrap_or_else(|| "session".into());

    let window_start = now - chrono::Duration::hours(SESSION_WINDOW_HOURS);
    let tokens = entries
        .iter()
        .filter(|e| e.timestamp.is_none_or(|t| t >= window_start))
        .map(|e| e.tokens)
        .sum();

    let last_activity = entries.iter().rev().find_map(|e| e.timestamp);
    let model = entries.iter().rev().find_map(|e| e.model.clone());

    let activity = classify(&entries, last_activity, now);

    Some(TranscriptSummary {
        session_id,
        project,
        cwd,
        model,
        tokens,
        last_activity,
        activity,
    })
}

fn classify(
    entries: &[TranscriptEntry],
    last_activity: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> Activity {
    let Some(last_activity) = last_activity else {
        return Activity::Idle;
    };
    let age = now.signed_duration_since(last_activity);
    // A timestamp in the future means a clock skew we shouldn't act on.
    if age < chrono::Duration::zero() {
        return Activity::Generating;
    }

    // An assistant tool request with no result after it means Claude Code is
    // parked at a permission prompt.
    let awaiting = entries
        .iter()
        .rev()
        .find(|e| e.has_tool_use || e.has_tool_result)
        .is_some_and(|e| e.has_tool_use && !e.has_tool_result);

    if awaiting && age < chrono::Duration::hours(1) {
        return Activity::AwaitingInput;
    }
    if age < chrono::Duration::seconds(GENERATING_WITHIN_SECS) {
        return Activity::Generating;
    }
    if age < chrono::Duration::minutes(DONE_WITHIN_MINS) {
        return Activity::Done;
    }
    Activity::Idle
}

/// Claude Code names project folders after the cwd with separators replaced by
/// dashes (`C:\Users\dev\app` -> `C--Users-dev-app`). We can't reverse that
/// unambiguously, so just take the trailing segment.
fn decode_project_dir(name: &str) -> String {
    name.rsplit('-')
        .find(|s| !s.is_empty())
        .unwrap_or(name)
        .to_string()
}

/// Filesystem locations the adapter reads. Injectable so tests don't need a
/// real `%USERPROFILE%`.
#[derive(Debug, Clone)]
pub struct ClaudePaths {
    pub root: PathBuf,
}

impl ClaudePaths {
    pub fn detect() -> Option<Self> {
        home_dir().map(|h| Self {
            root: h.join(".claude"),
        })
    }

    pub fn credentials_file(&self) -> PathBuf {
        self.root.join(".credentials.json")
    }

    pub fn projects_dir(&self) -> PathBuf {
        self.root.join("projects")
    }

    pub fn exists(&self) -> bool {
        self.root.is_dir()
    }
}

/// Collects Claude Code usage.
pub struct ClaudeAdapter {
    http: reqwest::Client,
    backoff: Backoff,
    paths: Option<ClaudePaths>,
    /// Last good reading, reused while we're backing off so the HUD keeps
    /// showing real numbers instead of blanking.
    last_good: Option<(DateTime<Utc>, Vec<UsageWindow>, Option<String>)>,
}

impl ClaudeAdapter {
    pub fn new(http: reqwest::Client) -> Self {
        Self {
            http,
            backoff: Backoff::default(),
            paths: ClaudePaths::detect(),
            last_good: None,
        }
    }

    #[cfg(test)]
    pub fn with_paths(http: reqwest::Client, paths: ClaudePaths) -> Self {
        Self {
            http,
            backoff: Backoff::default(),
            paths: Some(paths),
            last_good: None,
        }
    }

    /// Find credentials: the file first, then Credential Manager.
    pub fn credentials(&self) -> Option<Credentials> {
        if let Some(paths) = &self.paths {
            if let Ok(raw) = std::fs::read_to_string(paths.credentials_file()) {
                if let Some(creds) = parse_credentials(&raw) {
                    return Some(creds);
                }
            }
        }
        CLAUDE_CREDENTIAL_TARGETS
            .iter()
            .filter_map(|target| read_generic_credential(target))
            .find_map(|raw| parse_credentials(&raw))
    }

    /// Fetch usage, honouring the back-off schedule.
    ///
    /// Returns `Ok(None)` when we're deliberately holding off.
    async fn fetch_usage(
        &mut self,
        token: &str,
        now: DateTime<Utc>,
    ) -> anyhow::Result<Option<Vec<UsageWindow>>> {
        if !self.backoff.ready_at(now) {
            return Ok(None);
        }

        let response = self
            .http
            .get(USAGE_URL)
            .bearer_auth(token)
            .header("anthropic-beta", OAUTH_BETA)
            .header("accept", "application/json")
            .timeout(Duration::from_secs(10))
            .send()
            .await;

        let response = match response {
            Ok(r) => r,
            Err(err) => {
                self.backoff
                    .record_failure(now, None, rand::random::<f64>());
                return Err(err.into());
            }
        };

        let status = response.status();

        if status.as_u16() == 429 || status.is_server_error() {
            let retry_after = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok())
                .map(Duration::from_secs);
            self.backoff
                .record_failure(now, retry_after, rand::random::<f64>());
            anyhow::bail!("usage endpoint returned {status}");
        }

        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            // Not a rate-limit problem; re-running sooner won't help, but we
            // shouldn't punish the schedule either.
            anyhow::bail!("unauthorized ({status})");
        }

        if !status.is_success() {
            self.backoff
                .record_failure(now, None, rand::random::<f64>());
            anyhow::bail!("usage endpoint returned {status}");
        }

        let body = response.text().await?;
        self.backoff.record_success();

        let windows = parse_usage(&body);
        if windows.is_empty() {
            anyhow::bail!("usage response had no recognisable windows");
        }
        Ok(Some(windows))
    }

    /// Scan local transcripts for sessions and their live state.
    pub fn scan_sessions(&self, now: DateTime<Utc>) -> Vec<Session> {
        let Some(paths) = &self.paths else {
            return Vec::new();
        };
        let cutoff = now - chrono::Duration::hours(SESSION_WINDOW_HOURS);

        let mut sessions: Vec<Session> =
            newest_files(&paths.projects_dir(), "jsonl", MAX_TRANSCRIPTS)
                .into_iter()
                .filter_map(|p| summarise_transcript(&p, now))
                .filter(|s| s.last_activity.is_none_or(|t| t >= cutoff))
                .map(|s| Session {
                    id: s.session_id,
                    title: s.project,
                    cwd: s.cwd,
                    model: s.model,
                    activity: s.activity,
                    last_activity: s.last_activity,
                    tokens: Some(s.tokens).filter(|t| *t > 0),
                    detail: None,
                })
                .collect();

        // Most interesting first: blocked, then generating, then most recent.
        sessions.sort_by(|a, b| {
            b.activity
                .rank()
                .cmp(&a.activity.rank())
                .then(b.last_activity.cmp(&a.last_activity))
        });
        sessions
    }

    /// Estimate the session window from transcript tokens when the API is
    /// unavailable. Always flagged `estimated` so the UI can mark it.
    fn transcript_windows(sessions: &[Session]) -> Vec<UsageWindow> {
        let tokens: u64 = sessions.iter().filter_map(|s| s.tokens).sum();
        if tokens == 0 {
            return Vec::new();
        }
        vec![UsageWindow::new("five_hour_tokens", "5h tokens")
            .with_counts(tokens as f64, None)
            .with_unit(UsageUnit::Tokens)
            .estimated()]
    }

    /// Build the snapshot for this poll.
    pub async fn collect(&mut self, now: DateTime<Utc>) -> ProviderSnapshot {
        let installed = self.paths.as_ref().is_some_and(|p| p.exists());
        let sessions = self.scan_sessions(now);

        let Some(creds) = self.credentials() else {
            if !installed && sessions.is_empty() {
                return ProviderSnapshot::degraded(
                    ProviderId::ClaudeCode,
                    Health::Unavailable,
                    "Claude Code not found",
                );
            }
            // No token, but we can still report local activity and tokens.
            let windows = Self::transcript_windows(&sessions);
            let mut snap = ProviderSnapshot::new(ProviderId::ClaudeCode)
                .with_source("transcripts")
                .with_windows(windows)
                .with_sessions(sessions);
            snap.health = Health::NeedsAuth;
            snap.detail = Some("Sign in with `claude` to show usage limits".into());
            return snap;
        };

        if creds.is_expired(now) {
            let windows = Self::transcript_windows(&sessions);
            let mut snap = ProviderSnapshot::new(ProviderId::ClaudeCode)
                .with_source("transcripts")
                .with_account(plan_label(creds.subscription.as_deref()))
                .with_windows(windows)
                .with_sessions(sessions);
            snap.health = Health::NeedsAuth;
            snap.detail = Some("Token expired; run `claude` to refresh".into());
            return snap;
        }

        let account = plan_label(creds.subscription.as_deref());

        match self.fetch_usage(&creds.access_token, now).await {
            Ok(Some(windows)) => {
                self.last_good = Some((now, windows.clone(), account.clone()));
                ProviderSnapshot::new(ProviderId::ClaudeCode)
                    .with_source("oauth")
                    .with_account(account)
                    .with_windows(windows)
                    .with_sessions(sessions)
            }
            // Backing off: keep showing the last good numbers, flagged.
            Ok(None) => self.degraded_with_last_good(
                Health::RateLimited,
                "Rate limited; retrying shortly",
                sessions,
                now,
            ),
            Err(err) => {
                tracing::debug!(%err, "claude usage fetch failed");
                let health = if err.to_string().contains("unauthorized") {
                    Health::NeedsAuth
                } else if self.backoff.is_backing_off() {
                    Health::RateLimited
                } else {
                    Health::Error
                };
                self.degraded_with_last_good(health, err.to_string(), sessions, now)
            }
        }
    }

    /// Reuse the last good reading rather than blanking the HUD, marking the
    /// snapshot so the user can see the numbers aren't live.
    fn degraded_with_last_good(
        &self,
        health: Health,
        detail: impl Into<String>,
        sessions: Vec<Session>,
        now: DateTime<Utc>,
    ) -> ProviderSnapshot {
        let mut detail = detail.into();
        let (windows, account, source) = match &self.last_good {
            Some((at, windows, account)) => {
                let age = now.signed_duration_since(*at);
                detail = format!("{detail} (showing figures from {} ago)", human_age(age));
                (windows.clone(), account.clone(), "oauth (cached)")
            }
            None => (Self::transcript_windows(&sessions), None, "transcripts"),
        };

        let mut snap = ProviderSnapshot::new(ProviderId::ClaudeCode)
            .with_source(source)
            .with_account(account)
            .with_windows(windows)
            .with_sessions(sessions);
        snap.health = health;
        snap.detail = Some(detail);
        snap.retry_at = self.backoff.next_attempt();
        snap
    }
}

/// "3m", "2h" — compact enough for a one-line status.
fn human_age(age: chrono::Duration) -> String {
    let secs = age.num_seconds().max(0);
    match secs {
        s if s < 60 => format!("{s}s"),
        s if s < 3600 => format!("{}m", s / 60),
        s => format!("{}h", s / 3600),
    }
}

/// Aggregate per-provider token counts keyed by model, for the expanded card.
pub fn tokens_by_model(sessions: &[Session]) -> BTreeMap<String, u64> {
    let mut out = BTreeMap::new();
    for session in sessions {
        if let (Some(model), Some(tokens)) = (&session.model, session.tokens) {
            *out.entry(model.clone()).or_insert(0) += tokens;
        }
    }
    out
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
    fn parses_the_nested_credentials_envelope() {
        let raw = r#"{
          "claudeAiOauth": {
            "accessToken": "sk-ant-oat01-abc",
            "refreshToken": "sk-ant-ort01-xyz",
            "expiresAt": 1798761600000,
            "scopes": ["user:inference"],
            "subscriptionType": "max"
          }
        }"#;
        let creds = parse_credentials(raw).expect("should parse");
        assert_eq!(creds.access_token, "sk-ant-oat01-abc");
        assert_eq!(creds.subscription.as_deref(), Some("max"));
        assert_eq!(creds.expires_at, Some(at("2027-01-01T00:00:00Z")));
    }

    #[test]
    fn parses_a_flat_credentials_object_too() {
        let raw = r#"{"access_token":"tok","expires_at":"2027-01-01T00:00:00Z"}"#;
        let creds = parse_credentials(raw).unwrap();
        assert_eq!(creds.access_token, "tok");
        assert_eq!(creds.expires_at, Some(at("2027-01-01T00:00:00Z")));
    }

    #[test]
    fn rejects_credentials_without_a_token() {
        assert!(parse_credentials(r#"{"claudeAiOauth":{"refreshToken":"x"}}"#).is_none());
        assert!(parse_credentials(r#"{"claudeAiOauth":{"accessToken":""}}"#).is_none());
        assert!(parse_credentials("not json").is_none());
        assert!(parse_credentials("").is_none());
    }

    #[test]
    fn expiry_is_checked_against_the_current_time() {
        let creds = Credentials {
            access_token: "t".into(),
            expires_at: Some(at("2026-01-01T11:00:00Z")),
            subscription: None,
        };
        assert!(creds.is_expired(at(NOW)));
        assert!(!creds.is_expired(at("2026-01-01T10:00:00Z")));

        // No expiry recorded means we optimistically try the token.
        let creds = Credentials {
            expires_at: None,
            ..creds
        };
        assert!(!creds.is_expired(at(NOW)));
    }

    #[test]
    fn parses_the_documented_usage_shape() {
        let raw = r#"{
          "five_hour":  {"utilization": 42,   "resets_at": "2026-01-01T15:00:00Z"},
          "seven_day":  {"utilization": 12.5, "resets_at": "2026-01-05T00:00:00Z"},
          "seven_day_opus": {"utilization": 0, "resets_at": "2026-01-05T00:00:00Z"}
        }"#;
        let windows = parse_usage(raw);
        assert_eq!(windows.len(), 3);

        // Shortest window first.
        assert_eq!(windows[0].key, "five_hour");
        assert_eq!(windows[0].label, "5h session");
        assert_eq!(windows[0].used_pct, Some(42.0));
        assert_eq!(windows[0].resets_at, Some(at("2026-01-01T15:00:00Z")));

        assert_eq!(windows[1].key, "seven_day");
        assert_eq!(windows[1].used_pct, Some(12.5));
        assert!(!windows[0].estimated, "API figures are not estimates");
    }

    #[test]
    fn parses_used_limit_pairs_when_no_percentage_is_given() {
        let raw = r#"{"five_hour": {"used": 25000, "limit": 100000}}"#;
        let windows = parse_usage(raw);
        assert_eq!(windows[0].used_pct, Some(25.0));
        assert_eq!(windows[0].used, Some(25000.0));
        assert_eq!(windows[0].unit, UsageUnit::Tokens);
    }

    #[test]
    fn survives_schema_drift_in_the_undocumented_endpoint() {
        // Wrapped in `usage`, list-shaped, alternative field names.
        let raw = r#"{"usage": {"windows": [
            {"key": "five_hour", "used_percent": 80, "resetsAt": 1767279600}
        ]}}"#;
        let windows = parse_usage(raw);
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].used_pct, Some(80.0));
        assert_eq!(windows[0].resets_at, Some(at("2026-01-01T15:00:00Z")));

        // A brand-new window key still renders with a readable label.
        let windows = parse_usage(r#"{"thirty_day_sonnet": {"utilization": 5}}"#);
        assert_eq!(windows[0].label, "Thirty Day Sonnet");
    }

    #[test]
    fn unparseable_or_empty_usage_yields_nothing_rather_than_zeroes() {
        assert!(parse_usage("not json").is_empty());
        assert!(parse_usage("{}").is_empty());
        // An object with no usable numbers must not become a 0% ring.
        assert!(parse_usage(r#"{"five_hour": {"resets_at": "2026-01-01T15:00:00Z"}}"#).is_empty());
    }

    #[test]
    fn plan_labels_are_title_cased() {
        assert_eq!(plan_label(Some("max")).as_deref(), Some("Max"));
        assert_eq!(plan_label(Some("pro")).as_deref(), Some("Pro"));
        assert_eq!(
            plan_label(Some("something_new")).as_deref(),
            Some("Something_new")
        );
        assert_eq!(plan_label(None), None);
    }

    /// Build a transcript file from `(type, offset_secs, extra_json)` tuples.
    fn write_transcript(dir: &Path, name: &str, entries: &[(&str, i64, &str)]) -> PathBuf {
        let path = dir.join(name);
        let mut f = std::fs::File::create(&path).unwrap();
        for (kind, offset, extra) in entries {
            let ts = (at(NOW) + chrono::Duration::seconds(*offset)).to_rfc3339();
            let extra = if extra.is_empty() {
                String::new()
            } else {
                format!(",{extra}")
            };
            writeln!(
                f,
                r#"{{"type":"{kind}","timestamp":"{ts}","cwd":"C:\\dev\\myapp"{extra}}}"#
            )
            .unwrap();
        }
        path
    }

    #[test]
    fn detects_a_session_waiting_for_tool_approval() {
        let dir = tempfile::tempdir().unwrap();
        // Assistant asked to run a tool 30s ago and nothing has come back.
        let path = write_transcript(
            dir.path(),
            "s1.jsonl",
            &[
                ("user", -120, r#""message":{"role":"user","content":[]}"#),
                (
                    "assistant",
                    -30,
                    r#""message":{"role":"assistant","content":[{"type":"tool_use","name":"Bash"}]}"#,
                ),
            ],
        );
        let summary = summarise_transcript(&path, at(NOW)).unwrap();
        assert_eq!(summary.activity, Activity::AwaitingInput);
        assert_eq!(summary.project, "myapp");
    }

    #[test]
    fn an_approved_tool_call_is_not_treated_as_waiting() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_transcript(
            dir.path(),
            "s2.jsonl",
            &[
                (
                    "assistant",
                    -60,
                    r#""message":{"role":"assistant","content":[{"type":"tool_use","name":"Bash"}]}"#,
                ),
                (
                    "user",
                    -55,
                    r#""message":{"role":"user","content":[{"type":"tool_result"}]}"#,
                ),
            ],
        );
        let summary = summarise_transcript(&path, at(NOW)).unwrap();
        assert_eq!(summary.activity, Activity::Done);
    }

    #[test]
    fn a_very_recent_turn_counts_as_generating() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_transcript(
            dir.path(),
            "s3.jsonl",
            &[(
                "assistant",
                -5,
                r#""message":{"role":"assistant","content":[]}"#,
            )],
        );
        assert_eq!(
            summarise_transcript(&path, at(NOW)).unwrap().activity,
            Activity::Generating
        );
    }

    #[test]
    fn activity_decays_through_done_to_idle() {
        let dir = tempfile::tempdir().unwrap();
        let cases = [(-60, Activity::Done), (-3600, Activity::Idle)];
        for (offset, expected) in cases {
            let path = write_transcript(
                dir.path(),
                &format!("s{offset}.jsonl"),
                &[(
                    "assistant",
                    offset,
                    r#""message":{"role":"assistant","content":[]}"#,
                )],
            );
            assert_eq!(
                summarise_transcript(&path, at(NOW)).unwrap().activity,
                expected,
                "offset {offset}s"
            );
        }
    }

    #[test]
    fn a_stale_pending_tool_call_stops_nagging() {
        // A tool_use from days ago is an abandoned session, not a live prompt.
        let dir = tempfile::tempdir().unwrap();
        let path = write_transcript(
            dir.path(),
            "old.jsonl",
            &[(
                "assistant",
                -86_400,
                r#""message":{"role":"assistant","content":[{"type":"tool_use"}]}"#,
            )],
        );
        assert_eq!(
            summarise_transcript(&path, at(NOW)).unwrap().activity,
            Activity::Idle
        );
    }

    #[test]
    fn sums_every_token_bucket_in_the_window() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_transcript(
            dir.path(),
            "tokens.jsonl",
            &[
                (
                    "assistant",
                    -300,
                    r#""message":{"model":"claude-opus-5","usage":{"input_tokens":100,"output_tokens":50,"cache_read_input_tokens":1000,"cache_creation_input_tokens":25}}"#,
                ),
                (
                    "assistant",
                    -100,
                    r#""message":{"model":"claude-opus-5","usage":{"input_tokens":10,"output_tokens":5}}"#,
                ),
                // Outside the 5h window: must not be counted.
                (
                    "assistant",
                    -60 * 60 * 9,
                    r#""message":{"usage":{"input_tokens":999999}}"#,
                ),
            ],
        );
        let s = summarise_transcript(&path, at(NOW)).unwrap();
        assert_eq!(s.tokens, 100 + 50 + 1000 + 25 + 10 + 5);
        assert_eq!(s.model.as_deref(), Some("claude-opus-5"));
    }

    #[test]
    fn malformed_transcript_lines_are_skipped_not_fatal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mixed.jsonl");
        let ts = at(NOW).to_rfc3339();
        std::fs::write(
            &path,
            format!(
                "garbage not json\n\
                 {{\"type\":\"assistant\",\"timestamp\":\"{ts}\",\"cwd\":\"C:\\\\dev\\\\app\"}}\n\
                 {{unclosed\n"
            ),
        )
        .unwrap();
        let s = summarise_transcript(&path, at(NOW)).expect("valid lines still parse");
        assert_eq!(s.project, "app");
    }

    #[test]
    fn an_empty_transcript_yields_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.jsonl");
        std::fs::write(&path, "").unwrap();
        assert!(summarise_transcript(&path, at(NOW)).is_none());
    }

    #[test]
    fn project_name_falls_back_to_the_encoded_directory() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("C--Users-dev-cool-app");
        std::fs::create_dir_all(&project).unwrap();
        let path = project.join("s.jsonl");
        std::fs::write(
            &path,
            format!(
                r#"{{"type":"assistant","timestamp":"{}"}}"#,
                at(NOW).to_rfc3339()
            ),
        )
        .unwrap();
        assert_eq!(summarise_transcript(&path, at(NOW)).unwrap().project, "app");
    }

    #[test]
    fn scan_orders_blocked_sessions_ahead_of_busy_ones() {
        let dir = tempfile::tempdir().unwrap();
        let projects = dir.path().join("projects");
        std::fs::create_dir_all(&projects).unwrap();

        write_transcript(
            &projects,
            "busy.jsonl",
            &[(
                "assistant",
                -3,
                r#""message":{"role":"assistant","content":[]}"#,
            )],
        );
        write_transcript(
            &projects,
            "blocked.jsonl",
            &[(
                "assistant",
                -40,
                r#""message":{"role":"assistant","content":[{"type":"tool_use"}]}"#,
            )],
        );

        let adapter = ClaudeAdapter::with_paths(
            reqwest::Client::new(),
            ClaudePaths {
                root: dir.path().to_path_buf(),
            },
        );
        let sessions = adapter.scan_sessions(at(NOW));
        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions[0].id, "blocked");
        assert_eq!(sessions[0].activity, Activity::AwaitingInput);
        assert_eq!(sessions[1].activity, Activity::Generating);
    }

    #[tokio::test]
    async fn a_missing_install_reports_unavailable_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let mut adapter = ClaudeAdapter::with_paths(
            reqwest::Client::new(),
            ClaudePaths {
                root: dir.path().join("does-not-exist"),
            },
        );
        let snap = adapter.collect(at(NOW)).await;
        assert_eq!(snap.health, Health::Unavailable);
        assert!(snap.windows.is_empty());
    }

    #[tokio::test]
    async fn an_installed_but_signed_out_claude_still_reports_activity() {
        let dir = tempfile::tempdir().unwrap();
        let projects = dir.path().join("projects");
        std::fs::create_dir_all(&projects).unwrap();
        write_transcript(
            &projects,
            "s.jsonl",
            &[(
                "assistant",
                -10,
                r#""message":{"role":"assistant","usage":{"output_tokens":42},"content":[]}"#,
            )],
        );

        let mut adapter = ClaudeAdapter::with_paths(
            reqwest::Client::new(),
            ClaudePaths {
                root: dir.path().to_path_buf(),
            },
        );
        let snap = adapter.collect(at(NOW)).await;
        assert_eq!(snap.health, Health::NeedsAuth);
        assert_eq!(snap.activity, Activity::Generating);
        assert_eq!(snap.source.as_deref(), Some("transcripts"));
        // The token figure is derived locally, so it must be marked estimated.
        assert_eq!(snap.windows.len(), 1);
        assert!(snap.windows[0].estimated);
        assert_eq!(snap.windows[0].used, Some(42.0));
        assert_eq!(
            snap.windows[0].used_pct, None,
            "no denominator, no percentage"
        );
    }

    #[test]
    fn tokens_by_model_aggregates_across_sessions() {
        let sessions = vec![
            Session {
                model: Some("opus".into()),
                tokens: Some(10),
                ..Session::new("a", "a")
            },
            Session {
                model: Some("opus".into()),
                tokens: Some(5),
                ..Session::new("b", "b")
            },
            Session {
                model: Some("haiku".into()),
                tokens: Some(1),
                ..Session::new("c", "c")
            },
            Session {
                model: None,
                tokens: Some(99),
                ..Session::new("d", "d")
            },
        ];
        let totals = tokens_by_model(&sessions);
        assert_eq!(totals.get("opus"), Some(&15));
        assert_eq!(totals.get("haiku"), Some(&1));
        assert_eq!(totals.len(), 2);
    }

    #[test]
    fn human_age_is_compact() {
        assert_eq!(human_age(chrono::Duration::seconds(5)), "5s");
        assert_eq!(human_age(chrono::Duration::minutes(3)), "3m");
        assert_eq!(human_age(chrono::Duration::hours(2)), "2h");
    }
}
