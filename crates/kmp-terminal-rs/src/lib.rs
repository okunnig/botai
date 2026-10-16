//! Rust FFI Crate for KMP Terminal Emulator (`kmp-terminal-rs`)
//!
//! Provides zero-cost, memory-safe Rust bindings and an idiomatic API
//! over the Kotlin/Native C-ABI export `libterminal_core`.

use std::ffi::c_void;
use std::ptr::NonNull;

/// Returns `true` if the native Kotlin/Native `libterminal_core` library was linked at build time.
#[inline]
pub const fn is_native_linked() -> bool {
    cfg!(kmp_terminal_linked)
}

/// Raw C-ABI function declarations exported by Kotlin/Native `terminal-core`.
#[cfg(kmp_terminal_linked)]
pub mod ffi {
    use std::ffi::c_void;

    extern "C" {
        pub fn terminal_create(
            rows: i32,
            cols: i32,
            max_scrollback: i32,
            osc52_policy_ordinal: i32,
        ) -> *mut c_void;
        pub fn terminal_copy(handle: *mut c_void) -> *mut c_void;
        pub fn terminal_destroy(handle: *mut c_void);
        pub fn terminal_process_bytes(handle: *mut c_void, bytes: *const i8, len: i32);
        pub fn terminal_resize(handle: *mut c_void, rows: i32, cols: i32) -> i32;
        pub fn terminal_clear(handle: *mut c_void);
        pub fn terminal_reset(handle: *mut c_void);
        pub fn terminal_get_rows(handle: *mut c_void) -> i32;
        pub fn terminal_get_cols(handle: *mut c_void) -> i32;
        pub fn terminal_get_cursor_row(handle: *mut c_void) -> i32;
        pub fn terminal_get_cursor_col(handle: *mut c_void) -> i32;
        pub fn terminal_is_cursor_visible(handle: *mut c_void) -> i32;
        pub fn terminal_get_bell_count(handle: *mut c_void) -> i64;
        pub fn terminal_get_clipboard_write_count(handle: *mut c_void) -> i64;
        pub fn terminal_get_scrollback_size(handle: *mut c_void) -> i32;
        pub fn terminal_read_grid(
            handle: *mut c_void,
            out_buffer: *mut i32,
            max_cells: i32,
            palette_ordinal: i32,
        ) -> i32;
        pub fn terminal_extract_selection(
            handle: *mut c_void,
            start_row: i32,
            start_col: i32,
            end_row: i32,
            end_col: i32,
            out_utf8: *mut i8,
            capacity: i32,
        ) -> i32;
        pub fn terminal_get_window_title(
            handle: *mut c_void,
            out_utf8: *mut i8,
            capacity: i32,
        ) -> i32;
        pub fn terminal_get_last_clipboard_write(
            handle: *mut c_void,
            out_utf8: *mut i8,
            capacity: i32,
        ) -> i32;
        pub fn terminal_get_mouse_tracking_mode(handle: *mut c_void) -> i32;
        pub fn terminal_is_sgr_mouse_mode_enabled(handle: *mut c_void) -> i32;
        pub fn terminal_read_viewport(
            handle: *mut c_void,
            out_buffer: *mut i32,
            max_cells: i32,
            scrollback_offset: i32,
            palette_ordinal: i32,
        ) -> i32;
    }
}

/// Security policy for terminal OSC 52 clipboard write operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum Osc52Policy {
    /// Prompt user before clipboard write.
    Ask = 0,
    /// Automatically allow all OSC 52 clipboard writes.
    AlwaysAllow = 1,
    /// Silently ignore all OSC 52 clipboard writes.
    AlwaysDeny = 2,
}

/// Predefined color palettes matching Kotlin's `TerminalPalette.PRESETS`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum Palette {
    AnsiRaw = -1,
    Default = 0,
    SolarizedDark = 1,
    SolarizedLight = 2,
    Monokai = 3,
    Nord = 4,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum MouseTrackingMode {
    None = 0,
    Click = 1,
    CellMotion = 2,
    AllMotion = 3,
}

/// Semantic terminal color decoded when reading with `Palette::AnsiRaw`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawColor {
    Default,
    Indexed(u8),
    Rgb(u8, u8, u8),
}

