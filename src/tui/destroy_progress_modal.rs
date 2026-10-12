use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};

use crossterm::event::{KeyCode, KeyEvent, MouseEvent};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    widgets::{Paragraph, Widget},
};

use crate::config::Config;
use crate::project::ProjectDestroyEvent;
use crate::tui::progress_log;
use crate::tui::theme::Theme;

/// State for the asynchronous project destruction and progress modal.
pub struct DestroyProgressModalState {
    pub project_name: String,
    pub logs: Vec<String>,
    pub current_step: usize,
    pub total_steps: usize,
    pub status_message: String,
    pub is_finished: bool,
    pub is_success: bool,
    pub error: Option<String>,
    pub receiver: Option<Receiver<ProjectDestroyEvent>>,
    pub spinner_frame: usize,
    pub scroll_offset: usize,
}

impl DestroyProgressModalState {
    /// Creates a new `DestroyProgressModalState` for monitoring project destruction.
    pub fn new(project_name: impl Into<String>, receiver: Receiver<ProjectDestroyEvent>) -> Self {
        Self {
            project_name: project_name.into(),
            logs: Vec::new(),
            current_step: 1,
            total_steps: 4,
            status_message: "Initializing project destruction...".to_string(),
            is_finished: false,
            is_success: false,
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
    pub fn poll_events(&mut self) {
        if let Some(ref rx) = self.receiver {
            while let Ok(event) = rx.try_recv() {
                match event {
                    ProjectDestroyEvent::Log(line) => {
                        self.logs.push(line);
                    }
                    ProjectDestroyEvent::Step {
                        current,
                        total,
                        message,
                    } => {
                        self.current_step = current;
                        self.total_steps = total;
                        self.status_message = message;
                    }
                    ProjectDestroyEvent::Success => {
                        self.is_finished = true;
                        self.is_success = true;
                        self.status_message = "Project destroyed successfully.".to_string();
                    }
                    ProjectDestroyEvent::Error(err) => {
                        self.is_finished = true;
                        self.is_success = false;
                        self.error = Some(err);
                    }
                }
            }
        }
    }

    /// Scrolls the log viewport upwards (into older log lines).
    pub fn scroll_up(&mut self, lines: usize) {
        progress_log::scroll_up(&mut self.scroll_offset, lines, self.logs.len());
    }

    /// Scrolls the log viewport downwards (towards latest log lines).
    pub fn scroll_down(&mut self, lines: usize) {
        progress_log::scroll_down(&mut self.scroll_offset, lines);
    }

    /// Handles mouse events (e.g. scroll wheel) on the destroy progress modal.
    pub fn handle_mouse(&mut self, mouse: MouseEvent) -> bool {
        progress_log::handle_mouse_scroll(&mut self.scroll_offset, self.logs.len(), mouse)
    }

    /// Handles keyboard events when the destroy progress modal is displayed.
    /// Supports Up/Down/PageUp/PageDown scrolling.
    /// Returns `true` if the modal should be dismissed (on Esc, Enter, 'q', ' ' if finished or error).
    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Up => {
                self.scroll_up(1);
                false
            }
            KeyCode::Down => {
                self.scroll_down(1);
                false
            }
            KeyCode::PageUp => {
                self.scroll_up(10);
                false
            }
            KeyCode::PageDown => {
                self.scroll_down(10);
                false
            }
            KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') | KeyCode::Char(' ') => {
                self.is_finished || self.error.is_some()
            }
            _ => false,
        }
    }
}

/// Background worker function that executes project destruction and streams progress events.
pub fn run_destroy_worker(
    config: Option<Config>,
    cwd: PathBuf,
    project_name: String,
    force: bool,
    tx: Sender<ProjectDestroyEvent>,
) {
    let _ = crate::project::destroy_project_workspace_with_progress(
        config.as_ref(),
        &cwd,
        &project_name,
        force,
        move |ev| {
            let _ = tx.send(ev);
        },
    );
}

