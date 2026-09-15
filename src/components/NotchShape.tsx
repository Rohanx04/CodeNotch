/**
 * The strip's silhouette.
 *
 * Drawn as one SVG path rather than with `border-radius`, because the corners
 * are *concave*: the black recedes from the two corners away from the screen
 * edge, tapering to nothing where it meets the edge. That is what makes the
 * notch look carved out of the display. A rounded rectangle bulges the opposite
 * way and reads as a panel sitting on top of the desktop.
 *
 * The scoops are elliptical, not circular — their depth along the edge is set
 * independently of the strip's thickness, so the taper stays gentle on a thin
 * strip and tight on a thick one.
 */

import type { Edge } from "../types";

interface Props {
  /** Window-space size of the strip, in px. */
  width: number;
  height: number;
  /** How far the scoop reaches along the edge. */
  scoop: number;
  edge: Edge;
  className?: string;
}

/**
 * Build the silhouette for one edge.
 *
 * Local axes: `thickness` runs into the screen, `length` runs along the edge.
 * Each scoop is a quarter of an ellipse with radii (thickness, scoop), swept so
 * it bulges *into* the black.
 */
export function notchPath(
  edge: Edge,
  width: number,
  height: number,
  scoop: number,
): string {
  const vertical = edge === "left" || edge === "right";
  const length = vertical ? height : width;
  // Two scoops have to fit end to end, with something left between them.
  const s = Math.max(0, Math.min(scoop, length / 2));

  if (edge === "right") {
    // Flush to x = width; scoops on the left, at top and bottom.
    return [
      `M ${width} 0`,
      `A ${width} ${s} 0 0 1 0 ${s}`,
      `L 0 ${height - s}`,
      `A ${width} ${s} 0 0 1 ${width} ${height}`,
      "Z",
    ].join(" ");
  }

  if (edge === "left") {
    return [
      `M 0 0`,
      `A ${width} ${s} 0 0 0 ${width} ${s}`,
      `L ${width} ${height - s}`,
      `A ${width} ${s} 0 0 0 0 ${height}`,
      "Z",
    ].join(" ");
  }

  if (edge === "top") {
    // Flush to y = 0; scoops on the bottom, at left and right.
    return [
      `M 0 0`,
      `L ${width} 0`,
      `A ${s} ${height} 0 0 1 ${width - s} ${height}`,
      `L ${s} ${height}`,
      `A ${s} ${height} 0 0 1 0 0`,
      "Z",
    ].join(" ");
  }

  return [
    `M 0 ${height}`,
    `L ${width} ${height}`,
    `A ${s} ${height} 0 0 0 ${width - s} 0`,
    `L ${s} 0`,
    `A ${s} ${height} 0 0 0 0 ${height}`,
    "Z",
  ].join(" ");
}

export function NotchShape({ width, height, scoop, edge, className }: Props) {
  if (width <= 0 || height <= 0) return null;

  return (
    <svg
      className={className}
      width={width}
      height={height}
      viewBox={`0 0 ${width} ${height}`}
      // Exact pixel geometry: no scaling, so the curve meets the screen edge
      // cleanly instead of landing on a half pixel.
      preserveAspectRatio="none"
      aria-hidden
    >
      <path d={notchPath(edge, width, height, scoop)} fill="var(--notch-black)" />
    </svg>
  );
}
