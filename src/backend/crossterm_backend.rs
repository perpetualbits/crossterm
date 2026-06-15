use std::io::{self, Write};

use crossterm::{
    cursor::{Hide, MoveTo, Show},
    execute, queue,
    style::{
        Attribute, Color as CtColor, Colors, Print, ResetColor, SetAttribute,
        SetColors,
    },
    terminal::{
        self, disable_raw_mode, enable_raw_mode, size, EnterAlternateScreen,
        LeaveAlternateScreen,
    },
};

use crate::{
    backend::Backend,
    buffer::Cell,
    geometry::Rect,
    style::{Color, Modifier, Style},
};

// Synchronized-output DEC private mode sequences.
const BEGIN_SYNC: &[u8] = b"\x1b[?2026h";
const END_SYNC: &[u8] = b"\x1b[?2026l";

/// A [`Backend`] that drives a real terminal via `crossterm`.
///
/// ## Safety on exit
///
/// `CrosstermBackend` implements `Drop`: if [`enter`](Backend::enter) was
/// called but [`leave`](Backend::leave) was not (e.g. due to a panic or early
/// return), `Drop` calls `leave()` best-effort so the user's shell is restored.
/// [`enter`] also installs a panic hook that runs the restore sequences before
/// printing the panic message, keeping it readable in raw mode.
pub struct CrosstermBackend<W: Write> {
    writer: W,
    /// Last style emitted to the terminal; used to suppress redundant SGR sequences.
    last_style: Option<Style>,
    /// True after `enter()`, false after `leave()`. Guards the `Drop` restore.
    entered: bool,
}

impl<W: Write> CrosstermBackend<W> {
    pub fn new(writer: W) -> Self {
        Self { writer, last_style: None, entered: false }
    }

    /// Write the escape sequences that leave the alternate screen and show the
    /// cursor.  Does **not** call `disable_raw_mode` (a tty syscall), so this
    /// method can run against any `Write` sink including `Vec<u8>` in tests.
    fn write_restore(&mut self) -> io::Result<()> {
        execute!(self.writer, LeaveAlternateScreen, Show)
    }

    /// Simulate having entered interactive mode without calling `enable_raw_mode`.
    ///
    /// **Only for testing.** Sets the `entered` flag so that `Drop`/`leave()`
    /// will write the restore escape sequences, letting tests verify them
    /// against a `Vec<u8>` sink without requiring a real tty.
    #[doc(hidden)]
    pub fn mark_entered(&mut self) {
        self.entered = true;
    }
}

impl<W: Write> Drop for CrosstermBackend<W> {
    /// Restore the terminal if `enter()` was called but `leave()` was not.
    fn drop(&mut self) {
        if self.entered {
            let _ = self.leave();
        }
    }
}

/// Map a tile-engine [`Color`] to a crossterm color.
///
/// Naming convention: `White` is bright white (ANSI 15), `Gray` is standard
/// grey (ANSI 7), `DarkGray` is bright black (ANSI 8).
fn to_ct_color(c: Color) -> CtColor {
    match c {
        Color::Reset => CtColor::Reset,
        Color::Black => CtColor::Black,
        Color::Red => CtColor::DarkRed,
        Color::Green => CtColor::DarkGreen,
        Color::Yellow => CtColor::DarkYellow,
        Color::Blue => CtColor::DarkBlue,
        Color::Magenta => CtColor::DarkMagenta,
        Color::Cyan => CtColor::DarkCyan,
        Color::White => CtColor::White,       // bright white (ANSI 15)
        Color::DarkGray => CtColor::DarkGrey, // bright black (ANSI 8)
        Color::LightRed => CtColor::Red,
        Color::LightGreen => CtColor::Green,
        Color::LightYellow => CtColor::Yellow,
        Color::LightBlue => CtColor::Blue,
        Color::LightMagenta => CtColor::Magenta,
        Color::LightCyan => CtColor::Cyan,
        Color::Gray => CtColor::Grey,         // standard grey (ANSI 7)
        Color::Indexed(i) => CtColor::AnsiValue(i),
        Color::Rgb(r, g, b) => CtColor::Rgb { r, g, b },
    }
}

