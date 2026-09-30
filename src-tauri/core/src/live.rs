//! Live Claude Code sessions, from hook events.
//!
//! Transcripts tell CodeNotch what a session *was* doing, a poll late and by
//! inference -- "an assistant tool call with no result yet" might be a
//! permission prompt or might be a slow `cargo build`. Hook events say what it
//! is doing *now*, exactly: which tool it is running on which file, that it is
//! blocked on a permission prompt, that the turn just ended. When the user has
//! installed the hooks, [`LiveState`] keeps that picture and
//! [`LiveState::overlay`] lays it over the transcript-based snapshot, so the
//! ring and the session list move the moment Claude Code does.
//!
//! Only fresh information wins: a session that has gone quiet for
//! [`LIVE_TTL`] is left to the transcripts again, so a crashed session can
//! never pin its ring to "working" forever.

use std::collections::BTreeMap;

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::adapters::project_label;
use crate::model::{Activity, ProviderId, Session, Telemetry};

/// Steps remembered per session, for the rolling ticker.
pub const MAX_STEPS: usize = 12;
/// A session that has sent nothing for this long is handed back to the
/// transcripts.
pub const LIVE_TTL_MINS: i64 = 20;
/// "Finished" is news for this long, then settles to idle -- the same window
/// the transcript reader uses.
pub const DONE_TTL_MINS: i64 = 10;

/// One hook payload, as Claude Code sends it on stdin (and the relay forwards).
///
/// Every field is optional because every event carries a different subset;
/// unknown fields are ignored.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct HookEvent {
    #[serde(rename = "hook_event_name", default)]
    pub name: String,
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub cwd: String,
    #[serde(default)]
    pub tool_name: Option<String>,
    #[serde(default)]
    pub tool_input: Option<Value>,
    /// `UserPromptSubmit` carries the prompt here.
    #[serde(default)]
    pub prompt: Option<String>,
    /// `Notification`'s text.
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub notification_type: Option<String>,
}

impl HookEvent {
    pub fn from_json(value: &Value) -> Option<Self> {
        let event: HookEvent = serde_json::from_value(value.clone()).ok()?;
        (!event.name.is_empty()).then_some(event)
    }

    fn input(&self) -> &Value {
        static EMPTY: Value = Value::Null;
        self.tool_input.as_ref().unwrap_or(&EMPTY)
    }
}

/// What the webview shows for one live session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveSession {
    pub id: String,
    pub project: String,
    pub cwd: Option<String>,
    pub activity: Activity,
    /// The last [`MAX_STEPS`] steps, oldest first.
    pub steps: Vec<String>,
    /// Steps seen in total. The ticker compares this with what it has already
    /// shown, so a burst of steps scrolls past instead of being skipped.
    pub step_count: u64,
    pub last_event: DateTime<Utc>,
    /// The turn ended in an API error rather than a normal stop.
    pub failed: bool,
    #[serde(skip)]
    done_at: Option<DateTime<Utc>>,
}

impl LiveSession {
    fn new(id: &str, now: DateTime<Utc>) -> Self {
        Self {
            id: id.to_string(),
            project: "Session".into(),
            cwd: None,
            activity: Activity::Idle,
            steps: Vec::new(),
            step_count: 0,
            last_event: now,
            failed: false,
            done_at: None,
        }
    }

    fn push_step(&mut self, step: String) {
        self.steps.push(step);
        if self.steps.len() > MAX_STEPS {
            let excess = self.steps.len() - MAX_STEPS;
            self.steps.drain(..excess);
        }
        self.step_count += 1;
    }

    fn set_activity(&mut self, activity: Activity, now: DateTime<Utc>) {
        self.activity = activity;
        self.done_at = (activity == Activity::Done).then_some(now);
    }

    /// The line shown under the session's title.
    fn detail(&self) -> Option<String> {
        match self.activity {
            Activity::AwaitingInput => Some("Needs your approval".into()),
            Activity::Done if self.failed => Some("Stopped with an error".into()),
            Activity::Generating | Activity::Done => self.steps.last().cloned(),
            Activity::Idle => None,
        }
    }
}

