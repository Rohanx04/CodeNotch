//! Tauri commands the webview calls, plus the shared application state.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use tauri::{Emitter, Manager, State};

use tokio::sync::Mutex as AsyncMutex;

use codenotch_core::config::{Config, Edge, HudMetrics};
use codenotch_core::hook_settings::{HookPreview, HookStatus};
use codenotch_core::layout::HitRect;
use codenotch_core::live::LiveState;
use codenotch_core::model::{ProviderId, Telemetry};
use codenotch_core::Collector;

use crate::approvals::{ApprovalRequest, Approvals, Reply};
use crate::hud::{Hud, HudState};
use crate::platform;

/// Event carrying a fresh telemetry payload to the webview.
pub const TELEMETRY_EVENT: &str = "codenotch://telemetry";
/// Event carrying updated settings to the webview.
pub const CONFIG_EVENT: &str = "codenotch://config";

/// Everything the commands need. Held in Tauri's state.
pub struct AppState {
    pub hud: Arc<Hud>,
    pub collector: Arc<AsyncMutex<Collector>>,
    /// Latest published telemetry (live hook sessions laid over the
    /// collection), so a reloading webview gets data without waiting.
    pub latest: Arc<Mutex<Telemetry>>,
    /// The collector's last result, before the live overlay: what a hook
    /// event is laid over when it republishes between polls.
    pub raw: Arc<Mutex<Telemetry>>,
    /// Claude Code sessions as the hooks report them.
    pub live: Arc<Mutex<LiveState>>,
    /// The permission request on the notch, if any.
    pub approvals: Arc<Approvals>,
}

/// The payload a freshly loaded webview needs to render immediately.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Bootstrap {
    pub config: Config,
    pub telemetry: Telemetry,
    pub hud: HudState,
    /// Strip and popover dimensions, so the webview draws to the same numbers
    /// the window is sized with rather than its own copy of them.
    pub metrics: HudMetrics,
    pub edge: Edge,
    pub version: String,
    /// False on non-Windows dev builds, where the Win32 layer is a no-op.
    pub native_window: bool,
    /// A permission request already waiting when the webview (re)loaded.
    pub approval: Option<ApprovalRequest>,
}

