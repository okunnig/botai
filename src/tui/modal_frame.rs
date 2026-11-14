//! Geometry and bordered frame shared by the centred popup modals.

use ratatui::{
    buffer::Buffer,
    layout::{Rect, Size},
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Borders, Clear, Widget},
};

/// Centres a modal of up to `max` cells in `area`, keeping a 2-column and 1-row margin.
/// Returns `None` if the margin-limited size is smaller than `min`.
pub(crate) fn centered_modal(area: Rect, max: Size, min: Size) -> Option<Rect> {
    let width = max.width.min(area.width.saturating_sub(4));
    let height = max.height.min(area.height.saturating_sub(2));

    if width < min.width || height < min.height {
        return None;
    }

    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Some(Rect::new(x, y, width, height))
}

/// Clears `modal_area`, draws a bold bordered block titled `title` and returns its interior.
pub(crate) fn render_modal_frame<'a>(
    modal_area: Rect,
    buf: &mut Buffer,
    title: impl Into<Line<'a>>,
    border_color: Color,
) -> Rect {
    Clear.render(modal_area, buf);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(
            Style::default()
                .fg(border_color)
                .add_modifier(Modifier::BOLD),
        );
    let inner = block.inner(modal_area);
    block.render(modal_area, buf);
    inner
}