/// Every session a hook has told us about.
#[derive(Debug, Clone, Default)]
pub struct LiveState {
    sessions: BTreeMap<String, LiveSession>,
}

impl LiveState {
    /// Fold one hook event in. Returns false for events that change nothing.
    pub fn apply(&mut self, event: &HookEvent, now: DateTime<Utc>) -> bool {
        let id = if event.session_id.is_empty() {
            "session"
        } else {
            event.session_id.as_str()
        };
        let session = self
            .sessions
            .entry(id.to_string())
            .or_insert_with(|| LiveSession::new(id, now));
        session.last_event = now;
        if !event.cwd.is_empty() {
            session.project = project_label(&event.cwd);
            session.cwd = Some(event.cwd.clone());
        }

        match event.name.as_str() {
            "SessionStart" => {
                session.failed = false;
                session.set_activity(Activity::Idle, now);
            }
            "UserPromptSubmit" => {
                session.failed = false;
                session.set_activity(Activity::Generating, now);
                if let Some(prompt) = event.prompt.as_deref().filter(|p| !p.trim().is_empty()) {
                    session.push_step(clip(prompt.trim(), 60));
                }
            }
            "PreToolUse" => {
                session.set_activity(Activity::Generating, now);
                let tool = event.tool_name.as_deref().unwrap_or("Tool");
                session.push_step(step_label(tool, event.input()));
            }
            "PostToolUse" => session.set_activity(Activity::Generating, now),
            "PostToolUseFailure" => {
                session.set_activity(Activity::Generating, now);
                session.push_step("⚠ tool failed".into());
            }
            "PermissionRequest" => session.set_activity(Activity::AwaitingInput, now),
            "Notification" => match event.notification_type.as_deref() {
                Some("permission_prompt") | Some("agent_needs_input") => {
                    session.set_activity(Activity::AwaitingInput, now);
                }
                // Claude has been waiting on the user for a while: the turn is over.
                Some("idle_prompt") => session.set_activity(Activity::Done, now),
                _ => {
                    let message = event.message.as_deref().unwrap_or_default();
                    if message.to_lowercase().contains("permission") {
                        session.set_activity(Activity::AwaitingInput, now);
                    }
                }
            },
            "Stop" => session.set_activity(Activity::Done, now),
            "StopFailure" => {
                session.failed = true;
                session.set_activity(Activity::Done, now);
                session.push_step("⚠ stopped with an error".into());
            }
            "SubagentStart" => session.push_step("+ subagent".into()),
            "SubagentStop" => session.push_step("• subagent done".into()),
            "SessionEnd" => {
                session.steps.clear();
                session.failed = false;
                session.set_activity(Activity::Idle, now);
            }
            _ => return false,
        }
        true
    }

    /// A permission request was answered from the notch (or handed back to the
    /// terminal): the session is working again rather than blocked.
    pub fn resolve_approval(&mut self, session_id: &str, now: DateTime<Utc>) {
        if let Some(s) = self.sessions.get_mut(session_id) {
            if s.activity == Activity::AwaitingInput {
                s.set_activity(Activity::Generating, now);
                s.last_event = now;
            }
        }
    }

    /// Settle "finished" to idle and forget sessions that went quiet.
    pub fn expire(&mut self, now: DateTime<Utc>) {
        self.sessions.retain(|_, s| {
            now.signed_duration_since(s.last_event) < Duration::minutes(LIVE_TTL_MINS)
        });
        for s in self.sessions.values_mut() {
            if s.done_at
                .is_some_and(|t| now.signed_duration_since(t) >= Duration::minutes(DONE_TTL_MINS))
            {
                s.set_activity(Activity::Idle, now);
                s.failed = false;
            }
        }
    }

    pub fn sessions(&self) -> impl Iterator<Item = &LiveSession> {
        self.sessions.values()
    }

