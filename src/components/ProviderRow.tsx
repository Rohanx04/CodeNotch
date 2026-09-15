/**
 * One provider inside the expanded card: its rings, reset windows, and any
 * live sessions.
 *
 * Clicking the row raises that provider's window (terminal, VS Code, Cursor),
 * which is the whole point of showing session names here — you see something
 * is blocked and go straight to it.
 */

import {
  Bot,
  CircleAlert,
  Cpu,
  Github,
  MousePointerClick,
  Sparkles,
  Terminal,
} from "lucide-react";
import type { ComponentType } from "react";

import type { Config, ProviderId, ProviderSnapshot } from "../types";
import { FOCUSABLE } from "../types";
import {
  activityLabel,
  formatAgo,
  formatReset,
  healthLabel,
  windowValue,
} from "../lib/format";
import { Ring } from "./Ring";

const ICONS: Record<ProviderId, ComponentType<{ className?: string }>> = {
  claudeCode: Sparkles,
  cursor: MousePointerClick,
  copilot: Github,
  codex: Terminal,
  ollama: Cpu,
};

interface Props {
  provider: ProviderSnapshot;
  config: Config;
  onFocus: (provider: ProviderSnapshot, titleHint?: string | null) => void;
}

export function ProviderRow({ provider, config, onFocus }: Props) {
  const Icon = ICONS[provider.id] ?? Bot;
  const health = healthLabel(provider.health);
  const activity = activityLabel(provider.activity);
  const clickable = FOCUSABLE.has(provider.id);

  // Mirrors ProviderSnapshot::peak_pct: context windows don't set the tone.
  const peak = Math.max(
    ...provider.windows.filter((w) => !w.informational).map((w) => w.usedPct ?? -1),
    -1,
  );

  return (
    <div className="px-2.5 py-2">
      <button
        type="button"
        disabled={!clickable}
        onClick={() => onFocus(provider)}
        className={`flex w-full items-center gap-2 rounded-md px-1 py-0.5 text-left transition-colors ${
          clickable ? "hover:bg-white/6 cursor-pointer" : "cursor-default"
        }`}
        title={clickable ? `Bring ${provider.name} to the front` : undefined}
      >
        <Ring
          pct={peak >= 0 ? peak : null}
          activity={provider.activity}
          health={provider.health}
          size={20}
        />

        <span className="flex min-w-0 flex-1 flex-col">
          <span className="flex items-baseline gap-1.5">
            <Icon className="size-3 shrink-0 text-notch-muted" />
            <span className="truncate text-[12px] leading-tight font-medium">
              {provider.name}
            </span>
            {activity && (
              <span
                className={`shrink-0 text-[9px] leading-tight uppercase tracking-wide ${
                  provider.activity === "awaitingInput"
                    ? "text-state-wait activity-pulse"
                    : provider.activity === "generating"
                      ? "text-state-busy"
                      : "text-notch-faint"
                }`}
              >
                {activity}
              </span>
            )}
          </span>
          {provider.account && (
            <span className="truncate text-[10px] leading-tight text-notch-faint">
              {provider.account}
            </span>
          )}
        </span>

        {health && (
          <span className="flex shrink-0 items-center gap-1 text-[9px] text-notch-faint">
            <CircleAlert className="size-2.5" />
            {health}
          </span>
        )}
      </button>

      {/* Usage windows */}
      {provider.windows.length > 0 && (
        <div className="mt-1.5 flex flex-col gap-1 pl-1">
          {provider.windows.map((window) => {
            const reset = formatReset(window.resetsAt, config.resetAsCountdown);
            return (
              <div key={window.key} className="flex items-center gap-2">
                <span className="w-[76px] shrink-0 truncate text-[10px] text-notch-muted">
                  {window.label}
                </span>

                <span className="relative h-[3px] min-w-0 flex-1 overflow-hidden rounded-full bg-white/8">
                  {window.usedPct !== null && (
                    <span
                      className="absolute inset-y-0 left-0 rounded-full transition-[width] duration-500"
                      style={{
                        width: `${Math.min(window.usedPct, 100)}%`,
                        background: window.informational
                          ? "var(--color-notch-muted)"
                          : window.usedPct >= 90
                            ? "var(--color-ring-danger)"
                            : window.usedPct >= 70
                              ? "var(--color-ring-warn)"
                              : "var(--accent)",
                      }}
                    />
                  )}
                </span>

                <span
                  className="tnum w-[52px] shrink-0 text-right text-[10px] font-medium"
                  title={window.estimated ? "estimated from local data" : undefined}
                >
                  {windowValue(window)}
                </span>
                <span className="tnum w-[48px] shrink-0 text-right text-[9px] text-notch-faint">
                  {reset ?? ""}
                </span>
              </div>
            );
          })}
        </div>
      )}

      {/* Why there are no numbers, when there are none. */}
      {provider.windows.length === 0 && provider.detail && (
        <p className="mt-1 pl-1 text-[10px] leading-snug text-notch-faint">
          {provider.detail}
        </p>
      )}

      {/* Sessions */}
      {provider.sessions.length > 0 && (
        <div className="mt-1.5 flex flex-col gap-0.5 pl-1">
          {provider.sessions.slice(0, 4).map((session) => {
            const ago = formatAgo(session.lastActivity);
            return (
              <button
                key={session.id}
                type="button"
                disabled={!clickable}
                onClick={() => onFocus(provider, session.cwd ?? session.title)}
                className={`flex items-center gap-1.5 rounded px-1 py-[3px] text-left transition-colors ${
                  clickable ? "hover:bg-white/6 cursor-pointer" : "cursor-default"
                }`}
              >
                <span
                  className={`size-[5px] shrink-0 rounded-full ${
                    session.activity === "awaitingInput" ? "activity-pulse" : ""
                  }`}
                  style={{
                    background:
                      session.activity === "awaitingInput"
                        ? "var(--color-state-wait)"
                        : session.activity === "generating"
                          ? "var(--color-state-busy)"
                          : session.activity === "done"
                            ? "var(--color-state-done)"
                            : "var(--color-state-off)",
                  }}
                />
                <span className="min-w-0 flex-1 truncate text-[10px] text-notch-muted">
                  {session.title}
                </span>
                <span className="tnum shrink-0 text-[9px] text-notch-faint">
                  {session.detail ?? ago ?? ""}
                </span>
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
}
