//! Types shared between the collectors, the Tauri commands and the webview.
//!
//! Everything here is serialised to the frontend as camelCase JSON. The guiding
//! rule, borrowed from the macOS original, is that a failed collection must
//! degrade to a *visible* status (`stale`, `needsAuth`, `error`) rather than an
//! invented number, so [`Health`] is never optional and never silently `Ok`.

use std::collections::BTreeMap;

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

/// A provider CodeNotch knows how to collect from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProviderId {
    ClaudeCode,
    Cursor,
    Copilot,
    Codex,
    Ollama,
}

impl ProviderId {
    pub const ALL: [ProviderId; 5] = [
        ProviderId::ClaudeCode,
        ProviderId::Cursor,
        ProviderId::Copilot,
        ProviderId::Codex,
        ProviderId::Ollama,
    ];

    pub fn display_name(self) -> &'static str {
        match self {
            ProviderId::ClaudeCode => "Claude Code",
            ProviderId::Cursor => "Cursor",
            ProviderId::Copilot => "GitHub Copilot",
            ProviderId::Codex => "Codex",
            ProviderId::Ollama => "Ollama",
        }
    }

    /// Stable key used in the config file and in the frontend's sort order.
    pub fn key(self) -> &'static str {
        match self {
            ProviderId::ClaudeCode => "claudeCode",
            ProviderId::Cursor => "cursor",
            ProviderId::Copilot => "copilot",
            ProviderId::Codex => "codex",
            ProviderId::Ollama => "ollama",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        ProviderId::ALL.into_iter().find(|p| p.key() == key)
    }

    /// Executables to hunt for when the user clicks a provider card.
    pub fn focus_processes(self) -> &'static [&'static str] {
        match self {
            // Claude Code and Codex are CLIs, so we look for the terminal hosting them.
            ProviderId::ClaudeCode | ProviderId::Codex => &[
                "WindowsTerminal.exe",
                "wezterm-gui.exe",
                "alacritty.exe",
                "powershell.exe",
                "pwsh.exe",
                "cmd.exe",
                "Code.exe",
            ],
            ProviderId::Cursor => &["Cursor.exe"],
            ProviderId::Copilot => &["Code.exe", "devenv.exe", "WindowsTerminal.exe"],
            ProviderId::Ollama => &["ollama app.exe", "ollama.exe"],
        }
    }
}

/// Whether the provider's numbers can be trusted right now.
///
/// Ordering matters: [`Health::worst_of`] keeps the most alarming state so the
/// collapsed pill can summarise every provider in a single ring colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Health {
    /// Provider isn't installed / not running. Not an error, just nothing to show.
    Unavailable,
    /// Fresh numbers straight from the source.
    Ok,
    /// We have numbers, but they're older than the provider's staleness budget.
    Stale,
    /// Upstream told us to slow down; we're backing off and showing the last value.
    RateLimited,
    /// We found the provider but no usable credentials.
    NeedsAuth,
    /// Something broke. `detail` carries the reason.
    Error,
}

impl Health {
    pub fn worst_of(a: Health, b: Health) -> Health {
        a.max(b)
    }

    /// True when the provider has nothing meaningful to contribute to the HUD.
    pub fn is_quiet(self) -> bool {
        matches!(self, Health::Unavailable)
    }
}

/// What the provider's agent is doing right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Activity {
    /// Nothing running.
    #[default]
    Idle,
    /// A session finished recently and the user probably hasn't looked yet.
    Done,
    /// A model is producing tokens. Drives the spinning cyan arc.
    Generating,
    /// A CLI is blocked on a `[y/N]` style prompt. Drives the pulsing amber ring.
    AwaitingInput,
}

impl Activity {
    /// Precedence for rolling many sessions up into one indicator: a single
    /// blocked session outranks any amount of happily generating ones, because
    /// that is the state that actually needs the user.
    pub fn rank(self) -> u8 {
        match self {
            Activity::Idle => 0,
            Activity::Done => 1,
            Activity::Generating => 2,
            Activity::AwaitingInput => 3,
        }
    }

    pub fn merge(self, other: Activity) -> Activity {
        if other.rank() > self.rank() {
            other
        } else {
            self
        }
    }
}

/// What a usage window is counting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UsageUnit {
    #[default]
    Percent,
    Tokens,
    Requests,
    Credits,
    Bytes,
}

