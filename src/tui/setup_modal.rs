use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Widget},
};

use crate::config::Config;
use crate::project::{ProjectBuilder, ProjectSetupEvent};
use crate::session::{ProjectSession, PtySession, VtScreen};
use crate::tui::theme::Theme;

/// Events emitted during asynchronous project creation and setup.
#[allow(clippy::large_enum_variant)]
pub enum SetupProgressEvent {
    Log(String),
    Step {
        current: usize,
        total: usize,
        message: String,
    },
    Success {
        project_session: ProjectSession,
        pty: PtySession,
        vt: VtScreen,
    },
    Error(String),
}

impl std::fmt::Debug for SetupProgressEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Log(s) => f.debug_tuple("Log").field(s).finish(),
            Self::Step {
                current,
                total,
                message,
            } => f
                .debug_struct("Step")
                .field("current", current)
                .field("total", total)
                .field("message", message)
                .finish(),
            Self::Success {
                project_session, ..
            } => f
                .debug_struct("Success")
                .field("project_session", project_session)
                .finish(),
            Self::Error(s) => f.debug_tuple("Error").field(s).finish(),
        }
    }
}

/// State for the asynchronous project setup and progress modal.
pub struct SetupModalState {
    pub project_name: String,
    pub template_name: String,
    pub prompt: Option<String>,
    pub logs: Vec<String>,
    pub current_step: usize,
    pub total_steps: usize,
    pub status_message: String,
    pub is_finished: bool,
    pub error: Option<String>,
    pub receiver: Option<Receiver<SetupProgressEvent>>,
    pub spinner_frame: usize,
    pub scroll_offset: usize,
}

impl SetupModalState {
    /// Creates a new `SetupModalState` for monitoring project creation.
    pub fn new(
        project_name: impl Into<String>,
        template_name: impl Into<String>,
        prompt: Option<String>,
        receiver: Receiver<SetupProgressEvent>,
    ) -> Self {
        Self {
            project_name: project_name.into(),
            template_name: template_name.into(),
            prompt,
            logs: Vec::new(),
            current_step: 1,
            total_steps: 4,
            status_message: "Initializing project setup...".to_string(),
            is_finished: false,
            error: None,
            receiver: Some(receiver),
            spinner_frame: 0,
            scroll_offset: 0,
        }
    }

    /// Advances the spinner animation frame.
    pub fn tick(&mut self) {
        self.spinner_frame = self.spinner_frame.wrapping_add(1);
    }

    /// Polls the event receiver and updates logs, step progress, and termination state.
    /// Returns `Some((ProjectSession, PtySession, VtScreen))` if setup finished successfully.
    pub fn poll_events(&mut self) -> Option<(ProjectSession, PtySession, VtScreen)> {
        let mut success = None;
        if let Some(ref rx) = self.receiver {
            while let Ok(event) = rx.try_recv() {
                match event {
                    SetupProgressEvent::Log(line) => {
                        self.logs.push(line);
                    }
                    SetupProgressEvent::Step {
                        current,
                        total,
                        message,
                    } => {
                        self.current_step = current;
                        self.total_steps = total;
                        self.status_message = message;
                    }
                    SetupProgressEvent::Success {
                        project_session,
                        pty,
                        vt,
                    } => {
                        self.is_finished = true;
                        success = Some((project_session, pty, vt));
                    }
                    SetupProgressEvent::Error(err) => {
                        self.error = Some(err);
                        self.is_finished = true;
                    }
                }
            }
        }
        success
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

    /// Handles mouse events (e.g. scroll wheel) on the setup modal.
    pub fn handle_mouse(&mut self, mouse: crossterm::event::MouseEvent) -> bool {
        match mouse.kind {
            crossterm::event::MouseEventKind::ScrollUp => {
                self.scroll_up(3);
                true
            }
            crossterm::event::MouseEventKind::ScrollDown => {
                self.scroll_down(3);
                true
            }
            _ => false,
        }
    }

    /// Handles keyboard events when the setup modal is displayed.
    /// Returns `true` if the modal should be dismissed.
    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        if self.is_finished || self.error.is_some() {
            matches!(key.code, KeyCode::Esc | KeyCode::Enter)
        } else {
            false
        }
    }
}

