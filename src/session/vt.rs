use vt100::Parser;

/// Information about a single character cell in the virtual terminal grid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VtCell {
    /// Character or grapheme cluster in this cell.
    pub text: String,
    /// Foreground color.
    pub fg: Option<vt100::Color>,
    /// Background color.
    pub bg: Option<vt100::Color>,
    /// Whether bold styling is enabled.
    pub bold: bool,
    /// Whether italic styling is enabled.
    pub italic: bool,
    /// Whether underline styling is enabled.
    pub underline: bool,
    /// Whether inverse / reverse video is enabled.
    pub inverse: bool,
}

/// An in-memory virtual terminal screen backed by a VT100 / ANSI escape sequence parser.
pub struct VtScreen {
    parser: Parser,
}

impl VtScreen {
    /// Default scrollback buffer length (in lines).
    pub const DEFAULT_SCROLLBACK: usize = 1000;

    /// Creates a new virtual screen with the specified column and row dimensions.
    pub fn new(cols: u16, rows: u16) -> Self {
        Self {
            parser: Parser::new(rows, cols, Self::DEFAULT_SCROLLBACK),
        }
    }

    /// Creates a new virtual screen with custom scrollback history length.
    pub fn with_scrollback(cols: u16, rows: u16, scrollback: usize) -> Self {
        Self {
            parser: Parser::new(rows, cols, scrollback),
        }
    }

    /// Feeds raw terminal output bytes (including ANSI escape codes) into the virtual terminal parser.
    pub fn process(&mut self, bytes: &[u8]) {
        self.parser.process(bytes);
    }

    /// Resizes the virtual terminal grid to the new column and row dimensions.
    pub fn resize(&mut self, cols: u16, rows: u16) {
        self.parser.set_size(rows, cols);
    }

    /// Returns the plain text contents of the screen grid, with trailing whitespace trimmed per row.
    pub fn contents(&self) -> String {
        self.parser.screen().contents()
    }

    /// Returns formatted contents including ANSI formatting sequences.
    pub fn contents_formatted(&self) -> String {
        String::from_utf8_lossy(&self.parser.screen().contents_formatted()).to_string()
    }

    /// Returns the current cursor position as `(col, row)` (0-indexed).
    pub fn cursor_position(&self) -> (u16, u16) {
        let (row, col) = self.parser.screen().cursor_position();
        (col, row)
    }

    /// Returns whether the cursor is hidden.
    pub fn hide_cursor(&self) -> bool {
        self.parser.screen().hide_cursor()
    }

    /// Returns the screen dimensions as `(cols, rows)`.
    pub fn size(&self) -> (u16, u16) {
        let (rows, cols) = self.parser.screen().size();
        (cols, rows)
    }

    /// Returns the number of columns in the screen grid.
    pub fn cols(&self) -> u16 {
        self.parser.screen().size().1
    }

    /// Returns the number of rows in the screen grid.
    pub fn rows(&self) -> u16 {
        self.parser.screen().size().0
    }

    /// Inspects a specific cell at `(col, row)`.
    pub fn cell(&self, col: u16, row: u16) -> Option<VtCell> {
        let cell = self.parser.screen().cell(row, col)?;
        let fg = match cell.fgcolor() {
            vt100::Color::Default => None,
            other => Some(other),
        };
        let bg = match cell.bgcolor() {
            vt100::Color::Default => None,
            other => Some(other),
        };

        Some(VtCell {
            text: cell.contents().to_string(),
            fg,
            bg,
            bold: cell.bold(),
            italic: cell.italic(),
            underline: cell.underline(),
            inverse: cell.inverse(),
        })
    }

    /// Returns the current mouse protocol mode configured by the terminal client.
    pub fn mouse_protocol_mode(&self) -> vt100::MouseProtocolMode {
        self.parser.screen().mouse_protocol_mode()
    }

    /// Returns the current mouse protocol encoding configured by the terminal client.
    pub fn mouse_protocol_encoding(&self) -> vt100::MouseProtocolEncoding {
        self.parser.screen().mouse_protocol_encoding()
    }

    /// Returns whether the child application has enabled mouse reporting.
    pub fn is_mouse_mode_enabled(&self) -> bool {
        self.parser.screen().mouse_protocol_mode() != vt100::MouseProtocolMode::None
    }

    /// Returns the current scrollback line offset.
    pub fn scrollback(&self) -> usize {
        self.parser.screen().scrollback()
    }

    /// Sets the scrollback line offset.
    pub fn set_scrollback(&mut self, lines: usize) {
        self.parser.set_scrollback(lines);
    }

    /// Scrolls the virtual screen scrollback view up (older lines).
    pub fn scroll_up(&mut self, lines: usize) {
        let current = self.parser.screen().scrollback();
        self.parser.set_scrollback(current.saturating_add(lines));
    }

