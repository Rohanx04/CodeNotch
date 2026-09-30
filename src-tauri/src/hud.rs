//! The HUD window controller: where the notch sits, when it is open, and when
//! it takes the mouse.
//!
//! All sizing and positioning goes through here, so there is exactly one place
//! that decides how big the notch is and one call that moves it — which is
//! what keeps the "never steal focus" guarantee honest.
//!
//! ## One window size
//!
//! While the notch is on screen its window is a fixed panel, big enough for the
//! strip and the tallest card beside it (see `layout::panel_size`). Nothing
//! about the window changes as a card opens, closes or moves between rings, so
//! the webview can animate all of it freely -- the card springs open, plays its
//! close animation in full, and changes size as its content does -- without a
//! single `SetWindowPos`. The window only moves when the settings, the ring
//! count or the displays change, or when auto-hide swaps it for the wake strip.
//!
//! ## Click-through by hit test
//!
//! Most of that panel is empty, so it must not swallow clicks. The webview
//! reports the shapes it has painted (the strip, and the card while one is
//! open) and a cursor poll at ~60 Hz clears `WS_EX_TRANSPARENT` only while the
//! pointer is over one of them, with a generous margin so the flag is already
//! off by the time a moving pointer reaches a button. The same poll decides
//! hover, because a click-through window receives no mouse messages at all.
//!
//! The poll parks on a condvar whenever the notch is hidden or tucked away by
//! auto-hide, so a hidden notch costs no CPU.

use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::Result;
use serde::Serialize;
use tauri::{Emitter, WebviewWindow};

use codenotch_core::config::{Config, Edge, HudMetrics, MonitorChoice};
use codenotch_core::layout::{
    dock_panel, dock_wake, hit_test, panel_size, strip_size, wake_size, HitRect, Placement,
    WorkArea, HIT_MARGIN,
};
use codenotch_core::model::ProviderId;
use codenotch_core::presence::{AutoHide, Presence};

use crate::platform::{self, Backdrop, WindowHandle};

/// Event name the webview listens on for state changes.
pub const HUD_STATE_EVENT: &str = "codenotch://hud-state";

/// Cursor poll interval: one frame at 60 Hz.
const FRAME: Duration = Duration::from_millis(16);
/// Check for display changes every this many frames (about twice a second).
const SCREEN_CHECK_FRAMES: u32 = 30;

/// What the frontend needs to know about the window's own state.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HudState {
    /// A card should be on screen.
    pub open: bool,
    /// The pointer is on the strip or on the open card.
    pub hovering: bool,
    pub pinned: bool,
    pub hidden: bool,
    /// Open because of an attention peek rather than the pointer.
    pub peeking: bool,
    /// When the peek closes, in Unix milliseconds, for the countdown bar.
    pub peek_ends_at: Option<u64>,
    /// The provider a peek or alert is about, so the card shows the right one.
    pub focus: Option<ProviderId>,
    /// A permission request is holding the notch open.
    pub alert: bool,
    /// Auto-hide has tucked the strip into the edge (or is doing so).
    pub retracted: bool,
    /// The window is the thin wake strip rather than the panel.
    pub wake: bool,
    /// Tray → Pause: no collection, peeks, sounds or approvals.
    pub paused: bool,
    /// Logical window size.
    pub width: f64,
    pub height: f64,
    /// The strip's top-left inside the window, in logical pixels.
    pub strip_x: f64,
    pub strip_y: f64,
}

/// Which window the notch currently needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Tray → hide: no window at all.
    Hidden,
    /// Strip plus room for the card.
    Panel,
    /// Auto-hide's thin strip against the edge.
    Wake,
}

#[derive(Debug, Clone, Copy)]
struct Peek {
    until: Instant,
    ends_at_ms: u64,
    focus: Option<ProviderId>,
}

struct Inner {
    config: Config,
    hovering: bool,
    pinned: bool,
    peek: Option<Peek>,
    /// The provider whose permission request is on the notch.
    alert: Option<ProviderId>,
    /// Escape closed the card; it stays closed until the pointer next arrives.
    dismissed: bool,
    paused: bool,
    /// Some agent is working or waiting, which keeps auto-hide away.
    agents_busy: bool,
    providers: usize,
    auto_hide: AutoHide,
    rects: Vec<HitRect>,
    /// Where the window was last put, and at what scale.
    placed: Option<(Mode, Placement, f64)>,
    strip_offset: (f64, f64),
    /// The click-through flag as last applied, so Win32 is only called on change.
    click_through: Option<bool>,
    screen: Option<(Vec<WorkArea>, f64)>,
    frames: u32,
    last_state: Option<HudState>,
}

