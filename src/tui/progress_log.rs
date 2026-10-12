//! Shared layout, log box and scroll handling for the setup and destroy progress modals.

use std::borrow::Cow;

use crossterm::event::{MouseEvent, MouseEventKind};
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Widget},
};

use crate::tui::theme::Theme;

const SPINNER_CHARS: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

/// Inner areas of a progress modal: step header, log box and footer.
pub(crate) struct ProgressModalAreas {
    pub header: Rect,
    pub log: Rect,
    pub footer: Rect,
}

/// Scrolls a log viewport upwards (into older log lines), clamped to `total_logs`.
pub(crate) fn scroll_up(scroll_offset: &mut usize, lines: usize, total_logs: usize) {
    *scroll_offset = scroll_offset.saturating_add(lines).min(total_logs);
}

/// Scrolls a log viewport downwards (towards latest log lines).
pub(crate) fn scroll_down(scroll_offset: &mut usize, lines: usize) {
    *scroll_offset = scroll_offset.saturating_sub(lines);
}

/// Applies mouse wheel scrolling to a log viewport. Returns `true` if the event was handled.
pub(crate) fn handle_mouse_scroll(
    scroll_offset: &mut usize,
    total_logs: usize,
    mouse: MouseEvent,
) -> bool {
    match mouse.kind {
        MouseEventKind::ScrollUp => {
            scroll_up(scroll_offset, 3, total_logs);
            true
        }
        MouseEventKind::ScrollDown => {
            scroll_down(scroll_offset, 3);
            true
        }
        _ => false,
    }
}

/// Clears and draws the centered, bordered modal frame and splits its interior.
/// Returns `None` if `area` is too small to render the modal.
pub(crate) fn render_modal_frame(
    area: Rect,
    buf: &mut Buffer,
    title: String,
    border_color: ratatui::style::Color,
) -> Option<ProgressModalAreas> {
    let width = 76.min(area.width.saturating_sub(4));
    let height = 18.min(area.height.saturating_sub(2));

    if width < 24 || height < 8 {
        return None;
    }

    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let modal_area = Rect::new(x, y, width, height);

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

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(3),
            Constraint::Length(1),
        ])
        .split(inner);

    Some(ProgressModalAreas {
        header: chunks[0],
        log: chunks[1],
        footer: chunks[2],
    })
}

/// Builds a header line with a bold label followed by a styled value.
pub(crate) fn label_value_line<'a>(
    theme: &Theme,
    label: impl Into<Cow<'a, str>>,
    value: impl Into<Cow<'a, str>>,
    value_style: Style,
) -> Line<'a> {
    Line::from(vec![
        Span::styled(
            label,
            Style::default()
                .fg(theme.modal_label)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(value, value_style),
    ])
}

/// Builds the two header lines shown when the operation failed.
pub(crate) fn failure_header_lines<'a>(
    theme: &Theme,
    status_text: &'a str,
    err: &str,
) -> Vec<Line<'a>> {
    vec![
        label_value_line(
            theme,
            "Status: ",
            status_text,
            Style::default()
                .fg(theme.status_error)
                .add_modifier(Modifier::BOLD),
        ),
        Line::from(vec![Span::styled(
            format!("Reason: {err}"),
            Style::default().fg(theme.status_error),
        )]),
    ]
}

/// Builds the "Step [n/m]: <message>" header line.
pub(crate) fn step_line<'a>(
    theme: &Theme,
    current_step: usize,
    total_steps: usize,
    status_message: &'a str,
) -> Line<'a> {
    label_value_line(
        theme,
        format!("Step [{current_step}/{total_steps}]: "),
        status_message,
        Style::default()
            .fg(theme.status_waiting)
            .add_modifier(Modifier::BOLD),
    )
}

/// Returns the style for a log line based on its status prefix or keywords.
/// `highlight_warnings` colors `⚠` / "Warning" lines with the waiting color.
pub(crate) fn log_line_style(line: &str, theme: &Theme, highlight_warnings: bool) -> Style {
    if line.starts_with('✗') || line.contains("Error") || line.contains("Failed") {
        Style::default().fg(theme.status_error)
    } else if highlight_warnings && (line.starts_with('⚠') || line.contains("Warning")) {
        Style::default().fg(theme.status_waiting)
    } else if line.starts_with('✓') {
        Style::default()
            .fg(theme.status_done)
            .add_modifier(Modifier::BOLD)
    } else if line.starts_with('[') {
        Style::default().fg(theme.status_busy)
    } else {
        Style::default().fg(theme.input_text)
    }
}

/// Renders the bordered log box showing the visible window of `logs` for `scroll_offset`.
pub(crate) fn render_log_box(
    logs: &[String],
    scroll_offset: usize,
    title: &str,
    highlight_warnings: bool,
    area: Rect,
    buf: &mut Buffer,
    theme: &Theme,
) {
    let log_box_block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(theme.border_unfocused));
    let log_inner = log_box_block.inner(area);
    log_box_block.render(area, buf);

    let total_logs = logs.len();
    let max_lines = log_inner.height as usize;
    let effective_offset = scroll_offset.min(total_logs.saturating_sub(max_lines));
    let end_idx = total_logs.saturating_sub(effective_offset);
    let start_idx = end_idx.saturating_sub(max_lines);

    let log_lines: Vec<Line> = logs[start_idx..end_idx]
        .iter()
        .map(|line| {
            Line::from(Span::styled(
                line.as_str(),
                log_line_style(line, theme, highlight_warnings),
            ))
        })
        .collect();

    Paragraph::new(log_lines).render(log_inner, buf);
}

/// Builds the footer shown once the operation has finished.
pub(crate) fn finished_footer<'a>(hint: &'a str, theme: &Theme) -> Line<'a> {
    Line::from(Span::styled(
        hint,
        Style::default()
            .fg(theme.key_hint)
            .add_modifier(Modifier::BOLD),
    ))
}

/// Builds the animated spinner footer shown while the operation is running.
pub(crate) fn spinner_footer<'a>(spinner_frame: usize, text: &'a str, theme: &Theme) -> Line<'a> {
    let spinner = SPINNER_CHARS[spinner_frame % SPINNER_CHARS.len()];
    Line::from(vec![
        Span::styled(
            format!(" {spinner} "),
            Style::default()
                .fg(theme.status_busy)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(text, Style::default().fg(theme.status_bar_fg)),
    ])
}