/// Renders the live destruction progress and log modal into the given `Buffer`.
pub fn render_destroy_progress_modal(
    modal: &DestroyProgressModalState,
    area: Rect,
    buf: &mut Buffer,
    theme: &Theme,
) {
    let is_error = modal.error.is_some();
    let is_done = modal.is_finished && !is_error;

    let title = if is_error {
        format!(" Project Destruction Failed: {} ", modal.project_name)
    } else if is_done {
        format!(" Project Destroyed: {} ", modal.project_name)
    } else {
        format!(" Destroying Project: {} ", modal.project_name)
    };

    let border_color = if is_error {
        theme.status_error
    } else if is_done {
        theme.status_done
    } else {
        theme.status_busy
    };

    let Some(areas) = progress_log::render_modal_frame(area, buf, title, border_color) else {
        return;
    };

    let target_line = progress_log::label_value_line(
        theme,
        "Target: ",
        &modal.project_name,
        Style::default().fg(theme.status_bar_fg),
    );
    let header_lines = if let Some(ref err) = modal.error {
        progress_log::failure_header_lines(theme, "Destruction Failed", err)
    } else if is_done {
        vec![
            progress_log::label_value_line(
                theme,
                "Status: ",
                "Destroyed Successfully",
                Style::default()
                    .fg(theme.status_done)
                    .add_modifier(Modifier::BOLD),
            ),
            target_line,
        ]
    } else {
        vec![
            progress_log::step_line(
                theme,
                modal.current_step,
                modal.total_steps,
                &modal.status_message,
            ),
            target_line,
        ]
    };
    Paragraph::new(header_lines).render(areas.header, buf);

    progress_log::render_log_box(
        &modal.logs,
        modal.scroll_offset,
        " Destruction Logs ",
        true,
        areas.log,
        buf,
        theme,
    );

    let footer_text = if is_error || modal.is_finished {
        progress_log::finished_footer(" [Enter] / [Esc] Return to dashboard ", theme)
    } else {
        progress_log::spinner_footer(
            modal.spinner_frame,
            "Destroying project in background...",
            theme,
        )
    };
    Paragraph::new(footer_text).render(areas.footer, buf);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEventKind};
    use std::sync::mpsc::channel;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn test_destroy_progress_modal_state_lifecycle() {
        let (tx, rx) = channel();
        let mut modal = DestroyProgressModalState::new("payment-service", rx);

        assert_eq!(modal.project_name, "payment-service");
        assert_eq!(modal.current_step, 1);
        assert_eq!(modal.total_steps, 4);
        assert!(!modal.is_finished);
        assert!(!modal.is_success);
        assert!(modal.error.is_none());

        // Step updates and logs
        tx.send(ProjectDestroyEvent::Step {
            current: 2,
            total: 4,
            message: "Validating worktrees...".to_string(),
        })
        .unwrap();
        tx.send(ProjectDestroyEvent::Log(
            "Checking uncommitted changes".to_string(),
        ))
        .unwrap();

        modal.poll_events();
        assert_eq!(modal.current_step, 2);
        assert_eq!(modal.status_message, "Validating worktrees...");
        assert_eq!(modal.logs.len(), 1);

        // Spinner tick
        modal.tick();
        assert_eq!(modal.spinner_frame, 1);

        // In progress: keys do NOT dismiss modal
        assert!(!modal.handle_key(key(KeyCode::Esc)));
        assert!(!modal.handle_key(key(KeyCode::Enter)));
        assert!(!modal.handle_key(key(KeyCode::Char('q'))));
        assert!(!modal.handle_key(key(KeyCode::Char(' '))));

        // Navigation keys scroll
        assert_eq!(modal.scroll_offset, 0);
        modal.handle_key(key(KeyCode::Up));
        assert_eq!(modal.scroll_offset, 1);
        modal.handle_key(key(KeyCode::Down));
        assert_eq!(modal.scroll_offset, 0);
        modal.handle_key(key(KeyCode::PageUp));
        assert_eq!(modal.scroll_offset, 1); // min(logs.len() = 1)
        modal.handle_key(key(KeyCode::PageDown));
        assert_eq!(modal.scroll_offset, 0);

        // Success event
        tx.send(ProjectDestroyEvent::Success).unwrap();
        modal.poll_events();
        assert!(modal.is_finished);
        assert!(modal.is_success);

        // Once finished: Enter / Esc / q / Space dismiss
        assert!(modal.handle_key(key(KeyCode::Enter)));
        assert!(modal.handle_key(key(KeyCode::Esc)));
        assert!(modal.handle_key(key(KeyCode::Char('q'))));
        assert!(modal.handle_key(key(KeyCode::Char(' '))));
        assert!(!modal.handle_key(key(KeyCode::Char('a'))));
    }

    #[test]
    fn test_destroy_progress_modal_error_lifecycle() {
        let (tx, rx) = channel();
        let mut modal = DestroyProgressModalState::new("broken-service", rx);

        tx.send(ProjectDestroyEvent::Error(
            "Uncommitted changes detected".to_string(),
        ))
        .unwrap();
        modal.poll_events();

        assert!(modal.is_finished);
        assert!(!modal.is_success);
        assert_eq!(modal.error.as_deref(), Some("Uncommitted changes detected"));

        assert!(modal.handle_key(key(KeyCode::Enter)));
        assert!(modal.handle_key(key(KeyCode::Esc)));
    }

    #[test]
    fn test_destroy_progress_modal_mouse_scrolling() {
        let (_tx, rx) = channel();
        let mut modal = DestroyProgressModalState::new("test-project", rx);
        for i in 1..=20 {
            modal.logs.push(format!("Destroy log {i}"));
        }

        assert_eq!(modal.scroll_offset, 0);

        let scroll_up = MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 10,
            row: 10,
            modifiers: KeyModifiers::empty(),
        };
        assert!(modal.handle_mouse(scroll_up));
        assert_eq!(modal.scroll_offset, 3);

        let scroll_down = MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 10,
            row: 10,
            modifiers: KeyModifiers::empty(),
        };
        assert!(modal.handle_mouse(scroll_down));
        assert_eq!(modal.scroll_offset, 0);

        let click = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 10,
            row: 10,
            modifiers: KeyModifiers::empty(),
        };
        assert!(!modal.handle_mouse(click));
    }

    #[test]
    fn test_render_destroy_progress_modal_states() {
        let (_tx, rx) = channel();
        let mut modal = DestroyProgressModalState::new("auth-service", rx);
        modal
            .logs
            .push("[1/4] Running pre_destroy hook...".to_string());
        modal.logs.push("✓ Pre-destroy hook completed".to_string());

        let area = Rect::new(0, 0, 100, 30);
        let mut buf = Buffer::empty(area);
        let theme = Theme::default();

        // 1. Render in progress
        render_destroy_progress_modal(&modal, area, &mut buf, &theme);
        let text = buf
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect::<String>();
        assert!(text.contains("Destroying Project: auth-service"));
        assert!(text.contains("Destruction Logs"));
        assert!(text.contains("pre_destroy hook"));
        assert!(text.contains("Destroying project in background"));

        // 2. Render success
        modal.is_finished = true;
        modal.is_success = true;
        let mut succ_buf = Buffer::empty(area);
        render_destroy_progress_modal(&modal, area, &mut succ_buf, &theme);
        let succ_text = succ_buf
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect::<String>();
        assert!(succ_text.contains("Project Destroyed: auth-service"));
        assert!(succ_text.contains("Destroyed Successfully"));
        assert!(succ_text.contains("[Enter] / [Esc] Return to dashboard"));

        // 3. Render error
        modal.error = Some("Failed to delete directory".to_string());
        let mut err_buf = Buffer::empty(area);
        render_destroy_progress_modal(&modal, area, &mut err_buf, &theme);
        let err_text = err_buf
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect::<String>();
        assert!(err_text.contains("Project Destruction Failed: auth-service"));
        assert!(err_text.contains("Reason: Failed to delete directory"));
        assert!(err_text.contains("[Enter] / [Esc] Return to dashboard"));
    }
}
