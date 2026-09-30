//! The background polling loop, and publishing what it finds.
//!
//! Runs on Tauri's async runtime, refreshes whichever providers are due, lays
//! the live hook picture over the result, pushes it to the webview, and
//! decides when something deserves the user's attention (a peek, a sound, or
//! both).

use std::time::Duration;

use chrono::Utc;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use codenotch_core::model::{Activity, ProviderId, Telemetry};
use codenotch_core::{Alert, Health};

use crate::commands::{AppState, TELEMETRY_EVENT};

/// Never sleep longer than this, so a config change is picked up promptly.
const MAX_SLEEP: Duration = Duration::from_secs(15);

/// Event carrying a sound cue to the webview.
pub const CUE_EVENT: &str = "codenotch://cue";

/// The moments worth a sound. The webview owns the sounds (and whether they
/// are on); the backend only says what happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Cue {
    /// An agent is blocked on the user.
    Attention,
    /// A permission request arrived on the notch.
    Approval,
    /// An agent finished its turn.
    Finish,
    /// An agent's turn ended in an error.
    Error,
    /// A usage window crossed 80% or 100%.
    Threshold,
}

pub fn cue(app: &AppHandle, cue: Cue) {
    let paused = app.try_state::<AppState>().is_some_and(|s| s.hud.paused());
    if !paused {
        let _ = app.emit(CUE_EVENT, cue);
    }
}

/// Start the loop. Returns immediately; the work happens on a spawned task.
pub fn spawn(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        // A first pass straight away so the HUD isn't empty on launch.
        run_once(&app).await;

        loop {
            let interval = match app.try_state::<AppState>() {
                Some(state) => {
                    let collector = state.collector.lock().await;
                    Duration::from_secs(collector.tick_interval_secs())
                }
                // The app is shutting down.
                None => return,
            };

            tokio::time::sleep(interval.min(MAX_SLEEP)).await;
            run_once(&app).await;
        }
    });
}

/// One pass: poll what's due, publish it, react to it.
async fn run_once(app: &AppHandle) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };

    state.hud.keep_on_top();

    // Paused means paused: nothing is read and nothing goes out on the
    // network. The last numbers stay up, and age into "stale" honestly.
    let alerts = if state.hud.paused() {
        let telemetry = state.collector.lock().await.telemetry(Utc::now());
        store_raw(&state, telemetry);
        Vec::new()
    } else {
        let (telemetry, alerts) = {
            let mut collector = state.collector.lock().await;
            collector.poll(Utc::now()).await
        };
        store_raw(&state, telemetry);
        alerts
    };

    publish(app, alerts);
}

fn store_raw(state: &AppState, telemetry: Telemetry) {
    if let Ok(mut raw) = state.raw.lock() {
        *raw = telemetry;
    }
}

/// Force a full refresh now (used by the tray's "Refresh now").
pub async fn refresh(app: &AppHandle) {
    if let Some(state) = app.try_state::<AppState>() {
        state.collector.lock().await.invalidate();
    }
    run_once(app).await;
}

/// Lay the live hook picture over the latest collection and push it out.
///
/// Called after every poll and on every hook event, so the ring moves the
/// moment Claude Code does rather than at the next poll.
pub fn publish(app: &AppHandle, alerts: Vec<Alert>) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };

    let mut telemetry = state
        .raw
        .lock()
        .map(|t| t.clone())
        .unwrap_or_else(|_| Telemetry::empty());
    if let Ok(mut live) = state.live.lock() {
        live.overlay(&mut telemetry, Utc::now());
    }

    let previous = match state.latest.lock() {
        Ok(mut latest) => std::mem::replace(&mut *latest, telemetry.clone()),
        Err(_) => Telemetry::empty(),
    };

    if let Some((provider, activity)) = telemetry.attention_since(&previous) {
        let peek_secs = state.hud.config().peek_secs;
        let _ = state
            .hud
            .peek(Duration::from_secs(peek_secs), Some(provider));
        // A permission request on the notch has its own cue.
        if state.approvals.current().is_none() {
            let failed = telemetry
                .live
                .iter()
                .any(|s| s.failed && s.activity == Activity::Done);
            cue(
                app,
                match activity {
                    Activity::AwaitingInput => Cue::Attention,
                    _ if failed && provider == ProviderId::ClaudeCode => Cue::Error,
                    _ => Cue::Finish,
                },
            );
        }
    }

    // The strip is one ring per provider that has something to say, so its
    // length follows the collection rather than a fixed guess.
    let rings = telemetry
        .providers
        .iter()
        .filter(|p| p.health != Health::Unavailable)
        .count();
    let _ = state.hud.set_provider_count(rings);
    let _ = state.hud.set_agents_busy(matches!(
        telemetry.activity,
        Activity::Generating | Activity::AwaitingInput
    ));

    let _ = app.emit(TELEMETRY_EVENT, &telemetry);

    for alert in alerts {
        notify(app, &alert);
    }
}

/// React to a threshold crossing: the number just got interesting, so peek
/// at that provider's card and sound the cue.
///
/// Best-effort: a machine with sound off still gets the ring colour, which is
/// the important part.
fn notify(app: &AppHandle, alert: &Alert) {
    tracing::info!(
        provider = %alert.provider_name,
        window = %alert.window_label,
        pct = alert.used_pct,
        "usage threshold crossed"
    );

    if let Some(state) = app.try_state::<AppState>() {
        let secs = state.hud.config().peek_secs;
        let _ = state
            .hud
            .peek(Duration::from_secs(secs), Some(alert.provider));
    }
    cue(app, Cue::Threshold);
}
