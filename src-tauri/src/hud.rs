//! The HUD window controller: what size the notch is and where it sits.
//!
//! All sizing and positioning goes through here so there is exactly one place
//! that decides how big the notch is and one call that moves it — which is what
//! keeps the "never steal focus" guarantee honest.
//!
//! Hover is driven from the cursor position rather than from the webview's own
//! mouse events. It has to be: while the notch is resting it carries
//! `WS_EX_TRANSPARENT` so clicks fall through to whatever is underneath, and a
//! click-through window receives no mouse messages at all — not even
//! `mouseenter`. Polling `GetCursorPos` is what lets the notch be both
//! click-through *and* hoverable.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::Result;
use serde::Serialize;
use tauri::{Emitter, WebviewWindow};

use codenotch_core::config::{Config, Edge, HudMetrics, MonitorChoice};
use codenotch_core::layout::{hud_extent, Placement};

use crate::platform::{self, Backdrop, WindowHandle};

/// Event name the webview listens on for open/close changes.
pub const HUD_STATE_EVENT: &str = "codenotch://hud-state";

/// What the frontend needs to know about the window's own state.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HudState {
    /// A detail popover is open, so the window has grown inward to hold it.
    pub open: bool,
    pub pinned: bool,
    pub hidden: bool,
    /// True while a temporary attention peek is showing.
    pub peeking: bool,
    /// Logical size of the window, so the webview can place the strip within it.
    pub width: f64,
    pub height: f64,
}

struct Inner {
    config: Config,
    /// Cursor is over the notch (or over the popover it opened).
    hovering: bool,
    pinned: bool,
    peek_until: Option<Instant>,
    /// Number of rings to make room for.
    providers: usize,
    /// Logical content size the webview measured.
    content: Option<(f64, f64)>,
    last_placement: Option<Placement>,
    last_state: Option<HudState>,
}

impl Inner {
    /// Open when anything is asking for it.
    fn open(&self) -> bool {
        if self.config.hidden {
            return false;
        }
        self.config.always_expanded
            || self.pinned
            || self.hovering
            || self.peek_until.is_some_and(|t| Instant::now() < t)
    }

    fn metrics(&self) -> HudMetrics {
        self.config.metrics()
    }

