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

/// Tallest the expanded card may grow, in logical pixels, before its own list
/// starts scrolling. Keeps a runaway session list from covering the screen.
pub const MAX_EXPANDED_HEIGHT: f64 = 620.0;

/// Logical size of the HUD for the current state.
///
/// `content_height` is what the webview measured for its own content; it is
/// clamped so neither a not-yet-measured 0 nor an enormous session list can
/// produce a silly window.
pub fn hud_extent(metrics: HudMetrics, expanded: bool, content_height: Option<f64>) -> (f64, f64) {
    if !expanded {
        return (metrics.collapsed_width, metrics.collapsed_height);
    }
    let height = content_height
        .filter(|h| h.is_finite() && *h > 0.0)
        .unwrap_or(metrics.expanded_min_height)
        .clamp(metrics.expanded_min_height, MAX_EXPANDED_HEIGHT);
    (metrics.expanded_width, height)
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
mod extent_tests {
    use super::*;
    use crate::config::HudSize;

    #[test]
    fn collapsed_uses_the_pill_metrics() {
        let m = HudSize::Medium.metrics();
        assert_eq!(
            hud_extent(m, false, Some(500.0)),
            (m.collapsed_width, m.collapsed_height),
            "a measured content height is irrelevant while collapsed"
        );
    }

    #[test]
    fn expanded_follows_the_measured_content() {
        let m = HudSize::Medium.metrics();
        let (w, h) = hud_extent(m, true, Some(300.0));
        assert_eq!(w, m.expanded_width);
        assert_eq!(h, 300.0);
    }

    #[test]
    fn an_unmeasured_card_falls_back_to_the_minimum() {
        let m = HudSize::Medium.metrics();
        assert_eq!(hud_extent(m, true, None).1, m.expanded_min_height);
        // A zero or negative measurement is a not-yet-laid-out webview.
        assert_eq!(hud_extent(m, true, Some(0.0)).1, m.expanded_min_height);
        assert_eq!(hud_extent(m, true, Some(-5.0)).1, m.expanded_min_height);
        assert_eq!(hud_extent(m, true, Some(f64::NAN)).1, m.expanded_min_height);
    }

    #[test]
    fn a_runaway_session_list_is_capped() {
        let m = HudSize::Medium.metrics();
        assert_eq!(hud_extent(m, true, Some(9000.0)).1, MAX_EXPANDED_HEIGHT);
    }

    #[test]
    fn tiny_content_still_clears_the_minimum() {
        let m = HudSize::Large.metrics();
        assert_eq!(hud_extent(m, true, Some(10.0)).1, m.expanded_min_height);
    }
}
