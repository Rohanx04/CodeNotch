/**
 * Springs and curves for the notch's motion.
 *
 * Opening and closing are deliberately different gestures. Growing uses a
 * damped spring (SwiftUI's `response` / `dampingFraction` model), so a card
 * arrives with a little life in it; shrinking uses a fixed 340 ms curve with
 * no overshoot, so it leaves cleanly instead of bouncing on the way out.
 *
 * Adapted from the MIT-licensed motion code in Coucou (Copyright (c) 2026
 * Louis Raillé, https://github.com/Louis-CFM/coucou).
 */

export const clamp = (v: number, lo: number, hi: number) => Math.max(lo, Math.min(hi, v));
export const lerp = (a: number, b: number, t: number) => a + (b - a) * t;

/** `cubic-bezier(x1, y1, x2, y2)` as a function of progress. */
export function cubicBezier(x1: number, y1: number, x2: number, y2: number) {
  const bx = (t: number) => 3 * (1 - t) ** 2 * t * x1 + 3 * (1 - t) * t * t * x2 + t ** 3;
  const by = (t: number) => 3 * (1 - t) ** 2 * t * y1 + 3 * (1 - t) * t * t * y2 + t ** 3;
  return (x: number) => {
    // Bisection on x: a dozen steps is sub-pixel at any size we draw.
    let lo = 0;
    let hi = 1;
    let t = x;
    for (let i = 0; i < 12; i++) {
      if (bx(t) < x) lo = t;
      else hi = t;
      t = (lo + hi) / 2;
    }
    return by(t);
  };
}

/** The close curve: leaves briskly, lands without overshoot. */
export const closeCurve = cubicBezier(0.45, 0, 0.2, 1);
export const CLOSE_MS = 340;

/**
 * A damped spring: ω₀ = 2π / response, ζ = damping. Integrated in fixed
 * sub-steps so a dropped frame never destabilises it.
 */
export class Spring {
  value: number;
  target: number;
  velocity = 0;
  private omega: number;
  private zeta: number;

  constructor(value: number, response = 0.5, damping = 0.72) {
    this.value = value;
    this.target = value;
    this.omega = (2 * Math.PI) / response;
    this.zeta = damping;
  }

  configure(response: number, damping: number) {
    this.omega = (2 * Math.PI) / response;
    this.zeta = damping;
  }

  jump(value: number) {
    this.value = value;
    this.target = value;
    this.velocity = 0;
  }

  get settled(): boolean {
    return Math.abs(this.target - this.value) < 0.01 && Math.abs(this.velocity) < 0.05;
  }

  step(dt: number) {
    const steps = Math.max(1, Math.ceil(dt / (1 / 240)));
    const h = dt / steps;
    for (let i = 0; i < steps; i++) {
      const acc =
        this.omega * this.omega * (this.target - this.value) -
        2 * this.zeta * this.omega * this.velocity;
      this.velocity += acc * h;
      this.value += this.velocity * h;
    }
    if (this.settled) {
      this.value = this.target;
      this.velocity = 0;
    }
  }
}

/**
 * A value that springs when it grows and follows the close curve when it
 * shrinks.
 */
export class Tracked {
  private spring: Spring;
  private mode: "idle" | "spring" | "curve" = "idle";
  private from = 0;
  private to = 0;
  private start = 0;
  private duration = CLOSE_MS;

  constructor(value: number) {
    this.spring = new Spring(value);
  }

  get value() {
    return this.spring.value;
  }

  get target() {
    return this.spring.target;
  }

  get animating() {
    return this.mode !== "idle";
  }

  jump(value: number) {
    this.spring.jump(value);
    this.mode = "idle";
  }

  springTo(value: number, response = 0.5, damping = 0.72) {
    if (this.mode === "spring" && this.spring.target === value) return;
    this.spring.configure(response, damping);
    this.spring.target = value;
    this.mode = this.spring.value === value && this.spring.velocity === 0 ? "idle" : "spring";
  }

  curveTo(value: number, now: number, duration = CLOSE_MS) {
    if (this.mode === "curve" && this.to === value) return;
    this.from = this.spring.value;
    this.to = value;
    this.start = now;
    this.duration = duration;
    this.spring.target = value;
    this.spring.velocity = 0;
    this.mode = this.from === value ? "idle" : "curve";
  }

  step(dt: number, now: number) {
    if (this.mode === "spring") {
      this.spring.step(dt);
      if (this.spring.settled) this.mode = "idle";
    } else if (this.mode === "curve") {
      const p = clamp((now - this.start) / this.duration, 0, 1);
      this.spring.value = lerp(this.from, this.to, closeCurve(p));
      if (p >= 1) this.mode = "idle";
    }
  }
}

/** True when the user asked the system for less motion. */
export function prefersReducedMotion(): boolean {
  return (
    typeof window !== "undefined" &&
    window.matchMedia?.("(prefers-reduced-motion: reduce)").matches === true
  );
}
