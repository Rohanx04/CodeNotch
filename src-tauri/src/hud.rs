//! The HUD window controller: collapsed/expanded state and where it sits.
//!
//! All sizing and positioning goes through here so there is exactly one place
//! that decides how big the notch is and one call that moves it — which is what
//! keeps the "never steal focus" guarantee honest.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::Result;
use serde::Serialize;
use tauri::{Emitter, WebviewWindow};

use codenotch_core::config::Config;
use codenotch_core::layout::{hud_extent, Placement};

use crate::platform::{self, Backdrop, WindowHandle};

/// Event name the webview listens on for expand/collapse changes.
pub const HUD_STATE_EVENT: &str = "codenotch://hud-state";

/// What the frontend needs to know about the window's own state.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HudState {
    pub expanded: bool,
    pub pinned: bool,
    pub hidden: bool,
    /// True while a temporary attention peek is showing.
    pub peeking: bool,
}

struct Inner {
    config: Config,
    hovering: bool,
    pinned: bool,
    peek_until: Option<Instant>,
    /// Content height measured by the webview, in logical pixels.
    content_height: Option<f64>,
    last_placement: Option<Placement>,
    last_state: Option<HudState>,
}

impl Inner {
    /// Expanded when anything is asking for it.
    fn expanded(&self) -> bool {
        if self.config.hidden {
            return false;
        }
        self.config.always_expanded
            || self.pinned
            || self.hovering
            || self.peek_until.is_some_and(|t| Instant::now() < t)
    }

    fn state(&self) -> HudState {
        HudState {
            expanded: self.expanded(),
            pinned: self.pinned,
            hidden: self.config.hidden,
            peeking: self.peek_until.is_some_and(|t| Instant::now() < t),
        }
    }
}

/// Owns the HUD window.
pub struct Hud {
    window: WebviewWindow,
    inner: Mutex<Inner>,
}

impl Hud {
    pub fn new(window: WebviewWindow, config: Config) -> Self {
        Self {
            window,
            inner: Mutex::new(Inner {
                config,
                hovering: false,
                pinned: false,
                peek_until: None,
                content_height: None,
                last_placement: None,
                last_state: None,
            }),
        }
    }

    pub fn window(&self) -> &WebviewWindow {
        &self.window
    }

    pub fn config(&self) -> Config {
        self.inner.lock().expect("hud lock").config.clone()
    }

    /// Native handle, or `None` if the window has already gone away.
    fn handle(&self) -> Option<WindowHandle> {
        #[cfg(windows)]
        {
            self.window.hwnd().ok().map(|h| h.0 as WindowHandle)
        }
        #[cfg(not(windows))]
        {
            Some(0)
        }
    }

    /// One-time setup: HUD window styles and the Win11 appearance attributes.
    pub fn initialise(&self) -> Result<()> {
        let config = self.config();
        let Some(handle) = self.handle() else {
            return Ok(());
        };

        platform::apply_hud_chrome(handle, config.click_through_when_collapsed)?;
        platform::apply_appearance(
            handle,
            Backdrop::Inherit,
            platform::parse_hex_colour(&config.accent_hex()),
        );
        self.apply()?;

        if !config.hidden {
            let _ = self.window.show();
        }
        Ok(())
    }

    /// Recompute size/position and push the state to the webview.
    pub fn apply(&self) -> Result<()> {
        let (config, expanded, content_height, state, changed) = {
            let mut inner = self.inner.lock().expect("hud lock");
            let state = inner.state();
            let changed = inner.last_state.as_ref() != Some(&state);
            inner.last_state = Some(state.clone());
            (
                inner.config.clone(),
                state.expanded,
                inner.content_height,
                state,
                changed,
            )
        };

        if config.hidden {
            let _ = self.window.hide();
            if changed {
                let _ = self.window.emit(HUD_STATE_EVENT, &state);
            }
            return Ok(());
        }

        let Some(handle) = self.handle() else {
            return Ok(());
        };

        let (width, height) = hud_extent(config.metrics(), expanded, content_height);
        let monitor = match config.monitor {
            codenotch_core::config::MonitorChoice::Primary => None,
            codenotch_core::config::MonitorChoice::Index(i) => Some(i),
        };

        // Clicks only pass through while the pill is resting; an expanded card
        // has buttons on it.
        let click_through = !expanded && config.click_through_when_collapsed;
        platform::set_click_through(handle, click_through)?;

        let placement = platform::dock(
            handle,
            monitor,
            config.edge,
            config.edge_offset,
            config.margin,
            width,
            height,
        )?;

        // On non-Windows the platform layer is a no-op, so drive Tauri directly
        // to keep the dev experience usable there.
        #[cfg(not(windows))]
        {
            use tauri::{PhysicalPosition, PhysicalSize};
            let _ = self.window.set_size(PhysicalSize::new(
                placement.width.max(1) as u32,
                placement.height.max(1) as u32,
            ));
            let _ = self
                .window
                .set_position(PhysicalPosition::new(placement.x, placement.y));
        }

        self.inner.lock().expect("hud lock").last_placement = Some(placement);

        let _ = self.window.show();
        if changed {
            let _ = self.window.emit(HUD_STATE_EVENT, &state);
        }
        Ok(())
    }