/// One rolling limit window, e.g. Claude's 5-hour session window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageWindow {
    /// Stable identifier, e.g. `five_hour`.
    pub key: String,
    /// Short human label, e.g. `5h`.
    pub label: String,
    /// 0..=100. `None` when the provider exposes counts but no denominator.
    pub used_pct: Option<f32>,
    pub used: Option<f64>,
    pub limit: Option<f64>,
    pub unit: UsageUnit,
    pub resets_at: Option<DateTime<Utc>>,
    /// Set when this window is an estimate rather than a reported figure, so the
    /// UI can mark it with a `~`.
    #[serde(default)]
    pub estimated: bool,
    /// Set when the percentage is context rather than a quota being consumed.
    ///
    /// Ollama's GPU-residency share is the motivating case: 93% on the GPU is
    /// *good*, and colouring it like a limit about to be hit would be a lie.
    /// Informational windows are excluded from the provider's peak and never
    /// raise a threshold alert.
    #[serde(default)]
    pub informational: bool,
}

impl UsageWindow {
    pub fn new(key: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            used_pct: None,
            used: None,
            limit: None,
            unit: UsageUnit::Percent,
            resets_at: None,
            estimated: false,
            informational: false,
        }
    }

    pub fn with_pct(mut self, pct: f32) -> Self {
        self.used_pct = Some(pct.clamp(0.0, 100.0));
        self
    }

    pub fn with_counts(mut self, used: f64, limit: Option<f64>) -> Self {
        self.used = Some(used);
        self.limit = limit;
        if let Some(limit) = limit.filter(|l| *l > 0.0) {
            self.used_pct = Some(((used / limit) * 100.0).clamp(0.0, 100.0) as f32);
        }
        self
    }

    pub fn with_unit(mut self, unit: UsageUnit) -> Self {
        self.unit = unit;
        self
    }

    pub fn with_reset(mut self, at: Option<DateTime<Utc>>) -> Self {
        self.resets_at = at;
        self
    }

    pub fn estimated(mut self) -> Self {
        self.estimated = true;
        self
    }

    /// Mark this window as context, not a quota. See [`Self::informational`].
    pub fn informational(mut self) -> Self {
        self.informational = true;
        self
    }
}

/// A single agent session (a CLI run, a Cursor composer tab, a loaded model).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub id: String,
    /// Usually the project folder name, which is what the user recognises.
    pub title: String,
    pub cwd: Option<String>,
    pub model: Option<String>,
    pub activity: Activity,
    pub last_activity: Option<DateTime<Utc>>,
    /// Tokens consumed by this session, when the provider exposes them.
    pub tokens: Option<u64>,
    /// Free-form line shown under the title, e.g. `8.2 GB VRAM`.
    pub detail: Option<String>,
}

impl Session {
    pub fn new(id: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            cwd: None,
            model: None,
            activity: Activity::Idle,
            last_activity: None,
            tokens: None,
            detail: None,
        }
    }
}

/// Everything the HUD knows about one provider at one point in time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderSnapshot {
    pub id: ProviderId,
    pub name: String,
    pub health: Health,
    pub activity: Activity,
    /// Human explanation of a non-`Ok` health, shown in the expanded card.
    pub detail: Option<String>,
    /// Where the numbers came from, e.g. `oauth`, `transcripts`, `state.vscdb`.
    pub source: Option<String>,
    /// Plan / account label, e.g. `Max 20x`.
    pub account: Option<String>,
    pub windows: Vec<UsageWindow>,
    pub sessions: Vec<Session>,
    pub updated_at: DateTime<Utc>,
    /// When a rate limit is in force, when we will next try.
    pub retry_at: Option<DateTime<Utc>>,
}

impl ProviderSnapshot {
    pub fn new(id: ProviderId) -> Self {
        Self {
            id,
            name: id.display_name().to_string(),
            health: Health::Ok,
            activity: Activity::Idle,
            detail: None,
            source: None,
            account: None,
            windows: Vec::new(),
            sessions: Vec::new(),
            updated_at: Utc::now(),
            retry_at: None,
        }
    }

    /// A provider that produced no numbers, with the reason attached.
    pub fn degraded(id: ProviderId, health: Health, detail: impl Into<String>) -> Self {
        Self {
            health,
            detail: Some(detail.into()),
            ..Self::new(id)
        }
    }

    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = Some(source.into());
        self
    }

    pub fn with_account(mut self, account: Option<String>) -> Self {
        self.account = account;
        self
    }

    pub fn with_windows(mut self, windows: Vec<UsageWindow>) -> Self {
        self.windows = windows;
        self
    }

    pub fn with_sessions(mut self, sessions: Vec<Session>) -> Self {
        self.activity = sessions
            .iter()
            .fold(self.activity, |acc, s| acc.merge(s.activity));
        self.sessions = sessions;
        self
    }

    /// Highest utilisation across this provider's *quota* windows.
    pub fn peak_pct(&self) -> Option<f32> {
        self.windows
            .iter()
            .filter(|w| !w.informational)
            .filter_map(|w| w.used_pct)
            .fold(None, |acc: Option<f32>, pct| {
                Some(acc.map_or(pct, |a| a.max(pct)))
            })
    }

    /// Mark a previously-good snapshot as stale rather than dropping it, so the
    /// HUD keeps showing the last known numbers with a visible caveat.
    pub fn mark_stale(&mut self, reason: impl Into<String>) {
        if self.health == Health::Ok {
            self.health = Health::Stale;
        }
        self.detail = Some(reason.into());
        // Live activity can't be trusted once collection stops.
        self.activity = Activity::Idle;
        for session in &mut self.sessions {
            session.activity = Activity::Idle;
        }
    }
}

