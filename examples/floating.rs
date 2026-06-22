// SPDX-License-Identifier: LGPL-3.0-or-later
// Copyright (C) 2026  Epsilon Null Operation
//! Phase 1 demo — floating tiles + the free space around them (design note §2).
//!
//! Two floating tiles sit at sub-rectangles inset from the parent's borders.
//! The cells *around* them — the free space the future text engine and connector
//! router consume — are shaded live:
//!
//! ```text
//! ·········░░░░·············  ·  free cell      (free_cells_in_window, gutter G)
//! ········░┌──┐░···········   ░  gutter band    (cleared by the gutter, not free)
//! ········░│B │░···········   ┌┐ floating tile  (FloatLayer::solve)
//! ········░└──┘░···········
//! ·························
//! ```
//!
//! Move a float and the shading recomputes every frame — bounded by the visible
//! window, never the whole canvas. This is the only thing Phase 1 ships: stable
//! placement plus the two free-space views. No text flow or routing yet (those
//! are Phases 5 and 8); this demo only *visualizes* the foundation they build on.
//!
//! Keys
//!   Tab                 switch the active float
//!   ←↓↑→ or h j k l     move the active float (parent-local, clamped in-bounds)
//!   [ / ]               shrink / grow the gutter kept clear around every float
//!   q                   quit

use std::collections::HashSet;
use std::{io, time::Duration};

use crossterm::event::{Event, KeyCode, KeyEvent};

use mullion::{
    backend::CrosstermBackend,
    border::{BorderStyle, Borders, CornerStyle},
    float::{free_cells_in_window, free_intervals_in_rows, FloatChild, FloatLayer, FloatRect},
    poll_event,
    style::{Color, Modifier, Style},
    Buffer, LineWeight, Rect, Terminal,
};

// ── Demo state ──────────────────────────────────────────────────────────────────

/// Caller-assigned, stable ids for the two floats — durable across re-solves,
/// exactly like tiling-leaf ids.
const FLOAT_A: u64 = 1;
const FLOAT_B: u64 = 2;

/// Mutable demo state: the two floats' parent-local placements, which one is
/// active, and the current gutter.
///
/// Placements are stored **parent-local** (the same model `FloatLayer` uses) so
/// they are independent of where the parent lands on screen; `clamp_to` re-pins
/// them inside the parent each frame in case a resize shrank it.
struct State {
    a: FloatRect,
    b: FloatRect,
    active: u64,
    gutter: u16,
}

impl State {
    /// Initial layout: two small boxes offset from the top-left.
    fn new() -> Self {
        Self {
            a: FloatRect::new(6, 3, 16, 6),
            b: FloatRect::new(34, 9, 18, 7),
            active: FLOAT_A,
            gutter: 1,
        }
    }

    /// A mutable handle to whichever float is currently active.
    fn active_mut(&mut self) -> &mut FloatRect {
        if self.active == FLOAT_A { &mut self.a } else { &mut self.b }
    }

    /// Build the `FloatLayer` from the current placements (back-to-front order).
    fn layer(&self) -> FloatLayer {
        FloatLayer::new()
            .with_child(FloatChild::new(FLOAT_A, self.a))
            .with_child(FloatChild::new(FLOAT_B, self.b))
    }

    /// Clamp both placements so each float stays fully inside a `parent`-sized
    /// area.  A float wider/taller than the parent is pinned to the origin.
    fn clamp_to(&mut self, parent: Rect) {
        for f in [&mut self.a, &mut self.b] {
            f.x = f.x.min(parent.width.saturating_sub(f.width));
            f.y = f.y.min(parent.height.saturating_sub(f.height));
        }
    }

    /// Nudge the active float by `(dx, dy)` parent-local cells, saturating at the
    /// top-left edge; the bottom-right edge is enforced by `clamp_to` next frame.
    fn nudge(&mut self, dx: i32, dy: i32) {
        let f = self.active_mut();
        f.x = (f.x as i32 + dx).max(0) as u16;
        f.y = (f.y as i32 + dy).max(0) as u16;
    }
}

// ── Rendering ───────────────────────────────────────────────────────────────────

/// Glyph marking a free cell (inside the parent, outside every gutter-grown
/// float) — the cells a runaround slot or a router channel could use.
const FREE: &str = "·";
/// Glyph marking a gutter cell — cleared by the gutter but not itself free.
const GUTTER: &str = "░";

