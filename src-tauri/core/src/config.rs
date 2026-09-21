//! On-disk settings, stored at `%APPDATA%\CodeNotch\config.json`.
//!
//! Every field has a serde default so a hand-edited or older config file still
//! loads; unknown keys are ignored rather than rejected.

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::model::ProviderId;

/// Which screen edge the notch is pinned to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Edge {
    Top,
    Bottom,
    Left,
    #[default]
    Right,
}

impl Edge {
    pub fn is_horizontal(self) -> bool {
        matches!(self, Edge::Top | Edge::Bottom)
    }
}

/// Overall HUD scale.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HudSize {
    Small,
    #[default]
    Medium,
    Large,
}

/// Logical (DPI-independent) dimensions of the notch.
///
/// The notch is a strip docked against one screen edge holding one ring per
/// provider, plus a detail popover that appears alongside it on hover. "Along"
/// is down the strip on a vertical edge, across it on a horizontal one;
/// "thickness" is the other axis.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HudMetrics {
    /// How far the strip reaches into the screen.
    pub strip_thickness: f64,
    /// Space one provider occupies along the strip: ring, percentage, gap.
    ///
    /// Measured off the reference art at 1.47x the strip's thickness.
    pub slot: f64,
    /// Padding at each end of the strip, before the first slot.
    ///
    /// Measured at 0.53x the thickness. This is *not* the taper length: the
    /// silhouette's curve runs 1.21x the thickness and so reaches into the
    /// first slot, exactly as the reference does — a ring is inset far enough
    /// from the strip's sides that the last percent of the taper never clips
    /// it.
    pub strip_padding: f64,
    /// Diameter of a provider's ring.
    pub ring: f64,
    /// Size of the detail popover on the axis it extends along.
    pub popover_size: f64,
    /// Gap between the popover and the strip.
    ///
    /// 0.54x the thickness, because the card's tail reaches 0.4x across it and
    /// the reference leaves the rest as clear air.
    pub popover_gap: f64,
}

impl HudMetrics {
    /// Space the settings gear occupies at the tail of the strip.
    ///
    /// The gear is a sibling of the rings inside the strip, so it lengthens it
    /// just as a ring does. Leaving it out of the arithmetic made the window a
    /// gear shorter than its own contents, which clipped the gear off the end
    /// until the webview's measurement arrived -- and off the screen entirely
    /// once the clamp in `layout` kicked in.
    pub fn settings_extent(&self) -> f64 {
        self.strip_thickness * 0.55
    }

    /// Resting size of the strip holding `providers` rings, as
    /// (along the edge, into the screen).
    pub fn strip_extent(&self, providers: usize) -> (f64, f64) {
        // Never fewer than one slot: before the first poll there are no
        // providers, and a zero-height strip would simply vanish.
        let along =
            self.strip_padding * 2.0 + providers.max(1) as f64 * self.slot + self.settings_extent();
        (along, self.strip_thickness)
    }
}

impl HudSize {
    pub fn metrics(self) -> HudMetrics {
        match self {
            HudSize::Small => HudMetrics {
                strip_thickness: 32.0,
                slot: 47.0,
                strip_padding: 17.0,
                ring: 20.0,
                popover_size: 208.0,
                popover_gap: 17.0,
            },
            HudSize::Medium => HudMetrics {
                strip_thickness: 40.0,
                slot: 59.0,
                strip_padding: 21.0,
                ring: 25.0,
                popover_size: 232.0,
                popover_gap: 22.0,
            },
            HudSize::Large => HudMetrics {
                strip_thickness: 50.0,
                slot: 74.0,
                strip_padding: 27.0,
                ring: 32.0,
                popover_size: 264.0,
                popover_gap: 27.0,
            },
        }
    }
}

/// Which monitor to pin to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind", content = "index")]
pub enum MonitorChoice {
    #[default]
    Primary,
    /// Zero-based index into the enumerated monitor list.
    Index(usize),
}

