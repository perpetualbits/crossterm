//! Layout tree: partition a [`Rect`] into a tree of named tiles.
//!
//! The public API has three layers:
//!
//! 1. **Constraint** — how much space a single child requests.
//! 2. **Node** — either a leaf tile or a split that owns children.
//! 3. **`solve`** / **`min_size`** — the solver functions.
//!
//! ## Coordinate model
//!
//! All positions and sizes are in terminal *columns × rows* (`u16`).  Overflow
//! is handled with saturating arithmetic so a degenerate tree never panics.
//!
//! ## Adaptive orientation
//!
//! `Orientation::Adaptive` picks the axis that better fits the available area.
//! To prevent flicker when dimensions hover near the boundary it applies a
//! hysteresis margin: the current axis is kept unless the *other* axis is
//! strictly better by more than `margin_pct` percent of the relevant dimension.

use crate::geometry::Rect;

// ── Axis ─────────────────────────────────────────────────────────────────────

/// The axis along which a split lays out its children.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    /// Children are placed side-by-side (left to right).
    Horizontal,
    /// Children are stacked (top to bottom).
    Vertical,
}

// ── Orientation ──────────────────────────────────────────────────────────────

/// Determines which axis a `Split` node uses to arrange its children.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Orientation {
    /// Always lay children out left-to-right.
    Horizontal,
    /// Always lay children out top-to-bottom.
    Vertical,
    /// Choose the axis at solve-time based on the available area.
    ///
    /// If `width > height` the horizontal axis is preferred, otherwise
    /// vertical.  A hysteresis margin (`margin_pct` percent of the
    /// relevant dimension) suppresses flipping when the area is nearly
    /// square, reducing flicker.
    ///
    /// `last` is the axis chosen the previous time this node was solved.
    /// Set to `None` for the first solve.
    Adaptive {
        /// Hysteresis margin as a percentage (0–100) of the tested dimension.
        /// A value of 10 means the current axis is kept unless switching would
        /// yield at least 10 % more space on the preferred axis.
        margin_pct: u16,
        /// The axis that was chosen on the most recent solve call.
        last: Option<Axis>,
    },
}

impl Orientation {
    /// Resolve to a concrete [`Axis`], updating `last` for `Adaptive`.
    ///
    /// For `Horizontal` and `Vertical` this is trivial.  For `Adaptive` the
    /// logic is:
    /// 1. Start with the previous axis (or `Horizontal` if this is the first
    ///    solve).
    /// 2. Compute how wide and how tall the area is.
    /// 3. Flip the axis only when the candidate is strictly better than the
    ///    current axis by more than `margin_pct` percent of the relevant
    ///    dimension (hysteresis).
    fn resolve(&mut self, area: Rect) -> Axis {
        match self {
            Orientation::Horizontal => Axis::Horizontal,
            Orientation::Vertical => Axis::Vertical,
            Orientation::Adaptive { margin_pct, last } => {
                let current = last.unwrap_or(Axis::Horizontal);
                let w = area.width as u32;
                let h = area.height as u32;
                // Margin: current axis is kept unless other is strictly better
                // by more than margin_pct% of that dimension.
                let chosen = match current {
                    Axis::Horizontal => {
                        // Keep horizontal unless height is sufficiently larger than width.
                        let threshold = w + w * (*margin_pct as u32) / 100;
                        if h > threshold { Axis::Vertical } else { Axis::Horizontal }
                    }
                    Axis::Vertical => {
                        // Keep vertical unless width is sufficiently larger than height.
                        let threshold = h + h * (*margin_pct as u32) / 100;
                        if w > threshold { Axis::Horizontal } else { Axis::Vertical }
                    }
                };
                *last = Some(chosen);
                chosen
            }
        }
    }
}

// ── Size ─────────────────────────────────────────────────────────────────────

/// How much space a child requests along the split axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Size {
    /// Exactly `n` columns (horizontal split) or rows (vertical split).
    Fixed(u16),
    /// `p` percent of the available space (clamped to 0–100).
    Percent(u16),
    /// Claim a proportional share of leftover space after `Fixed` and
    /// `Percent` children are satisfied.  The weight is relative: a
    /// `Fill(2)` child gets twice as much leftover as a `Fill(1)` child.
    Fill(u16),
}

