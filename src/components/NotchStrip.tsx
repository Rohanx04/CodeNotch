/**
 * The notch itself: a black strip hugging one screen edge with a ring per
 * provider.
 *
 * The strip is flush against the edge and its two free corners are rounded,
 * while small concave fillets flare out where it meets the edge — the join that
 * makes it read as carved out of the display rather than a panel floating on
 * top of it.
 */

import { Settings2 } from "lucide-react";

import type { Edge, HudMetrics, ProviderSnapshot } from "../types";
import { ProviderRing } from "./ProviderRing";

interface Props {
  providers: ProviderSnapshot[];
  metrics: HudMetrics;
  edge: Edge;
  activeId: string | null;
  onHover: (provider: ProviderSnapshot | null) => void;
  onActivate: (provider: ProviderSnapshot) => void;
  onOpenSettings: () => void;
  /** Close whatever card is open without letting the notch collapse. */
  onDismissCard: () => void;
  settingsOpen: boolean;
  innerRef?: React.Ref<HTMLDivElement>;
}

export function NotchStrip({
  providers,
  metrics,
  edge,
  activeId,
  onHover,
  onActivate,
  onOpenSettings,
  onDismissCard,
  settingsOpen,
  innerRef,
}: Props) {
  const vertical = edge === "left" || edge === "right";

  return (
    <div
      ref={innerRef}
      className="notch-strip"
      data-edge={edge}
      style={{
        [vertical ? "width" : "height"]: metrics.stripThickness,
        padding: vertical
          ? `${metrics.stripPadding}px 0`
          : `0 ${metrics.stripPadding}px`,
      }}
    >
      {/* The concave joins to the screen edge, above and below the strip. */}
      <span className="notch-fillet" data-end="start" aria-hidden />
      <span className="notch-fillet" data-end="end" aria-hidden />

      <div
        className="notch-rings"
        style={{ flexDirection: vertical ? "column" : "row" }}
      >
        {providers.map((provider) => (
          <span
            key={provider.id}
            className="ring-cell"
            style={{ [vertical ? "height" : "width"]: metrics.slot }}
          >
            <ProviderRing
              provider={provider}
              size={metrics.ring}
              active={activeId === provider.id}
              onHover={onHover}
              onActivate={onActivate}
            />
          </span>
        ))}

        {/* The strip is the whole UI, so settings live on it too. */}
        <button
          type="button"
          className="strip-settings"
          onClick={onOpenSettings}
          onMouseEnter={onDismissCard}
          aria-label="CodeNotch settings"
          data-active={settingsOpen || undefined}
        >
          <Settings2 aria-hidden />
        </button>
      </div>
    </div>
  );
}
