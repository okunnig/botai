use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::{self, Stdout};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crossterm::{
    event::{self, Event, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};

use crate::config::Config;
use crate::context::{generate_agents_md, ProjectContext, RepoMapping};
use crate::git::{create_worktree, remove_worktree};
use crate::notify::{ActivityTracker, NotificationService};
use crate::session::{ProjectSession, PtyConfig, PtySession, SessionRegistry, VtScreen};
use crate::tui::dashboard::{render_dashboard, DashboardAction, DashboardState};
use crate::tui::events::{map_key_event, TuiAction};
use crate::tui::status_bar::StatusBar;
use crate::tui::ui::{compute_layout, TerminalView};

/// Main application runner managing the TUI loop, PTY streaming, activity tracking, and rendering.
pub struct TuiApp {
    pub registry: SessionRegistry,
    pub tracker: ActivityTracker,
    pub notifier: NotificationService,
    pub should_quit: bool,
    pub tick_rate: Duration,
    pub dashboard: Option<DashboardState>,
    pub config: Option<Config>,
    pub cwd: Option<PathBuf>,
}

struct TerminalCleanupGuard;

impl Drop for TerminalCleanupGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen, crossterm::cursor::Show);
    }
}

impl TuiApp {
    /// Creates a new `TuiApp` with the given registry.
    pub fn new(registry: SessionRegistry) -> Self {
        Self {
            registry,
            tracker: ActivityTracker::new(Duration::from_millis(1500)),
            notifier: NotificationService::new(true, Duration::from_secs(10)),
            should_quit: false,
            tick_rate: Duration::from_millis(30),
            dashboard: None,
            config: None,
            cwd: None,
        }
    }

    /// Sets the dashboard state on `TuiApp`.
    pub fn with_dashboard(mut self, dashboard: DashboardState) -> Self {
        self.dashboard = Some(dashboard);
        self
    }

    /// Sets the configuration on `TuiApp`.
    pub fn with_config(mut self, config: Config) -> Self {
        self.config = Some(config);
        self
    }

    /// Sets the working directory on `TuiApp`.
    pub fn with_cwd(mut self, cwd: PathBuf) -> Self {
        self.cwd = Some(cwd);
        self
    }

    /// Runs the interactive TUI application event loop.
    pub fn run(&mut self, stdout: &mut Stdout) -> io::Result<()> {
        enable_raw_mode()?;
        execute!(stdout, EnterAlternateScreen)?;
        let _guard = TerminalCleanupGuard;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        let res = self.run_loop(&mut terminal);

        // Always restore terminal state even on error
        disable_raw_mode()?;
        execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
        terminal.show_cursor()?;

        res
    }

    /// Internal event loop.
    pub fn run_loop<B: ratatui::backend::Backend>(
        &mut self,
        terminal: &mut Terminal<B>,
    ) -> io::Result<()> {
        let mut last_tick = Instant::now();

        while !self.should_quit {
            // 1. Process PTY outputs for all sessions
            self.process_pty_streams();

            // 2. Draw active frame
            terminal.draw(|f| {
                if self.registry.is_empty() && self.dashboard.is_some() {
                    if let Some(dashboard) = &self.dashboard {
                        render_dashboard(dashboard, f.area(), f.buffer_mut());
                    }
                } else {
                    let (status_area, term_area) = compute_layout(f.area());
                    f.render_widget(StatusBar::new(&self.registry), status_area);

                    if let Some(active) = self.registry.active_session() {
                        f.render_widget(TerminalView::new(active.vt()), term_area);
                    }
                }
            })?;

            // 3. Handle user input with timeout up to tick_rate
            let timeout = self.tick_rate.saturating_sub(last_tick.elapsed());

            if event::poll(timeout)? {
                if let Event::Key(key) = event::read()? {
                    if key.kind == KeyEventKind::Press {
                        if self.registry.is_empty() && self.dashboard.is_some() {
                            self.handle_dashboard_key(key);
                        } else {
                            self.handle_key_event(key);
                        }
                    }
                } else if let Event::Resize(cols, rows) = event::read()? {
                    let (_, term_area) =
                        compute_layout(ratatui::layout::Rect::new(0, 0, cols, rows));
                    if let Some(active) = self.registry.active_session_mut() {
                        let _ = active.pty_mut().resize(term_area.width, term_area.height);
                        active.vt_mut().resize(term_area.width, term_area.height);
                    }
                }
            }

            if last_tick.elapsed() >= self.tick_rate {
                last_tick = Instant::now();
            }
        }

        Ok(())
    }

