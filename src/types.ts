/**
 * Mirrors of the Rust types in `src-tauri/core/src/model.rs` and `config.rs`.
 *
 * Both sides serialise camelCase, so these line up field for field. Keep them
 * in sync: the Rust structs are the source of truth.
 */

export type ProviderId =
  | "claudeCode"
  | "cursor"
  | "copilot"
  | "codex"
  | "gemini"
  | "perplexity"
  | "ollama";

/** Whether a provider's numbers can be trusted right now. */
export type Health =
  | "unavailable"
  | "ok"
  | "stale"
  | "rateLimited"
  | "needsAuth"
  | "error";

/** What a provider's agent is doing. */
export type Activity = "idle" | "done" | "generating" | "awaitingInput";

export type UsageUnit = "percent" | "tokens" | "requests" | "credits" | "bytes";

export interface UsageWindow {
  key: string;
  label: string;
  /** 0..100, or null when the provider gives counts with no denominator. */
  usedPct: number | null;
  used: number | null;
  limit: number | null;
  unit: UsageUnit;
  /** RFC 3339. */
  resetsAt: string | null;
  /** Derived locally rather than reported; rendered with a `~`. */
  estimated: boolean;
  /** Context rather than a quota (e.g. Ollama's GPU share): never coloured as an alarm. */
  informational: boolean;
}

export interface Session {
  id: string;
  title: string;
  cwd: string | null;
  model: string | null;
  activity: Activity;
  lastActivity: string | null;
  tokens: number | null;
  detail: string | null;
}

export interface ProviderSnapshot {
  id: ProviderId;
  name: string;
  health: Health;
  activity: Activity;
  detail: string | null;
  source: string | null;
  account: string | null;
  windows: UsageWindow[];
  sessions: Session[];
  updatedAt: string;
  retryAt: string | null;
}

export interface Telemetry {
  providers: ProviderSnapshot[];
  generatedAt: string;
  peakPct: number | null;
  activity: Activity;
  health: Health;
}

export type Edge = "top" | "bottom" | "left" | "right";
export type HudSize = "small" | "medium" | "large";

export type MonitorChoice =
  | { kind: "primary" }
  | { kind: "index"; index: number };

export interface PollConfig {
  claudeSecs: number;
  cursorSecs: number;
  copilotSecs: number;
  codexSecs: number;
  geminiSecs: number;
  perplexitySecs: number;
  ollamaSecs: number;
  activitySecs: number;
}

export interface Config {
  edge: Edge;
  edgeOffset: number;
  margin: number;
  size: HudSize;
  accent: string;
  monitor: MonitorChoice;

  alwaysExpanded: boolean;
  hidden: boolean;
  clickThroughWhenCollapsed: boolean;
  peekOnAttention: boolean;
  peekSecs: number;
  notifyOnThresholds: boolean;
  resetAsCountdown: boolean;
  launchAtLogin: boolean;

  poll: PollConfig;
  providers: Record<string, boolean>;
  providerOrder: string[];
  ollamaUrl: string;
}

export interface HudState {
  /** A detail popover is open, so the window has grown inward to hold it. */
  open: boolean;
  pinned: boolean;
  hidden: boolean;
  peeking: boolean;
  /** Logical window size, so the webview can place the strip within it. */
  width: number;
  height: number;
}

/**
 * Strip and popover dimensions, sent by the backend so both sides draw to the
 * same numbers the window is sized with. Mirrors `HudMetrics` in Rust.
 */
export interface HudMetrics {
  stripThickness: number;
  slot: number;
  stripPadding: number;
  ring: number;
  popoverSize: number;
  popoverGap: number;
}

export interface MonitorInfo {
  index: number;
  label: string;
  width: number;
  height: number;
  primary: boolean;
}

export interface Bootstrap {
  config: Config;
  telemetry: Telemetry;
  hud: HudState;
  metrics: HudMetrics;
  edge: Edge;
  version: string;
  /** False on non-Windows dev builds, where the Win32 layer is a no-op. */
  nativeWindow: boolean;
}

/** Providers whose card can raise a real window when clicked. */
export const FOCUSABLE: ReadonlySet<ProviderId> = new Set<ProviderId>([
  "claudeCode",
  "cursor",
  "copilot",
  "codex",
  "gemini",
  "perplexity",
]);
