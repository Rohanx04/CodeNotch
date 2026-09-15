/**
 * The resting state: an ultra-compact pill showing only what can be read at a
 * glance — the worst utilisation across every provider, and one dot per
 * provider for its state.
 */

import type { ProviderId, Telemetry } from "../types";
import { formatPct } from "../lib/format";
import { Ring, StateDot } from "./Ring";

/** "GitHub Copilot" doesn't fit a 190px pill; these do. */
const SHORT_NAMES: Record<ProviderId, string> = {
  claudeCode: "Claude",
  cursor: "Cursor",
  copilot: "Copilot",
  codex: "Codex",
  ollama: "Ollama",
};

interface Props {
  telemetry: Telemetry;
}

export function CollapsedPill({ telemetry }: Props) {
  const visible = telemetry.providers.filter((p) => p.health !== "unavailable");

  // The provider driving the headline number, so the label names the right one.
  const peakOf = (p: (typeof visible)[number]) =>
    Math.max(
      ...p.windows.filter((w) => !w.informational).map((w) => w.usedPct ?? -1),
      -1,
    );

  const leader = visible.reduce<(typeof visible)[number] | null>(
    (best, p) => (peakOf(p) > (best ? peakOf(best) : -1) ? p : best),
    null,
  );

  const nothingYet = visible.length === 0;

  return (
    <div className="notch-surface flex h-full w-full items-center gap-2 rounded-full px-2.5">
      {/* The ring carries the attention state: amber and pulsing means a
          session is blocked on the user, so no second warning glyph is needed. */}
      <Ring
        pct={telemetry.peakPct}
        activity={telemetry.activity}
        health={telemetry.health}
        size={18}
      />

      <div className="flex min-w-0 flex-1 items-baseline gap-1.5">
        <span className="tnum text-[13px] leading-none font-semibold">
          {nothingYet ? "—" : formatPct(telemetry.peakPct)}
        </span>
        <span className="truncate text-[10px] leading-none text-notch-muted">
          {nothingYet ? "no providers" : leader ? SHORT_NAMES[leader.id] : ""}
        </span>
      </div>

      <div className="flex shrink-0 items-center gap-[3px]">
        {visible.slice(0, 5).map((provider) => (
          <StateDot
            key={provider.id}
            activity={provider.activity}
            health={provider.health}
            pct={peakOf(provider) >= 0 ? peakOf(provider) : null}
          />
        ))}
      </div>
    </div>
  );
}