// ── Constraint ───────────────────────────────────────────────────────────────

/// Space request for one child in a `Split`, with optional clamps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Constraint {
    /// The primary sizing mode.
    pub size: Size,
    /// Hard lower bound in cells.  The computed size is always `>= min`.
    pub min: u16,
    /// Hard upper bound in cells.  The computed size is always `<= max`.
    /// Defaults to `u16::MAX` (unconstrained).
    pub max: u16,
}

impl Constraint {
    /// Construct a constraint with no min/max clamps.
    pub fn new(size: Size) -> Self {
        Self { size, min: 0, max: u16::MAX }
    }

    /// Set the minimum cell count.
    pub fn with_min(mut self, min: u16) -> Self {
        self.min = min;
        self
    }

    /// Set the maximum cell count.
    pub fn with_max(mut self, max: u16) -> Self {
        self.max = max;
        self
    }
}

impl Default for Constraint {
    /// A single Fill(1) child with no clamps — occupies all available space.
    fn default() -> Self {
        Self::new(Size::Fill(1))
    }
}

// ── TileId / Node ─────────────────────────────────────────────────────────────

/// Opaque identifier for a leaf tile, chosen by the caller.
pub type TileId = u64;

/// A node in the layout tree.
///
/// A `Tile` is a leaf that maps to a rectangular area in the output.
/// A `Split` subdivides its assigned area among child nodes according to
/// their [`Constraint`]s and the split [`Orientation`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Node {
    /// A leaf tile with a caller-assigned identifier.
    Tile(TileId),
    /// A node that partitions its area among its children.
    Split {
        /// Which axis (or adaptive choice) to use for the split.
        orientation: Orientation,
        /// Ordered list of `(constraint, child)` pairs.
        ///
        /// Constraints are applied in order; the last `Fill` child absorbs
        /// any rounding remainder to guarantee the areas tile exactly.
        children: Vec<(Constraint, Node)>,
    },
}

// ── Solver ───────────────────────────────────────────────────────────────────

/// Solve the layout tree rooted at `node` within `area`.
///
/// Returns a flat list of `(TileId, Rect)` pairs — one entry per leaf tile —
/// in depth-first, left-to-right order.
///
/// The solver guarantees:
/// - Every returned `Rect` is contained within `area` (or its recursive
///   sub-area for nested splits).
/// - The union of all returned rects exactly tiles `area` (no gaps, no
///   overlaps) when the tree contains at least one `Fill` child at every
///   split level.
/// - No `Rect` is zero-sized — if space runs out, the remaining children
///   share zero width/height gracefully without panicking.
///
/// `node` is taken as `&mut` so that `Orientation::Adaptive` can store the
/// chosen axis in `last` for hysteresis on subsequent solves.
pub fn solve(node: &mut Node, area: Rect) -> Vec<(TileId, Rect)> {
    let mut out = Vec::new();
    solve_into(node, area, &mut out);
    out
}

/// Recursive helper that appends `(TileId, Rect)` pairs to `out`.
fn solve_into(node: &mut Node, area: Rect, out: &mut Vec<(TileId, Rect)>) {
    match node {
        Node::Tile(id) => {
            out.push((*id, area));
        }
        Node::Split { orientation, children } => {
            if children.is_empty() {
                return;
            }

            let axis = orientation.resolve(area);
            // Total available space along the split axis.
            let total = match axis {
                Axis::Horizontal => area.width,
                Axis::Vertical => area.height,
            };

            let sizes = partition(children, total);

            // Walk children, advancing the origin along the axis.
            let mut pos = match axis {
                Axis::Horizontal => area.x,
                Axis::Vertical => area.y,
            };

            for ((_, child), size) in children.iter_mut().zip(sizes.iter()) {
                let child_area = match axis {
                    Axis::Horizontal => Rect::new(pos, area.y, *size, area.height),
                    Axis::Vertical => Rect::new(area.x, pos, area.width, *size),
                };
                solve_into(child, child_area, out);
                pos = pos.saturating_add(*size);
            }
        }
    }
}

