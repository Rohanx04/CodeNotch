/** Small formatting helpers shared by the pill and the expanded card. */

import type { Activity, Health, UsageUnit, UsageWindow } from "../types";

/** "42%" — or "—" when there is no percentage to show. */
export function formatPct(pct: number | null | undefined): string {
  if (pct === null || pct === undefined || Number.isNaN(pct)) return "—";
  // Below 1% still reads as 1% rather than 0%, so "barely used" and "unused"
  // don't look identical.
  if (pct > 0 && pct < 1) return "1%";
  return `${Math.round(pct)}%`;
}

/** Compact counts: 1.2k, 3.4M. */
export function formatCount(value: number): string {
  const abs = Math.abs(value);
  if (abs >= 1_000_000_000) return `${(value / 1_000_000_000).toFixed(1)}B`;
  if (abs >= 1_000_000) return `${(value / 1_000_000).toFixed(1)}M`;
  if (abs >= 1_000) return `${(value / 1_000).toFixed(1)}k`;
  return `${Math.round(value)}`;
}

export function formatBytes(bytes: number): string {
  const units = ["B", "KB", "MB", "GB", "TB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  if (unit === 0) return `${Math.round(value)} B`;
  return `${value >= 100 ? value.toFixed(0) : value.toFixed(1)} ${units[unit]}`;
}

/** The headline value for a window: a percentage if there is one, else counts. */
export function windowValue(window: UsageWindow): string {
  const prefix = window.estimated ? "~" : "";
  if (window.usedPct !== null) return prefix + formatPct(window.usedPct);
  if (window.used === null) return "—";
  return prefix + formatUnit(window.used, window.unit);
}

export function formatUnit(value: number, unit: UsageUnit): string {
  switch (unit) {
    case "bytes":
      return formatBytes(value);
    case "tokens":
      return `${formatCount(value)} tok`;
    case "requests":
      return `${formatCount(value)} req`;
    case "credits":
      return `${formatCount(value)} cr`;
    default:
      return formatPct(value);
  }
}

/**
 * "resets in 2h 14m", or a clock time when the user prefers that.
 *
 * Returns null when there is no reset to show, so callers can omit the line
 * entirely rather than render an empty one.
 */
export function formatReset(
  resetsAt: string | null,
  asCountdown: boolean,
  now: number = Date.now(),
): string | null {
  if (!resetsAt) return null;
  const target = new Date(resetsAt).getTime();
  if (Number.isNaN(target)) return null;

  if (!asCountdown) {
    return new Date(target).toLocaleTimeString([], {
      hour: "2-digit",
      minute: "2-digit",
    });
  }

  const remaining = target - now;
  if (remaining <= 0) return "resetting";

  const minutes = Math.floor(remaining / 60_000);
  const days = Math.floor(minutes / 1440);
  const hours = Math.floor((minutes % 1440) / 60);
  const mins = minutes % 60;

  if (days > 0) return `${days}d ${hours}h`;
  if (hours > 0) return `${hours}h ${mins}m`;
  if (minutes > 0) return `${minutes}m`;
  return "<1m";
}

/** "3m ago" for a session's last activity. */
export function formatAgo(iso: string | null, now: number = Date.now()): string | null {
  if (!iso) return null;
  const then = new Date(iso).getTime();
  if (Number.isNaN(then)) return null;

  const secs = Math.max(0, Math.round((now - then) / 1000));
  if (secs < 10) return "just now";
  if (secs < 60) return `${secs}s ago`;
  const mins = Math.floor(secs / 60);
  if (mins < 60) return `${mins}m ago`;
  const hours = Math.floor(mins / 60);
  if (hours < 24) return `${hours}h ago`;
  return `${Math.floor(hours / 24)}d ago`;
}

/** Short human label for a health state. */
export function healthLabel(health: Health): string | null {
  switch (health) {
    case "ok":
      return null;
    case "stale":
      return "stale";
    case "rateLimited":
      return "rate limited";
    case "needsAuth":
      return "sign in";
    case "error":
      return "error";
    case "unavailable":
      return "not running";
  }
}

export function activityLabel(activity: Activity): string | null {
  switch (activity) {
    case "generating":
      return "working";
    case "awaitingInput":
      return "needs you";
    case "done":
      return "finished";
    case "idle":
      return null;
  }
}

/**
 * Ring colour for a utilisation level.
 *
 * Deliberately not a smooth gradient: the point is that a glance tells you
 * which of three buckets you're in.
 */
export function usageTone(pct: number | null): "accent" | "warn" | "danger" {
  if (pct === null) return "accent";
  if (pct >= 90) return "danger";
  if (pct >= 70) return "warn";
  return "accent";
}
