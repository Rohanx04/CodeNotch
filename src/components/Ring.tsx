/**
 * The usage ring: a track, an arc for how much of the limit is burned, and an
 * activity indicator layered on top.
 *
 * The activity layer is what makes the HUD glanceable — a thin cyan arc
 * sweeping the ring means an agent is generating; an amber pulse means one is
 * blocked waiting for the user.
 */

import type { Activity, Health } from "../types";
import { usageTone } from "../lib/format";

interface RingProps {
  /** 0..100, or null for "no number to show". */
  pct: number | null;
  activity: Activity;
  health: Health;
  /** Outer diameter in px. */
  size?: number;
  strokeWidth?: number;
  /** Text drawn in the middle (usually the percentage). */
  label?: string;
}

/**
 * Colour for the activity layer.
 *
 * "Waiting for you" is checked before the health colours on purpose: a blocked
 * agent is the most actionable thing the HUD can tell you, and a provider that
 * needs re-authenticating must not paint over it.
 */
function activityColour(activity: Activity, health: Health): string | null {
  if (activity === "awaitingInput") return "var(--color-state-wait)";
  if (health === "needsAuth" || health === "error") return "var(--color-state-bad)";
  switch (activity) {
    case "generating":
      return "var(--color-state-busy)";
    case "done":
      return "var(--color-state-done)";
    case "idle":
      return null;
  }
}

function usageColour(pct: number | null, health: Health): string {
  if (health === "unavailable") return "var(--color-state-off)";
  if (health === "stale" || health === "rateLimited") return "var(--color-notch-faint)";
  switch (usageTone(pct)) {
    case "danger":
      return "var(--color-ring-danger)";
    case "warn":
      return "var(--color-ring-warn)";
    default:
      return "var(--accent)";
  }
}

export function Ring({
  pct,
  activity,
  health,
  size = 18,
  strokeWidth = 2.5,
  label,
}: RingProps) {
  const radius = (size - strokeWidth) / 2;
  const circumference = 2 * Math.PI * radius;
  const fraction = pct === null ? 0 : Math.min(Math.max(pct, 0), 100) / 100;

  const activityTone = activityColour(activity, health);
  // A short arc that sweeps; 18% of the circle reads as a comet, not a gauge.
  const activityArc = circumference * 0.18;

  return (
    <svg
      width={size}
      height={size}
      viewBox={`0 0 ${size} ${size}`}
      className="shrink-0 overflow-visible"
      role="img"
      aria-label={
        pct === null ? "usage unknown" : `${Math.round(pct)} percent of limit used`
      }
    >
      {/* Rotate so 0% starts at 12 o'clock and fills clockwise. */}
      <g transform={`rotate(-90 ${size / 2} ${size / 2})`}>
        <circle
          cx={size / 2}
          cy={size / 2}
          r={radius}
          fill="none"
          stroke="var(--color-ring-track)"
          strokeWidth={strokeWidth}
        />

        {pct !== null && (
          <circle
            cx={size / 2}
            cy={size / 2}
            r={radius}
            fill="none"
            stroke={usageColour(pct, health)}
            strokeWidth={strokeWidth}
            strokeLinecap="round"
            strokeDasharray={`${circumference * fraction} ${circumference}`}
            style={{ transition: "stroke-dasharray 400ms ease, stroke 300ms ease" }}
          />
        )}

        {activityTone && (
          <circle
            cx={size / 2}
            cy={size / 2}
            r={radius}
            fill="none"
            stroke={activityTone}
            strokeWidth={activity === "awaitingInput" ? strokeWidth : strokeWidth * 0.7}
            strokeLinecap="round"
            strokeDasharray={
              activity === "generating"
                ? `${activityArc} ${circumference}`
                : `${circumference} 0`
            }
            className={
              activity === "generating"
                ? "activity-spin"
                : activity === "awaitingInput"
                  ? "activity-pulse"
                  : undefined
            }
            opacity={activity === "done" ? 0.55 : 1}
          />
        )}
      </g>

      {label && (
        <text
          x="50%"
          y="50%"
          dominantBaseline="central"
          textAnchor="middle"
          className="tnum fill-notch-text"
          style={{ fontSize: size * 0.34, fontWeight: 600 }}
        >
          {label}
        </text>
      )}
    </svg>
  );
}

/** A tiny state dot, used for the per-provider row on the collapsed pill. */
export function StateDot({
  activity,
  health,
  pct,
}: {
  activity: Activity;
  health: Health;
  pct: number | null;
}) {
  const busy = activityColour(activity, health);
  const colour =
    health === "unavailable"
      ? "var(--color-state-off)"
      : (busy ?? usageColour(pct, health));

  return (
    <span
      className={`inline-block size-[5px] rounded-full ${
        activity === "awaitingInput" ? "activity-pulse" : ""
      }`}
      style={{ background: colour }}
      aria-hidden
    />
  );
}
