use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::{self, Stdout};
use std::path::PathBuf;
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
    style::{Modifier, Style},
    text::{Line, Span},
    Terminal,
};

use crate::config::Config;
use crate::context::{generate_agents_md, ProjectContext, RepoMapping};
use crate::git::{create_worktree, remove_worktree_auto};
use crate::notify::{ActivityTracker, NotificationService};
use crate::session::{
    ProjectSession, PtyConfig, PtySession, SessionRegistry, SessionStatus, VtScreen,
};
use crate::tui::dashboard::{render_dashboard, DashboardAction, DashboardState};
use crate::tui::events::{map_key_event, TuiAction};
use crate::tui::quick_launcher::{
    render_quick_launcher, QuickLauncherAction, QuickLauncherState,
};
use crate::tui::setup_modal::{
    render_setup_modal, SetupModalState, SetupProgressEvent,
};
use crate::tui::status_bar::StatusBar;
use crate::tui::theme::Theme;
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
    pub setup_modal: Option<SetupModalState>,
    pub config: Option<Config>,
    pub cwd: Option<PathBuf>,
    pub viewport_size: Option<(u16, u16)>,
    pub leader_active: bool,
    pub showing_dashboard: bool,
}


struct TerminalCleanupGuard;

impl Drop for TerminalCleanupGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(
            io::stdout(),
            crossterm::event::DisableMouseCapture,
            LeaveAlternateScreen,
            crossterm::cursor::Show
        );
    }
}

