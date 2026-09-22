/**
 * One provider on the strip: the provider's mark inside a track ring, a
 * coloured arc around it for how much of the limit is gone, and the percentage
 * underneath.
 *
 * The arc doubles as the activity indicator — it sweeps while an agent is
 * generating and pulses amber when one is blocked on the user — so a single
 * glance at the strip answers both "how much is left" and "does anything need
 * me".
 */

import type { Health, ProviderSnapshot } from "../types";
import { formatPct, peakOf, usageTone } from "../lib/format";
import { BrandIcon } from "./BrandIcon";

interface Props {
  provider: ProviderSnapshot;
  /** Ring diameter in px, from the backend's metrics. */
  size: number;
  active: boolean;
  onHover: (provider: ProviderSnapshot | null) => void;
  onActivate: (provider: ProviderSnapshot) => void;
}

/** Traffic-light tone for a utilisation level. */
function toneColour(pct: number | null, health: Health): string {
  if (health === "unavailable") return "var(--tone-off)";
  if (health === "stale" || health === "rateLimited") return "var(--tone-stale)";
  if (health === "needsAuth" || health === "error") return "var(--tone-bad)";
  switch (usageTone(pct)) {
    case "high":
      return "var(--tone-high)";
    case "mid":
      return "var(--tone-mid)";
    default:
      return "var(--tone-low)";
  }
}

export function ProviderRing({
  provider,
  size,
  active,
  onHover,
  onActivate,
}: Props) {
  const pct = peakOf(provider);
  const colour = toneColour(pct, provider.health);

  // Bold enough to read as a gauge at a glance, without the ring closing up on
  // itself: at notch scale an eighth of the diameter leaves barely any hole for
  // the brand mark, so the stroke is a ninth with a hairline floor.
  const stroke = Math.max(2.5, size * 0.11);
  const radius = (size - stroke) / 2;
  const circumference = 2 * Math.PI * radius;
  const fraction = pct === null ? 0 : Math.min(Math.max(pct, 0), 100) / 100;

  const generating = provider.activity === "generating";
  const waiting = provider.activity === "awaitingInput";
  const iconSize = Math.max(8, Math.round(size * 0.42));

  return (
    <button
      type="button"
      className="ring-slot"
      onMouseEnter={() => onHover(provider)}
      onFocus={() => onHover(provider)}
      onClick={() => onActivate(provider)}
      aria-label={`${provider.name}: ${pct === null ? "usage unknown" : `${Math.round(pct)}% used`}`}
      data-active={active || undefined}
    >
      <span className="ring-disc" style={{ width: size, height: size }}>
        <svg
          viewBox={`0 0 ${size} ${size}`}
          width={size}
          height={size}
          className="ring-arc"
          aria-hidden
        >
          {/* Rotate so the arc starts at 12 o'clock and fills clockwise. */}
          <g transform={`rotate(-90 ${size / 2} ${size / 2})`}>
            <circle
              cx={size / 2}
              cy={size / 2}
              r={radius}
              fill="none"
              stroke="var(--ring-track)"
              strokeWidth={stroke}
            />
            {pct !== null && (
              <circle
                cx={size / 2}
                cy={size / 2}
                r={radius}
                fill="none"
                stroke={colour}
                strokeWidth={stroke}
                strokeLinecap="round"
                strokeDasharray={`${circumference * fraction} ${circumference}`}
                className={waiting ? "activity-pulse" : undefined}
                style={{
                  // The arc sweeps to its new level on the shared curve; the
                  // colour crosses the traffic-light boundary a little slower
                  // so green->amber->red reads as a blend, not a switch.
                  transition:
                    "stroke-dasharray 620ms var(--ease-liquid), stroke 420ms ease",
                }}
              />
            )}
            {generating && (
              // A short comet riding the ring, so "working" is visible even at
              // a glance across the room.
              <circle
                cx={size / 2}
                cy={size / 2}
                r={radius}
                fill="none"
                stroke="var(--tone-busy)"
                strokeWidth={stroke * 0.8}
                strokeLinecap="round"
                strokeDasharray={`${circumference * 0.16} ${circumference}`}
                className="activity-spin"
              />
            )}
          </g>
        </svg>

        <BrandIcon
          provider={provider.id}
          className="ring-mark"
          // Inline so the mark scales with the ring rather than a fixed step.
          {...{ style: { width: iconSize, height: iconSize } }}
        />
      </span>

      {/* The floor matters more than the ratio at the small end: below 9px the
          percentage stops being legible, and it is the number people read. */}
      <span
        className="ring-pct tnum"
        style={{ fontSize: Math.max(9, Math.round(size * 0.4)) }}
      >
        {pct === null ? "—" : formatPct(pct)}
      </span>
    </button>
  );
}
