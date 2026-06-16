// SPDX-License-Identifier: LGPL-3.0-or-later
// Copyright (C) 2026  Epsilon Null Operation
//! Hit-testing primitives: screen point → tile or carousel.
//!
//! The two public functions operate on the output of [`solve`] or on a
//! compatible slice of `(TileId, Rect)` pairs.  Because [`solve`] already
//! returns the clipped, scrolled on-screen rects for all visible carousel
//! children, hit-testing works for smooth-scrolled carousels for free — no
//! additional layout math is needed.
//!
//! ## Composing hit regions
//!
//! An application that renders separate tree regions (e.g. a header via
//! [`render_shared`] and a body carousel via [`render_carousel`]) should call
//! these primitives once per region rather than using
//! [`InputRouter::handle_mouse`], which tests the whole effective subtree.
//!
//! [`solve`]: crate::layout::solve
//! [`render_shared`]: crate::border::render_shared
//! [`render_carousel`]: crate::render::render_carousel
//! [`InputRouter::handle_mouse`]: crate::input::InputRouter::handle_mouse

use crate::geometry::Rect;
use crate::layout::{carousel_visible_entries, partition, Axis, Node, TileId};

// ── tile_at ───────────────────────────────────────────────────────────────────

/// The leaf whose rect contains `(x, y)`, searching a precomputed rect list.
///
/// Iterates `rects` from first to last, retaining the id of each matching rect.
/// The **last** match is returned so that a child rect drawn on top of a parent
/// region resolves to the child rather than the parent.  For the non-overlapping
/// output of [`solve`](crate::layout::solve) at most one rect contains any given
/// point, making the tie-breaking rule a no-op in typical use.
///
/// # Returns
/// `Some(TileId)` of the last rect that contains `(x, y)`, or `None` if none do.
pub fn tile_at(rects: &[(TileId, Rect)], x: u16, y: u16) -> Option<TileId> {
    let mut result = None;
    for &(id, rect) in rects {
        if rect.contains(x, y) {
            // Keep updating: last match wins for overlapping rects.
            result = Some(id);
        }
    }
    result
}

// ── carousel_at ───────────────────────────────────────────────────────────────

/// The innermost [`Carousel`](crate::layout::Node::Carousel) node whose region
/// contains `(x, y)`, found by descending `root` within `area` using the same
/// geometry as [`solve`](crate::layout::solve).
///
/// The descent mirrors `solve_into`:
/// - For a `Split`, child areas are computed via the same partition algorithm
///   used by `solve` and only the child whose area contains the point is visited.
/// - For a `Carousel`, the carousel is recorded as the current candidate and
///   then the visible children are visited in case a deeper carousel exists.
///
/// `root` is taken as `&mut` for the same reason as [`solve`](crate::layout::solve):
/// `Orientation::Adaptive`
/// needs to write its chosen axis into `last` for hysteresis, and `Carousel::scroll`
/// is clamped in place.  Neither mutation is observable differently from a normal
/// `solve` call.
///
/// # Returns
/// `Some(carousel_id)` of the innermost matching carousel, or `None` when no
/// carousel's region contains `(x, y)`.
pub fn carousel_at(root: &mut Node, area: Rect, x: u16, y: u16) -> Option<TileId> {
    carousel_at_impl(root, area, x, y)
}