    /// Pointer entered or left the window.
    pub fn set_hover(&self, hovering: bool) -> Result<()> {
        {
            let mut inner = self.inner.lock().expect("hud lock");
            if inner.hovering == hovering {
                return Ok(());
            }
            inner.hovering = hovering;
            // Leaving cancels a peek: the user has seen it.
            if !hovering {
                inner.peek_until = None;
            }
        }
        self.apply()
    }

    /// Record the height the webview measured for the expanded card.
    pub fn set_content_height(&self, height: f64) -> Result<()> {
        {
            let mut inner = self.inner.lock().expect("hud lock");
            // Ignore sub-pixel churn; every change costs a SetWindowPos.
            if inner
                .content_height
                .is_some_and(|h| (h - height).abs() < 1.0)
            {
                return Ok(());
            }
            inner.content_height = Some(height);
            if !inner.expanded() {
                return Ok(());
            }
        }
        self.apply()
    }

    /// Toggle "stay open", returning the new pinned state.
    pub fn toggle_pin(&self) -> Result<bool> {
        let pinned = {
            let mut inner = self.inner.lock().expect("hud lock");
            inner.pinned = !inner.pinned;
            inner.pinned
        };
        self.apply()?;
        Ok(pinned)
    }

    /// Briefly show the expanded card, for when an agent needs attention.
    ///
    /// The macOS original's rationale applies: a peek is useless behind a
    /// full-screen window, so it is time-boxed and always cancellable.
    pub fn peek(&self, duration: Duration) -> Result<()> {
        {
            let mut inner = self.inner.lock().expect("hud lock");
            if inner.config.hidden || !inner.config.peek_on_attention {
                return Ok(());
            }
            let until = Instant::now() + duration;
            // Never shorten an in-flight peek.
            if inner.peek_until.is_some_and(|t| t >= until) {
                return Ok(());
            }
            inner.peek_until = Some(until);
        }
        self.apply()
    }

    /// Called from the poll loop: expire peeks and keep the window on top.
    ///
    /// The topmost re-assert matters because other always-on-top windows (and
    /// apps going full-screen) can quietly push us down the z-order.
    pub fn tick(&self) -> Result<()> {
        let expired = {
            let mut inner = self.inner.lock().expect("hud lock");
            match inner.peek_until {
                Some(t) if Instant::now() >= t => {
                    inner.peek_until = None;
                    true
                }
                _ => false,
            }
        };

        if expired {
            self.apply()?;
        } else if let Some(handle) = self.handle() {
            if !self.config().hidden {
                let _ = platform::set_topmost(handle);
            }
        }
        Ok(())
    }

    /// Apply new settings, re-running the appearance calls the change affects.
    pub fn set_config(&self, config: Config) -> Result<()> {
        let accent_changed;
        {
            let mut inner = self.inner.lock().expect("hud lock");
            accent_changed = inner.config.accent_hex() != config.accent_hex();
            inner.config = config.clone();
        }

        if let Some(handle) = self.handle() {
            platform::apply_hud_chrome(handle, config.click_through_when_collapsed)?;
            if accent_changed {
                platform::apply_appearance(
                    handle,
                    Backdrop::Inherit,
                    platform::parse_hex_colour(&config.accent_hex()),
                );
                platform::refresh_frame(handle);
            }
        }
        self.apply()
    }

    pub fn state(&self) -> HudState {
        self.inner.lock().expect("hud lock").state()
    }

    /// Show/hide without discarding the rest of the configuration.
    pub fn set_hidden(&self, hidden: bool) -> Result<()> {
        let mut config = self.config();
        if config.hidden == hidden {
            return Ok(());
        }
        config.hidden = hidden;
        let _ = config.save();
        self.set_config(config)
    }
}