fn emit_style<W: Write>(w: &mut W, style: Style) -> io::Result<()> {
    // Reset all attributes first, then set the desired ones.
    queue!(w, ResetColor, SetAttribute(Attribute::Reset))?;
    queue!(w, SetColors(Colors::new(to_ct_color(style.fg), to_ct_color(style.bg))))?;
    if style.mods.contains(Modifier::BOLD) {
        queue!(w, SetAttribute(Attribute::Bold))?;
    }
    if style.mods.contains(Modifier::DIM) {
        queue!(w, SetAttribute(Attribute::Dim))?;
    }
    if style.mods.contains(Modifier::ITALIC) {
        queue!(w, SetAttribute(Attribute::Italic))?;
    }
    if style.mods.contains(Modifier::UNDERLINE) {
        queue!(w, SetAttribute(Attribute::Underlined))?;
    }
    if style.mods.contains(Modifier::REVERSE) {
        queue!(w, SetAttribute(Attribute::Reverse))?;
    }
    Ok(())
}

impl<W: Write> Backend for CrosstermBackend<W> {
    fn size(&self) -> io::Result<Rect> {
        let (w, h) = size()?;
        Ok(Rect::new(0, 0, w, h))
    }

    fn draw<'a>(
        &mut self,
        changes: impl Iterator<Item = (u16, u16, &'a Cell)>,
    ) -> io::Result<()> {
        for (x, y, cell) in changes {
            queue!(self.writer, MoveTo(x, y))?;
            let style = cell.style;
            if self.last_style != Some(style) {
                emit_style(&mut self.writer, style)?;
                self.last_style = Some(style);
            }
            queue!(self.writer, Print(&cell.symbol))?;
        }
        Ok(())
    }

    fn begin_frame(&mut self) -> io::Result<()> {
        self.writer.write_all(BEGIN_SYNC)?;
        Ok(())
    }

    fn end_frame(&mut self) -> io::Result<()> {
        self.writer.write_all(END_SYNC)?;
        self.flush()
    }

    fn clear(&mut self) -> io::Result<()> {
        execute!(
            self.writer,
            terminal::Clear(terminal::ClearType::All),
            MoveTo(0, 0)
        )
    }

    fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()
    }

    fn enter(&mut self) -> io::Result<()> {
        enable_raw_mode()?;
        execute!(self.writer, EnterAlternateScreen, Hide)?;
        self.entered = true;

        // Install a panic hook that restores the terminal before the panic
        // message is printed, so it remains readable in raw mode.
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let _ = disable_raw_mode();
            let _ = execute!(std::io::stderr(), LeaveAlternateScreen, Show);
            prev(info);
        }));

        Ok(())
    }

    fn leave(&mut self) -> io::Result<()> {
        let r = self.write_restore();
        let _ = disable_raw_mode(); // best-effort; harmless when no real tty
        self.entered = false;
        r
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_mapping_white_gray_darkgray() {
        assert_eq!(to_ct_color(Color::White), CtColor::White,
            "White must map to bright white (ANSI 15)");
        assert_eq!(to_ct_color(Color::Gray), CtColor::Grey,
            "Gray must map to standard grey (ANSI 7)");
        assert_eq!(to_ct_color(Color::DarkGray), CtColor::DarkGrey,
            "DarkGray must map to bright black (ANSI 8)");
    }

    #[test]
    fn leave_writes_restore_sequences() {
        let mut buf = Vec::<u8>::new();
        {
            let mut backend = CrosstermBackend::new(&mut buf);
            backend.entered = true; // test seam: skip enable_raw_mode
            backend.leave().unwrap();
            assert!(!backend.entered, "entered flag must be cleared");
        } // drop backend here to release the &mut buf borrow
        let out = String::from_utf8_lossy(&buf);
        assert!(out.contains("\x1b[?1049l"), "missing leave-alt-screen in: {out:?}");
        assert!(out.contains("\x1b[?25h"), "missing show-cursor in: {out:?}");
    }

    #[test]
    fn drop_writes_restore_sequences() {
        let mut buf = Vec::<u8>::new();
        {
            let mut backend = CrosstermBackend::new(&mut buf);
            backend.entered = true; // test seam: skip enable_raw_mode
        } // Drop triggers here; releases borrow so buf is readable below
        let out = String::from_utf8_lossy(&buf);
        assert!(out.contains("\x1b[?1049l"), "Drop must emit leave-alt-screen");
        assert!(out.contains("\x1b[?25h"), "Drop must emit show-cursor");
    }
}