/// Background worker function that executes git repository resolution, cloning/fetching,
/// worktree creation, context generation, and PTY process spawning.
pub fn run_setup_worker(
    template_name: String,
    project_name: String,
    prompt: Option<String>,
    config: Option<Config>,
    cwd: PathBuf,
    viewport_size: (u16, u16),
    tx: Sender<SetupProgressEvent>,
) {
    let cfg = config.unwrap_or_else(|| {
        Config::load_from_str("default_agent = \"claude\"\nrepos_dir = \".botai_repos\"\n")
            .unwrap_or_else(|_| Config {
                default_agent: "claude".to_string(),
                theme: "tokyo-night".to_string(),
                repos_dir: PathBuf::from(".botai_repos"),
                agents: HashMap::new(),
                templates: HashMap::new(),
            })
    });

    let (cols, rows) = viewport_size;
    let mut builder = ProjectBuilder::new(cfg, &template_name, &project_name)
        .with_cwd(cwd)
        .with_viewport(cols, rows);

    if let Some(ref p) = prompt {
        builder = builder.with_prompt(p);
    }

    let tx_sink = tx.clone();
    let progress_sink = move |event: ProjectSetupEvent| match event {
        ProjectSetupEvent::Step {
            current,
            total,
            message,
        } => {
            let _ = tx_sink.send(SetupProgressEvent::Step {
                current,
                total,
                message,
            });
        }
        ProjectSetupEvent::Log(msg) => {
            let _ = tx_sink.send(SetupProgressEvent::Log(msg));
        }
        ProjectSetupEvent::Success => {}
        ProjectSetupEvent::Error(err) => {
            let _ = tx_sink.send(SetupProgressEvent::Error(err));
        }
    };

    match builder.build(progress_sink) {
        Ok((ps, pty, vt)) => {
            let _ = tx.send(SetupProgressEvent::Log(
                "✓ Project setup completed successfully!".to_string(),
            ));
            let _ = tx.send(SetupProgressEvent::Success {
                project_session: ps,
                pty,
                vt,
            });
        }
        Err(err) => {
            let err_msg = err.to_string();
            let _ = tx.send(SetupProgressEvent::Log(format!("✗ Error: {err_msg}")));
            let _ = tx.send(SetupProgressEvent::Error(err_msg));
        }
    }
}

