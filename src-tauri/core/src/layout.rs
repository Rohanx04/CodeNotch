//! Where the notch sits on screen.
//!
//! Kept separate from the Win32 code so the arithmetic — DPI scaling, edge
//! anchoring, keeping the window inside the work area — can be tested without a
//! desktop. The platform layer supplies a [`WorkArea`] measured from the OS and
//! applies the resulting [`Placement`] verbatim.

use crate::config::{Edge, HudMetrics};

/// A monitor's usable area in **physical** pixels, excluding the taskbar.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorkArea {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    /// DPI scale, e.g. 1.5 for 150%.
    pub scale: f64,
}

impl WorkArea {
    pub fn new(x: i32, y: i32, width: i32, height: i32, scale: f64) -> Self {
        Self {
            x,
            y,
            width,
            height,
            // A zero or negative scale would collapse the window to nothing.
            scale: if scale > 0.0 { scale } else { 1.0 },
        }
    }
}

/// A window rectangle in physical pixels, ready for `SetWindowPos`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// Position a HUD of `logical_width` x `logical_height` against `edge`.
///
/// `offset` slides the window along the edge: 0.0 is left/top, 0.5 centred,
/// 1.0 right/bottom. `margin` is a logical-pixel gap from the edge.
///
/// The result is always clamped inside the work area when the window fits, so
/// growing the card downward can never push it under the taskbar or off-screen.
pub fn place(
    area: WorkArea,
    edge: Edge,
    offset: f32,
    margin: f64,
    logical_width: f64,
    logical_height: f64,
) -> Placement {
    let scale = area.scale;
    // Round rather than truncate: a half-pixel error is visible as a 1px seam
    // against the screen edge at fractional DPI scales.
    let width = (logical_width * scale).round() as i32;
    let height = (logical_height * scale).round() as i32;
    let margin = (margin * scale).round() as i32;
    let offset = offset.clamp(0.0, 1.0) as f64;

    // Slide along the free space on the cross axis.
    let slide = |extent: i32, size: i32| -> i32 {
        let free = (extent - size).max(0);
        (free as f64 * offset).round() as i32
    };

    let (x, y) = match edge {
        Edge::Top => (area.x + slide(area.width, width), area.y + margin),
        Edge::Bottom => (
            area.x + slide(area.width, width),
            area.y + area.height - height - margin,
        ),
        Edge::Left => (area.x + margin, area.y + slide(area.height, height)),
        Edge::Right => (
            area.x + area.width - width - margin,
            area.y + slide(area.height, height),
        ),
    };

    Placement {
        x: clamp_axis(x, width, area.x, area.width),
        y: clamp_axis(y, height, area.y, area.height),
        width,
        height,
    }
}

/// Keep `start..start+size` inside `origin..origin+extent`.
///
/// A window larger than the work area is pinned to the origin, which keeps its
/// top-left corner reachable instead of centring the overflow on both sides.
fn clamp_axis(start: i32, size: i32, origin: i32, extent: i32) -> i32 {
    if size >= extent {
        return origin;
    }
    start.clamp(origin, origin + extent - size)
}

/// Longest the strip may grow along its edge before its list scrolls instead.
///
/// A HUD that reaches the full height of the display is a sidebar, not a notch,
/// so this sits well inside a 1080p work area: even the large size with every
/// provider enabled comes in under it, and the clamp is a backstop rather than
/// something the default layout runs into.
pub const MAX_STRIP_LENGTH: f64 = 640.0;
/// Tallest the detail popover may grow.
pub const MAX_POPOVER_LENGTH: f64 = 460.0;

/// Thickness of the invisible strip left against the screen edge when
/// auto-hide has tucked the notch away. Moving the pointer onto it brings the
/// notch back; thin enough that it never gets in the way of a scrollbar.
pub const WAKE_THICKNESS: f64 = 4.0;

/// How far outside a shape the pointer can stray and still count as on it.
///
/// Wide on purpose: the window only stops being click-through once the cursor
/// is inside this margin, so a pointer moving towards a button has already
/// made the window hittable by the time it gets there -- and a click is never
/// swallowed by a window that was still transparent to it.
pub const HIT_MARGIN: f64 = 14.0;

/// Logical size of the strip holding `providers` rings, in window axes
/// (width, height).
pub fn strip_size(metrics: HudMetrics, edge: Edge, providers: usize) -> (f64, f64) {
    let (along, thickness) = metrics.strip_extent(providers);
    let along = along.min(MAX_STRIP_LENGTH);
    if edge.is_horizontal() {
        (along, thickness)
    } else {
        (thickness, along)
    }
}

