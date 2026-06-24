// SPDX-License-Identifier: LGPL-3.0-or-later
// Copyright (C) 2026  Epsilon Null Operation
//! A TV playing a synthesised colour signal through the [`Video`] widget — colour
//! bars, a reference strip, a moving luma ramp, and a bouncing highlight — to show
//! faithful reproduction, the two cell encodings, and the optional CRT/grading filters.
//!
//! Keys
//!   e              encoding: braille ↔ half-block
//!   1              scanlines       4   gamma
//!   2              vignette        5   saturation
//!   3              phosphor (amber) 6  greyscale
//!   space          pause / resume
//!   q              quit

use std::{io, time::Duration};

use crossterm::event::{Event, KeyCode, KeyEvent};

use mullion::{
    backend::CrosstermBackend,
    poll_event,
    style::{Color, Modifier, Style},
    video::{Encoding, Filter, Frame, Rgb, Video},
    Buffer, Rect, Terminal,
};

const FRAME_W: usize = 192;
const FRAME_H: usize = 144;

const FILTERS: [Filter; 6] = [
    Filter::Scanlines(0.4),
    Filter::Vignette(0.6),
    Filter::Phosphor { hue: 40.0, sat: 0.7 },
    Filter::Gamma(1.8),
    Filter::Saturation(1.8),
    Filter::Grayscale,
];
const FILTER_NAMES: [&str; 6] = ["scanlines", "vignette", "phosphor", "gamma", "saturation", "greyscale"];

struct State {
    t: f32,
    encoding: Encoding,
    filters: [bool; 6],
    paused: bool,
}

/// The test signal at normalised `(u, v)` and time `t`: SMPTE-ish colour bars over a
/// reference strip and a drifting luma ramp, with a bright disc bouncing across.
fn signal(u: f32, v: f32, t: f32) -> Rgb {
    const BARS: [Rgb; 7] = [
        (255, 255, 255), (255, 255, 0), (0, 255, 255), (0, 255, 0), (255, 0, 255), (255, 0, 0), (0, 0, 255),
    ];
    let base = if v < 0.66 {
        BARS[((u * 7.0) as usize).min(6)]
    } else if v < 0.74 {
        let g = if (u * 12.0) as usize % 2 == 0 { 210 } else { 20 };
        (g, g, g)
    } else {
        let g = ((u + t * 0.05).fract() * 255.0) as u8;
        (g, g, g)
    };
    // A bright disc drifting over the signal — motion and a moving edge for detail.
    let (cx, cy) = (0.5 + 0.34 * (t * 0.7).sin(), 0.33 + 0.16 * (t * 1.1).cos());
    let d = ((u - cx).powi(2) + (v - cy).powi(2) * 2.25).sqrt();
    if d < 0.07 {
        (255, 255, 255)
    } else {
        base
    }
}

fn frame_area(area: Rect) -> Rect {
    Rect::new(0, 1, area.width, area.height.saturating_sub(2))
}

fn render(buf: &mut Buffer, st: &State) {
    let area = buf.area;
    if area.height < 4 {
        return;
    }
    let t = st.t;
    let pixels: Vec<Rgb> = (0..FRAME_H)
        .flat_map(|y| (0..FRAME_W).map(move |x| (x, y)))
        .map(|(x, y)| signal((x as f32 + 0.5) / FRAME_W as f32, (y as f32 + 0.5) / FRAME_H as f32, t))
        .collect();
    let frame = Frame::from_rgb(FRAME_W, FRAME_H, pixels);

    let mut video = Video::new().encoding(st.encoding);
    for (i, &on) in st.filters.iter().enumerate() {
        if on {
            video = video.filter(FILTERS[i]);
        }
    }
    video.render_frame(buf, frame_area(area), &frame);

    buf.set_string(0, 0, "tv — e:encoding  1-6:filters  space:pause  q:quit",
        Style::default().fg(Color::White).add_modifier(Modifier::BOLD));
    let active: Vec<&str> = (0..6).filter(|&i| st.filters[i]).map(|i| FILTER_NAMES[i]).collect();
    let status = format!(" encoding: {}   filters: {}",
        match st.encoding { Encoding::Braille => "braille", Encoding::HalfBlock => "half-block" },
        if active.is_empty() { "none (faithful)".to_string() } else { active.join(", ") });
    let sstyle = Style::default().fg(Color::Black).bg(Color::Gray);
    for x in 0..area.width {
        buf.set_string(x, area.height - 1, " ", sstyle);
    }
    buf.set_string(0, area.height - 1, &status, sstyle);
}

fn main() -> io::Result<()> {
    let backend = CrosstermBackend::new(io::stdout());
    let mut term = Terminal::new(backend)?;
    term.enter()?;
    let result = run(&mut term);
    term.leave()?;
    result
}

fn run(term: &mut Terminal<CrosstermBackend<io::Stdout>>) -> io::Result<()> {
    let mut st = State { t: 0.0, encoding: Encoding::Braille, filters: [false; 6], paused: false };
    loop {
        term.draw(|buf| render(buf, &st))?;
        if let Some(Event::Key(KeyEvent { code, .. })) = poll_event(Duration::from_millis(60))? {
            match code {
                KeyCode::Char('q') => break,
                KeyCode::Char('e') => {
                    st.encoding = match st.encoding {
                        Encoding::Braille => Encoding::HalfBlock,
                        Encoding::HalfBlock => Encoding::Braille,
                    };
                }
                KeyCode::Char(' ') => st.paused = !st.paused,
                KeyCode::Char(c @ '1'..='6') => {
                    let i = c as usize - '1' as usize;
                    st.filters[i] = !st.filters[i];
                }
                _ => {}
            }
        }
        if !st.paused {
            st.t += 0.08;
        }
    }
    Ok(())
}