impl Inner {
    fn peek_active(&self, now: Instant) -> bool {
        self.peek.is_some_and(|p| now < p.until)
    }

    fn mode(&self) -> Mode {
        if self.config.hidden {
            Mode::Hidden
        } else if self.auto_hide.presence() == Presence::Retracted {
            Mode::Wake
        } else {
            Mode::Panel
        }
    }

    /// Whether a card should be showing.
    fn open(&self, now: Instant) -> bool {
        if self.config.hidden || self.auto_hide.presence() != Presence::Shown {
            return false;
        }
        // A permission request outranks everything, Escape included: closing
        // it would leave Claude Code waiting on a card nobody can see.
        if self.alert.is_some() {
            return true;
        }
        !self.dismissed
            && (self.config.always_expanded
                || self.pinned
                || self.hovering
                || self.peek_active(now))
    }

    /// Anything that should keep auto-hide from tucking the notch away.
    fn busy(&self, now: Instant) -> bool {
        self.hovering || self.agents_busy || self.open(now)
    }

    fn metrics(&self) -> HudMetrics {
        self.config.metrics()
    }

    fn state(&self, now: Instant) -> HudState {
        let mode = self.mode();
        let metrics = self.metrics();
        let (width, height) = match mode {
            Mode::Wake => wake_size(metrics, self.config.edge, self.providers),
            _ => panel_size(metrics, self.config.edge, self.providers),
        };
        let peeking = self.peek_active(now);
        HudState {
            open: self.open(now),
            hovering: self.hovering,
            pinned: self.pinned,
            hidden: self.config.hidden,
            peeking,
            peek_ends_at: self.peek.filter(|_| peeking).map(|p| p.ends_at_ms),
            focus: self
                .alert
                .or_else(|| self.peek.filter(|_| peeking).and_then(|p| p.focus)),
            alert: self.alert.is_some(),
            retracted: self.auto_hide.presence() != Presence::Shown,
            wake: mode == Mode::Wake,
            paused: self.paused,
            width,
            height,
            strip_x: self.strip_offset.0,
            strip_y: self.strip_offset.1,
        }
    }

    /// Record a hover change. Leaving cancels a peek (the user has seen it)
    /// and lifts an Escape dismissal, so the next visit opens normally.
    fn hover(&mut self, hovering: bool) -> bool {
        if self.hovering == hovering {
            return false;
        }
        self.hovering = hovering;
        if !hovering {
            self.peek = None;
            self.dismissed = false;
        }
        true
    }
}

/// Parks the cursor poll whenever there is nothing to poll for.
struct Gate {
    active: Mutex<bool>,
    wake: Condvar,
}

impl Gate {
    fn set(&self, on: bool) {
        let mut active = self.active.lock().expect("gate lock");
        if *active != on {
            *active = on;
            self.wake.notify_all();
        }
    }

    fn is_active(&self) -> bool {
        *self.active.lock().expect("gate lock")
    }

    fn wait(&self) {
        let mut active = self.active.lock().expect("gate lock");
        while !*active {
            active = self.wake.wait(active).expect("gate lock");
        }
    }
}

/// Owns the HUD window.
pub struct Hud {
    window: WebviewWindow,
    inner: Mutex<Inner>,
    gate: Gate,
}

fn unix_ms(at: Instant) -> u64 {
    let now = Instant::now();
    let wall = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let offset = at.saturating_duration_since(now);
    (wall + offset).as_millis() as u64
}

fn auto_hide_delay(config: &Config) -> Duration {
    Duration::from_secs(config.auto_hide_secs.clamp(5, 3600))
}

