/**
 * What a live Claude Code session is doing, as a rolling three-line ticker.
 *
 * Three rows: the step just completed (dim, with a tick), the current step
 * (shimmering, with a chevron), and the next one waiting below. A new step
 * rolls everything up one line over 380 ms. Every row position comes from a
 * single clock, and new steps are *queued* rather than chained onto CSS
 * transitions, so a burst of steps scrolls past in order instead of two rows
 * landing on one line or a step vanishing mid-animation.
 *
 * Adapted from Coucou's TickerView (MIT, Copyright (c) 2026 Louis Raillé).
 */

import { useEffect, useRef } from "react";
import { Check, ChevronRight } from "lucide-react";

import { clamp, cubicBezier, lerp, prefersReducedMotion } from "../lib/motion";

const ROW_H = 17;
const DURATION = 380;
/** Beyond this many queued steps, skip ahead rather than scroll forever. */
const MAX_QUEUE = 4;
/** The completed row is set smaller than the current one. */
const COMPLETED_SCALE = 0.9;
const EASE = cubicBezier(0.4, 0, 0.2, 1);

interface Props {
  steps: string[];
  /** Steps seen in total; how the ticker knows which ones are new. */
  stepCount: number;
  /** Session id: a different session re-seeds rather than scrolls. */
  sessionId: string;
}

interface Row {
  el: HTMLDivElement;
  text: string;
}

/** 0 = current (shimmering, full size), 1 = completed (dim, smaller). */
function place(row: Row, y: number, phase: number, opacity: number) {
  const scale = 1 - phase * (1 - COMPLETED_SCALE);
  row.el.style.transform = `translate(${-phase * 6}px, ${y}px) scale(${scale})`;
  row.el.style.opacity = String(opacity);
  row.el.style.setProperty("--phase", String(phase));
}

function setText(row: Row, text: string) {
  if (row.text === text) return;
  row.text = text;
  row.el.querySelectorAll<HTMLElement>("[data-text]").forEach((n) => (n.textContent = text));
}

export function StepTicker({ steps, stepCount, sessionId }: Props) {
  const refs = [useRef<HTMLDivElement>(null), useRef<HTMLDivElement>(null), useRef<HTMLDivElement>(null)];
  const rows = useRef<Row[] | null>(null);
  const queue = useRef<string[]>([]);
  const shown = useRef<{ session: string; count: number } | null>(null);
  const start = useRef<number | null>(null);
  const frame = useRef<number | null>(null);

  const rest = () => {
    const [a, b, c] = rows.current!;
    place(a, 0, 1, 1);
    place(b, ROW_H, 0, 1);
    place(c, ROW_H * 2, 0, 0);
  };

  const tick = (now: number) => {
    frame.current = null;
    const [a, b, c] = rows.current!;
    if (start.current === null) {
      if (queue.current.length === 0) return;
      setText(c, queue.current[0]);
      place(c, ROW_H * 2, 0, 0);
      start.current = now;
    }
    const p = clamp((now - start.current) / DURATION, 0, 1);
    const e = EASE(p);
    // The completed row leaves upward and fades a little faster than it moves.
    place(a, lerp(0, -ROW_H, e), 1, clamp(1 - p * 1.35, 0, 1));
    place(b, lerp(ROW_H, 0, e), e, 1);
    place(c, lerp(ROW_H * 2, ROW_H, e), 0, e);

    if (p >= 1) {
      // Commit: texts move, elements stay put -- no reordering, no overlap.
      setText(a, b.text);
      setText(b, c.text);
      queue.current.shift();
      start.current = null;
      rest();
    }
    if (start.current !== null || queue.current.length > 0) {
      frame.current = requestAnimationFrame(tick);
    }
  };

  useEffect(() => {
    if (!rows.current) {
      rows.current = refs.map((r) => ({ el: r.current!, text: "" }));
    }
    const current = steps[steps.length - 1] ?? "…";
    const previous = steps[steps.length - 2] ?? "";
    const seen = shown.current;

    // First render, a different session, or a restart: drop into place.
    if (!seen || seen.session !== sessionId || stepCount < seen.count || prefersReducedMotion()) {
      queue.current = [];
      start.current = null;
      setText(rows.current[0], previous);
      setText(rows.current[1], current);
      rest();
      shown.current = { session: sessionId, count: stepCount };
      return;
    }

    const fresh = Math.min(stepCount - seen.count, steps.length);
    if (fresh <= 0) return;
    queue.current.push(...steps.slice(steps.length - fresh));
    if (queue.current.length > MAX_QUEUE) queue.current = queue.current.slice(-MAX_QUEUE);
    shown.current = { session: sessionId, count: stepCount };
    if (frame.current === null) frame.current = requestAnimationFrame(tick);
    // `tick` and `rest` only touch refs, so they are safe to leave out of the deps.
  }, [steps, stepCount, sessionId]);

  useEffect(
    () => () => {
      if (frame.current !== null) cancelAnimationFrame(frame.current);
    },
    [],
  );

  return (
    <div className="ticker" aria-live="polite">
      {refs.map((ref, i) => (
        <div key={i} ref={ref} className="ticker-row">
          <span className="ticker-icon">
            <ChevronRight className="ticker-chevron" aria-hidden />
            <Check className="ticker-check" aria-hidden />
          </span>
          <span className="ticker-text">
            <span data-text className="shimmer ticker-now" />
            <span data-text className="ticker-done" />
          </span>
        </div>
      ))}
    </div>
  );
}