    /// Scrolls the virtual screen scrollback view down (newer lines towards live view).
    pub fn scroll_down(&mut self, lines: usize) {
        let current = self.parser.screen().scrollback();
        self.parser.set_scrollback(current.saturating_sub(lines));
    }

    /// Resets the scrollback view back to live output (offset 0).
    pub fn reset_scrollback(&mut self) {
        if self.parser.screen().scrollback() != 0 {
            self.parser.set_scrollback(0);
        }
    }

    /// Returns a reference to the underlying `vt100::Parser`.
    pub fn parser(&self) -> &Parser {
        &self.parser
    }

    /// Returns a mutable reference to the underlying `vt100::Parser`.
    pub fn parser_mut(&mut self) -> &mut Parser {
        &mut self.parser
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vt_screen_creation_and_dimensions() {
        let screen = VtScreen::new(80, 24);
        assert_eq!(screen.size(), (80, 24));
        assert_eq!(screen.cols(), 80);
        assert_eq!(screen.rows(), 24);
        assert_eq!(screen.cursor_position(), (0, 0));
    }

    #[test]
    fn test_vt_screen_process_and_contents() {
        let mut screen = VtScreen::new(80, 24);
        screen.process(b"Hello, botai virtual terminal!\r\nLine 2");
        let text = screen.contents();
        assert!(text.contains("Hello, botai virtual terminal!"));
        assert!(text.contains("Line 2"));
    }

    #[test]
    fn test_vt_screen_formatting_and_colors() {
        let mut screen = VtScreen::new(80, 24);
        // \x1b[1;31m -> Bold Red, \x1b[0m -> Reset
        screen.process(b"\x1b[1;31mBoldRed\x1b[0m");

        let formatted = screen.contents_formatted();
        assert!(!formatted.is_empty());

        let cell = screen.cell(0, 0).expect("Expected cell at 0,0");
        assert_eq!(cell.text, "B");
        assert!(cell.bold);
        assert_eq!(cell.fg, Some(vt100::Color::Idx(1)));
    }

    #[test]
    fn test_vt_screen_resize() {
        let mut screen = VtScreen::new(80, 24);
        assert_eq!(screen.size(), (80, 24));

        screen.resize(120, 40);
        assert_eq!(screen.size(), (120, 40));
        assert_eq!(screen.cols(), 120);
        assert_eq!(screen.rows(), 40);
    }

    #[test]
    fn test_vt_cell_struct_construction() {
        let cell = VtCell {
            text: "A".to_string(),
            fg: Some(vt100::Color::Idx(1)),
            bg: None,
            bold: true,
            italic: false,
            underline: false,
            inverse: false,
        };
        assert_eq!(cell.text, "A");
        assert!(cell.bold);
        assert!(!cell.italic);
        assert_eq!(cell.fg, Some(vt100::Color::Idx(1)));
    }

    #[test]
    fn test_vt_screen_cursor_position_and_hide_cursor() {
        let mut screen = VtScreen::new(80, 24);
        assert_eq!(screen.cursor_position(), (0, 0));
        assert!(
            !screen.hide_cursor(),
            "Default cursor should be visible (hide_cursor = false)"
        );

        // Hide cursor ANSI escape: \x1b[?25l
        screen.process(b"\x1b[?25l");
        assert!(
            screen.hide_cursor(),
            "Cursor should now be hidden (hide_cursor = true)"
        );

        // Show cursor ANSI escape: \x1b[?25h
        screen.process(b"\x1b[?25h");
        assert!(
            !screen.hide_cursor(),
            "Cursor should now be visible (hide_cursor = false)"
        );

        // Move cursor: \x1b[10;15H (1-indexed row 10, col 15 -> 0-indexed col 14, row 9)
        screen.process(b"\x1b[10;15H");
        assert_eq!(screen.cursor_position(), (14, 9));
    }

    #[test]
    fn test_vt_screen_scrollback_and_mouse_mode() {
        let mut screen = VtScreen::new(80, 5);
        assert_eq!(screen.scrollback(), 0);
        assert!(!screen.is_mouse_mode_enabled());

        // Fill with lines to create scrollback
        for i in 1..=20 {
            screen.process(format!("Line {}\r\n", i).as_bytes());
        }

        screen.scroll_up(5);
        assert_eq!(screen.scrollback(), 5);

        screen.scroll_up(10);
        assert_eq!(screen.scrollback(), 15);

        screen.scroll_down(5);
        assert_eq!(screen.scrollback(), 10);

        screen.reset_scrollback();
        assert_eq!(screen.scrollback(), 0);

        // Enable mouse tracking: \x1b[?1000h (SET_VT200_MOUSE)
        screen.process(b"\x1b[?1000h");
        assert!(screen.is_mouse_mode_enabled());

        // Disable mouse tracking: \x1b[?1000l
        screen.process(b"\x1b[?1000l");
        assert!(!screen.is_mouse_mode_enabled());
    }
}
