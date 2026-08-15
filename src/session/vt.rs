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
}
