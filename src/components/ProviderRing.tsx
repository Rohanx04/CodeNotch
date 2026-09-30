/**
 * One provider on the strip: the provider's mark inside a track ring, a
 * coloured arc around it for how much of the limit is gone, and the percentage
 * underneath.
 *
 * The ring is also the provider's activity light, in layers:
 *
 * - the **arc** sweeps while an agent is generating and pulses amber when one
 *   is blocked on the user;
 * - a soft **glow** behind the disc takes the state's colour;
 * - a **badge** on the corner pops in for the states that want the user --
 *   blocked, finished, broken;
 * - and each *change* of state gets a one-off gesture: a small hop out of the
 *   edge when an agent starts waiting, a spin and a few sparks when one
 *   finishes, a shake when something breaks.
 *
 * On launch every arc sweeps up from empty, one ring after another.
 */

import { useEffect, useRef, useState } from "react";
import { Check } from "lucide-react";

import type { Activity, Edge, Health, ProviderSnapshot } from "../types";
import { formatPct, peakOf, usageTone } from "../lib/format";
import { prefersReducedMotion } from "../lib/motion";
import { BrandIcon } from "./BrandIcon";

interface Props {
  provider: ProviderSnapshot;
  /** Ring diameter in px, from the backend's metrics. */
  size: number;
  edge: Edge;
  active: boolean;
  /** A live Claude Code session ended in an error. */
  failed?: boolean;
  /** Launch sweep: how long this ring waits before filling. */
  introDelay: number;
  onHover: (provider: ProviderSnapshot | null) => void;
  onActivate: (provider: ProviderSnapshot) => void;
}

type BadgeKind = "attention" | "done" | "error" | null;

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

const broken = (h: Health) => h === "error" || h === "needsAuth";

function badgeFor(activity: Activity, health: Health, failed: boolean): BadgeKind {
  if (activity === "awaitingInput") return "attention";
  if (broken(health) || (failed && activity === "done")) return "error";
  if (activity === "done") return "done";
  return null;
}

/** Which way is "out of the edge", for the hop. */
function inward(edge: Edge, distance: number): string {
  switch (edge) {
    case "right":
      return `translate(${-distance}px, 0)`;
    case "left":
      return `translate(${distance}px, 0)`;
    case "top":
      return `translate(0, ${distance}px)`;
    case "bottom":
      return `translate(0, ${-distance}px)`;
  }
}

/**
 * The corner badge. A change shrinks the old one away first, then the new one
 * pops in with a little overshoot -- so a swap reads as a swap rather than a
 * flicker.
 */
function Badge({ kind }: { kind: BadgeKind }) {
  const [shown, setShown] = useState<BadgeKind>(kind);
  const [leaving, setLeaving] = useState(false);

  useEffect(() => {
    if (kind === shown) return;
    if (shown === null) {
      setShown(kind);
      return;
    }
    setLeaving(true);
    const timer = window.setTimeout(() => {
      setShown(kind);
      setLeaving(false);
    }, 90);
    return () => window.clearTimeout(timer);
  }, [kind, shown]);

  if (!shown) return null;
  return (
    <span key={shown} className="ring-badge" data-kind={shown} data-leaving={leaving || undefined}>
      {shown === "attention" && "!"}
      {shown === "done" && <Check aria-hidden />}
      {shown === "error" && "×"}
    </span>
  );
}

export function ProviderRing({
  provider,
  size,
  edge,
  active,
  failed = false,
  introDelay,
  onHover,
  onActivate,
}: Props) {
  const pct = peakOf(provider);
  const colour = toneColour(pct, provider.health);
  const discRef = useRef<HTMLSpanElement | null>(null);
  const previous = useRef<{ activity: Activity; health: Health; failed: boolean } | null>(null);
  const [intro, setIntro] = useState(false);
  const [sparks, setSparks] = useState(0);

  // Bold enough to read as a gauge at a glance, without the ring closing up on
  // itself: at notch scale an eighth of the diameter leaves barely any hole for
  // the brand mark, so the stroke is a ninth with a hairline floor.
  const stroke = Math.max(2.5, size * 0.11);
  const radius = (size - stroke) / 2;
  const circumference = 2 * Math.PI * radius;
  const fraction = pct === null || !intro ? 0 : Math.min(Math.max(pct, 0), 100) / 100;

  const generating = provider.activity === "generating";
  const waiting = provider.activity === "awaitingInput";
  const iconSize = Math.max(8, Math.round(size * 0.42));
  const badge = badgeFor(provider.activity, provider.health, failed);

  // The launch sweep: every arc starts empty and fills in turn.
  useEffect(() => {
    const timer = window.setTimeout(() => setIntro(true), introDelay);
    return () => window.clearTimeout(timer);
  }, [introDelay]);

  // A gesture for each change of state -- never for the state itself, and
  // never on the first render.
  useEffect(() => {
    const before = previous.current;
    previous.current = { activity: provider.activity, health: provider.health, failed };
    const disc = discRef.current;
    if (!before || !disc || prefersReducedMotion()) return;

    const nowBroken = broken(provider.health) || (failed && provider.activity === "done");
    const wasBroken = broken(before.health) || (before.failed && before.activity === "done");

    if (nowBroken && !wasBroken) {
      const d = size * 0.08;
      disc.animate(
        [
          { transform: "translateX(0)" },
          { transform: `translateX(${d}px)`, offset: 0.18 },
          { transform: `translateX(${-d}px)`, offset: 0.43 },
          { transform: `translateX(${d * 0.6}px)`, offset: 0.68 },
          { transform: "translateX(0)" },
        ],
        { duration: 280, easing: "ease-out" },
      );
    } else if (provider.activity === "awaitingInput" && before.activity !== "awaitingInput") {
      disc.animate(
        [
          { transform: "translate(0, 0)" },
          { transform: inward(edge, size * 0.2), offset: 0.33, easing: "cubic-bezier(0.34, 1.56, 0.64, 1)" },
          { transform: "translate(0, 0)" },
        ],
        { duration: 450, easing: "cubic-bezier(0.22, 1, 0.36, 1)" },
      );
    } else if (provider.activity === "done" && before.activity !== "done") {
      disc.animate([{ transform: "rotate(0deg)" }, { transform: "rotate(360deg)" }], {
        duration: 950,
        easing: "cubic-bezier(0.65, 0, 0.35, 1)",
      });
      setSparks((n) => n + 1);
    }
  }, [provider.activity, provider.health, failed, edge, size]);

  return (
    <button
      type="button"
      className="ring-slot"
      onMouseEnter={() => onHover(provider)}
      onFocus={() => onHover(provider)}
      onClick={() => onActivate(provider)}
      aria-label={`${provider.name}: ${pct === null ? "usage unknown" : `${Math.round(pct)}% used`}`}
      data-active={active || undefined}
      data-ring={provider.id}
    >
      <span className="ring-disc" ref={discRef} style={{ width: size, height: size }}>
        <span
          className="ring-glow"
          data-state={badge ?? (generating ? "busy" : "idle")}
          aria-hidden
        />

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

        <Badge kind={badge} />

        {sparks > 0 && (
          <span key={sparks} className="ring-sparks" aria-hidden>
            {[0, 1, 2, 3, 4].map((i) => (
              <i key={i} style={{ "--spark-angle": `${i * 72 - 90}deg` } as React.CSSProperties} />
            ))}
          </span>
        )}
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