/// Poll cadence per provider, in seconds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PollConfig {
    /// Claude's usage endpoint is rate limited, so we are deliberately gentle.
    pub claude_secs: u64,
    pub cursor_secs: u64,
    pub copilot_secs: u64,
    pub codex_secs: u64,
    pub gemini_secs: u64,
    pub perplexity_secs: u64,
    pub ollama_secs: u64,
    /// Cheap local file-watch pass that drives the activity ring between polls.
    pub activity_secs: u64,
}

impl Default for PollConfig {
    fn default() -> Self {
        Self {
            claude_secs: 90,
            cursor_secs: 45,
            copilot_secs: 120,
            codex_secs: 45,
            gemini_secs: 60,
            perplexity_secs: 120,
            ollama_secs: 10,
            activity_secs: 3,
        }
    }
}

impl PollConfig {
    pub fn for_provider(&self, id: ProviderId) -> u64 {
        let secs = match id {
            ProviderId::ClaudeCode => self.claude_secs,
            ProviderId::Cursor => self.cursor_secs,
            ProviderId::Copilot => self.copilot_secs,
            ProviderId::Codex => self.codex_secs,
            ProviderId::Gemini => self.gemini_secs,
            ProviderId::Perplexity => self.perplexity_secs,
            ProviderId::Ollama => self.ollama_secs,
        };
        // Guard against a hand-edited config pinning a CPU core or hammering the API.
        secs.clamp(5, 3600)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Config {
    pub edge: Edge,
    /// Position along the edge, 0.0 = left/top .. 1.0 = right/bottom.
    pub edge_offset: f32,
    /// Gap between the HUD and the screen's work area, in logical pixels.
    pub margin: f64,
    pub size: HudSize,
    /// Ring accent as `#rrggbb`.
    pub accent: String,
    pub monitor: MonitorChoice,

    /// Stay expanded instead of collapsing when the pointer leaves.
    pub always_expanded: bool,
    /// Hide the HUD entirely (tray icon stays).
    pub hidden: bool,
    /// Let clicks fall through to the window underneath while collapsed.
    pub click_through_when_collapsed: bool,
    /// Briefly expand when an agent finishes or starts waiting for input.
    pub peek_on_attention: bool,
    /// Seconds a peek stays open.
    pub peek_secs: u64,
    /// Desktop notification when a window crosses 80% / 100%.
    pub notify_on_thresholds: bool,
    /// Show reset windows as a countdown rather than a clock time.
    pub reset_as_countdown: bool,
    /// Start CodeNotch when the user signs in (HKCU ...\Run).
    pub launch_at_login: bool,

    pub poll: PollConfig,
    /// Per-provider on/off, keyed by [`ProviderId::key`].
    pub providers: BTreeMap<String, bool>,
    /// Display order, keyed by [`ProviderId::key`]. Unlisted providers sort last.
    pub provider_order: Vec<String>,

    /// Where to reach Ollama. Kept configurable for remote/WSL daemons.
    pub ollama_url: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            edge: Edge::Right,
            edge_offset: 0.5,
            margin: 0.0,
            size: HudSize::Medium,
            accent: "#22d3ee".to_string(),
            monitor: MonitorChoice::Primary,

            always_expanded: false,
            hidden: false,
            click_through_when_collapsed: true,
            peek_on_attention: true,
            peek_secs: 5,
            notify_on_thresholds: true,
            reset_as_countdown: true,
            launch_at_login: false,

            poll: PollConfig::default(),
            providers: ProviderId::ALL
                .iter()
                .map(|p| (p.key().to_string(), true))
                .collect(),
            provider_order: ProviderId::ALL
                .iter()
                .map(|p| p.key().to_string())
                .collect(),

            ollama_url: "http://127.0.0.1:11434".to_string(),
        }
    }
}