/// The full HUD payload pushed to the webview on every poll.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Telemetry {
    pub providers: Vec<ProviderSnapshot>,
    pub generated_at: DateTime<Utc>,
    /// Worst utilisation across every provider, for the collapsed pill.
    pub peak_pct: Option<f32>,
    /// Rolled-up activity across every provider.
    pub activity: Activity,
    /// Worst health across the providers that have anything to say.
    pub health: Health,
}

impl Telemetry {
    pub fn from_snapshots(mut providers: Vec<ProviderSnapshot>) -> Self {
        providers.sort_by_key(|p| p.id);

        let peak_pct = providers
            .iter()
            .filter(|p| p.health != Health::Unavailable)
            .filter_map(|p| p.peak_pct())
            .fold(None, |acc: Option<f32>, pct| {
                Some(acc.map_or(pct, |a| a.max(pct)))
            });

        let activity = providers
            .iter()
            .fold(Activity::Idle, |acc, p| acc.merge(p.activity));

        let health = providers
            .iter()
            .filter(|p| !p.health.is_quiet())
            .fold(Health::Ok, |acc, p| Health::worst_of(acc, p.health));

        Self {
            providers,
            generated_at: Utc::now(),
            peak_pct,
            activity,
            health,
        }
    }

    pub fn empty() -> Self {
        Self {
            providers: Vec::new(),
            generated_at: Utc::now(),
            peak_pct: None,
            activity: Activity::Idle,
            health: Health::Ok,
        }
    }

    pub fn get(&self, id: ProviderId) -> Option<&ProviderSnapshot> {
        self.providers.iter().find(|p| p.id == id)
    }
}

/// Thresholds we alert on, matching the macOS app's 80% / 100% notifications.
pub const ALERT_THRESHOLDS: [f32; 2] = [80.0, 100.0];

/// Tracks which thresholds we've already fired for, so crossing 80% notifies
/// once rather than on every poll. Reset when a window's `resets_at` passes.
#[derive(Debug, Default)]
pub struct AlertLedger {
    fired: BTreeMap<(ProviderId, String), f32>,
}

impl AlertLedger {
    /// Returns the threshold to alert on, if this reading just crossed one.
    pub fn observe(&mut self, provider: ProviderId, window: &UsageWindow) -> Option<f32> {
        if window.informational {
            return None;
        }
        let pct = window.used_pct?;
        let key = (provider, window.key.clone());
        let previous = self.fired.get(&key).copied().unwrap_or(0.0);

        // A drop of more than a few points means the window rolled over.
        if pct + 5.0 < previous {
            self.fired.remove(&key);
            return None;
        }

        // Highest threshold crossed by this reading wins, so a jump straight
        // past 80 to 100 reports 100 rather than nagging twice.
        let crossed = ALERT_THRESHOLDS
            .iter()
            .copied()
            .rfind(|t| pct >= *t && previous < *t)?;

        self.fired.insert(key, pct.max(crossed));
        Some(crossed)
    }
}

