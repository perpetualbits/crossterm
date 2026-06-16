// SPDX-License-Identifier: LGPL-3.0-or-later
// Copyright (C) 2026  Epsilon Null Operation
//! Compilable version of the §2 "Getting started" snippet from
//! `docs/mullion-manual.md`.
//!
//! Keeping this as a real example (not a `no_run` doctest) ensures the
//! manual's API sketch stays in sync with the actual crate.  Run with:
//!
//! ```text
//! cargo run --example quickstart
//! ```

use mullion::border::{frame_tiles, BorderStyle, Borders, CornerStyle, LineWeight};
use mullion::layout::solve;
use mullion::style::Style;
use mullion::{Buffer, Constraint, Node, Orientation, Rect, Size};

const HEADER: u64 = 1;
const SIDEBAR: u64 = 2;
const MAIN: u64 = 3;

fn build() -> Node {
    Node::Split {
        orientation: Orientation::Vertical,
        children: vec![
            (Constraint::new(Size::Fixed(3)), Node::Tile(HEADER)),
            (Constraint::new(Size::Fill(1)), Node::Split {
                orientation: Orientation::Horizontal,
                children: vec![
                    (Constraint::new(Size::Fixed(20)), Node::Tile(SIDEBAR)),
                    (Constraint::new(Size::Fill(1)),   Node::Tile(MAIN)),
                ],
            }),
        ],
    }
}

fn draw(buf: &mut Buffer, root: &mut Node) {
    let style = BorderStyle {
        weight:  LineWeight::Light,
        corners: CornerStyle::Square,
        style:   Style::default(),
    };
    let rects   = solve(root, buf.area);                             // tree → [(TileId, Rect)]
    let content = frame_tiles(buf, &rects, Borders::ALL, &style);   // borders, returns interiors
    for (id, area) in content {
        match id {
            HEADER  => { buf.set_string(area.x, area.y, "mullion", Style::default()); }
            SIDEBAR => { /* paint sidebar into `area` */ }
            MAIN    => { /* paint main pane into `area` */ }
            _ => {}
        }
    }
}

fn main() {
    // Use an in-memory buffer so this runs without a real terminal.
    let area = Rect::new(0, 0, 80, 24);
    let mut root = build();
    let mut buf  = Buffer::empty(area);
    draw(&mut buf, &mut root);
    println!("Rendered {}×{} frame — quickstart OK.", area.width, area.height);
}