impl RawColor {
    #[inline]
    pub fn from_packed(val: i32) -> Self {
        if val < 0 {
            Self::Default
        } else {
            let tag = (val as u32) >> 24;
            match tag {
                1 | 2 => Self::Indexed((val & 0xFF) as u8),
                _ => {
                    let r = ((val >> 16) & 0xFF) as u8;
                    let g = ((val >> 8) & 0xFF) as u8;
                    let b = (val & 0xFF) as u8;
                    Self::Rgb(r, g, b)
                }
            }
        }
    }
}

/// Bitmask representing cell text formatting styles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CellAttributes(pub u32);

impl CellAttributes {
    pub const BOLD: u32 = 1 << 0;
    pub const ITALIC: u32 = 1 << 1;
    pub const UNDERLINE: u32 = 1 << 2;
    pub const REVERSE: u32 = 1 << 3;
    pub const DIM: u32 = 1 << 4;
    pub const BLINK: u32 = 1 << 5;
    pub const STRIKETHROUGH: u32 = 1 << 6;
    pub const OVERLINE: u32 = 1 << 7;
    pub const CONCEAL: u32 = 1 << 8;
    pub const ROW_WRAPPED: u32 = 1 << 9;
    pub const WIDE_CHAR: u32 = 1 << 10;
    pub const WIDE_SPACER: u32 = 1 << 11;

    #[inline]
    pub fn bold(&self) -> bool {
        (self.0 & Self::BOLD) != 0
    }

    #[inline]
    pub fn italic(&self) -> bool {
        (self.0 & Self::ITALIC) != 0
    }

    #[inline]
    pub fn underline(&self) -> bool {
        (self.0 & Self::UNDERLINE) != 0
    }

    #[inline]
    pub fn reverse(&self) -> bool {
        (self.0 & Self::REVERSE) != 0
    }

    #[inline]
    pub fn dim(&self) -> bool {
        (self.0 & Self::DIM) != 0
    }

    #[inline]
    pub fn blink(&self) -> bool {
        (self.0 & Self::BLINK) != 0
    }

    #[inline]
    pub fn strikethrough(&self) -> bool {
        (self.0 & Self::STRIKETHROUGH) != 0
    }

    #[inline]
    pub fn overline(&self) -> bool {
        (self.0 & Self::OVERLINE) != 0
    }

    #[inline]
    pub fn conceal(&self) -> bool {
        (self.0 & Self::CONCEAL) != 0
    }

    #[inline]
    pub fn row_wrapped(&self) -> bool {
        (self.0 & Self::ROW_WRAPPED) != 0
    }

    #[inline]
    pub fn wide_char(&self) -> bool {
        (self.0 & Self::WIDE_CHAR) != 0
    }

    #[inline]
    pub fn wide_spacer(&self) -> bool {
        (self.0 & Self::WIDE_SPACER) != 0
    }
}

/// Packed cell format directly compatible with `terminal_read_grid` (4 x `i32` = 16 bytes).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PackedCell {
    /// Unicode codepoint (e.g. 32 for space, 65 for 'A').
    pub codepoint: i32,
    /// Effective foreground 24-bit RGB (0xRRGGBB).
    pub fg_rgb: i32,
    /// Effective background 24-bit RGB (0xRRGGBB, or -1 for transparent default).
    pub bg_rgb: i32,
    /// Attribute bitmask flags.
    pub attributes: i32,
}

const _: () =
    assert!(std::mem::size_of::<PackedCell>() == 16 && std::mem::align_of::<PackedCell>() == 4);

impl PackedCell {
    /// Returns the character glyph, or space if null/non-unicode.
    #[inline]
    pub fn ch(&self) -> char {
        if self.codepoint > 0 {
            std::char::from_u32(self.codepoint as u32).unwrap_or(' ')
        } else {
            ' '
        }
    }

    /// Decodes the attribute bitmask into structured flags.
    #[inline]
    pub fn attrs(&self) -> CellAttributes {
        CellAttributes(self.attributes as u32)
    }

    /// Returns decoded foreground semantic color.
    #[inline]
    pub fn fg_raw(&self) -> RawColor {
        RawColor::from_packed(self.fg_rgb)
    }

    /// Returns decoded background semantic color.
    #[inline]
    pub fn bg_raw(&self) -> RawColor {
        RawColor::from_packed(self.bg_rgb)
    }

    /// Returns the character glyph, or empty string if wide spacer cell.
    #[inline]
    pub fn glyph(&self) -> String {
        if self.attrs().wide_spacer() {
            String::new()
        } else {
            self.ch().to_string()
        }
    }
}

