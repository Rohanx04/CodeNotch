/**
 * The detail card that opens beside a hovered ring.
 *
 * One block per usage window: what it counts and when it resets on the top
 * line, a bar, then the percentage. A tail on the edge nearest the strip points
 * back at the ring it belongs to, so with several rings stacked up it is always
 * obvious which one you are reading.
 */

import type { Config, Edge, ProviderSnapshot } from "../types";
import {
  activityLabel,
  formatAgo,
  formatReset,
  healthLabel,
  windowValue,
} from "../lib/format";
import { BrandIcon } from "./BrandIcon";

interface Props {
  provider: ProviderSnapshot;
  config: Config;
  edge: Edge;
  /** Centre of the ring this belongs to, in window coordinates. */
  anchor: number;
  onFocusProvider: (provider: ProviderSnapshot, titleHint?: string | null) => void;
}

/** Tail size in px; the CSS triangle is drawn to match. */
const TAIL = 9;

export function UsagePopover({
  provider,
  config,
  edge,
  anchor,
  onFocusProvider,
}: Props) {
  const vertical = edge === "left" || edge === "right";
  const health = healthLabel(provider.health);
  const activity = activityLabel(provider.activity);

  return (
    <div
      className="popover"
      data-edge={edge}
      // The tail tracks the ring; everything else stays put.
      style={
        vertical
          ? ({ "--tail-offset": `${anchor}px` } as React.CSSProperties)
          : ({ "--tail-offset": `${anchor}px` } as React.CSSProperties)
      }
      role="dialog"
      aria-label={`${provider.name} usage`}
    >
      <span className="popover-tail" style={{ "--tail": `${TAIL}px` } as React.CSSProperties} />

      <header className="popover-head">
        <BrandIcon provider={provider.id} className="popover-mark" />
        <span className="popover-title">{provider.name} Usage</span>
        {activity && (
          <span
            className="popover-activity"
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
              <span className="session-title">{session.title}</span>
              <span className="session-meta tnum">
                {session.detail ?? formatAgo(session.lastActivity) ?? ""}
              </span>
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