impl Config {
    pub fn is_enabled(&self, id: ProviderId) -> bool {
        // Default to on: a provider added in a later version shouldn't be
        // invisible just because an old config file predates it.
        self.providers.get(id.key()).copied().unwrap_or(true)
    }

    pub fn metrics(&self) -> HudMetrics {
        self.size.metrics()
    }

    /// Normalised accent, falling back to the default when the string is junk.
    pub fn accent_hex(&self) -> String {
        let s = self.accent.trim();
        let valid =
            s.len() == 7 && s.starts_with('#') && s[1..].chars().all(|c| c.is_ascii_hexdigit());
        if valid {
            s.to_lowercase()
        } else {
            "#22d3ee".to_string()
        }
    }

    pub fn path() -> Result<PathBuf> {
        let dir = dirs::config_dir()
            .context("could not resolve %APPDATA% (config dir)")?
            .join("CodeNotch");
        Ok(dir.join("config.json"))
    }

    /// Load from disk, falling back to defaults when the file is missing.
    ///
    /// A corrupt file is *not* fatal: we log and carry on with defaults so the
    /// HUD still starts, and the next save rewrites it.
    pub fn load() -> Self {
        match Self::try_load() {
            Ok(Some(cfg)) => cfg,
            Ok(None) => Self::default(),
            Err(err) => {
                tracing::warn!(%err, "config unreadable, falling back to defaults");
                Self::default()
            }
        }
    }

    fn try_load() -> Result<Option<Self>> {
        let path = Self::path()?;
        if !path.exists() {
            return Ok(None);
        }
        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let cfg: Config =
            serde_json::from_str(&raw).with_context(|| format!("parsing {}", path.display()))?;
        Ok(Some(cfg.sanitised()))
    }

