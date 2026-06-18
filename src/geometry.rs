// SPDX-License-Identifier: LGPL-3.0-or-later
// Copyright (C) 2026  Epsilon Null Operation
/// A rectangle in terminal cell coordinates.
///
/// The coordinate system places the origin `(0, 0)` at the top-left corner of
/// the terminal.  Both axes increase downward and rightward.  All values are in
/// **terminal cell units** (columns for x/width, rows for y/height).
///
/// `Rect` is used as both a region descriptor (the area a buffer covers) and a
/// clipping/intersection primitive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Rect {
    /// Column of the left edge (inclusive).
    pub x: u16,
    /// Row of the top edge (inclusive).
    pub y: u16,
    /// Number of columns.
    pub width: u16,
    /// Number of rows.
    pub height: u16,
}

impl Rect {
    /// Construct a `Rect` from its top-left corner and dimensions.
    pub fn new(x: u16, y: u16, width: u16, height: u16) -> Self {
        Self { x, y, width, height }
    }

    /// Return the total number of cells in the rectangle.
    ///
    /// Widened to `u32` to avoid overflow for large terminals (a 65535×65535
    /// rect would overflow `u16`).
    pub fn area(self) -> u32 {
        u32::from(self.width) * u32::from(self.height)
    }

    /// Return the column one past the right edge (exclusive right bound).
    ///
    /// `saturating_add` is used so that a rect at the maximum `u16` position
    /// never wraps around to 0.
    pub fn right(self) -> u16 {
        self.x.saturating_add(self.width)
    }

    /// Return the row one past the bottom edge (exclusive bottom bound).
    ///
    /// `saturating_add` is used for the same overflow-safety reason as `right`.
    pub fn bottom(self) -> u16 {
        self.y.saturating_add(self.height)
    }

    /// Return `true` if the rectangle has no cells.
    pub fn is_empty(self) -> bool {
        self.width == 0 || self.height == 0
    }

