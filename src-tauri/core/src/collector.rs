//! Polls the adapters on their own schedules and rolls the results into one
//! [`Telemetry`] payload for the HUD.
//!
//! Each provider has its own cadence (see [`PollConfig`](crate::config::PollConfig)),
//! because hitting a rate-limited HTTP endpoint as often as a local file read
//! would be a good way to get throttled. Between polls the last snapshot is
//! reused, and once it exceeds the provider's staleness budget it is marked
//! [`Health::Stale`] rather than quietly presented as current.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};

use crate::adapters::{
    claude::ClaudeAdapter, codex::CodexAdapter, copilot::CopilotAdapter, cursor::CursorAdapter,
    gemini::GeminiAdapter, ollama::OllamaAdapter, perplexity::PerplexityAdapter,
};
use crate::config::Config;
use crate::model::{
    staleness_budget, AlertLedger, Health, ProviderId, ProviderSnapshot, Telemetry,
};

/// A usage threshold crossing worth notifying about.
#[derive(Debug, Clone, PartialEq)]
pub struct Alert {
    pub provider: ProviderId,
    pub provider_name: String,
    pub window_label: String,
    /// The threshold crossed (80 or 100).
    pub threshold: f32,
    pub used_pct: f32,
}

impl Alert {
    pub fn title(&self) -> String {
        if self.threshold >= 100.0 {
            format!("{} limit reached", self.provider_name)
        } else {
            format!("{} at {:.0}%", self.provider_name, self.used_pct)
        }
    }

    pub fn body(&self) -> String {
        format!("{} window · {:.0}% used", self.window_label, self.used_pct)
    }
}

/// When each provider was last collected, and what it said.
#[derive(Debug, Clone)]
struct CacheEntry {
    collected_at: DateTime<Utc>,
    snapshot: ProviderSnapshot,
}

/// Owns the adapters and their schedules.
pub struct Collector {
    config: Config,
    claude: ClaudeAdapter,
    cursor: CursorAdapter,
    copilot: CopilotAdapter,
    codex: CodexAdapter,
    gemini: GeminiAdapter,
    perplexity: PerplexityAdapter,
    ollama: OllamaAdapter,
    cache: BTreeMap<ProviderId, CacheEntry>,
    alerts: AlertLedger,
    /// Forces every provider to refresh on the next poll.
    refresh_all: bool,
}

