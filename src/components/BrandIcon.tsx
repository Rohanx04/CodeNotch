/**
 * Provider glyphs for the rings.
 *
 * Drawn here as simple geometry rather than pulled from an icon set: the marks
 * need to read at 22px inside a ring, on black, in a single colour, and none of
 * the general-purpose icon libraries has anything for these tools.
 */

import type { ProviderId } from "../types";

interface Props {
  provider: ProviderId;
  className?: string;
}

/** Claude: a radial burst of tapered spokes. */
function ClaudeMark() {
  // Eight spokes, alternating long and short, drawn as tapered quads so the
  // mark keeps its weight when it shrinks.
  const spokes = Array.from({ length: 8 }, (_, i) => i * 45);
  return (
    <g>
      {spokes.map((angle, i) => {
        const long = i % 2 === 0;
        const outer = long ? 10.5 : 8.5;
        const width = long ? 1.9 : 1.5;
        return (
          <rect
            key={angle}
            x={12 - width / 2}
            y={12 - outer}
            width={width}
            height={outer * 2}
            rx={width / 2}
            transform={`rotate(${angle} 12 12)`}
          />
        );
      })}
    </g>
  );
}

/** Cursor: an isometric prism, the shape its editor uses. */
function CursorMark() {
  return (
    <g fill="none" strokeWidth={1.6} strokeLinejoin="round">
      <path d="M12 3 20 7.5v9L12 21 4 16.5v-9L12 3Z" />
      <path d="M12 3v9m0 0 8-4.5M12 12l-8-4.5M12 12v9" />
    </g>
  );
}

/** Codex: a six-fold knot, echoing the OpenAI rosette without copying it. */
function CodexMark() {
  const petals = [0, 60, 120];
  return (
    <g fill="none" strokeWidth={1.5}>
      {petals.map((angle) => (
        <ellipse
          key={angle}
          cx={12}
          cy={12}
          rx={4}
          ry={9}
          transform={`rotate(${angle} 12 12)`}
        />
      ))}
    </g>
  );
}

/** Copilot: the familiar forked silhouette, simplified to two lobes. */
function CopilotMark() {
  return (
    <g fill="none" strokeWidth={1.6} strokeLinecap="round">
      <path d="M4 13.5c0-3 1.6-4.5 3.4-4.5 1.2 0 2 .6 2.4 1.4M20 13.5c0-3-1.6-4.5-3.4-4.5-1.2 0-2 .6-2.4 1.4" />
      <path d="M4 13.5c0 3.6 3.6 6 8 6s8-2.4 8-6" />
      <path d="M9 13.2v1.6M15 13.2v1.6" />
    </g>
  );
}

/** Ollama: a stack of memory cells, since what it costs you is RAM. */
function OllamaMark() {
  return (
    <g fill="none" strokeWidth={1.6} strokeLinecap="round">
      <rect x={6.5} y={6.5} width={11} height={11} rx={3} />
      <rect x={10} y={10} width={4} height={4} rx={1} />
      <path d="M9.5 3.5v3M14.5 3.5v3M9.5 17.5v3M14.5 17.5v3M3.5 9.5h3M3.5 14.5h3M17.5 9.5h3M17.5 14.5h3" />
    </g>
  );
}

const MARKS: Record<ProviderId, () => React.JSX.Element> = {
  claudeCode: ClaudeMark,
  cursor: CursorMark,
  codex: CodexMark,
  copilot: CopilotMark,
  ollama: OllamaMark,
};

export function BrandIcon({ provider, className }: Props) {
  const Mark = MARKS[provider] ?? ClaudeMark;
  // Claude's mark is solid, the rest are strokes; `currentColor` drives both so
  // callers only ever set a text colour.
  const stroked = provider !== "claudeCode";
  return (
    <svg
      viewBox="0 0 24 24"
      className={className}
      fill={stroked ? "none" : "currentColor"}
      stroke={stroked ? "currentColor" : "none"}
      aria-hidden
    >
      <Mark />
    </svg>
  );
}