    /// Reads output bytes from all running PTY sessions into their respective VT screens.
    pub fn process_pty_streams(&mut self) {
        let now = Instant::now();
        let sessions = self.registry.list_mut();

        for session in sessions {
            if let Ok(bytes) = session.pty_mut().read_nonblocking() {
                if !bytes.is_empty() {
                    session.vt_mut().process(&bytes);
                    let new_status = self.tracker.record_activity();
                    session.project_session.set_status(new_status);
                }
            }

            let is_alive = session.pty_mut().is_alive();
            let exit_code = session
                .pty_mut()
                .try_wait()
                .ok()
                .flatten()
                .and_then(|s| s.code.map(|c| c as i32));

            let (status, notif, _) = self.tracker.evaluate(is_alive, exit_code, now);
            session.project_session.set_status(status);

            if let Some(mut event) = notif {
                match &mut event {
                    crate::notify::NotificationEvent::Completed { project_name, .. }
                    | crate::notify::NotificationEvent::WaitingForInput { project_name }
                    | crate::notify::NotificationEvent::Error { project_name, .. } => {
                        *project_name = session.project_name().to_string();
                    }
                }
                let _ = self.notifier.notify(&event);
            }
        }
    }

    /// Dispatches a key event to the dashboard and processes the resulting `DashboardAction`.
    pub fn handle_dashboard_key(&mut self, key: crossterm::event::KeyEvent) {
        if let Some(mut dashboard) = self.dashboard.take() {
            let action = dashboard.handle_key(key);
            self.dashboard = Some(dashboard);
            self.handle_dashboard_action(action);
        }
    }

    /// Processes a `DashboardAction` emitted by the dashboard.
    pub fn handle_dashboard_action(&mut self, action: DashboardAction) {
        match action {
            DashboardAction::Quit => {
                self.should_quit = true;
            }
            DashboardAction::Attach(project) => {
                let _ = self.attach_project(&project);
            }
            DashboardAction::CreateProject {
                workspace,
                project,
                prompt,
            } => {
                let _ = self.create_and_start_project(&workspace, &project, prompt.as_deref());
            }
            DashboardAction::Destroy(project) => {
                let _ = self.destroy_project(&project);
            }
            DashboardAction::None => {}
        }
    }

