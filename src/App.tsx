/**
 * The HUD shell.
 *
 * Two responsibilities beyond rendering:
 *
 * 1. **Hover intent.** Entering expands immediately; leaving collapses after a
 *    short grace period, so crossing the pill on the way somewhere else doesn't
 *    make it flap open and shut.
 * 2. **Measurement.** The Tauri window is sized to the content, so the expanded
 *    card measures itself and reports its height back to Rust, which resizes the
 *    window without ever taking focus.
 */

import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";

import { CollapsedPill } from "./components/CollapsedPill";
import { ExpandedCard } from "./components/ExpandedCard";
import { SettingsPanel } from "./components/SettingsPanel";
import { DEMO_BOOTSTRAP } from "./lib/demo";
import { events, IN_TAURI, ipc } from "./lib/ipc";
import type {
  Config,
  HudState,
  MonitorInfo,
  ProviderSnapshot,
  Telemetry,
} from "./types";

/** Grace period before collapsing, so a passing pointer doesn't toggle it. */
const COLLAPSE_DELAY_MS = 180;

export default function App() {
  const [config, setConfig] = useState<Config>(DEMO_BOOTSTRAP.config);
  const [telemetry, setTelemetry] = useState<Telemetry>(
    IN_TAURI
      ? { ...DEMO_BOOTSTRAP.telemetry, providers: [] }
      : DEMO_BOOTSTRAP.telemetry,
  );
  const [hud, setHud] = useState<HudState>(DEMO_BOOTSTRAP.hud);
  const [version, setVersion] = useState(DEMO_BOOTSTRAP.version);
  const [monitors, setMonitors] = useState<MonitorInfo[]>([]);
  const [showSettings, setShowSettings] = useState(false);
  const [refreshing, setRefreshing] = useState(false);
  /** Local mirror of hover, so the UI responds before Rust answers. */
  const [hovering, setHovering] = useState(false);

  const collapseTimer = useRef<number | null>(null);
  const contentRef = useRef<HTMLDivElement | null>(null);

  // --- Bootstrap and subscriptions ---------------------------------------

  useEffect(() => {
    let cancelled = false;

    ipc.ready().then((bootstrap) => {
      if (cancelled || !bootstrap) return;
      setConfig(bootstrap.config);
      setTelemetry(bootstrap.telemetry);
      setHud(bootstrap.hud);
      setVersion(bootstrap.version);
    });

    const unsubscribers = [
      events.telemetry(setTelemetry),
      events.config(setConfig),
      events.hudState(setHud),
    ];

    return () => {
      cancelled = true;
      unsubscribers.forEach((p) => p.then((off) => off()));
    };
  }, []);

  // Apply the accent colour to the CSS custom property everything reads from.
  useEffect(() => {
    document.documentElement.style.setProperty("--accent", config.accent);
  }, [config.accent]);

  // --- Hover intent -------------------------------------------------------

  const clearCollapseTimer = () => {
    if (collapseTimer.current !== null) {
      window.clearTimeout(collapseTimer.current);
      collapseTimer.current = null;
    }
  };

  const handleEnter = useCallback(() => {
    clearCollapseTimer();
    setHovering(true);
    void ipc.hover(true);
  }, []);

  const handleLeave = useCallback(() => {
    clearCollapseTimer();
    collapseTimer.current = window.setTimeout(() => {
      setHovering(false);
      setShowSettings(false);
      void ipc.hover(false);
    }, COLLAPSE_DELAY_MS);
  }, []);

  useEffect(() => clearCollapseTimer, []);

  // The window can expand for reasons other than hover (a peek, a pin), and the
  // pointer may already be elsewhere; trust the backend for the real state.
  const expanded = hud.expanded || hovering || config.alwaysExpanded;

  // --- Content measurement ------------------------------------------------

  useLayoutEffect(() => {
    const element = contentRef.current;
    if (!element || !expanded) return;

    const report = () => {
      // Round up: a fractional height would leave a hairline of transparent
      // window below the card.
      const height = Math.ceil(element.getBoundingClientRect().height);
      if (height > 0) void ipc.setContentHeight(height);
    };

    report();
    const observer = new ResizeObserver(report);
    observer.observe(element);
    return () => observer.disconnect();
  }, [expanded, showSettings, telemetry, config.size]);

  // Monitors are only needed by the settings panel, and only change rarely.
  useEffect(() => {
    if (!showSettings) return;
    ipc.listMonitors().then((list) => list && setMonitors(list));
  }, [showSettings]);

  // --- Actions ------------------------------------------------------------

  const handleConfigChange = useCallback((next: Config) => {
    // Optimistic: the panel should feel instant, and the backend echoes back.
    setConfig(next);
    void ipc.setConfig(next);
  }, []);

  const handleRefresh = useCallback(async () => {
    setRefreshing(true);
    const fresh = await ipc.refreshNow();
    if (fresh) setTelemetry(fresh);
    setRefreshing(false);
  }, []);

  const handleTogglePin = useCallback(async () => {
    const pinned = await ipc.togglePin();
    if (pinned !== null) setHud((prev) => ({ ...prev, pinned }));
  }, []);

  const handleFocusProvider = useCallback(
    (provider: ProviderSnapshot, titleHint?: string | null) => {
      void ipc.focusProvider(provider.id, titleHint ?? null);
    },
    [],
  );

  // --- Render -------------------------------------------------------------

  return (
    <div
      className="h-full w-full"
      onMouseEnter={handleEnter}
      onMouseLeave={handleLeave}
    >
      {expanded ? (
        <div
          ref={contentRef}
          className="notch-surface reveal overflow-hidden rounded-xl"
        >
          {showSettings ? (
            <SettingsPanel
              config={config}
              monitors={monitors}
              version={version}
              onChange={handleConfigChange}
              onClose={() => setShowSettings(false)}
              onOpenConfigDir={() => void ipc.openConfigDir()}
              onQuit={() => void ipc.quit()}
            />
          ) : (
            <ExpandedCard
              telemetry={telemetry}
              config={config}
              pinned={hud.pinned}
              refreshing={refreshing}
              onTogglePin={handleTogglePin}
              onRefresh={handleRefresh}
              onOpenSettings={() => setShowSettings(true)}
              onFocusProvider={handleFocusProvider}
            />
          )}
        </div>
      ) : (
        <CollapsedPill telemetry={telemetry} />
      )}
    </div>
  );
}
