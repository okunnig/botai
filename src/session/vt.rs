use vt100::Parser;

/// Longest Device Status Report query this module recognises (`ESC [ ? 6 n`).
const MAX_DSR_QUERY_LEN: usize = 5;

/// Builds replies to Device Status Report queries a child process writes to its terminal.
///
/// This is not optional on Windows. ConPTY emits `ESC[6n` as soon as a session starts and
/// **blocks until it is answered** — measured here, a child produced its first byte of real
/// output 101 ms after the reply and nothing at all (4 bytes, just the query) after 8 s
/// without one. Every PTY session hung at startup until botai learned to answer.
///
/// A query can be split across two reads, so the trailing bytes of each chunk are carried
/// over into the next scan.
#[derive(Debug, Default)]
pub struct DsrResponder {
    /// Tail of the previous chunk that could be the start of a split query.
    pending: Vec<u8>,
}

impl DsrResponder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Scans one chunk of child output and returns the bytes to write back to the PTY.
    ///
    /// `cursor` is `(col, row)`, 0-indexed, as returned by [`VtScreen::cursor_position`];
    /// terminal reports are 1-based, so both are incremented.
    pub fn responses(&mut self, bytes: &[u8], cursor: (u16, u16)) -> Vec<u8> {
        let mut scan = std::mem::take(&mut self.pending);
        scan.extend_from_slice(bytes);

        let (col, row) = cursor;
        let (row1, col1) = (row.saturating_add(1), col.saturating_add(1));

        let mut out = Vec::new();
        let mut i = 0;
        while i < scan.len() {
            if scan[i] != 0x1b {
                i += 1;
                continue;
            }
            let rest = &scan[i..];
            if rest.starts_with(b"\x1b[6n") {
                // CPR: cursor position report.
                out.extend_from_slice(format!("\x1b[{row1};{col1}R").as_bytes());
                i += 4;
            } else if rest.starts_with(b"\x1b[?6n") {
                // DECXCPR: extended cursor position report, with page number.
                out.extend_from_slice(format!("\x1b[?{row1};{col1};1R").as_bytes());
                i += 5;
            } else if rest.starts_with(b"\x1b[5n") {
                // DSR: report terminal status. 0 means "ready, no malfunction".
                out.extend_from_slice(b"\x1b[0n");
                i += 4;
            } else if rest.len() < MAX_DSR_QUERY_LEN && Self::is_query_prefix(rest) {
                // Truncated query at the end of the chunk: hold it for the next read
                // rather than scanning past it. Only the unconsumed tail is carried,
                // so a query already answered above is never replayed.
                self.pending = rest.to_vec();
                return out;
            } else {
                i += 1;
            }
        }

        out
    }

    /// Whether `bytes` could still grow into one of the queries above.
    fn is_query_prefix(bytes: &[u8]) -> bool {
        const QUERIES: [&[u8]; 3] = [b"\x1b[6n", b"\x1b[?6n", b"\x1b[5n"];
        QUERIES.iter().any(|q| q.starts_with(bytes))
    }
}

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
        self.parser.screen_mut().set_size(rows, cols);
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

    /// Returns a reference to the underlying `vt100::Screen` for zero-allocation rendering and inspection.
    pub fn screen(&self) -> &vt100::Screen {
        self.parser.screen()
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
        self.parser.screen_mut().set_scrollback(lines);
    }

    /// Scrolls the virtual screen scrollback view up (older lines).
    pub fn scroll_up(&mut self, lines: usize) {
        let current = self.parser.screen().scrollback();
        self.parser
            .screen_mut()
            .set_scrollback(current.saturating_add(lines));
    }

    /// Scrolls the virtual screen scrollback view down (newer lines towards live view).
    pub fn scroll_down(&mut self, lines: usize) {
        let current = self.parser.screen().scrollback();
        self.parser
            .screen_mut()
            .set_scrollback(current.saturating_sub(lines));
    }

    /// Resets the scrollback view back to live output (offset 0).
    pub fn reset_scrollback(&mut self) {
        if self.parser.screen().scrollback() != 0 {
            self.parser.screen_mut().set_scrollback(0);
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
    fn test_vt_screen_reference() {
        let mut screen = VtScreen::new(80, 24);
        screen.process(b"ZeroAlloc");
        let vt100_screen = screen.screen();
        assert_eq!(vt100_screen.size(), (24, 80));
        let cell = vt100_screen.cell(0, 0).expect("Expected cell at 0,0");
        assert_eq!(cell.contents(), "Z");
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

    #[test]
    fn test_dsr_responder_answers_cursor_position_request() {
        let mut dsr = DsrResponder::new();
        // cursor_position() is (col, row) 0-indexed; the report is 1-based row;col.
        assert_eq!(dsr.responses(b"\x1b[6n", (9, 4)), b"\x1b[5;10R".to_vec());

        // The query ConPTY actually emits at startup, at the home position.
        let mut dsr = DsrResponder::new();
        assert_eq!(dsr.responses(b"\x1b[6n", (0, 0)), b"\x1b[1;1R".to_vec());
    }

    #[test]
    fn test_dsr_responder_handles_other_queries_and_ignores_plain_output() {
        let mut dsr = DsrResponder::new();
        assert_eq!(dsr.responses(b"\x1b[5n", (0, 0)), b"\x1b[0n".to_vec());
        assert_eq!(
            dsr.responses(b"\x1b[?6n", (2, 1)),
            b"\x1b[?2;3;1R".to_vec()
        );

        // Ordinary output, and escape sequences that are not status requests, must not
        // produce a reply - writing stray bytes back would corrupt the child's input.
        assert!(dsr.responses(b"hello world\r\n", (0, 0)).is_empty());
        assert!(dsr.responses(b"\x1b[1;31mred\x1b[0m", (0, 0)).is_empty());
        assert!(dsr.responses(b"\x1b[?1000h", (0, 0)).is_empty());
    }

    #[test]
    fn test_dsr_responder_reassembles_query_split_across_reads() {
        let mut dsr = DsrResponder::new();
        // A read can end mid-sequence; the tail has to carry over.
        assert!(dsr.responses(b"some output\x1b[", (0, 0)).is_empty());
        assert_eq!(dsr.responses(b"6n", (0, 0)), b"\x1b[1;1R".to_vec());
    }

    #[test]
    fn test_dsr_responder_does_not_replay_an_answered_query() {
        let mut dsr = DsrResponder::new();
        // A chunk ending exactly on a complete query must consume it, not carry it over -
        // otherwise the next read would answer the same query a second time.
        assert_eq!(dsr.responses(b"out\x1b[6n", (0, 0)), b"\x1b[1;1R".to_vec());
        assert!(dsr.responses(b"more output", (0, 0)).is_empty());
    }
}
