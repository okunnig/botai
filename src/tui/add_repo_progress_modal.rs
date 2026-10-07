use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};

use crossterm::event::{KeyCode, KeyEvent, MouseEvent, MouseEventKind};
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Widget},
};

use crate::config::Config;
use crate::project::ProjectAddRepoEvent;
use crate::tui::theme::Theme;

/// Animated braille spinner frames.
const SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// State for the asynchronous repository addition progress modal.
pub struct AddRepoProgressModalState {
    pub project_name: String,
    pub target_repos: Vec<String>,
    pub added_repos: Vec<String>,
    pub logs: Vec<String>,
    pub current_step: usize,
    pub total_steps: usize,
    pub status_message: String,
    pub is_finished: bool,
    pub is_success: bool,
    pub error: Option<String>,
    pub receiver: Option<Receiver<ProjectAddRepoEvent>>,
    pub spinner_frame: usize,
    pub scroll_offset: usize,
}

impl AddRepoProgressModalState {
    /// Creates a new `AddRepoProgressModalState` for monitoring repository addition.
    pub fn new(
        project_name: impl Into<String>,
        target_repos: Vec<String>,
        receiver: Receiver<ProjectAddRepoEvent>,
    ) -> Self {
        Self {
            project_name: project_name.into(),
            target_repos,
            added_repos: Vec::new(),
            logs: Vec::new(),
            current_step: 1,
            total_steps: 4,
            status_message: "Initializing repository addition...".to_string(),
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
        let mut disconnected = false;
        if let Some(ref rx) = self.receiver {
            loop {
                match rx.try_recv() {
                    Ok(event) => match event {
                        ProjectAddRepoEvent::Log(line) => {
                            if let Some(rest) = line.strip_prefix("✓ Created git worktree for '")
                            {
                                if let Some(repo) = rest.strip_suffix('\'') {
                                    if !self.added_repos.contains(&repo.to_string()) {
                                        self.added_repos.push(repo.to_string());
                                    }
                                }
                            }
                            self.logs.push(line);
                        }
                        ProjectAddRepoEvent::Step {
                            current,
                            total,
                            message,
                        } => {
                            self.current_step = current;
                            self.total_steps = total;
                            self.status_message = message;
                        }
                        ProjectAddRepoEvent::Success => {
                            self.is_finished = true;
                            self.is_success = true;
                            self.status_message = "Repository addition completed.".to_string();
                        }
                        ProjectAddRepoEvent::Error(err) => {
                            self.logs.push(format!("✗ Error: {err}"));
                            if !self.is_finished {
                                self.error = Some(err);
                            }
                        }
                    },
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        disconnected = true;
                        break;
                    }
                }
            }
        }

        if disconnected && !self.is_finished {
            self.is_finished = true;
            if self.error.is_some() && !self.is_success {
                self.status_message = "Failed to add repositories.".to_string();
            }
        }
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

    /// Handles mouse events (e.g. scroll wheel) on the progress modal.
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

    /// Handles keyboard events when the progress modal is displayed.
    /// Supports Up/Down/PageUp/PageDown scrolling.
    /// Returns `true` if the modal should be dismissed.
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

/// Background worker function that drives `add_worktrees_to_project` and streams progress events.
pub fn run_add_repo_worker(
    config: Config,
    project_dir: PathBuf,
    repos: Vec<String>,
    tx: Sender<ProjectAddRepoEvent>,
) {
    let _ = crate::project::add_worktrees_to_project(&config, &project_dir, &repos, move |ev| {
        let _ = tx.send(ev);
    });
}

/// Renders the Add Repository progress modal into the given `Buffer`.
pub fn render_add_repo_progress_modal(
    modal: &AddRepoProgressModalState,
    area: Rect,
    buf: &mut Buffer,
    theme: &Theme,
) {
    let width = 76.min(area.width.saturating_sub(4));
    let height = 18.min(area.height.saturating_sub(2));

    if width < 24 || height < 8 {
        return;
    }

    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let modal_area = Rect::new(x, y, width, height);

    Clear.render(modal_area, buf);

    let is_error = modal.error.is_some() && !modal.is_success;
    let is_done = modal.is_finished && modal.is_success;

    let title = if is_error {
        format!(" Adding Repositories Failed: {} ", modal.project_name)
    } else if is_done {
        format!(" Added Repositories: {} ", modal.project_name)
    } else {
        format!(" Adding Repositories: {} ", modal.project_name)
    };

    let border_color = if is_error {
        theme.status_error
    } else if is_done {
        theme.status_done
    } else {
        theme.status_busy
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .title(Span::styled(
            title,
            Style::default()
                .fg(theme.modal_title)
                .add_modifier(Modifier::BOLD),
        ));

    let inner = block.inner(modal_area);
    block.render(modal_area, buf);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // Step progress & spinner
            Constraint::Length(1), // Divider
            Constraint::Min(3),    // Logs viewport
            Constraint::Length(1), // Divider
            Constraint::Length(1), // Footer hint
        ])
        .split(inner);

    // Step progress & spinner
    let spinner_char = if modal.is_finished {
        if is_error {
            "✗"
        } else {
            "✓"
        }
    } else {
        let idx = modal.spinner_frame % SPINNER_FRAMES.len();
        SPINNER_FRAMES[idx]
    };

    let spinner_style = if is_error {
        Style::default()
            .fg(theme.status_error)
            .add_modifier(Modifier::BOLD)
    } else if is_done {
        Style::default()
            .fg(theme.status_done)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(theme.status_busy)
            .add_modifier(Modifier::BOLD)
    };

    let header_line = Line::from(vec![
        Span::styled(format!("{spinner_char} "), spinner_style),
        Span::styled(
            format!("Step [{}/{}]: ", modal.current_step, modal.total_steps),
            Style::default()
                .fg(theme.modal_label)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            &modal.status_message,
            Style::default().fg(theme.status_bar_fg),
        ),
    ]);
    Paragraph::new(header_line).render(chunks[0], buf);

    // Top divider
    let divider = Line::from(Span::styled(
        "─".repeat(chunks[1].width as usize),
        Style::default().fg(theme.border_unfocused),
    ));
    Paragraph::new(divider).render(chunks[1], buf);

    // Logs block
    let logs_height = chunks[2].height as usize;
    let total_logs = modal.logs.len();
    let effective_scroll = modal
        .scroll_offset
        .min(total_logs.saturating_sub(logs_height));
    let skip_count = total_logs.saturating_sub(logs_height + effective_scroll);

    let visible_logs: Vec<Line> = modal
        .logs
        .iter()
        .skip(skip_count)
        .take(logs_height)
        .map(|log| {
            if log.starts_with('✓') {
                Line::from(Span::styled(log, Style::default().fg(theme.status_done)))
            } else if log.starts_with('⚠') {
                Line::from(Span::styled(log, Style::default().fg(theme.status_waiting)))
            } else if log.starts_with('✗') {
                Line::from(Span::styled(log, Style::default().fg(theme.status_error)))
            } else {
                Line::from(Span::styled(log, Style::default().fg(theme.status_bar_fg)))
            }
        })
        .collect();

    let logs_block = Block::default().borders(Borders::NONE).title(Span::styled(
        " Logs ",
        Style::default().fg(theme.border_unfocused),
    ));
    Paragraph::new(visible_logs)
        .block(logs_block)
        .render(chunks[2], buf);

    // Bottom divider
    let bottom_divider = Line::from(Span::styled(
        "─".repeat(chunks[3].width as usize),
        Style::default().fg(theme.border_unfocused),
    ));
    Paragraph::new(bottom_divider).render(chunks[3], buf);

    // Footer
    let footer_text = if modal.is_finished {
        "[Enter] / [Esc] Return to project"
    } else {
        "Adding repositories in background... [↑/↓] Scroll logs"
    };

    let footer_line = Line::from(Span::styled(
        footer_text,
        Style::default()
            .fg(theme.key_hint)
            .add_modifier(Modifier::BOLD),
    ));
    Paragraph::new(footer_line).render(chunks[4], buf);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, crossterm::event::KeyModifiers::empty())
    }

    #[test]
    fn test_add_repo_progress_modal_lifecycle() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut modal = AddRepoProgressModalState::new("proj-x", vec!["repo1".to_string()], rx);

        assert!(!modal.is_finished);
        assert!(!modal.handle_key(key(KeyCode::Enter)));

        tx.send(ProjectAddRepoEvent::Step {
            current: 2,
            total: 4,
            message: "Creating worktrees...".to_string(),
        })
        .unwrap();
        tx.send(ProjectAddRepoEvent::Log(
            "✓ Created git worktree for 'repo1'".to_string(),
        ))
        .unwrap();
        tx.send(ProjectAddRepoEvent::Success).unwrap();

        modal.poll_events();
        assert!(modal.is_finished);
        assert!(modal.is_success);
        assert_eq!(modal.added_repos, vec!["repo1"]);
        assert_eq!(modal.current_step, 2);

        // Can dismiss with Enter or Esc
        assert!(modal.handle_key(key(KeyCode::Enter)));
        assert!(modal.handle_key(key(KeyCode::Esc)));
    }

    #[test]
    fn test_add_repo_progress_modal_render() {
        let (_tx, rx) = std::sync::mpsc::channel();
        let mut modal = AddRepoProgressModalState::new("proj-x", vec!["repo1".to_string()], rx);
        modal.logs.push("[1/4] Cloning...".to_string());
        modal
            .logs
            .push("✓ Created git worktree for 'repo1'".to_string());
        modal.is_finished = true;
        modal.is_success = true;

        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        let theme = Theme::default();

        render_add_repo_progress_modal(&modal, area, &mut buf, &theme);

        let content = buf
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect::<String>();

        assert!(content.contains("Added Repositories: proj-x"));
        assert!(content.contains("[Enter] / [Esc] Return to project"));
    }
}