/// Safe wrapper around a Kotlin/Native `TerminalEmulator` handle.
pub struct Terminal {
    #[allow(dead_code)]
    handle: NonNull<c_void>,
}

unsafe impl Send for Terminal {}

#[cfg(kmp_terminal_linked)]
impl Terminal {
    /// Creates a new terminal emulator with standard scrollback (1000) and `Osc52Policy::AlwaysAllow`.
    pub fn new(rows: i32, cols: i32) -> Self {
        Self::with_config(rows, cols, 1000, Osc52Policy::AlwaysAllow)
    }

    /// Creates a new terminal emulator with custom dimensions, scrollback, and OSC 52 policy.
    pub fn with_config(
        rows: i32,
        cols: i32,
        max_scrollback: i32,
        osc52_policy: Osc52Policy,
    ) -> Self {
        let raw = unsafe { ffi::terminal_create(rows, cols, max_scrollback, osc52_policy as i32) };
        let handle = NonNull::new(raw).expect("terminal_create returned null handle");
        Terminal { handle }
    }

    /// Processes raw terminal input bytes through the ANSI state machine.
    pub fn process_bytes(&mut self, data: &[u8]) {
        if data.is_empty() {
            return;
        }
        unsafe {
            ffi::terminal_process_bytes(
                self.handle.as_ptr(),
                data.as_ptr() as *const i8,
                data.len().min(i32::MAX as usize) as i32,
            );
        }
    }

    /// Convenience helper to feed a UTF-8 string into the terminal emulator.
    pub fn process_str(&mut self, text: &str) {
        self.process_bytes(text.as_bytes());
    }

    /// Resizes the terminal grid to new rows and cols.
    pub fn resize(&mut self, rows: i32, cols: i32) -> bool {
        let res = unsafe { ffi::terminal_resize(self.handle.as_ptr(), rows, cols) };
        res != 0
    }

    /// Clears the terminal screen buffer.
    pub fn clear(&mut self) {
        unsafe { ffi::terminal_clear(self.handle.as_ptr()) };
    }

    /// Resets terminal cursor, attributes, and screen buffer to defaults.
    pub fn reset(&mut self) {
        unsafe { ffi::terminal_reset(self.handle.as_ptr()) };
    }

    /// Returns the current number of rows.
    pub fn rows(&self) -> i32 {
        unsafe { ffi::terminal_get_rows(self.handle.as_ptr()) }
    }

    /// Returns the current number of columns.
    pub fn cols(&self) -> i32 {
        unsafe { ffi::terminal_get_cols(self.handle.as_ptr()) }
    }

    /// Returns the cursor position as `(row, col)`.
    pub fn cursor_pos(&self) -> (i32, i32) {
        unsafe {
            let r = ffi::terminal_get_cursor_row(self.handle.as_ptr());
            let c = ffi::terminal_get_cursor_col(self.handle.as_ptr());
            (r, c)
        }
    }

    /// Returns whether the terminal cursor is currently visible.
    pub fn cursor_visible(&self) -> bool {
        unsafe { ffi::terminal_is_cursor_visible(self.handle.as_ptr()) != 0 }
    }

    /// Returns the active mouse tracking mode.
    pub fn mouse_tracking_mode(&self) -> MouseTrackingMode {
        let mode = unsafe { ffi::terminal_get_mouse_tracking_mode(self.handle.as_ptr()) };
        match mode {
            1 => MouseTrackingMode::Click,
            2 => MouseTrackingMode::CellMotion,
            3 => MouseTrackingMode::AllMotion,
            _ => MouseTrackingMode::None,
        }
    }

    /// Returns whether SGR 1006 extended mouse coordinate encoding is enabled.
    pub fn sgr_mouse_mode(&self) -> bool {
        unsafe { ffi::terminal_is_sgr_mouse_mode_enabled(self.handle.as_ptr()) != 0 }
    }

    /// Returns the total number of BEL (ASCII 7) events received.
    pub fn bell_count(&self) -> i64 {
        unsafe { ffi::terminal_get_bell_count(self.handle.as_ptr()) }
    }

    /// Returns the total number of OSC 52 clipboard write events received.
    pub fn clipboard_write_count(&self) -> i64 {
        unsafe { ffi::terminal_get_clipboard_write_count(self.handle.as_ptr()) }
    }

    /// Returns the number of lines currently stored in the scrollback history buffer.
    pub fn scrollback_size(&self) -> i32 {
        unsafe { ffi::terminal_get_scrollback_size(self.handle.as_ptr()) }
    }

