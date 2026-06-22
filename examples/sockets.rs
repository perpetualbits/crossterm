// SPDX-License-Identifier: LGPL-3.0-or-later
// Copyright (C) 2026  Epsilon Null Operation
//! Phase 6 demo — sockets / ports (design note §5.1).
//!
//! A node (a bordered tile) carries sockets carved into its left edge (inputs)
//! and right edge (outputs). Each socket is a **bookended gap** in the border
//! (`┴●┬` on a side edge) with a circle terminal in the opening — `●` connected,
//! `○` idle. When connected, the circle glows with the streaming flow gradient.
//!
//! The socket geometry is pinned; only the gradient animates, and only because
//! the loop advances its clock.
//!
//! Keys
//!   ↑ / ↓ or + / -   add / remove a socket pair
//!   space            pause / resume (also toggles ● connected ↔ ○ idle)
//!   q                quit

use std::{io, time::Duration};

use crossterm::event::{Event, KeyCode, KeyEvent};

use mullion::{
    backend::CrosstermBackend,
    border::{draw_box, BorderStyle, Borders, CornerStyle},
    label::Side,
    poll_event,
    socket::{draw_socket, Flow, FlowStyle, Socket},
    style::{Color, Modifier, Style},
    Buffer, LineWeight, Rect, Terminal,
};

struct State {
    pairs: usize,
    t: f32,
    paused: bool,
}

/// Anchor offsets for `count` bookended sockets down an edge `edge_len` long,
/// stride 3 (so the `┴●┬` stacks never collide) and centred.
fn port_offsets(edge_len: u16, count: usize) -> Vec<u16> {
    let out = Vec::new();
    if edge_len < 5 {
        return out;
    }
    let (lo, hi) = (2u16, edge_len - 3); // valid anchor rows (caps clear of corners)
    let span = hi - lo;
    let max_n = (span / 3 + 1) as usize;
    let n = count.min(max_n);
    if n == 0 {
        return out;
    }
    let used = (n as u16 - 1) * 3;
    let start = lo + (span - used) / 2;
    (0..n).map(|k| start + k as u16 * 3).collect()
}

// ── Rendering ───────────────────────────────────────────────────────────────────

fn render(buf: &mut Buffer, st: &State) {
    let area = buf.area;
    if area.height < 8 || area.width < 20 {
        return;
    }
    let status_y = area.height - 1;
    let node_w = (area.width - 8).min(40);
    let node_h = (status_y - 2).clamp(7, 22);
    let node = Rect::new(
        (area.width - node_w) / 2,
        1 + (status_y.saturating_sub(1).saturating_sub(node_h)) / 2,
        node_w,
        node_h,
    );

    draw_box(buf, node, Borders::ALL, &BorderStyle {
        weight: LineWeight::Heavy,
        corners: CornerStyle::Rounded,
        style: Style::default().fg(Color::DarkGray),
    });
    if node.width as usize > 6 {
        buf.set_string(node.x + 2, node.y + node.height / 2, "node",
            Style::default().fg(Color::Gray).add_modifier(Modifier::BOLD));
    }

    let connected = !st.paused;
    for (i, &off) in port_offsets(node.height, st.pairs).iter().enumerate() {
        port(buf, node, Side::Left, off, i, st.t, connected);
        port(buf, node, Side::Right, off, i + 64, st.t, connected);
    }

    buf.set_string(0, 0, "sockets — ↑↓/+-:count  space:pause  q:quit",
        Style::default().fg(Color::White).add_modifier(Modifier::BOLD));
    let status = format!(" sockets: {} in / {} out   {}", st.pairs, st.pairs,
        if connected { "● connected (flow running)" } else { "○ idle (paused)" });
    let sstyle = Style::default().fg(Color::Black).bg(Color::Gray);
    for x in 0..area.width {
        buf.set_string(x, status_y, " ", sstyle);
    }
    buf.set_string(0, status_y, &status, sstyle);
}

/// Draw one bookended socket; when connected, recolour the circle with the flow
/// gradient so it pulses with "live" flow.
fn port(buf: &mut Buffer, node: Rect, side: Side, offset: u16, band: usize, t: f32, connected: bool) {
    let s = Socket::new(side, offset, if side == Side::Left { Flow::In } else { Flow::Out }, 0);
    let base = if connected {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::Gray)
    };
    draw_socket(buf, node, &s, connected, base);
    if connected {
        if let Some((ax, ay)) = s.anchor(node) {
            let style = FlowStyle { band, ..FlowStyle::default() }.color(0.5, t, true);
            buf.set_grapheme(ax, ay, "●", style);
        }
    }
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

fn run(term: &mut Terminal<CrosstermBackend<io::Stdout>>) -> io::Result<()> {
    let mut st = State { pairs: 3, t: 0.0, paused: false };
    loop {
        term.draw(|buf| render(buf, &st))?;
        match poll_event(Duration::from_millis(50))? {
            None if !st.paused => st.t += 0.05,
            None => {}
            Some(Event::Resize(_, _)) => {}
            Some(Event::Key(KeyEvent { code, .. })) => match code {
                KeyCode::Char('q') => break,
                KeyCode::Up | KeyCode::Char('+') => st.pairs = (st.pairs + 1).min(20),
                KeyCode::Down | KeyCode::Char('-') => st.pairs = st.pairs.saturating_sub(1),
                KeyCode::Char(' ') => st.paused = !st.paused,
                _ => {}
            },
            _ => {}
        }
    }
    Ok(())
}
