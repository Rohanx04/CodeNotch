/**
 * The HUD shell.
 *
 * The window is a fixed panel (Rust sizes it; see `hud.rs`) holding the strip
 * against the screen edge and room for a card beside it. Because the window
 * never changes size as cards come and go, everything here is free to move:
 *
 * 1. **Which card.** The hovered ring's card; a permission request; the
 *    provider a peek is about; or, when pinned open, the most urgent one.
 * 2. **Motion.** One animation loop owns the card and the strip. The card
 *    springs open out of the strip and closes on a fixed curve with no
 *    overshoot; its height springs when its content changes; it glides along
 *    the edge between rings with the tail tracking it; the strip slides into
 *    the edge for auto-hide and back out on launch. Content changes crossfade.
 *    The loop stops itself as soon as nothing is moving.
 * 3. **Hit rects.** The loop reports the shapes it painted, and Rust makes the
 *    window click-through everywhere else.
 */

import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";

import { ApprovalCard } from "./components/ApprovalCard";
import { NotchStrip } from "./components/NotchStrip";
import { SettingsPanel } from "./components/SettingsPanel";
import { CardTail, UsagePopover } from "./components/UsagePopover";
import { DEMO_APPROVAL, DEMO_BOOTSTRAP, DEMO_STEPS } from "./lib/demo";
import { peakOf } from "./lib/format";
import { events, IN_TAURI, ipc } from "./lib/ipc";
import { HIT_MARGIN, MAX_POPOVER_LENGTH, panelSize, stripOffset } from "./lib/layout";
import { clamp, prefersReducedMotion, Spring, Tracked } from "./lib/motion";
import { Sound } from "./lib/sound";
import type {
  Activity,
  ApprovalRequest,
  Config,
  Edge,
  HitRect,
  HudMetrics,
  HudState,
  MonitorInfo,
  ProviderSnapshot,
  Telemetry,
} from "./types";

/** How far the card travels out of the strip as it opens. */
const ENTER_TRAVEL = 8;
/** How long a leaving view lingers while the next one fades in. */
const CROSSFADE_MS = 200;

type CardView =
  | { kind: "provider"; id: string }
  | { kind: "approval" }
  | { kind: "settings" };

const viewKey = (v: CardView | null) =>
  v === null ? null : v.kind === "provider" ? `p:${v.id}` : v.kind;

const RANK: Record<Activity, number> = { awaitingInput: 3, generating: 2, done: 1, idle: 0 };

/** The provider most worth showing when nothing in particular was asked for. */
function mostUrgent(rings: ProviderSnapshot[]): ProviderSnapshot | null {
  return (
    [...rings].sort(
      (a, b) =>
        RANK[b.activity] - RANK[a.activity] || (peakOf(b) ?? -1) - (peakOf(a) ?? -1),
    )[0] ?? null
  );
}

function hitTest(rects: HitRect[], x: number, y: number): boolean {
  return rects.some(
    (r) =>
      r.w > 0 &&
      r.h > 0 &&
      x >= r.x - HIT_MARGIN &&
      x <= r.x + r.w + HIT_MARGIN &&
      y >= r.y - HIT_MARGIN &&
      y <= r.y + r.h + HIT_MARGIN,
  );
}

const sameRects = (a: HitRect[], b: HitRect[]) =>
  a.length === b.length &&
  a.every(
    (r, i) =>
      Math.abs(r.x - b[i].x) < 0.5 &&
      Math.abs(r.y - b[i].y) < 0.5 &&
      Math.abs(r.w - b[i].w) < 0.5 &&
      Math.abs(r.h - b[i].h) < 0.5,
  );

/** The browser preview has no backend; these URL switches stand in for it. */
const PREVIEW = new URLSearchParams(IN_TAURI ? "" : window.location.search);