    /// Attaches to an existing project directory and adds its session to the registry.
    pub fn attach_project(&mut self, project_name: &str) -> anyhow::Result<()> {
        let cwd = self
            .cwd
            .clone()
            .or_else(|| env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."));
        let project_dir = cwd.join(project_name);

        if !project_dir.exists() {
            return Ok(());
        }

        let default_config = Config::load_from_str("default_agent = \"claude\"\n").ok();
        let config = self.config.as_ref().or(default_config.as_ref());

        let agent_name = config.map(|c| c.default_agent.as_str()).unwrap_or("claude");

        let (cmd, args, env_map) = if let Some(cfg) = config {
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

        let mut pty_config = PtyConfig::new(&cmd, &project_dir)
            .with_args(args)
            .with_dimensions(100, 30);

        for (k, v) in &env_map {
            pty_config = pty_config.with_env(k, v);
        }

        let pty = PtySession::spawn(pty_config)?;
        let vt = VtScreen::new(100, 30);
        let ps = ProjectSession::new(project_name, "attached", agent_name);

        self.registry.add(ps, pty, vt)?;
        Ok(())
    }

    /// Creates worktrees, synthesizes `AGENTS.md`, launches the agent PTY session, and registers it.
    pub fn create_and_start_project(
        &mut self,
        workspace_name: &str,
        project_name: &str,
        prompt: Option<&str>,
    ) -> anyhow::Result<()> {
        let cwd = self
            .cwd
            .clone()
            .or_else(|| env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."));
        let project_dir = cwd.join(project_name);

        let default_config = Config::load_from_str("default_agent = \"claude\"\n").ok();
        let config = self.config.as_ref().or(default_config.as_ref());

        let ws = config.and_then(|c| c.get_workspace(workspace_name));
        let agent_name = ws
            .and_then(|w| w.default_agent.as_deref())
            .or_else(|| config.map(|c| c.default_agent.as_str()))
            .unwrap_or("claude");

        let (agent_cmd, agent_args, agent_env) = if let Some(cfg) = config {
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

        fs::create_dir_all(&project_dir)?;

        let mut repo_mappings = Vec::new();
        if let Some(ws) = ws {
            for repo_path in &ws.repos {
                let repo_name = repo_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("repo");
                let worktree_path = project_dir.join(repo_name);
                let _ = create_worktree(repo_path, &worktree_path, project_name);
                repo_mappings.push(RepoMapping::new(
                    repo_name,
                    repo_path.clone(),
                    worktree_path,
                ));
            }
        }

        let project_context = ProjectContext {
            project_name: project_name.to_string(),
            workspace_name: workspace_name.to_string(),
            description: ws.and_then(|w| w.description.clone()),
            repos: repo_mappings,
            env: ws.map(|w| w.env.clone()).unwrap_or_default(),
            initial_prompt: prompt.map(|s| s.to_string()),
        };

        let _ = generate_agents_md(&project_dir, &project_context);

        let mut pty_config = PtyConfig::new(&agent_cmd, &project_dir)
            .with_args(agent_args)
            .with_dimensions(100, 30);

        for (k, v) in &agent_env {
            pty_config = pty_config.with_env(k, v);
        }

        if let Some(ws) = ws {
            for (k, v) in &ws.env {
                pty_config = pty_config.with_env(k, v);
            }
        }

        let pty = PtySession::spawn(pty_config)?;
        let vt = VtScreen::new(100, 30);
        let mut ps = ProjectSession::new(project_name, workspace_name, agent_name);
        if let Some(p) = prompt {
            ps = ps.with_prompt(p);
        }

        self.registry.add(ps, pty, vt)?;
        Ok(())
    }

    /// Destroys a project directory and its worktrees, then refreshes the dashboard.
    pub fn destroy_project(&mut self, project_name: &str) -> anyhow::Result<()> {
        let cwd = self
            .cwd
            .clone()
            .or_else(|| env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."));
        let project_dir = cwd.join(project_name);

        if project_dir.exists() {
            if let Ok(entries) = fs::read_dir(&project_dir) {
                for entry in entries.flatten() {
                    let child_path = entry.path();
                    if child_path.is_dir() && child_path.join(".git").exists() {
                        let _ = remove_worktree(Path::new("."), &child_path, true);
                    }
                }
            }
            let _ = fs::remove_dir_all(&project_dir);
        }

        self.refresh_dashboard();
        Ok(())
    }

    /// Refreshes the local projects and workspaces list within the dashboard.
    pub fn refresh_dashboard(&mut self) {
        if let Some(dashboard) = &mut self.dashboard {
            let cwd = self
                .cwd
                .clone()
                .or_else(|| env::current_dir().ok())
                .unwrap_or_else(|| PathBuf::from("."));
            if let Some(cfg) = &self.config {
                *dashboard = DashboardState::from_env(cfg, &cwd);
            } else {
                let default_cfg = Config::load_from_str("default_agent = \"claude\"\n")
                    .unwrap_or_else(|_| Config {
                        default_agent: "claude".to_string(),
                        agents: HashMap::new(),
                        workspaces: HashMap::new(),
                    });
                *dashboard = DashboardState::from_env(&default_cfg, &cwd);
            }
        }
    }

    /// Dispatches key actions based on hotkeys or PTY passthrough.
    pub fn handle_key_event(&mut self, key: crossterm::event::KeyEvent) {
        let action = map_key_event(key);

        match action {
            TuiAction::Quit | TuiAction::Detach => {
                self.should_quit = true;
            }
            TuiAction::SwitchSession(idx) => {
                let _ = self.registry.set_active_index(idx);
            }
            TuiAction::NextSession => {
                self.registry.next_session();
            }
            TuiAction::PrevSession => {
                self.registry.prev_session();
            }
            TuiAction::CloseSession => {
                if let Some(active) = self.registry.active_session() {
                    let id = active.id().to_string();
                    let _ = self.registry.remove(&id);
                    if self.registry.is_empty() {
                        self.refresh_dashboard();
                    }
                }
            }
            TuiAction::Passthrough(bytes) => {
                if let Some(active) = self.registry.active_session_mut() {
                    let _ = active.pty_mut().write_all(&bytes);
                }
            }
            TuiAction::Resize(..) | TuiAction::Tick => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tui_app_creation_and_quit() {
        let registry = SessionRegistry::new();
        let mut app = TuiApp::new(registry);
        assert!(!app.should_quit);

        let quit_key = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('q'),
            crossterm::event::KeyModifiers::ALT,
        );
        app.handle_key_event(quit_key);
        assert!(app.should_quit);
    }

    #[test]
    fn test_tui_app_dashboard_actions() {
        let registry = SessionRegistry::new();
        let mut app = TuiApp::new(registry);
        let dashboard = DashboardState::new(vec!["ws1".to_string()], vec!["proj1".to_string()]);
        app = app.with_dashboard(dashboard);

        // Test dashboard quit action
        app.handle_dashboard_action(DashboardAction::Quit);
        assert!(app.should_quit);
    }
}
