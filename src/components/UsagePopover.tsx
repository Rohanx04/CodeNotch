/**
 * The detail card that opens beside a hovered ring.
 *
 * One block per usage window: what it counts and when it resets on the top
 * line, a bar, then the percentage. When the Claude Code hooks are installed,
 * a live ticker underneath shows what the current session is doing, step by
 * step.
 */

import type { Config, Edge, LiveSession, ProviderSnapshot } from "../types";
import {
  activityLabel,
  formatAgo,
  formatReset,
  healthLabel,
  windowValue,
} from "../lib/format";
import { projectColour } from "../lib/layout";
import { BrandIcon } from "./BrandIcon";
import { StepTicker } from "./StepTicker";

interface Props {
  provider: ProviderSnapshot;
  config: Config;
  /** Live Claude Code sessions, from the hooks. */
  live: LiveSession[];
  onFocusProvider: (provider: ProviderSnapshot, titleHint?: string | null) => void;
}

/**
 * The tail, as multiples of the strip's thickness.
 *
 * Measured off the reference along with the silhouette: a long spike with
 * concave flanks, not a triangle. Each flank is one quadratic whose control
 * point sits on the card's own edge, 46% of the way from the tip back to the
 * base corner — which is what gives it the drawn-out, liquid look.
 */
const TAIL_LENGTH = 0.4;
const TAIL_HALF = 0.33;
const TAIL_CONTROL = 0.46;

/**
 * The tail, pointing from the card back at the ring it belongs to. Its
 * position along the card comes from `--tail-offset`, which App's animation
 * loop sets as the card glides between rings.
 */
export function CardTail({ edge, thickness }: { edge: Edge; thickness: number }) {
  const length = Math.round(thickness * TAIL_LENGTH);
  const half = Math.round(thickness * TAIL_HALF);
  const c = half * (1 - TAIL_CONTROL);

  // Drawn pointing right, then rotated onto the edge that needs it.
  const vertical = edge === "left" || edge === "right";
  const w = vertical ? length : half * 2;
  const h = vertical ? half * 2 : length;
  const flip = edge === "left" || edge === "top";
  const at = (along: number, across: number) => {
    const a = flip ? length - along : along;
    return vertical ? `${a} ${across}` : `${across} ${a}`;
  };

  return (
    <svg
      className="popover-tail"
      width={w}
      height={h}
      viewBox={`0 0 ${w} ${h}`}
      style={{ "--tail-half": `${half}px` } as React.CSSProperties}
      aria-hidden
    >
      <path
        d={[
          `M ${at(0, 0)}`,
          `Q ${at(0, c)} ${at(length, half)}`,
          `Q ${at(0, half * 2 - c)} ${at(0, half * 2)}`,
          "Z",
        ].join(" ")}
        fill="var(--popover-bg)"
      />
    </svg>
  );
}

/** The live session worth watching: blocked, then working, then most recent. */
function liveFocus(live: LiveSession[]): LiveSession | null {
  const rank = { awaitingInput: 3, generating: 2, done: 1, idle: 0 } as const;
  return (
    [...live]
      .filter((s) => s.steps.length > 0)
      .sort(
        (a, b) =>
          rank[b.activity] - rank[a.activity] ||
          new Date(b.lastEvent).getTime() - new Date(a.lastEvent).getTime(),
      )[0] ?? null
  );
}

export function UsagePopover({ provider, config, live, onFocusProvider }: Props) {
  const health = healthLabel(provider.health);
  const activity = activityLabel(provider.activity);
  const ticker = provider.id === "claudeCode" ? liveFocus(live) : null;

  return (
    <div className="popover" role="dialog" aria-label={`${provider.name} usage`}>
      <header className="popover-head">
        <BrandIcon provider={provider.id} className="popover-mark" />
        {/* Just the name: at notch width "Claude Code Usage" truncates, and the
            bars underneath already say this is a usage card. The dialog's
            aria-label still spells it out for screen readers. */}
        <span className="popover-title">{provider.name}</span>
        {activity && (
          <span
            className={`popover-activity${provider.activity === "generating" ? " shimmer" : ""}`}
            data-state={provider.activity}
          >
            {activity}
          </span>
        )}
      </header>

      {provider.account && <p className="popover-account">{provider.account}</p>}

      {provider.windows.length > 0 ? (
        <div className="popover-windows">
          {provider.windows.map((window) => {
            const reset = formatReset(window.resetsAt, config.resetAsCountdown);
            const pct = window.usedPct;
            const tone =
              window.informational || pct === null
                ? "info"
                : pct >= 70
                  ? "high"
                  : pct >= 40
                    ? "mid"
                    : "low";
            return (
              <div key={window.key} className="usage-block">
                <div className="usage-line">
                  <span className="usage-label">{window.label}</span>
                  <span className="usage-reset tnum">
                    {reset ? (config.resetAsCountdown ? `Resets in ${reset}` : `Resets ${reset}`) : ""}
                  </span>
                </div>
                <div className="usage-track">
                  <div
                    className="usage-fill"
                    data-tone={tone}
                    style={{ width: `${pct === null ? 0 : Math.min(pct, 100)}%` }}
                  />
                </div>
                <div className="usage-value tnum">
                  {windowValue(window)}
                  {pct !== null && " Used"}
                </div>
              </div>
            );
          })}
        </div>
      ) : (
        <p className="popover-empty">
          {provider.detail ?? "Nothing to report yet."}
        </p>
      )}

      {/* An explanation only when the numbers can't be trusted. */}
      {health && provider.windows.length > 0 && provider.detail && (
        <p className="popover-note">{provider.detail}</p>
      )}

      {ticker && (
        <div className="popover-live">
          <span className="popover-live-label">
            <i style={{ background: projectColour(ticker.project) }} />
            {ticker.project}
          </span>
          <StepTicker
            steps={ticker.steps}
            stepCount={ticker.stepCount}
            sessionId={ticker.id}
          />
        </div>
      )}

      {provider.sessions.length > 0 && (
        <div className="popover-sessions">
          {provider.sessions.slice(0, 3).map((session) => (
            <button
              key={session.id}
              type="button"
              className="session-row"
              onClick={() => onFocusProvider(provider, session.cwd ?? session.title)}
            >
              <span className="session-dot" data-state={session.activity} />
              <span
                className="session-project"
                style={{ background: projectColour(session.title) }}
                aria-hidden
              />
              <span className="session-title">{session.title}</span>
              <span
                className={`session-meta tnum${session.activity === "generating" && session.detail ? " shimmer" : ""}`}
              >
                {session.detail ?? formatAgo(session.lastActivity) ?? ""}
              </span>
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