    /// Clamp anything that could push the HUD off-screen or spin the poller.
    fn sanitised(mut self) -> Self {
        self.edge_offset = self.edge_offset.clamp(0.0, 1.0);
        self.margin = self.margin.clamp(0.0, 400.0);
        self.peek_secs = self.peek_secs.clamp(1, 60);
        self.accent = self.accent_hex();
        if self.ollama_url.trim().is_empty() {
            self.ollama_url = Config::default().ollama_url;
        }
        self
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let json = serde_json::to_string_pretty(self)?;

        // Write-then-rename so a crash mid-write can't leave a truncated config.
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, json).with_context(|| format!("writing {}", tmp.display()))?;
        std::fs::rename(&tmp, &path).with_context(|| format!("replacing {}", path.display()))?;
        Ok(())
    }

    /// Provider display order: configured keys first, then anything new.
    pub fn ordered_providers(&self) -> Vec<ProviderId> {
        let mut out: Vec<ProviderId> = self
            .provider_order
            .iter()
            .filter_map(|k| ProviderId::from_key(k))
            .collect();
        for id in ProviderId::ALL {
            if !out.contains(&id) {
                out.push(id);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_config_files_fill_in_defaults() {
        let cfg: Config = serde_json::from_str(r#"{"edge":"bottom","size":"large"}"#).unwrap();
        assert_eq!(cfg.edge, Edge::Bottom);
        assert_eq!(cfg.size, HudSize::Large);
        // Untouched fields keep their defaults.
        assert_eq!(cfg.accent, "#22d3ee");
        assert_eq!(cfg.poll.claude_secs, 90);
        assert!(cfg.peek_on_attention);
    }

    #[test]
    fn unknown_keys_are_ignored_rather_than_fatal() {
        let cfg: Config =
            serde_json::from_str(r#"{"edge":"left","somethingFromTheFuture":123}"#).unwrap();
        assert_eq!(cfg.edge, Edge::Left);
    }

    #[test]
    fn roundtrips_through_json() {
        let mut cfg = Config {
            edge: Edge::Right,
            always_expanded: true,
            ..Default::default()
        };
        cfg.providers.insert("cursor".into(), false);
        let back: Config = serde_json::from_str(&serde_json::to_string(&cfg).unwrap()).unwrap();
        assert_eq!(cfg, back);
        assert!(!back.is_enabled(ProviderId::Cursor));
        assert!(back.is_enabled(ProviderId::ClaudeCode));
    }

    #[test]
    fn accent_falls_back_when_malformed() {
        let mut cfg = Config {
            accent: "not-a-colour".into(),
            ..Default::default()
        };
        assert_eq!(cfg.accent_hex(), "#22d3ee");
        cfg.accent = "#FF00AA".into();
        assert_eq!(cfg.accent_hex(), "#ff00aa");
        cfg.accent = "#f0a".into(); // shorthand isn't accepted
        assert_eq!(cfg.accent_hex(), "#22d3ee");
    }

    #[test]
    fn sanitise_clamps_out_of_range_values() {
        let cfg: Config =
            serde_json::from_str(r#"{"edgeOffset": 4.5, "margin": -20, "peekSecs": 9999}"#)
                .unwrap();
        let cfg = cfg.sanitised();
        assert_eq!(cfg.edge_offset, 1.0);
        assert_eq!(cfg.margin, 0.0);
        assert_eq!(cfg.peek_secs, 60);
    }

    #[test]
    fn poll_interval_is_clamped_to_something_sane() {
        let poll = PollConfig {
            claude_secs: 0,
            ollama_secs: 100_000,
            ..PollConfig::default()
        };
        assert_eq!(poll.for_provider(ProviderId::ClaudeCode), 5);
        assert_eq!(poll.for_provider(ProviderId::Ollama), 3600);
    }

    #[test]
    fn provider_order_appends_unknown_providers() {
        let cfg = Config {
            provider_order: vec!["ollama".into(), "bogus".into()],
            ..Default::default()
        };
        let order = cfg.ordered_providers();
        assert_eq!(order[0], ProviderId::Ollama);
        assert_eq!(order.len(), ProviderId::ALL.len());
    }

    #[test]
    fn strip_grows_by_one_slot_per_provider() {
        let m = HudSize::Medium.metrics();
        let (one, thickness) = m.strip_extent(1);
        let (three, _) = m.strip_extent(3);
        assert_eq!(three - one, m.slot * 2.0);
        assert_eq!(thickness, m.strip_thickness);
    }

    #[test]
    fn an_empty_strip_still_holds_one_slot() {
        // Before the first poll there are no providers; a zero-length strip
        // would disappear off screen entirely.
        let m = HudSize::Medium.metrics();
        assert_eq!(m.strip_extent(0), m.strip_extent(1));
    }

    #[test]
    fn every_size_keeps_the_reference_proportions() {
        // The silhouette is derived from the thickness, so everything laid out
        // against it has to scale with the thickness too -- otherwise the
        // small and large strips stop looking like the same object.
        for size in [HudSize::Small, HudSize::Medium, HudSize::Large] {
            let m = size.metrics();
            for (name, ratio, want) in [
                ("slot", m.slot / m.strip_thickness, 1.47),
                ("padding", m.strip_padding / m.strip_thickness, 0.53),
                ("ring", m.ring / m.strip_thickness, 0.63),
            ] {
                assert!(
                    (ratio - want).abs() < 0.02,
                    "{size:?} {name} is {ratio:.2}x thickness, expected ~{want:.2}x"
                );
            }
        }
    }

    #[test]
    fn the_settings_gear_is_part_of_the_strip_length() {
        // The gear is laid out on the strip alongside the rings. Leaving it out
        // of the arithmetic made the window shorter than its own contents, so
        // the gear was clipped off the end.
        let m = HudSize::Medium.metrics();
        let (along, _) = m.strip_extent(3);
        assert_eq!(
            along,
            m.strip_padding * 2.0 + 3.0 * m.slot + m.settings_extent()
        );
        assert!(m.settings_extent() > 0.0);
    }

    #[test]
    fn a_full_strip_stays_a_notch_rather_than_a_sidebar() {
        // Every provider enabled, at every size, on the smallest work area we
        // support. A HUD that reaches the bottom of a 1080p desktop is not a
        // notch -- and at that point the layout clamp starts silently cutting
        // contents off the end instead.
        const WORK_AREA_1080P: f64 = 1040.0;
        for size in [HudSize::Small, HudSize::Medium, HudSize::Large] {
            let m = size.metrics();
            let (along, thickness) = m.strip_extent(ProviderId::ALL.len());
            assert!(
                along < WORK_AREA_1080P * 0.65,
                "{size:?} strip is {along}px long on a {WORK_AREA_1080P}px screen"
            );
            assert!(
                along <= crate::layout::MAX_STRIP_LENGTH,
                "{size:?} strip ({along}px) is clamped by MAX_STRIP_LENGTH, so it would be cut off"
            );
            assert!(
                thickness <= 56.0,
                "{size:?} strip reaches {thickness}px into the screen"
            );
            // Open, the whole HUD still has to leave the desktop usable.
            assert!(
                thickness + m.popover_gap + m.popover_size < 360.0,
                "{size:?} HUD is too deep when a card is open"
            );
        }
    }

    /// The browser preview (`npm run dev`) has no backend, so it carries its own
    /// copy of the medium metrics. That copy is what the UI is iterated
    /// against, and it silently went stale once already -- leaving the preview
    /// showing a notch two and a half times the size of the real one.
    #[test]
    fn the_frontend_demo_metrics_match_medium() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../src/lib/demo.ts")
            .canonicalize();
        // Vendored builds won't have the frontend beside them; nothing to check.
        let Ok(path) = path else { return };
        let Ok(source) = std::fs::read_to_string(&path) else {
            return;
        };

        let block = source
            .split_once("DEMO_METRICS")
            .and_then(|(_, rest)| rest.split_once('}'))
            .map(|(block, _)| block.to_string())
            .expect("DEMO_METRICS should be declared in demo.ts");

        let field = |name: &str| -> f64 {
            block
                .split_once(&format!("{name}:"))
                .and_then(|(_, rest)| rest.split(&[',', '\n'][..]).next())
                .and_then(|v| v.trim().parse::<f64>().ok())
                .unwrap_or_else(|| panic!("{name} missing from DEMO_METRICS"))
        };

        let m = HudSize::Medium.metrics();
        for (name, demo, rust) in [
            ("stripThickness", field("stripThickness"), m.strip_thickness),
            ("slot", field("slot"), m.slot),
            ("stripPadding", field("stripPadding"), m.strip_padding),
            ("ring", field("ring"), m.ring),
            ("popoverSize", field("popoverSize"), m.popover_size),
            ("popoverGap", field("popoverGap"), m.popover_gap),
        ] {
            assert_eq!(
                demo, rust,
                "demo.ts {name} is {demo}, but HudSize::Medium says {rust}"
            );
        }
    }

    #[test]
    fn the_popover_gap_clears_the_card_tail() {
        // UsagePopover draws a tail 0.4x the strip's thickness; a gap narrower
        // than that would have the tail crossing into the strip.
        for size in [HudSize::Small, HudSize::Medium, HudSize::Large] {
            let m = size.metrics();
            assert!(
                m.popover_gap > m.strip_thickness * 0.4,
                "{size:?} tail overlaps the strip"
            );
        }
    }

    #[test]
    fn every_size_keeps_the_ring_inside_its_slot() {
        for size in [HudSize::Small, HudSize::Medium, HudSize::Large] {
            let m = size.metrics();
            assert!(
                m.ring < m.strip_thickness,
                "{size:?} ring overflows the strip"
            );
            assert!(m.slot > m.ring, "{size:?} slot is smaller than its ring");
            assert!(
                m.popover_size > m.strip_thickness,
                "{size:?} popover too narrow"
            );
        }
    }
}