    /// Lay the live picture over the transcript-based telemetry.
    ///
    /// Sessions the transcripts already know are updated in place; sessions
    /// too new to have a transcript yet are added. The provider's activity and
    /// the rolled-up activity are recomputed, so the ring follows at once.
    pub fn overlay(&mut self, telemetry: &mut Telemetry, now: DateTime<Utc>) {
        self.expire(now);
        telemetry.live = self
            .sessions
            .values()
            .filter(|s| s.activity != Activity::Idle || !s.steps.is_empty())
            .cloned()
            .collect();

        let Some(claude) = telemetry
            .providers
            .iter_mut()
            .find(|p| p.id == ProviderId::ClaudeCode)
        else {
            return;
        };
        if self.sessions.is_empty() {
            return;
        }

        for live in self.sessions.values() {
            match claude.sessions.iter_mut().find(|s| s.id == live.id) {
                Some(session) => {
                    session.activity = live.activity;
                    session.last_activity = Some(live.last_event);
                    if let Some(detail) = live.detail() {
                        session.detail = Some(detail);
                    }
                }
                None if live.activity != Activity::Idle => {
                    let mut session = Session::new(live.id.clone(), live.project.clone());
                    session.cwd = live.cwd.clone();
                    session.activity = live.activity;
                    session.last_activity = Some(live.last_event);
                    session.detail = live.detail();
                    claude.sessions.push(session);
                }
                None => {}
            }
        }

        // Most urgent first, as the transcript reader orders them.
        claude.sessions.sort_by(|a, b| {
            b.activity
                .rank()
                .cmp(&a.activity.rank())
                .then(b.last_activity.cmp(&a.last_activity))
        });
        claude.activity = claude
            .sessions
            .iter()
            .fold(Activity::Idle, |acc, s| acc.merge(s.activity));
        telemetry.activity = telemetry
            .providers
            .iter()
            .fold(Activity::Idle, |acc, p| acc.merge(p.activity));
    }
}

/// Tool name → verb, for the step ticker.
fn verb(tool: &str) -> &str {
    match tool {
        "Bash" | "PowerShell" => "Run",
        "Read" => "Read",
        "Write" => "Write",
        "Edit" | "MultiEdit" => "Edit",
        "Glob" => "Find",
        "Grep" => "Search",
        "WebSearch" => "Web search",
        "WebFetch" => "Fetch",
        "TodoWrite" => "Todos",
        "Task" | "Agent" => "Agent",
        "LS" => "List",
        "NotebookEdit" => "Notebook",
        other => other,
    }
}

/// "Edit · main.rs", "Run · cargo test": what one tool call is doing, short.
pub fn step_label(tool: &str, input: &Value) -> String {
    let label = verb(tool);
    let text = |k: &str| input.get(k).and_then(Value::as_str).map(str::trim);
    if let Some(cmd) = text("command").filter(|s| !s.is_empty()) {
        return format!("{label} · {}", clip(cmd, 40));
    }
    for key in ["file_path", "path", "notebook_path"] {
        if let Some(path) = text(key).filter(|s| !s.is_empty()) {
            return format!("{label} · {}", project_label(path));
        }
    }
    for key in ["pattern", "query", "url", "description"] {
        if let Some(value) = text(key).filter(|s| !s.is_empty()) {
            return format!("{label} · {}", clip(value, 40));
        }
    }
    label.to_string()
}

/// What the Allow button actually authorises. Approving "Write" tells you
/// nothing; approving `Write · C:\…\.env` tells you everything, and that is the
/// whole point of approving from the notch rather than blind.
///
/// Ordered by how specific the field is, so an unfamiliar tool still shows
/// whatever identifying string it carries instead of just its name.
pub fn approval_target(tool: &str, input: &Value) -> String {
    for field in [
        "command",
        "file_path",
        "path",
        "notebook_path",
        "url",
        "query",
        "pattern",
        "prompt",
    ] {
        if let Some(value) = input.get(field).and_then(Value::as_str) {
            let value = value.trim();
            if !value.is_empty() {
                return format!("{tool} · {}", clip(value, 400));
            }
        }
    }
    tool.to_string()
}

