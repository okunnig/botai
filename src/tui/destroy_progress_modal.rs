use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
};

use crate::config::Config;
use crate::project::ProjectDestroyEvent;
use crate::tui::progress_log::{self, ProgressLog, ProgressModalText, WarningHighlight};
use crate::tui::theme::Theme;

/// State for the asynchronous project destruction and progress modal.
pub struct DestroyProgressModalState {
    pub project_name: String,
    pub progress: ProgressLog,
    pub is_success: bool,
    pub receiver: Option<Receiver<ProjectDestroyEvent>>,
}

const TEXT: ProgressModalText = ProgressModalText {
    log_title: " Destruction Logs ",
    warnings: WarningHighlight::Highlighted,
    running_footer: "Destroying project in background...",
    finished_footer: " [Enter] / [Esc] Return to dashboard ",
};

impl DestroyProgressModalState {
    /// Creates a new `DestroyProgressModalState` for monitoring project destruction.
    pub fn new(project_name: impl Into<String>, receiver: Receiver<ProjectDestroyEvent>) -> Self {
        Self {
            project_name: project_name.into(),
            progress: ProgressLog::new("Initializing project destruction..."),
            is_success: false,
            receiver: Some(receiver),
        }
    }

    /// Polls the event receiver and updates logs, step progress, and termination state.
    pub fn poll_events(&mut self) {
        if let Some(ref rx) = self.receiver {
            while let Ok(event) = rx.try_recv() {
                match event {
                    ProjectDestroyEvent::Log(line) => {
                        self.progress.logs.push(line);
                    }
                    ProjectDestroyEvent::Step {
                        current,
                        total,
                        message,
                    } => {
                        self.progress.set_step(current, total, message);
                    }
                    ProjectDestroyEvent::Success => {
                        self.progress.is_finished = true;
                        self.is_success = true;
                        self.progress.status_message =
                            "Project destroyed successfully.".to_string();
                    }
                    ProjectDestroyEvent::Error(err) => {
                        self.progress.is_finished = true;
                        self.is_success = false;
                        self.progress.error = Some(err);
                    }
                }
            }
        }
    }

    /// Handles keyboard events when the destroy progress modal is displayed.
    /// Supports Up/Down/PageUp/PageDown scrolling.
    /// Returns `true` if the modal should be dismissed (on Esc, Enter, 'q', ' ' if finished or error).
    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Up => {
                self.progress.scroll_up(1);
                false
            }
            KeyCode::Down => {
                self.progress.scroll_down(1);
                false
            }
            KeyCode::PageUp => {
                self.progress.scroll_up(10);
                false
            }
            KeyCode::PageDown => {
                self.progress.scroll_down(10);
                false
            }
            KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') | KeyCode::Char(' ') => {
                self.progress.is_finished || self.progress.error.is_some()
            }
            _ => false,
        }
    }
}

/// Background worker function that executes project destruction and streams progress events.
pub fn run_destroy_worker(
    config: Config,
    cwd: PathBuf,
    project_name: String,
    force: bool,
    tx: Sender<ProjectDestroyEvent>,
) {
    let _ = crate::project::destroy_project_workspace_with_progress(
        Some(&config),
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
    let progress = &modal.progress;
    let is_error = progress.error.is_some();
    let is_done = progress.is_finished && !is_error;

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

    let target_line = || {
        progress_log::label_value_line(
            theme,
            "Target: ",
            &modal.project_name,
            Style::default().fg(theme.status_bar_fg),
        )
    };
    let header_lines = if let Some(ref err) = progress.error {
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
            target_line(),
        ]
    } else {
        vec![
            progress_log::step_line(
                theme,
                progress.current_step,
                progress.total_steps,
                &progress.status_message,
            ),
            target_line(),
        ]
    };
    progress.render(&TEXT, title, border_color, header_lines, area, buf, theme);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{
        KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    };
    use std::sync::mpsc::channel;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn test_destroy_progress_modal_state_lifecycle() {
        let (tx, rx) = channel();
        let mut modal = DestroyProgressModalState::new("payment-service", rx);

        assert_eq!(modal.project_name, "payment-service");
        assert_eq!(modal.progress.current_step, 1);
        assert_eq!(modal.progress.total_steps, 4);
        assert!(!modal.progress.is_finished);
        assert!(!modal.is_success);
        assert!(modal.progress.error.is_none());

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
        assert_eq!(modal.progress.current_step, 2);
        assert_eq!(modal.progress.status_message, "Validating worktrees...");
        assert_eq!(modal.progress.logs.len(), 1);

        // Spinner tick
        modal.progress.tick();
        assert_eq!(modal.progress.spinner_frame, 1);

        // In progress: keys do NOT dismiss modal
        assert!(!modal.handle_key(key(KeyCode::Esc)));
        assert!(!modal.handle_key(key(KeyCode::Enter)));
        assert!(!modal.handle_key(key(KeyCode::Char('q'))));
        assert!(!modal.handle_key(key(KeyCode::Char(' '))));

        // Navigation keys scroll
        assert_eq!(modal.progress.scroll_offset, 0);
        modal.handle_key(key(KeyCode::Up));
        assert_eq!(modal.progress.scroll_offset, 1);
        modal.handle_key(key(KeyCode::Down));
        assert_eq!(modal.progress.scroll_offset, 0);
        modal.handle_key(key(KeyCode::PageUp));
        assert_eq!(modal.progress.scroll_offset, 1); // min(logs.len() = 1)
        modal.handle_key(key(KeyCode::PageDown));
        assert_eq!(modal.progress.scroll_offset, 0);

        // Success event
        tx.send(ProjectDestroyEvent::Success).unwrap();
        modal.poll_events();
        assert!(modal.progress.is_finished);
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

        assert!(modal.progress.is_finished);
        assert!(!modal.is_success);
        assert_eq!(
            modal.progress.error.as_deref(),
            Some("Uncommitted changes detected")
        );

        assert!(modal.handle_key(key(KeyCode::Enter)));
        assert!(modal.handle_key(key(KeyCode::Esc)));
    }

    #[test]
    fn test_destroy_progress_modal_mouse_scrolling() {
        let (_tx, rx) = channel();
        let mut modal = DestroyProgressModalState::new("test-project", rx);
        for i in 1..=20 {
            modal.progress.logs.push(format!("Destroy log {i}"));
        }

        assert_eq!(modal.progress.scroll_offset, 0);

        let scroll_up = MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 10,
            row: 10,
            modifiers: KeyModifiers::empty(),
        };
        assert!(modal.progress.handle_mouse(scroll_up));
        assert_eq!(modal.progress.scroll_offset, 3);

        let scroll_down = MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 10,
            row: 10,
            modifiers: KeyModifiers::empty(),
        };
        assert!(modal.progress.handle_mouse(scroll_down));
        assert_eq!(modal.progress.scroll_offset, 0);

        let click = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 10,
            row: 10,
            modifiers: KeyModifiers::empty(),
        };
        assert!(!modal.progress.handle_mouse(click));
    }

    #[test]
    fn test_render_destroy_progress_modal_states() {
        let (_tx, rx) = channel();
        let mut modal = DestroyProgressModalState::new("auth-service", rx);
        modal
            .progress
            .logs
            .push("[1/4] Running pre_destroy hook...".to_string());
        modal
            .progress
            .logs
            .push("✓ Pre-destroy hook completed".to_string());

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
        modal.progress.is_finished = true;
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
        modal.progress.error = Some("Failed to delete directory".to_string());
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