/// Draw one frame: shade the free space, then draw the two floats on top.
///
/// Steps, in order:
/// 1. Reserve a help row and a status row; the rest is the float parent.
/// 2. Solve the floats to absolute, parent-clipped rects (`FloatLayer::solve`).
/// 3. Ask for the free cells over the visible window (`free_cells_in_window`),
///    the router view, and shade them; cells that are neither free nor inside a
///    float are the gutter band.
/// 4. Draw each float as a box, the active one heavy + accented.
/// 5. Read the per-row slots on the active float's mid-row
///    (`free_intervals_in_rows`, the text view) for the status line.
fn render(buf: &mut Buffer, state: &mut State) {
    let area = buf.area;
    if area.height < 4 {
        return; // too short to host help + status + a usable parent
    }

    // Row 0 is help, the last row is status; the band between is the parent the
    // floats live in.
    let help_y = 0;
    let status_y = area.height - 1;
    let parent = Rect::new(0, 1, area.width, status_y - 1);

    // Keep both floats inside the (possibly just-resized) parent before solving.
    state.clamp_to(parent);
    let layer = state.layer();
    let rects = layer.solve(parent); // (id, absolute clipped rect) pairs

    // The router view: every free cell in the visible window.  A HashSet gives
    // O(1) classification while shading.
    let just_rects: Vec<Rect> = rects.iter().map(|&(_, r)| r).collect();
    let free: HashSet<(u16, u16)> =
        free_cells_in_window(parent, &just_rects, state.gutter, parent)
            .into_iter()
            .collect();

    let free_style = Style::default().fg(Color::DarkGray);
    let gutter_style = Style::default().fg(Color::Blue);

    // Shade the parent: free cells get a dot, cells cleared by the gutter (not
    // free, but not inside a float either) get the gutter glyph, and float
    // interiors are left blank for the box pass below.
    for y in parent.y..parent.bottom() {
        for x in parent.x..parent.right() {
            if free.contains(&(x, y)) {
                buf.set_string(x, y, FREE, free_style);
            } else if !just_rects.iter().any(|r| r.contains(x, y)) {
                buf.set_string(x, y, GUTTER, gutter_style);
            }
            // else: inside a float → leave blank, the box pass draws here.
        }
    }

    // Draw the floats. The active one is Heavy + cyan so it reads as selected.
    for &(id, rect) in &rects {
        if rect.is_empty() {
            continue;
        }
        let is_active = id == state.active;
        let (weight, color) = if is_active {
            (LineWeight::Heavy, Color::Cyan)
        } else {
            (LineWeight::Light, Color::Gray)
        };
        let bstyle = BorderStyle {
            weight,
            corners: CornerStyle::Rounded,
            style: Style::default().fg(color),
        };
        mullion::border::draw_box(buf, rect, Borders::ALL, &bstyle);

        // Label the float just inside its top-left corner when there is room.
        if rect.width > 2 && rect.height > 1 {
            let name = if id == FLOAT_A { "A" } else { "B" };
            let label_style = Style::default()
                .fg(color)
                .add_modifier(if is_active { Modifier::BOLD } else { Modifier::empty() });
            buf.set_string(rect.x + 1, rect.y + 1, name, label_style);
        }
    }

    // ── Help row ──────────────────────────────────────────────────────────
    let help = "floating tiles — Tab:switch  hjkl/arrows:move  [ ]:gutter  q:quit";
    buf.set_string(0, help_y, help, Style::default().fg(Color::White).add_modifier(Modifier::BOLD));

    // ── Status row ────────────────────────────────────────────────────────
    // The text view: how many slots the active float's mid-row splits into.
    let active_rect = rects
        .iter()
        .find(|&&(id, _)| id == state.active)
        .map(|&(_, r)| r)
        .unwrap_or_default();
    let mid_row = active_rect.y + active_rect.height / 2;
    let slots = free_intervals_in_rows(parent, &just_rects, state.gutter, mid_row..mid_row + 1);
    let slot_str: String = slots
        .iter()
        .map(|iv| format!("[{},{})", iv.start, iv.end))
        .collect::<Vec<_>>()
        .join(" ");

    let af = if state.active == FLOAT_A { state.a } else { state.b };
    let status = format!(
        " active:{}  pos:({},{}) size:{}×{}  gutter:{}  free:{}  row {} slots: {}",
        if state.active == FLOAT_A { "A" } else { "B" },
        af.x, af.y, af.width, af.height,
        state.gutter,
        free.len(),
        mid_row,
        if slot_str.is_empty() { "(none)".into() } else { slot_str },
    );
    let st = Style::default().fg(Color::Black).bg(Color::Gray);
    for x in 0..area.width {
        buf.set_string(x, status_y, " ", st);
    }
    buf.set_string(0, status_y, &status, st);
}

// ── Main / event loop ───────────────────────────────────────────────────────────

fn main() -> io::Result<()> {
    let backend = CrosstermBackend::new(io::stdout());
    let mut term = Terminal::new(backend)?;
    term.enter()?;
    let result = run(&mut term);
    term.leave()?;
    result
}

/// Event loop: draw, then handle one key with a short poll timeout.
///
/// Movement and gutter edits mutate `State`; the next `draw` re-solves the floats
/// and re-shades the free space, so every change is reflected immediately.
fn run(term: &mut Terminal<CrosstermBackend<io::Stdout>>) -> io::Result<()> {
    let mut state = State::new();

    loop {
        term.draw(|buf| render(buf, &mut state))?;

        match poll_event(Duration::from_millis(50))? {
            None | Some(Event::Resize(_, _)) => {}
            Some(Event::Key(KeyEvent { code, .. })) => match code {
                KeyCode::Char('q') => break,
                KeyCode::Tab => {
                    // Toggle which float the movement keys drive.
                    state.active = if state.active == FLOAT_A { FLOAT_B } else { FLOAT_A };
                }
                KeyCode::Char('[') => state.gutter = state.gutter.saturating_sub(1),
                KeyCode::Char(']') => state.gutter = (state.gutter + 1).min(8),
                KeyCode::Left | KeyCode::Char('h') => state.nudge(-1, 0),
                KeyCode::Right | KeyCode::Char('l') => state.nudge(1, 0),
                KeyCode::Up | KeyCode::Char('k') => state.nudge(0, -1),
                KeyCode::Down | KeyCode::Char('j') => state.nudge(0, 1),
                _ => {}
            },
            _ => {}
        }
    }
    Ok(())
}