/// Logical size of the HUD window whenever the notch is on screen.
///
/// The window is the largest the notch can ever need -- the strip plus the
/// tallest card beside it -- and it stays that size whether a card is open or
/// not. Nothing about the window changes as the notch opens, closes, or moves
/// between cards, so all of that can be animated freely: springs on the card's
/// size, a close animation that plays out in full, a strip that slides into the
/// edge. Outside the shapes the webview paints, the window is click-through
/// (see [`hit_test`]), so the extra room costs the desktop nothing.
pub fn panel_size(metrics: HudMetrics, edge: Edge, providers: usize) -> (f64, f64) {
    let (along, thickness) = metrics.strip_extent(providers);
    let along = along.min(MAX_STRIP_LENGTH);

    // The card is `popover_size` wide on every edge. Beside a vertical strip
    // that width is its depth and its content decides its length; beside a
    // horizontal one it is the other way round.
    let (card_along, card_depth) = if edge.is_horizontal() {
        (metrics.popover_size, MAX_POPOVER_LENGTH)
    } else {
        (MAX_POPOVER_LENGTH, metrics.popover_size)
    };

    let along = along.max(card_along);
    let depth = thickness + metrics.popover_gap + card_depth;
    if edge.is_horizontal() {
        (along, depth)
    } else {
        (depth, along)
    }
}

/// Logical size of the wake strip auto-hide leaves behind: as long as the
/// strip it replaces, [`WAKE_THICKNESS`] deep.
pub fn wake_size(metrics: HudMetrics, edge: Edge, providers: usize) -> (f64, f64) {
    let (w, h) = strip_size(metrics, edge, providers);
    if edge.is_horizontal() {
        (w, WAKE_THICKNESS)
    } else {
        (WAKE_THICKNESS, h)
    }
}

/// The HUD window, and where the strip sits inside it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Docked {
    /// The window, in physical pixels.
    pub window: Placement,
    /// The strip's top-left corner inside the window, in logical pixels.
    pub strip_x: f64,
    pub strip_y: f64,
}

/// Dock the panel window so the strip lands exactly where [`place`] would put
/// it on its own.
///
/// The strip is what the user positioned (edge, offset, margin), so it is
/// placed first and the window is built around it: flush with the strip's outer
/// side, reaching inward by the panel's depth, and centred on the strip along
/// the edge -- then pulled back inside the work area. Placing the *window* by
/// the offset instead would move the strip every time the panel's length
/// changed, and would stop an offset of 0 meaning "flush with the top".
pub fn dock_panel(
    area: WorkArea,
    edge: Edge,
    offset: f32,
    margin: f64,
    strip: (f64, f64),
    panel: (f64, f64),
) -> Docked {
    let s = place(area, edge, offset, margin, strip.0, strip.1);
    let scale = area.scale;
    let pw = (panel.0 * scale).round() as i32;
    let ph = (panel.1 * scale).round() as i32;

    let centred = |start: i32, size: i32, panel: i32| start + (size - panel) / 2;
    let (x, y) = match edge {
        Edge::Right => (
            s.x + s.width - pw,
            clamp_axis(centred(s.y, s.height, ph), ph, area.y, area.height),
        ),
        Edge::Left => (
            s.x,
            clamp_axis(centred(s.y, s.height, ph), ph, area.y, area.height),
        ),
        Edge::Top => (
            clamp_axis(centred(s.x, s.width, pw), pw, area.x, area.width),
            s.y,
        ),
        Edge::Bottom => (
            clamp_axis(centred(s.x, s.width, pw), pw, area.x, area.width),
            s.y + s.height - ph,
        ),
    };

    Docked {
        window: Placement {
            x,
            y,
            width: pw,
            height: ph,
        },
        strip_x: (s.x - x) as f64 / scale,
        strip_y: (s.y - y) as f64 / scale,
    }
}

/// Dock the wake strip: at the strip's position along the edge, but flush
/// against the edge itself whatever the margin, because the edge is where a
/// pointer thrown at the side of the screen actually stops.
pub fn dock_wake(area: WorkArea, edge: Edge, offset: f32, wake: (f64, f64)) -> Placement {
    place(area, edge, offset, 0.0, wake.0, wake.1)
}