/// Cut to `max` characters, on a character boundary, with an ellipsis.
fn clip(s: &str, max: usize) -> String {
    let one_line = s.lines().next().unwrap_or_default();
    let mut out: String = one_line.chars().take(max).collect();
    if one_line.chars().count() > max || s.lines().nth(1).is_some() {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ProviderSnapshot;
    use serde_json::json;

    fn t0() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-09-30T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn ev(value: Value) -> HookEvent {
        HookEvent::from_json(&value).expect("valid event")
    }

    fn telemetry_with(sessions: Vec<Session>) -> Telemetry {
        Telemetry::from_snapshots(vec![
            ProviderSnapshot::new(ProviderId::ClaudeCode).with_sessions(sessions)
        ])
    }

    #[test]
    fn events_without_a_name_are_ignored() {
        assert!(HookEvent::from_json(&json!({"session_id": "a"})).is_none());
        assert!(HookEvent::from_json(&json!("nonsense")).is_none());
    }

    #[test]
    fn tool_steps_read_like_a_log() {
        assert_eq!(
            step_label("Edit", &json!({"file_path": r"C:\src\app\main.rs"})),
            "Edit · main.rs"
        );
        assert_eq!(
            step_label("Bash", &json!({"command": "cargo test --all"})),
            "Run · cargo test --all"
        );
        assert_eq!(
            step_label("Grep", &json!({"pattern": "fn main"})),
            "Search · fn main"
        );
        assert_eq!(step_label("Mystery", &json!({})), "Mystery");
        let long = step_label("Bash", &json!({"command": "x".repeat(100)}));
        assert!(long.chars().count() <= "Run · ".len() + 41);
        assert!(long.ends_with('…'));
    }

    #[test]
    fn the_approval_target_names_exactly_what_is_allowed() {
        assert_eq!(
            approval_target(
                "Bash",
                &json!({"command": "rm -rf build", "description": "clean"})
            ),
            "Bash · rm -rf build"
        );
        assert_eq!(
            approval_target("Write", &json!({"file_path": r"C:\app\.env"})),
            r"Write · C:\app\.env"
        );
        assert_eq!(approval_target("Odd", &json!({"x": 1})), "Odd");
    }

    #[test]
    fn a_turn_goes_working_then_waiting_then_finished() {
        let mut live = LiveState::default();
        let now = t0();
        live.apply(
            &ev(json!({"hook_event_name": "UserPromptSubmit", "session_id": "s", "cwd": "/w/app", "prompt": "fix the build"})),
            now,
        );
        live.apply(
            &ev(json!({"hook_event_name": "PreToolUse", "session_id": "s", "tool_name": "Read", "tool_input": {"file_path": "/w/app/Cargo.toml"}})),
            now,
        );
        let s = live.sessions().next().unwrap();
        assert_eq!(s.activity, Activity::Generating);
        assert_eq!(s.project, "app");
        assert_eq!(s.steps, vec!["fix the build", "Read · Cargo.toml"]);
        assert_eq!(s.step_count, 2);

        live.apply(
            &ev(json!({"hook_event_name": "PermissionRequest", "session_id": "s", "tool_name": "Bash"})),
            now,
        );
        assert_eq!(
            live.sessions().next().unwrap().activity,
            Activity::AwaitingInput
        );
        live.resolve_approval("s", now);
        assert_eq!(
            live.sessions().next().unwrap().activity,
            Activity::Generating
        );

        live.apply(
            &ev(json!({"hook_event_name": "Stop", "session_id": "s"})),
            now,
        );
        assert_eq!(live.sessions().next().unwrap().activity, Activity::Done);
    }

    #[test]
    fn steps_are_capped_but_still_counted() {
        let mut live = LiveState::default();
        for i in 0..(MAX_STEPS + 5) {
            live.apply(
                &ev(json!({"hook_event_name": "PreToolUse", "session_id": "s", "tool_name": "Grep", "tool_input": {"pattern": format!("p{i}")}})),
                t0(),
            );
        }
        let s = live.sessions().next().unwrap();
        assert_eq!(s.steps.len(), MAX_STEPS);
        assert_eq!(s.step_count, (MAX_STEPS + 5) as u64);
        assert_eq!(
            s.steps.last().unwrap(),
            &format!("Search · p{}", MAX_STEPS + 4)
        );
    }

    #[test]
    fn finished_settles_and_quiet_sessions_are_forgotten() {
        let mut live = LiveState::default();
        live.apply(
            &ev(json!({"hook_event_name": "Stop", "session_id": "s"})),
            t0(),
        );
        live.expire(t0() + Duration::minutes(DONE_TTL_MINS));
        assert_eq!(live.sessions().next().unwrap().activity, Activity::Idle);
        live.expire(t0() + Duration::minutes(LIVE_TTL_MINS));
        assert_eq!(live.sessions().count(), 0);
    }

    #[test]
    fn the_overlay_moves_a_known_session_and_its_ring() {
        let mut transcript = Session::new("s", "app");
        transcript.activity = Activity::Idle;
        let mut t = telemetry_with(vec![transcript]);

        let mut live = LiveState::default();
        live.apply(
            &ev(json!({"hook_event_name": "PreToolUse", "session_id": "s", "tool_name": "Edit", "tool_input": {"file_path": "/w/app/lib.rs"}})),
            t0(),
        );
        live.overlay(&mut t, t0());

        let claude = t.get(ProviderId::ClaudeCode).unwrap();
        assert_eq!(claude.sessions.len(), 1);
        assert_eq!(claude.sessions[0].activity, Activity::Generating);
        assert_eq!(claude.sessions[0].detail.as_deref(), Some("Edit · lib.rs"));
        assert_eq!(claude.activity, Activity::Generating);
        assert_eq!(t.activity, Activity::Generating);
        assert_eq!(t.live.len(), 1);
    }

    #[test]
    fn a_session_too_new_for_a_transcript_is_added() {
        let mut t = telemetry_with(vec![Session::new("old", "other")]);
        let mut live = LiveState::default();
        live.apply(
            &ev(json!({"hook_event_name": "PermissionRequest", "session_id": "new", "cwd": "/w/fresh"})),
            t0(),
        );
        live.overlay(&mut t, t0());
        let claude = t.get(ProviderId::ClaudeCode).unwrap();
        assert_eq!(claude.sessions.len(), 2);
        // Blocked sessions sort first.
        assert_eq!(claude.sessions[0].id, "new");
        assert_eq!(claude.sessions[0].title, "fresh");
        assert_eq!(
            claude.sessions[0].detail.as_deref(),
            Some("Needs your approval")
        );
        assert_eq!(t.activity, Activity::AwaitingInput);
    }

    #[test]
    fn an_ended_session_forces_its_transcript_idle() {
        let mut transcript = Session::new("s", "app");
        transcript.activity = Activity::Generating;
        let mut t = telemetry_with(vec![transcript]);
        let mut live = LiveState::default();
        live.apply(
            &ev(json!({"hook_event_name": "SessionEnd", "session_id": "s"})),
            t0(),
        );
        live.overlay(&mut t, t0());
        assert_eq!(
            t.get(ProviderId::ClaudeCode).unwrap().activity,
            Activity::Idle
        );
        assert!(t.live.is_empty(), "nothing to tick through");
    }

    #[test]
    fn with_no_hooks_the_telemetry_is_untouched() {
        let mut transcript = Session::new("s", "app");
        transcript.activity = Activity::Done;
        let mut t = telemetry_with(vec![transcript]);
        let before = t.clone();
        LiveState::default().overlay(&mut t, t0());
        assert_eq!(t, before);
    }

    #[test]
    fn a_failed_stop_is_marked_as_such() {
        let mut live = LiveState::default();
        live.apply(
            &ev(json!({"hook_event_name": "StopFailure", "session_id": "s"})),
            t0(),
        );
        let s = live.sessions().next().unwrap();
        assert!(s.failed);
        assert_eq!(s.detail().as_deref(), Some("Stopped with an error"));
    }
}
