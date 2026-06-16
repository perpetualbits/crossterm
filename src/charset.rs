// SPDX-License-Identifier: LGPL-3.0-or-later
// Copyright (C) 2026  Epsilon Null Operation
//! ASCII fallback for Unicode box-drawing characters.
//!
//! [`box_to_ascii`] maps every glyph the junction resolver can produce to one
//! of three ASCII characters — `'-'`, `'|'`, or `'+'` — leaving all other
//! content untouched.  The backend applies this automatically when
//! [`CrosstermBackend::set_unicode(false)`](crate::CrosstermBackend::set_unicode)
//! is active, which
//! [`apply_capabilities`](crate::CrosstermBackend::apply_capabilities) sets
//! from [`Capabilities::unicode`](crate::capabilities::Capabilities::unicode).

/// Map a box-drawing glyph to its closest ASCII equivalent.
///
/// | Class                                     | Result |
/// |-------------------------------------------|--------|
/// | Horizontal through-lines and stubs        | `'-'`  |
/// | Vertical through-lines and stubs          | `'|'`  |
/// | Corners, tees, crosses, rounded corners   | `'+'`  |
/// | Any other character (content, spaces, …)  | identity |
///
/// Covers every glyph that [`junction::resolve`](crate::junction::resolve) can
/// produce: the full light/heavy/mixed-weight set, the pure-double set (`═ ║
/// ╔╗╚╝ …`), and the four rounded corners used by
/// [`CornerStyle::Rounded`](crate::border::CornerStyle).
pub fn box_to_ascii(ch: char) -> char {
    match ch {
        // ── Horizontal lines and stubs (→ '-') ──────────────────────────────
        // Straight horizontals: light, heavy, double
        '─' | '━' | '═'
        // Mixed-weight horizontal through-lines
        | '╼' | '╾'
        // Single-arm horizontal stubs: light-right, heavy-right, light-left, heavy-left
        | '╶' | '╺' | '╴' | '╸'
        => '-',

        // ── Vertical lines and stubs (→ '|') ────────────────────────────────
        // Straight verticals: light, heavy, double
        '│' | '┃' | '║'
        // Mixed-weight vertical through-lines
        | '╽' | '╿'
        // Single-arm vertical stubs: light-down, heavy-down, light-up, heavy-up
        | '╷' | '╻' | '╵' | '╹'
        => '|',

        // ── Corners (→ '+') ─────────────────────────────────────────────────
        // Light/heavy top-left (down + right)
        '┌' | '┍' | '┎' | '┏'
        // Light/heavy top-right (down + left)
        | '┐' | '┑' | '┒' | '┓'
        // Light/heavy bottom-left (up + right)
        | '└' | '┕' | '┖' | '┗'
        // Light/heavy bottom-right (up + left)
        | '┘' | '┙' | '┚' | '┛'
        // Double corners
        | '╔' | '╗' | '╚' | '╝'
        // Rounded corners (CornerStyle::Rounded)
        | '╭' | '╮' | '╰' | '╯'

        // ── Down tees, up arm absent (→ '+') ────────────────────────────────
        | '┬' | '┭' | '┮' | '┯' | '┰' | '┱' | '┲' | '┳'

        // ── Up tees, down arm absent (→ '+') ────────────────────────────────
        | '┴' | '┵' | '┶' | '┷' | '┸' | '┹' | '┺' | '┻'

        // ── Right tees, left arm absent (→ '+') ─────────────────────────────
        | '├' | '┝' | '┞' | '┟' | '┠' | '┡' | '┢' | '┣'

        // ── Left tees, right arm absent (→ '+') ─────────────────────────────
        | '┤' | '┥' | '┦' | '┧' | '┨' | '┩' | '┪' | '┫'

        // Double tees
        | '╠' | '╣' | '╦' | '╩'

        // ── Crosses, all four arms present (→ '+') ───────────────────────────
        // All 16 light/heavy arm combinations
        | '┼' | '┽' | '┾' | '┿'
        | '╀' | '╁' | '╂' | '╃' | '╄' | '╅' | '╆' | '╇' | '╈' | '╉' | '╊' | '╋'
        // Double cross
        | '╬'
        => '+',

        // Non-box characters — content text, spaces, wide graphemes — are
        // returned unchanged; the application is responsible for wide-glyph
        // fallback if needed.
        _ => ch,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::border::LineWeight;
    use crate::junction::{resolve, EdgeCell};

    /// Drive every arm combination the resolver can produce and assert each
    /// output maps to `-`, `|`, or `+`.  This exhaustive check ensures
    /// `box_to_ascii` stays in sync with the junction table as glyphs are
    /// added or changed.
    #[test]
    fn all_resolver_glyphs_map_to_ascii_line_or_cross() {
        let arms = [
            None,
            Some(LineWeight::Light),
            Some(LineWeight::Heavy),
            Some(LineWeight::Double),
        ];
        let mut covered = 0usize;
        for &up in &arms {
            for &down in &arms {
                for &left in &arms {
                    for &right in &arms {
                        let cell = EdgeCell { up, down, left, right };
                        if let Some(ch) = resolve(&cell) {
                            let ascii = box_to_ascii(ch);
                            assert!(
                                matches!(ascii, '-' | '|' | '+'),
                                "box_to_ascii({ch:?}) = {ascii:?}, must be '-', '|', or '+' \
                                 (EdgeCell {{ up:{up:?}, down:{down:?}, left:{left:?}, right:{right:?} }})",
                            );
                            covered += 1;
                        }
                    }
                }
            }
        }
        // Sanity: the resolver must have produced at least the 80 non-empty
        // light/heavy glyphs plus the 11 double glyphs.
        assert!(covered >= 80, "expected ≥80 resolver outputs, got {covered}");
    }

    #[test]
    fn horizontal_chars_map_to_dash() {
        for ch in ['─', '━', '═', '╼', '╾', '╶', '╺', '╴', '╸'] {
            assert_eq!(box_to_ascii(ch), '-', "'{ch}' must map to '-'");
        }
    }

    #[test]
    fn vertical_chars_map_to_pipe() {
        for ch in ['│', '┃', '║', '╽', '╿', '╷', '╻', '╵', '╹'] {
            assert_eq!(box_to_ascii(ch), '|', "'{ch}' must map to '|'");
        }
    }

    #[test]
    fn corners_tees_crosses_map_to_plus() {
        for ch in [
            '┌', '┐', '└', '┘', '╔', '╗', '╚', '╝', '╭', '╮', '╰', '╯',
            '┬', '┴', '├', '┤', '╦', '╩', '╠', '╣',
            '┼', '╬', '╋',
        ] {
            assert_eq!(box_to_ascii(ch), '+', "'{ch}' must map to '+'");
        }
    }

    #[test]
    fn non_box_chars_are_identity() {
        for ch in ['A', 'z', ' ', '0', '/', '!', '\n'] {
            assert_eq!(box_to_ascii(ch), ch, "non-box '{ch:?}' must be unchanged");
        }
    }
}