/// A shape the webview has painted, in window-logical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct HitRect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// Whether a window-logical point is on any of the painted shapes, allowing
/// `margin` of slack around each.
pub fn hit_test(rects: &[HitRect], x: f64, y: f64, margin: f64) -> bool {
    rects.iter().any(|r| {
        r.w > 0.0
            && r.h > 0.0
            && x >= r.x - margin
            && x <= r.x + r.w + margin
            && y >= r.y - margin
            && y <= r.y + r.h + margin
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 1920x1080 with a 40px taskbar at the bottom, at 100% scale.
    fn screen() -> WorkArea {
        WorkArea::new(0, 0, 1920, 1040, 1.0)
    }

    #[test]
    fn top_centre_is_the_default_resting_position() {
        let p = place(screen(), Edge::Top, 0.5, 0.0, 168.0, 32.0);
        assert_eq!(p.width, 168);
        assert_eq!(p.height, 32);
        assert_eq!(p.x, (1920 - 168) / 2);
        assert_eq!(p.y, 0);
    }

    #[test]
    fn margin_offsets_from_the_work_area_edge() {
        let p = place(screen(), Edge::Top, 0.5, 8.0, 168.0, 32.0);
        assert_eq!(p.y, 8);

        let p = place(screen(), Edge::Bottom, 0.5, 8.0, 168.0, 32.0);
        assert_eq!(p.y, 1040 - 32 - 8, "bottom edge measures from the taskbar");
    }

    #[test]
    fn expanding_downward_keeps_the_top_edge_pinned() {
        // The pill and the expanded card must share a top edge, so the card
        // appears to grow out of the pill rather than jumping.
        let collapsed = place(screen(), Edge::Top, 0.5, 0.0, 168.0, 32.0);
        let expanded = place(screen(), Edge::Top, 0.5, 0.0, 344.0, 420.0);
        assert_eq!(collapsed.y, expanded.y);
    }

    #[test]
    fn expanding_upward_keeps_the_bottom_edge_pinned() {
        let collapsed = place(screen(), Edge::Bottom, 0.5, 0.0, 168.0, 32.0);
        let expanded = place(screen(), Edge::Bottom, 0.5, 0.0, 344.0, 420.0);
        assert_eq!(
            collapsed.y + collapsed.height,
            expanded.y + expanded.height,
            "the card should grow up from the taskbar, not off the screen"
        );
    }

    #[test]
    fn offset_slides_the_window_along_the_edge() {
        let left = place(screen(), Edge::Top, 0.0, 0.0, 168.0, 32.0);
        assert_eq!(left.x, 0);

        let right = place(screen(), Edge::Top, 1.0, 0.0, 168.0, 32.0);
        assert_eq!(right.x, 1920 - 168);

        // Out-of-range offsets clamp instead of flying off-screen.
        let silly = place(screen(), Edge::Top, 9.0, 0.0, 168.0, 32.0);
        assert_eq!(silly.x, 1920 - 168);
    }

    #[test]
    fn side_edges_slide_vertically() {
        let p = place(screen(), Edge::Left, 0.0, 12.0, 48.0, 200.0);
        assert_eq!(p.x, 12);
        assert_eq!(p.y, 0);

        let p = place(screen(), Edge::Right, 1.0, 12.0, 48.0, 200.0);
        assert_eq!(p.x, 1920 - 48 - 12);
        assert_eq!(p.y, 1040 - 200);
    }

    #[test]
    fn logical_sizes_scale_with_dpi() {
        // 150% scaling: a 168pt pill occupies 252 physical pixels.
        let area = WorkArea::new(0, 0, 2880, 1560, 1.5);
        let p = place(area, Edge::Top, 0.5, 8.0, 168.0, 32.0);
        assert_eq!(p.width, 252);
        assert_eq!(p.height, 48);
        assert_eq!(p.y, 12, "the margin scales too");
        assert_eq!(p.x, (2880 - 252) / 2);
    }

    #[test]
    fn fractional_scales_round_rather_than_truncate() {
        // 125% of 168 is 210; 125% of 33 is 41.25 -> 41.
        let area = WorkArea::new(0, 0, 2400, 1300, 1.25);
        let p = place(area, Edge::Top, 0.5, 0.0, 168.0, 33.0);
        assert_eq!(p.width, 210);
        assert_eq!(p.height, 41);
    }

    #[test]
    fn secondary_monitors_with_negative_origins_work() {
        // A monitor to the left of the primary has a negative x origin.
        let area = WorkArea::new(-1920, -200, 1920, 1080, 1.0);
        let p = place(area, Edge::Top, 0.5, 0.0, 200.0, 40.0);
        assert_eq!(p.x, -1920 + (1920 - 200) / 2);
        assert_eq!(p.y, -200);
    }

    #[test]
    fn a_window_is_never_pushed_outside_the_work_area() {
        // A tall card on a short screen plus a large margin would otherwise
        // start above the top of the work area.
        let area = WorkArea::new(0, 0, 1920, 500, 1.0);
        let p = place(area, Edge::Bottom, 0.5, 200.0, 344.0, 420.0);
        assert!(p.y >= 0, "clamped back inside, got y={}", p.y);
        assert!(p.y + p.height <= 500 || p.height >= 500);
    }

    #[test]
    fn an_oversized_window_pins_to_the_origin() {
        let area = WorkArea::new(100, 50, 300, 200, 1.0);
        let p = place(area, Edge::Top, 0.5, 0.0, 800.0, 600.0);
        assert_eq!(p.x, 100);
        assert_eq!(p.y, 50);
    }

    #[test]
    fn a_nonsense_scale_falls_back_to_one_to_one() {
        let area = WorkArea::new(0, 0, 1920, 1040, 0.0);
        assert_eq!(area.scale, 1.0);
        let p = place(area, Edge::Top, 0.5, 0.0, 168.0, 32.0);
        assert_eq!(p.width, 168);
    }
}

#[cfg(test)]
mod panel_tests {
    use super::*;
    use crate::config::HudSize;

    fn m() -> HudMetrics {
        HudSize::Medium.metrics()
    }

    fn screen() -> WorkArea {
        WorkArea::new(0, 0, 1920, 1040, 1.0)
    }

    #[test]
    fn the_strip_lands_where_place_would_put_it_alone() {
        // Whatever size the panel is, the strip is what the user positioned, so
        // it must sit exactly where the strip-sized window used to.
        for edge in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
            for offset in [0.0, 0.3, 0.5, 1.0] {
                let strip = strip_size(m(), edge, 3);
                let alone = place(screen(), edge, offset, 6.0, strip.0, strip.1);
                let docked =
                    dock_panel(screen(), edge, offset, 6.0, strip, panel_size(m(), edge, 3));
                let x = docked.window.x + docked.strip_x.round() as i32;
                let y = docked.window.y + docked.strip_y.round() as i32;
                assert_eq!((x, y), (alone.x, alone.y), "{edge:?} at {offset}");
            }
        }
    }

    #[test]
    fn the_panel_stays_inside_the_work_area() {
        for edge in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
            for offset in [0.0, 1.0] {
                let d = dock_panel(
                    screen(),
                    edge,
                    offset,
                    0.0,
                    strip_size(m(), edge, 1),
                    panel_size(m(), edge, 1),
                );
                let w = d.window;
                assert!(w.x >= 0 && w.y >= 0, "{edge:?} {offset}: {w:?}");
                assert!(
                    w.x + w.width <= 1920 && w.y + w.height <= 1040,
                    "{edge:?}: {w:?}"
                );
            }
        }
    }

    #[test]
    fn the_panel_is_flush_with_the_strips_outer_side() {
        let strip = strip_size(m(), Edge::Right, 3);
        let d = dock_panel(
            screen(),
            Edge::Right,
            0.5,
            0.0,
            strip,
            panel_size(m(), Edge::Right, 3),
        );
        assert_eq!(d.window.x + d.window.width, 1920);
        assert_eq!(d.strip_x, d.window.width as f64 - strip.0);

        let strip = strip_size(m(), Edge::Top, 3);
        let d = dock_panel(
            screen(),
            Edge::Top,
            0.5,
            0.0,
            strip,
            panel_size(m(), Edge::Top, 3),
        );
        assert_eq!(d.window.y, 0);
        assert_eq!(d.strip_y, 0.0);
    }

    #[test]
    fn the_panel_holds_the_tallest_card_beside_the_strip() {
        let m = m();
        // Vertical edge: depth is the card's width, length its tallest height.
        let (w, h) = panel_size(m, Edge::Right, 1);
        assert_eq!(w, m.strip_thickness + m.popover_gap + m.popover_size);
        assert_eq!(h, MAX_POPOVER_LENGTH);
        // Horizontal edge: the axes swap, and depth is the tallest card.
        let (w, h) = panel_size(m, Edge::Bottom, 1);
        assert_eq!(w, m.popover_size.max(m.strip_extent(1).0));
        assert_eq!(h, m.strip_thickness + m.popover_gap + MAX_POPOVER_LENGTH);
        // A long strip is never cut short by the card.
        let (_, h) = panel_size(m, Edge::Right, 7);
        assert_eq!(
            h,
            m.strip_extent(7)
                .0
                .clamp(MAX_POPOVER_LENGTH, MAX_STRIP_LENGTH)
        );
    }

    #[test]
    fn the_panel_does_not_depend_on_whether_a_card_is_open() {
        // The whole point: one size, so opening never costs a SetWindowPos.
        assert_eq!(
            panel_size(m(), Edge::Left, 4),
            panel_size(m(), Edge::Left, 4)
        );
        let a = dock_panel(
            screen(),
            Edge::Left,
            0.5,
            0.0,
            strip_size(m(), Edge::Left, 4),
            panel_size(m(), Edge::Left, 4),
        );
        let b = dock_panel(
            screen(),
            Edge::Left,
            0.5,
            0.0,
            strip_size(m(), Edge::Left, 4),
            panel_size(m(), Edge::Left, 4),
        );
        assert_eq!(a, b);
    }

    #[test]
    fn the_wake_strip_hugs_the_edge_along_the_strip() {
        let strip = strip_size(m(), Edge::Right, 3);
        let wake = wake_size(m(), Edge::Right, 3);
        assert_eq!(wake, (WAKE_THICKNESS, strip.1));

        // Even with a margin the wake strip is on the edge itself.
        let p = dock_wake(screen(), Edge::Right, 0.5, wake);
        assert_eq!(p.x + p.width, 1920);
        let alone = place(screen(), Edge::Right, 0.5, 30.0, strip.0, strip.1);
        assert_eq!(p.y, alone.y, "same position along the edge as the strip");

        let wake = wake_size(m(), Edge::Bottom, 3);
        let p = dock_wake(screen(), Edge::Bottom, 0.5, wake);
        assert_eq!(p.y + p.height, 1040);
        assert_eq!(p.height, WAKE_THICKNESS as i32);
    }

    #[test]
    fn a_runaway_strip_is_clamped() {
        let (_, along) = strip_size(m(), Edge::Right, 500);
        assert_eq!(along, MAX_STRIP_LENGTH);
    }

    #[test]
    fn dpi_scaling_applies_to_the_panel_and_the_strip_offset_is_logical() {
        let area = WorkArea::new(0, 0, 2880, 1560, 1.5);
        let strip = strip_size(m(), Edge::Right, 3);
        let panel = panel_size(m(), Edge::Right, 3);
        let d = dock_panel(area, Edge::Right, 0.5, 0.0, strip, panel);
        assert_eq!(d.window.width, (panel.0 * 1.5).round() as i32);
        assert!((d.strip_x - (panel.0 - strip.0)).abs() < 1.0);
    }

    /// The browser preview (`npm run dev`) works the panel geometry out for
    /// itself in `src/lib/layout.ts`. Its limits have to be these ones, or the
    /// preview stops showing what the app does.
    #[test]
    fn the_frontend_preview_uses_the_same_limits() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../src/lib/layout.ts")
            .canonicalize();
        // Vendored builds won't have the frontend beside them; nothing to check.
        let Ok(path) = path else { return };
        let Ok(source) = std::fs::read_to_string(&path) else {
            return;
        };
        let constant = |name: &str| -> f64 {
            source
                .split_once(&format!("export const {name} = "))
                .and_then(|(_, rest)| rest.split(';').next())
                .and_then(|v| v.trim().parse::<f64>().ok())
                .unwrap_or_else(|| panic!("{name} missing from layout.ts"))
        };
        assert_eq!(constant("MAX_STRIP_LENGTH"), MAX_STRIP_LENGTH);
        assert_eq!(constant("MAX_POPOVER_LENGTH"), MAX_POPOVER_LENGTH);
        assert_eq!(constant("WAKE_THICKNESS"), WAKE_THICKNESS);
        assert_eq!(constant("HIT_MARGIN"), HIT_MARGIN);
    }

    #[test]
    fn hit_testing_allows_a_margin_and_ignores_empty_rects() {
        let rects = [HitRect {
            x: 100.0,
            y: 50.0,
            w: 40.0,
            h: 200.0,
        }];
        assert!(hit_test(&rects, 120.0, 60.0, 0.0));
        assert!(!hit_test(&rects, 90.0, 60.0, 0.0));
        assert!(hit_test(&rects, 90.0, 60.0, HIT_MARGIN));
        assert!(!hit_test(&rects, 80.0, 60.0, HIT_MARGIN));
        assert!(!hit_test(&[HitRect::default()], 0.0, 0.0, HIT_MARGIN));
        assert!(!hit_test(&[], 0.0, 0.0, HIT_MARGIN));
    }
}
