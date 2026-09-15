/**
 * Provider marks for the rings.
 *
 * Drawn here as geometry rather than pulled from an icon set: these have to
 * read at ~24px inside a ring, on black, in a single colour, and no
 * general-purpose icon library carries marks for these tools.
 *
 * They are recognisable renditions, not official artwork. Each vendor's logo is
 * their trademark; these are drawn to identify the tool the ring belongs to,
 * which is the whole reason the strip is legible at a glance.
 */

import type { ProviderId } from "../types";

interface Props {
  provider: ProviderId;
  className?: string;
  style?: React.CSSProperties;
}

/** Anthropic's starburst: rays radiating from a common centre. */
function ClaudeMark() {
  // Rays alternate long and short. Drawn as rounded bars rather than tapered
  // kites: at 24px a taper collapses into a thin sliver and the mark loses the
  // weight that makes it readable on black.
  const rays = Array.from({ length: 8 }, (_, i) => ({
    angle: i * 45,
    long: i % 2 === 0,
  }));
  return (
    <g fill="currentColor">
      {rays.map(({ angle, long }) => {
        const reach = long ? 10.2 : 8.2;
        const width = long ? 2.0 : 1.6;
        return (
          <rect
            key={angle}
            x={12 - width / 2}
            y={12 - reach}
            width={width}
            height={reach * 2}
            rx={width / 2}
            transform={`rotate(${angle} 12 12)`}
          />
        );
      })}
    </g>
  );
}

/** OpenAI's knot: three loops woven into a six-lobed rosette. */
function OpenAiMark() {
  // Three long rounded loops at 60 degrees to each other. Six arcs swung about
  // a circle (the literal construction) collapse into a swirl at this size;
  // overlapping stadia keep the woven, six-lobed silhouette legible.
  const loops = [0, 60, 120];
  return (
    <g fill="none" strokeWidth={1.5}>
      {loops.map((angle) => (
        <rect
          key={angle}
          x={8.6}
          y={2.9}
          width={6.8}
          height={18.2}
          rx={3.4}
          transform={`rotate(${angle} 12 12)`}
        />
      ))}
    </g>
  );
}

/** Gemini's four-pointed spark: straight points, concave flanks. */
function GeminiMark() {
  return (
    <path
      fill="currentColor"
      d="M12 1.8 C12 7.4 16.6 12 22.2 12 C16.6 12 12 16.6 12 22.2
         C12 16.6 7.4 12 1.8 12 C7.4 12 12 7.4 12 1.8 Z"
    />
  );
}

/** Perplexity's mark: a split spine with chevrons above and below. */
function PerplexityMark() {
  return (
    <g fill="none" strokeWidth={1.5} strokeLinecap="round" strokeLinejoin="round">
      {/* The spine runs the full height. */}
      <path d="M12 3.4v17.2" />
      {/* Two chevrons opening away from the centre. */}
      <path d="M4.2 9.2 12 3.4l7.8 5.8M4.2 14.8 12 20.6l7.8-5.8" />
      {/* Side rails, which is what makes it read as a mark and not an arrow. */}
      <path d="M4.2 9.2v5.6M19.8 9.2v5.6" />
      <path d="M4.2 12h3.1M16.7 12h3.1" />
    </g>
  );
}

/** Cursor's prism. */
function CursorMark() {
  return (
    <g fill="none" strokeWidth={1.5} strokeLinejoin="round">
      <path d="M12 2.8 20.4 7.6v8.8L12 21.2 3.6 16.4V7.6L12 2.8Z" />
      <path d="M12 2.8v9.4m0 0 8.4-4.6M12 12.2 3.6 7.6M12 12.2v9" />
    </g>
  );
}

/** Copilot's visor. */
function CopilotMark() {
  return (
    <g fill="none" strokeWidth={1.5} strokeLinecap="round" strokeLinejoin="round">
      <path d="M3.4 13c0-3.3 1.8-5 3.8-5 1.4 0 2.3.7 2.8 1.7M20.6 13c0-3.3-1.8-5-3.8-5-1.4 0-2.3.7-2.8 1.7" />
      <path d="M3.4 13c0 4 4 6.6 8.6 6.6s8.6-2.6 8.6-6.6" />
      <path d="M9 12.8v1.9M15 12.8v1.9" />
    </g>
  );
}

/** Ollama's llama, reduced to a silhouette that survives 24px. */
function OllamaMark() {
  return (
    <g fill="none" strokeWidth={1.5} strokeLinecap="round" strokeLinejoin="round">
      {/* Ears. */}
      <path d="M8.4 7.2V4.3c0-.7.9-1 1.3-.4l1.4 2.2M15.6 7.2V4.3c0-.7-.9-1-1.3-.4l-1.4 2.2" />
      {/* Head and muzzle. */}
      <path d="M8.4 7.2c0 2.6-1.6 3.6-1.6 6.2 0 2.5 2.3 4.2 5.2 4.2s5.2-1.7 5.2-4.2c0-2.6-1.6-3.6-1.6-6.2" />
      <path d="M10 20.4v-2.6M14 20.4v-2.6" />
      <path d="M10.6 12.2h.01M13.4 12.2h.01" />
    </g>
  );
}

const MARKS: Record<ProviderId, () => React.JSX.Element> = {
  claudeCode: ClaudeMark,
  codex: OpenAiMark,
  gemini: GeminiMark,
  perplexity: PerplexityMark,
  cursor: CursorMark,
  copilot: CopilotMark,
  ollama: OllamaMark,
};

/** Marks drawn with fills rather than strokes. */
const FILLED: ReadonlySet<ProviderId> = new Set<ProviderId>(["claudeCode", "gemini"]);

export function BrandIcon({ provider, className, style }: Props) {
  const Mark = MARKS[provider] ?? ClaudeMark;
  const filled = FILLED.has(provider);

  return (
    <svg
      viewBox="0 0 24 24"
      className={className}
      style={style}
      // `currentColor` on both so callers only ever set a text colour.
      fill={filled ? "currentColor" : "none"}
      stroke={filled ? "none" : "currentColor"}
      aria-hidden
    >
      <Mark />
    </svg>
  );
}
