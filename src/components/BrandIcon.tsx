/**
 * Provider marks for the rings.
 *
 * Six of the seven are the vendors' own marks, from Simple Icons' CC0 icon
 * data (see brandPaths.ts). They have to read at ~22px inside a ring, on
 * black, in a single colour, which is exactly what those outlines are drawn
 * for. Each logo remains its owner's trademark; they identify which tool a
 * ring belongs to and nothing more.
 *
 * Codex is drawn here by hand instead — OpenAI had their mark withdrawn from
 * Simple Icons, so this is a rendition rather than their artwork.
 */

import type { ProviderId } from "../types";
import { BRAND_PATHS } from "./brandPaths";

interface Props {
  provider: ProviderId;
  className?: string;
  style?: React.CSSProperties;
}

/**
 * A rendition of OpenAI's knot: three long loops woven at 60 degrees.
 *
 * Six arcs swung about a circle — the literal construction — collapse into a
 * swirl at this size; overlapping stadia keep the six-lobed silhouette.
 */
function CodexMark() {
  return (
    <g fill="none" stroke="currentColor" strokeWidth={1.4}>
      {[0, 60, 120].map((angle) => (
        <rect
          key={angle}
          x={8.9}
          y={2.6}
          width={6.2}
          height={18.8}
          rx={3.1}
          transform={`rotate(${angle} 12 12)`}
        />
      ))}
    </g>
  );
}

export function BrandIcon({ provider, className, style }: Props) {
  const path = BRAND_PATHS[provider];

  return (
    <svg viewBox="0 0 24 24" className={className} style={style} aria-hidden>
      {path ? <path d={path} fill="currentColor" /> : <CodexMark />}
    </svg>
  );
}