impl Hud {
    pub fn new(window: WebviewWindow, config: Config) -> Self {
        let now = Instant::now();
        Self {
            window,
            inner: Mutex::new(Inner {
                auto_hide: AutoHide::new(config.auto_hide, auto_hide_delay(&config), now),
                config,
                hovering: false,
                pinned: false,
                peek: None,
                alert: None,
                dismissed: false,
                paused: false,
                agents_busy: false,
                providers: 0,
                rects: Vec::new(),
                placed: None,
                strip_offset: (0.0, 0.0),
                click_through: None,
                screen: None,
                frames: 0,
                last_state: None,
            }),
            gate: Gate {
                active: Mutex::new(false),
                wake: Condvar::new(),
            },
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().expect("hud lock")
    }

    pub fn window(&self) -> &WebviewWindow {
        &self.window
    }

    pub fn config(&self) -> Config {
        self.lock().config.clone()
    }

    pub fn metrics(&self) -> HudMetrics {
        self.lock().metrics()
    }

    pub fn edge(&self) -> Edge {
        self.lock().config.edge
    }

    pub fn paused(&self) -> bool {
        self.lock().paused
    }

    pub fn state(&self) -> HudState {
        self.lock().state(Instant::now())
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

    /// One-time setup: HUD window styles and the appearance attributes.
    pub fn initialise(&self) -> Result<()> {
        if let Some(handle) = self.handle() {
            // Click-through from the start: the cursor poll clears it over the
            // strip once the webview has reported where the strip is.
            platform::apply_hud_chrome(handle, true)?;
            platform::apply_appearance(handle, Backdrop::None);
        }
        self.lock().click_through = Some(true);
        self.apply()
    }

    /// Start the cursor poll. It runs at ~60 Hz while the panel is on screen
    /// and sleeps on a condvar the rest of the time.
    pub fn spawn_cursor_poll(self: &Arc<Self>) {
        let hud = Arc::clone(self);
        let spawned = std::thread::Builder::new()
            .name("hud-cursor".into())
            .spawn(move || loop {
                hud.gate.wait();
                while hud.gate.is_active() {
                    std::thread::sleep(FRAME);
                    hud.frame();
                }
            });
        if let Err(err) = spawned {
            tracing::warn!(%err, "could not start the cursor poll; hover will rely on the webview");
        }
    }

    /// Recompute where the window belongs, move it if it has to, and push the
    /// state to the webview when it changed.
    ///
    /// No lock is held across a Win32 call: moving the window sends it
    /// messages synchronously, and a command waiting on the lock on the main
    /// thread would then deadlock with us.
    pub fn apply(&self) -> Result<()> {
        let (mode, monitor, edge, offset, margin, metrics, providers) = {
            let inner = self.lock();
            let c = &inner.config;
            (
                inner.mode(),
                match c.monitor {
                    MonitorChoice::Primary => None,
                    MonitorChoice::Index(i) => Some(i),
                },
                c.edge,
                c.edge_offset,
                c.margin,
                inner.metrics(),
                inner.providers,
            )
        };

        let Some(handle) = self.handle() else {
            return Ok(());
        };

        let target = if mode == Mode::Hidden {
            None
        } else {
            let area = platform::work_area(handle, monitor)?;
            Some(match mode {
                Mode::Wake => {
                    let wake = dock_wake(area, edge, offset, wake_size(metrics, edge, providers));
                    (wake, (0.0, 0.0), area.scale)
                }
                _ => {
                    let docked = dock_panel(
                        area,
                        edge,
                        offset,
                        margin,
                        strip_size(metrics, edge, providers),
                        panel_size(metrics, edge, providers),
                    );
                    (docked.window, (docked.strip_x, docked.strip_y), area.scale)
                }
            })
        };

        let (moved, click_through, state, changed) = {
            let mut inner = self.lock();
            let previous_mode = inner.placed.map(|(m, _, _)| m);
            let moved = match target {
                Some((placement, offset, scale)) => {
                    inner.strip_offset = offset;
                    let same = inner
                        .placed
                        .is_some_and(|(m, p, s)| m == mode && p == placement && s == scale);
                    inner.placed = Some((mode, placement, scale));
                    (!same).then_some(placement)
                }
                None => {
                    inner.placed = None;
                    None
                }
            };

            // The wake strip exists to be hovered, so it always takes the
            // mouse. Arriving back on the panel it starts click-through until
            // the poll finds the pointer on a shape. Otherwise the poll owns
            // the flag: resetting it on every re-dock would let a click through
            // the settings card for a frame each time a setting changed.
            let want = match mode {
                Mode::Wake => Some(false),
                Mode::Panel if previous_mode != Some(Mode::Panel) => Some(true),
                _ => None,
            };
            let click_through = want.filter(|w| inner.click_through != Some(*w));
            if let Some(w) = click_through {
                inner.click_through = Some(w);
            }

            let state = inner.state(Instant::now());
            let changed = inner.last_state.as_ref() != Some(&state);
            inner.last_state = Some(state.clone());
            (moved, click_through, state, changed)
        };

        if mode == Mode::Hidden {
            let _ = self.window.hide();
        } else {
            if let Some(placement) = moved {
                platform::move_no_activate(handle, placement)?;
                // Off Windows the platform layer is a no-op, so drive Tauri
                // directly to keep the dev experience usable there.
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
            }
            if let Some(enabled) = click_through {
                platform::set_click_through(handle, enabled)?;
            }
            let _ = self.window.show();
        }

        self.gate.set(mode == Mode::Panel);

        if changed {
            let _ = self.window.emit(HUD_STATE_EVENT, &state);
        }
        Ok(())
    }

    /// One cursor-poll frame: hover and click-through from the pointer, peek
    /// expiry, auto-hide, and display changes.
    fn frame(&self) {
        let now = Instant::now();
        let cursor = platform::cursor_pos();
        let check_screen;
        let (flip, relayout) = {
            let mut inner = self.lock();
            if inner.mode() != Mode::Panel {
                return;
            }
            let mut relayout = false;
            let mut flip = None;

            // Hover and click-through, when there is a real cursor to read.
            if let (Some((cx, cy)), Some((_, placement, scale))) = (cursor, inner.placed) {
                let x = (cx - placement.x) as f64 / scale;
                let y = (cy - placement.y) as f64 / scale;
                let inside = hit_test(&inner.rects, x, y, HIT_MARGIN);
                if inner.click_through != Some(!inside) {
                    inner.click_through = Some(!inside);
                    flip = Some(!inside);
                }
                relayout |= inner.hover(inside);
            }

            if inner.peek.is_some_and(|p| now >= p.until) {
                inner.peek = None;
                relayout = true;
            }

            let busy = inner.busy(now);
            relayout |= inner.auto_hide.tick(busy, now);

            inner.frames = inner.frames.wrapping_add(1);
            check_screen = inner.frames % SCREEN_CHECK_FRAMES == 0;
            (flip, relayout)
        };

        if let (Some(enabled), Some(handle)) = (flip, self.handle()) {
            let _ = platform::set_click_through(handle, enabled);
        }

        // Monitors get plugged in, unplugged, rearranged and rescaled, and a
        // notch pinned to coordinates that no longer exist is a notch nobody
        // can reach.
        let mut screen_changed = false;
        if check_screen {
            if let Some(handle) = self.handle() {
                let key = (platform::monitors(), platform::window_scale(handle));
                let mut inner = self.lock();
                screen_changed = inner.screen.as_ref().is_some_and(|k| *k != key);
                inner.screen = Some(key);
                if screen_changed {
                    inner.placed = None;
                }
            }
        }

        if relayout || screen_changed {
            if screen_changed {
                tracing::info!("display layout changed — repositioning the notch");
            }
            let _ = self.apply();
        }
    }

    /// The shapes the webview has painted, in window-logical pixels.
    pub fn set_hit_rects(&self, rects: Vec<HitRect>) {
        self.lock().rects = rects;
    }

    /// Pointer entered or left the notch, as reported by the webview. Only
    /// used where there is no native cursor to poll (non-Windows dev builds).
    pub fn set_hover(&self, hovering: bool) -> Result<()> {
        if platform::cursor_pos().is_some() {
            return Ok(());
        }
        let changed = self.lock().hover(hovering);
        if changed {
            self.apply()?;
        }
        Ok(())
    }

    /// Close the card: Escape pressed while the notch has the keyboard, or a
    /// provider window just raised over it.
    pub fn dismiss(&self) -> Result<()> {
        {
            let mut inner = self.lock();
            if inner.alert.is_some() {
                return Ok(());
            }
            inner.peek = None;
            inner.dismissed = true;
        }
        self.apply()
    }

    /// The pointer found the wake strip, or something else wants the notch back.
    pub fn wake(&self) -> Result<()> {
        let changed = self.lock().auto_hide.wake(Instant::now());
        if changed {
            self.apply()?;
        }
        Ok(())
    }

    /// How many rings the strip has to hold.
    pub fn set_provider_count(&self, providers: usize) -> Result<()> {
        {
            let mut inner = self.lock();
            if inner.providers == providers {
                return Ok(());
            }
            inner.providers = providers;
        }
        self.apply()
    }

    /// Whether any agent is working or waiting. A change in either direction
    /// matters to auto-hide; starting work also brings a tucked-away notch back.
    pub fn set_agents_busy(&self, busy: bool) -> Result<()> {
        let woke = {
            let mut inner = self.lock();
            if inner.agents_busy == busy {
                return Ok(());
            }
            inner.agents_busy = busy;
            busy && inner.auto_hide.wake(Instant::now())
        };
        if woke {
            self.apply()?;
        }
        Ok(())
    }

    /// Toggle "stay open", returning the new pinned state.
    pub fn toggle_pin(&self) -> Result<bool> {
        let pinned = {
            let mut inner = self.lock();
            inner.pinned = !inner.pinned;
            inner.dismissed = false;
            inner.auto_hide.wake(Instant::now());
            inner.pinned
        };
        self.apply()?;
        Ok(pinned)
    }

    /// Briefly open the notch on `focus`'s card, for when an agent needs
    /// attention. Time-boxed and cancellable -- a peek is useless behind a
    /// full-screen window -- and never while paused.
    pub fn peek(&self, duration: Duration, focus: Option<ProviderId>) -> Result<()> {
        {
            let mut inner = self.lock();
            if inner.config.hidden || !inner.config.peek_on_attention || inner.paused {
                return Ok(());
            }
            let now = Instant::now();
            let until = now + duration;
            // Never shorten an in-flight peek, but let a newer one retarget it.
            let until = inner
                .peek
                .filter(|p| p.until > until)
                .map_or(until, |p| p.until);
            inner.peek = Some(Peek {
                until,
                ends_at_ms: unix_ms(until),
                focus: focus.or_else(|| inner.peek.and_then(|p| p.focus)),
            });
            inner.dismissed = false;
            inner.auto_hide.wake(now);
        }
        self.apply()
    }

    /// Hold the notch open on a permission request, or release it.
    pub fn set_alert(&self, provider: Option<ProviderId>) -> Result<()> {
        {
            let mut inner = self.lock();
            if inner.alert == provider {
                return Ok(());
            }
            inner.alert = provider;
            if provider.is_some() {
                inner.auto_hide.wake(Instant::now());
            }
        }
        self.apply()
    }

    pub fn set_paused(&self, paused: bool) -> Result<()> {
        {
            let mut inner = self.lock();
            if inner.paused == paused {
                return Ok(());
            }
            inner.paused = paused;
            if paused {
                inner.peek = None;
            }
        }
        self.apply()
    }

    /// Re-assert topmost z-order. Other always-on-top windows (and apps going
    /// full-screen) can quietly push us down.
    pub fn keep_on_top(&self) {
        let visible = self.lock().mode() != Mode::Hidden;
        if let (true, Some(handle)) = (visible, self.handle()) {
            let _ = platform::set_topmost(handle);
        }
    }

    /// Apply new settings, re-running the appearance calls the change affects.
    pub fn set_config(&self, config: Config) -> Result<()> {
        {
            let mut inner = self.lock();
            let now = Instant::now();
            inner
                .auto_hide
                .configure(config.auto_hide, auto_hide_delay(&config), now);
            inner.config = config;
        }

        if let Some(handle) = self.handle() {
            // Re-assert the styles with click-through as the poll last set it,
            // so changing a setting from the card never makes the card
            // transparent under the pointer that is changing it.
            let click_through = self.lock().click_through.unwrap_or(true);
            platform::apply_hud_chrome(handle, click_through)?;
            platform::apply_appearance(handle, Backdrop::None);
            platform::refresh_frame(handle);
        }
        self.apply()
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
