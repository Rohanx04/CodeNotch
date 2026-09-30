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

/** A Claude Code session as the hooks report it. Mirrors `LiveSession`. */
export interface LiveSession {
  id: string;
  project: string;
  cwd: string | null;
  activity: Activity;
  /** The most recent steps, oldest first. */
  steps: string[];
  /** Steps seen in total, so the ticker can tell how many are new. */
  stepCount: number;
  lastEvent: string;
  failed: boolean;
}

export interface Telemetry {
  providers: ProviderSnapshot[];
  generatedAt: string;
  peakPct: number | null;
  activity: Activity;
  health: Health;
  /** Live Claude Code sessions; empty unless the hooks are installed. */
  live: LiveSession[];
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
  /** Retired: click-through follows the painted shapes now. Kept so configs round-trip. */
  clickThroughWhenCollapsed: boolean;
  peekOnAttention: boolean;
  peekSecs: number;
  notifyOnThresholds: boolean;
  resetAsCountdown: boolean;
  launchAtLogin: boolean;
  autoHide: boolean;
  autoHideSecs: number;
  sound: boolean;
  soundVolume: number;

  poll: PollConfig;
  providers: Record<string, boolean>;
  providerOrder: string[];
  ollamaUrl: string;
}

export interface HudState {
  /** A card should be on screen. */
  open: boolean;
  /** The pointer is on the strip or the open card. */
  hovering: boolean;
  pinned: boolean;
  hidden: boolean;
  /** Open because of an attention peek rather than the pointer. */
  peeking: boolean;
  /** When the peek closes, Unix milliseconds. */
  peekEndsAt: number | null;
  /** The provider a peek or alert is about. */
  focus: ProviderId | null;
  /** A permission request is holding the notch open. */
  alert: boolean;
  /** Auto-hide has tucked the strip into the edge. */
  retracted: boolean;
  /** The window is the thin wake strip. */
  wake: boolean;
  paused: boolean;
  /** Logical window size. */
  width: number;
  height: number;
  /** The strip's top-left inside the window. */
  stripX: number;
  stripY: number;
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
  /** A permission request already waiting when the webview loaded. */
  approval: ApprovalRequest | null;
}

/** A shape the webview painted, in window-logical pixels. */
export interface HitRect {
  x: number;
  y: number;
  w: number;
  h: number;
}

/** A Claude Code permission request on the notch. */
export interface ApprovalRequest {
  id: string;
  sessionId: string;
  project: string;
  tool: string;
  /** Exactly what Allow authorises, e.g. `Bash · cargo publish`. */
  target: string;
  /** When the terminal takes over, Unix milliseconds. */
  expiresAt: number;
}

export type Cue = "attention" | "approval" | "finish" | "error" | "threshold";

export interface HookStatus {
  installed: boolean;
  settingsPath: string;
  hookPath: string;
  hookReady: boolean;
}

export interface HookPreview {
  install: boolean;
  diff: string;
  backup: string;
  settingsPath: string;
  fingerprint: string;
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