/// Called once the webview has mounted.
#[tauri::command]
pub fn hud_ready(state: State<'_, AppState>) -> Result<Bootstrap, String> {
    state.hud.apply().map_err(|e| e.to_string())?;

    Ok(Bootstrap {
        config: state.hud.config(),
        telemetry: state.latest.lock().map_err(|e| e.to_string())?.clone(),
        hud: state.hud.state(),
        metrics: state.hud.metrics(),
        edge: state.hud.edge(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        native_window: cfg!(windows),
        approval: state.approvals.current(),
    })
}

/// Pointer entered or left the notch. Only acted on where there is no native
/// cursor to poll (non-Windows dev builds).
#[tauri::command]
pub fn hud_hover(hovering: bool, state: State<'_, AppState>) -> Result<(), String> {
    state.hud.set_hover(hovering).map_err(|e| e.to_string())
}

/// The shapes the webview has painted, which the cursor poll hit-tests to
/// decide where the window takes the mouse.
#[tauri::command]
pub fn hud_set_hit_rects(rects: Vec<HitRect>, state: State<'_, AppState>) {
    state.hud.set_hit_rects(rects);
}

/// The pointer found auto-hide's wake strip.
#[tauri::command]
pub fn hud_wake(state: State<'_, AppState>) -> Result<(), String> {
    state.hud.wake().map_err(|e| e.to_string())
}

/// Close the open card (Escape, when the webview has the keyboard).
#[tauri::command]
pub fn hud_dismiss(state: State<'_, AppState>) -> Result<(), String> {
    state.hud.dismiss().map_err(|e| e.to_string())
}

/// Toggle "stay expanded".
#[tauri::command]
pub fn hud_toggle_pin(state: State<'_, AppState>) -> Result<bool, String> {
    state.hud.toggle_pin().map_err(|e| e.to_string())
}

/// Current settings.
#[tauri::command]
pub fn get_config(state: State<'_, AppState>) -> Config {
    state.hud.config()
}

/// Persist new settings and apply them everywhere.
#[tauri::command]
pub async fn set_config(
    config: Config,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<Config, String> {
    // The webview is not trusted to send sane values.
    let config = config.sanitised();

    // Save first: if this fails the user should hear about it rather than see
    // a setting silently revert on the next launch.
    config.save().map_err(|e| e.to_string())?;

    if cfg!(windows) {
        platform::set_launch_at_login(config.launch_at_login).map_err(|e| e.to_string())?;
    }

    state
        .hud
        .set_config(config.clone())
        .map_err(|e| e.to_string())?;
    state.collector.lock().await.set_config(config.clone());

    let _ = app.emit(CONFIG_EVENT, &config);
    Ok(config)
}

/// Collect every provider right now, rather than waiting for the schedule.
#[tauri::command]
pub async fn refresh_now(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<Telemetry, String> {
    crate::poll::refresh(&app).await;
    Ok(state.latest.lock().map_err(|e| e.to_string())?.clone())
}

/// Bring a provider's window to the front.
///
/// Returns false when nothing matching is running, so the UI can say so instead
/// of appearing to do nothing.
#[tauri::command]
pub fn focus_provider(
    provider: ProviderId,
    title_hint: Option<String>,
    state: State<'_, AppState>,
) -> Result<bool, String> {
    // Close the card first: it is about to be behind whatever we raise.
    let _ = state.hud.dismiss();

    platform::focus_provider_window(provider.focus_processes(), title_hint.as_deref())
        .map_err(|e| e.to_string())
}

/// Hide the HUD (the tray icon stays).
#[tauri::command]
pub fn set_hidden(hidden: bool, state: State<'_, AppState>) -> Result<(), String> {
    state.hud.set_hidden(hidden).map_err(|e| e.to_string())
}

/// Briefly expand the HUD.
#[tauri::command]
pub fn peek(seconds: Option<u64>, state: State<'_, AppState>) -> Result<(), String> {
    let secs = seconds.unwrap_or(5).clamp(1, 60);
    state
        .hud
        .peek(Duration::from_secs(secs), None)
        .map_err(|e| e.to_string())
}

/// Monitors available to pin to, for the settings UI.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MonitorInfo {
    pub index: usize,
    pub label: String,
    pub width: i32,
    pub height: i32,
    pub primary: bool,
}

#[tauri::command]
pub fn list_monitors() -> Vec<MonitorInfo> {
    platform::monitors()
        .into_iter()
        .enumerate()
        .map(|(index, area)| MonitorInfo {
            index,
            label: format!("Display {} · {}×{}", index + 1, area.width, area.height),
            width: area.width,
            height: area.height,
            primary: area.x == 0 && area.y == 0,
        })
        .collect()
}

/// Quit the application.
#[tauri::command]
pub fn quit_app(app: tauri::AppHandle) {
    app.exit(0);
}

/// Open the config file's folder so the user can hand-edit it.
#[tauri::command]
pub fn open_config_dir(app: tauri::AppHandle) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;

    let path = Config::path().map_err(|e| e.to_string())?;
    let dir = path
        .parent()
        .ok_or_else(|| "config path has no parent".to_string())?;
    // Make sure it exists, or the shell will just beep at the user.
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;

    app.opener()
        .open_path(dir.to_string_lossy(), None::<&str>)
        .map_err(|e| e.to_string())
}

/// Whether CodeNotch's Claude Code hooks are installed.
#[tauri::command]
pub fn hooks_status() -> HookStatus {
    crate::hooks::installer().status()
}

/// What installing (or removing) the hooks would change in
/// `~/.claude/settings.json`. Writes nothing.
#[tauri::command]
pub fn hooks_preview(install: bool) -> Result<HookPreview, String> {
    crate::hooks::installer().preview(install, chrono::Local::now())
}

/// Apply a previewed change. Only ever called from an explicit click on the
/// diff the user was shown; `fingerprint` ties the write to that diff.
#[tauri::command]
pub fn hooks_apply(install: bool, fingerprint: String) -> Result<String, String> {
    crate::hooks::installer().write(install, &fingerprint, chrono::Local::now())
}

/// The approval card is on screen and a human can act on it.
#[tauri::command]
pub fn approval_ack(id: String, state: State<'_, AppState>) {
    state.approvals.reply(&id, Reply::Ack);
}

/// Allow or Deny, from an explicit click on the approval card.
#[tauri::command]
pub fn approval_decide(id: String, decision: String, state: State<'_, AppState>) {
    let word = match decision.as_str() {
        "allow" => "allow",
        _ => "deny",
    };
    state.approvals.reply(&id, Reply::Decision(word));
}

/// Convenience for the tray and the poll loop.
pub fn app_state(app: &tauri::AppHandle) -> Option<State<'_, AppState>> {
    app.try_state::<AppState>()
}
