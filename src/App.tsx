/**
 * The HUD shell.
 *
 * The window is exactly the strip while resting, and grows inward to hold the
 * detail popover when a ring is hovered. Three responsibilities beyond
 * rendering:
 *
 * 1. **Anchoring.** The strip always hugs the screen edge, so on a right or
 *    bottom edge it sits at the far end of the window and the popover takes the
 *    inward side.
 * 2. **Hover intent.** Moving between rings switches the popover instantly;
 *    leaving closes it after a short grace period so crossing the notch on the
 *    way somewhere else doesn't make it flap.
 * 3. **Measurement.** The window is sized to its content, so the laid-out size
 *    goes back to Rust, which resizes without ever taking focus.
 */

import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";

import { NotchStrip } from "./components/NotchStrip";
import { SettingsPanel } from "./components/SettingsPanel";
import { UsagePopover } from "./components/UsagePopover";
import { DEMO_BOOTSTRAP } from "./lib/demo";
import { events, IN_TAURI, ipc } from "./lib/ipc";
import type {
  Config,
  Edge,
  HudMetrics,
  HudState,
  MonitorInfo,
  ProviderSnapshot,
  Telemetry,
} from "./types";

/** Grace period before closing, so a passing pointer doesn't toggle it. */
const CLOSE_DELAY_MS = 200;