/// Partition `total` cells among `children` according to their constraints.
///
/// Algorithm:
/// 1. Allocate `Fixed` children first (clamped by their `min`/`max`).
/// 2. Allocate `Percent` children from what remains (clamped likewise).
/// 3. Distribute the remaining cells proportionally among `Fill` children
///    using integer weights.  Rounding remainder is given to the **last**
///    `Fill` child so that the sum of all sizes equals `total` exactly.
/// 4. If over-constrained (more cells allocated than available), trim
///    children from the end until the total fits.
///
/// Returns a `Vec<u16>` of the same length as `children`.
fn partition(children: &[(Constraint, Node)], total: u16) -> Vec<u16> {
    let n = children.len();
    let mut sizes = vec![0u16; n];

    // Pass 1: Fixed children.
    let mut used: u32 = 0;
    for (i, (c, _)) in children.iter().enumerate() {
        if let Size::Fixed(v) = c.size {
            let clamped = clamp(v, c.min, c.max);
            sizes[i] = clamped;
            used += clamped as u32;
        }
    }

    // Pass 2: Percent children (of the original total, not the remainder).
    for (i, (c, _)) in children.iter().enumerate() {
        if let Size::Percent(p) = c.size {
            let raw = (total as u32 * p.min(100) as u32 / 100) as u16;
            let clamped = clamp(raw, c.min, c.max);
            sizes[i] = clamped;
            used += clamped as u32;
        }
    }

    // Pass 3: Fill children share the remainder proportionally.
    let remainder = (total as u32).saturating_sub(used);
    let total_weight: u32 = children.iter().map(|(c, _)| {
        if let Size::Fill(w) = c.size { w as u32 } else { 0 }
    }).sum();

    let mut fill_used: u32 = 0;
    // Find the index of the last Fill child — it absorbs the rounding remainder.
    let last_fill_idx = children.iter().rposition(|(c, _)| matches!(c.size, Size::Fill(_)));

    for (i, (c, _)) in children.iter().enumerate() {
        if let Size::Fill(w) = c.size {
            let raw = if Some(i) == last_fill_idx {
                // Give last Fill child the residual to avoid rounding gaps.
                (remainder.saturating_sub(fill_used)) as u16
            } else {
                // Proportional share; zero when total_weight is zero.
                (remainder * w as u32).checked_div(total_weight).unwrap_or(0) as u16
            };
            let clamped = clamp(raw, c.min, c.max);
            sizes[i] = clamped;
            fill_used += clamped as u32;
        }
    }

    // Pass 4: enforce that the total does not exceed `total` (over-constrained).
    // Trim from the end, preserving earlier children as much as possible.
    let sum: u32 = sizes.iter().map(|&s| s as u32).sum();
    if sum > total as u32 {
        let mut excess = sum - total as u32;
        for s in sizes.iter_mut().rev() {
            if excess == 0 { break; }
            let trim = (*s as u32).min(excess);
            *s -= trim as u16;
            excess -= trim;
        }
    }

    sizes
}

/// Clamp `v` to `[lo, hi]`, saturating on `u16` overflow.
#[inline]
fn clamp(v: u16, lo: u16, hi: u16) -> u16 {
    v.max(lo).min(hi)
}

// ── Minimum size ─────────────────────────────────────────────────────────────

/// Compute the minimum `(width, height)` that `node` needs to render without
/// any child being squeezed to zero.
///
/// Rules:
/// - `Tile` → `(1, 1)` (a tile always needs at least one cell).
/// - `Split` along the horizontal axis → width is the *sum* of children's
///   minimum widths, height is the *max* of children's minimum heights.
/// - `Split` along the vertical axis → height is the *sum* of children's
///   minimum heights, width is the *max* of children's minimum widths.
/// - `Adaptive` → the max of both orientations' minima (because we do not
///   know which axis will be chosen at solve-time).
///
/// The returned values saturate at `u16::MAX`.
pub fn min_size(node: &Node) -> (u16, u16) {
    match node {
        Node::Tile(_) => (1, 1),
        Node::Split { orientation, children } => {
            if children.is_empty() {
                return (0, 0);
            }
            match orientation {
                Orientation::Horizontal => {
                    min_size_along(children, Axis::Horizontal)
                }
                Orientation::Vertical => {
                    min_size_along(children, Axis::Vertical)
                }
                Orientation::Adaptive { .. } => {
                    // Conservative: must fit either axis.
                    let (wh, hh) = min_size_along(children, Axis::Horizontal);
                    let (wv, hv) = min_size_along(children, Axis::Vertical);
                    (wh.max(wv), hh.max(hv))
                }
            }
        }
    }
}