    /// Reads grid cells directly into a pre-allocated slice without any heap allocation.
    ///
    /// Returns the number of cells populated.
    pub fn read_grid_into(&self, out: &mut [PackedCell], palette: Palette) -> usize {
        if out.is_empty() {
            return 0;
        }
        let res = unsafe {
            ffi::terminal_read_grid(
                self.handle.as_ptr(),
                out.as_mut_ptr() as *mut i32,
                out.len().min(i32::MAX as usize) as i32,
                palette as i32,
            )
        };
        if res < 0 {
            0
        } else {
            res as usize
        }
    }

    /// Convenience method to allocate and read all screen grid cells for the given palette.
    pub fn read_grid(&self, palette: Palette) -> Vec<PackedCell> {
        let total = (self.rows() * self.cols()).max(0) as usize;
        let mut buf = vec![
            PackedCell {
                codepoint: ' ' as i32,
                fg_rgb: 0,
                bg_rgb: -1,
                attributes: 0,
            };
            total
        ];
        let count = self.read_grid_into(&mut buf, palette);
        buf.truncate(count);
        buf
    }

    /// Reads viewport cells with optional scrollback offset directly into a pre-allocated slice.
    ///
    /// Returns the number of cells populated.
    pub fn read_viewport_into(
        &self,
        out: &mut [PackedCell],
        scrollback_offset: usize,
        palette: Palette,
    ) -> usize {
        if out.is_empty() {
            return 0;
        }
        let res = unsafe {
            ffi::terminal_read_viewport(
                self.handle.as_ptr(),
                out.as_mut_ptr() as *mut i32,
                out.len().min(i32::MAX as usize) as i32,
                scrollback_offset.min(i32::MAX as usize) as i32,
                palette as i32,
            )
        };
        if res < 0 {
            0
        } else {
            res as usize
        }
    }

    /// Convenience method to allocate and read viewport grid cells with optional scrollback offset.
    pub fn read_viewport(&self, scrollback_offset: usize, palette: Palette) -> Vec<PackedCell> {
        let total = (self.rows() * self.cols()).max(0) as usize;
        let mut buf = vec![
            PackedCell {
                codepoint: ' ' as i32,
                fg_rgb: 0,
                bg_rgb: -1,
                attributes: 0,
            };
            total
        ];
        let count = self.read_viewport_into(&mut buf, scrollback_offset, palette);
        buf.truncate(count);
        buf
    }

    fn read_ffi_string<F>(&self, mut reader: F) -> Option<String>
    where
        F: FnMut(*mut i8, i32) -> i32,
    {
        let mut initial_buf = [0u8; 256];
        let len = reader(
            initial_buf.as_mut_ptr() as *mut i8,
            initial_buf.len() as i32,
        );
        if len < 0 {
            return None;
        }
        if len == 0 {
            return Some(String::new());
        }
        let ulen = len as usize;
        if ulen <= initial_buf.len() {
            return Some(
                String::from_utf8(initial_buf[..ulen].to_vec())
                    .unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned()),
            );
        }
        let mut buf = vec![0u8; ulen];
        let written = reader(buf.as_mut_ptr() as *mut i8, buf.len() as i32);
        if written < 0 {
            return None;
        }
        buf.truncate(written as usize);
        Some(
            String::from_utf8(buf)
                .unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned()),
        )
    }

    /// Extracts text from the terminal within the given selection range.
    pub fn extract_selection(
        &self,
        start_row: i32,
        start_col: i32,
        end_row: i32,
        end_col: i32,
    ) -> String {
        let handle = self.handle.as_ptr();
        self.read_ffi_string(|ptr, cap| unsafe {
            ffi::terminal_extract_selection(
                handle, start_row, start_col, end_row, end_col, ptr, cap,
            )
        })
        .unwrap_or_default()
    }

    /// Returns the window title if set via ANSI sequence (OSC 0 or OSC 2), or `None`.
    pub fn window_title(&self) -> Option<String> {
        let handle = self.handle.as_ptr();
        self.read_ffi_string(|ptr, cap| unsafe { ffi::terminal_get_window_title(handle, ptr, cap) })
    }

    /// Returns the payload of the most recent OSC 52 clipboard write, or `None`.
    pub fn last_clipboard_write(&self) -> Option<String> {
        let handle = self.handle.as_ptr();
        self.read_ffi_string(|ptr, cap| unsafe {
            ffi::terminal_get_last_clipboard_write(handle, ptr, cap)
        })
    }
}

