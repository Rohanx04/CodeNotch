//! Auto-hide: when nothing is going on, the notch tucks itself into the edge.
//!
//! Three states. **Shown** is the notch as usual. After `idle_after` with no
//! hover, no open card and no agent at work it starts **Retracting**: the
//! webview slides the strip into the screen edge, and once that animation has
//! had time to play the notch is **Retracted** -- the window shrinks to a thin
//! wake strip along the edge and the cursor poll parks, so a hidden notch costs
//! no CPU at all. Anything that needs the user (a hover on the wake strip, an
//! agent starting, finishing or asking for permission) brings it straight back.
//!
//! Pure state and explicit clocks, so every transition is unit-tested.

use std::time::{Duration, Instant};

/// How long the webview gets to slide the strip away before the window
/// shrinks under it. A little longer than the 340 ms close curve.
pub const RETRACT_ANIMATION: Duration = Duration::from_millis(420);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Presence {
    Shown,
    Retracting,
    Retracted,
}

#[derive(Debug, Clone)]
pub struct AutoHide {
    enabled: bool,
    idle_after: Duration,
    presence: Presence,
    last_busy: Instant,
    retracted_at: Option<Instant>,
}

impl AutoHide {
    pub fn new(enabled: bool, idle_after: Duration, now: Instant) -> Self {
        Self {
            enabled,
            idle_after,
            presence: Presence::Shown,
            last_busy: now,
            retracted_at: None,
        }
    }

    pub fn presence(&self) -> Presence {
        self.presence
    }

    /// Apply new settings. Turning auto-hide off brings the notch back at once;
    /// either way the idle clock restarts, so a settings change never makes the
    /// notch vanish from under the pointer that just changed it.
    ///
    /// Returns true when the presence changed.
    pub fn configure(&mut self, enabled: bool, idle_after: Duration, now: Instant) -> bool {
        self.enabled = enabled;
        self.idle_after = idle_after;
        self.wake(now)
    }

    /// Something needs the notch: show it and restart the idle clock.
    ///
    /// Returns true when the presence changed.
    pub fn wake(&mut self, now: Instant) -> bool {
        self.last_busy = now;
        self.retracted_at = None;
        let changed = self.presence != Presence::Shown;
        self.presence = Presence::Shown;
        changed
    }

    /// Advance the clock. `busy` is anything that should keep the notch out:
    /// the pointer on it, a card open, an agent working or waiting.
    ///
    /// Returns true when the presence changed.
    pub fn tick(&mut self, busy: bool, now: Instant) -> bool {
        if !self.enabled || busy {
            return self.wake(now);
        }
        match self.presence {
            Presence::Shown if now.saturating_duration_since(self.last_busy) >= self.idle_after => {
                self.presence = Presence::Retracting;
                self.retracted_at = Some(now + RETRACT_ANIMATION);
                true
            }
            Presence::Retracting if self.retracted_at.is_some_and(|t| now >= t) => {
                self.presence = Presence::Retracted;
                true
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const IDLE: Duration = Duration::from_secs(60);

    #[test]
    fn disabled_never_hides() {
        let t0 = Instant::now();
        let mut a = AutoHide::new(false, IDLE, t0);
        assert!(!a.tick(false, t0 + Duration::from_secs(3600)));
        assert_eq!(a.presence(), Presence::Shown);
    }

    #[test]
    fn idle_retracts_then_the_window_shrinks_after_the_animation() {
        let t0 = Instant::now();
        let mut a = AutoHide::new(true, IDLE, t0);
        assert!(!a.tick(false, t0 + Duration::from_secs(59)));
        assert!(a.tick(false, t0 + IDLE));
        assert_eq!(a.presence(), Presence::Retracting);

        // Still animating: the window must not shrink under the strip yet.
        assert!(!a.tick(false, t0 + IDLE + Duration::from_millis(100)));
        assert_eq!(a.presence(), Presence::Retracting);

        assert!(a.tick(false, t0 + IDLE + RETRACT_ANIMATION));
        assert_eq!(a.presence(), Presence::Retracted);
    }

    #[test]
    fn being_busy_keeps_it_out_and_restarts_the_clock() {
        let t0 = Instant::now();
        let mut a = AutoHide::new(true, IDLE, t0);
        a.tick(true, t0 + Duration::from_secs(50));
        // 50 s of busy then 59 s idle: not yet.
        assert!(!a.tick(false, t0 + Duration::from_secs(109)));
        assert_eq!(a.presence(), Presence::Shown);
        assert!(a.tick(false, t0 + Duration::from_secs(110)));
    }

    #[test]
    fn waking_brings_it_back_from_either_hidden_state() {
        let t0 = Instant::now();
        let mut a = AutoHide::new(true, IDLE, t0);
        a.tick(false, t0 + IDLE);
        assert!(a.wake(t0 + IDLE + Duration::from_millis(50)));
        assert_eq!(a.presence(), Presence::Shown);

        a.tick(false, t0 + IDLE * 3);
        a.tick(false, t0 + IDLE * 3 + RETRACT_ANIMATION);
        assert_eq!(a.presence(), Presence::Retracted);
        assert!(a.tick(true, t0 + IDLE * 4));
        assert_eq!(a.presence(), Presence::Shown);
    }

    #[test]
    fn turning_it_off_shows_the_notch_at_once() {
        let t0 = Instant::now();
        let mut a = AutoHide::new(true, IDLE, t0);
        a.tick(false, t0 + IDLE);
        a.tick(false, t0 + IDLE + RETRACT_ANIMATION);
        assert!(a.configure(false, IDLE, t0 + IDLE * 2));
        assert_eq!(a.presence(), Presence::Shown);
    }
}
