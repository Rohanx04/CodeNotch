//! Claude Code permission requests, answered from the notch.
//!
//! The relay (`codenotch-hook`) hands a `PermissionRequest` over the pipe and
//! waits. This module puts it on the notch, waits for Allow or Deny, and gives
//! the relay a bare `allow` or `deny` -- turning that into the JSON Claude Code
//! expects is the relay's job, so the wire format lives in exactly one place.
//!
//! Claude Code is never held hostage by a card nobody can see:
//!
//! * **One card at a time.** A second request while one is on screen goes
//!   straight back to the terminal rather than replacing the first, which
//!   would leave request A waiting on a decision nobody can give.
//! * **Two waits.** The webview must confirm the card is actually up within
//!   [`ACK_TIMEOUT`]; only then does the long wait for a human begin. A hidden,
//!   paused or crashed webview costs Claude Code under a second, not minutes.
//! * **A deadline.** After [`DECISION_TIMEOUT`] the request is released and the
//!   terminal asks as usual. It is shorter than the relay's own budget, so we
//!   always answer first.
//! * **An explicit click.** Nothing is ever approved without one.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::mpsc;

use codenotch_core::live::{approval_target, HookEvent};
use codenotch_core::model::ProviderId;

use crate::commands::AppState;
use crate::poll::{cue, Cue};

/// Carries the request on screen (or `null` once it is gone) to the webview.
pub const APPROVAL_EVENT: &str = "codenotch://approval";

/// How long the webview gets to say "the card is up".
const ACK_TIMEOUT: Duration = Duration::from_millis(800);
/// How long a human gets. Slightly under the relay's 110 s.
const DECISION_TIMEOUT: Duration = Duration::from_secs(108);

static COUNTER: AtomicU64 = AtomicU64::new(1);

/// The permission request on the notch.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalRequest {
    pub id: String,
    pub session_id: String,
    pub project: String,
    pub tool: String,
    /// Exactly what Allow authorises, e.g. `Bash · cargo publish`.
    pub target: String,
    /// When the terminal takes over, in Unix milliseconds.
    pub expires_at: u64,
}

/// What the webview can say about a request.
#[derive(Debug)]
pub enum Reply {
    /// The card is on screen and a human can act on it.
    Ack,
    /// A human clicked `allow` or `deny`.
    Decision(&'static str),
}

/// The request on screen, if any, and the channel its answer goes down.
#[derive(Default)]
pub struct Approvals {
    current: Mutex<Option<(ApprovalRequest, mpsc::Sender<Reply>)>>,
}

impl Approvals {
    pub fn current(&self) -> Option<ApprovalRequest> {
        self.current
            .lock()
            .expect("approvals lock")
            .as_ref()
            .map(|(r, _)| r.clone())
    }

    /// Claim the notch for a request. False when one is already on it.
    fn begin(&self, request: ApprovalRequest, reply: mpsc::Sender<Reply>) -> bool {
        let mut current = self.current.lock().expect("approvals lock");
        if current.is_some() {
            return false;
        }
        *current = Some((request, reply));
        true
    }

    fn finish(&self, id: &str) {
        let mut current = self.current.lock().expect("approvals lock");
        if current.as_ref().is_some_and(|(r, _)| r.id == id) {
            *current = None;
        }
    }

    /// Hand the request on screen back to the terminal (Pause): dropping its
    /// channel ends the wait with no decision.
    pub fn release(&self) {
        *self.current.lock().expect("approvals lock") = None;
    }

    /// Route the webview's word to the waiting request. Stale ids (a card
    /// clicked just as it timed out) are dropped.
    pub fn reply(&self, id: &str, reply: Reply) {
        let current = self.current.lock().expect("approvals lock");
        match current.as_ref() {
            Some((request, tx)) if request.id == id => {
                let _ = tx.try_send(reply);
            }
            _ => tracing::info!(id, "reply for a request that is no longer pending"),
        }
    }
}

fn unix_ms_in(delay: Duration) -> u64 {
    (SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        + delay)
        .as_millis() as u64
}

/// Run one permission request end to end. Returns the word to write back to
/// the relay, or `None` to let the terminal ask.
pub async fn handle(app: &AppHandle, payload: Value) -> Option<String> {
    let state = app.try_state::<AppState>()?;
    let event = HookEvent::from_json(&payload)?;

    if state.hud.paused() || state.hud.config().hidden {
        tracing::info!("permission request while paused or hidden — the terminal takes over");
        return None;
    }

    let tool = event.tool_name.clone().unwrap_or_else(|| "Tool".into());
    let input = event.tool_input.clone().unwrap_or(Value::Null);
    let request = ApprovalRequest {
        id: format!(
            "{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ),
        session_id: event.session_id.clone(),
        project: codenotch_core::adapters::project_label(&event.cwd),
        target: approval_target(&tool, &input),
        tool,
        expires_at: unix_ms_in(DECISION_TIMEOUT),
    };

    let (tx, mut rx) = mpsc::channel::<Reply>(4);
    if !state.approvals.begin(request.clone(), tx) {
        tracing::info!(
            "a permission request is already on the notch — the terminal takes this one"
        );
        return None;
    }
    let id = request.id.clone();
    tracing::info!(id, tool = %request.tool, "permission request");

    crate::hooks::apply_event(app, &event);
    let _ = state.hud.set_alert(Some(ProviderId::ClaudeCode));
    let _ = app.emit(APPROVAL_EVENT, Some(&request));
    cue(app, Cue::Approval);

    let decision = wait(&id, &mut rx).await;

    state.approvals.finish(&id);
    let _ = app.emit(APPROVAL_EVENT, None::<ApprovalRequest>);
    let _ = state.hud.set_alert(None);
    crate::hooks::resolve_approval(app, &event.session_id);

    decision.map(str::to_string)
}

/// A short wait for "the card is up", then the long one for a human.
async fn wait(id: &str, rx: &mut mpsc::Receiver<Reply>) -> Option<&'static str> {
    match tokio::time::timeout(ACK_TIMEOUT, rx.recv()).await {
        Ok(Some(Reply::Ack)) => {}
        // A click that beats the ack is still a click.
        Ok(Some(Reply::Decision(d))) => {
            tracing::info!(id, decision = d, "answered");
            return Some(d);
        }
        Ok(None) => return None,
        Err(_) => {
            tracing::info!(
                id,
                "the notch never showed the card — the terminal takes over"
            );
            return None;
        }
    }

    // The card acks again whenever it is shown again (the pointer went to
    // another ring and came back), so later acks are expected and ignored:
    // only a decision, the deadline or a released request ends this wait.
    let deadline = tokio::time::Instant::now() + DECISION_TIMEOUT;
    loop {
        match tokio::time::timeout_at(deadline, rx.recv()).await {
            Ok(Some(Reply::Ack)) => continue,
            Ok(Some(Reply::Decision(d))) => {
                tracing::info!(id, decision = d, "answered");
                return Some(d);
            }
            Ok(None) => {
                tracing::info!(id, "released — the terminal takes over");
                return None;
            }
            Err(_) => {
                tracing::info!(id, "no answer in time — the terminal takes over");
                return None;
            }
        }
    }
}
