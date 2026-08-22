use std::collections::HashMap;
use std::fs;
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
use crate::context::{generate_agents_md, ProjectContext, RepoMapping};
use crate::git::{create_worktree, remove_worktree_auto};
use crate::session::{ProjectSession, PtyConfig, PtySession, VtScreen};
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
        self.scroll_offset = self.scroll_offset.saturating_add(lines).min(self.logs.len());
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
    let result = (|| -> anyhow::Result<(ProjectSession, PtySession, VtScreen)> {
        // Step 1: Checking / resolving template repositories
        let _ = tx.send(SetupProgressEvent::Step {
            current: 1,
            total: 4,
            message: format!("Checking repositories for template '{}'...", template_name),
        });
        let _ = tx.send(SetupProgressEvent::Log(format!(
            "[1/4] Checking repositories for template '{}'...",
            template_name
        )));

        let project_dir = cwd.join(&project_name);
        if project_dir.exists() {
            anyhow::bail!("Project directory '{}' already exists in CWD", project_name);
        }

        let default_config =
            Config::load_from_str("default_agent = \"claude\"\nrepos_dir = \".botai_repos\"\n").ok();
        let config_ref = config.as_ref().or(default_config.as_ref());

        let tpl = config_ref.and_then(|c| c.get_template(&template_name));
        let agent_name = tpl
            .and_then(|w| w.default_agent.as_deref())
            .or_else(|| config_ref.map(|c| c.default_agent.as_str()))
            .unwrap_or("claude");

        let (agent_cmd, agent_args, agent_env) = if let Some(cfg) = config_ref {
            if let Some(agent) = cfg.get_agent(agent_name) {
                (agent.command.clone(), agent.args.clone(), agent.env.clone())
            } else {
                #[cfg(windows)]
                let default_shell = "cmd.exe".to_string();
                #[cfg(not(windows))]
                let default_shell = "sh".to_string();
                (default_shell, Vec::new(), HashMap::new())
            }
        } else {
            #[cfg(windows)]
            let default_shell = "cmd.exe".to_string();
            #[cfg(not(windows))]
            let default_shell = "sh".to_string();
            (default_shell, Vec::new(), HashMap::new())
        };

        // Step 2: Auto-cloning missing repos & fetching
        let _ = tx.send(SetupProgressEvent::Step {
            current: 2,
            total: 4,
            message: "Cloning and updating repositories...".to_string(),
        });

        let mut resolved_repos_list = Vec::new();
        if let Some(cfg) = config_ref {
            match cfg.resolve_template_repos(&template_name) {
                Ok(repos) => {
                    resolved_repos_list = repos;
                }
                Err(err) => {
                    anyhow::bail!("Failed to resolve template repositories: {}", err);
                }
            }
        }

        for repo in &resolved_repos_list {
            if !repo.exists_locally() {
                if let Some(url) = &repo.clone_url {
                    let _ = tx.send(SetupProgressEvent::Log(format!(
                        "[2/4] Cloning repository '{}' from {}...",
                        repo.name, url
                    )));
                    crate::git::clone_repo(url, &repo.local_path)?;
                } else {
                    anyhow::bail!(
                        "Repository '{}' does not exist locally and has no remote URL configured",
                        repo.name
                    );
                }
            } else {
                let _ = tx.send(SetupProgressEvent::Log(format!(
                    "[2/4] Fetching latest updates for repository '{}'...",
                    repo.name
                )));
                let _ = crate::git::fetch_repo(&repo.local_path);
            }
        }

        // Step 3: Creating project directory and git worktrees
        let _ = tx.send(SetupProgressEvent::Step {
            current: 3,
            total: 4,
            message: "Creating project directory and git worktrees...".to_string(),
        });
        fs::create_dir_all(&project_dir)?;
        let project_dir_canon = project_dir
            .canonicalize()
            .unwrap_or_else(|_| project_dir.clone());

        let mut repo_mappings: Vec<RepoMapping> = Vec::new();
        for repo in &resolved_repos_list {
            let worktree_path = project_dir_canon.join(&repo.name);
            let _ = tx.send(SetupProgressEvent::Log(format!(
                "[3/4] Creating git worktree for '{}' at {}...",
                repo.name,
                worktree_path.display()
            )));
            if let Err(err) = create_worktree(&repo.local_path, &worktree_path, &project_name) {
                // Rollback partially created worktrees & project_dir
                for mapping in &repo_mappings {
                    let _ = remove_worktree_auto(&mapping.worktree_path, true);
                }
                let _ = fs::remove_dir_all(&project_dir);
                anyhow::bail!("Failed to create worktree for '{}': {}", repo.name, err);
            }
            repo_mappings.push(RepoMapping::new(
                repo.name.clone(),
                repo.local_path.clone(),
                worktree_path,
            ));
        }

        // Step 4: Generating AGENTS.md context
        let _ = tx.send(SetupProgressEvent::Step {
            current: 4,
            total: 4,
            message: "Generating AGENTS.md context orientation file...".to_string(),
        });
        let _ = tx.send(SetupProgressEvent::Log(
            "[4/4] Generating AGENTS.md context orientation file...".to_string(),
        ));

        let project_context = ProjectContext {
            project_name: project_name.clone(),
            template_name: template_name.clone(),
            description: tpl.and_then(|w| w.description.clone()),
            agents_md_append: tpl.and_then(|w| w.agents_md_append.clone()),
            repos: repo_mappings,
            env: tpl.map(|w| w.env.clone()).unwrap_or_default(),
            initial_prompt: prompt.clone(),
        };

        if let Err(err) = generate_agents_md(&project_dir_canon, &project_context) {
            let _ = fs::remove_dir_all(&project_dir);
            anyhow::bail!("Failed to generate AGENTS.md: {}", err);
        }

        // Step 5: Spawning agent CLI in pseudo-terminal
        let (cols, rows) = viewport_size;
        let _ = tx.send(SetupProgressEvent::Log(format!(
            "Spawning AI agent CLI '{}' in pseudo-terminal ({}x{})...",
            agent_cmd, cols, rows
        )));

        let mut pty_config = PtyConfig::new(&agent_cmd, &project_dir_canon)
            .with_args(agent_args)
            .with_dimensions(cols, rows);

        for (k, v) in &agent_env {
            pty_config = pty_config.with_env(k, v);
        }

        if let Some(tpl) = tpl {
            for (k, v) in &tpl.env {
                pty_config = pty_config.with_env(k, v);
            }
        }

        let pty = match PtySession::spawn(pty_config) {
            Ok(p) => p,
            Err(err) => {
                let _ = fs::remove_dir_all(&project_dir);
                anyhow::bail!("Failed to spawn agent process '{}': {}", agent_cmd, err);
            }
        };

        let vt = VtScreen::new(cols, rows);
        let mut ps = ProjectSession::new(&project_name, &template_name, agent_name);
        if let Some(ref p) = prompt {
            ps = ps.with_prompt(p);
        }

        Ok((ps, pty, vt))
    })();

    match result {
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
            let _ = tx.send(SetupProgressEvent::Log(format!("✗ Error: {}", err_msg)));
            let _ = tx.send(SetupProgressEvent::Error(err_msg));
        }
    }
}

/// Renders the live setup progress and log modal into the given `Buffer`.
pub fn render_setup_modal(
    modal: &SetupModalState,
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
                format!("Reason: {}", err),
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
    let effective_offset = modal.scroll_offset.min(total_logs.saturating_sub(max_lines));
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
            let style = if line.starts_with('✗')
                || line.contains("Error")
                || line.contains("Failed")
            {
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
                format!(" {} ", spinner),
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
            .map(|c| c.symbol())
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
            .map(|c| c.symbol())
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
            modal.logs.push(format!("Log line {}", i));
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
            .map(|c| c.symbol())
            .collect::<String>();
        assert!(text.contains("Log line 1"));
    }
}