    /// Return `true` if the cell at `(x, y)` lies within this rectangle.
    ///
    /// The bounds are `[self.x, self.right())` and `[self.y, self.bottom())`,
    /// i.e. the right and bottom edges are exclusive.
    pub fn contains(self, x: u16, y: u16) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }

    /// Return the number of cells on the border perimeter.
    ///
    /// The border visits every cell on the outer ring of the rectangle exactly
    /// once, clockwise from the top-left corner.  For a rectangle with `width`
    /// W and `height` H, the count is `2*(W+H) - 4`; the four corner cells
    /// are shared between two edges but counted only once in the clockwise
    /// walk.
    ///
    /// Returns `self.area()` for rectangles smaller than 2×2 (where there is
    /// no distinct interior to distinguish from the border).
    pub fn border_len(self) -> u32 {
        if self.width < 2 || self.height < 2 {
            return self.area();
        }
        2 * (u32::from(self.width) + u32::from(self.height)) - 4
    }

    /// Return the normalised position `s ∈ [0, 1)` of cell `(x, y)` on the
    /// clockwise border perimeter, starting from the top-left corner.
    ///
    /// The walk order is:
    /// - **Top edge** (`y == self.y`): left → right, s = 0 … W/(2W+2H-4).
    /// - **Right edge** (`x == self.right()-1`): top → bottom.
    /// - **Bottom edge** (`y == self.bottom()-1`): right → left.
    /// - **Left edge** (`x == self.x`): bottom → top.
    ///
    /// Each corner belongs to the edge that first reaches it in the clockwise
    /// walk (so the top-left corner is at `s = 0`, top-right is on the top
    /// edge, bottom-right is on the right edge, and bottom-left is on the
    /// bottom edge).  Interior cells, and cells outside the rectangle, return
    /// `0.0` (same as the top-left corner — callers that need to distinguish
    /// them should check [`Rect::contains`] first).
    ///
    /// # Use case
    ///
    /// Feed the result into [`ease::gaussian`](crate::ease::gaussian) or a
    /// sinusoid to animate a smooth, wrap-around effect on a box border — for
    /// example a colour bump that travels continuously around the rectangle
    /// without a visible seam at the starting corner.
    ///
    /// ```
    /// use mullion::Rect;
    ///
    /// let r = Rect::new(0, 0, 5, 4); // 5 wide, 4 tall; border_len = 14
    /// assert_eq!(r.border_pos(0, 0), 0.0 / 14.0);  // top-left  (top edge, s=0)
    /// assert_eq!(r.border_pos(4, 0), 4.0 / 14.0);  // top-right (top edge, s=4)
    /// assert_eq!(r.border_pos(4, 3), 7.0 / 14.0);  // bot-right (right edge, s=4+3=7)
    /// assert_eq!(r.border_pos(0, 3), 11.0 / 14.0); // bot-left  (bottom edge, s=4+3+4=11)
    /// ```
    pub fn border_pos(self, x: u16, y: u16) -> f32 {
        if self.width < 2 || self.height < 2 {
            return 0.0;
        }
        let bx0 = self.x;
        let by0 = self.y;
        let bx1 = self.x + self.width - 1;
        let by1 = self.y + self.height - 1;
        let w = u32::from(self.width - 1);
        let h = u32::from(self.height - 1);
        let perim = (2 * (w + h)) as f32;

        let s: u32 = if y == by0 {
            u32::from(x.saturating_sub(bx0))          // top →
        } else if x == bx1 {
            w + u32::from(y.saturating_sub(by0))      // right ↓
        } else if y == by1 {
            w + h + u32::from(bx1.saturating_sub(x)) // bottom ←
        } else if x == bx0 {
            2 * w + h + u32::from(by1.saturating_sub(y)) // left ↑
        } else {
            return 0.0; // interior or outside
        };

        s as f32 / perim
    }

    /// Return the largest `Rect` that fits within both `self` and `other`.
    ///
    /// Computes the overlap by taking the maximum of the two left/top edges and
    /// the minimum of the two right/bottom edges.  Returns `Rect::default()`
    /// (zero-sized, at the origin) when the two rectangles are adjacent or
    /// non-overlapping — callers should check `is_empty()` before using the
    /// result.
    pub fn intersection(self, other: Rect) -> Rect {
        // The inner corners of the potential overlap region.
        let x1 = self.x.max(other.x);
        let y1 = self.y.max(other.y);
        let x2 = self.right().min(other.right());
        let y2 = self.bottom().min(other.bottom());
        // If the right edge did not extend past the left edge (or bottom past
        // top), the rectangles do not overlap.
        if x2 <= x1 || y2 <= y1 {
            Rect::default()
        } else {
            Rect { x: x1, y: y1, width: x2 - x1, height: y2 - y1 }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn area() {
        assert_eq!(Rect::new(0, 0, 10, 5).area(), 50);
        assert_eq!(Rect::new(0, 0, 0, 5).area(), 0);
    }

    #[test]
    fn contains() {
        let r = Rect::new(2, 3, 4, 5);
        assert!(r.contains(2, 3));
        assert!(r.contains(5, 7));
        assert!(!r.contains(6, 7)); // x == right()
        assert!(!r.contains(5, 8)); // y == bottom()
        assert!(!r.contains(1, 3));
    }

    #[test]
    fn intersection_overlap() {
        let a = Rect::new(0, 0, 10, 10);
        let b = Rect::new(5, 5, 10, 10);
        assert_eq!(a.intersection(b), Rect::new(5, 5, 5, 5));
    }

    #[test]
    fn intersection_no_overlap() {
        let a = Rect::new(0, 0, 5, 5);
        let b = Rect::new(10, 10, 5, 5);
        assert!(a.intersection(b).is_empty());
    }

    #[test]
    fn intersection_adjacent() {
        let a = Rect::new(0, 0, 5, 5);
        let b = Rect::new(5, 0, 5, 5);
        assert!(a.intersection(b).is_empty());
    }
}