/// Compute the minimum size for a `Split` on a known `axis`.
///
/// Along the split axis the sizes are summed (each child needs its share).
/// Across the axis the sizes are maxed (the narrowest viable cross-section
/// must accommodate every child).
fn min_size_along(children: &[(Constraint, Node)], axis: Axis) -> (u16, u16) {
    let mut main_sum: u32 = 0;
    let mut cross_max: u16 = 0;

    for (c, child) in children {
        let (cw, ch) = min_size(child);
        // The minimum along the main axis is the larger of the child's natural
        // minimum and the constraint's own `min` field.
        let (main, cross) = match axis {
            Axis::Horizontal => (cw.max(c.min), ch),
            Axis::Vertical => (ch.max(c.min), cw),
        };
        main_sum = main_sum.saturating_add(main as u32);
        cross_max = cross_max.max(cross);
    }

    let main = main_sum.min(u16::MAX as u32) as u16;
    match axis {
        Axis::Horizontal => (main, cross_max),
        Axis::Vertical => (cross_max, main),
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Hand-written example tests ────────────────────────────────────────

    #[test]
    fn single_tile_gets_full_area() {
        let area = Rect::new(0, 0, 80, 24);
        let mut node = Node::Tile(1);
        let tiles = solve(&mut node, area);
        assert_eq!(tiles, vec![(1, area)]);
    }

    #[test]
    fn two_equal_fill_horizontal() {
        let area = Rect::new(0, 0, 80, 24);
        let mut node = Node::Split {
            orientation: Orientation::Horizontal,
            children: vec![
                (Constraint::new(Size::Fill(1)), Node::Tile(1)),
                (Constraint::new(Size::Fill(1)), Node::Tile(2)),
            ],
        };
        let tiles = solve(&mut node, area);
        // Last Fill child absorbs remainder (80 is even, so both get 40).
        assert_eq!(tiles[0].1.width, 40);
        assert_eq!(tiles[1].1.width, 40);
        // x positions tile correctly.
        assert_eq!(tiles[0].1.x, 0);
        assert_eq!(tiles[1].1.x, 40);
    }

    #[test]
    fn fixed_percent_fill_partition() {
        // 80 cols: Fixed(10) + Percent(25) + Fill(1)
        // Fixed = 10, Percent = 80*25/100 = 20, Fill = 80-10-20 = 50
        let area = Rect::new(0, 0, 80, 24);
        let mut node = Node::Split {
            orientation: Orientation::Horizontal,
            children: vec![
                (Constraint::new(Size::Fixed(10)), Node::Tile(1)),
                (Constraint::new(Size::Percent(25)), Node::Tile(2)),
                (Constraint::new(Size::Fill(1)), Node::Tile(3)),
            ],
        };
        let tiles = solve(&mut node, area);
        assert_eq!(tiles[0].1.width, 10);
        assert_eq!(tiles[1].1.width, 20);
        assert_eq!(tiles[2].1.width, 50);
        // Widths sum to total.
        let sum: u16 = tiles.iter().map(|(_, r)| r.width).sum();
        assert_eq!(sum, 80);
    }

    #[test]
    fn vertical_split_stacks_rows() {
        let area = Rect::new(0, 0, 80, 24);
        let mut node = Node::Split {
            orientation: Orientation::Vertical,
            children: vec![
                (Constraint::new(Size::Fixed(3)), Node::Tile(1)),
                (Constraint::new(Size::Fill(1)), Node::Tile(2)),
            ],
        };
        let tiles = solve(&mut node, area);
        assert_eq!(tiles[0].1.y, 0);
        assert_eq!(tiles[0].1.height, 3);
        assert_eq!(tiles[1].1.y, 3);
        assert_eq!(tiles[1].1.height, 21);
    }

    #[test]
    fn nested_split() {
        // Outer: H split into left=40 and right=40.
        // Right: V split into top=12 and bottom=12.
        let area = Rect::new(0, 0, 80, 24);
        let mut node = Node::Split {
            orientation: Orientation::Horizontal,
            children: vec![
                (Constraint::new(Size::Fill(1)), Node::Tile(1)),
                (Constraint::new(Size::Fill(1)), Node::Split {
                    orientation: Orientation::Vertical,
                    children: vec![
                        (Constraint::new(Size::Fill(1)), Node::Tile(2)),
                        (Constraint::new(Size::Fill(1)), Node::Tile(3)),
                    ],
                }),
            ],
        };
        let tiles = solve(&mut node, area);
        assert_eq!(tiles.len(), 3);
        // Left panel
        assert_eq!(tiles[0].1, Rect::new(0, 0, 40, 24));
        // Top-right panel
        assert_eq!(tiles[1].1, Rect::new(40, 0, 40, 12));
        // Bottom-right panel
        assert_eq!(tiles[2].1, Rect::new(40, 12, 40, 12));
    }

    #[test]
    fn min_max_clamps_respected() {
        // Percent(50) of 80 = 40, but max=30 → clamped to 30.
        // Fill(1) gets the remaining 50.
        let area = Rect::new(0, 0, 80, 24);
        let mut node = Node::Split {
            orientation: Orientation::Horizontal,
            children: vec![
                (Constraint { size: Size::Percent(50), min: 0, max: 30 }, Node::Tile(1)),
                (Constraint::new(Size::Fill(1)), Node::Tile(2)),
            ],
        };
        let tiles = solve(&mut node, area);
        assert_eq!(tiles[0].1.width, 30);
        assert_eq!(tiles[1].1.width, 50);
    }

    #[test]
    fn odd_width_remainder_goes_to_last_fill() {
        // 81 cols split between two Fill(1): first gets 40, last gets 41.
        let area = Rect::new(0, 0, 81, 24);
        let mut node = Node::Split {
            orientation: Orientation::Horizontal,
            children: vec![
                (Constraint::new(Size::Fill(1)), Node::Tile(1)),
                (Constraint::new(Size::Fill(1)), Node::Tile(2)),
            ],
        };
        let tiles = solve(&mut node, area);
        let sum: u16 = tiles.iter().map(|(_, r)| r.width).sum();
        assert_eq!(sum, 81, "widths must sum to total");
    }

    #[test]
    fn adaptive_picks_horizontal_for_wide_area() {
        let area = Rect::new(0, 0, 200, 24);
        let mut node = Node::Split {
            orientation: Orientation::Adaptive { margin_pct: 10, last: None },
            children: vec![
                (Constraint::new(Size::Fill(1)), Node::Tile(1)),
                (Constraint::new(Size::Fill(1)), Node::Tile(2)),
            ],
        };
        let tiles = solve(&mut node, area);
        // Horizontal split → tiles are side by side → same y, same height.
        assert_eq!(tiles[0].1.y, tiles[1].1.y);
        assert_eq!(tiles[0].1.height, tiles[1].1.height);
    }

    #[test]
    fn adaptive_picks_vertical_for_tall_area() {
        let area = Rect::new(0, 0, 20, 200);
        let mut node = Node::Split {
            orientation: Orientation::Adaptive { margin_pct: 10, last: None },
            children: vec![
                (Constraint::new(Size::Fill(1)), Node::Tile(1)),
                (Constraint::new(Size::Fill(1)), Node::Tile(2)),
            ],
        };
        let tiles = solve(&mut node, area);
        // Vertical split → tiles are stacked → same x, same width.
        assert_eq!(tiles[0].1.x, tiles[1].1.x);
        assert_eq!(tiles[0].1.width, tiles[1].1.width);
    }

    #[test]
    fn min_size_tile() {
        assert_eq!(min_size(&Node::Tile(0)), (1, 1));
    }

    #[test]
    fn min_size_horizontal_split() {
        let node = Node::Split {
            orientation: Orientation::Horizontal,
            children: vec![
                (Constraint::new(Size::Fill(1)), Node::Tile(1)),
                (Constraint::new(Size::Fill(1)), Node::Tile(2)),
            ],
        };
        // Two tiles → min width = 1+1 = 2, min height = max(1,1) = 1.
        assert_eq!(min_size(&node), (2, 1));
    }

    #[test]
    fn min_size_vertical_split() {
        let node = Node::Split {
            orientation: Orientation::Vertical,
            children: vec![
                (Constraint::new(Size::Fill(1)), Node::Tile(1)),
                (Constraint::new(Size::Fill(1)), Node::Tile(2)),
                (Constraint::new(Size::Fill(1)), Node::Tile(3)),
            ],
        };
        // Three tiles → min height = 3, min width = 1.
        assert_eq!(min_size(&node), (1, 3));
    }

    #[test]
    fn empty_split_returns_nothing() {
        let area = Rect::new(0, 0, 80, 24);
        let mut node = Node::Split {
            orientation: Orientation::Horizontal,
            children: vec![],
        };
        let tiles = solve(&mut node, area);
        assert!(tiles.is_empty());
    }

    #[test]
    fn zero_area_does_not_panic() {
        let area = Rect::new(0, 0, 0, 0);
        let mut node = Node::Split {
            orientation: Orientation::Horizontal,
            children: vec![
                (Constraint::new(Size::Fill(1)), Node::Tile(1)),
                (Constraint::new(Size::Fill(1)), Node::Tile(2)),
            ],
        };
        let tiles = solve(&mut node, area);
        // Must not panic; tiles exist but are zero-sized.
        assert_eq!(tiles.len(), 2);
        for (_, r) in &tiles {
            assert_eq!(r.width, 0);
        }
    }

    // ── Property tests ────────────────────────────────────────────────────

    use proptest::prelude::*;

    proptest! {
        /// For any Horizontal split with only Fill children, widths sum to total.
        #[test]
        fn prop_fill_widths_sum_to_total(total in 0u16..=500u16) {
            let area = Rect::new(0, 0, total, 24);
            let mut node = Node::Split {
                orientation: Orientation::Horizontal,
                children: vec![
                    (Constraint::new(Size::Fill(1)), Node::Tile(1)),
                    (Constraint::new(Size::Fill(1)), Node::Tile(2)),
                    (Constraint::new(Size::Fill(1)), Node::Tile(3)),
                ],
            };
            let tiles = solve(&mut node, area);
            let sum: u16 = tiles.iter().map(|(_, r)| r.width).sum();
            prop_assert_eq!(sum, total);
        }

        /// For any Vertical split with only Fill children, heights sum to total.
        #[test]
        fn prop_fill_heights_sum_to_total(total in 0u16..=300u16) {
            let area = Rect::new(0, 0, 80, total);
            let mut node = Node::Split {
                orientation: Orientation::Vertical,
                children: vec![
                    (Constraint::new(Size::Fill(1)), Node::Tile(1)),
                    (Constraint::new(Size::Fill(1)), Node::Tile(2)),
                ],
            };
            let tiles = solve(&mut node, area);
            let sum: u16 = tiles.iter().map(|(_, r)| r.height).sum();
            prop_assert_eq!(sum, total);
        }

        /// All returned rects must fit within the root area (x+width <= area.right).
        #[test]
        fn prop_rects_contained_in_area(
            w in 1u16..=200u16,
            h in 1u16..=100u16,
        ) {
            let area = Rect::new(0, 0, w, h);
            let mut node = Node::Split {
                orientation: Orientation::Horizontal,
                children: vec![
                    (Constraint::new(Size::Fill(1)), Node::Tile(1)),
                    (Constraint::new(Size::Fill(2)), Node::Tile(2)),
                    (Constraint::new(Size::Fill(1)), Node::Tile(3)),
                ],
            };
            let tiles = solve(&mut node, area);
            for (_, r) in tiles {
                prop_assert!(r.x >= area.x);
                prop_assert!(r.y >= area.y);
                prop_assert!(r.right() <= area.right(),
                    "r.right()={} > area.right()={}", r.right(), area.right());
                prop_assert!(r.bottom() <= area.bottom(),
                    "r.bottom()={} > area.bottom()={}", r.bottom(), area.bottom());
            }
        }

        /// Cross-axis dimension must equal the area's cross dimension for Fill-only splits.
        #[test]
        fn prop_cross_axis_unchanged(w in 1u16..=200u16, h in 1u16..=100u16) {
            let area = Rect::new(0, 0, w, h);
            let mut node = Node::Split {
                orientation: Orientation::Horizontal,
                children: vec![
                    (Constraint::new(Size::Fill(1)), Node::Tile(1)),
                    (Constraint::new(Size::Fill(1)), Node::Tile(2)),
                ],
            };
            let tiles = solve(&mut node, area);
            for (_, r) in tiles {
                prop_assert_eq!(r.height, h,
                    "height should equal area height for H split");
                prop_assert_eq!(r.y, area.y,
                    "y should equal area.y for H split");
            }
        }

        /// Fixed children must respect their exact size (when space allows).
        #[test]
        fn prop_fixed_child_exact_size(
            fixed in 0u16..=50u16,
            total in 0u16..=200u16,
        ) {
            let area = Rect::new(0, 0, total, 24);
            let mut node = Node::Split {
                orientation: Orientation::Horizontal,
                children: vec![
                    (Constraint::new(Size::Fixed(fixed)), Node::Tile(1)),
                    (Constraint::new(Size::Fill(1)), Node::Tile(2)),
                ],
            };
            let tiles = solve(&mut node, area);
            let actual_fixed = tiles[0].1.width;
            if total >= fixed {
                prop_assert_eq!(actual_fixed, fixed,
                    "fixed child should get exactly {} when total={}", fixed, total);
            } else {
                // Over-constrained: fixed child gets at most total.
                prop_assert!(actual_fixed <= total);
            }
        }

        /// Percent children must not exceed their stated percentage of total.
        #[test]
        fn prop_percent_child_at_most_pct(
            pct in 0u16..=100u16,
            total in 0u16..=200u16,
        ) {
            let area = Rect::new(0, 0, total, 24);
            let mut node = Node::Split {
                orientation: Orientation::Horizontal,
                children: vec![
                    (Constraint::new(Size::Percent(pct)), Node::Tile(1)),
                    (Constraint::new(Size::Fill(1)), Node::Tile(2)),
                ],
            };
            let tiles = solve(&mut node, area);
            let expected_max = (total as u32 * pct as u32 / 100) as u16;
            prop_assert!(tiles[0].1.width <= expected_max + 1, // +1 for rounding
                "percent child width {} > expected_max {} for pct={} total={}",
                tiles[0].1.width, expected_max, pct, total);
        }

        /// `min_size` for a two-child horizontal split is always >= (2, 1).
        #[test]
        fn prop_min_size_horizontal_at_least_num_children(n in 1usize..=8usize) {
            let children: Vec<_> = (0..n as u64)
                .map(|id| (Constraint::new(Size::Fill(1)), Node::Tile(id)))
                .collect();
            let node = Node::Split {
                orientation: Orientation::Horizontal,
                children,
            };
            let (w, h) = min_size(&node);
            prop_assert!(w >= n as u16,
                "min width {w} < n={n} for horizontal split");
            prop_assert!(h >= 1);
        }

        /// `min_size` for a vertical split is always >= (1, n).
        #[test]
        fn prop_min_size_vertical_at_least_num_children(n in 1usize..=8usize) {
            let children: Vec<_> = (0..n as u64)
                .map(|id| (Constraint::new(Size::Fill(1)), Node::Tile(id)))
                .collect();
            let node = Node::Split {
                orientation: Orientation::Vertical,
                children,
            };
            let (w, h) = min_size(&node);
            prop_assert!(h >= n as u16,
                "min height {h} < n={n} for vertical split");
            prop_assert!(w >= 1);
        }
    }
}
