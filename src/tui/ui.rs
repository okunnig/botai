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

        for row in 0..area.height {
            for col in 0..area.width {
                let target_x = area.x + col;
                let target_y = area.y + row;

                if let Some(cell) = self.vt.cell(col, row) {
                    let mut style = Style::default();

                    if let Some(fg) = cell.fg {
                        style = style.fg(vt_color_to_ratatui(fg));
                    }
                    if let Some(bg) = cell.bg {
                        style = style.bg(vt_color_to_ratatui(bg));
                    }
                    if cell.bold {
                        style = style.add_modifier(Modifier::BOLD);
                    }
                    if cell.italic {
                        style = style.add_modifier(Modifier::ITALIC);
                    }
                    if cell.underline {
                        style = style.add_modifier(Modifier::UNDERLINED);
                    }
                    if cell.inverse {
                        style = style.add_modifier(Modifier::REVERSED);
                    }

                    if let Some(rat_cell) = buf.cell_mut((target_x, target_y)) {
                        if !cell.text.is_empty() {
                            rat_cell.set_symbol(&cell.text);
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
}