#[cfg(kmp_terminal_linked)]
impl Clone for Terminal {
    fn clone(&self) -> Self {
        let raw = unsafe { ffi::terminal_copy(self.handle.as_ptr()) };
        let handle = NonNull::new(raw).expect("terminal_copy returned null handle");
        Terminal { handle }
    }
}

#[cfg(kmp_terminal_linked)]
impl Drop for Terminal {
    fn drop(&mut self) {
        unsafe {
            ffi::terminal_destroy(self.handle.as_ptr());
        }
    }
}

#[cfg(not(kmp_terminal_linked))]
impl Terminal {
    /// Creates a stub terminal emulator when native library is not linked.
    pub fn new(_rows: i32, _cols: i32) -> Self {
        Terminal {
            handle: NonNull::dangling(),
        }
    }

    /// Creates a stub terminal emulator when native library is not linked.
    pub fn with_config(
        _rows: i32,
        _cols: i32,
        _max_scrollback: i32,
        _osc52_policy: Osc52Policy,
    ) -> Self {
        Terminal {
            handle: NonNull::dangling(),
        }
    }

    /// No-op in stub mode.
    pub fn process_bytes(&mut self, _data: &[u8]) {}

    /// No-op in stub mode.
    pub fn process_str(&mut self, _text: &str) {}

    /// Stub resize returning true.
    pub fn resize(&mut self, _rows: i32, _cols: i32) -> bool {
        true
    }

    /// No-op in stub mode.
    pub fn clear(&mut self) {}

    /// No-op in stub mode.
    pub fn reset(&mut self) {}

    /// Returns default rows (24) in stub mode.
    pub fn rows(&self) -> i32 {
        24
    }

    /// Returns default cols (80) in stub mode.
    pub fn cols(&self) -> i32 {
        80
    }

    /// Returns cursor position (0, 0) in stub mode.
    pub fn cursor_pos(&self) -> (i32, i32) {
        (0, 0)
    }

    /// Returns cursor visibility (true) in stub mode.
    pub fn cursor_visible(&self) -> bool {
        true
    }

    /// Returns default mouse tracking mode (None) in stub mode.
    pub fn mouse_tracking_mode(&self) -> MouseTrackingMode {
        MouseTrackingMode::None
    }

    /// Returns SGR mouse mode (false) in stub mode.
    pub fn sgr_mouse_mode(&self) -> bool {
        false
    }

    /// Returns bell count (0) in stub mode.
    pub fn bell_count(&self) -> i64 {
        0
    }

    /// Returns clipboard write count (0) in stub mode.
    pub fn clipboard_write_count(&self) -> i64 {
        0
    }

    /// Returns scrollback size (0) in stub mode.
    pub fn scrollback_size(&self) -> i32 {
        0
    }

    /// Returns 0 populated cells in stub mode.
    pub fn read_grid_into(&self, _out: &mut [PackedCell], _palette: Palette) -> usize {
        0
    }

    /// Returns empty grid vector in stub mode.
    pub fn read_grid(&self, _palette: Palette) -> Vec<PackedCell> {
        Vec::new()
    }

    /// Returns 0 populated cells in stub mode.
    pub fn read_viewport_into(
        &self,
        _out: &mut [PackedCell],
        _scrollback_offset: usize,
        _palette: Palette,
    ) -> usize {
        0
    }

    /// Returns empty viewport vector in stub mode.
    pub fn read_viewport(&self, _scrollback_offset: usize, _palette: Palette) -> Vec<PackedCell> {
        Vec::new()
    }

    /// Returns empty selection string in stub mode.
    pub fn extract_selection(
        &self,
        _start_row: i32,
        _start_col: i32,
        _end_row: i32,
        _end_col: i32,
    ) -> String {
        String::new()
    }

    /// Returns None in stub mode.
    pub fn window_title(&self) -> Option<String> {
        None
    }

    /// Returns None in stub mode.
    pub fn last_clipboard_write(&self) -> Option<String> {
        None
    }
}

#[cfg(not(kmp_terminal_linked))]
impl Clone for Terminal {
    fn clone(&self) -> Self {
        Terminal {
            handle: NonNull::dangling(),
        }
    }
}

