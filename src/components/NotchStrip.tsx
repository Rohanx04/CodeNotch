/**
 * The notch itself: a black strip hugging one screen edge with a ring per
 * provider.
 *
 * The strip is flush against the edge and tapers away at both ends along the
 * curve in NotchShape — the join that makes it read as carved out of the
 * display rather than a panel floating on top of it.
 *
 * Its position and the auto-hide slide into the edge are driven from App's
 * animation loop through `innerRef`, not from props, so they run at frame rate
 * without re-rendering the rings.
 */

import { useLayoutEffect, useRef, useState } from "react";
import { Settings2 } from "lucide-react";

import type { Edge, HudMetrics, LiveSession, ProviderSnapshot } from "../types";
import { NotchShape } from "./NotchShape";
import { ProviderRing } from "./ProviderRing";

interface Props {
  providers: ProviderSnapshot[];
  live: LiveSession[];
  metrics: HudMetrics;
  edge: Edge;
  activeId: string | null;
  paused: boolean;
  onHover: (provider: ProviderSnapshot | null) => void;
  onActivate: (provider: ProviderSnapshot) => void;
  onOpenSettings: () => void;
  /** Close whatever card is open without letting the notch collapse. */
  onDismissCard: () => void;
  settingsOpen: boolean;
  innerRef?: React.Ref<HTMLDivElement>;
}

/** Stagger between rings in the launch sweep. */
const INTRO_STAGGER_MS = 70;
/** Let the strip slide out before the first arc starts filling. */
const INTRO_START_MS = 220;

export function NotchStrip({
  providers,
  live,
  metrics,
  edge,
  activeId,
  paused,
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

  // Everything inside the strip is sized from the metrics rather than in fixed
  // pixels, so the small/medium/large strips stay the same object at three
  // scales. `gearSize` mirrors `HudMetrics::settings_extent` in Rust, which is
  // what reserves room for the gear when the window is sized.
  const gearSize = Math.round(metrics.stripThickness * 0.55);
  const ringGap = Math.max(2, Math.round(metrics.ring * 0.16));
  const claudeFailed = live.some((s) => s.failed && s.activity === "done");

  // The silhouette is drawn at exact pixel size, so it has to follow the strip
  // as rings come and go. `offset*`, not the bounding rect: the strip is often
  // mid-slide, and the rect would include the transform.
  useLayoutEffect(() => {
    const node = shellRef.current;
    if (!node) return;
    const measure = () => {
      setShell({ width: Math.ceil(node.offsetWidth), height: Math.ceil(node.offsetHeight) });
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
      data-paused={paused || undefined}
      style={
        {
          [vertical ? "width" : "height"]: metrics.stripThickness,
          padding: vertical
            ? `${metrics.stripPadding}px 0`
            : `0 ${metrics.stripPadding}px`,
          "--ring-gap": `${ringGap}px`,
          "--gear-size": `${gearSize}px`,
        } as React.CSSProperties
      }
    >
      <NotchShape
        className="notch-silhouette"
        width={shell.width}
        height={shell.height}
        edge={edge}
      />

      <div
        className="notch-rings"
        style={{ flexDirection: vertical ? "column" : "row" }}
      >
        {providers.map((provider, index) => (
          <span
            key={provider.id}
            className="ring-cell"
            style={{ [vertical ? "height" : "width"]: metrics.slot }}
          >
            <ProviderRing
              provider={provider}
              size={metrics.ring}
              edge={edge}
              active={activeId === provider.id}
              failed={provider.id === "claudeCode" && claudeFailed}
              introDelay={INTRO_START_MS + index * INTRO_STAGGER_MS}
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
