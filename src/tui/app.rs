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
use ratatui::{
    backend::CrosstermBackend,
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    Terminal,
};

use crate::config::Config;
use crate::context::{generate_agents_md, ProjectContext, RepoMapping};
use crate::git::{create_worktree, remove_worktree};
use crate::notify::{ActivityTracker, NotificationService};
use crate::session::{
    ProjectSession, PtyConfig, PtySession, SessionRegistry, SessionStatus, VtScreen,
};
use crate::tui::dashboard::{render_dashboard, DashboardAction, DashboardState};
use crate::tui::events::{map_key_event, TuiAction};
use crate::tui::quick_launcher::{
    render_quick_launcher, QuickLauncherAction, QuickLauncherState,
};
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
    pub quick_launcher: Option<QuickLauncherState>,
    pub config: Option<Config>,
    pub cwd: Option<PathBuf>,
    pub viewport_size: Option<(u16, u16)>,
}

struct TerminalCleanupGuard;

impl Drop for TerminalCleanupGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen, crossterm::cursor::Show);
    }
}

/// Renders an error / crash banner at the bottom line of the given viewport area.
pub fn render_error_banner(exit_code: Option<u32>, area: Rect, buf: &mut Buffer) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let code_str = exit_code
        .map(|c| c.to_string())
        .unwrap_or_else(|| "1".to_string());
    let msg = format!(
        " [Process exited with code {} - Press Enter or Alt+X to dismiss] ",
        code_str
    );
    let banner_y = area.bottom().saturating_sub(1);
    let style = Style::default()
        .bg(Color::Red)
        .fg(Color::White)
        .add_modifier(Modifier::BOLD);

    for x in area.left()..area.right() {
        if let Some(cell) = buf.cell_mut((x, banner_y)) {
            cell.set_style(style).set_char(' ');
        }
    }
    let line = Line::from(Span::styled(&msg, style));
    buf.set_line(area.x, banner_y, &line, area.width);
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
            quick_launcher: None,
            config: None,
            cwd: None,
            viewport_size: None,
        }
    }

    /// Sets the dashboard state on `TuiApp`.
    pub fn with_dashboard(mut self, dashboard: DashboardState) -> Self {
        self.dashboard = Some(dashboard);
        self
    }

    /// Sets the quick launcher state on `TuiApp`.
    pub fn with_quick_launcher(mut self, quick_launcher: QuickLauncherState) -> Self {
        self.quick_launcher = Some(quick_launcher);
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

    /// Resizes all registered sessions to the specified viewport column and row dimensions.
    pub fn resize_all_sessions(&mut self, cols: u16, rows: u16) {
        self.registry.resize_all(cols, rows);
    }

    /// Removes a session by ID and handles exit transitions (dashboard refresh or quitting).
    pub fn close_session(&mut self, id: &str) {
        let _ = self.registry.remove(id);
        if self.registry.is_empty() {
            if self.dashboard.is_some() {
                self.refresh_dashboard();
            } else {
                self.should_quit = true;
            }
        }
    }

    /// Runs the interactive TUI application event loop.
    pub fn run(&mut self, stdout: &mut Stdout) -> io::Result<()> {
        enable_raw_mode()?;
        execute!(stdout, EnterAlternateScreen, crossterm::cursor::Show)?;
        let _guard = TerminalCleanupGuard;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        let res = self.run_loop(&mut terminal);

        // Always restore terminal state even on error
        disable_raw_mode()?;
        execute!(terminal.backend_mut(), LeaveAlternateScreen, crossterm::cursor::Show)?;
        terminal.show_cursor()?;

        res
    }

    /// Internal event loop.
    pub fn run_loop<B: ratatui::backend::Backend>(
        &mut self,
        terminal: &mut Terminal<B>,
    ) -> io::Result<()> {
        let mut last_tick = Instant::now();

        // Query initial terminal viewport size and synchronize sessions
        let initial_size = terminal.size()?;
        let (_, initial_term_area) = compute_layout(ratatui::layout::Rect::new(
            0,
            0,
            initial_size.width,
            initial_size.height,
        ));
        self.viewport_size = Some((initial_term_area.width, initial_term_area.height));
        self.resize_all_sessions(initial_term_area.width, initial_term_area.height);

        while !self.should_quit {
            // 1. Synchronize viewport dimensions before processing PTY streams or drawing
            let current_size = terminal.size()?;
            let (_, term_area) = compute_layout(ratatui::layout::Rect::new(
                0,
                0,
                current_size.width,
                current_size.height,
            ));
            if self.viewport_size != Some((term_area.width, term_area.height)) {
                self.viewport_size = Some((term_area.width, term_area.height));
                self.resize_all_sessions(term_area.width, term_area.height);
            }

            // 2. Process PTY outputs for all sessions
            self.process_pty_streams();

            // 3. Draw active frame
            terminal.draw(|f| {
                let (status_area, term_area) = compute_layout(f.area());

                if self.registry.is_empty() && self.dashboard.is_some() {
                    if let Some(dashboard) = &self.dashboard {
                        render_dashboard(dashboard, f.area(), f.buffer_mut());
                        if let Some(pos) = dashboard.cursor_position(f.area()) {
                            f.set_cursor_position(pos);
                        }
                    }
                } else {
                    f.render_widget(StatusBar::new(&self.registry), status_area);

                    if let Some(active) = self.registry.active_session() {
                        f.render_widget(TerminalView::new(active.vt()), term_area);

                        if active.status() == SessionStatus::Error {
                            render_error_banner(
                                active.project_session.exit_code,
                                term_area,
                                f.buffer_mut(),
                            );
                        } else if !active.vt().hide_cursor() {
                            let (col, row) = active.vt().cursor_position();
                            if col < term_area.width && row < term_area.height {
                                f.set_cursor_position(ratatui::layout::Position::new(
                                    term_area.x + col,
                                    term_area.y + row,
                                ));
                            }
                        }
                    }
                }

                if let Some(launcher) = &self.quick_launcher {
                    render_quick_launcher(launcher, f.area(), f.buffer_mut());
                    if let Some(pos) = launcher.cursor_position(f.area()) {
                        f.set_cursor_position(pos);
                    }
                }
            })?;

            // 4. Handle user input with timeout up to tick_rate
            let timeout = self.tick_rate.saturating_sub(last_tick.elapsed());

            if event::poll(timeout)? {
                while event::poll(Duration::from_millis(0))? {
                    match event::read()? {
                        Event::Key(key) => {
                            if key.kind == KeyEventKind::Press {
                                if self.quick_launcher.is_some() {
                                    self.handle_launcher_key(key);
                                } else if self.registry.is_empty() && self.dashboard.is_some() {
                                    self.handle_dashboard_key(key);
                                } else {
                                    self.handle_key_event(key);
                                }
                            }
                        }
                        Event::Resize(cols, rows) => {
                            let (_, term_area) =
                                compute_layout(ratatui::layout::Rect::new(0, 0, cols, rows));
                            if self.viewport_size != Some((term_area.width, term_area.height)) {
                                self.viewport_size = Some((term_area.width, term_area.height));
                                self.resize_all_sessions(term_area.width, term_area.height);
                            }
                        }
                        _ => {}
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
        let mut clean_exits = Vec::new();

        for session in self.registry.list_mut() {
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

            if let Some(code) = exit_code {
                session.project_session.exit_code = Some(code as u32);
            }

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

            if !is_alive {
                if exit_code == Some(0) {
                    clean_exits.push(session.id().to_string());
                } else {
                    session.project_session.set_status(SessionStatus::Error);
                }
            }
        }

        for id in clean_exits {
            self.close_session(&id);
        }
    }

    /// Opens the in-session Quick Project Launcher modal.
    pub fn open_quick_launcher(&mut self) {
        let cwd = self
            .cwd
            .clone()
            .or_else(|| env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."));

        let mut local_projects = Vec::new();
        if let Ok(entries) = fs::read_dir(&cwd) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() && path.join("AGENTS.md").is_file() {
                    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                        local_projects.push(name.to_string());
                    }
                }
            }
        }
        local_projects.sort();

        let mut workspaces = Vec::new();
        if let Some(cfg) = &self.config {
            workspaces = cfg.workspaces.keys().cloned().collect();
        }
        workspaces.sort();

        let active_ws = self
            .registry
            .active_session()
            .map(|s| s.workspace_name().to_string());

        let launcher = QuickLauncherState::new(workspaces, local_projects, active_ws.as_deref());
        self.quick_launcher = Some(launcher);
    }

    /// Dispatches a key event to the quick launcher and handles any triggered action.
    pub fn handle_launcher_key(&mut self, key: crossterm::event::KeyEvent) {
        if let Some(mut launcher) = self.quick_launcher.take() {
            let action = launcher.handle_key(key);
            match action {
                QuickLauncherAction::Cancel => {
                    self.quick_launcher = None;
                }
                QuickLauncherAction::Create {
                    workspace,
                    project,
                    prompt,
                } => {
                    match self.create_and_start_project(&workspace, &project, prompt.as_deref()) {
                        Ok(()) => {
                            let new_idx = self.registry.len().saturating_sub(1);
                            let _ = self.registry.set_active_index(new_idx);
                            self.quick_launcher = None;
                        }
                        Err(err) => {
                            launcher.set_error(err.to_string());
                            self.quick_launcher = Some(launcher);
                        }
                    }
                }
                QuickLauncherAction::Attach(project) => {
                    if let Some(pos) = self
                        .registry
                        .list()
                        .iter()
                        .position(|s| s.project_name() == project)
                    {
                        let _ = self.registry.set_active_index(pos);
                        self.quick_launcher = None;
                    } else {
                        match self.attach_project(&project) {
                            Ok(()) => {
                                let new_idx = self.registry.len().saturating_sub(1);
                                let _ = self.registry.set_active_index(new_idx);
                                self.quick_launcher = None;
                            }
                            Err(err) => {
                                launcher.set_error(err.to_string());
                                self.quick_launcher = Some(launcher);
                            }
                        }
                    }
                }
                QuickLauncherAction::None => {
                    self.quick_launcher = Some(launcher);
                }
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
        if self.registry.get_by_project(project_name).is_some() {
            anyhow::bail!("Project '{}' already has an active session in registry", project_name);
        }

        let cwd = self
            .cwd
            .clone()
            .or_else(|| env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."));
        let project_dir = cwd.join(project_name);

        if !project_dir.exists() {
            anyhow::bail!("Project directory '{}' does not exist in CWD", project_name);
        }
        let project_dir = project_dir.canonicalize().unwrap_or(project_dir);

        let default_config =
            Config::load_from_str("default_agent = \"claude\"\nrepos_dir = \".botai_repos\"\n").ok();
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

        let (cols, rows) = self.viewport_size.unwrap_or((100, 30));

        let mut pty_config = PtyConfig::new(&cmd, &project_dir)
            .with_args(args)
            .with_dimensions(cols, rows);

        for (k, v) in &env_map {
            pty_config = pty_config.with_env(k, v);
        }

        let pty = PtySession::spawn(pty_config)?;
        let vt = VtScreen::new(cols, rows);
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
        if self.registry.get_by_project(project_name).is_some() {
            anyhow::bail!("Project '{}' already has an active session in registry", project_name);
        }

        let cwd = self
            .cwd
            .clone()
            .or_else(|| env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."));
        let project_dir = cwd.join(project_name);

        if project_dir.exists() {
            anyhow::bail!("Project directory '{}' already exists in CWD", project_name);
        }

        let default_config =
            Config::load_from_str("default_agent = \"claude\"\nrepos_dir = \".botai_repos\"\n").ok();
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
        let project_dir = project_dir.canonicalize().unwrap_or(project_dir);

        let mut repo_mappings = Vec::new();
        if let Some(cfg) = config {
            if let Ok(resolved_repos) = cfg.resolve_workspace_repos(workspace_name) {
                for repo in resolved_repos {
                    if !repo.exists_locally() {
                        if let Some(url) = &repo.clone_url {
                            let _ = crate::git::clone_repo(url, &repo.local_path);
                        }
                    } else {
                        let _ = crate::git::fetch_repo(&repo.local_path);
                    }
                    let worktree_path = project_dir.join(&repo.name);
                    let _ = create_worktree(&repo.local_path, &worktree_path, project_name);
                    repo_mappings.push(RepoMapping::new(
                        repo.name,
                        repo.local_path,
                        worktree_path,
                    ));
                }
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

        let (cols, rows) = self.viewport_size.unwrap_or((100, 30));

        let mut pty_config = PtyConfig::new(&agent_cmd, &project_dir)
            .with_args(agent_args)
            .with_dimensions(cols, rows);

        for (k, v) in &agent_env {
            pty_config = pty_config.with_env(k, v);
        }

        if let Some(ws) = ws {
            for (k, v) in &ws.env {
                pty_config = pty_config.with_env(k, v);
            }
        }

        let pty = PtySession::spawn(pty_config)?;
        let vt = VtScreen::new(cols, rows);
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
                let default_cfg =
                    Config::load_from_str("default_agent = \"claude\"\nrepos_dir = \".botai_repos\"\n")
                        .unwrap_or_else(|_| Config {
                            default_agent: "claude".to_string(),
                            repos_dir: PathBuf::from(".botai_repos"),
                            agents: HashMap::new(),
                            workspaces: HashMap::new(),
                        });
                *dashboard = DashboardState::from_env(&default_cfg, &cwd);
            }
        }
    }

    /// Dispatches key actions based on hotkeys or PTY passthrough.
    pub fn handle_key_event(&mut self, key: crossterm::event::KeyEvent) {
        let is_active_errored = self
            .registry
            .active_session()
            .map(|s| s.status() == SessionStatus::Error)
            .unwrap_or(false);

        // If the active session is errored, dismiss it on Enter, Space, or Alt+X
        if is_active_errored {
            let is_dismiss_key = match key.code {
                crossterm::event::KeyCode::Enter | crossterm::event::KeyCode::Char(' ') => {
                    !key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL)
                        && !key.modifiers.contains(crossterm::event::KeyModifiers::ALT)
                }
                crossterm::event::KeyCode::Char('x') | crossterm::event::KeyCode::Char('X') => {
                    key.modifiers.contains(crossterm::event::KeyModifiers::ALT)
                }
                _ => false,
            };

            if is_dismiss_key {
                if let Some(active) = self.registry.active_session() {
                    let id = active.id().to_string();
                    self.close_session(&id);
                    return;
                }
            }
        }

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
            TuiAction::OpenLauncher => {
                self.open_quick_launcher();
            }
            TuiAction::CloseSession => {
                if let Some(active) = self.registry.active_session() {
                    let id = active.id().to_string();
                    self.close_session(&id);
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

    #[test]
    fn test_tui_app_open_launcher_on_alt_c() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let cwd = tmp.path().to_path_buf();

        let config_toml = r#"
default_agent = "mock"
repos_dir = "/tmp/repos"

[agents.mock]
command = "sh"
args = ["-c", "exit 0"]

[workspaces.ecommerce]
repos = []
"#;
        let config = Config::load_from_str(config_toml).unwrap();
        let registry = SessionRegistry::new();
        let mut app = TuiApp::new(registry)
            .with_config(config)
            .with_cwd(cwd);

        assert!(app.quick_launcher.is_none());

        // Send Alt+C key
        let alt_c = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('c'),
            crossterm::event::KeyModifiers::ALT,
        );
        app.handle_key_event(alt_c);

        assert!(app.quick_launcher.is_some());
        let launcher = app.quick_launcher.as_ref().unwrap();
        assert_eq!(launcher.workspaces, vec!["ecommerce".to_string()]);
    }

    #[test]
    fn test_tui_app_launcher_create_project_lifecycle() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let cwd = tmp.path().to_path_buf();

        #[cfg(windows)]
        let (cmd, args) = ("cmd.exe", r#"["/c", "exit 0"]"#);
        #[cfg(not(windows))]
        let (cmd, args) = ("sh", r#"["-c", "exit 0"]"#);

        let config_toml = format!(
            r#"
default_agent = "mock"
repos_dir = "/tmp/repos"

[agents.mock]
command = "{}"
args = {}

[workspaces.backend]
repos = []
"#,
            cmd, args
        );
        let config = Config::load_from_str(&config_toml).unwrap();
        let registry = SessionRegistry::new();
        let mut app = TuiApp::new(registry)
            .with_config(config)
            .with_cwd(cwd.clone());

        // 1. Open launcher
        app.open_quick_launcher();
        assert!(app.quick_launcher.is_some());

        // 2. Type project name
        let mut launcher = app.quick_launcher.take().unwrap();
        launcher.focus = crate::tui::quick_launcher::LauncherFocus::ProjectName;
        for c in "billing-service".chars() {
            launcher.handle_key(crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Char(c),
                crossterm::event::KeyModifiers::NONE,
            ));
        }
        app.quick_launcher = Some(launcher);

        // 3. Submit with Enter
        let enter_key = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_launcher_key(enter_key);

        // Modal should close on success and registry should contain 1 active session
        assert!(app.quick_launcher.is_none());
        assert_eq!(app.registry.len(), 1);
        let active = app.registry.active_session().unwrap();
        assert_eq!(active.project_name(), "billing-service");
        assert!(active.pty().config().working_dir.is_absolute());
        assert_eq!(
            active.pty().config().working_dir,
            cwd.join("billing-service").canonicalize().unwrap()
        );
        assert!(cwd.join("billing-service").join("AGENTS.md").is_file());

        // 4. Try creating another project with the same name -> Collision should keep modal open with error
        app.open_quick_launcher();
        let mut launcher = app.quick_launcher.take().unwrap();
        launcher.focus = crate::tui::quick_launcher::LauncherFocus::ProjectName;
        for c in "billing-service".chars() {
            launcher.handle_key(crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Char(c),
                crossterm::event::KeyModifiers::NONE,
            ));
        }
        app.quick_launcher = Some(launcher);

        app.handle_launcher_key(enter_key);
        assert!(app.quick_launcher.is_some(), "Modal must remain open on error");
        assert!(
            app.quick_launcher
                .as_ref()
                .unwrap()
                .error_message
                .is_some(),
            "Error message must be set on collision"
        );
    }

    #[test]
    fn test_tui_app_launcher_attach_and_switch_focus() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let cwd = tmp.path().to_path_buf();

        #[cfg(windows)]
        let (cmd, args) = ("cmd.exe", r#"["/c", "exit 0"]"#);
        #[cfg(not(windows))]
        let (cmd, args) = ("sh", r#"["-c", "exit 0"]"#);

        let config_toml = format!(
            r#"
default_agent = "mock"
repos_dir = "/tmp/repos"

[agents.mock]
command = "{}"
args = {}

[workspaces.ws1]
repos = []
"#,
            cmd, args
        );
        let config = Config::load_from_str(&config_toml).unwrap();

        // Create existing project directory with AGENTS.md
        let proj_dir = cwd.join("existing-proj");
        fs::create_dir(&proj_dir).unwrap();
        fs::write(proj_dir.join("AGENTS.md"), "# Project").unwrap();

        let registry = SessionRegistry::new();
        let mut app = TuiApp::new(registry)
            .with_config(config)
            .with_cwd(cwd.clone());

        // 1. Open launcher, switch to Attach tab and attach "existing-proj"
        app.open_quick_launcher();
        let mut launcher = app.quick_launcher.take().unwrap();
        launcher.switch_tab(crate::tui::quick_launcher::LauncherTab::Attach);
        app.quick_launcher = Some(launcher);

        let enter_key = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_launcher_key(enter_key);

        assert!(app.quick_launcher.is_none());
        assert_eq!(app.registry.len(), 1);
        let active = app.registry.active_session().unwrap();
        assert_eq!(active.project_name(), "existing-proj");
        assert!(active.pty().config().working_dir.is_absolute());
        assert_eq!(
            active.pty().config().working_dir,
            cwd.join("existing-proj").canonicalize().unwrap()
        );

        // 2. Open launcher again, attaching the already-running project should switch focus to it
        app.open_quick_launcher();
        let mut launcher = app.quick_launcher.take().unwrap();
        launcher.switch_tab(crate::tui::quick_launcher::LauncherTab::Attach);
        app.quick_launcher = Some(launcher);

        app.handle_launcher_key(enter_key);
        assert!(app.quick_launcher.is_none());
        assert_eq!(app.registry.len(), 1);
        assert_eq!(app.registry.active_index(), Some(0));
    }

    #[test]
    fn test_tui_app_launcher_cancel_on_esc() {
        let registry = SessionRegistry::new();
        let mut app = TuiApp::new(registry);
        app.quick_launcher = Some(QuickLauncherState::new(vec![], vec![], None));

        let esc_key = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Esc,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_launcher_key(esc_key);
        assert!(app.quick_launcher.is_none());
    }

    #[test]
    fn test_tui_app_resize_all_sessions() {
        let tmp = tempfile::tempdir().expect("tempdir");
        #[cfg(windows)]
        let (cmd, args) = ("cmd.exe", vec!["/c", "exit 0"]);
        #[cfg(not(windows))]
        let (cmd, args) = ("sh", vec!["-c", "true"]);

        let mut registry = SessionRegistry::new();
        let config1 = PtyConfig::new(cmd, tmp.path()).with_args(args.clone()).with_dimensions(80, 24);
        let pty1 = PtySession::spawn(config1).unwrap();
        let vt1 = VtScreen::new(80, 24);
        let ps1 = ProjectSession::new("proj1", "ws1", "mock");
        registry.add(ps1, pty1, vt1).unwrap();

        let config2 = PtyConfig::new(cmd, tmp.path()).with_args(args).with_dimensions(80, 24);
        let pty2 = PtySession::spawn(config2).unwrap();
        let vt2 = VtScreen::new(80, 24);
        let ps2 = ProjectSession::new("proj2", "ws1", "mock");
        registry.add(ps2, pty2, vt2).unwrap();

        let mut app = TuiApp::new(registry);
        assert_eq!(app.registry.list()[0].vt().size(), (80, 24));
        assert_eq!(app.registry.list()[1].vt().size(), (80, 24));

        app.resize_all_sessions(130, 45);
        assert_eq!(app.registry.list()[0].vt().size(), (130, 45));
        assert_eq!(app.registry.list()[1].vt().size(), (130, 45));
    }

    #[test]
    fn test_tui_app_viewport_size_applied_to_new_project_sessions() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let cwd = tmp.path().to_path_buf();

        #[cfg(windows)]
        let (cmd, args) = ("cmd.exe", r#"["/c", "exit 0"]"#);
        #[cfg(not(windows))]
        let (cmd, args) = ("sh", r#"["-c", "exit 0"]"#);

        let config_toml = format!(
            r#"
default_agent = "mock"
repos_dir = "/tmp/repos"

[agents.mock]
command = "{}"
args = {}

[workspaces.backend]
repos = []
"#,
            cmd, args
        );
        let config = Config::load_from_str(&config_toml).unwrap();
        let registry = SessionRegistry::new();
        let mut app = TuiApp::new(registry)
            .with_config(config)
            .with_cwd(cwd.clone());

        // Set custom viewport size
        app.viewport_size = Some((115, 38));

        app.create_and_start_project("backend", "resized-project", None).unwrap();
        let active = app.registry.active_session().unwrap();
        assert_eq!(active.vt().size(), (115, 38));

        // Create existing project dir and attach
        let attach_dir = cwd.join("attached-resized");
        fs::create_dir(&attach_dir).unwrap();
        fs::write(attach_dir.join("AGENTS.md"), "# Project").unwrap();

        app.attach_project("attached-resized").unwrap();
        let attached = app.registry.get_by_project("attached-resized").unwrap();
        assert_eq!(attached.vt().size(), (115, 38));
    }

    #[test]
    fn test_render_error_banner() {
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        render_error_banner(Some(127), area, &mut buf);

        let bottom_row_text = (0..80)
            .map(|x| buf.cell((x, 23)).unwrap().symbol())
            .collect::<String>();

        assert!(bottom_row_text.contains("[Process exited with code 127 - Press Enter or Alt+X to dismiss]"));

        let cell = buf.cell((0, 23)).unwrap();
        assert_eq!(cell.bg, Color::Red);
        assert_eq!(cell.fg, Color::White);
        assert!(cell.modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn test_session_exit_normal_multi_session_closes_tab_and_switches_focus() {
        let tmp = tempfile::tempdir().expect("tempdir");

        #[cfg(windows)]
        let (cmd1, args1) = ("cmd.exe", vec!["/c", "exit 0"]);
        #[cfg(not(windows))]
        let (cmd1, args1) = ("sh", vec!["-c", "exit 0"]);

        #[cfg(windows)]
        let (cmd2, args2) = ("cmd.exe", vec!["/c", "ping -n 10 127.0.0.1 > nul"]);
        #[cfg(not(windows))]
        let (cmd2, args2) = ("sh", vec!["-c", "sleep 10"]);

        let mut registry = SessionRegistry::new();
        let config1 = PtyConfig::new(cmd1, tmp.path()).with_args(args1);
        let pty1 = PtySession::spawn(config1).unwrap();
        let vt1 = VtScreen::new(80, 24);
        let ps1 = ProjectSession::new("proj1-finishing", "ws", "mock");
        registry.add(ps1, pty1, vt1).unwrap();

        let config2 = PtyConfig::new(cmd2, tmp.path()).with_args(args2);
        let pty2 = PtySession::spawn(config2).unwrap();
        let vt2 = VtScreen::new(80, 24);
        let ps2 = ProjectSession::new("proj2-running", "ws", "mock");
        registry.add(ps2, pty2, vt2).unwrap();

        let mut app = TuiApp::new(registry);
        assert_eq!(app.registry.len(), 2);
        assert_eq!(app.registry.active_index(), Some(0));

        // Wait for session 1 to exit 0
        std::thread::sleep(Duration::from_millis(150));

        app.process_pty_streams();

        // Session 1 should be removed, leaving only session 2
        assert_eq!(app.registry.len(), 1);
        assert_eq!(app.registry.active_index(), Some(0));
        assert_eq!(app.registry.active_session().unwrap().project_name(), "proj2-running");
        assert!(!app.should_quit);
    }

    #[test]
    fn test_session_exit_normal_last_session_with_dashboard_returns_to_dashboard() {
        let tmp = tempfile::tempdir().expect("tempdir");

        #[cfg(windows)]
        let (cmd, args) = ("cmd.exe", vec!["/c", "exit 0"]);
        #[cfg(not(windows))]
        let (cmd, args) = ("sh", vec!["-c", "exit 0"]);

        let mut registry = SessionRegistry::new();
        let config = PtyConfig::new(cmd, tmp.path()).with_args(args);
        let pty = PtySession::spawn(config).unwrap();
        let vt = VtScreen::new(80, 24);
        let ps = ProjectSession::new("proj-done", "ws", "mock");
        registry.add(ps, pty, vt).unwrap();

        let dashboard = DashboardState::new(vec!["ws".to_string()], vec![]);
        let mut app = TuiApp::new(registry).with_dashboard(dashboard);

        // Wait for process to exit 0
        std::thread::sleep(Duration::from_millis(150));

        app.process_pty_streams();

        // Registry is empty, dashboard is active, should_quit is false
        assert_eq!(app.registry.len(), 0);
        assert!(app.dashboard.is_some());
        assert!(!app.should_quit);
    }

    #[test]
    fn test_session_exit_normal_last_session_without_dashboard_sets_should_quit() {
        let tmp = tempfile::tempdir().expect("tempdir");

        #[cfg(windows)]
        let (cmd, args) = ("cmd.exe", vec!["/c", "exit 0"]);
        #[cfg(not(windows))]
        let (cmd, args) = ("sh", vec!["-c", "exit 0"]);

        let mut registry = SessionRegistry::new();
        let config = PtyConfig::new(cmd, tmp.path()).with_args(args);
        let pty = PtySession::spawn(config).unwrap();
        let vt = VtScreen::new(80, 24);
        let ps = ProjectSession::new("proj-done", "ws", "mock");
        registry.add(ps, pty, vt).unwrap();

        let mut app = TuiApp::new(registry);
        assert!(app.dashboard.is_none());

        // Wait for process to exit 0
        std::thread::sleep(Duration::from_millis(150));

        app.process_pty_streams();

        // Registry is empty, should_quit is true
        assert_eq!(app.registry.len(), 0);
        assert!(app.should_quit);
    }

    #[test]
    fn test_session_exit_error_retained_and_dismissed_with_enter() {
        let tmp = tempfile::tempdir().expect("tempdir");

        #[cfg(windows)]
        let (cmd, args) = ("cmd.exe", vec!["/c", "exit 42"]);
        #[cfg(not(windows))]
        let (cmd, args) = ("sh", vec!["-c", "exit 42"]);

        let mut registry = SessionRegistry::new();
        let config = PtyConfig::new(cmd, tmp.path()).with_args(args);
        let pty = PtySession::spawn(config).unwrap();
        let vt = VtScreen::new(80, 24);
        let ps = ProjectSession::new("proj-fail", "ws", "mock");
        registry.add(ps, pty, vt).unwrap();

        let mut app = TuiApp::new(registry);

        // Wait for process to exit 42
        std::thread::sleep(Duration::from_millis(150));

        app.process_pty_streams();

        // Errored session remains in registry with status Error and exit code 42
        assert_eq!(app.registry.len(), 1);
        let active = app.registry.active_session().unwrap();
        assert_eq!(active.status(), SessionStatus::Error);
        assert_eq!(active.project_session.exit_code, Some(42));
        assert!(!app.should_quit);

        // Pressing normal keys like 'a' does not dismiss it
        let key_a = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('a'),
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key_event(key_a);
        assert_eq!(app.registry.len(), 1);

        // Pressing Enter dismisses the errored session
        let enter_key = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key_event(enter_key);

        assert_eq!(app.registry.len(), 0);
        assert!(app.should_quit);
    }

    #[test]
    fn test_session_exit_error_dismissed_with_space_and_alt_x() {
        let tmp = tempfile::tempdir().expect("tempdir");

        #[cfg(windows)]
        let (cmd, args) = ("cmd.exe", vec!["/c", "exit 1"]);
        #[cfg(not(windows))]
        let (cmd, args) = ("sh", vec!["-c", "exit 1"]);

        // Test Space dismissal with dashboard
        let mut registry1 = SessionRegistry::new();
        let config1 = PtyConfig::new(cmd, tmp.path()).with_args(args.clone());
        let pty1 = PtySession::spawn(config1).unwrap();
        let vt1 = VtScreen::new(80, 24);
        let ps1 = ProjectSession::new("proj-space", "ws", "mock");
        registry1.add(ps1, pty1, vt1).unwrap();

        let dashboard = DashboardState::new(vec!["ws".to_string()], vec![]);
        let mut app1 = TuiApp::new(registry1).with_dashboard(dashboard);
        std::thread::sleep(Duration::from_millis(150));
        app1.process_pty_streams();
        assert_eq!(app1.registry.len(), 1);

        let space_key = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char(' '),
            crossterm::event::KeyModifiers::NONE,
        );
        app1.handle_key_event(space_key);
        assert_eq!(app1.registry.len(), 0);
        assert!(app1.dashboard.is_some());
        assert!(!app1.should_quit);

        // Test Alt+X dismissal
        let mut registry2 = SessionRegistry::new();
        let config2 = PtyConfig::new(cmd, tmp.path()).with_args(args);
        let pty2 = PtySession::spawn(config2).unwrap();
        let vt2 = VtScreen::new(80, 24);
        let ps2 = ProjectSession::new("proj-alt-x", "ws", "mock");
        registry2.add(ps2, pty2, vt2).unwrap();

        let mut app2 = TuiApp::new(registry2);
        std::thread::sleep(Duration::from_millis(150));
        app2.process_pty_streams();
        assert_eq!(app2.registry.len(), 1);

        let alt_x = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('x'),
            crossterm::event::KeyModifiers::ALT,
        );
        app2.handle_key_event(alt_x);
        assert_eq!(app2.registry.len(), 0);
        assert!(app2.should_quit);
    }
}