/// How long a snapshot may go without a refresh before it is called stale.
pub fn staleness_budget(id: ProviderId) -> Duration {
    match id {
        // The OAuth usage endpoint is polled slowly and is rate limited.
        ProviderId::ClaudeCode => Duration::minutes(15),
        ProviderId::Cursor | ProviderId::Copilot | ProviderId::Codex => Duration::minutes(10),
        // Local, cheap, and interesting only while it is live.
        ProviderId::Ollama => Duration::minutes(2),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activity_precedence_puts_blocked_sessions_first() {
        assert_eq!(
            Activity::Generating.merge(Activity::AwaitingInput),
            Activity::AwaitingInput
        );
        assert_eq!(
            Activity::AwaitingInput.merge(Activity::Generating),
            Activity::AwaitingInput
        );
        assert_eq!(Activity::Idle.merge(Activity::Done), Activity::Done);
        assert_eq!(Activity::Done.merge(Activity::Idle), Activity::Done);
    }

    #[test]
    fn health_worst_of_prefers_the_alarming_state() {
        assert_eq!(
            Health::worst_of(Health::Ok, Health::NeedsAuth),
            Health::NeedsAuth
        );
        assert_eq!(
            Health::worst_of(Health::Error, Health::Stale),
            Health::Error
        );
        assert_eq!(
            Health::worst_of(Health::Ok, Health::Unavailable),
            Health::Ok
        );
    }

    #[test]
    fn usage_window_derives_percentage_from_counts() {
        let w = UsageWindow::new("five_hour", "5h").with_counts(25.0, Some(100.0));
        assert_eq!(w.used_pct, Some(25.0));

        // No denominator means no invented percentage.
        let w = UsageWindow::new("tokens", "Tokens").with_counts(1234.0, None);
        assert_eq!(w.used_pct, None);

        // Over-limit readings clamp instead of overflowing the ring.
        let w = UsageWindow::new("x", "x").with_counts(150.0, Some(100.0));
        assert_eq!(w.used_pct, Some(100.0));
    }

    #[test]
    fn telemetry_rolls_up_peak_activity_and_health() {
        let a = ProviderSnapshot::new(ProviderId::ClaudeCode).with_windows(vec![UsageWindow::new(
            "five_hour",
            "5h",
        )
        .with_pct(42.0)]);
        let mut b = ProviderSnapshot::new(ProviderId::Cursor)
            .with_windows(vec![UsageWindow::new("month", "Mo").with_pct(88.0)]);
        b.activity = Activity::Generating;
        let c = ProviderSnapshot::degraded(ProviderId::Codex, Health::NeedsAuth, "no token");

        let t = Telemetry::from_snapshots(vec![b, c, a]);
        assert_eq!(t.peak_pct, Some(88.0));
        assert_eq!(t.activity, Activity::Generating);
        assert_eq!(t.health, Health::NeedsAuth);
        // Sorted into a stable display order regardless of collection order.
        assert_eq!(t.providers[0].id, ProviderId::ClaudeCode);
    }

    #[test]
    fn informational_windows_are_context_not_quota() {
        let snap = ProviderSnapshot::new(ProviderId::Ollama).with_windows(vec![
            UsageWindow::new("vram", "On GPU")
                .with_pct(93.0)
                .informational(),
            UsageWindow::new("quota", "Quota").with_pct(10.0),
        ]);
        assert_eq!(
            snap.peak_pct(),
            Some(10.0),
            "a 93%-on-GPU reading must not present as a nearly-exhausted limit"
        );

        let mut ledger = AlertLedger::default();
        let gpu = UsageWindow::new("vram", "On GPU")
            .with_pct(100.0)
            .informational();
        assert_eq!(ledger.observe(ProviderId::Ollama, &gpu), None);
    }

    #[test]
    fn unavailable_providers_do_not_drag_down_overall_health() {
        let ok = ProviderSnapshot::new(ProviderId::ClaudeCode);
        let missing =
            ProviderSnapshot::degraded(ProviderId::Ollama, Health::Unavailable, "not running");
        let t = Telemetry::from_snapshots(vec![ok, missing]);
        assert_eq!(t.health, Health::Ok);
    }

    #[test]
    fn alert_ledger_fires_once_per_threshold_and_resets_on_rollover() {
        let mut ledger = AlertLedger::default();
        let win = |pct: f32| UsageWindow::new("five_hour", "5h").with_pct(pct);

        assert_eq!(ledger.observe(ProviderId::ClaudeCode, &win(50.0)), None);
        assert_eq!(
            ledger.observe(ProviderId::ClaudeCode, &win(81.0)),
            Some(80.0)
        );
        // Still above 80 but already alerted.
        assert_eq!(ledger.observe(ProviderId::ClaudeCode, &win(85.0)), None);
        assert_eq!(
            ledger.observe(ProviderId::ClaudeCode, &win(100.0)),
            Some(100.0)
        );
        // Window rolls over -> ledger clears and can alert again.
        assert_eq!(ledger.observe(ProviderId::ClaudeCode, &win(3.0)), None);
        assert_eq!(
            ledger.observe(ProviderId::ClaudeCode, &win(80.0)),
            Some(80.0)
        );
    }

    #[test]
    fn jumping_straight_past_both_thresholds_reports_the_higher_one() {
        let mut ledger = AlertLedger::default();
        let win = UsageWindow::new("week", "7d").with_pct(100.0);
        assert_eq!(ledger.observe(ProviderId::Cursor, &win), Some(100.0));
    }

    #[test]
    fn marking_stale_keeps_numbers_but_clears_live_activity() {
        let mut snap = ProviderSnapshot::new(ProviderId::ClaudeCode)
            .with_windows(vec![UsageWindow::new("five_hour", "5h").with_pct(60.0)])
            .with_sessions(vec![Session {
                activity: Activity::Generating,
                ..Session::new("s1", "api")
            }]);
        assert_eq!(snap.activity, Activity::Generating);

        snap.mark_stale("collector timed out");
        assert_eq!(snap.health, Health::Stale);
        assert_eq!(snap.activity, Activity::Idle);
        assert_eq!(snap.sessions[0].activity, Activity::Idle);
        assert_eq!(snap.peak_pct(), Some(60.0));
    }
}