export default function App() {
  const [config, setConfig] = useState<Config>(DEMO_BOOTSTRAP.config);
  const [telemetry, setTelemetry] = useState<Telemetry>(
    IN_TAURI
      ? { ...DEMO_BOOTSTRAP.telemetry, providers: [] }
      : DEMO_BOOTSTRAP.telemetry,
  );
  const [hud, setHud] = useState<HudState>(DEMO_BOOTSTRAP.hud);
  const [metrics, setMetrics] = useState<HudMetrics>(DEMO_BOOTSTRAP.metrics);
  const [edge, setEdge] = useState<Edge>(DEMO_BOOTSTRAP.edge);
  const [version, setVersion] = useState(DEMO_BOOTSTRAP.version);
  const [monitors, setMonitors] = useState<MonitorInfo[]>([]);
  const [showSettings, setShowSettings] = useState(false);
  const [activeId, setActiveId] = useState<string | null>(null);

  const closeTimer = useRef<number | null>(null);
  const stripRef = useRef<HTMLDivElement | null>(null);
  const popoverRef = useRef<HTMLDivElement | null>(null);
  const [tailAnchor, setTailAnchor] = useState<number | null>(null);
  const [popoverLength, setPopoverLength] = useState(0);

  // Providers that have nothing to say don't earn a ring.
  const rings = useMemo(
    () => telemetry.providers.filter((p) => p.health !== "unavailable"),
    [telemetry],
  );

  const vertical = edge === "left" || edge === "right";
  const active = rings.find((p) => p.id === activeId) ?? null;

  // --- Bootstrap and subscriptions ---------------------------------------

  useEffect(() => {
    let cancelled = false;

    ipc.ready().then((bootstrap) => {
      if (cancelled || !bootstrap) return;
      setConfig(bootstrap.config);
      setTelemetry(bootstrap.telemetry);
      setHud(bootstrap.hud);
      setMetrics(bootstrap.metrics);
      setEdge(bootstrap.edge);
      setVersion(bootstrap.version);
    });

    const unsubscribers = [
      events.telemetry(setTelemetry),
      events.config((next) => {
        setConfig(next);
        setEdge(next.edge);
      }),
      events.hudState(setHud),
    ];

    return () => {
      cancelled = true;
      unsubscribers.forEach((p) => p.then((off) => off()));
    };
  }, []);

  useEffect(() => {
    document.documentElement.style.setProperty("--accent", config.accent);
  }, [config.accent]);

  // The backend closed the notch (peek expired, cursor left): drop the popover
  // so the two sides don't disagree about what is on screen.
  useEffect(() => {
    if (!hud.open) {
      setActiveId(null);
      setShowSettings(false);
    }
  }, [hud.open]);

  // --- Hover intent -------------------------------------------------------

  const clearCloseTimer = () => {
    if (closeTimer.current !== null) {
      window.clearTimeout(closeTimer.current);
      closeTimer.current = null;
    }
  };

  const handleHover = useCallback((provider: ProviderSnapshot | null) => {
    clearCloseTimer();
    if (provider) {
      setActiveId(provider.id);
      setShowSettings(false);
      void ipc.hover(true);
      return;
    }
    closeTimer.current = window.setTimeout(() => {
      setActiveId(null);
      void ipc.hover(false);
    }, CLOSE_DELAY_MS);
  }, []);

  useEffect(() => clearCloseTimer, []);

  // --- Measurement --------------------------------------------------------

  // The window must be long enough for the strip and for a popover that may
  // overhang it, and deep enough for the popover when one is open.
  useLayoutEffect(() => {
    const strip = stripRef.current;
    if (!strip) return;

    const report = () => {
      const stripBox = strip.getBoundingClientRect();
      const popBox = popoverRef.current?.getBoundingClientRect();

      const stripLength = vertical ? stripBox.height : stripBox.width;
      const popLength = popBox ? (vertical ? popBox.height : popBox.width) : 0;
      // Round up: a fractional size leaves a hairline of transparent window.
      const length = Math.ceil(Math.max(stripLength, popLength));

      // Measure the depth too, rather than assuming `popoverSize`.
      //
      // The card is laid out `popoverSize` wide whichever edge it is on. On a
      // left/right edge that width *is* the depth, so the two agree. On a
      // top/bottom edge the depth is the card's height, which its content
      // decides -- so using `popoverSize` there sized the window from the
      // card's width, an unrelated axis. A short card left a slab of dead
      // window above it, and a tall one was cut off at the top.
      const popDepth = popBox ? (vertical ? popBox.width : popBox.height) : 0;
      const depth = Math.ceil(
        metrics.stripThickness + (popBox ? metrics.popoverGap + popDepth : 0),
      );

      setPopoverLength(popLength);

      if (length > 0) {
        void ipc.setContentSize(
          vertical ? depth : length,
          vertical ? length : depth,
        );
      }
    };

    report();
    const observer = new ResizeObserver(report);
    observer.observe(strip);
    if (popoverRef.current) observer.observe(popoverRef.current);
    return () => observer.disconnect();
  }, [vertical, metrics, rings.length, activeId, showSettings, telemetry]);

  useEffect(() => {
    if (!showSettings) return;
    ipc.listMonitors().then((list) => list && setMonitors(list));
  }, [showSettings]);

  // Point the tail at the hovered ring's disc. Measuring beats arithmetic here:
  // a slot also holds the percentage label, so its centre sits below the disc's.
  useLayoutEffect(() => {
    const strip = stripRef.current;
    if (!strip || !activeId) {
      setTailAnchor(null);
      return;
    }
    const disc = strip.querySelector<HTMLElement>("[data-active] .ring-disc");
    if (!disc) return;

    const discBox = disc.getBoundingClientRect();
    setTailAnchor(
      vertical
        ? discBox.top + discBox.height / 2
        : discBox.left + discBox.width / 2,
    );
  }, [activeId, vertical, metrics, rings.length, telemetry]);

  // --- Actions ------------------------------------------------------------

  const handleConfigChange = useCallback((next: Config) => {
    setConfig(next);
    void ipc.setConfig(next);
  }, []);

  const handleFocusProvider = useCallback(
    (provider: ProviderSnapshot, titleHint?: string | null) => {
      void ipc.focusProvider(provider.id, titleHint ?? null);
    },
    [],
  );

  // Clicking a ring raises that tool's window; the settings ring is the
  // exception, since there is nothing to raise.
  const handleActivate = useCallback(
    (provider: ProviderSnapshot) => handleFocusProvider(provider),
    [handleFocusProvider],
  );

  // --- Placement ----------------------------------------------------------

  /**
   * Centre the popover on the ring it points at, then keep it on screen.
   *
   * Clamping is what makes the tail worth having: for a ring near either end of
   * the strip the card has to slide back inside the window, so its centre no
   * longer lines up with the ring and only the tail says which one you are
   * reading.
   */
  const { popoverStart, tailOffset } = useMemo(() => {
    const windowLength = vertical ? window.innerHeight : window.innerWidth;
    const anchor = tailAnchor ?? windowLength / 2;

    if (popoverLength <= 0) {
      return { popoverStart: 0, tailOffset: anchor };
    }

    const limit = Math.max(0, windowLength - popoverLength);
    const start = Math.min(Math.max(anchor - popoverLength / 2, 0), limit);

    // Keep the whole tail off the card's rounded corners: its base is as tall
    // as TAIL_HALF either side of the anchor.
    const inset = metrics.stripThickness * 0.4;
    const offset = Math.min(
      Math.max(anchor - start, inset),
      Math.max(popoverLength - inset, inset),
    );
    return { popoverStart: start, tailOffset: offset };
  }, [vertical, tailAnchor, popoverLength, metrics, activeId]);

  // The strip hugs the edge; the popover fills the rest of the window.
  const stripStyle: React.CSSProperties = vertical
    ? { [edge === "right" ? "right" : "left"]: 0, top: 0, bottom: 0 }
    : { [edge === "bottom" ? "bottom" : "top"]: 0, left: 0, right: 0 };

  const popoverStyle: React.CSSProperties = vertical
    ? {
        [edge === "right" ? "right" : "left"]:
          metrics.stripThickness + metrics.popoverGap,
        width: metrics.popoverSize,
        top: popoverStart,
      }
    : {
        [edge === "bottom" ? "bottom" : "top"]:
          metrics.stripThickness + metrics.popoverGap,
        width: metrics.popoverSize,
        left: popoverStart,
      };

  return (
    <div className="relative h-full w-full" onMouseLeave={() => handleHover(null)}>
      {active && !showSettings && (
        <div ref={popoverRef} className="absolute reveal" style={popoverStyle}>
          <UsagePopover
            provider={active}
            config={config}
            edge={edge}
            anchor={tailOffset}
            thickness={metrics.stripThickness}
            onFocusProvider={handleFocusProvider}
          />
        </div>
      )}

      {showSettings && (
        <div ref={popoverRef} className="absolute reveal" style={popoverStyle}>
          <div className="popover popover-flush">
            <SettingsPanel
              config={config}
              monitors={monitors}
              version={version}
              onChange={handleConfigChange}
              onClose={() => setShowSettings(false)}
              onOpenConfigDir={() => void ipc.openConfigDir()}
              onQuit={() => void ipc.quit()}
            />
          </div>
        </div>
      )}

      <div className="absolute" style={stripStyle}>
        <NotchStrip
          innerRef={stripRef}
          providers={rings}
          metrics={metrics}
          edge={edge}
          activeId={activeId}
          onHover={handleHover}
          onActivate={handleActivate}
          onOpenSettings={() => {
            clearCloseTimer();
            setActiveId(null);
            setShowSettings((open) => !open);
            void ipc.hover(true);
          }}
          onDismissCard={() => {
            // Moving onto the gear closes the provider card, but the pointer is
            // still on the notch -- so don't tell the backend it left.
            clearCloseTimer();
            setActiveId(null);
          }}
          settingsOpen={showSettings}
        />
      </div>
    </div>
  );
}
