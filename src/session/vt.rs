use std::sync::atomic::{AtomicBool, Ordering};
use vt100::Parser;

/// Device Status Report queries and their replies.
///
/// The cursor reports claim the home position: ConPTY only needs *an* answer to unblock, and
/// the child's own cursor is not tracked here.
const DSR_QUERIES: [(&[u8], &[u8]); 3] = [
    (b"\x1b[6n", b"\x1b[1;1R"),     // CPR
    (b"\x1b[?6n", b"\x1b[?1;1;1R"), // DECXCPR
    (b"\x1b[5n", b"\x1b[0n"),       // status: no malfunction
];

/// Answers Device Status Report queries from a child process.
///
/// ConPTY writes `ESC[6n` on startup and blocks until it is answered, so an unanswered
/// session never produces output at all.
#[derive(Debug, Default)]
pub struct DsrResponder {
    /// Trailing bytes of the last chunk that may be a query split across reads.
    pending: Vec<u8>,
}

impl DsrResponder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the bytes to write back for any query in `bytes`.
    pub fn responses(&mut self, bytes: &[u8]) -> Vec<u8> {
        // Avoid copying the chunk in the overwhelmingly common case of no carried-over tail.
        if self.pending.is_empty() {
            self.scan(bytes)
        } else {
            let mut scan = std::mem::take(&mut self.pending);
            scan.extend_from_slice(bytes);
            self.scan(&scan)
        }
    }

    fn scan(&mut self, scan: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut i = 0;
        while i < scan.len() {
            if scan[i] != 0x1b {
                i += 1;
                continue;
            }
            let rest = &scan[i..];
            match DSR_QUERIES.iter().find(|(q, _)| rest.starts_with(q)) {
                Some((query, reply)) => {
                    out.extend_from_slice(reply);
                    i += query.len();
                }
                // A truncated query at the very end is held for the next read. Only the
                // unconsumed tail is carried, so an answered query is never replayed.
                None if DSR_QUERIES.iter().any(|(q, _)| q.starts_with(rest)) => {
                    self.pending = rest.to_vec();
                    return out;
                }
                None => i += 1,
            }
        }
        out
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
    /// Whether dim / faint styling is enabled.
    pub dim: bool,
    /// Whether italic styling is enabled.
    pub italic: bool,
    /// Whether underline styling is enabled.
    pub underline: bool,
    /// Whether inverse / reverse video is enabled.
    pub inverse: bool,
}

static KMP_TERMINAL_OVERRIDE: AtomicBool = AtomicBool::new(false);
static KMP_TERMINAL_OVERRIDE_SET: AtomicBool = AtomicBool::new(false);

/// Explicitly configures whether the experimental `kmp-terminal` engine is enabled at runtime.
pub fn set_kmp_terminal_enabled(enabled: bool) {
    KMP_TERMINAL_OVERRIDE.store(enabled, Ordering::SeqCst);
    KMP_TERMINAL_OVERRIDE_SET.store(true, Ordering::SeqCst);
}

/// Clears any explicit runtime override for `kmp-terminal`, restoring environment-variable or default behavior.
pub fn clear_kmp_terminal_override() {
    KMP_TERMINAL_OVERRIDE_SET.store(false, Ordering::SeqCst);
}

/// Returns whether the experimental `kmp-terminal` engine is currently active.
///
/// Under `kmp-terminal` feature:
/// - Honors explicit runtime override if set via `set_kmp_terminal_enabled(...)` or `--kmp-terminal`.
/// - Honors `BOTAI_KMP_TERMINAL=0` / `false` to disable.
/// - Defaults to `true` when compiled with `--features kmp-terminal`.
///
/// When compiled without `kmp-terminal`: always returns `false`.
pub fn is_kmp_terminal_enabled() -> bool {
    #[cfg(feature = "kmp-terminal")]
    {
        if KMP_TERMINAL_OVERRIDE_SET.load(Ordering::SeqCst) {
            return KMP_TERMINAL_OVERRIDE.load(Ordering::SeqCst);
        }
        if let Ok(val) = std::env::var("BOTAI_KMP_TERMINAL") {
            let v = val.trim();
            if v == "0" || v.eq_ignore_ascii_case("false") {
                return false;
            }
            if v == "1" || v.eq_ignore_ascii_case("true") {
                return true;
            }
        }
        true
    }
    #[cfg(not(feature = "kmp-terminal"))]
    {
        false
    }
}