/// Recursive implementation shared by `carousel_at`.
///
/// `area` is the region the caller has assigned to `node` in the virtual layout
/// descent.  Returns the innermost carousel id found under `(x, y)`, or `None`.
fn carousel_at_impl(node: &mut Node, area: Rect, x: u16, y: u16) -> Option<TileId> {
    // Short-circuit: if the point is outside this node's area there is nothing
    // to find below it either — no child can contain a point its parent doesn't.
    if !area.contains(x, y) {
        return None;
    }

    match node {
        // Leaf tiles don't contain carousels.
        Node::Tile(_) => None,

        Node::Split { orientation, children } => {
            let axis = orientation.resolve(area);
            let total = match axis {
                Axis::Horizontal => area.width,
                Axis::Vertical => area.height,
            };
            let sizes = partition(children, total);
            let mut pos = match axis {
                Axis::Horizontal => area.x,
                Axis::Vertical => area.y,
            };
            for ((_, child), &size) in children.iter_mut().zip(sizes.iter()) {
                let child_area = match axis {
                    Axis::Horizontal => Rect::new(pos, area.y, size, area.height),
                    Axis::Vertical => Rect::new(area.x, pos, area.width, size),
                };
                // The containment check at the top of carousel_at_impl will
                // reject children whose area does not include the point.
                if let Some(id) = carousel_at_impl(child, child_area, x, y) {
                    return Some(id);
                }
                pos = pos.saturating_add(size);
            }
            None
        }

        Node::Carousel { id: carousel_id, orientation, scroll, children, .. } => {
            // This carousel's area contains the point — it is our candidate.
            let candidate = *carousel_id;

            let axis = orientation.resolve(area);
            let main_extent = match axis {
                Axis::Horizontal => area.width,
                Axis::Vertical => area.height,
            };
            let vp_main_origin = match axis {
                Axis::Horizontal => area.x,
                Axis::Vertical => area.y,
            };

            // Derive visible children using the same helper as solve_into so
            // the two paths cannot diverge in which tiles are considered visible.
            let extents: Vec<u16> = children.iter().map(|(e, _)| *e).collect();
            let (clamped, entries) = carousel_visible_entries(&extents, *scroll, main_extent);
            *scroll = clamped; // clamp in place, matching solve_into behaviour

            for (child_idx, v_start, ext) in entries {
                // Derive the on-screen clipped rect for this child, exactly as
                // solve_into does — see its inline comments for the arithmetic.
                let vis_start = v_start.max(clamped as u32);
                let vis_end = (v_start + ext as u32)
                    .min(clamped as u32 + main_extent as u32);
                let vis_len = (vis_end - vis_start) as u16;
                let screen_start = vp_main_origin + (vis_start - clamped as u32) as u16;
                let child_area = match axis {
                    Axis::Horizontal => Rect::new(screen_start, area.y, vis_len, area.height),
                    Axis::Vertical => Rect::new(area.x, screen_start, area.width, vis_len),
                };

                // Recurse to discover a deeper (inner) carousel.
                if let Some(inner) = carousel_at_impl(&mut children[child_idx].1, child_area, x, y) {
                    return Some(inner);
                }
            }

            // No deeper carousel was found — this one is the innermost.
            Some(candidate)
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{Constraint, Orientation, Size};

    // ── tile_at ───────────────────────────────────────────────────────────

    #[test]
    fn tile_at_hit_inside_each_rect() {
        let rects = vec![
            (0u64, Rect::new(0, 0, 20, 10)),
            (1u64, Rect::new(20, 0, 20, 10)),
        ];
        assert_eq!(tile_at(&rects, 5, 5), Some(0), "left tile");
        assert_eq!(tile_at(&rects, 25, 5), Some(1), "right tile");
    }

    #[test]
    fn tile_at_miss_in_gap() {
        // Rect 0 covers [0,10)×[0,10); rect 1 covers [11,21)×[0,10).
        // Cell x=10 falls in the one-cell gap between them.
        let rects = vec![
            (0u64, Rect::new(0, 0, 10, 10)),
            (1u64, Rect::new(11, 0, 10, 10)),
        ];
        assert_eq!(tile_at(&rects, 10, 5), None);
    }

    #[test]
    fn tile_at_last_match_wins_for_overlapping_rects() {
        // Parent covers (0,0,20,20); child (drawn later) covers (5,5,10,10).
        let rects = vec![
            (0u64, Rect::new(0, 0, 20, 20)),
            (1u64, Rect::new(5, 5, 10, 10)),
        ];
        assert_eq!(tile_at(&rects, 10, 10), Some(1), "child (later entry) must win");
        assert_eq!(tile_at(&rects, 1, 1), Some(0), "point only in parent");
    }

    #[test]
    fn tile_at_empty_list_returns_none() {
        assert_eq!(tile_at(&[], 5, 5), None);
    }

    #[test]
    fn tile_at_boundary_cells_inclusive_exclusive() {
        // Rect [2,6)×[3,8): corners x∈{2,5}, y∈{3,7}.
        let rects = vec![(42u64, Rect::new(2, 3, 4, 5))];
        assert_eq!(tile_at(&rects, 2, 3), Some(42), "top-left corner");
        assert_eq!(tile_at(&rects, 5, 7), Some(42), "bottom-right corner");
        assert_eq!(tile_at(&rects, 6, 5), None, "x == right() is exclusive");
        assert_eq!(tile_at(&rects, 4, 8), None, "y == bottom() is exclusive");
    }

    // ── carousel_at ───────────────────────────────────────────────────────

    fn h_carousel(id: TileId, scroll: u16, children: Vec<(u16, Node)>) -> Node {
        Node::Carousel { id, orientation: Orientation::Horizontal, scroll, children }
    }

    #[test]
    fn carousel_at_single_carousel_hit() {
        let area = Rect::new(0, 0, 40, 10);
        let mut node = h_carousel(7, 0, vec![(20, Node::Tile(0)), (20, Node::Tile(1))]);
        assert_eq!(carousel_at(&mut node, area, 10, 5), Some(7));
    }

    #[test]
    fn carousel_at_point_outside_area_returns_none() {
        let area = Rect::new(0, 0, 40, 10);
        let mut node = h_carousel(7, 0, vec![(20, Node::Tile(0))]);
        // x=50 is beyond the carousel's area.
        assert_eq!(carousel_at(&mut node, area, 50, 5), None);
    }

    #[test]
    fn carousel_at_nested_returns_innermost() {
        // Outer carousel (id=1): two children of 20 cells each.
        // Left child is a Tile; right child is an inner carousel (id=2).
        let area = Rect::new(0, 0, 40, 10);
        let inner = Node::Carousel {
            id: 2,
            orientation: Orientation::Horizontal,
            scroll: 0,
            children: vec![(20, Node::Tile(0))],
        };
        let mut outer = Node::Carousel {
            id: 1,
            orientation: Orientation::Horizontal,
            scroll: 0,
            children: vec![(20, Node::Tile(0)), (20, inner)],
        };
        // Point in the outer-only region (x=5, left child is a Tile) → outer.
        assert_eq!(carousel_at(&mut outer, area, 5, 5), Some(1));
        // Point in the inner carousel region (x=25) → innermost.
        assert_eq!(carousel_at(&mut outer, area, 25, 5), Some(2));
    }

    #[test]
    fn carousel_at_pure_split_tree_returns_none() {
        let area = Rect::new(0, 0, 40, 10);
        let mut node = Node::Split {
            orientation: Orientation::Horizontal,
            children: vec![
                (Constraint::new(Size::Fill(1)), Node::Tile(0)),
                (Constraint::new(Size::Fill(1)), Node::Tile(1)),
            ],
        };
        assert_eq!(carousel_at(&mut node, area, 10, 5), None);
    }

    #[test]
    fn carousel_at_carousel_inside_split() {
        // Split [Tile(0) | Carousel(id=5) [Tile(1), Tile(2)]] in 40×10.
        // Each half gets 20 cols: tiles x=0..20, carousel x=20..40.
        let area = Rect::new(0, 0, 40, 10);
        let carousel = Node::Carousel {
            id: 5,
            orientation: Orientation::Horizontal,
            scroll: 0,
            children: vec![(10, Node::Tile(1)), (10, Node::Tile(2))],
        };
        let mut node = Node::Split {
            orientation: Orientation::Horizontal,
            children: vec![
                (Constraint::new(Size::Fill(1)), Node::Tile(0)),
                (Constraint::new(Size::Fill(1)), carousel),
            ],
        };
        assert_eq!(carousel_at(&mut node, area, 5, 5), None, "tile half — no carousel");
        assert_eq!(carousel_at(&mut node, area, 25, 5), Some(5), "carousel half");
    }

    #[test]
    fn carousel_at_scrolled_carousel_uses_visible_rects() {
        // Carousel id=3, H, scroll=10: first 10-cell child is off-screen.
        // Viewport 10 wide, two children each 10 cells; scroll=10 makes child 1
        // the first visible.  A click anywhere in the viewport still returns the
        // carousel id since the whole area belongs to the carousel.
        let area = Rect::new(0, 0, 10, 5);
        let mut node = h_carousel(
            3, 10,
            vec![(10, Node::Tile(0)), (10, Node::Tile(1))],
        );
        assert_eq!(carousel_at(&mut node, area, 5, 2), Some(3));
    }
}
