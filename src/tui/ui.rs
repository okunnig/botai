use crate::session::VtScreen;
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color as RatatuiColor, Modifier, Style},
    widgets::Widget,
};

/// Converts a `vt100::Color` into a `ratatui::style::Color`.
pub fn vt_color_to_ratatui(color: vt100::Color) -> RatatuiColor {
    match color {
        vt100::Color::Idx(idx) => match idx {
            0 => RatatuiColor::Black,
            1 => RatatuiColor::Red,
            2 => RatatuiColor::Green,
            3 => RatatuiColor::Yellow,
            4 => RatatuiColor::Blue,
            5 => RatatuiColor::Magenta,
            6 => RatatuiColor::Cyan,
            7 => RatatuiColor::Gray,
            8 => RatatuiColor::DarkGray,
            9 => RatatuiColor::LightRed,
            10 => RatatuiColor::LightGreen,
            11 => RatatuiColor::LightYellow,
            12 => RatatuiColor::LightBlue,
            13 => RatatuiColor::LightMagenta,
            14 => RatatuiColor::LightCyan,
            15 => RatatuiColor::White,
            other => RatatuiColor::Indexed(other),
        },
        vt100::Color::Rgb(r, g, b) => RatatuiColor::Rgb(r, g, b),
        vt100::Color::Default => RatatuiColor::Reset,
    }
}

/// The vertical height (in terminal rows) allocated to the status bar at the top of the host terminal.
pub const STATUS_BAR_HEIGHT: u16 = 1;

/// Splits the given terminal area into status bar and terminal viewport areas.
pub fn compute_layout(area: Rect) -> (Rect, Rect) {
    if area.height == 0 || area.width == 0 {
        return (Rect::default(), Rect::default());
    }

    let status_area = Rect {
        x: area.x,
        y: area.y,
        width: area.width,
        height: STATUS_BAR_HEIGHT.min(area.height),
    };

    let terminal_area = Rect {
        x: area.x,
        y: area.y.saturating_add(STATUS_BAR_HEIGHT),
        width: area.width,
        height: area.height.saturating_sub(STATUS_BAR_HEIGHT),
    };

    (status_area, terminal_area)
}

/// A Ratatui widget that renders a `VtScreen` grid into the terminal buffer.
pub struct TerminalView<'a> {
    vt: &'a VtScreen,
}

impl<'a> TerminalView<'a> {
    /// Creates a new `TerminalView` for the given virtual terminal screen.
    pub fn new(vt: &'a VtScreen) -> Self {
        Self { vt }
    }

