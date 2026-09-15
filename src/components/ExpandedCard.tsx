/**
 * The hover state: the full readout. Header, one row per provider, footer.
 */

import { Pin, PinOff, RefreshCw, Settings2 } from "lucide-react";

import type { Config, ProviderSnapshot, Telemetry } from "../types";
import { formatAgo, formatPct } from "../lib/format";
import { ProviderRow } from "./ProviderRow";
import { Ring } from "./Ring";

interface Props {
  telemetry: Telemetry;
  config: Config;
  pinned: boolean;
  refreshing: boolean;
  onTogglePin: () => void;
  onRefresh: () => void;
  onOpenSettings: () => void;
  onFocusProvider: (provider: ProviderSnapshot, titleHint?: string | null) => void;
}

export function ExpandedCard({
  telemetry,
  config,
  pinned,
  refreshing,
  onTogglePin,
  onRefresh,
  onOpenSettings,
  onFocusProvider,
}: Props) {
  // Providers that aren't installed are noise in the full card too, but if
  // nothing at all is installed we show them so the user isn't staring at a
  // blank panel wondering whether the app is broken.
  const installed = telemetry.providers.filter((p) => p.health !== "unavailable");
  const rows = installed.length > 0 ? installed : telemetry.providers;

  return (
    <div className="flex flex-col">
      <header className="flex items-center gap-2 border-b border-white/8 px-2.5 py-2">
        <Ring
          pct={telemetry.peakPct}
          activity={telemetry.activity}
          health={telemetry.health}
          size={20}
        />
        <div className="flex min-w-0 flex-1 flex-col">
          <span className="text-[12px] leading-tight font-semibold">
            {formatPct(telemetry.peakPct)}{" "}
            <span className="font-normal text-notch-muted">of nearest limit</span>
          </span>
          <span className="tnum truncate text-[9px] leading-tight text-notch-faint">
            updated {formatAgo(telemetry.generatedAt) ?? "just now"}
          </span>
        </div>

        <button
          type="button"
          onClick={onRefresh}
          aria-label="Refresh now"
          title="Refresh now"
          className="cursor-pointer rounded p-1 text-notch-muted transition-colors hover:bg-white/8 hover:text-notch-text"
        >
          <RefreshCw className={`size-3 ${refreshing ? "animate-spin" : ""}`} />
        </button>
        <button
          type="button"
          onClick={onTogglePin}
          aria-label={pinned ? "Unpin" : "Keep expanded"}
          title={pinned ? "Unpin" : "Keep expanded"}
          className={`cursor-pointer rounded p-1 transition-colors hover:bg-white/8 ${
            pinned ? "text-[var(--accent)]" : "text-notch-muted hover:text-notch-text"
          }`}
        >
          {pinned ? <Pin className="size-3" /> : <PinOff className="size-3" />}
        </button>
        <button
          type="button"
          onClick={onOpenSettings}
          aria-label="Settings"
          title="Settings"
          className="cursor-pointer rounded p-1 text-notch-muted transition-colors hover:bg-white/8 hover:text-notch-text"
        >
          <Settings2 className="size-3" />
        </button>
      </header>

      <div className="thin-scroll max-h-[540px] divide-y divide-white/6 overflow-y-auto">
        {rows.length === 0 ? (
          <p className="px-3 py-6 text-center text-[11px] text-notch-faint">
            No providers detected yet.
          </p>
        ) : (
          rows.map((provider) => (
            <ProviderRow
              key={provider.id}
              provider={provider}
              config={config}
              onFocus={onFocusProvider}
            />
          ))
        )}
      </div>
    </div>
  );
}
