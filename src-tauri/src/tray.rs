//! System tray icon and menu.
//!
//! The tray is the only always-visible affordance when the HUD is hidden, so it
//! carries the show/hide toggle and a way out of the app.

use anyhow::Result;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager};

use crate::commands::app_state;

const ID_SHOW: &str = "toggle-visible";
const ID_PIN: &str = "toggle-pin";
const ID_REFRESH: &str = "refresh";
const ID_SETTINGS: &str = "open-config";
const ID_QUIT: &str = "quit";

/// Build the tray icon and wire its menu.
pub fn build(app: &AppHandle) -> Result<()> {
    let hidden = app_state(app)
        .map(|s| s.hud.config().hidden)
        .unwrap_or(false);

    let show = CheckMenuItem::with_id(app, ID_SHOW, "Show notch", true, !hidden, None::<&str>)?;
    let pin = MenuItem::with_id(app, ID_PIN, "Keep expanded", true, None::<&str>)?;
    let refresh = MenuItem::with_id(app, ID_REFRESH, "Refresh now", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, ID_SETTINGS, "Open config folder", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, ID_QUIT, "Quit CodeNotch", true, None::<&str>)?;

    let menu = Menu::with_items(
        app,
        &[
            &show,
            &pin,
            &PredefinedMenuItem::separator(app)?,
            &refresh,
            &settings,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;

    TrayIconBuilder::with_id("codenotch-tray")
        .icon(
            app.default_window_icon()
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("no default window icon was bundled"))?,
        )
        .tooltip("CodeNotch")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(move |app, event| {
            let handle = app.app_handle().clone();
            match event.id().as_ref() {
                ID_SHOW => {
                    if let Some(state) = app.try_state::<crate::commands::AppState>() {
                        let hidden = state.hud.config().hidden;
                        let _ = state.hud.set_hidden(!hidden);
                        let _ = show.set_checked(hidden);
                    }
                }
                ID_PIN => {
                    if let Some(state) = app.try_state::<crate::commands::AppState>() {
                        let _ = state.hud.toggle_pin();
                    }
                }
                ID_REFRESH => {
                    // The command is async; spawn so the menu handler returns.
                    tauri::async_runtime::spawn(async move {
                        crate::poll::refresh(&handle).await;
                    });
                }
                ID_SETTINGS => {
                    let _ = crate::commands::open_config_dir(handle);
                }
                ID_QUIT => app.exit(0),
                _ => {}
            }
        })
        .on_tray_icon_event(|tray, event| {
            // Left-clicking the tray icon peeks the HUD, which is the quickest
            // way to check usage when the notch is hidden.
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                if let Some(state) = tray.app_handle().try_state::<crate::commands::AppState>() {
                    let _ = state.hud.set_hidden(false);
                    let _ = state.hud.peek(std::time::Duration::from_secs(6));
                }
            }
        })
        .build(app)?;

    Ok(())
}
