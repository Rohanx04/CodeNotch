//! CodeNotch: a Windows HUD overlay for AI coding-assistant usage limits.
//!
//! The Tauri app is thin on purpose. All the collection logic lives in
//! `codenotch-core` (which builds and tests anywhere); this crate owns the
//! window, the Win32 integration, the tray, and the IPC surface.

pub mod approvals;
pub mod commands;
pub mod hooks;
pub mod hud;
#[cfg(windows)]
pub mod pipe;
pub mod platform;
pub mod poll;
pub mod tray;

use std::sync::{Arc, Mutex};

use tauri::Manager;
use tokio::sync::Mutex as AsyncMutex;

use codenotch_core::config::Config;
use codenotch_core::live::LiveState;
use codenotch_core::model::Telemetry;
use codenotch_core::Collector;

use approvals::Approvals;

use commands::AppState;
use hud::Hud;

/// Label of the HUD window declared in `tauri.conf.json`.
const HUD_WINDOW: &str = "hud";

/// Build and run the application.
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("CODENOTCH_LOG")
                .unwrap_or_else(|_| "codenotch=info,codenotch_core=info".into()),
        )
        .with_target(false)
        .init();

    tauri::Builder::default()
        // A second launch should surface the existing notch, not start a rival
        // one fighting over the same screen corner.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Some(state) = app.try_state::<AppState>() {
                let _ = state.hud.set_hidden(false);
                let _ = state.hud.wake();
                let _ = state.hud.peek(std::time::Duration::from_secs(5), None);
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            commands::hud_ready,
            commands::hud_hover,
            commands::hud_set_hit_rects,
            commands::hud_wake,
            commands::hud_dismiss,
            commands::hud_toggle_pin,
            commands::get_config,
            commands::set_config,
            commands::refresh_now,
            commands::focus_provider,
            commands::set_hidden,
            commands::peek,
            commands::list_monitors,
            commands::open_config_dir,
            commands::quit_app,
            commands::hooks_status,
            commands::hooks_preview,
            commands::hooks_apply,
            commands::approval_ack,
            commands::approval_decide,
        ])
        .setup(|app| {
            let config = Config::load();

            let window = app
                .get_webview_window(HUD_WINDOW)
                .ok_or_else(|| format!("window `{HUD_WINDOW}` is missing from tauri.conf.json"))?;

            let hud = Arc::new(Hud::new(window, config.clone()));
            hud.initialise()?;
            hud.spawn_cursor_poll();

            app.manage(AppState {
                hud: hud.clone(),
                collector: Arc::new(AsyncMutex::new(Collector::new(config))),
                latest: Arc::new(Mutex::new(Telemetry::empty())),
                raw: Arc::new(Mutex::new(Telemetry::empty())),
                live: Arc::new(Mutex::new(LiveState::default())),
                approvals: Arc::new(Approvals::default()),
            });

            // The hook relay and its pipe. Both are harmless when the hooks
            // are not installed: nothing ever connects.
            #[cfg(windows)]
            {
                hooks::ensure_hook_exe(app.handle());
                pipe::start(app.handle().clone());
            }

            if let Err(err) = tray::build(&app.handle().clone()) {
                // A missing tray is survivable; a missing HUD is not.
                tracing::warn!(%err, "could not create the tray icon");
            }

            poll::spawn(app.handle().clone());
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running CodeNotch");
}