    /// Logical window size for the current state.
    fn extent(&self) -> (f64, f64) {
        hud_extent(
            self.metrics(),
            self.config.edge,
            self.providers,
            self.open(),
            self.content,
        )
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
                providers: 0,
                content: None,
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

    pub fn metrics(&self) -> HudMetrics {
        self.inner.lock().expect("hud lock").metrics()
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
        platform::apply_appearance(handle, Backdrop::None);
        self.apply()?;

        if !config.hidden {
            let _ = self.window.show();
        }
        Ok(())
    }

    /// Recompute size/position and push the state to the webview.
    pub fn apply(&self) -> Result<()> {
        let (config, open, state, changed) = {
            let mut inner = self.inner.lock().expect("hud lock");
            let (width, height) = inner.extent();
            let state = HudState {
                open: inner.open(),
                pinned: inner.pinned,
                hidden: inner.config.hidden,
                peeking: inner.peek_until.is_some_and(|t| Instant::now() < t),
                width,
                height,
            };
            let changed = inner.last_state.as_ref() != Some(&state);
            inner.last_state = Some(state.clone());
            (inner.config.clone(), state.open, state, changed)
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

        // Clicks only pass through while the notch is resting; an open popover
        // has things on it to click.
        let click_through = !open && config.click_through_when_collapsed;
        platform::set_click_through(handle, click_through)?;

        let monitor = match config.monitor {
            MonitorChoice::Primary => None,
            MonitorChoice::Index(i) => Some(i),
        };

        let placement = platform::dock(
            handle,
            monitor,
            config.edge,
            config.edge_offset,
            config.margin,
            state.width,
            state.height,
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

    /// Pointer entered or left the notch.
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

    /// Record the size the webview measured for its content.
    pub fn set_content_size(&self, width: f64, height: f64) -> Result<()> {
        {
            let mut inner = self.inner.lock().expect("hud lock");
            // Ignore sub-pixel churn; every change costs a SetWindowPos.
            if inner
                .content
                .is_some_and(|(w, h)| (w - width).abs() < 1.0 && (h - height).abs() < 1.0)
            {
                return Ok(());
            }
            inner.content = Some((width, height));
        }
        self.apply()
    }

    /// How many rings the strip has to hold.
    pub fn set_provider_count(&self, providers: usize) -> Result<()> {
        {
            let mut inner = self.inner.lock().expect("hud lock");
            if inner.providers == providers {
                return Ok(());
            }
            inner.providers = providers;
            // The measured size belongs to the old ring count.
            inner.content = None;
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

    /// Briefly open the notch, for when an agent needs attention.
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

    /// The rectangle the cursor has to be inside for the notch to open.
    ///
    /// While resting that is the strip itself. While open it is the whole
    /// window, so moving from a ring onto its popover doesn't close it.
    fn hover_rect(&self) -> Option<Placement> {
        let inner = self.inner.lock().expect("hud lock");
        let placement = inner.last_placement?;
        if inner.open() {
            return Some(placement);
        }

        // Resting: the window is already strip-sized, so it is the rect.
        Some(placement)
    }

    /// Poll the cursor and open or close the notch to match.
    ///
    /// Returns true when the hover state changed.
    fn poll_cursor(&self) -> Result<bool> {
        let Some((x, y)) = platform::cursor_pos() else {
            // No cursor source (non-Windows): the webview's own mouse events
            // drive hover instead.
            return Ok(false);
        };
        let Some(rect) = self.hover_rect() else {
            return Ok(false);
        };

        let inside =
            x >= rect.x && x < rect.x + rect.width && y >= rect.y && y < rect.y + rect.height;

        let changed = {
            let inner = self.inner.lock().expect("hud lock");
            inner.hovering != inside
        };
        if changed {
            self.set_hover(inside)?;
        }
        Ok(changed)
    }

    /// Called from the poll loop: track the cursor, expire peeks, stay on top.
    ///
    /// The topmost re-assert matters because other always-on-top windows (and
    /// apps going full-screen) can quietly push us down the z-order.
    pub fn tick(&self) -> Result<()> {
        if self.config().hidden {
            return Ok(());
        }

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

        let moved = self.poll_cursor()?;

        if expired && !moved {
            self.apply()?;
        } else if let Some(handle) = self.handle() {
            let _ = platform::set_topmost(handle);
        }
        Ok(())
    }

    /// Apply new settings, re-running the appearance calls the change affects.
    pub fn set_config(&self, config: Config) -> Result<()> {
        {
            let mut inner = self.inner.lock().expect("hud lock");
            // Size and edge changes invalidate what the webview measured.
            if inner.config.size != config.size || inner.config.edge != config.edge {
                inner.content = None;
            }
            inner.config = config.clone();
        }

        if let Some(handle) = self.handle() {
            platform::apply_hud_chrome(handle, config.click_through_when_collapsed)?;
            // Cheap, and none of it depends on the config: re-asserting keeps
            // the frame suppressed if anything has reset it since setup. (This
            // used to run only when the accent changed, back when the accent
            // tinted the window border; it paints the rings and nothing else
            // now.)
            platform::apply_appearance(handle, Backdrop::None);
            platform::refresh_frame(handle);
        }
        self.apply()
    }

    pub fn state(&self) -> HudState {
        let inner = self.inner.lock().expect("hud lock");
        let (width, height) = inner.extent();
        HudState {
            open: inner.open(),
            pinned: inner.pinned,
            hidden: inner.config.hidden,
            peeking: inner.peek_until.is_some_and(|t| Instant::now() < t),
            width,
            height,
        }
    }

    /// Which edge the strip is docked to, for the webview's own layout.
    pub fn edge(&self) -> Edge {
        self.inner.lock().expect("hud lock").config.edge
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
