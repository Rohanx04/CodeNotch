/**
 * Panel geometry for the browser preview.
 *
 * In the desktop app Rust decides the window's size and where the strip sits
 * in it (`layout::panel_size` / `dock_panel`) and sends both in `HudState`.
 * `npm run dev` has no backend, so the preview derives the same numbers here.
 * The constants must match `src-tauri/core/src/layout.rs`; a Rust test reads
 * this file to keep them honest.
 */

import type { Edge, HudMetrics } from "../types";

export const MAX_STRIP_LENGTH = 640;
export const MAX_POPOVER_LENGTH = 460;
export const WAKE_THICKNESS = 4;
/** Slack around each painted shape that still counts as on it. */
export const HIT_MARGIN = 14;

/** Strip length along its edge for `rings` providers. */
export function stripAlong(m: HudMetrics, rings: number): number {
  const along = m.stripPadding * 2 + Math.max(1, rings) * m.slot + m.stripThickness * 0.55;
  return Math.min(along, MAX_STRIP_LENGTH);
}

/** Logical window size while the notch is on screen: strip plus tallest card. */
export function panelSize(m: HudMetrics, edge: Edge, rings: number) {
  const horizontal = edge === "top" || edge === "bottom";
  const cardAlong = horizontal ? m.popoverSize : MAX_POPOVER_LENGTH;
  const cardDepth = horizontal ? MAX_POPOVER_LENGTH : m.popoverSize;
  const along = Math.max(stripAlong(m, rings), cardAlong);
  const depth = m.stripThickness + m.popoverGap + cardDepth;
  return horizontal ? { width: along, height: depth } : { width: depth, height: along };
}

/** Where the strip sits inside the panel: flush with the edge, centred along it. */
export function stripOffset(m: HudMetrics, edge: Edge, rings: number) {
  const panel = panelSize(m, edge, rings);
  const along = stripAlong(m, rings);
  switch (edge) {
    case "right":
      return { x: panel.width - m.stripThickness, y: (panel.height - along) / 2 };
    case "left":
      return { x: 0, y: (panel.height - along) / 2 };
    case "top":
      return { x: (panel.width - along) / 2, y: 0 };
    case "bottom":
      return { x: (panel.width - along) / 2, y: panel.height - m.stripThickness };
  }
}

/**
 * A stable colour per project, so the same repository reads the same in
 * every session list. Hashed from the name; no palette to maintain.
 */
const PROJECT_COLOURS = ["#22c55e", "#eab308", "#60a5fa", "#e879f9", "#f97316", "#2dd4bf"];

export function projectColour(name: string): string {
  let hash = 0;
  for (let i = 0; i < name.length; i++) hash = (hash * 31 + name.charCodeAt(i)) | 0;
  return PROJECT_COLOURS[Math.abs(hash) % PROJECT_COLOURS.length];
}