    /// Renders the virtual terminal contents into the given buffer area.
    pub fn render_to_buffer(&self, area: Rect, buf: &mut Buffer) {
        if area.height == 0 || area.width == 0 {
            return;
        }

        #[cfg(feature = "kmp-terminal")]
        if let Some((vt_cols, vt_rows, cells)) = self.vt.read_kmp_viewport() {
            for row in 0..area.height {
                for col in 0..area.width {
                    let target_x = area.x + col;
                    let target_y = area.y + row;

                    if row < vt_rows && col < vt_cols {
                        let idx = (row as usize) * (vt_cols as usize) + (col as usize);
                        if let Some(cell) = cells.get(idx) {
                            let mut style = Style::default();

                            match cell.fg_raw() {
                                kmp_terminal_rs::RawColor::Default => {}
                                kmp_terminal_rs::RawColor::Indexed(idx) => {
                                    style = style.fg(vt_color_to_ratatui(vt100::Color::Idx(idx)));
                                }
                                kmp_terminal_rs::RawColor::Rgb(r, g, b) => {
                                    style = style.fg(RatatuiColor::Rgb(r, g, b));
                                }
                            }

                            match cell.bg_raw() {
                                kmp_terminal_rs::RawColor::Default => {}
                                kmp_terminal_rs::RawColor::Indexed(idx) => {
                                    style = style.bg(vt_color_to_ratatui(vt100::Color::Idx(idx)));
                                }
                                kmp_terminal_rs::RawColor::Rgb(r, g, b) => {
                                    style = style.bg(RatatuiColor::Rgb(r, g, b));
                                }
                            }

                            let attrs = cell.attrs();
                            if attrs.bold() {
                                style = style.add_modifier(Modifier::BOLD);
                            }
                            if attrs.dim() {
                                style = style.add_modifier(Modifier::DIM);
                            }
                            if attrs.italic() {
                                style = style.add_modifier(Modifier::ITALIC);
                            }
                            if attrs.underline() {
                                style = style.add_modifier(Modifier::UNDERLINED);
                            }
                            if attrs.reverse() {
                                style = style.add_modifier(Modifier::REVERSED);
                            }

                            if let Some(rat_cell) = buf.cell_mut((target_x, target_y)) {
                                if attrs.wide_spacer() {
                                    rat_cell.set_symbol("");
                                    rat_cell.set_style(style);
                                } else if cell.codepoint > 0 {
                                    let s = cell.glyph();
                                    if s.is_empty() {
                                        rat_cell.set_char(' ');
                                    } else {
                                        rat_cell.set_symbol(&s);
                                    }
                                    rat_cell.set_style(style);
                                } else {
                                    rat_cell.set_char(' ');
                                    rat_cell.set_style(style);
                                }
                            }
                            continue;
                        }
                    }

                    if let Some(rat_cell) = buf.cell_mut((target_x, target_y)) {
                        rat_cell.set_char(' ');
                        rat_cell.set_style(Style::default());
                    }
                }
            }
            return;
        }

        let screen = self.vt.screen();

        for row in 0..area.height {
            for col in 0..area.width {
                let target_x = area.x + col;
                let target_y = area.y + row;

                if let Some(cell) = screen.cell(row, col) {
                    let mut style = Style::default();

                    match cell.fgcolor() {
                        vt100::Color::Default => {}
                        other => style = style.fg(vt_color_to_ratatui(other)),
                    }
                    match cell.bgcolor() {
                        vt100::Color::Default => {}
                        other => style = style.bg(vt_color_to_ratatui(other)),
                    }
                    if cell.bold() {
                        style = style.add_modifier(Modifier::BOLD);
                    }
                    if cell.dim() {
                        style = style.add_modifier(Modifier::DIM);
                    }
                    if cell.italic() {
                        style = style.add_modifier(Modifier::ITALIC);
                    }
                    if cell.underline() {
                        style = style.add_modifier(Modifier::UNDERLINED);
                    }
                    if cell.inverse() {
                        style = style.add_modifier(Modifier::REVERSED);
                    }

                    if let Some(rat_cell) = buf.cell_mut((target_x, target_y)) {
                        let contents = cell.contents();
                        if !contents.is_empty() {
                            rat_cell.set_symbol(contents);
                        } else {
                            rat_cell.set_char(' ');
                        }
                        rat_cell.set_style(style);
                    }
                } else if let Some(rat_cell) = buf.cell_mut((target_x, target_y)) {
                    rat_cell.set_char(' ');
                    rat_cell.set_style(Style::default());
                }
            }
        }
    }
}

impl<'a> Widget for TerminalView<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        self.render_to_buffer(area, buf);
    }
}