/// Renders an error / crash banner at the bottom line of the given viewport area.
pub fn render_error_banner(exit_code: Option<u32>, area: Rect, buf: &mut Buffer, theme: &Theme) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let code_str = exit_code
        .map(|c| c.to_string())
        .unwrap_or_else(|| "1".to_string());
    let msg = format!(
        " [Process exited with code {} - Press Enter or Ctrl+Space w to dismiss] ",
        code_str
    );
    let banner_y = area.bottom().saturating_sub(1);
    let style = Style::default()
        .bg(theme.error_banner_bg)
        .fg(theme.error_banner_fg)
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
            setup_modal: None,
            config: None,
            cwd: None,
            viewport_size: None,
            leader_active: false,
            showing_dashboard: false,
        }
    }

    /// Returns whether the dashboard should currently be displayed instead of an active session.
    pub fn is_showing_dashboard(&self) -> bool {
        (self.showing_dashboard || self.registry.is_empty()) && self.dashboard.is_some()
    }

    /// Sets the dashboard state on `TuiApp`.
    pub fn with_dashboard(mut self, mut dashboard: DashboardState) -> Self {
        dashboard.set_running_sessions(self.running_sessions_map());
        self.dashboard = Some(dashboard);
        self
    }

    /// Sets the quick launcher state on `TuiApp`.
    pub fn with_quick_launcher(mut self, mut quick_launcher: QuickLauncherState) -> Self {
        quick_launcher.set_running_sessions(self.running_sessions_map());
        self.quick_launcher = Some(quick_launcher);
        self
    }

    /// Sets the setup modal state on `TuiApp`.
    pub fn with_setup_modal(mut self, setup_modal: SetupModalState) -> Self {
        self.setup_modal = Some(setup_modal);
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
                self.showing_dashboard = true;
            } else {
                self.should_quit = true;
            }
        }
    }

    /// Runs the interactive TUI application event loop.
    pub fn run(&mut self, stdout: &mut Stdout) -> io::Result<()> {
        enable_raw_mode()?;
        execute!(
            stdout,
            EnterAlternateScreen,
            crossterm::event::EnableMouseCapture,
            crossterm::cursor::Show
        )?;
        let _guard = TerminalCleanupGuard;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        let res = self.run_loop(&mut terminal);

        // Always restore terminal state even on error
        disable_raw_mode()?;
        execute!(
            terminal.backend_mut(),
            crossterm::event::DisableMouseCapture,
            LeaveAlternateScreen,
            crossterm::cursor::Show
        )?;
        terminal.show_cursor()?;

        res
    }

    /// Extracts a map of project names to their current session status from the registry.
    pub fn running_sessions_map(&self) -> HashMap<String, SessionStatus> {
        self.registry
            .list()
            .iter()
            .map(|s| (s.project_name().to_string(), s.status()))
            .collect()
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
            // Synchronize viewport dimensions before processing PTY streams or drawing
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

            // 1. Poll setup progress events
            self.poll_setup_progress();

            // 2. Process PTY outputs for all sessions
            self.process_pty_streams();

            // Synchronize running sessions on dashboard and quick_launcher
            let running = self.running_sessions_map();
            if let Some(dashboard) = &mut self.dashboard {
                dashboard.set_running_sessions(running.clone());
            }
            if let Some(launcher) = &mut self.quick_launcher {
                launcher.set_running_sessions(running);
            }

            // 3. Draw active frame
            let theme = self.config.as_ref().map(|c| c.theme()).unwrap_or_default();
            terminal.draw(|f| {
                let (status_area, term_area) = compute_layout(f.area());

                if self.is_showing_dashboard() {
                    if let Some(dashboard) = &self.dashboard {
                        render_dashboard(dashboard, f.area(), f.buffer_mut(), &theme);
                        if let Some(pos) = dashboard.cursor_position(f.area()) {
                            f.set_cursor_position(pos);
                        }
                    }
                } else {
                    f.render_widget(
                        StatusBar::new(&self.registry)
                            .with_leader(self.leader_active)
                            .with_theme(&theme),
                        status_area,
                    );

                    if let Some(active) = self.registry.active_session() {
                        f.render_widget(TerminalView::new(active.vt()), term_area);

                        if active.status() == SessionStatus::Error {
                            render_error_banner(
                                active.project_session.exit_code,
                                term_area,
                                f.buffer_mut(),
                                &theme,
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
                    render_quick_launcher(launcher, f.area(), f.buffer_mut(), &theme);
                    if let Some(pos) = launcher.cursor_position(f.area()) {
                        f.set_cursor_position(pos);
                    }
                }

                if let Some(setup_modal) = &self.setup_modal {
                    render_setup_modal(setup_modal, f.area(), f.buffer_mut(), &theme);
                }
            })?;

            // 4. Handle user input with timeout up to tick_rate
            let timeout = self.tick_rate.saturating_sub(last_tick.elapsed());

            if event::poll(timeout)? {
                while event::poll(Duration::from_millis(0))? {
                    match event::read()? {
                        Event::Key(key) => {
                            if key.kind == KeyEventKind::Press {
                                if self.setup_modal.is_some() {
                                    self.handle_setup_modal_key(key);
                                } else if self.quick_launcher.is_some() {
                                    self.handle_launcher_key(key);
                                } else if self.is_showing_dashboard() {
                                    self.handle_dashboard_key(key);
                                } else {
                                    self.handle_key_event(key);
                                }
                            }
                        }
                        Event::Mouse(mouse) => {
                            if self.setup_modal.is_some() {
                                self.handle_setup_modal_mouse(mouse);
                            } else if self.quick_launcher.is_some() {
                                self.handle_launcher_mouse(mouse);
                            } else if self.is_showing_dashboard() {
                                self.handle_dashboard_mouse(mouse);
                            } else if !self.leader_active {
                                self.handle_mouse_event(mouse);
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

        let mut templates = Vec::new();
        if let Some(cfg) = &self.config {
            templates = cfg.templates.keys().cloned().collect();
        }
        templates.sort();

        let active_tpl = self
            .registry
            .active_session()
            .map(|s| s.template_name().to_string());

        let running = self.running_sessions_map();
        let launcher = QuickLauncherState::new(templates, local_projects, active_tpl.as_deref())
            .with_running_sessions(running);
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
                    template,
                    project,
                    prompt,
                } => {
                    self.quick_launcher = None;
                    self.start_project_creation(&template, &project, prompt.as_deref());
                }
                QuickLauncherAction::Attach(project) => {
                    if let Some(pos) = self
                        .registry
                        .list()
                        .iter()
                        .position(|s| s.project_name() == project)
                    {
                        let _ = self.registry.set_active_index(pos);
                        self.showing_dashboard = false;
                    } else if self.attach_project(&project).is_ok() {
                        let new_idx = self.registry.len().saturating_sub(1);
                        let _ = self.registry.set_active_index(new_idx);
                        self.showing_dashboard = false;
                    }
                }
                QuickLauncherAction::None => {
                    self.quick_launcher = Some(launcher);
                }
            }
        }
    }

    /// Dispatches a mouse event to the quick launcher and handles any triggered action.
    pub fn handle_launcher_mouse(&mut self, mouse: crossterm::event::MouseEvent) {
        if let Some(mut launcher) = self.quick_launcher.take() {
            let action = launcher.handle_mouse(mouse);
            match action {
                QuickLauncherAction::Cancel => {
                    self.quick_launcher = None;
                }
                QuickLauncherAction::Create {
                    template,
                    project,
                    prompt,
                } => {
                    self.quick_launcher = None;
                    self.start_project_creation(&template, &project, prompt.as_deref());
                }
                QuickLauncherAction::Attach(project) => {
                    if let Some(pos) = self
                        .registry
                        .list()
                        .iter()
                        .position(|s| s.project_name() == project)
                    {
                        let _ = self.registry.set_active_index(pos);
                        self.showing_dashboard = false;
                    } else if self.attach_project(&project).is_ok() {
                        let new_idx = self.registry.len().saturating_sub(1);
                        let _ = self.registry.set_active_index(new_idx);
                        self.showing_dashboard = false;
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

    /// Dispatches a mouse event to the dashboard and processes the resulting `DashboardAction`.
    pub fn handle_dashboard_mouse(&mut self, mouse: crossterm::event::MouseEvent) {
        if let Some(mut dashboard) = self.dashboard.take() {
            let action = dashboard.handle_mouse(mouse);
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
                if let Some(pos) = self
                    .registry
                    .list()
                    .iter()
                    .position(|s| s.project_name() == project)
                {
                    let _ = self.registry.set_active_index(pos);
                    self.showing_dashboard = false;
                } else if self.attach_project(&project).is_ok() {
                    let new_idx = self.registry.len().saturating_sub(1);
                    let _ = self.registry.set_active_index(new_idx);
                    self.showing_dashboard = false;
                }
            }
            DashboardAction::CreateProject {
                template,
                project,
                prompt,
            } => {
                self.start_project_creation(&template, &project, prompt.as_deref());
            }
            DashboardAction::Destroy(project) => {
                if let Some(session) = self.registry.get_by_project(&project) {
                    let id = session.id().to_string();
                    let _ = self.registry.remove(&id);
                }
                let _ = self.destroy_project(&project);
            }
            DashboardAction::None => {}
        }
    }

    /// Polls setup modal events (if any) and adds newly created project sessions to registry upon completion.
    pub fn poll_setup_progress(&mut self) {
        if let Some(modal) = &mut self.setup_modal {
            modal.tick();
            if let Some((ps, pty, vt)) = modal.poll_events() {
                let _ = self.registry.add(ps, pty, vt);
                let new_idx = self.registry.len().saturating_sub(1);
                let _ = self.registry.set_active_index(new_idx);
                self.setup_modal = None;
                self.showing_dashboard = false;
            }
        }
    }

    /// Handles keyboard events when the setup modal is active.
    pub fn handle_setup_modal_key(&mut self, key: crossterm::event::KeyEvent) {
        if let Some(modal) = &mut self.setup_modal {
            if modal.handle_key(key) {
                self.setup_modal = None;
                if self.is_showing_dashboard() {
                    self.refresh_dashboard();
                }
            }
        }
    }

    /// Handles mouse events when the setup modal is active.
    pub fn handle_setup_modal_mouse(&mut self, mouse: crossterm::event::MouseEvent) {
        if let Some(modal) = &mut self.setup_modal {
            let _ = modal.handle_mouse(mouse);
        }
    }

    /// Starts asynchronous project creation on a background worker thread and activates `setup_modal`.
    pub fn start_project_creation(
        &mut self,
        template_name: &str,
        project_name: &str,
        prompt: Option<&str>,
    ) {
        let (tx, rx) = std::sync::mpsc::channel();
        let modal = SetupModalState::new(
            project_name,
            template_name,
            prompt.map(String::from),
            rx,
        );
        self.setup_modal = Some(modal);

        if self.registry.get_by_project(project_name).is_some() {
            let _ = tx.send(SetupProgressEvent::Error(format!(
                "Project '{}' already has an active session in registry",
                project_name
            )));
            if let Some(modal) = &mut self.setup_modal {
                let _ = modal.poll_events();
            }
            return;
        }

        let cwd = self
            .cwd
            .clone()
            .or_else(|| env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."));
        let config = self.config.clone();
        let viewport_size = self.viewport_size.unwrap_or((100, 30));
        let tpl_name = template_name.to_string();
        let proj_name = project_name.to_string();
        let prompt_owned = prompt.map(String::from);

        std::thread::spawn(move || {
            crate::tui::setup_modal::run_setup_worker(
                tpl_name,
                proj_name,
                prompt_owned,
                config,
                cwd,
                viewport_size,
                tx,
            );
        });
    }


    /// Attaches to an existing project directory and adds its session to the registry.
    pub fn attach_project(&mut self, project_name: &str) -> anyhow::Result<()> {
        if let Some(pos) = self
            .registry
            .list()
            .iter()
            .position(|s| s.project_name() == project_name)
        {
            let _ = self.registry.set_active_index(pos);
            return Ok(());
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
        template_name: &str,
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

        let tpl = config.and_then(|c| c.get_template(template_name));
        let agent_name = tpl
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
            if let Ok(resolved_repos) = cfg.resolve_template_repos(template_name) {
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
            template_name: template_name.to_string(),
            description: tpl.and_then(|w| w.description.clone()),
            agents_md_append: tpl.and_then(|w| w.agents_md_append.clone()),
            repos: repo_mappings,
            env: tpl.map(|w| w.env.clone()).unwrap_or_default(),
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

        if let Some(tpl) = tpl {
            for (k, v) in &tpl.env {
                pty_config = pty_config.with_env(k, v);
            }
        }

        let pty = PtySession::spawn(pty_config)?;
        let vt = VtScreen::new(cols, rows);
        let mut ps = ProjectSession::new(project_name, template_name, agent_name);
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
                        let _ = remove_worktree_auto(&child_path, true);
                    }
                }
            }
            let _ = fs::remove_dir_all(&project_dir);
        }

        self.refresh_dashboard();
        Ok(())
    }

    /// Refreshes the local projects and templates list within the dashboard.
    pub fn refresh_dashboard(&mut self) {
        if self.dashboard.is_some() {
            let cwd = self
                .cwd
                .clone()
                .or_else(|| env::current_dir().ok())
                .unwrap_or_else(|| PathBuf::from("."));
            let running = self.running_sessions_map();
            let new_dashboard = if let Some(cfg) = &self.config {
                DashboardState::from_env(cfg, &cwd).with_running_sessions(running)
            } else {
                let default_cfg =
                    Config::load_from_str("default_agent = \"claude\"\nrepos_dir = \".botai_repos\"\n")
                        .unwrap_or_else(|_| Config {
                            default_agent: "claude".to_string(),
                            theme: "tokyo-night".to_string(),
                            repos_dir: PathBuf::from(".botai_repos"),
                            agents: HashMap::new(),
                            templates: HashMap::new(),
                        });
                DashboardState::from_env(&default_cfg, &cwd).with_running_sessions(running)
            };
            self.dashboard = Some(new_dashboard);
        }
    }

    /// Dispatches key actions based on leader hotkeys or PTY passthrough.
    pub fn handle_key_event(&mut self, key: crossterm::event::KeyEvent) {
        let is_active_errored = self
            .registry
            .active_session()
            .map(|s| s.status() == SessionStatus::Error)
            .unwrap_or(false);

        // If the active session is errored, dismiss it on Enter or plain Space (when leader is inactive)
        if is_active_errored && !self.leader_active {
            let is_dismiss_key = match key.code {
                crossterm::event::KeyCode::Enter | crossterm::event::KeyCode::Char(' ') => {
                    !key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL)
                        && !key.modifiers.contains(crossterm::event::KeyModifiers::ALT)
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

        let action = map_key_event(key, self.leader_active);

        match action {
            TuiAction::EnterLeader => {
                self.leader_active = true;
            }
            TuiAction::CancelLeader => {
                self.leader_active = false;
            }
            TuiAction::Quit => {
                self.leader_active = false;
                self.should_quit = true;
            }
            TuiAction::Detach => {
                self.leader_active = false;
                if self.dashboard.is_some() {
                    self.refresh_dashboard();
                    self.showing_dashboard = true;
                    self.should_quit = false;
                } else {
                    self.should_quit = true;
                }
            }
            TuiAction::SwitchSession(idx) => {
                self.leader_active = false;
                let _ = self.registry.set_active_index(idx);
            }
            TuiAction::NextSession => {
                self.leader_active = false;
                self.registry.next_session();
            }
            TuiAction::PrevSession => {
                self.leader_active = false;
                self.registry.prev_session();
            }
            TuiAction::OpenLauncher => {
                self.leader_active = false;
                self.open_quick_launcher();
            }
            TuiAction::CloseSession => {
                self.leader_active = false;
                if let Some(active) = self.registry.active_session() {
                    let id = active.id().to_string();
                    self.close_session(&id);
                }
            }
            TuiAction::Passthrough(bytes) => {
                self.leader_active = false;
                if let Some(active) = self.registry.active_session_mut() {
                    let _ = active.pty_mut().write_all(&bytes);
                }
            }
            TuiAction::Resize(..) | TuiAction::Tick => {}
        }
    }

    /// Forwards mouse scroll events to the active session's PTY as ANSI escape sequences.
    pub fn handle_mouse_event(&mut self, mouse: crossterm::event::MouseEvent) {
        if self.leader_active {
            return;
        }
        let bytes = crate::tui::events::mouse_event_to_bytes(&mouse);
        if !bytes.is_empty() {
            if let Some(active) = self.registry.active_session_mut() {
                if active.pty_mut().write_all(&bytes).is_ok() {
                    let new_status = self.tracker.record_activity();
                    active.project_session.set_status(new_status);
                }
            }
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
        assert!(!app.leader_active);

        // Enter leader mode via Ctrl+Space
        let ctrl_space = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char(' '),
            crossterm::event::KeyModifiers::CONTROL,
        );
        app.handle_key_event(ctrl_space);
        assert!(app.leader_active);
        assert!(!app.should_quit);

        // Press 'q' while leader is active
        let quit_key = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('q'),
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key_event(quit_key);
        assert!(app.should_quit);
        assert!(!app.leader_active);
    }

    #[test]
    fn test_tui_app_leader_cancellation_and_passthrough() {
        let registry = SessionRegistry::new();
        let mut app = TuiApp::new(registry);
        assert!(!app.leader_active);

        // 1. Enter leader mode
        let ctrl_space = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char(' '),
            crossterm::event::KeyModifiers::CONTROL,
        );
        app.handle_key_event(ctrl_space);
        assert!(app.leader_active);

        // 2. Esc cancels leader mode
        let esc_key = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Esc,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key_event(esc_key);
        assert!(!app.leader_active);

        // 3. Unrecognized key cancels leader mode
        app.handle_key_event(ctrl_space);
        assert!(app.leader_active);
        let z_key = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('z'),
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key_event(z_key);
        assert!(!app.leader_active);
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
    fn test_tui_app_open_launcher_on_leader_c() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let cwd = tmp.path().to_path_buf();

        let config_toml = r#"
default_agent = "mock"
repos_dir = "/tmp/repos"

[agents.mock]
command = "sh"
args = ["-c", "exit 0"]

[templates.ecommerce]
repos = []
"#;
        let config = Config::load_from_str(config_toml).unwrap();
        let registry = SessionRegistry::new();
        let mut app = TuiApp::new(registry)
            .with_config(config)
            .with_cwd(cwd);

        assert!(app.quick_launcher.is_none());

        // Send Ctrl+Space to activate leader
        let ctrl_space = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char(' '),
            crossterm::event::KeyModifiers::CONTROL,
        );
        app.handle_key_event(ctrl_space);
        assert!(app.leader_active);

        // Send 'c' key
        let c_key = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('c'),
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key_event(c_key);

        assert!(!app.leader_active);
        assert!(app.quick_launcher.is_some());
        let launcher = app.quick_launcher.as_ref().unwrap();
        assert_eq!(launcher.templates, vec!["ecommerce".to_string()]);
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

[templates.backend]
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

        // Quick launcher should close and setup_modal should open
        assert!(app.quick_launcher.is_none());
        assert!(app.setup_modal.is_some());

        // Wait for setup worker to finish
        let start = std::time::Instant::now();
        while app.setup_modal.is_some() && start.elapsed() < Duration::from_secs(5) {
            app.poll_setup_progress();
            std::thread::sleep(Duration::from_millis(20));
        }

        // Setup modal should close on success and registry should contain 1 active session
        assert!(app.setup_modal.is_none());
        assert_eq!(app.registry.len(), 1);
        let active = app.registry.active_session().unwrap();
        assert_eq!(active.project_name(), "billing-service");
        assert!(active.pty().config().working_dir.is_absolute());
        assert_eq!(
            active.pty().config().working_dir,
            cwd.join("billing-service").canonicalize().unwrap()
        );
        assert!(cwd.join("billing-service").join("AGENTS.md").is_file());

        // 4. Try creating another project with the same name -> Collision should open setup_modal with error
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
        assert!(app.quick_launcher.is_none());
        assert!(app.setup_modal.is_some(), "Setup modal must be open on error");
        assert!(
            app.setup_modal
                .as_ref()
                .unwrap()
                .error
                .is_some(),
            "Error message must be set in setup_modal on collision"
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

[templates.ws1]
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

[templates.backend]
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
        let theme = Theme::default();
        render_error_banner(Some(127), area, &mut buf, &theme);

        let bottom_row_text = (0..80)
            .map(|x| buf.cell((x, 23)).unwrap().symbol())
            .collect::<String>();

        assert!(bottom_row_text.contains("[Process exited with code 127 - Press Enter or Ctrl+Space w to dismiss]"));

        let cell = buf.cell((0, 23)).unwrap();
        assert_eq!(cell.bg, theme.error_banner_bg);
        assert_eq!(cell.fg, theme.error_banner_fg);
        assert!(cell.modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn test_render_error_banner_themed() {
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        let theme = Theme::cyberpunk();
        render_error_banner(Some(2), area, &mut buf, &theme);

        let bottom_row_text = (0..80)
            .map(|x| buf.cell((x, 23)).unwrap().symbol())
            .collect::<String>();

        assert!(bottom_row_text.contains("[Process exited with code 2 - Press Enter or Ctrl+Space w to dismiss]"));

        let cell = buf.cell((0, 23)).unwrap();
        assert_eq!(cell.bg, theme.error_banner_bg);
        assert_eq!(cell.fg, theme.error_banner_fg);
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
    fn test_session_exit_error_dismissed_with_space_and_leader_w() {
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

        // Test Ctrl+Space then 'w' dismissal
        let mut registry2 = SessionRegistry::new();
        let config2 = PtyConfig::new(cmd, tmp.path()).with_args(args);
        let pty2 = PtySession::spawn(config2).unwrap();
        let vt2 = VtScreen::new(80, 24);
        let ps2 = ProjectSession::new("proj-leader-w", "ws", "mock");
        registry2.add(ps2, pty2, vt2).unwrap();

        let mut app2 = TuiApp::new(registry2);
        std::thread::sleep(Duration::from_millis(150));
        app2.process_pty_streams();
        assert_eq!(app2.registry.len(), 1);

        let ctrl_space = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char(' '),
            crossterm::event::KeyModifiers::CONTROL,
        );
        app2.handle_key_event(ctrl_space);
        assert!(app2.leader_active);

        let w_key = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('w'),
            crossterm::event::KeyModifiers::NONE,
        );
        app2.handle_key_event(w_key);
        assert_eq!(app2.registry.len(), 0);
        assert!(app2.should_quit);
    }

    #[test]
    fn test_tui_app_detach_with_dashboard() {
        let tmp = tempfile::tempdir().expect("tempdir");
        #[cfg(windows)]
        let (cmd, args) = ("cmd.exe", vec!["/c", "ping -n 10 127.0.0.1 > nul"]);
        #[cfg(not(windows))]
        let (cmd, args) = ("sh", vec!["-c", "sleep 10"]);

        let mut registry = SessionRegistry::new();
        let config = PtyConfig::new(cmd, tmp.path()).with_args(args);
        let pty = PtySession::spawn(config).unwrap();
        let vt = VtScreen::new(80, 24);
        let ps = ProjectSession::new("running-proj", "ws", "mock");
        registry.add(ps, pty, vt).unwrap();

        let dashboard = DashboardState::new(vec!["ws".to_string()], vec![]);
        let mut app = TuiApp::new(registry).with_dashboard(dashboard);

        assert!(!app.showing_dashboard);
        assert!(!app.is_showing_dashboard());
        assert!(!app.should_quit);

        // Enter leader mode Ctrl+Space
        let ctrl_space = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char(' '),
            crossterm::event::KeyModifiers::CONTROL,
        );
        app.handle_key_event(ctrl_space);
        assert!(app.leader_active);

        // Detach key 'd'
        let d_key = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('d'),
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key_event(d_key);

        assert!(!app.leader_active);
        assert!(!app.should_quit);
        assert!(app.showing_dashboard);
        assert!(app.is_showing_dashboard());
        assert_eq!(app.registry.len(), 1);
        assert_eq!(app.registry.active_session().unwrap().project_name(), "running-proj");
    }

    #[test]
    fn test_tui_app_reattach_to_existing_session_from_dashboard() {
        let tmp = tempfile::tempdir().expect("tempdir");
        #[cfg(windows)]
        let (cmd, args) = ("cmd.exe", vec!["/c", "ping -n 10 127.0.0.1 > nul"]);
        #[cfg(not(windows))]
        let (cmd, args) = ("sh", vec!["-c", "sleep 10"]);

        let mut registry = SessionRegistry::new();
        let config1 = PtyConfig::new(cmd, tmp.path()).with_args(args.clone());
        let pty1 = PtySession::spawn(config1).unwrap();
        let vt1 = VtScreen::new(80, 24);
        let ps1 = ProjectSession::new("proj1", "ws", "mock");
        registry.add(ps1, pty1, vt1).unwrap();

        let config2 = PtyConfig::new(cmd, tmp.path()).with_args(args);
        let pty2 = PtySession::spawn(config2).unwrap();
        let vt2 = VtScreen::new(80, 24);
        let ps2 = ProjectSession::new("proj2", "ws", "mock");
        registry.add(ps2, pty2, vt2).unwrap();

        let dashboard = DashboardState::new(
            vec!["ws".to_string()],
            vec!["proj1".to_string(), "proj2".to_string()],
        );
        let mut app = TuiApp::new(registry).with_dashboard(dashboard);

        // Detach to dashboard
        app.showing_dashboard = true;
        assert!(app.is_showing_dashboard());
        assert_eq!(app.registry.active_index(), Some(0));

        // Attach "proj2" from dashboard
        app.handle_dashboard_action(DashboardAction::Attach("proj2".to_string()));

        assert!(!app.showing_dashboard);
        assert!(!app.is_showing_dashboard());
        assert_eq!(app.registry.active_index(), Some(1));
        assert_eq!(
            app.registry.active_session().unwrap().project_name(),
            "proj2"
        );
        assert_eq!(app.registry.len(), 2);
    }

    #[test]
    fn test_tui_app_detach_without_dashboard() {
        let tmp = tempfile::tempdir().expect("tempdir");
        #[cfg(windows)]
        let (cmd, args) = ("cmd.exe", vec!["/c", "ping -n 10 127.0.0.1 > nul"]);
        #[cfg(not(windows))]
        let (cmd, args) = ("sh", vec!["-c", "sleep 10"]);

        let mut registry = SessionRegistry::new();
        let config = PtyConfig::new(cmd, tmp.path()).with_args(args);
        let pty = PtySession::spawn(config).unwrap();
        let vt = VtScreen::new(80, 24);
        let ps = ProjectSession::new("running-proj", "ws", "mock");
        registry.add(ps, pty, vt).unwrap();

        let mut app = TuiApp::new(registry);
        assert!(app.dashboard.is_none());

        // Enter leader mode Ctrl+Space
        let ctrl_space = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char(' '),
            crossterm::event::KeyModifiers::CONTROL,
        );
        app.handle_key_event(ctrl_space);
        assert!(app.leader_active);

        // Detach key 'd'
        let d_key = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('d'),
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key_event(d_key);

        assert!(!app.leader_active);
        assert!(app.should_quit);
    }

    #[test]
    fn test_tui_app_destroy_running_project_from_dashboard() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let cwd = tmp.path().to_path_buf();

        let proj_dir = cwd.join("active-proj");
        fs::create_dir(&proj_dir).unwrap();
        fs::write(proj_dir.join("AGENTS.md"), "# Project").unwrap();

        #[cfg(windows)]
        let (cmd, args) = ("cmd.exe", vec!["/c", "ping -n 10 127.0.0.1 > nul"]);
        #[cfg(not(windows))]
        let (cmd, args) = ("sh", vec!["-c", "sleep 10"]);

        let mut registry = SessionRegistry::new();
        let config = PtyConfig::new(cmd, &proj_dir).with_args(args);
        let pty = PtySession::spawn(config).unwrap();
        let vt = VtScreen::new(80, 24);
        let ps = ProjectSession::new("active-proj", "ws", "mock");
        registry.add(ps, pty, vt).unwrap();

        let dashboard = DashboardState::new(
            vec!["ws".to_string()],
            vec!["active-proj".to_string()],
        );
        let mut app = TuiApp::new(registry)
            .with_dashboard(dashboard)
            .with_cwd(cwd.clone());

        assert_eq!(app.registry.len(), 1);
        assert!(proj_dir.exists());

        // Destroy the project from dashboard
        app.handle_dashboard_action(DashboardAction::Destroy("active-proj".to_string()));

        // Session must be removed from registry
        assert_eq!(app.registry.len(), 0);
        assert!(app.registry.get_by_project("active-proj").is_none());

        // Directory must be removed
        assert!(!proj_dir.exists());
    }

    #[test]
    fn test_tui_app_start_project_creation_success() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let cwd = tmp.path().to_path_buf();

        #[cfg(windows)]
        let shell_cmd = "cmd.exe";
        #[cfg(not(windows))]
        let shell_cmd = "sh";

        let config_toml = format!(
            r#"
default_agent = "mock"
repos_dir = "{}/repos"

[agents.mock]
command = "{}"
args = []

[templates.test_ws]
repos = []
"#,
            cwd.display().to_string().replace('\\', "/"),
            shell_cmd
        );
        let config = Config::load_from_str(&config_toml).unwrap();

        let registry = SessionRegistry::new();
        let mut app = TuiApp::new(registry)
            .with_config(config)
            .with_cwd(cwd.clone());

        app.start_project_creation("test_ws", "async-proj", Some("Initial prompt"));
        assert!(app.setup_modal.is_some());

        // Wait up to 5 seconds for worker thread to finish
        let start = std::time::Instant::now();
        while app.setup_modal.is_some() && start.elapsed() < Duration::from_secs(5) {
            app.poll_setup_progress();
            std::thread::sleep(Duration::from_millis(20));
        }

        assert!(
            app.setup_modal.is_none(),
            "Setup modal should close upon successful completion"
        );
        assert_eq!(app.registry.len(), 1);
        let active = app.registry.active_session().unwrap();
        assert_eq!(active.project_name(), "async-proj");
        assert_eq!(active.template_name(), "test_ws");
        assert!(cwd.join("async-proj").join("AGENTS.md").exists());
    }

    #[test]
    fn test_tui_app_start_project_creation_duplicate_error_retention_and_close() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let cwd = tmp.path().to_path_buf();

        #[cfg(windows)]
        let (cmd, args) = ("cmd.exe", vec!["/c", "ping -n 10 127.0.0.1 > nul"]);
        #[cfg(not(windows))]
        let (cmd, args) = ("sh", vec!["-c", "sleep 10"]);

        let mut registry = SessionRegistry::new();
        let config = PtyConfig::new(cmd, &cwd).with_args(args);
        let pty = PtySession::spawn(config).unwrap();
        let vt = VtScreen::new(80, 24);
        let ps = ProjectSession::new("active-proj", "ws", "mock");
        registry.add(ps, pty, vt).unwrap();

        let mut app = TuiApp::new(registry).with_cwd(cwd.clone());

        // Try creating project with name that already exists in registry
        app.start_project_creation("ws", "active-proj", None);
        assert!(app.setup_modal.is_some());
        let modal = app.setup_modal.as_ref().unwrap();
        assert!(modal.error.is_some());
        assert!(modal.is_finished);

        // Key press other than Esc/Enter should keep modal open
        let dummy_key = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('a'),
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_setup_modal_key(dummy_key);
        assert!(app.setup_modal.is_some());

        // Esc key should dismiss modal
        let esc_key = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Esc,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_setup_modal_key(esc_key);
        assert!(app.setup_modal.is_none());
    }

    #[test]
    fn test_tui_app_handle_mouse_event_scroll_forwarding_to_pty() {
        use crossterm::event::{KeyModifiers, MouseEvent, MouseEventKind};

        let tmp = tempfile::tempdir().expect("tempdir");
        #[cfg(windows)]
        let (cmd, args) = ("cmd.exe", vec!["/c", "ping -n 10 127.0.0.1 > nul"]);
        #[cfg(not(windows))]
        let (cmd, args) = ("sh", vec!["-c", "sleep 10"]);

        let mut registry = SessionRegistry::new();
        let config = PtyConfig::new(cmd, tmp.path()).with_args(args);
        let pty = PtySession::spawn(config).unwrap();
        let vt = VtScreen::new(80, 24);
        let ps = ProjectSession::new("test-pty-scroll", "ws", "mock");
        registry.add(ps, pty, vt).unwrap();

        let mut app = TuiApp::new(registry);
        assert_eq!(app.registry.len(), 1);

        // 1. Send ScrollUp event
        let scroll_up = MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 10,
            row: 10,
            modifiers: KeyModifiers::empty(),
        };
        app.handle_mouse_event(scroll_up);
        assert_eq!(
            app.registry.active_session().unwrap().status(),
            SessionStatus::Busy
        );

        // 2. Send ScrollDown event
        let scroll_down = MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 10,
            row: 10,
            modifiers: KeyModifiers::empty(),
        };
        app.handle_mouse_event(scroll_down);
        assert_eq!(
            app.registry.active_session().unwrap().status(),
            SessionStatus::Busy
        );
    }

    #[test]
    fn test_tui_app_handle_mouse_event_ignored_when_leader_active() {
        use crossterm::event::{KeyModifiers, MouseEvent, MouseEventKind};

        let tmp = tempfile::tempdir().expect("tempdir");
        #[cfg(windows)]
        let (cmd, args) = ("cmd.exe", vec!["/c", "ping -n 10 127.0.0.1 > nul"]);
        #[cfg(not(windows))]
        let (cmd, args) = ("sh", vec!["-c", "sleep 10"]);

        let mut registry = SessionRegistry::new();
        let config = PtyConfig::new(cmd, tmp.path()).with_args(args);
        let pty = PtySession::spawn(config).unwrap();
        let vt = VtScreen::new(80, 24);
        let mut ps = ProjectSession::new("test-leader-scroll", "ws", "mock");
        ps.set_status(SessionStatus::WaitingForInput);
        registry.add(ps, pty, vt).unwrap();

        let mut app = TuiApp::new(registry);
        app.leader_active = true;

        let scroll_up = MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 10,
            row: 10,
            modifiers: KeyModifiers::empty(),
        };
        app.handle_mouse_event(scroll_up);

        // Status should remain WaitingForInput because leader is active
        assert_eq!(
            app.registry.active_session().unwrap().status(),
            SessionStatus::WaitingForInput
        );
    }

    #[test]
    fn test_tui_app_handle_mouse_event_ignored_non_scroll() {
        use crossterm::event::{MouseButton, KeyModifiers, MouseEvent, MouseEventKind};

        let tmp = tempfile::tempdir().expect("tempdir");
        #[cfg(windows)]
        let (cmd, args) = ("cmd.exe", vec!["/c", "ping -n 10 127.0.0.1 > nul"]);
        #[cfg(not(windows))]
        let (cmd, args) = ("sh", vec!["-c", "sleep 10"]);

        let mut registry = SessionRegistry::new();
        let config = PtyConfig::new(cmd, tmp.path()).with_args(args);
        let pty = PtySession::spawn(config).unwrap();
        let vt = VtScreen::new(80, 24);
        let mut ps = ProjectSession::new("test-non-scroll", "ws", "mock");
        ps.set_status(SessionStatus::WaitingForInput);
        registry.add(ps, pty, vt).unwrap();

        let mut app = TuiApp::new(registry);

        let click = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 10,
            row: 10,
            modifiers: KeyModifiers::empty(),
        };
        app.handle_mouse_event(click);

        assert_eq!(
            app.registry.active_session().unwrap().status(),
            SessionStatus::WaitingForInput
        );
    }

    #[test]
    fn test_tui_app_mouse_scrolling_routes_to_dashboard_and_setup_modal() {
        use crossterm::event::{KeyModifiers, MouseEvent, MouseEventKind};

        // 1. Dashboard routing
        let registry = SessionRegistry::new();
        let dashboard = DashboardState::new(
            vec!["ws1".to_string(), "ws2".to_string()],
            vec!["proj1".to_string(), "proj2".to_string()],
        );
        let mut app = TuiApp::new(registry).with_dashboard(dashboard);
        app.showing_dashboard = true;

        let scroll_down = MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 10,
            row: 10,
            modifiers: KeyModifiers::empty(),
        };
        app.handle_dashboard_mouse(scroll_down);
        assert_eq!(
            app.dashboard.as_ref().unwrap().selected_project_idx,
            1
        );

        // 2. Setup modal routing
        let (_tx, rx) = std::sync::mpsc::channel();
        let mut modal = SetupModalState::new("proj-modal", "tpl", None, rx);
        for i in 1..=10 {
            modal.logs.push(format!("log {}", i));
        }
        app.setup_modal = Some(modal);

        let scroll_up = MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 10,
            row: 10,
            modifiers: KeyModifiers::empty(),
        };
        app.handle_setup_modal_mouse(scroll_up);
        assert_eq!(
            app.setup_modal.as_ref().unwrap().scroll_offset,
            3
        );
    }
}

