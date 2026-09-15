//! The background polling loop.
//!
//! Runs on Tauri's async runtime, refreshes whichever providers are due, pushes
//! the result to the webview, and decides when something deserves the user's
//! attention (a peek, a notification, or both).

use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager};

use codenotch_core::model::Activity;
use codenotch_core::Alert;

use crate::commands::{AppState, TELEMETRY_EVENT};

/// Never sleep longer than this, so a config change is picked up promptly.
const MAX_SLEEP: Duration = Duration::from_secs(15);

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

    // Keep the window on top and expire any peek, even on ticks where no
    // provider was due.
    let _ = state.hud.tick();

    let previous_activity = state
        .latest
        .lock()
        .ok()
        .map(|t| t.activity)
        .unwrap_or(Activity::Idle);

    let (telemetry, alerts) = {
        let mut collector = state.collector.lock().await;
        collector.poll(chrono::Utc::now()).await
    };

    let peek_secs = state.hud.config().peek_secs;

    // Surface the HUD when an agent starts waiting on the user, or has just
    // finished. Both are transitions, not states: re-peeking every poll while
    // something sits blocked would be intolerable.
    let deserves_attention = telemetry.activity != previous_activity
        && matches!(telemetry.activity, Activity::AwaitingInput | Activity::Done);
    if deserves_attention {
        let _ = state.hud.peek(Duration::from_secs(peek_secs));
    }

    // The strip is one ring per provider that has something to say, so its
    // length follows the collection rather than a fixed guess.
    let rings = telemetry
        .providers
        .iter()
        .filter(|p| p.health != codenotch_core::Health::Unavailable)
        .count();
    let _ = state.hud.set_provider_count(rings);

    if let Ok(mut latest) = state.latest.lock() {
        *latest = telemetry.clone();
    }
    let _ = app.emit(TELEMETRY_EVENT, &telemetry);

    for alert in alerts {
        notify(app, &alert);
    }
}

/// Force a full refresh now (used by the tray's "Refresh now").
pub async fn refresh(app: &AppHandle) {
    if let Some(state) = app.try_state::<AppState>() {
        state.collector.lock().await.invalidate();
    }
    run_once(app).await;
}

/// Show a desktop notification for a threshold crossing.
///
/// Best-effort: a machine with notifications switched off still gets the ring
/// colour, which is the important part.
fn notify(app: &AppHandle, alert: &Alert) {
    tracing::info!(
        provider = %alert.provider_name,
        window = %alert.window_label,
        pct = alert.used_pct,
        "usage threshold crossed"
    );

    // Also nudge the HUD open, since the number just got interesting.
    if let Some(state) = app.try_state::<AppState>() {
        let secs = state.hud.config().peek_secs;
        let _ = state.hud.peek(Duration::from_secs(secs));
    }
}