export default function App() {
  const [config, setConfig] = useState<Config>(DEMO_BOOTSTRAP.config);
  const [telemetry, setTelemetry] = useState<Telemetry>(
    IN_TAURI ? { ...DEMO_BOOTSTRAP.telemetry, providers: [], live: [] } : DEMO_BOOTSTRAP.telemetry,
  );
  const [hud, setHud] = useState<HudState>(DEMO_BOOTSTRAP.hud);
  const [metrics, setMetrics] = useState<HudMetrics>(DEMO_BOOTSTRAP.metrics);
  const [edge, setEdge] = useState<Edge>(DEMO_BOOTSTRAP.edge);
  const [version, setVersion] = useState(DEMO_BOOTSTRAP.version);
  const [native, setNative] = useState(false);
  const [monitors, setMonitors] = useState<MonitorInfo[]>([]);
  const [approval, setApproval] = useState<ApprovalRequest | null>(null);
  const [showSettings, setShowSettings] = useState(false);
  const [activeId, setActiveId] = useState<string | null>(null);

  const vertical = edge === "left" || edge === "right";
  const reduced = useMemo(prefersReducedMotion, []);

  // Providers that have nothing to say don't earn a ring.
  const rings = useMemo(
    () => telemetry.providers.filter((p) => p.health !== "unavailable"),
    [telemetry],
  );

  // Rust sizes the window and places the strip in it; the browser preview
  // works the same numbers out for itself.
  const geometry = useMemo(() => {
    if (IN_TAURI) {
      return { width: hud.width, height: hud.height, stripX: hud.stripX, stripY: hud.stripY };
    }
    const panel = panelSize(metrics, edge, rings.length);
    const strip = stripOffset(metrics, edge, rings.length);
    return { ...panel, stripX: strip.x, stripY: strip.y };
  }, [hud.width, hud.height, hud.stripX, hud.stripY, metrics, edge, rings.length]);

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
      setNative(bootstrap.nativeWindow);
      setApproval(bootstrap.approval);
    });

    const unsubscribers = [
      events.telemetry(setTelemetry),
      events.config((next) => {
        setConfig(next);
        setEdge(next.edge);
      }),
      events.hudState(setHud),
      events.approval((request) => {
        setApproval(request);
        // A permission request takes the card, so it is on screen within the
        // backend's short "is anyone there" window.
        if (request) {
          setActiveId(null);
          setShowSettings(false);
        }
      }),
      events.cue((cue) => Sound.play(cue)),
    ];

    return () => {
      cancelled = true;
      unsubscribers.forEach((p) => p.then((off) => off()));
    };
  }, []);

  useEffect(() => {
    document.documentElement.style.setProperty("--accent", config.accent);
  }, [config.accent]);

  useEffect(() => {
    Sound.configure(config.sound && !hud.paused, config.soundVolume);
  }, [config.sound, config.soundVolume, hud.paused]);

  useEffect(() => {
    if (!showSettings) return;
    ipc.listMonitors().then((list) => list && setMonitors(list));
  }, [showSettings]);

  // --- The browser preview's stand-ins for the backend ----------------------

  const simulateHover = useCallback(
    (hovering: boolean) => {
      setHud((h) => {
        if (h.hovering === hovering) return h;
        const peeking = h.peeking && (h.peekEndsAt ?? 0) > Date.now() && hovering;
        return {
          ...h,
          hovering,
          peeking: hovering ? h.peeking : false,
          peekEndsAt: hovering ? h.peekEndsAt : null,
          focus: h.alert ? h.focus : peeking ? h.focus : null,
          open: hovering || h.alert || h.pinned || config.alwaysExpanded,
        };
      });
    },
    [config.alwaysExpanded],
  );

  useEffect(() => {
    if (IN_TAURI) return;
    const timers: number[] = [];
    // `?approval`: a permission request arrives after a moment.
    if (PREVIEW.has("approval")) {
      timers.push(
        window.setTimeout(() => {
          setApproval({ ...DEMO_APPROVAL, expiresAt: Date.now() + 108_000 });
          setHud((h) => ({ ...h, alert: true, open: true, focus: "claudeCode" }));
          Sound.play("approval");
        }, 900),
      );
    }
    // `?peek`: an attention peek with its countdown.
    if (PREVIEW.has("peek")) {
      timers.push(
        window.setTimeout(() => {
          const ends = Date.now() + config.peekSecs * 1000;
          setHud((h) => ({ ...h, open: true, peeking: true, peekEndsAt: ends, focus: "claudeCode" }));
          Sound.play("attention");
          timers.push(
            window.setTimeout(
              () =>
                setHud((h) =>
                  h.peeking ? { ...h, peeking: false, peekEndsAt: null, focus: null, open: h.hovering } : h,
                ),
              config.peekSecs * 1000,
            ),
          );
        }, 900),
      );
    }
    // `?hide`: auto-hide tucks the strip away, and a click brings it back.
    if (PREVIEW.has("hide")) {
      timers.push(window.setTimeout(() => setHud((h) => ({ ...h, retracted: true })), 1500));
    }
    // `?gestures`: Copilot works, finishes, then waits on you, and Gemini
    // breaks and recovers, so every ring reaction can be watched.
    if (PREVIEW.has("gestures")) {
      const cycle: Activity[] = ["generating", "done", "awaitingInput", "idle"];
      let beat = 0;
      const timer = window.setInterval(() => {
        const activity = cycle[beat % cycle.length];
        const broken = beat % 2 === 0;
        beat += 1;
        setTelemetry((t) => ({
          ...t,
          providers: t.providers.map((p) =>
            p.id === "copilot"
              ? { ...p, activity }
              : p.id === "gemini"
                ? { ...p, health: broken ? "error" : "ok" }
                : p,
          ),
        }));
      }, 1600);
      timers.push(timer);
    }
    // The live ticker keeps rolling so it can be seen.
    let step = 0;
    const ticker = window.setInterval(() => {
      setTelemetry((t) => ({
        ...t,
        live: t.live.map((s, i) =>
          i === 0
            ? {
                ...s,
                steps: [...s.steps, DEMO_STEPS[step++ % DEMO_STEPS.length]].slice(-12),
                stepCount: s.stepCount + 1,
                lastEvent: new Date().toISOString(),
              }
            : s,
        ),
      }));
    }, 2600);
    return () => {
      timers.forEach((t) => {
        window.clearTimeout(t);
        window.clearInterval(t);
      });
      window.clearInterval(ticker);
    };
    // Preview wiring runs once; the peek length is read when it fires.
  }, []);

  // --- Motion state ---------------------------------------------------------

  const containerRef = useRef<HTMLDivElement | null>(null);
  const stripWrapRef = useRef<HTMLDivElement | null>(null);
  const stripRef = useRef<HTMLDivElement | null>(null);
  const cardRef = useRef<HTMLDivElement | null>(null);
  const shellRef = useRef<HTMLDivElement | null>(null);
  const layerRef = useRef<HTMLDivElement | null>(null);
  const countdownRef = useRef<HTMLDivElement | null>(null);

  const motion = useRef({
    presence: new Tracked(0),
    height: new Spring(0, 0.42, 0.82),
    anchor: new Spring(0, 0.42, 0.86),
    retract: new Tracked(1),
    rects: [] as HitRect[],
    running: false,
    last: 0,
  });
  // Values the loop reads, kept current without restarting it.
  const live = useRef({ hud, metrics, edge, vertical, config, onClosed: () => {} });
  live.current = {
    hud,
    metrics,
    edge,
    vertical,
    config,
    onClosed: () => {
      setShown(null);
      setLeaving(null);
      setActiveId(null);
      setShowSettings(false);
    },
  };

  // --- Which card ---------------------------------------------------------

  const wanted: CardView | null = useMemo(() => {
    if (!hud.open || hud.retracted) return null;
    if (showSettings) return { kind: "settings" };
    if (activeId) {
      if (approval && activeId === "claudeCode") return { kind: "approval" };
      if (rings.some((p) => p.id === activeId)) return { kind: "provider", id: activeId };
    }
    if (approval) return { kind: "approval" };
    if (hud.focus && rings.some((p) => p.id === hud.focus)) {
      return { kind: "provider", id: hud.focus };
    }
    if (hud.pinned || config.alwaysExpanded) {
      const urgent = mostUrgent(rings);
      return urgent ? { kind: "provider", id: urgent.id } : null;
    }
    // On the strip between rings: wait for a ring rather than guess.
    return null;
  }, [hud.open, hud.retracted, hud.focus, hud.pinned, showSettings, activeId, approval, rings, config.alwaysExpanded]);

  // The card on screen, which outlives `wanted` while it animates closed.
  const [shown, setShown] = useState<CardView | null>(null);
  const [leaving, setLeaving] = useState<{ view: CardView; quick: boolean } | null>(null);
  const [entry, setEntry] = useState<"none" | "view" | "quick">("none");
  const shownRef = useRef<CardView | null>(null);
  shownRef.current = shown;
  const lastApproval = useRef<ApprovalRequest | null>(null);
  if (approval) lastApproval.current = approval;

  const wantedKey = viewKey(wanted);
  const shownKey = viewKey(shown);
  useEffect(() => {
    if (!wanted || wantedKey === shownKey) return;
    if (shown && motion.current.presence.target > 0) {
      // Switching content on an open card: crossfade. Provider to provider is
      // a glide along the strip, so it gets the quick fade.
      const quick = shown.kind === "provider" && wanted.kind === "provider";
      setLeaving({ view: shown, quick });
      setEntry(quick ? "quick" : "view");
      const timer = window.setTimeout(() => setLeaving(null), CROSSFADE_MS);
      setShown(wanted);
      return () => window.clearTimeout(timer);
    }
    setLeaving(null);
    setEntry("none");
    setShown(wanted);
    // `shown` is read, not tracked: only a change of what is wanted matters.
  }, [wantedKey]);

  // --- The animation loop ---------------------------------------------------

  const frame = useCallback((now: number) => {
    const m = motion.current;
    const { hud, metrics, edge, vertical, onClosed } = live.current;
    const dt = Math.min(0.05, (now - m.last) / 1000);
    m.last = now;

    m.presence.step(dt, now);
    m.height.step(dt);
    m.anchor.step(dt);
    m.retract.step(dt, now);

    const container = containerRef.current;
    const card = cardRef.current;
    const shell = shellRef.current;
    const stripWrap = stripWrapRef.current;

    // The strip, sliding into or out of its edge.
    if (stripWrap) {
      const out = m.retract.value * (metrics.stripThickness + 6);
      const [sx, sy] =
        edge === "right" ? [out, 0] : edge === "left" ? [-out, 0] : edge === "top" ? [0, -out] : [0, out];
      stripWrap.style.transform = `translate3d(${sx}px, ${sy}px, 0)`;
    }

    // The card: out of the strip, sized to its content, beside its ring.
    const p = m.presence.value;
    if (card && shell && container) {
      const panelAlong = vertical ? container.offsetHeight : container.offsetWidth;
      const height = m.height.value;
      const along = vertical ? height : metrics.popoverSize;
      const start = clamp(m.anchor.value - along / 2, 0, Math.max(0, panelAlong - along));
      const inset = metrics.stripThickness * 0.4;
      const tail = clamp(m.anchor.value - start, inset, Math.max(inset, along - inset));

      shell.style.height = `${height}px`;
      card.style[vertical ? "top" : "left"] = `${start}px`;
      card.style.setProperty("--tail-offset", `${tail}px`);

      const travel = (1 - clamp(p, 0, 1)) * ENTER_TRAVEL;
      const [tx, ty] =
        edge === "right" ? [travel, 0] : edge === "left" ? [-travel, 0] : edge === "top" ? [0, -travel] : [0, travel];
      card.style.transform = `translate3d(${tx}px, ${ty}px, 0) scale(${0.96 + 0.04 * p})`;
      card.style.transformOrigin =
        edge === "right"
          ? `100% ${tail}px`
          : edge === "left"
            ? `0 ${tail}px`
            : edge === "top"
              ? `${tail}px 0`
              : `${tail}px 100%`;
      card.style.opacity = String(clamp(p, 0, 1));
      card.style.visibility = p < 0.01 && m.presence.target === 0 ? "hidden" : "visible";
    }

    // The peek countdown: a bar that shrinks through the last few seconds.
    let counting = false;
    const bar = countdownRef.current;
    if (bar) {
      const peekOnly = hud.peeking && !hud.hovering && !hud.alert && hud.peekEndsAt !== null;
      const windowMs = Math.min(10, live.current.config.peekSecs * 0.6) * 1000;
      const remaining = peekOnly ? (hud.peekEndsAt ?? 0) - Date.now() : 0;
      const fraction = remaining > 0 && remaining < windowMs ? remaining / windowMs : 0;
      bar.style.transform = `translateX(-50%) scaleX(${fraction})`;
      counting = peekOnly && remaining > 0;
    }

    // The shapes on screen, for Rust's click-through test.
    if (container) {
      const origin = container.getBoundingClientRect();
      const rects: HitRect[] = [];
      const strip = stripRef.current?.getBoundingClientRect();
      if (strip && m.retract.value < 0.9) {
        rects.push({ x: strip.left - origin.left, y: strip.top - origin.top, w: strip.width, h: strip.height });
      }
      if (card && p > 0.05) {
        const box = card.getBoundingClientRect();
        // Stretch the card's rect back across the gap to the strip, so moving
        // from a ring onto its card never crosses click-through air.
        const gap = metrics.popoverGap;
        let { left, top, right, bottom } = box;
        if (edge === "right") right += gap;
        else if (edge === "left") left -= gap;
        else if (edge === "top") top -= gap;
        else bottom += gap;
        rects.push({ x: left - origin.left, y: top - origin.top, w: right - left, h: bottom - top });
      }
      if (!sameRects(rects, m.rects)) {
        m.rects = rects;
        void ipc.setHitRects(rects);
      }
    }

    if (m.presence.target === 0 && !m.presence.animating && shownRef.current) {
      onClosed();
    }

    const busy =
      m.presence.animating ||
      !m.height.settled ||
      !m.anchor.settled ||
      m.retract.animating ||
      counting;
    if (busy) {
      requestAnimationFrame(frame);
    } else {
      m.running = false;
    }
  }, []);

  const kick = useCallback(() => {
    const m = motion.current;
    if (m.running) return;
    m.running = true;
    m.last = performance.now();
    requestAnimationFrame(frame);
  }, [frame]);

  // Open and close.
  const open = wanted !== null;
  useEffect(() => {
    const m = motion.current;
    const now = performance.now();
    if (open) {
      if (m.presence.target !== 1 && !hud.peeking && !hud.alert) Sound.play("open");
      if (reduced) m.presence.jump(1);
      else m.presence.springTo(1, 0.5, 0.72);
    } else {
      if (m.presence.target === 1 && !hud.peeking) Sound.play("close");
      if (reduced) m.presence.jump(0);
      else m.presence.curveTo(0, now);
    }
    kick();
    // Sounds read the peek state at the moment of the change only.
  }, [open, kick, reduced]);

  // Auto-hide: tuck the strip into the edge, or slide it back out. The first
  // run is the launch: the strip starts tucked away and slides out.
  useEffect(() => {
    const m = motion.current;
    if (hud.retracted) {
      if (reduced) m.retract.jump(1);
      else m.retract.curveTo(1, performance.now());
    } else if (reduced) {
      m.retract.jump(0);
    } else {
      m.retract.springTo(0, 0.55, 0.78);
    }
    kick();
  }, [hud.retracted, hud.wake, kick, reduced]);

  // The card's height follows its content, springing when the content changes.
  useLayoutEffect(() => {
    const layer = layerRef.current;
    if (!layer) return;
    const measure = () => {
      const m = motion.current;
      const target = Math.min(layer.offsetHeight, MAX_POPOVER_LENGTH);
      if (target <= 0) return;
      // Opening from nothing: arrive at full height; the entrance is the
      // scale, not a grow from zero.
      if (m.presence.value < 0.05 || reduced) m.height.jump(target);
      else m.height.target = target;
      kick();
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(layer);
    return () => observer.disconnect();
  }, [shownKey, kick, reduced]);

  // The card's position along the strip: centred on its ring (or the gear),
  // gliding when it moves to another one.
  useLayoutEffect(() => {
    const container = containerRef.current;
    const strip = stripRef.current;
    if (!container || !strip || !shown) return;
    const selector =
      shown.kind === "settings"
        ? ".strip-settings"
        : `[data-ring="${shown.kind === "approval" ? "claudeCode" : shown.id}"] .ring-disc`;
    const target = strip.querySelector<HTMLElement>(selector);
    if (!target) return;
    const origin = container.getBoundingClientRect();
    const box = target.getBoundingClientRect();
    const centre = vertical
      ? box.top + box.height / 2 - origin.top
      : box.left + box.width / 2 - origin.left;
    const m = motion.current;
    if (m.presence.value < 0.05 || reduced) m.anchor.jump(centre);
    else m.anchor.target = centre;
    kick();
  }, [shownKey, vertical, metrics, rings.length, geometry, kick, reduced]);

  // Anything that moves the layout needs a frame to re-report the hit rects.
  useEffect(kick, [kick, geometry, rings.length, metrics, edge, hud.peeking, hud.hovering]);

  // --- Hover where there is no native cursor poll ---------------------------

  useEffect(() => {
    if (native) return;
    let last: boolean | null = null;
    const onMove = (e: MouseEvent) => {
      const container = containerRef.current;
      if (!container) return;
      const origin = container.getBoundingClientRect();
      const inside = hitTest(motion.current.rects, e.clientX - origin.left, e.clientY - origin.top);
      if (inside === last) return;
      last = inside;
      if (IN_TAURI) void ipc.hover(inside);
      else simulateHover(inside);
    };
    window.addEventListener("mousemove", onMove);
    return () => window.removeEventListener("mousemove", onMove);
  }, [native, simulateHover]);

  // Escape closes the card when the notch has the keyboard (never a
  // permission request: that needs a real answer).
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || !hud.open || hud.alert) return;
      if (IN_TAURI) void ipc.dismiss();
      else setHud((h) => ({ ...h, open: false, peeking: false, focus: null }));
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [hud.open, hud.alert]);

  // --- Actions ------------------------------------------------------------

  const handleHover = useCallback((provider: ProviderSnapshot | null) => {
    if (!provider) return;
    setActiveId(provider.id);
    setShowSettings(false);
  }, []);

  const handleConfigChange = useCallback((next: Config) => {
    setConfig(next);
    setEdge(next.edge);
    void ipc.setConfig(next);
  }, []);

  const handleFocusProvider = useCallback(
    (provider: ProviderSnapshot, titleHint?: string | null) => {
      void ipc.focusProvider(provider.id, titleHint ?? null);
    },
    [],
  );

  const handleDecide = useCallback((decision: "allow" | "deny") => {
    const request = lastApproval.current;
    if (!request) return;
    Sound.play(decision);
    if (IN_TAURI) {
      void ipc.approvalDecide(request.id, decision);
    } else {
      setApproval(null);
      setHud((h) => ({ ...h, alert: false, focus: null, open: h.hovering }));
    }
  }, []);

  const handleApprovalShown = useCallback((id: string) => {
    void ipc.approvalAck(id);
  }, []);

  // --- Rendering ----------------------------------------------------------

  const renderView = (view: CardView) => {
    switch (view.kind) {
      case "settings":
        return (
          <SettingsPanel
            config={config}
            monitors={monitors}
            version={version}
            onChange={handleConfigChange}
            onClose={() => setShowSettings(false)}
            onOpenConfigDir={() => void ipc.openConfigDir()}
            onQuit={() => void ipc.quit()}
          />
        );
      case "approval": {
        const request = approval ?? lastApproval.current;
        return request ? (
          <ApprovalCard request={request} onDecide={handleDecide} onShown={handleApprovalShown} />
        ) : null;
      }
      case "provider": {
        const provider = rings.find((p) => p.id === view.id);
        return provider ? (
          <UsagePopover
            provider={provider}
            config={config}
            live={telemetry.live}
            onFocusProvider={handleFocusProvider}
          />
        ) : null;
      }
    }
  };

  if (hud.hidden) return null;

  // Tucked away: the window is a thin strip against the edge, and touching it
  // brings the notch back.
  if (hud.wake) {
    return (
      <div
        className="wake-strip"
        data-edge={edge}
        onMouseEnter={() => void ipc.wake()}
        aria-label="Show CodeNotch"
      />
    );
  }

  // In the app the container *is* the window. In the browser preview it is
  // docked to the viewport's edge the way the window is docked to the screen.
  const containerStyle: React.CSSProperties = IN_TAURI
    ? { left: 0, top: 0, width: geometry.width, height: geometry.height }
    : {
        width: geometry.width,
        height: geometry.height,
        ...(vertical
          ? { [edge]: 0, top: "50%", marginTop: -geometry.height / 2 }
          : { [edge]: 0, left: "50%", marginLeft: -geometry.width / 2 }),
      };

  const depth = metrics.stripThickness + metrics.popoverGap;
  const cardStyle: React.CSSProperties = { width: metrics.popoverSize, [edge]: depth };

  return (
    <div ref={containerRef} className="hud-panel" style={containerStyle}>
      <div
        ref={cardRef}
        className="notch-card"
        data-edge={edge}
        style={cardStyle}
        onMouseEnter={() => shown?.kind === "provider" && setActiveId(shown.id)}
      >
        <CardTail edge={edge} thickness={metrics.stripThickness} />
        <div
          ref={shellRef}
          className={`card-shell${shown?.kind === "settings" ? " card-shell-flush" : ""}`}
        >
          {leaving && (
            <div className={`card-layer card-leave${leaving.quick ? " quick" : ""}`} aria-hidden>
              {renderView(leaving.view)}
            </div>
          )}
          {shown && (
            <div
              key={shownKey}
              ref={layerRef}
              className={`card-layer ${entry === "none" ? "card-rise" : `card-enter-${entry}`}`}
            >
              {renderView(shown)}
            </div>
          )}
        </div>
        <div ref={countdownRef} className="card-countdown" aria-hidden />
      </div>

      <div
        ref={stripWrapRef}
        className="strip-wrap"
        style={{ left: geometry.stripX, top: geometry.stripY }}
      >
        <NotchStrip
          innerRef={stripRef}
          providers={rings}
          live={telemetry.live}
          metrics={metrics}
          edge={edge}
          activeId={shown?.kind === "provider" ? shown.id : shown?.kind === "approval" ? "claudeCode" : null}
          paused={hud.paused}
          onHover={handleHover}
          onActivate={handleFocusProvider}
          onOpenSettings={() => {
            setActiveId(null);
            setShowSettings((open) => !open);
          }}
          onDismissCard={() => setActiveId(null)}
          settingsOpen={shown?.kind === "settings"}
        />
      </div>
    </div>
  );
}
