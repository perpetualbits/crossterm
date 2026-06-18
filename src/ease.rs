// SPDX-License-Identifier: LGPL-3.0-or-later
// Copyright (C) 2026  Epsilon Null Operation
//! Animation easing and shape functions.
//!
//! Pure floating-point utilities that do not depend on any other mullion
//! module. Useful when animating layout weights, colours, or any other
//! smoothly varying quantity.
//!
//! # Easing
//!
//! The typical pattern is: maintain a `t: f32` in `[0, 1]` that advances with
//! the animation clock, then feed it through an easing function to produce the
//! value that drives a `Size::Fill(weight)` or a colour interpolation.
//!
//! ```
//! use mullion::ease::smoothstep;
//!
//! # let t = 0.5_f32;
//! let weight = 1.0 + smoothstep(t) * 399.0; // 1 → 400
//! ```
//!
//! # Color bumps on a border loop
//!
//! Combine [`gaussian`] with [`Rect::border_pos`](crate::Rect::border_pos) to
//! animate color bumps that travel around a rectangular border:
//!
//! ```no_run
//! use mullion::{Rect, ease::gaussian};
//!
//! fn bump_brightness(rect: Rect, x: u16, y: u16, t: f32) -> f32 {
//!     let s = rect.border_pos(x, y);
//!     let center = (0.3_f32 + t * 0.08).rem_euclid(1.0); // travels CW
//!     let diff = (s - center + 0.5).rem_euclid(1.0) - 0.5; // wrap-around distance
//!     gaussian(diff, 0.07)
//! }
//! ```

/// Smooth-step easing: `3t² − 2t³`.
///
/// Returns 0 at `t = 0`, 1 at `t = 1`, and has zero first-derivative at both
/// ends — an animation driven by this function starts and stops without an
/// abrupt jolt. Input is clamped to `[0, 1]`.
///
/// ```
/// use mullion::ease::smoothstep;
/// assert_eq!(smoothstep(0.0), 0.0);
/// assert_eq!(smoothstep(1.0), 1.0);
/// assert!((smoothstep(0.5) - 0.5).abs() < 1e-6);
/// ```
pub fn smoothstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Linear interpolation: `a + (b − a) × t`.
///
/// No clamping — `t` outside `[0, 1]` extrapolates freely.
///
/// ```
/// use mullion::ease::lerp;
/// assert!((lerp(0.0, 10.0, 0.3) - 3.0).abs() < 1e-6);
/// assert!((lerp(5.0, 15.0, 0.5) - 10.0).abs() < 1e-6);
/// ```
pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Normalised Gaussian kernel: `exp(−x² / (2σ²))`.
///
/// Returns values in `(0, 1]`, peaking at `1.0` when `x = 0`. The `sigma`
/// parameter controls the width — roughly 68 % of the area lies within `±σ`
/// of centre.
///
/// Useful for smooth colour bumps, highlight pulses, and any effect that
/// fades symmetrically away from a centre point. Pair with
/// [`Rect::border_pos`](crate::Rect::border_pos) to animate effects
/// that travel around a rectangular border loop.
///
/// ```
/// use mullion::ease::gaussian;
/// assert_eq!(gaussian(0.0, 1.0), 1.0);          // peak at centre
/// assert!(gaussian(1.0, 1.0) < gaussian(0.5, 1.0)); // falls off with distance
/// ```
pub fn gaussian(x: f32, sigma: f32) -> f32 {
    (-x * x / (2.0 * sigma * sigma)).exp()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smoothstep_endpoints() {
        assert_eq!(smoothstep(0.0), 0.0);
        assert_eq!(smoothstep(1.0), 1.0);
    }

    #[test]
    fn smoothstep_clamps_below() {
        assert_eq!(smoothstep(-0.5), 0.0);
    }

    #[test]
    fn smoothstep_clamps_above() {
        assert_eq!(smoothstep(1.5), 1.0);
    }

    #[test]
    fn smoothstep_symmetric_midpoint() {
        assert!((smoothstep(0.5) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn lerp_basic() {
        assert!((lerp(0.0, 10.0, 0.3) - 3.0).abs() < 1e-6);
    }

    #[test]
    fn lerp_extrapolates() {
        assert!((lerp(0.0, 10.0, 1.5) - 15.0).abs() < 1e-6);
    }

    #[test]
    fn gaussian_peak_is_one() {
        assert_eq!(gaussian(0.0, 1.0), 1.0);
    }

    #[test]
    fn gaussian_monotone_falloff() {
        assert!(gaussian(1.0, 1.0) < gaussian(0.5, 1.0));
        assert!(gaussian(2.0, 1.0) < gaussian(1.0, 1.0));
    }

    #[test]
    fn gaussian_width_controlled_by_sigma() {
        // Wider sigma → larger value at x=1.
        assert!(gaussian(1.0, 2.0) > gaussian(1.0, 0.5));
    }
}