#[cfg(feature = "kmp-terminal")]
struct KmpBackend {
    term: kmp_terminal_rs::Terminal,
    scrollback_offset: usize,
}

/// An in-memory virtual terminal screen backed by a VT100 / ANSI escape sequence parser
/// or the experimental Kotlin Multiplatform `kmp-terminal` engine.
pub struct VtScreen {
    parser: Parser,
    #[cfg(feature = "kmp-terminal")]
    kmp: Option<KmpBackend>,
}

impl VtScreen {
    /// Default scrollback buffer length (in lines).
    pub const DEFAULT_SCROLLBACK: usize = 1000;

    /// Creates a new virtual screen with the specified column and row dimensions.
    pub fn new(cols: u16, rows: u16) -> Self {
        #[cfg(feature = "kmp-terminal")]
        let kmp = if is_kmp_terminal_enabled() {
            Some(KmpBackend {
                term: kmp_terminal_rs::Terminal::with_config(
                    i32::from(rows.max(1)),
                    i32::from(cols.max(1)),
                    Self::DEFAULT_SCROLLBACK as i32,
                    kmp_terminal_rs::Osc52Policy::AlwaysAllow,
                ),
                scrollback_offset: 0,
            })
        } else {
            None
        };

        Self {
            parser: Parser::new(rows, cols, Self::DEFAULT_SCROLLBACK),
            #[cfg(feature = "kmp-terminal")]
            kmp,
        }
    }

    /// Creates a new virtual screen with custom scrollback history length.
    pub fn with_scrollback(cols: u16, rows: u16, scrollback: usize) -> Self {
        #[cfg(feature = "kmp-terminal")]
        let kmp = if is_kmp_terminal_enabled() {
            Some(KmpBackend {
                term: kmp_terminal_rs::Terminal::with_config(
                    i32::from(rows.max(1)),
                    i32::from(cols.max(1)),
                    scrollback as i32,
                    kmp_terminal_rs::Osc52Policy::AlwaysAllow,
                ),
                scrollback_offset: 0,
            })
        } else {
            None
        };

        Self {
            parser: Parser::new(rows, cols, scrollback),
            #[cfg(feature = "kmp-terminal")]
            kmp,
        }
    }

    /// Creates a virtual screen explicitly using the `kmp-terminal` engine.
    #[cfg(feature = "kmp-terminal")]
    pub fn new_kmp(cols: u16, rows: u16) -> Self {
        Self {
            parser: Parser::new(rows, cols, Self::DEFAULT_SCROLLBACK),
            kmp: Some(KmpBackend {
                term: kmp_terminal_rs::Terminal::with_config(
                    i32::from(rows.max(1)),
                    i32::from(cols.max(1)),
                    Self::DEFAULT_SCROLLBACK as i32,
                    kmp_terminal_rs::Osc52Policy::AlwaysAllow,
                ),
                scrollback_offset: 0,
            }),
        }
    }

    /// Creates a virtual screen explicitly using the legacy `vt100` parser backend.
    pub fn new_vt100(cols: u16, rows: u16) -> Self {
        Self {
            parser: Parser::new(rows, cols, Self::DEFAULT_SCROLLBACK),
            #[cfg(feature = "kmp-terminal")]
            kmp: None,
        }
    }

    /// Returns whether this virtual screen instance is backed by `kmp-terminal`.
    pub fn is_using_kmp(&self) -> bool {
        #[cfg(feature = "kmp-terminal")]
        {
            self.kmp.is_some()
        }
        #[cfg(not(feature = "kmp-terminal"))]
        {
            false
        }
    }