#[cfg(not(kmp_terminal_linked))]
impl Drop for Terminal {
    fn drop(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_native_linked() {
        assert_eq!(is_native_linked(), cfg!(kmp_terminal_linked));
        #[cfg(kmp_terminal_linked)]
        assert!(is_native_linked());
    }

    #[test]
    fn test_attributes_bitmask_helpers() {
        let attrs = CellAttributes(
            CellAttributes::BOLD
                | CellAttributes::ITALIC
                | CellAttributes::UNDERLINE
                | CellAttributes::REVERSE
                | CellAttributes::DIM
                | CellAttributes::BLINK
                | CellAttributes::STRIKETHROUGH
                | CellAttributes::OVERLINE
                | CellAttributes::CONCEAL
                | CellAttributes::ROW_WRAPPED,
        );

        assert!(attrs.bold());
        assert!(attrs.italic());
        assert!(attrs.underline());
        assert!(attrs.reverse());
        assert!(attrs.dim());
        assert!(attrs.blink());
        assert!(attrs.strikethrough());
        assert!(attrs.overline());
        assert!(attrs.conceal());
        assert!(attrs.row_wrapped());

        let clean = CellAttributes(0);
        assert!(!clean.bold());
        assert!(!clean.italic());
        assert!(!clean.underline());
        assert!(!clean.reverse());
        assert!(!clean.dim());
        assert!(!clean.blink());
        assert!(!clean.strikethrough());
        assert!(!clean.overline());
        assert!(!clean.conceal());
        assert!(!clean.row_wrapped());

        let cell = PackedCell {
            codepoint: 'Z' as i32,
            fg_rgb: 0xFFFFFF,
            bg_rgb: 0x000000,
            attributes: CellAttributes::BOLD as i32,
        };
        assert_eq!(cell.ch(), 'Z');
        assert!(cell.attrs().bold());
    }

    #[test]
    fn test_creation_dimensions_cursor() {
        if !is_native_linked() {
            return;
        }
        let mut term = Terminal::new(24, 80);
        assert_eq!(term.rows(), 24);
        assert_eq!(term.cols(), 80);
        assert_eq!(term.cursor_pos(), (0, 0));
        assert!(term.cursor_visible());

        term.process_str("Hello");
        assert_eq!(term.cursor_pos(), (0, 5));
    }

    #[test]
    fn test_ansi_colors_and_attributes() {
        if !is_native_linked() {
            return;
        }
        let mut term = Terminal::new(10, 30);
        term.process_str(
            "\x1b[1mBold\x1b[0m\x1b[3mItalic\x1b[0m\x1b[4mUnderline\x1b[0m\x1b[9mStrike\x1b[0m",
        );
        let grid = term.read_grid(Palette::Default);

        // Bold: "Bold" starting at cell 0
        assert_eq!(grid[0].ch(), 'B');
        assert!(grid[0].attrs().bold());
        assert!(!grid[0].attrs().italic());

        // Italic: "Italic" starting at cell 4
        assert_eq!(grid[4].ch(), 'I');
        assert!(grid[4].attrs().italic());
        assert!(!grid[4].attrs().bold());

        // Underline: "Underline" starting at cell 10
        assert_eq!(grid[10].ch(), 'U');
        assert!(grid[10].attrs().underline());

        // Strikethrough: "Strike" starting at cell 19
        assert_eq!(grid[19].ch(), 'S');
        assert!(grid[19].attrs().strikethrough());

        // 24-bit TrueColor RGB check on row 1
        term.process_str("\r\n\x1b[38;2;12;34;56;48;2;78;90;12mColor\x1b[0m");
        let grid2 = term.read_grid(Palette::Default);
        let cell = &grid2[30]; // row 1, col 0
        assert_eq!(cell.ch(), 'C');
        assert_eq!(cell.fg_rgb, (12 << 16) | (34 << 8) | 56);
        assert_eq!(cell.bg_rgb, (78 << 16) | (90 << 8) | 12);
    }

    #[test]
    fn test_zero_alloc_read_grid_and_palette_switching() {
        if !is_native_linked() {
            return;
        }
        let mut term = Terminal::new(4, 10);
        term.process_str("\x1b[31mRED\x1b[0m");

        let mut buffer = [PackedCell {
            codepoint: 0,
            fg_rgb: 0,
            bg_rgb: 0,
            attributes: 0,
        }; 40];

        // 1. Read with Default palette (red = 0xCD0000)
        let count = term.read_grid_into(&mut buffer, Palette::Default);
        assert_eq!(count, 40);
        assert_eq!(buffer[0].ch(), 'R');
        assert_eq!(buffer[0].fg_rgb, 0xCD0000);

        // 2. Read with Nord palette (red = 0xBF616A)
        term.read_grid_into(&mut buffer, Palette::Nord);
        assert_eq!(buffer[0].ch(), 'R');
        assert_eq!(buffer[0].fg_rgb, 0xBF616A);

        // 3. Read with Solarized Light palette (red = 0xDC322F)
        term.read_grid_into(&mut buffer, Palette::SolarizedLight);
        assert_eq!(buffer[0].ch(), 'R');
        assert_eq!(buffer[0].fg_rgb, 0xDC322F);
    }

    #[test]
    fn test_resize_and_line_wrap_scrollback() {
        if !is_native_linked() {
            return;
        }
        let mut term = Terminal::with_config(3, 5, 20, Osc52Policy::AlwaysAllow);
        // Write 6 characters into a 5-column grid; should wrap after 5th char
        term.process_str("123456");

        let grid = term.read_grid(Palette::Default);
        assert_eq!(grid[0].ch(), '1');
        assert_eq!(grid[4].ch(), '5');
        assert_eq!(grid[5].ch(), '6');
        assert!(grid[0].attrs().row_wrapped() || grid[4].attrs().row_wrapped());

        let resized = term.resize(5, 10);
        assert!(resized);
        assert_eq!(term.rows(), 5);
        assert_eq!(term.cols(), 10);

        // Push lines beyond viewport to trigger scrollback
        term.process_str("\n\n\n\n\n\n\n\n");
        assert!(term.scrollback_size() > 0);
    }

    #[test]
    fn test_deep_clone_independence() {
        if !is_native_linked() {
            return;
        }
        let mut term = Terminal::new(5, 10);
        term.process_str("Original");

        let mut cloned = term.clone();
        assert_eq!(term.rows(), cloned.rows());
        assert_eq!(term.cols(), cloned.cols());

        let g1 = term.read_grid(Palette::Default);
        let g2 = cloned.read_grid(Palette::Default);
        assert_eq!(g1[0].ch(), 'O');
        assert_eq!(g2[0].ch(), 'O');

        // Mutating cloned terminal must not affect original
        cloned.process_str("\x1b[1;1HModified");
        let g1_after = term.read_grid(Palette::Default);
        let g2_after = cloned.read_grid(Palette::Default);
        assert_eq!(g1_after[0].ch(), 'O');
        assert_eq!(g2_after[0].ch(), 'M');
    }

    #[test]
    fn test_window_title_and_osc52_clipboard() {
        if !is_native_linked() {
            return;
        }
        let mut term = Terminal::with_config(5, 20, 100, Osc52Policy::AlwaysAllow);
        assert_eq!(term.window_title(), None);
        assert_eq!(term.last_clipboard_write(), None);
        assert_eq!(term.clipboard_write_count(), 0);

        // Set window title via OSC 2
        term.process_str("\x1b]2;Rust Terminal\x07");
        assert_eq!(term.window_title(), Some("Rust Terminal".to_string()));

        // Set clipboard via OSC 52 (Base64 of "Hello Rust" is "SGVsbG8gUnVzdA==")
        term.process_str("\x1b]52;c;SGVsbG8gUnVzdA==\x07");
        assert_eq!(term.clipboard_write_count(), 1);
        assert_eq!(term.last_clipboard_write(), Some("Hello Rust".to_string()));
    }

    #[test]
    fn test_bell_count_clear_reset() {
        if !is_native_linked() {
            return;
        }
        let mut term = Terminal::new(5, 10);
        assert_eq!(term.bell_count(), 0);

        // Send 3 bell signals
        term.process_str("\x07\x07\x07");
        assert_eq!(term.bell_count(), 3);

        term.process_str("Hello");
        assert_eq!(term.cursor_pos(), (0, 5));

        term.clear();
        let grid = term.read_grid(Palette::Default);
        assert_eq!(grid[0].ch(), ' ');

        term.reset();
        assert_eq!(term.cursor_pos(), (0, 0));
    }

    #[test]
    fn test_text_selection_extraction() {
        if !is_native_linked() {
            return;
        }
        let mut term = Terminal::new(5, 20);
        term.process_str("Hello World\r\nSecond Line");

        let sel_hello = term.extract_selection(0, 0, 0, 4);
        assert_eq!(sel_hello, "Hello");

        let sel_world = term.extract_selection(0, 6, 0, 10);
        assert_eq!(sel_world, "World");
    }

    #[test]
    fn test_process_bytes_slice() {
        if !is_native_linked() {
            return;
        }
        let mut term = Terminal::new(4, 15);
        let raw_bytes: [u8; 5] = *b"ABCDE";
        term.process_bytes(&raw_bytes);
        assert_eq!(term.cursor_pos(), (0, 5));
        let grid = term.read_grid(Palette::Default);
        assert_eq!(grid[0].ch(), 'A');
        assert_eq!(grid[4].ch(), 'E');
    }

    #[test]
    fn test_raw_color_from_packed() {
        assert_eq!(RawColor::from_packed(-1), RawColor::Default);
        assert_eq!(RawColor::from_packed(-100), RawColor::Default);

        let tag1 = (1 << 24) | 42;
        assert_eq!(RawColor::from_packed(tag1), RawColor::Indexed(42));

        let tag2 = (2 << 24) | 200;
        assert_eq!(RawColor::from_packed(tag2), RawColor::Indexed(200));

        let rgb = (12 << 16) | (34 << 8) | 56;
        assert_eq!(RawColor::from_packed(rgb), RawColor::Rgb(12, 34, 56));
    }

    #[test]
    fn test_wide_char_attributes_and_glyph() {
        let wide_char_attrs = CellAttributes(CellAttributes::WIDE_CHAR);
        assert!(wide_char_attrs.wide_char());
        assert!(!wide_char_attrs.wide_spacer());

        let wide_spacer_attrs = CellAttributes(CellAttributes::WIDE_SPACER);
        assert!(wide_spacer_attrs.wide_spacer());
        assert!(!wide_spacer_attrs.wide_char());

        let cell_char = PackedCell {
            codepoint: '界' as i32,
            fg_rgb: (1 << 24) | 4,
            bg_rgb: -1,
            attributes: CellAttributes::WIDE_CHAR as i32,
        };
        assert_eq!(cell_char.glyph(), "界");
        assert_eq!(cell_char.fg_raw(), RawColor::Indexed(4));
        assert_eq!(cell_char.bg_raw(), RawColor::Default);

        let cell_spacer = PackedCell {
            codepoint: ' ' as i32,
            fg_rgb: 0,
            bg_rgb: -1,
            attributes: CellAttributes::WIDE_SPACER as i32,
        };
        assert_eq!(cell_spacer.glyph(), "");
    }

    #[test]
    fn test_mouse_tracking_mode() {
        if !is_native_linked() {
            return;
        }
        let mut term = Terminal::new(10, 40);
        assert_eq!(term.mouse_tracking_mode(), MouseTrackingMode::None);

        term.process_str("\x1b[?1000h");
        assert_eq!(term.mouse_tracking_mode(), MouseTrackingMode::Click);

        term.process_str("\x1b[?1002h");
        assert_eq!(term.mouse_tracking_mode(), MouseTrackingMode::CellMotion);

        term.process_str("\x1b[?1003h");
        assert_eq!(term.mouse_tracking_mode(), MouseTrackingMode::AllMotion);

        term.process_str("\x1b[?1000l");
        assert_eq!(term.mouse_tracking_mode(), MouseTrackingMode::None);
    }

    #[test]
    fn test_sgr_mouse_mode() {
        if !is_native_linked() {
            return;
        }
        let mut term = Terminal::new(10, 40);
        assert!(!term.sgr_mouse_mode());

        term.process_str("\x1b[?1006h");
        assert!(term.sgr_mouse_mode());

        term.process_str("\x1b[?1006l");
        assert!(!term.sgr_mouse_mode());
    }

    #[test]
    fn test_read_viewport_into_ansi_raw() {
        if !is_native_linked() {
            return;
        }
        let mut term = Terminal::with_config(3, 10, 50, Osc52Policy::AlwaysAllow);
        term.process_str("Row 0\r\nRow 1\r\nRow 2\r\n\x1b[31mRow 3\x1b[0m\r\nRow 4");

        let mut buffer = [PackedCell {
            codepoint: 0,
            fg_rgb: 0,
            bg_rgb: 0,
            attributes: 0,
        }; 30];

        let count = term.read_viewport_into(&mut buffer, 0, Palette::AnsiRaw);
        assert_eq!(count, 30);

        let vp = term.read_viewport(0, Palette::AnsiRaw);
        assert_eq!(vp.len(), 30);

        if term.scrollback_size() > 0 {
            let scrolled = term.read_viewport(1, Palette::AnsiRaw);
            assert_eq!(scrolled.len(), 30);
        }
    }
}
