/**
 * Short sound cues, synthesised on the fly.
 *
 * No audio files: each cue is a couple of enveloped oscillator notes, so
 * CodeNotch ships no sound assets and every cue can be tuned in code. The
 * engine idea (a shared master gain, overlapping cues, suspending the audio
 * context when quiet) follows Coucou's sound engine; the sounds themselves are
 * CodeNotch's own.
 *
 * A running AudioContext keeps an audio thread alive even when nothing is
 * playing, which shows up as a steady trickle of CPU on an idle machine, so
 * the context is suspended shortly after the last cue and resumed on demand.
 */

export type CueName =
  | "open"
  | "close"
  | "attention"
  | "approval"
  | "finish"
  | "error"
  | "threshold"
  | "allow"
  | "deny";

type Wave = OscillatorType;

interface Note {
  /** Hz. */
  freq: number;
  /** Seconds after the cue starts. */
  at: number;
  /** Seconds. */
  dur: number;
  wave?: Wave;
  /** Relative loudness, 0..1. */
  gain?: number;
  /** Glide to this frequency over the note. */
  slideTo?: number;
}

/** Note values, so the cues read as music rather than magic numbers. */
const N = {
  A3: 220,
  C5: 523.25,
  E5: 659.25,
  G5: 783.99,
  A5: 880,
  C6: 1046.5,
  E6: 1318.5,
  G6: 1568,
};

const CUES: Record<CueName, Note[]> = {
  // Barely there: the card opening under the pointer shouldn't nag.
  open: [{ freq: N.G5, at: 0, dur: 0.06, wave: "sine", gain: 0.35, slideTo: N.C6 }],
  close: [{ freq: N.C6, at: 0, dur: 0.07, wave: "sine", gain: 0.3, slideTo: N.G5 }],
  // An agent is blocked on you: two bright taps.
  attention: [
    { freq: N.E6, at: 0, dur: 0.09, wave: "triangle", gain: 0.8 },
    { freq: N.A5, at: 0.12, dur: 0.12, wave: "triangle", gain: 0.8 },
  ],
  // A permission request: three rising taps, the most insistent cue.
  approval: [
    { freq: N.A5, at: 0, dur: 0.08, wave: "triangle", gain: 0.85 },
    { freq: N.C6, at: 0.1, dur: 0.08, wave: "triangle", gain: 0.85 },
    { freq: N.E6, at: 0.2, dur: 0.14, wave: "triangle", gain: 0.85 },
  ],
  // Done: a small major arpeggio.
  finish: [
    { freq: N.C5, at: 0, dur: 0.1, wave: "sine", gain: 0.7 },
    { freq: N.E5, at: 0.08, dur: 0.1, wave: "sine", gain: 0.7 },
    { freq: N.G5, at: 0.16, dur: 0.18, wave: "sine", gain: 0.7 },
  ],
  // An error: low and falling.
  error: [
    { freq: N.E5, at: 0, dur: 0.12, wave: "square", gain: 0.25, slideTo: N.A3 },
    { freq: N.A3, at: 0.15, dur: 0.16, wave: "square", gain: 0.22 },
  ],
  // A usage window crossed 80% / 100%.
  threshold: [
    { freq: N.G6, at: 0, dur: 0.08, wave: "sine", gain: 0.6 },
    { freq: N.G6, at: 0.14, dur: 0.08, wave: "sine", gain: 0.6 },
  ],
  allow: [
    { freq: N.E5, at: 0, dur: 0.07, wave: "sine", gain: 0.6 },
    { freq: N.C6, at: 0.07, dur: 0.12, wave: "sine", gain: 0.6 },
  ],
  deny: [{ freq: N.E5, at: 0, dur: 0.1, wave: "sine", gain: 0.5, slideTo: N.C5 }],
};

class SoundEngine {
  private enabled = true;
  private volume = 0.12;
  private ctx: AudioContext | null = null;
  private master: GainNode | null = null;
  private idleTimer: number | null = null;

  configure(enabled: boolean, volume: number) {
    this.enabled = enabled;
    this.volume = Math.max(0, Math.min(0.2, volume));
    if (this.master) this.master.gain.value = this.volume;
  }

  private context(): AudioContext | null {
    if (this.ctx) return this.ctx;
    const Ctor =
      window.AudioContext ??
      (window as unknown as { webkitAudioContext?: typeof AudioContext }).webkitAudioContext;
    if (!Ctor) return null;
    try {
      this.ctx = new Ctor();
      this.master = this.ctx.createGain();
      this.master.gain.value = this.volume;
      this.master.connect(this.ctx.destination);
    } catch {
      // No audio device: cues are a nicety, never a failure.
      this.ctx = null;
    }
    return this.ctx;
  }

  play(name: CueName) {
    if (!this.enabled || this.volume <= 0) return;
    const ctx = this.context();
    const master = this.master;
    if (!ctx || !master) return;
    if (ctx.state === "suspended") void ctx.resume();

    const t0 = ctx.currentTime + 0.01;
    let end = t0;
    for (const note of CUES[name]) {
      const osc = ctx.createOscillator();
      const env = ctx.createGain();
      const start = t0 + note.at;
      const stop = start + note.dur;
      osc.type = note.wave ?? "sine";
      osc.frequency.setValueAtTime(note.freq, start);
      if (note.slideTo) osc.frequency.exponentialRampToValueAtTime(note.slideTo, stop);
      // A fast attack and an exponential tail: a tap, not a beep.
      const peak = note.gain ?? 0.6;
      env.gain.setValueAtTime(0.0001, start);
      env.gain.exponentialRampToValueAtTime(peak, start + 0.008);
      env.gain.exponentialRampToValueAtTime(0.0001, stop);
      osc.connect(env).connect(master);
      osc.start(start);
      osc.stop(stop + 0.02);
      end = Math.max(end, stop);
    }
    this.scheduleIdle((end - ctx.currentTime) * 1000);
  }

  /** Suspend once the last cue has rung out. */
  private scheduleIdle(tailMs: number) {
    if (this.idleTimer !== null) window.clearTimeout(this.idleTimer);
    this.idleTimer = window.setTimeout(() => {
      this.idleTimer = null;
      void this.ctx?.suspend();
    }, Math.max(0, tailMs) + 1500);
  }
}

export const Sound = new SoundEngine();