    /// Feeds raw terminal output bytes (including ANSI escape codes) into the virtual terminal parser.
    pub fn process(&mut self, bytes: &[u8]) {
        #[cfg(feature = "kmp-terminal")]
        if let Some(kmp) = &mut self.kmp {
            kmp.term.process_bytes(bytes);
            return;
        }
        self.parser.process(bytes);
    }

    /// Resizes the virtual terminal grid to the new column and row dimensions.
    pub fn resize(&mut self, cols: u16, rows: u16) {
        #[cfg(feature = "kmp-terminal")]
        if let Some(kmp) = &mut self.kmp {
            kmp.term
                .resize(i32::from(rows.max(1)), i32::from(cols.max(1)));
            let max_sb = kmp.term.scrollback_size().max(0) as usize;
            kmp.scrollback_offset = kmp.scrollback_offset.min(max_sb);
            self.parser.screen_mut().set_size(rows, cols);
            return;
        }
        self.parser.screen_mut().set_size(rows, cols);
    }

    /// Returns the plain text contents of the screen grid, with trailing whitespace trimmed per row.
    pub fn contents(&self) -> String {
        #[cfg(feature = "kmp-terminal")]
        if let Some(kmp) = &self.kmp {
            let cols = kmp.term.cols().max(0) as usize;
            let rows = kmp.term.rows().max(0) as usize;
            if cols == 0 || rows == 0 {
                return String::new();
            }
            let cells = kmp
                .term
                .read_viewport(kmp.scrollback_offset, kmp_terminal_rs::Palette::AnsiRaw);
            let (cur_row, cur_col) = kmp.term.cursor_pos();
            let mut lines: Vec<String> = Vec::with_capacity(rows);
            for r in 0..rows {
                let mut line = String::with_capacity(cols);
                for c in 0..cols {
                    let idx = r * cols + c;
                    if idx < cells.len() {
                        let cell = &cells[idx];
                        if !cell.attrs().wide_spacer() {
                            line.push(cell.ch());
                        }
                    }
                }
                let min_len = if kmp.scrollback_offset == 0 && r == cur_row as usize {
                    (cur_col.max(0) as usize).min(line.len())
                } else {
                    0
                };
                while line.len() > min_len && line.ends_with(' ') {
                    line.pop();
                }
                lines.push(line);
            }
            while let Some(last) = lines.last() {
                if last.is_empty() {
                    lines.pop();
                } else {
                    break;
                }
            }
            return lines.join("\n");
        }
        self.parser.screen().contents()
    }

    /// Returns formatted contents including ANSI formatting sequences.
    pub fn contents_formatted(&self) -> String {
        #[cfg(feature = "kmp-terminal")]
        if self.kmp.is_some() {
            return self.contents();
        }
        String::from_utf8_lossy(&self.parser.screen().contents_formatted()).to_string()
    }

    /// Returns the current cursor position as `(col, row)` (0-indexed).
    pub fn cursor_position(&self) -> (u16, u16) {
        #[cfg(feature = "kmp-terminal")]
        if let Some(kmp) = &self.kmp {
            let (r, c) = kmp.term.cursor_pos();
            return (c.max(0) as u16, r.max(0) as u16);
        }
        let (row, col) = self.parser.screen().cursor_position();
        (col, row)
    }

    /// Returns whether the cursor is hidden.
    pub fn hide_cursor(&self) -> bool {
        #[cfg(feature = "kmp-terminal")]
        if let Some(kmp) = &self.kmp {
            return !kmp.term.cursor_visible();
        }
        self.parser.screen().hide_cursor()
    }

    /// Returns the screen dimensions as `(cols, rows)`.
    pub fn size(&self) -> (u16, u16) {
        #[cfg(feature = "kmp-terminal")]
        if let Some(kmp) = &self.kmp {
            return (kmp.term.cols().max(0) as u16, kmp.term.rows().max(0) as u16);
        }
        let (rows, cols) = self.parser.screen().size();
        (cols, rows)
    }

