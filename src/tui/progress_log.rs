//! Shared layout, log box and scroll handling for the setup and destroy progress modals.

use std::borrow::Cow;

use crossterm::event::{MouseEvent, MouseEventKind};
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Widget},
};

use crate::tui::theme::Theme;

const SPINNER_CHARS: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

/// Whether `⚠` / "Warning" log lines get their own highlight color.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WarningHighlight {
    Plain,
    Highlighted,
}

/// Fixed wording and log styling of one kind of progress modal.
pub(crate) struct ProgressModalText {
    pub log_title: &'static str,
    pub warnings: WarningHighlight,
    pub running_footer: &'static str,
    pub finished_footer: &'static str,
}

/// Log, step and completion state shared by the setup and destroy progress modals.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgressLog {
    pub logs: Vec<String>,
    pub current_step: usize,
    pub total_steps: usize,
    pub status_message: String,
    pub is_finished: bool,
    pub error: Option<String>,
    pub spinner_frame: usize,
    pub scroll_offset: usize,
}

impl ProgressLog {
    /// Creates an unfinished progress log at step 1 of 4 showing `status_message`.
    pub fn new(status_message: impl Into<String>) -> Self {
        Self {
            logs: Vec::new(),
            current_step: 1,
            total_steps: 4,
            status_message: status_message.into(),
            is_finished: false,
            error: None,
            spinner_frame: 0,
            scroll_offset: 0,
        }
    }

    /// Advances the spinner animation frame.
    pub fn tick(&mut self) {
        self.spinner_frame = self.spinner_frame.wrapping_add(1);
    }

    /// Records a step progress update.
    pub fn set_step(&mut self, current: usize, total: usize, message: String) {
        self.current_step = current;
        self.total_steps = total;
        self.status_message = message;
    }

    /// Scrolls the log viewport upwards (into older log lines).
    pub fn scroll_up(&mut self, lines: usize) {
        self.scroll_offset = self
            .scroll_offset
            .saturating_add(lines)
            .min(self.logs.len());
    }

    /// Scrolls the log viewport downwards (towards latest log lines).
    pub fn scroll_down(&mut self, lines: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(lines);
    }

    /// Applies mouse wheel scrolling to the log viewport. Returns `true` if the event was handled.
    pub fn handle_mouse(&mut self, mouse: MouseEvent) -> bool {
        match mouse.kind {
            MouseEventKind::ScrollUp => {
                self.scroll_up(3);
                true
            }
            MouseEventKind::ScrollDown => {
                self.scroll_down(3);
                true
            }
            _ => false,
        }
    }

    /// Renders the modal frame, `header_lines`, the log box and the footer.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn render<'a>(
        &self,
        text: &ProgressModalText,
        title: impl Into<Line<'a>>,
        border_color: Color,
        header_lines: Vec<Line>,
        area: Rect,
        buf: &mut Buffer,
        theme: &Theme,
    ) {
        let Some(areas) = render_modal_frame(area, buf, title, border_color) else {
            return;
        };
        Paragraph::new(header_lines).render(areas.header, buf);
        render_log_box(
            &self.logs,
            self.scroll_offset,
            text.log_title,
            text.warnings,
            areas.log,
            buf,
            theme,
        );
        self.render_footer(text, areas.footer, buf, theme);
    }

    /// Renders the close hint once finished or failed, otherwise the animated spinner.
    fn render_footer(&self, text: &ProgressModalText, area: Rect, buf: &mut Buffer, theme: &Theme) {
        let footer = if self.error.is_some() || self.is_finished {
            Line::from(Span::styled(
                text.finished_footer,
                Style::default()
                    .fg(theme.key_hint)
                    .add_modifier(Modifier::BOLD),
            ))
        } else {
            let spinner = SPINNER_CHARS[self.spinner_frame % SPINNER_CHARS.len()];
            Line::from(vec![
                Span::styled(
                    format!(" {spinner} "),
                    Style::default()
                        .fg(theme.status_busy)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    text.running_footer,
                    Style::default().fg(theme.status_bar_fg),
                ),
            ])
        };
        Paragraph::new(footer).render(area, buf);
    }
}

/// Inner areas of a progress modal: step header, log box and footer.
struct ProgressModalAreas {
    header: Rect,
    log: Rect,
    footer: Rect,
}

/// Clears and draws the centered, bordered modal frame and splits its interior.
/// Returns `None` if `area` is too small to render the modal.
fn render_modal_frame<'a>(
    area: Rect,
    buf: &mut Buffer,
    title: impl Into<Line<'a>>,
    border_color: Color,
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
fn log_line_style(line: &str, theme: &Theme, warnings: WarningHighlight) -> Style {
    if line.starts_with('✗') || line.contains("Error") || line.contains("Failed") {
        Style::default().fg(theme.status_error)
    } else if warnings == WarningHighlight::Highlighted
        && (line.starts_with('⚠') || line.contains("Warning"))
    {
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
fn render_log_box(
    logs: &[String],
    scroll_offset: usize,
    title: &str,
    warnings: WarningHighlight,
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
                log_line_style(line, theme, warnings),
            ))
        })
        .collect();

    Paragraph::new(log_lines).render(log_inner, buf);
}