impl<'a> Widget for &TerminalView<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        self.render_to_buffer(area, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vt_color_to_ratatui_mapping() {
        assert_eq!(
            vt_color_to_ratatui(vt100::Color::Idx(0)),
            RatatuiColor::Black
        );
        assert_eq!(vt_color_to_ratatui(vt100::Color::Idx(1)), RatatuiColor::Red);
        assert_eq!(
            vt_color_to_ratatui(vt100::Color::Idx(2)),
            RatatuiColor::Green
        );
        assert_eq!(
            vt_color_to_ratatui(vt100::Color::Idx(200)),
            RatatuiColor::Indexed(200)
        );
        assert_eq!(
            vt_color_to_ratatui(vt100::Color::Rgb(10, 20, 30)),
            RatatuiColor::Rgb(10, 20, 30)
        );
        assert_eq!(
            vt_color_to_ratatui(vt100::Color::Default),
            RatatuiColor::Reset
        );
    }

    #[test]
    fn test_compute_layout_dimensions() {
        let area = Rect::new(0, 0, 80, 24);
        let (status_area, term_area) = compute_layout(area);

        assert_eq!(status_area, Rect::new(0, 0, 80, 1));
        assert_eq!(term_area, Rect::new(0, 1, 80, 23));

        // Edge case: 0 height
        let (zero_status, zero_term) = compute_layout(Rect::new(0, 0, 0, 0));
        assert_eq!(zero_status, Rect::default());
        assert_eq!(zero_term, Rect::default());
    }

    #[test]
    fn test_terminal_view_renders_vt_cells_and_styles() {
        let mut vt = VtScreen::new(80, 24);
        vt.process(b"\x1b[1;32mHello Botai!\x1b[0m");

        let view = TerminalView::new(&vt);
        let mut buffer = Buffer::empty(Rect::new(0, 0, 80, 24));
        view.render_to_buffer(Rect::new(0, 0, 80, 24), &mut buffer);

        let cell_0 = buffer.cell((0, 0)).expect("cell 0,0");
        assert_eq!(cell_0.symbol(), "H");
        assert_eq!(cell_0.fg, RatatuiColor::Green);
        assert!(cell_0.modifier.contains(Modifier::BOLD));

        let cell_1 = buffer.cell((1, 0)).expect("cell 1,0");
        assert_eq!(cell_1.symbol(), "e");
        assert_eq!(cell_1.fg, RatatuiColor::Green);

        let cell_space = buffer.cell((5, 0)).expect("cell 5,0");
        assert_eq!(cell_space.symbol(), " ");
    }

    #[test]
    fn test_terminal_view_renders_all_modifiers_and_bg() {
        let mut vt = VtScreen::new(40, 10);
        // Italic (3), Underline (4), Inverse (7), Red fg (31), Blue bg (44)
        vt.process(b"\x1b[3;4;7;31;44mZ\x1b[0m");

        let view = TerminalView::new(&vt);
        let mut buffer = Buffer::empty(Rect::new(0, 0, 40, 10));
        view.render_to_buffer(Rect::new(0, 0, 40, 10), &mut buffer);

        let cell_z = buffer.cell((0, 0)).expect("cell 0,0");
        assert_eq!(cell_z.symbol(), "Z");
        assert_eq!(cell_z.fg, RatatuiColor::Red);
        assert_eq!(cell_z.bg, RatatuiColor::Blue);
        assert!(cell_z.modifier.contains(Modifier::ITALIC));
        assert!(cell_z.modifier.contains(Modifier::UNDERLINED));
        assert!(cell_z.modifier.contains(Modifier::REVERSED));

        // Area exceeding vt screen dimensions (e.g. height 20 when vt is 10)
        let mut large_buffer = Buffer::empty(Rect::new(0, 0, 40, 20));
        view.render_to_buffer(Rect::new(0, 0, 40, 20), &mut large_buffer);
        let out_of_bounds_cell = large_buffer.cell((0, 15)).expect("cell 0,15");
        assert_eq!(out_of_bounds_cell.symbol(), " ");
        assert_eq!(out_of_bounds_cell.fg, RatatuiColor::Reset);
        assert_eq!(out_of_bounds_cell.bg, RatatuiColor::Reset);
        assert_eq!(out_of_bounds_cell.modifier, Modifier::empty());
    }

    #[test]
    fn test_terminal_view_renders_dim() {
        let mut vt = VtScreen::new(40, 10);
        vt.process(b"\x1b[2mdim\x1b[0m normal \x1b[2mA\x1b[22mB");

        let view = TerminalView::new(&vt);
        let mut buffer = Buffer::empty(Rect::new(0, 0, 40, 10));
        view.render_to_buffer(Rect::new(0, 0, 40, 10), &mut buffer);

        let cell_d = buffer.cell((0, 0)).expect("cell 0,0");
        assert_eq!(cell_d.symbol(), "d");
        assert!(cell_d.modifier.contains(Modifier::DIM));
        assert!(!cell_d.modifier.contains(Modifier::BOLD));

        let cell_n = buffer.cell((4, 0)).expect("cell 4,0");
        assert_eq!(cell_n.symbol(), "n");
        assert!(!cell_n.modifier.contains(Modifier::DIM));

        let cell_a = buffer.cell((11, 0)).expect("cell 11,0");
        assert_eq!(cell_a.symbol(), "A");
        assert!(cell_a.modifier.contains(Modifier::DIM));

        let cell_b = buffer.cell((12, 0)).expect("cell 12,0");
        assert_eq!(cell_b.symbol(), "B");
        assert!(!cell_b.modifier.contains(Modifier::DIM));
    }
}