/// Renders the live setup progress and log modal into the given `Buffer`.
pub fn render_setup_modal(modal: &SetupModalState, area: Rect, buf: &mut Buffer, theme: &Theme) {
    let width = 76.min(area.width.saturating_sub(4));
    let height = 18.min(area.height.saturating_sub(2));

    if width < 24 || height < 8 {
        return;
    }

    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let modal_area = Rect::new(x, y, width, height);

    // Clear background
    Clear.render(modal_area, buf);

    let is_error = modal.error.is_some();
    let title = if is_error {
        format!(" Project Setup Failed: {} ", modal.project_name)
    } else {
        format!(" Setting Up Project: {} ", modal.project_name)
    };

    let border_color = if is_error {
        theme.status_error
    } else {
        theme.status_busy
    };

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

    // Split inner area into: Step Header, Log Box, and Footer
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(3),
            Constraint::Length(1),
        ])
        .split(inner);

    let header_area = chunks[0];
    let log_area = chunks[1];
    let footer_area = chunks[2];

    // 1. Header (Progress Step)
    let header_lines = if let Some(ref err) = modal.error {
        vec![
            Line::from(vec![
                Span::styled(
                    "Status: ",
                    Style::default()
                        .fg(theme.modal_label)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    "Setup Failed",
                    Style::default()
                        .fg(theme.status_error)
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(vec![Span::styled(
                format!("Reason: {err}"),
                Style::default().fg(theme.status_error),
            )]),
        ]
    } else {
        vec![
            Line::from(vec![
                Span::styled(
                    format!("Step [{}/{}]: ", modal.current_step, modal.total_steps),
                    Style::default()
                        .fg(theme.modal_label)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    &modal.status_message,
                    Style::default()
                        .fg(theme.status_waiting)
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(Span::styled(
                format!("Template: {}", modal.template_name),
                Style::default().fg(theme.status_bar_fg),
            )),
        ]
    };
    Paragraph::new(header_lines).render(header_area, buf);

    // 2. Log Box
    let log_box_block = Block::default()
        .borders(Borders::ALL)
        .title(" Setup Logs ")
        .border_style(Style::default().fg(theme.border_unfocused));
    let log_inner = log_box_block.inner(log_area);
    log_box_block.render(log_area, buf);

    let total_logs = modal.logs.len();
    let max_lines = log_inner.height as usize;
    let effective_offset = modal
        .scroll_offset
        .min(total_logs.saturating_sub(max_lines));
    let end_idx = total_logs.saturating_sub(effective_offset);
    let start_idx = end_idx.saturating_sub(max_lines);
    let log_slice: Vec<&String> = if total_logs > 0 {
        modal.logs[start_idx..end_idx].iter().collect()
    } else {
        Vec::new()
    };

    let log_lines: Vec<Line> = log_slice
        .into_iter()
        .map(|line| {
            let style =
                if line.starts_with('✗') || line.contains("Error") || line.contains("Failed") {
                    Style::default().fg(theme.status_error)
                } else if line.starts_with('✓') {
                    Style::default()
                        .fg(theme.status_done)
                        .add_modifier(Modifier::BOLD)
                } else if line.starts_with('[') {
                    Style::default().fg(theme.status_busy)
                } else {
                    Style::default().fg(theme.input_text)
                };
            Line::from(Span::styled(line, style))
        })
        .collect();

    Paragraph::new(log_lines).render(log_inner, buf);

    // 3. Footer
    let footer_text = if is_error || modal.is_finished {
        Line::from(Span::styled(
            " [Esc] / [Enter] Close ",
            Style::default()
                .fg(theme.key_hint)
                .add_modifier(Modifier::BOLD),
        ))
    } else {
        const SPINNER_CHARS: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
        let spinner = SPINNER_CHARS[modal.spinner_frame % SPINNER_CHARS.len()];
        Line::from(vec![
            Span::styled(
                format!(" {spinner} "),
                Style::default()
                    .fg(theme.status_busy)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "Setting up environment in background...",
                Style::default().fg(theme.status_bar_fg),
            ),
        ])
    };
    Paragraph::new(footer_text).render(footer_area, buf);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use std::sync::mpsc::channel;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn test_setup_modal_state_lifecycle() {
        let (tx, rx) = channel();
        let mut modal = SetupModalState::new("my-proj", "my-ws", Some("do task".into()), rx);

        assert_eq!(modal.project_name, "my-proj");
        assert_eq!(modal.template_name, "my-ws");
        assert_eq!(modal.current_step, 1);
        assert!(!modal.is_finished);
        assert!(modal.error.is_none());

        // Send progress events
        tx.send(SetupProgressEvent::Step {
            current: 2,
            total: 4,
            message: "Cloning repos...".to_string(),
        })
        .unwrap();
        tx.send(SetupProgressEvent::Log("cloning repo 1".to_string()))
            .unwrap();

        assert!(modal.poll_events().is_none());
        assert_eq!(modal.current_step, 2);
        assert_eq!(modal.status_message, "Cloning repos...");
        assert_eq!(modal.logs.len(), 1);

        // Advance tick
        modal.tick();
        assert_eq!(modal.spinner_frame, 1);

        // Keys while in-progress should not dismiss
        assert!(!modal.handle_key(key(KeyCode::Esc)));
        assert!(!modal.handle_key(key(KeyCode::Enter)));

        // Send Error event
        tx.send(SetupProgressEvent::Error("Network timeout".to_string()))
            .unwrap();
        assert!(modal.poll_events().is_none());
        assert!(modal.is_finished);
        assert_eq!(modal.error.as_deref(), Some("Network timeout"));

        // Keys after error should dismiss
        assert!(modal.handle_key(key(KeyCode::Esc)));
        assert!(modal.handle_key(key(KeyCode::Enter)));
    }

    #[test]
    fn test_render_setup_modal_in_progress_and_error() {
        let (_tx, rx) = channel();
        let mut modal = SetupModalState::new("payment-api", "finance", None, rx);
        modal.logs.push("[1/4] Checking repos...".to_string());
        modal.logs.push("[2/4] Cloning payment-core...".to_string());

        let area = Rect::new(0, 0, 100, 30);
        let mut buf = Buffer::empty(area);
        let theme = Theme::default();

        render_setup_modal(&modal, area, &mut buf, &theme);
        let text = buf
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect::<String>();

        assert!(text.contains("Setting Up Project: payment-api"));
        assert!(text.contains("Setup Logs"));
        assert!(text.contains("payment-core"));
        assert!(text.contains("Setting up environment in background"));

        // Now test error rendering
        modal.error = Some("Branch collision on 'payment-api'".to_string());
        modal.is_finished = true;
        let mut err_buf = Buffer::empty(area);
        render_setup_modal(&modal, area, &mut err_buf, &theme);
        let err_text = err_buf
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect::<String>();

        assert!(err_text.contains("Project Setup Failed: payment-api"));
        assert!(err_text.contains("Branch collision on 'payment-api'"));
        assert!(err_text.contains("[Esc] / [Enter] Close"));
    }

    #[test]
    fn test_setup_modal_mouse_scrolling() {
        use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

        let (_tx, rx) = channel();
        let mut modal = SetupModalState::new("test-project", "template", None, rx);
        for i in 1..=20 {
            modal.logs.push(format!("Log line {i}"));
        }

        assert_eq!(modal.scroll_offset, 0);

        // ScrollUp increases scroll offset by 3
        let scroll_up = MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 10,
            row: 10,
            modifiers: KeyModifiers::empty(),
        };
        assert!(modal.handle_mouse(scroll_up));
        assert_eq!(modal.scroll_offset, 3);

        modal.handle_mouse(scroll_up);
        assert_eq!(modal.scroll_offset, 6);

        // ScrollDown decreases scroll offset by 3
        let scroll_down = MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 10,
            row: 10,
            modifiers: KeyModifiers::empty(),
        };
        assert!(modal.handle_mouse(scroll_down));
        assert_eq!(modal.scroll_offset, 3);

        modal.handle_mouse(scroll_down);
        assert_eq!(modal.scroll_offset, 0);

        // ScrollDown at 0 stays at 0
        modal.handle_mouse(scroll_down);
        assert_eq!(modal.scroll_offset, 0);

        // Non-scroll mouse event returns false
        let click = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 10,
            row: 10,
            modifiers: KeyModifiers::empty(),
        };
        assert!(!modal.handle_mouse(click));

        // Test rendering with scroll offset
        modal.scroll_up(15);
        let area = Rect::new(0, 0, 100, 20);
        let mut buf = Buffer::empty(area);
        let theme = Theme::default();
        render_setup_modal(&modal, area, &mut buf, &theme);
        let text = buf
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect::<String>();
        assert!(text.contains("Log line 1"));
    }
}