    /// Returns the number of columns in the screen grid.
    pub fn cols(&self) -> u16 {
        #[cfg(feature = "kmp-terminal")]
        if let Some(kmp) = &self.kmp {
            return kmp.term.cols().max(0) as u16;
        }
        self.parser.screen().size().1
    }

    /// Returns the number of rows in the screen grid.
    pub fn rows(&self) -> u16 {
        #[cfg(feature = "kmp-terminal")]
        if let Some(kmp) = &self.kmp {
            return kmp.term.rows().max(0) as u16;
        }
        self.parser.screen().size().0
    }

    /// Returns a reference to the underlying `vt100::Screen` for zero-allocation rendering and inspection.
    pub fn screen(&self) -> &vt100::Screen {
        self.parser.screen()
    }

    /// Inspects a specific cell at `(col, row)`.
    pub fn cell(&self, col: u16, row: u16) -> Option<VtCell> {
        #[cfg(feature = "kmp-terminal")]
        if let Some(kmp) = &self.kmp {
            let cols = kmp.term.cols().max(0) as u16;
            let rows = kmp.term.rows().max(0) as u16;
            if col >= cols || row >= rows {
                return None;
            }
            let cells = kmp
                .term
                .read_viewport(kmp.scrollback_offset, kmp_terminal_rs::Palette::AnsiRaw);
            let idx = (row as usize) * (cols as usize) + (col as usize);
            let cell = cells.get(idx)?;
            let fg = match cell.fg_raw() {
                kmp_terminal_rs::RawColor::Default => None,
                kmp_terminal_rs::RawColor::Indexed(idx) => Some(vt100::Color::Idx(idx)),
                kmp_terminal_rs::RawColor::Rgb(r, g, b) => Some(vt100::Color::Rgb(r, g, b)),
            };
            let bg = match cell.bg_raw() {
                kmp_terminal_rs::RawColor::Default => None,
                kmp_terminal_rs::RawColor::Indexed(idx) => Some(vt100::Color::Idx(idx)),
                kmp_terminal_rs::RawColor::Rgb(r, g, b) => Some(vt100::Color::Rgb(r, g, b)),
            };
            let attrs = cell.attrs();
            return Some(VtCell {
                text: cell.glyph(),
                fg,
                bg,
                bold: attrs.bold(),
                dim: attrs.dim(),
                italic: attrs.italic(),
                underline: attrs.underline(),
                inverse: attrs.reverse(),
            });
        }
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
            dim: cell.dim(),
            italic: cell.italic(),
            underline: cell.underline(),
            inverse: cell.inverse(),
        })
    }

    /// Returns the current mouse protocol mode configured by the terminal client.
    pub fn mouse_protocol_mode(&self) -> vt100::MouseProtocolMode {
        #[cfg(feature = "kmp-terminal")]
        if let Some(kmp) = &self.kmp {
            return match kmp.term.mouse_tracking_mode() {
                kmp_terminal_rs::MouseTrackingMode::None => vt100::MouseProtocolMode::None,
                kmp_terminal_rs::MouseTrackingMode::Click => vt100::MouseProtocolMode::PressRelease,
                kmp_terminal_rs::MouseTrackingMode::CellMotion => {
                    vt100::MouseProtocolMode::ButtonMotion
                }
                kmp_terminal_rs::MouseTrackingMode::AllMotion => {
                    vt100::MouseProtocolMode::AnyMotion
                }
            };
        }
        self.parser.screen().mouse_protocol_mode()
    }

    /// Returns the current mouse protocol encoding configured by the terminal client.
    pub fn mouse_protocol_encoding(&self) -> vt100::MouseProtocolEncoding {
        #[cfg(feature = "kmp-terminal")]
        if let Some(kmp) = &self.kmp {
            return if kmp.term.sgr_mouse_mode() {
                vt100::MouseProtocolEncoding::Sgr
            } else {
                vt100::MouseProtocolEncoding::Default
            };
        }
        self.parser.screen().mouse_protocol_encoding()
    }

    /// Returns whether the child application has enabled mouse reporting.
    pub fn is_mouse_mode_enabled(&self) -> bool {
        self.mouse_protocol_mode() != vt100::MouseProtocolMode::None
    }

    /// Returns the current scrollback line offset.
    pub fn scrollback(&self) -> usize {
        #[cfg(feature = "kmp-terminal")]
        if let Some(kmp) = &self.kmp {
            return kmp.scrollback_offset;
        }
        self.parser.screen().scrollback()
    }

    /// Sets the scrollback line offset.
    pub fn set_scrollback(&mut self, lines: usize) {
        #[cfg(feature = "kmp-terminal")]
        if let Some(kmp) = &mut self.kmp {
            let max_sb = kmp.term.scrollback_size().max(0) as usize;
            kmp.scrollback_offset = lines.min(max_sb);
            return;
        }
        self.parser.screen_mut().set_scrollback(lines);
    }

    /// Scrolls the virtual screen scrollback view up (older lines).
    pub fn scroll_up(&mut self, lines: usize) {
        #[cfg(feature = "kmp-terminal")]
        if let Some(kmp) = &mut self.kmp {
            let max_sb = kmp.term.scrollback_size().max(0) as usize;
            kmp.scrollback_offset = kmp.scrollback_offset.saturating_add(lines).min(max_sb);
            return;
        }
        let current = self.parser.screen().scrollback();
        self.parser
            .screen_mut()
            .set_scrollback(current.saturating_add(lines));
    }

    /// Scrolls the virtual screen scrollback view down (newer lines towards live view).
    pub fn scroll_down(&mut self, lines: usize) {
        #[cfg(feature = "kmp-terminal")]
        if let Some(kmp) = &mut self.kmp {
            kmp.scrollback_offset = kmp.scrollback_offset.saturating_sub(lines);
            return;
        }
        let current = self.parser.screen().scrollback();
        self.parser
            .screen_mut()
            .set_scrollback(current.saturating_sub(lines));
    }

    /// Resets the scrollback view back to live output (offset 0).
    pub fn reset_scrollback(&mut self) {
        #[cfg(feature = "kmp-terminal")]
        if let Some(kmp) = &mut self.kmp {
            kmp.scrollback_offset = 0;
            return;
        }
        if self.parser.screen().scrollback() != 0 {
            self.parser.screen_mut().set_scrollback(0);
        }
    }

    /// Reads KMP viewport cell buffer when backed by `kmp-terminal`.
    #[cfg(feature = "kmp-terminal")]
    pub(crate) fn read_kmp_viewport(&self) -> Option<(u16, u16, Vec<kmp_terminal_rs::PackedCell>)> {
        let kmp = self.kmp.as_ref()?;
        let cols = kmp.term.cols().max(0) as u16;
        let rows = kmp.term.rows().max(0) as u16;
        let cells = kmp
            .term
            .read_viewport(kmp.scrollback_offset, kmp_terminal_rs::Palette::AnsiRaw);
        Some((cols, rows, cells))
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
    fn test_vt_screen_dim_attribute() {
        let mut screen = VtScreen::new(80, 24);
        screen.process(b"\x1b[2mD\x1b[0mP");

        let dim_cell = screen.cell(0, 0).expect("Expected cell at 0,0");
        assert_eq!(dim_cell.text, "D");
        assert!(dim_cell.dim);
        assert!(!dim_cell.bold);

        let plain_cell = screen.cell(1, 0).expect("Expected cell at 1,0");
        assert_eq!(plain_cell.text, "P");
        assert!(!plain_cell.dim);
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
        let mut screen = VtScreen::new_vt100(80, 24);
        screen.process(b"ZeroAlloc");
        let vt100_screen = screen.screen();
        assert_eq!(vt100_screen.size(), (24, 80));
        let cell = vt100_screen.cell(0, 0).expect("Expected cell at 0,0");
        assert_eq!(cell.contents(), "Z");
    }

    #[cfg(feature = "kmp-terminal")]
    #[test]
    fn test_kmp_screen_text_and_contents() {
        let mut screen = VtScreen::new_kmp(80, 24);
        assert!(screen.is_using_kmp());
        screen.process(b"Hello KMP Terminal\r\nSecond Line");
        screen.process(b"\r\n\x1b]133;A\x07Line 2 prompt > ");
        let text = screen.contents();
        assert!(text.contains("Hello KMP Terminal"));
        assert!(text.contains("Second Line"));
        assert!(text.contains("Line 2 prompt > "));
        let formatted = screen.contents_formatted();
        assert!(!formatted.is_empty());
    }

    #[cfg(feature = "kmp-terminal")]
    #[test]
    fn test_kmp_screen_sgr_colors_and_modifiers() {
        let mut screen = VtScreen::new_kmp(80, 24);
        screen.process(
            b"\x1b[1;31mBoldRed\x1b[0m \x1b[2;3mDimItalic\x1b[0m \x1b[4;7mUnderlineRev\x1b[0m",
        );

        let cell_bold = screen.cell(0, 0).expect("cell 0,0");
        assert_eq!(cell_bold.text, "B");
        assert!(cell_bold.bold);
        assert_eq!(cell_bold.fg, Some(vt100::Color::Idx(1)));

        let cell_dim = screen.cell(8, 0).expect("cell 8,0");
        assert_eq!(cell_dim.text, "D");
        assert!(cell_dim.dim);
        assert!(cell_dim.italic);

        let cell_rev = screen.cell(18, 0).expect("cell 18,0");
        assert_eq!(cell_rev.text, "U");
        assert!(cell_rev.underline);
        assert!(cell_rev.inverse);

        // 24-bit TrueColor RGB
        screen.process(b"\r\n\x1b[38;2;12;34;56;48;2;78;90;12mColor\x1b[0m");
        let cell_rgb = screen.cell(0, 1).expect("cell 0,1");
        assert_eq!(cell_rgb.text, "C");
        assert_eq!(cell_rgb.fg, Some(vt100::Color::Rgb(12, 34, 56)));
        assert_eq!(cell_rgb.bg, Some(vt100::Color::Rgb(78, 90, 12)));
    }

    #[cfg(feature = "kmp-terminal")]
    #[test]
    fn test_kmp_screen_wide_cjk_and_emoji() {
        let mut screen = VtScreen::new_kmp(80, 24);
        // "日本語" UTF-8 bytes: \xE6\x97\xA5 \xE6\x9C\xAC \xE8\xAA\x9E
        screen.process("日本語".as_bytes());

        let cell_first = screen.cell(0, 0).expect("col 0");
        assert_eq!(cell_first.text, "日");

        let cell_spacer = screen.cell(1, 0).expect("col 1 wide spacer");
        assert_eq!(cell_spacer.text, "");

        let cell_second = screen.cell(2, 0).expect("col 2");
        assert_eq!(cell_second.text, "本");

        // Read viewport directly
        let (cols, rows, packed_cells) = screen.read_kmp_viewport().expect("read_kmp_viewport");
        assert_eq!(cols, 80);
        assert_eq!(rows, 24);
        assert!(packed_cells[1].attrs().wide_spacer());
    }

    #[cfg(feature = "kmp-terminal")]
    #[test]
    fn test_kmp_screen_cursor_and_visibility() {
        let mut screen = VtScreen::new_kmp(80, 24);
        assert_eq!(screen.cursor_position(), (0, 0));
        assert!(!screen.hide_cursor());

        screen.process(b"\x1b[?25l");
        assert!(screen.hide_cursor());

        screen.process(b"\x1b[?25h");
        assert!(!screen.hide_cursor());

        screen.process(b"\x1b[10;15H");
        assert_eq!(screen.cursor_position(), (14, 9));
    }

    #[cfg(feature = "kmp-terminal")]
    #[test]
    fn test_kmp_screen_scrollback() {
        let mut screen = VtScreen::new_kmp(80, 5);
        assert_eq!(screen.scrollback(), 0);

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
    }

    #[cfg(feature = "kmp-terminal")]
    #[test]
    fn test_kmp_screen_mouse_mode() {
        let mut screen = VtScreen::new_kmp(80, 24);
        assert!(!screen.is_mouse_mode_enabled());
        assert_eq!(screen.mouse_protocol_mode(), vt100::MouseProtocolMode::None);

        screen.process(b"\x1b[?1000h");
        assert!(screen.is_mouse_mode_enabled());
        assert_eq!(
            screen.mouse_protocol_mode(),
            vt100::MouseProtocolMode::PressRelease
        );

        screen.process(b"\x1b[?1002h");
        assert_eq!(
            screen.mouse_protocol_mode(),
            vt100::MouseProtocolMode::ButtonMotion
        );

        screen.process(b"\x1b[?1003h");
        assert_eq!(
            screen.mouse_protocol_mode(),
            vt100::MouseProtocolMode::AnyMotion
        );

        screen.process(b"\x1b[?1006h");
        assert_eq!(
            screen.mouse_protocol_encoding(),
            vt100::MouseProtocolEncoding::Sgr
        );

        screen.process(b"\x1b[?1006l");
        assert_eq!(
            screen.mouse_protocol_encoding(),
            vt100::MouseProtocolEncoding::Default
        );

        screen.process(b"\x1b[?1000l");
        assert!(!screen.is_mouse_mode_enabled());
    }

    #[test]
    fn test_kmp_terminal_toggle_functions() {
        set_kmp_terminal_enabled(true);
        #[cfg(feature = "kmp-terminal")]
        assert!(is_kmp_terminal_enabled());
        #[cfg(not(feature = "kmp-terminal"))]
        assert!(!is_kmp_terminal_enabled());

        set_kmp_terminal_enabled(false);
        assert!(!is_kmp_terminal_enabled());

        clear_kmp_terminal_override();
    }

    #[test]
    fn test_vt_cell_struct_construction() {
        let cell = VtCell {
            text: "A".to_string(),
            fg: Some(vt100::Color::Idx(1)),
            bg: None,
            bold: true,
            dim: false,
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
    fn test_dsr_responder_answers_each_known_query() {
        let mut dsr = DsrResponder::new();
        // The query ConPTY emits at startup, and the two other status requests.
        assert_eq!(dsr.responses(b"\x1b[6n"), b"\x1b[1;1R".to_vec());
        assert_eq!(dsr.responses(b"\x1b[?6n"), b"\x1b[?1;1;1R".to_vec());
        assert_eq!(dsr.responses(b"\x1b[5n"), b"\x1b[0n".to_vec());
    }

    #[test]
    fn test_dsr_responder_ignores_plain_output_and_other_escapes() {
        // Writing stray bytes back would corrupt the child's input.
        let mut dsr = DsrResponder::new();
        assert!(dsr.responses(b"hello world\r\n").is_empty());
        assert!(dsr.responses(b"\x1b[1;31mred\x1b[0m").is_empty());
        assert!(dsr.responses(b"\x1b[?1000h").is_empty());
    }

    #[test]
    fn test_dsr_responder_reassembles_query_split_across_reads() {
        let mut dsr = DsrResponder::new();
        assert!(dsr.responses(b"some output\x1b[").is_empty());
        assert_eq!(dsr.responses(b"6n"), b"\x1b[1;1R".to_vec());
    }

    #[test]
    fn test_dsr_responder_does_not_replay_an_answered_query() {
        let mut dsr = DsrResponder::new();
        // A chunk ending exactly on a complete query must consume it, not carry it over.
        assert_eq!(dsr.responses(b"out\x1b[6n"), b"\x1b[1;1R".to_vec());
        assert!(dsr.responses(b"more output").is_empty());
    }
}
