/**
 * The notch itself: a black strip hugging one screen edge with a ring per
 * provider.
 *
 * The strip is flush against the edge and its two free corners are rounded,
 * while small concave fillets flare out where it meets the edge — the join that
 * makes it read as carved out of the display rather than a panel floating on
 * top of it.
 */

import { useLayoutEffect, useRef, useState } from "react";
import { Settings2 } from "lucide-react";

import type { Edge, HudMetrics, ProviderSnapshot } from "../types";
import { NotchShape } from "./NotchShape";
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
  const shellRef = useRef<HTMLDivElement | null>(null);
  const [shell, setShell] = useState({ width: 0, height: 0 });

  // The silhouette is drawn at exact pixel size, so it has to follow the strip
  // as rings come and go.
  useLayoutEffect(() => {
    const node = shellRef.current;
    if (!node) return;
    const measure = () => {
      const box = node.getBoundingClientRect();
      setShell({ width: Math.ceil(box.width), height: Math.ceil(box.height) });
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(node);
    return () => observer.disconnect();
  }, [vertical, metrics, providers.length]);

  return (
    <div
      ref={(node) => {
        shellRef.current = node;
        if (typeof innerRef === "function") innerRef(node);
        else if (innerRef) {
          (innerRef as React.RefObject<HTMLDivElement | null>).current = node;
        }
      }}
      className="notch-strip"
      data-edge={edge}
      style={{
        [vertical ? "width" : "height"]: metrics.stripThickness,
        padding: vertical
          ? `${metrics.stripPadding}px 0`
          : `0 ${metrics.stripPadding}px`,
      }}
    >
      <NotchShape
        className="notch-silhouette"
        width={shell.width}
        height={shell.height}
        scoop={metrics.stripPadding}
        edge={edge}
      />

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
