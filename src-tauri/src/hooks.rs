//! Claude Code hooks on the app side: installing them (opt-in, see
//! `codenotch_core::hook_settings`), keeping the relay executable in place,
//! and folding hook events into the live session picture.

use std::path::PathBuf;

use chrono::Utc;
use serde_json::Value;
use tauri::{AppHandle, Manager};

use codenotch_core::hook_settings::HookInstaller;
use codenotch_core::live::HookEvent;

use crate::commands::AppState;

/// File name of the relay, shipped as a bundle resource.
pub const HOOK_EXE: &str = "codenotch-hook.exe";

/// Where the relay lives once installed: `%LOCALAPPDATA%\CodeNotch\bin`.
///
/// A fixed path outside the install directory, so the command written into
/// `settings.json` keeps working across updates and reinstalls.
pub fn hook_exe_path() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("CodeNotch")
        .join("bin")
        .join(HOOK_EXE)
}

pub fn installer() -> HookInstaller {
    let settings = HookInstaller::default_settings_path()
        .unwrap_or_else(|| PathBuf::from(".claude").join("settings.json"));
    HookInstaller::new(settings, hook_exe_path())
}

/// Copy the relay into place on launch.
///
/// It comes from the bundle's resources in an installed build and sits in the
/// workspace's `target/release` under `tauri dev`. Every candidate is tried,
/// because getting this wrong is silent: the hooks would point at a file that
/// does not exist and Claude Code would simply never reach us.
pub fn ensure_hook_exe(app: &AppHandle) {
    let dest = hook_exe_path();
    let Some(dir) = dest.parent() else { return };
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }

    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(p) = app
        .path()
        .resolve(HOOK_EXE, tauri::path::BaseDirectory::Resource)
    {
        candidates.push(p);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            candidates.push(parent.join(HOOK_EXE));
            candidates.push(parent.join("..").join("release").join(HOOK_EXE));
        }
    }

    let Some(src) = candidates.iter().find(|p| p.exists()) else {
        tracing::info!(
            looked_in = ?candidates,
            "codenotch-hook.exe not found; Claude Code hooks cannot be installed from this build"
        );
        return;
    };

    let same = match (std::fs::metadata(src), std::fs::metadata(&dest)) {
        (Ok(a), Ok(b)) => a.len() == b.len() && a.modified().ok() == b.modified().ok(),
        _ => false,
    };
    if same {
        return;
    }
    // A hook may be running right now and hold the file open; keeping the old
    // copy is fine until next launch.
    if let Err(err) = std::fs::copy(src, &dest) {
        if !dest.exists() {
            tracing::warn!(%err, "could not install codenotch-hook.exe");
        }
    }
}

/// A hook event that nobody waits on. Ignored while paused.
pub fn on_event(app: &AppHandle, payload: &Value) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    if state.hud.paused() {
        return;
    }
    if let Some(event) = HookEvent::from_json(payload) {
        tracing::debug!(event = %event.name, "hook");
        apply_event(app, &event);
    }
}

/// Fold one event into the live picture and republish if it changed anything.
pub fn apply_event(app: &AppHandle, event: &HookEvent) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let changed = state
        .live
        .lock()
        .map(|mut live| live.apply(event, Utc::now()))
        .unwrap_or(false);
    if changed {
        crate::poll::publish(app, Vec::new());
    }
}

/// A permission request was answered or released: the session is no longer
/// blocked on the notch.
pub fn resolve_approval(app: &AppHandle, session_id: &str) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    if let Ok(mut live) = state.live.lock() {
        live.resolve_approval(session_id, Utc::now());
    }
    crate::poll::publish(app, Vec::new());
}