impl Collector {
    pub fn new(config: Config) -> Self {
        let http = reqwest::Client::builder()
            .user_agent(concat!("CodeNotch/", env!("CARGO_PKG_VERSION")))
            .build()
            .unwrap_or_default();

        Self {
            claude: ClaudeAdapter::new(http.clone()),
            cursor: CursorAdapter::new(),
            copilot: CopilotAdapter::new(),
            codex: CodexAdapter::new(),
            gemini: GeminiAdapter::new(),
            perplexity: PerplexityAdapter::new(),
            ollama: OllamaAdapter::new(http, config.ollama_url.clone()),
            cache: BTreeMap::new(),
            alerts: AlertLedger::default(),
            refresh_all: true,
            config,
        }
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Swap in new settings; the next poll refreshes everything so the change
    /// is visible immediately rather than at the end of the slowest interval.
    pub fn set_config(&mut self, config: Config) {
        self.ollama.set_base_url(config.ollama_url.clone());
        self.config = config;
        self.refresh_all = true;

        // Drop anything the user just turned off so it can't linger on screen.
        self.cache.retain(|id, _| self.config.is_enabled(*id));
    }

    /// Force a full refresh on the next poll.
    pub fn invalidate(&mut self) {
        self.refresh_all = true;
    }

    /// Whether `id` is due for collection.
    fn is_due(&self, id: ProviderId, now: DateTime<Utc>) -> bool {
        if self.refresh_all {
            return true;
        }
        match self.cache.get(&id) {
            None => true,
            Some(entry) => {
                let interval = chrono::Duration::seconds(self.config.poll.for_provider(id) as i64);
                now.signed_duration_since(entry.collected_at) >= interval
            }
        }
    }

    /// Collect one provider.
    async fn collect_one(&mut self, id: ProviderId, now: DateTime<Utc>) -> ProviderSnapshot {
        match id {
            ProviderId::ClaudeCode => self.claude.collect(now).await,
            ProviderId::Cursor => self.cursor.collect(now),
            ProviderId::Copilot => self.copilot.collect(now),
            ProviderId::Codex => self.codex.collect(now),
            ProviderId::Gemini => self.gemini.collect(now),
            ProviderId::Perplexity => self.perplexity.collect(now),
            ProviderId::Ollama => self.ollama.collect(now).await,
        }
    }

    /// Refresh whatever is due and return the current picture plus any
    /// threshold crossings that just happened.
    pub async fn poll(&mut self, now: DateTime<Utc>) -> (Telemetry, Vec<Alert>) {
        let mut alerts = Vec::new();

        for id in self.config.ordered_providers() {
            if !self.config.is_enabled(id) {
                self.cache.remove(&id);
                continue;
            }
            if !self.is_due(id, now) {
                continue;
            }

            let snapshot = self.collect_one(id, now).await;

            // Only alert on numbers we actually trust.
            if self.config.notify_on_thresholds && snapshot.health == Health::Ok {
                for window in &snapshot.windows {
                    if window.estimated {
                        continue;
                    }
                    if let Some(threshold) = self.alerts.observe(id, window) {
                        alerts.push(Alert {
                            provider: id,
                            provider_name: snapshot.name.clone(),
                            window_label: window.label.clone(),
                            threshold,
                            used_pct: window.used_pct.unwrap_or(threshold),
                        });
                    }
                }
            }

            self.cache.insert(
                id,
                CacheEntry {
                    collected_at: now,
                    snapshot,
                },
            );
        }

        self.refresh_all = false;
        (self.telemetry(now), alerts)
    }

    /// Build the payload from cache, ageing snapshots past their budget.
    pub fn telemetry(&self, now: DateTime<Utc>) -> Telemetry {
        let snapshots = self
            .config
            .ordered_providers()
            .into_iter()
            .filter(|id| self.config.is_enabled(*id))
            .filter_map(|id| {
                let entry = self.cache.get(&id)?;
                let mut snapshot = entry.snapshot.clone();
                let age = now.signed_duration_since(entry.collected_at);

                if age > staleness_budget(id) && snapshot.health != Health::Unavailable {
                    snapshot.mark_stale(format!(
                        "No update for {} minutes",
                        age.num_minutes().max(1)
                    ));
                }
                Some(snapshot)
            })
            .collect();

        Telemetry::from_snapshots(snapshots)
    }

    /// Shortest interval across the enabled providers, which is how often the
    /// caller needs to tick.
    pub fn tick_interval_secs(&self) -> u64 {
        self.config
            .ordered_providers()
            .into_iter()
            .filter(|id| self.config.is_enabled(*id))
            .map(|id| self.config.poll.for_provider(id))
            .min()
            .unwrap_or(60)
            // Never spin faster than the activity pass needs.
            .max(self.config.poll.activity_secs.max(1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::PollConfig;

    fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    /// A config with nothing installed, so every adapter reports Unavailable
    /// quickly and deterministically.
    fn test_config() -> Config {
        Config {
            // Point Ollama at a closed port so it fails fast.
            ollama_url: "http://127.0.0.1:1".into(),
            notify_on_thresholds: true,
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn poll_produces_a_snapshot_for_every_enabled_provider() {
        let mut collector = Collector::new(test_config());
        let (telemetry, alerts) = collector.poll(at("2026-01-01T12:00:00Z")).await;

        assert_eq!(telemetry.providers.len(), ProviderId::ALL.len());
        assert!(
            alerts.is_empty(),
            "nothing is installed, so nothing to alert"
        );
        // Providers sort into a stable display order.
        assert_eq!(telemetry.providers[0].id, ProviderId::ClaudeCode);
    }

    #[tokio::test]
    async fn disabled_providers_are_dropped_from_the_payload() {
        let mut config = test_config();
        config.providers.insert("ollama".into(), false);
        config.providers.insert("cursor".into(), false);

        let mut collector = Collector::new(config);
        let (telemetry, _) = collector.poll(at("2026-01-01T12:00:00Z")).await;

        let ids: Vec<_> = telemetry.providers.iter().map(|p| p.id).collect();
        assert!(!ids.contains(&ProviderId::Ollama));
        assert!(!ids.contains(&ProviderId::Cursor));
        assert_eq!(ids.len(), ProviderId::ALL.len() - 2);
    }

    #[tokio::test]
    async fn turning_a_provider_off_removes_it_immediately() {
        let mut collector = Collector::new(test_config());
        let now = at("2026-01-01T12:00:00Z");
        collector.poll(now).await;
        assert!(collector.telemetry(now).get(ProviderId::Ollama).is_some());

        let mut config = test_config();
        config.providers.insert("ollama".into(), false);
        collector.set_config(config);

        assert!(
            collector.telemetry(now).get(ProviderId::Ollama).is_none(),
            "a disabled provider must not survive in the cache"
        );
    }

    #[tokio::test]
    async fn providers_are_only_re_collected_when_due() {
        let mut collector = Collector::new(Config {
            poll: PollConfig {
                ollama_secs: 10,
                ..PollConfig::default()
            },
            ..test_config()
        });

        let start = at("2026-01-01T12:00:00Z");
        collector.poll(start).await;

        // Well inside every interval: nothing is due.
        let soon = start + chrono::Duration::seconds(5);
        assert!(!collector.is_due(ProviderId::Ollama, soon));
        assert!(!collector.is_due(ProviderId::ClaudeCode, soon));

        // Past Ollama's 10s interval but not Claude's 90s one.
        let later = start + chrono::Duration::seconds(11);
        assert!(collector.is_due(ProviderId::Ollama, later));
        assert!(!collector.is_due(ProviderId::ClaudeCode, later));
    }

    #[tokio::test]
    async fn a_config_change_forces_a_full_refresh() {
        let mut collector = Collector::new(test_config());
        let start = at("2026-01-01T12:00:00Z");
        collector.poll(start).await;

        let soon = start + chrono::Duration::seconds(1);
        assert!(!collector.is_due(ProviderId::ClaudeCode, soon));

        collector.set_config(test_config());
        assert!(
            collector.is_due(ProviderId::ClaudeCode, soon),
            "new settings should be reflected right away"
        );
    }

    #[tokio::test]
    async fn stale_snapshots_are_marked_rather_than_silently_shown() {
        let mut collector = Collector::new(test_config());
        let start = at("2026-01-01T12:00:00Z");

        // Seed a healthy Claude snapshot directly; the real adapter needs a
        // Claude install to produce one.
        collector.poll(start).await;
        collector.cache.insert(
            ProviderId::ClaudeCode,
            CacheEntry {
                collected_at: start,
                snapshot: ProviderSnapshot::new(ProviderId::ClaudeCode).with_windows(vec![
                    crate::model::UsageWindow::new("five_hour", "5h").with_pct(40.0),
                ]),
            },
        );

        // Inside the budget: still OK.
        let fresh = collector
            .telemetry(start + chrono::Duration::minutes(5))
            .get(ProviderId::ClaudeCode)
            .unwrap()
            .clone();
        assert_eq!(fresh.health, Health::Ok);

        // Past it: marked stale, but the last numbers are kept.
        let stale = collector
            .telemetry(start + chrono::Duration::minutes(30))
            .get(ProviderId::ClaudeCode)
            .unwrap()
            .clone();
        assert_eq!(stale.health, Health::Stale);
        assert_eq!(stale.peak_pct(), Some(40.0));
        assert!(stale.detail.unwrap().contains("30 minutes"));
    }

    #[tokio::test]
    async fn unavailable_providers_are_not_aged_into_staleness() {
        let mut collector = Collector::new(test_config());
        let start = at("2026-01-01T12:00:00Z");
        collector.poll(start).await;

        let much_later = start + chrono::Duration::hours(4);
        let ollama = collector
            .telemetry(much_later)
            .get(ProviderId::Ollama)
            .unwrap()
            .clone();
        assert_eq!(
            ollama.health,
            Health::Unavailable,
            "a tool that isn't running can't go stale"
        );
    }

    #[test]
    fn tick_interval_follows_the_fastest_enabled_provider() {
        let collector = Collector::new(Config {
            poll: PollConfig {
                ollama_secs: 10,
                activity_secs: 3,
                ..PollConfig::default()
            },
            ..test_config()
        });
        assert_eq!(collector.tick_interval_secs(), 10);

        // With Ollama off, the next-fastest provider sets the pace.
        let mut config = test_config();
        config.providers.insert("ollama".into(), false);
        config.poll = PollConfig {
            ollama_secs: 10,
            activity_secs: 3,
            ..PollConfig::default()
        };
        let collector = Collector::new(config);
        assert_eq!(collector.tick_interval_secs(), 45);
    }

    #[tokio::test]
    async fn threshold_alerts_fire_once_per_crossing() {
        let mut collector = Collector::new(test_config());
        let window = |pct: f32| crate::model::UsageWindow::new("five_hour", "5h").with_pct(pct);

        // Drive the ledger directly: it is the piece that decides.
        let snap = ProviderSnapshot::new(ProviderId::ClaudeCode);
        assert!(collector
            .alerts
            .observe(ProviderId::ClaudeCode, &window(50.0))
            .is_none());
        assert_eq!(
            collector
                .alerts
                .observe(ProviderId::ClaudeCode, &window(85.0)),
            Some(80.0)
        );
        assert!(collector
            .alerts
            .observe(ProviderId::ClaudeCode, &window(86.0))
            .is_none());
        assert_eq!(snap.health, Health::Ok);
    }

    #[test]
    fn alert_copy_reads_sensibly() {
        let alert = Alert {
            provider: ProviderId::ClaudeCode,
            provider_name: "Claude Code".into(),
            window_label: "5h session".into(),
            threshold: 80.0,
            used_pct: 83.0,
        };
        assert_eq!(alert.title(), "Claude Code at 83%");
        assert_eq!(alert.body(), "5h session window · 83% used");

        let maxed = Alert {
            threshold: 100.0,
            used_pct: 100.0,
            ..alert
        };
        assert_eq!(maxed.title(), "Claude Code limit reached");
    }
}
