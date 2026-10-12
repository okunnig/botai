use std::collections::HashMap;
use std::env;
use std::io::{self, Stdout};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossterm::{
    event::{
        DisableMouseCapture, EnableMouseCapture, Event, KeyboardEnhancementFlags,
        PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
    },
    execute,
    terminal::{
        disable_raw_mode, enable_raw_mode, supports_keyboard_enhancement, EnterAlternateScreen,
        LeaveAlternateScreen,
    },
};
use ratatui::{
    backend::CrosstermBackend,
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    Terminal,
};

use crate::config::{Config, ConfigError};
use crate::notify::NotificationService;
use crate::project::ProjectBuilder;
use crate::session::{
    ProjectSession, PtyConfig, PtySession, PtyWaker, SessionRegistry, SessionStatus, VtScreen,
};
use crate::tui::creation_modal::{CreationModalAction, ProjectCreationModal};
use crate::tui::dashboard::{
    render_dashboard, render_destroy_modal, DashboardAction, DashboardState, DestroyModalState,
};
use crate::tui::destroy_progress_modal::{
    render_destroy_progress_modal, DestroyProgressModalState,
};
use crate::tui::events::{
    is_incomplete_sgr_mouse, is_shift_enter, parse_raw_events, parse_sgr_mouse,
    translate_mouse_event, RawAction, RawLeaderDecoder,
};
use crate::tui::setup_modal::{render_setup_modal, SetupModalState, SetupProgressEvent};
use crate::tui::status_bar::StatusBar;
use crate::tui::theme::Theme;
use crate::tui::ui::{compute_layout, TerminalView, STATUS_BAR_HEIGHT};

/// Main application runner managing the TUI loop, PTY streaming, activity tracking, and rendering.
pub struct TuiApp {
    pub registry: SessionRegistry,
    pub notifier: NotificationService,
    pub should_quit: bool,
    pub tick_rate: Duration,
    pub dashboard: Option<DashboardState>,
    pub creation_modal: Option<ProjectCreationModal>,
    pub setup_modal: Option<SetupModalState>,
    pub destroy_modal: Option<DestroyModalState>,
    pub destroy_progress_modal: Option<DestroyProgressModalState>,
    pub config: Option<Config>,
    pub cwd: Option<PathBuf>,
    pub viewport_size: Option<(u16, u16)>,
    pub leader_active: bool,
    pub showing_dashboard: bool,
    pub raw_decoder: RawLeaderDecoder,
    pub pending_raw_input: Vec<u8>,
    pub config_path: Option<PathBuf>,
    pub config_last_modified: Option<std::time::SystemTime>,
    pub config_file_len: Option<u64>,
}

/// Whether botai pushed kitty keyboard flags, so exactly one matching pop is emitted.
///
/// Process-global because the panic hook is installed before any `TuiApp` exists and cannot
/// reach a guard instance.
static KEYBOARD_ENHANCEMENT_PUSHED: AtomicBool = AtomicBool::new(false);

/// Pushes kitty keyboard flags if the terminal supports the protocol.
///
/// Best effort: on Windows the push always fails, and that must not stop botai starting.
/// Must run before `run_loop` spawns the stdin reader — on unix the support query polls
/// `STDIN_FILENO`, the same descriptor that thread consumes.
fn push_keyboard_enhancement<W: std::io::Write>(writer: &mut W) {
    if !matches!(supports_keyboard_enhancement(), Ok(true)) {
        return;
    }
    if execute!(
        writer,
        PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
    )
    .is_ok()
    {
        KEYBOARD_ENHANCEMENT_PUSHED.store(true, Ordering::SeqCst);
    }
}

/// Pops the flags, once, and only if botai pushed them.
///
/// Consuming the flag keeps push and pop symmetric across the three teardown paths; popping a
/// level botai never pushed would corrupt a parent kitty-protocol application's stack.
fn pop_keyboard_enhancement<W: std::io::Write>(writer: &mut W) {
    if KEYBOARD_ENHANCEMENT_PUSHED.swap(false, Ordering::SeqCst) {
        let _ = execute!(writer, PopKeyboardEnhancementFlags);
    }
}

/// Windows console flag; not re-exported by `crossterm_winapi`.
#[cfg(windows)]
const ENABLE_VIRTUAL_TERMINAL_INPUT: u32 = 0x0200;

/// Whether botai set the bit itself, so teardown only clears what it set.
#[cfg(windows)]
static VIRTUAL_TERMINAL_INPUT_ENABLED: AtomicBool = AtomicBool::new(false);

/// Makes the console deliver key presses as VT escape sequences on stdin.
///
/// botai reads raw stdin bytes rather than `crossterm::event::read`, and crossterm never sets
/// this flag — on Windows it reads `InputRecord`s through the console API instead. Without it,
/// keys carrying no character (arrows, Home/End, Delete, PageUp/PageDown) produce no bytes at
/// all and `parse_raw_events` never sees them.
///
/// Must run *after* `EnableMouseCapture`, whose Windows path overwrites the whole console mode.
#[cfg(windows)]
fn enable_virtual_terminal_input() {
    use crossterm_winapi::{ConsoleMode, Handle};

    let Ok(handle) = Handle::current_in_handle() else {
        return;
    };
    let console_mode = ConsoleMode::from(handle);
    let Ok(mode) = console_mode.mode() else {
        return;
    };
    if mode & ENABLE_VIRTUAL_TERMINAL_INPUT != 0 {
        return;
    }
    if console_mode
        .set_mode(mode | ENABLE_VIRTUAL_TERMINAL_INPUT)
        .is_ok()
    {
        VIRTUAL_TERMINAL_INPUT_ENABLED.store(true, Ordering::SeqCst);
    }
}

/// Clears the bit, but only if botai set it.
///
/// Clears just that bit rather than restoring a saved mode, so it cannot fight with raw mode
/// or `DisableMouseCapture`, which own other bits of the same value.
#[cfg(windows)]
fn disable_virtual_terminal_input() {
    use crossterm_winapi::{ConsoleMode, Handle};

    if !VIRTUAL_TERMINAL_INPUT_ENABLED.swap(false, Ordering::SeqCst) {
        return;
    }
    let Ok(handle) = Handle::current_in_handle() else {
        return;
    };
    let console_mode = ConsoleMode::from(handle);
    let Ok(mode) = console_mode.mode() else {
        return;
    };
    let _ = console_mode.set_mode(mode & !ENABLE_VIRTUAL_TERMINAL_INPUT);
}

/// No-op: unix terminals already deliver key presses as escape sequences on stdin.
#[cfg(not(windows))]
fn enable_virtual_terminal_input() {}

/// No-op counterpart to [`enable_virtual_terminal_input`].
#[cfg(not(windows))]
fn disable_virtual_terminal_input() {}

struct TerminalCleanupGuard {
    in_tmux: bool,
}

impl TerminalCleanupGuard {
    pub fn emit_tmux_modify_other_keys<W: std::io::Write>(
        writer: &mut W,
        enable: bool,
    ) -> std::io::Result<()> {
        if enable {
            writer.write_all(b"\x1b[>4;2m")?;
        } else {
            writer.write_all(b"\x1b[>4;0m")?;
        }
        writer.flush()
    }
}

impl Drop for TerminalCleanupGuard {
    fn drop(&mut self) {
        let mut out = io::stdout();
        if self.in_tmux {
            let _ = Self::emit_tmux_modify_other_keys(&mut out, false);
        }
        // Own `execute!`, so a failing pop cannot short-circuit the restores below.
        pop_keyboard_enhancement(&mut out);
        disable_virtual_terminal_input();
        let _ = execute!(
            out,
            DisableMouseCapture,
            LeaveAlternateScreen,
            crossterm::cursor::Show
        );
        // Last: crossterm's Windows DisableMouseCapture restores the console mode saved
        // during EnableMouseCapture, which was captured after raw mode was enabled.
        let _ = disable_raw_mode();
    }
}

/// Installs a panic hook that restores the host terminal to a clean, usable state
/// (disabling raw mode, resetting tmux modifyOtherKeys, leaving alternate screen,
/// disabling mouse capture, popping keyboard enhancement flags, and revealing the cursor)
/// before delegating to the previous panic hook.
pub fn install_panic_hook() {
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        let in_tmux = std::env::var_os("TMUX").is_some();
        let mut stdout = std::io::stdout();
        if in_tmux {
            let _ = TerminalCleanupGuard::emit_tmux_modify_other_keys(&mut stdout, false);
        }
        pop_keyboard_enhancement(&mut stdout);
        disable_virtual_terminal_input();
        let _ = crossterm::execute!(
            stdout,
            crossterm::event::DisableMouseCapture,
            crossterm::terminal::LeaveAlternateScreen,
            crossterm::cursor::Show
        );
        // Last: see TerminalCleanupGuard::drop.
        let _ = crossterm::terminal::disable_raw_mode();
        previous_hook(panic_info);
    }));
}

/// Renders an error / crash banner at the bottom line of the given viewport area.
pub fn render_error_banner(exit_code: Option<u32>, area: Rect, buf: &mut Buffer, theme: &Theme) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let code_str = exit_code
        .map(|c| c.to_string())
        .unwrap_or_else(|| "1".to_string());
    let msg =
        format!(" [Process exited with code {code_str} - Press Enter or Ctrl+Space w to dismiss] ");
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
            notifier: NotificationService::new(true, Duration::from_secs(10)),
            should_quit: false,
            tick_rate: Duration::from_millis(30),
            dashboard: None,
            creation_modal: None,
            setup_modal: None,
            destroy_modal: None,
            destroy_progress_modal: None,
            config: None,
            cwd: None,
            viewport_size: None,
            leader_active: false,
            showing_dashboard: false,
            raw_decoder: RawLeaderDecoder::new(),
            pending_raw_input: Vec::new(),
            config_path: None,
            config_last_modified: None,
            config_file_len: None,
        }
    }

    /// Returns whether the dashboard should currently be displayed instead of an active session.
    pub fn is_showing_dashboard(&self) -> bool {
        (self.showing_dashboard || self.registry.is_empty()) && self.dashboard.is_some()
    }

    /// Sets the dashboard state on `TuiApp`.
    pub fn with_dashboard(mut self, mut dashboard: DashboardState) -> Self {
        dashboard.set_running_sessions(self.running_sessions_map());
        if dashboard.config_path.is_none() {
            dashboard.config_path = self.config_path.clone();
            dashboard.config_last_modified = self.config_last_modified;
            dashboard.config_file_len = self.config_file_len;
        } else if self.config_path.is_none() {
            self.config_path = dashboard.config_path.clone();
            self.config_last_modified = dashboard.config_last_modified;
            self.config_file_len = dashboard.config_file_len;
        }
        self.dashboard = Some(dashboard);
        self
    }

    /// Sets the creation modal state on `TuiApp`.
    pub fn with_creation_modal(mut self, creation_modal: ProjectCreationModal) -> Self {
        self.creation_modal = Some(creation_modal);
        self
    }

    /// Sets the setup modal state on `TuiApp`.
    pub fn with_setup_modal(mut self, setup_modal: SetupModalState) -> Self {
        self.setup_modal = Some(setup_modal);
        self
    }

    /// Sets the destroy modal state on `TuiApp`.
    pub fn with_destroy_modal(mut self, modal: DestroyModalState) -> Self {
        self.destroy_modal = Some(modal);
        self
    }

    /// Opens the in-session destroy project confirmation modal.
    pub fn open_destroy_modal(&mut self, project_name: impl Into<String>) {
        self.destroy_modal = Some(DestroyModalState::new(project_name));
    }

    /// Alias for `handle_input_bytes`.
    pub fn process_raw_input(&mut self, bytes: &[u8]) {
        self.handle_input_bytes(bytes);
    }

    /// Sets the destroy progress modal state on `TuiApp`.
    pub fn with_destroy_progress_modal(mut self, modal: DestroyProgressModalState) -> Self {
        self.destroy_progress_modal = Some(modal);
        self
    }

    /// Sets the configuration on `TuiApp`.
    pub fn with_config(mut self, config: Config) -> Self {
        self.config = Some(config);
        self
    }

    /// Sets the configuration file path used for reloading.
    pub fn with_config_path<P: Into<PathBuf>>(mut self, path: P) -> Self {
        let p = path.into();
        let meta = std::fs::metadata(&p).ok();
        let mtime = meta.as_ref().and_then(|m| m.modified().ok());
        let flen = meta.map(|m| m.len());
        if let Some(dashboard) = &mut self.dashboard {
            dashboard.config_path = Some(p.clone());
            dashboard.config_last_modified = mtime;
            dashboard.config_file_len = flen;
        }
        self.config_path = Some(p);
        self.config_last_modified = mtime;
        self.config_file_len = flen;
        self
    }

    /// Reloads configuration from disk if a configuration file path is known and valid.
    /// Updates `self.config` and the dashboard's configuration/templates if active.
    pub fn reload_config(&mut self) -> Result<bool, ConfigError> {
        let path_opt = self
            .config_path
            .clone()
            .or_else(|| self.dashboard.as_ref().and_then(|d| d.config_path.clone()));

        if let Some(path) = path_opt {
            if path.exists() {
                let meta = std::fs::metadata(&path)?;
                let curr_len = meta.len();
                let last = self
                    .config_last_modified
                    .or_else(|| self.dashboard.as_ref().and_then(|d| d.config_last_modified));
                let last_len = self
                    .config_file_len
                    .or_else(|| self.dashboard.as_ref().and_then(|d| d.config_file_len));
                if let (Some(last), Ok(curr)) = (last, meta.modified()) {
                    if curr == last && last_len == Some(curr_len) {
                        return Ok(false);
                    }
                }
                let curr_mtime = meta.modified().ok();
                self.config_last_modified = curr_mtime;
                self.config_file_len = Some(curr_len);
                let new_config = Config::load_from_file(&path)?;
                if let Some(dashboard) = &mut self.dashboard {
                    dashboard.config = new_config.clone();
                    let mut templates: Vec<String> = new_config.templates.keys().cloned().collect();
                    templates.sort();
                    dashboard.templates = templates;
                    dashboard.config_path = Some(path.clone());
                    dashboard.config_last_modified = curr_mtime;
                    dashboard.config_file_len = Some(curr_len);
                }
                self.config = Some(new_config);
                if self.config_path.is_none() {
                    self.config_path = Some(path);
                }
                return Ok(true);
            }
        }
        Ok(false)
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
        let in_tmux = std::env::var_os("TMUX").is_some();
        // Install the cleanup guard before any further terminal mutation: every fallible step
        // below leaves the terminal altered, so an early return has to restore it. This also
        // brings `enable_raw_mode` above under the guard.
        let _guard = TerminalCleanupGuard { in_tmux };
        if in_tmux {
            let _ = TerminalCleanupGuard::emit_tmux_modify_other_keys(stdout, true);
        }
        execute!(
            stdout,
            EnterAlternateScreen,
            EnableMouseCapture,
            crossterm::cursor::Show
        )?;
        // Separate and best effort: on Windows this command always fails, and inside the
        // `execute!` above it would abort startup entirely. Probed here, before `run_loop`
        // spawns the stdin reader thread the unix query would otherwise race.
        push_keyboard_enhancement(stdout);
        // After `EnableMouseCapture` above, whose Windows path overwrites the whole console
        // input mode. Without this the arrow keys never reach `parse_raw_events`.
        enable_virtual_terminal_input();

        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        let res = self.run_loop(&mut terminal);

        if in_tmux {
            let _ =
                TerminalCleanupGuard::emit_tmux_modify_other_keys(terminal.backend_mut(), false);
        }
        // Teardown itself is left to `_guard`, which runs on drop immediately after
        // `terminal` and does the same sequence.

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

    /// Runs the interactive event loop using a background thread reading raw stdin bytes.
    pub fn run_loop<B: ratatui::backend::Backend>(
        &mut self,
        terminal: &mut Terminal<B>,
    ) -> io::Result<()>
    where
        B::Error: std::fmt::Display,
    {
        let (stdin_tx, stdin_rx) = std::sync::mpsc::channel::<Vec<u8>>();
        let waker_tx = stdin_tx.clone();
        let waker: PtyWaker = Arc::new(move || {
            let _ = waker_tx.send(Vec::new());
        });
        self.registry.set_waker(waker);

        std::thread::spawn(move || {
            use std::io::Read;
            let mut stdin = std::io::stdin().lock();
            let mut buf = [0u8; 4096];
            loop {
                match stdin.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if stdin_tx.send(buf[..n].to_vec()).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        self.run_loop_with_receiver(terminal, &stdin_rx)
    }

    /// Runs the TUI loop reading from a receiver channel of raw stdin byte chunks.
    pub fn run_loop_with_receiver<B: ratatui::backend::Backend>(
        &mut self,
        terminal: &mut Terminal<B>,
        stdin_rx: &std::sync::mpsc::Receiver<Vec<u8>>,
    ) -> io::Result<()>
    where
        B::Error: std::fmt::Display,
    {
        let mut last_tick = Instant::now();
        let mut dirty = true;

        // Query initial terminal viewport size and synchronize sessions
        let initial_size = terminal
            .size()
            .map_err(|e| io::Error::other(e.to_string()))?;
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
            let current_size = terminal
                .size()
                .map_err(|e| io::Error::other(e.to_string()))?;
            let (_, term_area) = compute_layout(ratatui::layout::Rect::new(
                0,
                0,
                current_size.width,
                current_size.height,
            ));
            if self.viewport_size != Some((term_area.width, term_area.height)) {
                self.viewport_size = Some((term_area.width, term_area.height));
                self.resize_all_sessions(term_area.width, term_area.height);
                dirty = true;
            }

            // 1. Poll setup progress events and modal validation
            if self.poll_setup_progress() {
                dirty = true;
            }
            if self.poll_destroy_progress() {
                dirty = true;
            }
            if let Some(dashboard) = &mut self.dashboard {
                if dashboard.poll_validation() {
                    dirty = true;
                }
            }
            if let Some(modal) = &mut self.creation_modal {
                if modal.poll_validation() {
                    dirty = true;
                }
            }

            // 2. Process PTY outputs for all sessions
            if self.process_pty_streams() {
                dirty = true;
            }

            // Synchronize running sessions on dashboard
            if self.is_showing_dashboard() && dirty {
                let running = self.running_sessions_map();
                if let Some(dashboard) = &mut self.dashboard {
                    dashboard.set_running_sessions(running);
                }
            }

            // 3. Draw active frame conditionally
            if dirty {
                let theme = self.config.as_ref().map(|c| c.theme()).unwrap_or_default();
                terminal
                    .draw(|f| {
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
                                } else if !active.vt().hide_cursor()
                                    && active.vt().scrollback() == 0
                                {
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

                        if let Some(modal) = &self.creation_modal {
                            modal.render_modal(f.area(), f.buffer_mut(), &theme);
                            if let Some(pos) = modal.cursor_position(f.area()) {
                                f.set_cursor_position(pos);
                            }
                        }

                        if let Some(setup_modal) = &self.setup_modal {
                            render_setup_modal(setup_modal, f.area(), f.buffer_mut(), &theme);
                        }

                        if let Some(destroy_modal) = &self.destroy_modal {
                            render_destroy_modal(destroy_modal, f.area(), f.buffer_mut(), &theme);
                        }

                        if let Some(destroy_modal) = &self.destroy_progress_modal {
                            render_destroy_progress_modal(
                                destroy_modal,
                                f.area(),
                                f.buffer_mut(),
                                &theme,
                            );
                        }
                    })
                    .map_err(|e| io::Error::other(e.to_string()))?;

                dirty = false;
            }

            // 4. Handle raw input bytes with timeout up to tick_rate
            let timeout = self.tick_rate.saturating_sub(last_tick.elapsed());
            match stdin_rx.recv_timeout(timeout) {
                Ok(bytes) => {
                    let mut all_bytes = bytes;
                    while let Ok(more) = stdin_rx.try_recv() {
                        all_bytes.extend(more);
                    }
                    if !all_bytes.is_empty() {
                        self.handle_input_bytes(&all_bytes);
                        dirty = true;
                    }
                }
                Err(_) => {
                    if self.flush_pending_input() {
                        dirty = true;
                    }
                }
            }

            if last_tick.elapsed() >= self.tick_rate {
                last_tick = Instant::now();
            }
        }

        Ok(())
    }

    /// Routes incoming raw input bytes either to active modal/dashboard event decoders or directly to PTY passthrough/leader decoder.
    pub fn handle_input_bytes(&mut self, bytes: &[u8]) {
        if self.destroy_progress_modal.is_some() {
            let events = parse_raw_events(bytes);
            for event in events {
                match event {
                    Event::Key(key) => self.handle_destroy_progress_modal_key(key),
                    Event::Paste(_) => {}
                    Event::Mouse(mouse) => self.handle_destroy_progress_modal_mouse(mouse),
                    _ => {}
                }
            }
        } else if self.destroy_modal.is_some() {
            let events = parse_raw_events(bytes);
            for event in events {
                if let Event::Key(key) = event {
                    self.handle_destroy_modal_key(key);
                }
            }
        } else if self.setup_modal.is_some() {
            let events = parse_raw_events(bytes);
            for event in events {
                match event {
                    Event::Key(key) => self.handle_setup_modal_key(key),
                    Event::Paste(text) => self.handle_setup_modal_paste(&text),
                    Event::Mouse(mouse) => self.handle_setup_modal_mouse(mouse),
                    _ => {}
                }
            }
        } else if self.creation_modal.is_some() {
            let events = parse_raw_events(bytes);
            for event in events {
                match event {
                    Event::Key(key) => self.handle_creation_modal_key(key),
                    Event::Paste(text) => self.handle_creation_modal_paste(&text),
                    Event::Mouse(mouse) => self.handle_creation_modal_mouse(mouse),
                    _ => {}
                }
            }
        } else if self.is_showing_dashboard() {
            let events = parse_raw_events(bytes);
            for event in events {
                match event {
                    Event::Key(key) => self.handle_dashboard_key(key),
                    Event::Paste(text) => self.handle_dashboard_paste(&text),
                    Event::Mouse(mouse) => self.handle_dashboard_mouse(mouse),
                    _ => {}
                }
            }
        } else {
            self.handle_raw_bytes(bytes);
        }
    }

    /// Reads output bytes from all running PTY sessions into their respective VT screens.
    /// Returns `true` if any bytes were processed, status changed, or sessions exited (indicating UI state mutated).
    pub fn process_pty_streams(&mut self) -> bool {
        let mut changed = false;
        let now = Instant::now();
        let mut clean_exits = Vec::new();

        let active_session_id = self.registry.active_session().map(|s| s.id().to_string());
        let is_focused_mode = self.creation_modal.is_none()
            && self.setup_modal.is_none()
            && self.destroy_progress_modal.is_none()
            && self.destroy_modal.is_none()
            && !self.is_showing_dashboard();

        for session in self.registry.list_mut() {
            let session_id = session.id().to_string();
            let is_active_focused =
                is_focused_mode && active_session_id.as_deref() == Some(&session_id);
            let prev_status = session.project_session.status;

            if let Ok(bytes) = session.pty_mut().read_nonblocking() {
                if !bytes.is_empty() {
                    changed = true;
                    session.vt_mut().process(&bytes);
                    let notifs = session.tracker.process_bytes(&bytes);
                    session.project_session.set_status(session.tracker.status());

                    for mut event in notifs {
                        event.set_project_name(session.project_name());
                        if event.is_waiting() && is_active_focused {
                            // Suppress notification when user is actively focused on this session
                        } else {
                            let _ = self.notifier.notify(&event);
                        }
                    }
                }
            }

            let exit_status = session.pty_mut().try_wait().ok().flatten();
            let is_alive = exit_status.is_none();
            let exit_code = exit_status.and_then(|s| s.code.map(|c| c as i32));

            if let Some(code) = exit_code {
                if session.project_session.exit_code != Some(code as u32) {
                    session.project_session.exit_code = Some(code as u32);
                    changed = true;
                }
            }

            let (status, notif, _) = session.tracker.evaluate(is_alive, exit_code, now);
            session.project_session.set_status(status);

            if session.project_session.status != prev_status {
                changed = true;
            }

            if let Some(mut event) = notif {
                event.set_project_name(session.project_name());
                if event.is_waiting() && is_active_focused {
                    // Suppress notification when user is actively focused on this session
                } else {
                    let _ = self.notifier.notify(&event);
                }
            }

            if !is_alive {
                if exit_code == Some(0) {
                    clean_exits.push(session.id().to_string());
                } else if session.project_session.status != SessionStatus::Error {
                    session.project_session.set_status(SessionStatus::Error);
                    changed = true;
                }
            }
        }

        if !clean_exits.is_empty() {
            changed = true;
            for id in clean_exits {
                self.close_session(&id);
            }
        }

        changed
    }

    /// Returns the configured working directory, falling back to the process CWD, then `.`.
    fn effective_cwd(&self) -> PathBuf {
        self.cwd
            .clone()
            .or_else(|| env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."))
    }

    /// Opens the in-session project creation modal, reloading potentially updated configuration first.
    pub fn open_creation_modal(&mut self) {
        let _ = self.reload_config();

        let cwd = self.effective_cwd();

        let modal = if let Some(cfg) = &self.config {
            ProjectCreationModal::new(cfg, &cwd, None)
        } else {
            let fallback_cfg = Config::fallback();
            ProjectCreationModal::new(&fallback_cfg, &cwd, None)
        };
        self.creation_modal = Some(modal);
    }

    /// Dispatches a key event to the creation modal and handles any triggered action.
    pub fn handle_creation_modal_key(&mut self, key: crossterm::event::KeyEvent) {
        if let Some(mut modal) = self.creation_modal.take() {
            match modal.handle_key(key) {
                CreationModalAction::Cancel => {
                    self.creation_modal = None;
                }
                CreationModalAction::Submit {
                    template,
                    project,
                    agent_override,
                    prompt,
                    selected_repos,
                } => {
                    self.creation_modal = None;
                    self.start_project_creation(
                        &template,
                        &project,
                        agent_override.as_deref(),
                        prompt.as_deref(),
                        selected_repos,
                    );
                }
                CreationModalAction::None => {
                    self.creation_modal = Some(modal);
                }
            }
        }
    }

    /// Dispatches a mouse event to the creation modal.
    pub fn handle_creation_modal_mouse(&mut self, mouse: crossterm::event::MouseEvent) {
        if let Some(modal) = &mut self.creation_modal {
            modal.handle_mouse(mouse);
        }
    }

    /// Dispatches a key event to the dashboard and processes the resulting `DashboardAction`.
    pub fn handle_dashboard_key(&mut self, key: crossterm::event::KeyEvent) {
        self.dispatch_to_dashboard(|dashboard| dashboard.handle_key(key));
    }

    /// Dispatches a mouse event to the dashboard and processes the resulting `DashboardAction`.
    pub fn handle_dashboard_mouse(&mut self, mouse: crossterm::event::MouseEvent) {
        self.dispatch_to_dashboard(|dashboard| dashboard.handle_mouse(mouse));
    }

    /// Runs `handler` on the dashboard, adopts any configuration it reloaded, then processes
    /// the resulting `DashboardAction`.
    fn dispatch_to_dashboard(
        &mut self,
        handler: impl FnOnce(&mut DashboardState) -> DashboardAction,
    ) {
        if let Some(mut dashboard) = self.dashboard.take() {
            let action = handler(&mut dashboard);
            if self.config.as_ref() != Some(&dashboard.config) {
                self.config = Some(dashboard.config.clone());
                if self.config_path.is_none() && dashboard.config_path.is_some() {
                    self.config_path = dashboard.config_path.clone();
                }
                self.config_last_modified = dashboard.config_last_modified;
                self.config_file_len = dashboard.config_file_len;
            }
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
                agent_override,
                prompt,
                selected_repos,
            } => {
                self.start_project_creation(
                    &template,
                    &project,
                    agent_override.as_deref(),
                    prompt.as_deref(),
                    selected_repos,
                );
            }
            DashboardAction::Destroy(project) => {
                self.start_project_destruction(&project);
            }
            DashboardAction::None => {}
        }
    }

    /// Polls setup modal events (if any) and adds newly created project sessions to registry upon completion.
    /// Returns `true` if setup progress, logs, or sessions changed.
    pub fn poll_setup_progress(&mut self) -> bool {
        let mut changed = false;
        if let Some(modal) = &mut self.setup_modal {
            let was_finished = modal.is_finished;
            let old_logs = modal.logs.len();
            let old_step = modal.current_step;
            modal.tick();
            if !was_finished {
                changed = true;
            }
            if let Some((ps, pty, vt)) = modal.poll_events() {
                let _ = self.registry.add(ps, pty, vt);
                let new_idx = self.registry.len().saturating_sub(1);
                let _ = self.registry.set_active_index(new_idx);
                self.setup_modal = None;
                self.showing_dashboard = false;
                changed = true;
            } else if modal.logs.len() != old_logs
                || modal.current_step != old_step
                || modal.is_finished != was_finished
            {
                changed = true;
            }
        }
        changed
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

    /// Polls destruction progress events and advances modal spinner animation.
    /// Returns `true` if destruction progress or logs changed.
    pub fn poll_destroy_progress(&mut self) -> bool {
        let mut changed = false;
        if let Some(modal) = &mut self.destroy_progress_modal {
            let was_finished = modal.is_finished;
            let old_logs = modal.logs.len();
            let old_step = modal.current_step;
            modal.tick();
            if !was_finished {
                changed = true;
            }
            modal.poll_events();
            if modal.logs.len() != old_logs
                || modal.current_step != old_step
                || modal.is_finished != was_finished
            {
                changed = true;
            }
        }
        changed
    }

    /// Handles keyboard events when the in-session destroy confirmation modal is active.
    pub fn handle_destroy_modal_key(&mut self, key: crossterm::event::KeyEvent) {
        if let Some(mut modal) = self.destroy_modal.take() {
            match modal.handle_key(key) {
                Some(DashboardAction::Destroy(project_name)) => {
                    self.destroy_modal = None;
                    self.start_project_destruction(&project_name);
                }
                Some(DashboardAction::None) => {
                    self.destroy_modal = None;
                }
                _ => {
                    self.destroy_modal = Some(modal);
                }
            }
        }
    }

    /// Handles keyboard events when the destruction progress modal is active.
    /// Dismisses the modal and refreshes dashboard on completion/error.
    pub fn handle_destroy_progress_modal_key(&mut self, key: crossterm::event::KeyEvent) {
        if let Some(modal) = &mut self.destroy_progress_modal {
            if modal.handle_key(key) {
                self.destroy_progress_modal = None;
                self.refresh_dashboard();
                if self.registry.is_empty() {
                    self.showing_dashboard = true;
                }
            }
        }
    }

    /// Handles mouse events (such as scroll wheel) when the destruction progress modal is active.
    pub fn handle_destroy_progress_modal_mouse(&mut self, mouse: crossterm::event::MouseEvent) {
        if let Some(modal) = &mut self.destroy_progress_modal {
            let _ = modal.handle_mouse(mouse);
        }
    }

    /// Starts asynchronous project destruction on a background worker thread and activates `destroy_progress_modal`.
    pub fn start_project_destruction(&mut self, project_name: &str) {
        if let Some(session) = self.registry.get_by_project(project_name) {
            let id = session.id().to_string();
            if let Ok(Some(mut entry)) = self.registry.remove(&id) {
                let _ = entry.pty.kill();
            }
        }

        let (tx, rx) = std::sync::mpsc::channel();
        let modal = DestroyProgressModalState::new(project_name, rx);
        self.destroy_progress_modal = Some(modal);

        let cwd = self.effective_cwd();
        let config = self.config.clone();
        let proj_name = project_name.to_string();

        std::thread::spawn(move || {
            crate::tui::destroy_progress_modal::run_destroy_worker(
                config, cwd, proj_name, true, tx,
            );
        });
    }

    /// Starts asynchronous project creation on a background worker thread and activates `setup_modal`.
    pub fn start_project_creation(
        &mut self,
        template_name: &str,
        project_name: &str,
        agent_override: Option<&str>,
        prompt: Option<&str>,
        selected_repos: Vec<String>,
    ) {
        let _ = self.reload_config();
        let (tx, rx) = std::sync::mpsc::channel();
        let modal = SetupModalState::new(project_name, template_name, prompt.map(String::from), rx);
        self.setup_modal = Some(modal);

        if self.registry.get_by_project(project_name).is_some() {
            let _ = tx.send(SetupProgressEvent::Error(format!(
                "Project '{project_name}' already has an active session in registry"
            )));
            if let Some(modal) = &mut self.setup_modal {
                let _ = modal.poll_events();
            }
            return;
        }

        let cwd = self.effective_cwd();
        let config = self.config.clone();
        let viewport_size = self.viewport_size.unwrap_or((100, 30));
        let tpl_name = template_name.to_string();
        let proj_name = project_name.to_string();
        let agent_owned = agent_override.map(String::from);
        let prompt_owned = prompt.map(String::from);

        std::thread::spawn(move || {
            crate::tui::setup_modal::run_setup_worker(
                tpl_name,
                proj_name,
                agent_owned,
                prompt_owned,
                config,
                cwd,
                viewport_size,
                tx,
                selected_repos,
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

        let cwd = self.effective_cwd();
        let project_dir = cwd.join(project_name);

        if !project_dir.exists() {
            anyhow::bail!("Project directory '{project_name}' does not exist in CWD");
        }
        let project_dir = dunce::canonicalize(&project_dir).unwrap_or(project_dir);

        let default_config = Config::fallback();
        let config = self.config.as_ref().unwrap_or(&default_config);

        let agent_name = config.default_agent.as_str();

        let (cmd, args, env_map) = if let Some(agent) = config.get_agent(agent_name) {
            (agent.command.clone(), agent.args.clone(), agent.env.clone())
        } else {
            (crate::session::default_shell(), Vec::new(), HashMap::new())
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
        let _ = self.reload_config();
        let config = self.config.clone().unwrap_or_else(Config::fallback);
        let cwd = self.effective_cwd();
        let (cols, rows) = self.viewport_size.unwrap_or((100, 30));

        let mut builder = ProjectBuilder::new(config, template_name, project_name)
            .with_cwd(cwd)
            .with_viewport(cols, rows);
        if let Some(p) = prompt {
            builder = builder.with_prompt(p);
        }
        let (ps, pty, vt) = builder.build(|_| {})?;
        self.registry.add(ps, pty, vt)?;
        Ok(())
    }

    /// Destroys a project directory and its worktrees, then refreshes the dashboard.
    pub fn destroy_project(&mut self, project_name: &str) -> anyhow::Result<()> {
        let cwd = self.effective_cwd();

        crate::project::destroy_project_workspace(self.config.as_ref(), &cwd, project_name, true)?;

        self.refresh_dashboard();
        Ok(())
    }

    /// Refreshes the local projects and templates list within the dashboard.
    pub fn refresh_dashboard(&mut self) {
        let _ = self.reload_config();
        let cwd = self.effective_cwd();
        let running = self.running_sessions_map();
        let mut new_dashboard = if let Some(cfg) = &self.config {
            DashboardState::from_env(cfg, &cwd).with_running_sessions(running)
        } else {
            let default_cfg = Config::fallback();
            DashboardState::from_env(&default_cfg, &cwd).with_running_sessions(running)
        };
        if let Some(ref path) = self.config_path {
            new_dashboard = new_dashboard.with_config_path(path.clone());
        }
        self.dashboard = Some(new_dashboard);
    }

    /// Dispatches a decoded `RawAction` to the multiplexer or active PTY session.
    pub fn handle_raw_action(&mut self, action: RawAction) {
        match action {
            RawAction::EnterLeader => {
                self.leader_active = true;
                self.raw_decoder.set_leader_active(true);
            }
            RawAction::CancelLeader => {
                self.leader_active = false;
                self.raw_decoder.set_leader_active(false);
            }
            RawAction::Quit => {
                self.leader_active = false;
                self.raw_decoder.set_leader_active(false);
                self.should_quit = true;
            }
            RawAction::Detach => {
                self.leader_active = false;
                self.raw_decoder.set_leader_active(false);
                if self.dashboard.is_some() {
                    self.refresh_dashboard();
                    self.showing_dashboard = true;
                    self.should_quit = false;
                } else {
                    self.should_quit = true;
                }
            }
            RawAction::SwitchSession(idx) => {
                self.leader_active = false;
                self.raw_decoder.set_leader_active(false);
                let _ = self.registry.set_active_index(idx);
            }
            RawAction::NextSession => {
                self.leader_active = false;
                self.raw_decoder.set_leader_active(false);
                self.registry.next_session();
            }
            RawAction::PrevSession => {
                self.leader_active = false;
                self.raw_decoder.set_leader_active(false);
                self.registry.prev_session();
            }
            RawAction::OpenLauncher => {
                self.leader_active = false;
                self.raw_decoder.set_leader_active(false);
                self.open_creation_modal();
            }
            RawAction::CloseSession => {
                self.leader_active = false;
                self.raw_decoder.set_leader_active(false);
                if let Some(active) = self.registry.active_session() {
                    let id = active.id().to_string();
                    self.close_session(&id);
                }
            }
            RawAction::DestroyProject => {
                self.leader_active = false;
                self.raw_decoder.set_leader_active(false);
                if let Some(active) = self.registry.active_session() {
                    let project_name = active.project_name().to_string();
                    self.open_destroy_modal(project_name);
                }
            }
            RawAction::Passthrough(bytes) => {
                self.leader_active = false;
                self.raw_decoder.set_leader_active(false);
                if let Some(active) = self.registry.active_session_mut() {
                    active.vt_mut().reset_scrollback();

                    let bytes_to_write = if is_shift_enter(&bytes) {
                        if active
                            .project_session
                            .agent_name
                            .eq_ignore_ascii_case("agy")
                        {
                            b"\x1b[13;2u".to_vec()
                        } else {
                            b"\n".to_vec()
                        }
                    } else {
                        bytes
                    };

                    if active.pty_mut().write_all(&bytes_to_write).is_ok()
                        && active.status() != SessionStatus::Error
                    {
                        let new_status = active.tracker.record_activity();
                        active.project_session.set_status(new_status);
                    }
                }
            }
        }
    }

    /// Processes a slice of non-mouse raw input bytes (errored session dismiss check + leader decoder).
    /// Returns `true` if an active errored session was dismissed.
    fn process_non_mouse_bytes(&mut self, segment: &[u8]) -> bool {
        if segment.is_empty() {
            return false;
        }

        let is_active_errored = self
            .registry
            .active_session()
            .map(|s| s.status() == SessionStatus::Error)
            .unwrap_or(false);

        // If the active session is errored, dismiss it on Enter (\r, \n) or plain Space (when leader is inactive)
        if is_active_errored && !self.leader_active {
            let is_dismiss = segment
                .iter()
                .any(|&b| b == b'\r' || b == b'\n' || b == b' ');
            if is_dismiss {
                if let Some(active) = self.registry.active_session() {
                    let id = active.id().to_string();
                    self.close_session(&id);
                    return true;
                }
            }
        }

        self.raw_decoder.set_leader_active(self.leader_active);
        let actions = self.raw_decoder.decode(segment);
        for action in actions {
            self.handle_raw_action(action);
        }
        false
    }

    /// Feeds raw stdin bytes directly into the `RawLeaderDecoder` and processes the resulting actions,
    /// demuxing SGR mouse sequences so they are dispatched to `handle_mouse_event` without dropping
    /// interleaved or trailing keystrokes, and buffering incomplete cross-chunk SGR mouse sequences.
    pub fn handle_raw_bytes(&mut self, bytes: &[u8]) {
        let mut combined = Vec::with_capacity(self.pending_raw_input.len() + bytes.len());
        combined.extend_from_slice(&self.pending_raw_input);
        combined.extend_from_slice(bytes);
        self.pending_raw_input.clear();

        let mut i = 0;
        let mut seg_start = 0;

        while i < combined.len() {
            let slice = &combined[i..];
            if slice.starts_with(b"\x1b[<") || is_incomplete_sgr_mouse(slice) {
                if let Some((consumed, mouse)) = parse_sgr_mouse(slice) {
                    // Process non-mouse key bytes accumulated before this mouse sequence
                    if seg_start < i && self.process_non_mouse_bytes(&combined[seg_start..i]) {
                        return; // Errored session dismissed
                    }
                    self.handle_mouse_event(mouse);
                    i += consumed;
                    seg_start = i;
                    continue;
                } else if is_incomplete_sgr_mouse(slice) {
                    // Incomplete SGR mouse sequence at the tail of input
                    if seg_start < i && self.process_non_mouse_bytes(&combined[seg_start..i]) {
                        return; // Errored session dismissed
                    }
                    self.pending_raw_input.extend_from_slice(slice);
                    seg_start = combined.len();
                    break;
                }
            }
            i += 1;
        }

        // Process trailing non-mouse key bytes
        if seg_start < combined.len() {
            self.process_non_mouse_bytes(&combined[seg_start..]);
        }
    }

    /// Flushes pending incomplete raw input bytes and raw escape sequences when timeout expires.
    /// Returns `true` if any pending input or actions were flushed.
    pub fn flush_pending_input(&mut self) -> bool {
        let had_pending = !self.pending_raw_input.is_empty();
        if had_pending {
            let pending = std::mem::take(&mut self.pending_raw_input);
            self.process_non_mouse_bytes(&pending);
        }
        let actions = self.raw_decoder.flush();
        let had_actions = !actions.is_empty();
        for action in actions {
            self.handle_raw_action(action);
        }
        had_pending || had_actions
    }

    /// Forwards pasted text to the creation modal if active.
    pub fn handle_creation_modal_paste(&mut self, text: &str) {
        if let Some(modal) = &mut self.creation_modal {
            modal.handle_paste(text);
        }
    }

    /// Forwards pasted text to the dashboard if active.
    pub fn handle_dashboard_paste(&mut self, text: &str) {
        if let Some(dashboard) = &mut self.dashboard {
            dashboard.handle_paste(text);
        }
    }

    /// Handles pasted text when the setup progress modal is active (no-op).
    pub fn handle_setup_modal_paste(&mut self, _text: &str) {
        // Setup progress modal does not take user text input
    }

    /// Forwards mouse events to the active session:
    /// - If child process enabled mouse tracking (e.g. SGR mode in htop/vim/claude):
    ///   - Ignores clicks on the status bar (`row < STATUS_BAR_HEIGHT`).
    ///   - Translates host terminal coordinates to child viewport coordinates (`row - STATUS_BAR_HEIGHT`).
    ///   - Checks bounds against the active VT height (`child_row < vt.rows()`).
    ///   - Formats translated event as standard SGR escape sequences and writes to PTY.
    /// - If child process did not enable mouse tracking (e.g. agy, claude, shell), scrolls the VtScreen scrollback buffer so the user can review past output.
    pub fn handle_mouse_event(&mut self, mouse: crossterm::event::MouseEvent) {
        if self.leader_active {
            return;
        }
        if let Some(active) = self.registry.active_session_mut() {
            if active.vt().is_mouse_mode_enabled() {
                if let Some(child_mouse) =
                    translate_mouse_event(mouse, STATUS_BAR_HEIGHT, active.vt().rows())
                {
                    let bytes = crate::tui::events::mouse_event_to_sgr_bytes(&child_mouse);
                    if !bytes.is_empty() && active.pty_mut().write_all(&bytes).is_ok() {
                        let new_status = active.tracker.record_activity();
                        active.project_session.set_status(new_status);
                    }
                }
            } else {
                match mouse.kind {
                    crossterm::event::MouseEventKind::ScrollUp => {
                        active.vt_mut().scroll_up(3);
                    }
                    crossterm::event::MouseEventKind::ScrollDown => {
                        active.vt_mut().scroll_down(3);
                    }
                    _ => {}
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use std::fs;

    fn toml_path(p: &std::path::Path) -> String {
        p.to_string_lossy().replace('\\', "/")
    }

    fn create_mock_git_repo(path: &std::path::Path) {
        fs::create_dir_all(path).unwrap();
        let _ = std::process::Command::new("git")
            .args(["init", "-b", "main"])
            .current_dir(path)
            .status();
        let _ = std::process::Command::new("git")
            .args(["config", "user.name", "Botai Test"])
            .current_dir(path)
            .status();
        let _ = std::process::Command::new("git")
            .args(["config", "user.email", "test@botai.dev"])
            .current_dir(path)
            .status();
        fs::write(path.join("README.md"), "# Test Repo\n").unwrap();
        let _ = std::process::Command::new("git")
            .args(["add", "README.md"])
            .current_dir(path)
            .status();
        let _ = std::process::Command::new("git")
            .args(["commit", "-m", "Initial commit"])
            .current_dir(path)
            .status();
    }

    #[test]
    fn test_tui_app_creation_and_quit() {
        let registry = SessionRegistry::new();
        let mut app = TuiApp::new(registry);
        assert!(!app.should_quit);
        assert!(!app.leader_active);

        // Enter leader mode via Ctrl+Space (0x00)
        app.handle_raw_bytes(&[0x00]);
        assert!(app.leader_active);
        assert!(!app.should_quit);

        // Press 'q' while leader is active
        app.handle_raw_bytes(b"q");
        assert!(app.should_quit);
        assert!(!app.leader_active);
    }

    #[test]
    fn test_tui_app_leader_cancellation_and_passthrough() {
        let registry = SessionRegistry::new();
        let mut app = TuiApp::new(registry);
        assert!(!app.leader_active);

        // 1. Enter leader mode
        app.handle_raw_bytes(&[0x00]);
        assert!(app.leader_active);

        // 2. Unrecognized key cancels leader mode
        app.handle_raw_bytes(b"z");
        assert!(!app.leader_active);

        // 3. Arrow / CSI escape cancels leader mode
        app.handle_raw_bytes(&[0x00]);
        assert!(app.leader_active);
        app.handle_raw_bytes(b"\x1b[A");
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
        let mut app = TuiApp::new(registry).with_config(config).with_cwd(cwd);

        assert!(app.creation_modal.is_none());

        // Send Ctrl+Space to activate leader
        app.handle_raw_bytes(&[0x00]);
        assert!(app.leader_active);

        // Send 'c' key
        app.handle_raw_bytes(b"c");

        assert!(!app.leader_active);
        assert!(app.creation_modal.is_some());
        let modal = app.creation_modal.as_ref().unwrap();
        assert_eq!(modal.step, crate::tui::CreationStep::SelectTemplate);
        assert_eq!(modal.templates, vec!["ecommerce".to_string()]);
    }

    #[test]
    fn test_tui_app_creation_modal_step_transitions_and_cancel() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let cwd = tmp.path().to_path_buf();
        let config_toml = r#"
default_agent = "mock"
repos_dir = "/tmp/repos"

[templates.tpl]
repos = []
"#;
        let config = Config::load_from_str(config_toml).unwrap();
        let registry = SessionRegistry::new();
        let mut app = TuiApp::new(registry).with_config(config).with_cwd(cwd);

        // 1. Open creation modal -> Step 1
        app.open_creation_modal();
        assert!(app.creation_modal.is_some());
        assert_eq!(
            app.creation_modal.as_ref().unwrap().step,
            crate::tui::CreationStep::SelectTemplate
        );

        // 2. Press Enter -> advances to Step 2
        let enter_key = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_creation_modal_key(enter_key);
        assert_eq!(
            app.creation_modal.as_ref().unwrap().step,
            crate::tui::CreationStep::ConfigureDetails
        );

        // 3. Press Esc in Step 2 -> returns to Step 1 without losing modal
        let esc_key = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Esc,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_creation_modal_key(esc_key);
        assert!(app.creation_modal.is_some());
        assert_eq!(
            app.creation_modal.as_ref().unwrap().step,
            crate::tui::CreationStep::SelectTemplate
        );

        // 4. Press Esc in Step 1 -> cancels and closes modal
        app.handle_creation_modal_key(esc_key);
        assert!(app.creation_modal.is_none());
    }

    #[test]
    fn test_tui_app_creation_modal_submit_lifecycle() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let cwd = tmp.path().to_path_buf();
        let repo_dir = tmp.path().join("repo_backend");
        create_mock_git_repo(&repo_dir);

        #[cfg(windows)]
        let (cmd, args) = ("cmd.exe", r#"["/c", "exit 0"]"#);
        #[cfg(not(windows))]
        let (cmd, args) = ("sh", r#"["-c", "exit 0"]"#);

        let config_toml = format!(
            r#"
default_agent = "mock"
repos_dir = "{}"

[agents.mock]
command = "{cmd}"
args = {args}

[templates.backend]
repos = ['{}']
"#,
            toml_path(tmp.path()),
            toml_path(&repo_dir)
        );
        let config = Config::load_from_str(&config_toml).unwrap();
        let registry = SessionRegistry::new();
        let mut app = TuiApp::new(registry)
            .with_config(config)
            .with_cwd(cwd.clone());

        // 1. Open creation modal
        app.open_creation_modal();
        assert!(app.creation_modal.is_some());
        assert_eq!(
            app.creation_modal.as_ref().unwrap().step,
            crate::tui::CreationStep::SelectTemplate
        );

        // 2. Advance to Step 2 with Enter
        let enter_key = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_creation_modal_key(enter_key);
        assert_eq!(
            app.creation_modal.as_ref().unwrap().step,
            crate::tui::CreationStep::ConfigureDetails
        );

        // 3. Type project name
        for c in "billing-service".chars() {
            app.handle_creation_modal_key(crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Char(c),
                crossterm::event::KeyModifiers::NONE,
            ));
        }
        assert_eq!(
            app.creation_modal.as_ref().unwrap().project_name_input,
            "billing-service"
        );

        // 4. Submit with Enter -> triggers start_project_creation and clears creation_modal
        app.handle_creation_modal_key(enter_key);
        assert!(app.creation_modal.is_none());
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
            dunce::canonicalize(cwd.join("billing-service")).unwrap()
        );
        assert!(cwd.join("billing-service").join("AGENTS.md").is_file());
    }

    #[test]
    fn test_tui_app_creation_modal_hardware_cursor_position() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let cwd = tmp.path().to_path_buf();
        let config_toml = r#"
default_agent = "mock"
repos_dir = "/tmp/repos"

[templates.ecommerce]
repos = []
"#;
        let config = Config::load_from_str(config_toml).unwrap();
        let registry = SessionRegistry::new();
        let mut app = TuiApp::new(registry).with_config(config).with_cwd(cwd);

        app.open_creation_modal();
        assert!(app.creation_modal.is_some());

        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        // Step 1: hardware cursor is positioned on search filter input
        let area = Rect::new(0, 0, 80, 24);
        let expected_pos = app.creation_modal.as_ref().unwrap().cursor_position(area);
        assert!(expected_pos.is_some());

        terminal
            .draw(|f| {
                if let Some(modal) = &app.creation_modal {
                    let theme = app.config.as_ref().map(|c| c.theme()).unwrap_or_default();
                    modal.render_modal(f.area(), f.buffer_mut(), &theme);
                    if let Some(pos) = modal.cursor_position(f.area()) {
                        f.set_cursor_position(pos);
                    }
                }
            })
            .unwrap();

        assert_eq!(terminal.get_cursor_position().ok(), expected_pos);

        // Advance to Step 2: cursor is positioned on project name input
        let enter_key = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_creation_modal_key(enter_key);

        let step2_pos = app.creation_modal.as_ref().unwrap().cursor_position(area);
        assert!(step2_pos.is_some());

        terminal
            .draw(|f| {
                if let Some(modal) = &app.creation_modal {
                    let theme = app.config.as_ref().map(|c| c.theme()).unwrap_or_default();
                    modal.render_modal(f.area(), f.buffer_mut(), &theme);
                    if let Some(pos) = modal.cursor_position(f.area()) {
                        f.set_cursor_position(pos);
                    }
                }
            })
            .unwrap();

        assert_eq!(terminal.get_cursor_position().ok(), step2_pos);
    }

    #[test]
    fn test_tui_app_creation_modal_paste_and_mouse_routing() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let cwd = tmp.path().to_path_buf();
        let config_toml = r#"
default_agent = "mock"
repos_dir = "/tmp/repos"

[templates.alpha]
repos = []

[templates.beta]
repos = []
"#;
        let config = Config::load_from_str(config_toml).unwrap();
        let registry = SessionRegistry::new();
        let mut app = TuiApp::new(registry).with_config(config).with_cwd(cwd);

        app.open_creation_modal();
        assert!(app.creation_modal.is_some());

        // Initial selected template is index 0 ("alpha")
        assert_eq!(
            app.creation_modal.as_ref().unwrap().selected_template(),
            Some("alpha")
        );

        // Route mouse scroll down event: cycles to next template ("beta")
        let scroll_down = crossterm::event::MouseEvent {
            kind: crossterm::event::MouseEventKind::ScrollDown,
            column: 10,
            row: 10,
            modifiers: crossterm::event::KeyModifiers::NONE,
        };
        app.handle_creation_modal_mouse(scroll_down);
        assert_eq!(
            app.creation_modal.as_ref().unwrap().selected_template(),
            Some("beta")
        );

        // Advance to Step 2
        let enter_key = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_creation_modal_key(enter_key);

        // Route paste event: pastes text into project_name_input
        app.handle_creation_modal_paste("my-pasted-app");
        assert_eq!(
            app.creation_modal.as_ref().unwrap().project_name_input,
            "my-pasted-app"
        );
    }

    #[test]
    fn test_tui_app_open_creation_modal_preserves_config() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let cwd = tmp.path().to_path_buf();

        let config_toml = r#"
default_agent = "custom-agent"
repos_dir = "/custom/repos/path"

[agents.custom-agent]
command = "custom-agent-bin"
args = ["--interactive"]

[repos.custom_repo]
url = "https://github.com/example/custom_repo.git"
default_branch = "feature"

[templates.custom_template]
repos = ["custom_repo"]
"#;
        let config = Config::load_from_str(config_toml).unwrap();
        let registry = SessionRegistry::new();
        let mut app = TuiApp::new(registry).with_config(config).with_cwd(cwd);

        app.open_creation_modal();
        let modal = app
            .creation_modal
            .as_ref()
            .expect("creation modal must be opened");
        let modal_config = &modal.config;

        assert_eq!(modal_config.default_agent, "custom-agent");
        assert_eq!(modal_config.repos_dir, PathBuf::from("/custom/repos/path"));
        assert!(modal_config.agents.contains_key("custom-agent"));
        assert_eq!(
            modal_config.agents["custom-agent"].command,
            "custom-agent-bin"
        );
        assert_eq!(
            modal_config.agents["custom-agent"].args,
            vec!["--interactive"]
        );
        assert!(modal_config.repos.contains_key("custom_repo"));
        assert_eq!(
            modal_config.repos["custom_repo"].url.as_deref(),
            Some("https://github.com/example/custom_repo.git")
        );
        assert!(modal_config.templates.contains_key("custom_template"));
    }

    #[test]
    fn test_tui_app_resize_all_sessions() {
        let tmp = tempfile::tempdir().expect("tempdir");
        #[cfg(windows)]
        let (cmd, args) = ("cmd.exe", vec!["/c", "exit 0"]);
        #[cfg(not(windows))]
        let (cmd, args) = ("sh", vec!["-c", "true"]);

        let mut registry = SessionRegistry::new();
        let config1 = PtyConfig::new(cmd, tmp.path())
            .with_args(args.clone())
            .with_dimensions(80, 24);
        let pty1 = PtySession::spawn(config1).unwrap();
        let vt1 = VtScreen::new(80, 24);
        let ps1 = ProjectSession::new("proj1", "ws1", "mock");
        registry.add(ps1, pty1, vt1).unwrap();

        let config2 = PtyConfig::new(cmd, tmp.path())
            .with_args(args)
            .with_dimensions(80, 24);
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
        let repo_dir = tmp.path().join("repo_backend");
        create_mock_git_repo(&repo_dir);

        #[cfg(windows)]
        let (cmd, args) = ("cmd.exe", r#"["/c", "exit 0"]"#);
        #[cfg(not(windows))]
        let (cmd, args) = ("sh", r#"["-c", "exit 0"]"#);

        let config_toml = format!(
            r#"
default_agent = "mock"
repos_dir = "{}"

[agents.mock]
command = "{cmd}"
args = {args}

[templates.backend]
repos = ['{}']
"#,
            toml_path(tmp.path()),
            toml_path(&repo_dir)
        );
        let config = Config::load_from_str(&config_toml).unwrap();
        let registry = SessionRegistry::new();
        let mut app = TuiApp::new(registry)
            .with_config(config)
            .with_cwd(cwd.clone());

        // Set custom viewport size
        app.viewport_size = Some((115, 38));

        app.create_and_start_project("backend", "resized-project", None)
            .unwrap();
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

        assert!(bottom_row_text
            .contains("[Process exited with code 127 - Press Enter or Ctrl+Space w to dismiss]"));

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

        assert!(bottom_row_text
            .contains("[Process exited with code 2 - Press Enter or Ctrl+Space w to dismiss]"));

        let cell = buf.cell((0, 23)).unwrap();
        assert_eq!(cell.bg, theme.error_banner_bg);
        assert_eq!(cell.fg, theme.error_banner_fg);
        assert!(cell.modifier.contains(Modifier::BOLD));
    }

    fn wait_for(app: &mut TuiApp, predicate: impl Fn(&TuiApp) -> bool) {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(3) {
            app.process_pty_streams();
            if predicate(app) {
                return;
            }
            std::thread::sleep(Duration::from_millis(15));
        }
        app.process_pty_streams();
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
        wait_for(&mut app, |a| a.registry.len() == 1);

        // Session 1 should be removed, leaving only session 2
        assert_eq!(app.registry.len(), 1);
        assert_eq!(app.registry.active_index(), Some(0));
        assert_eq!(
            app.registry.active_session().unwrap().project_name(),
            "proj2-running"
        );
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
        wait_for(&mut app, |a| a.registry.is_empty());

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
        wait_for(&mut app, |a| a.registry.is_empty());

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
        wait_for(&mut app, |a| {
            a.registry
                .active_session()
                .is_some_and(|s| s.status() == SessionStatus::Error)
        });

        // Errored session remains in registry with status Error and exit code 42
        assert_eq!(app.registry.len(), 1);
        let active = app.registry.active_session().unwrap();
        assert_eq!(active.status(), SessionStatus::Error);
        assert_eq!(active.project_session.exit_code, Some(42));
        assert!(!app.should_quit);

        // Pressing normal keys like 'a' does not dismiss it
        app.handle_raw_bytes(b"a");
        assert_eq!(app.registry.len(), 1);

        // Pressing Enter dismisses the errored session
        app.handle_raw_bytes(b"\r");

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
        wait_for(&mut app1, |a| {
            a.registry
                .active_session()
                .is_some_and(|s| s.status() == SessionStatus::Error)
        });
        assert_eq!(app1.registry.len(), 1);

        app1.handle_raw_bytes(b" ");
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
        wait_for(&mut app2, |a| {
            a.registry
                .active_session()
                .is_some_and(|s| s.status() == SessionStatus::Error)
        });
        assert_eq!(app2.registry.len(), 1);

        app2.handle_raw_bytes(&[0x00]);
        assert!(app2.leader_active);

        app2.handle_raw_bytes(b"w");
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
        app.handle_raw_bytes(&[0x00]);
        assert!(app.leader_active);

        // Detach key 'x'
        app.handle_raw_bytes(b"x");

        assert!(!app.leader_active);
        assert!(!app.should_quit);
        assert!(app.showing_dashboard);
        assert!(app.is_showing_dashboard());
        assert_eq!(app.registry.len(), 1);
        assert_eq!(
            app.registry.active_session().unwrap().project_name(),
            "running-proj"
        );
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
        app.handle_raw_bytes(&[0x00]);
        assert!(app.leader_active);

        // Detach key 'x'
        app.handle_raw_bytes(b"x");

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

        let dashboard =
            DashboardState::new(vec!["ws".to_string()], vec!["active-proj".to_string()]);
        let mut app = TuiApp::new(registry)
            .with_dashboard(dashboard)
            .with_cwd(cwd.clone());

        assert_eq!(app.registry.len(), 1);
        assert!(proj_dir.exists());

        // Destroy the project from dashboard
        app.handle_dashboard_action(DashboardAction::Destroy("active-proj".to_string()));

        // Session must be removed from registry immediately
        assert_eq!(app.registry.len(), 0);
        assert!(app.registry.get_by_project("active-proj").is_none());
        assert!(app.destroy_progress_modal.is_some());

        // Wait for asynchronous destruction to complete
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(5) {
            app.poll_destroy_progress();
            if let Some(modal) = &app.destroy_progress_modal {
                if modal.is_finished {
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }

        // Dismiss modal
        app.handle_destroy_progress_modal_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(app.destroy_progress_modal.is_none());

        // Directory must be removed
        assert!(!proj_dir.exists());
    }

    #[test]
    fn test_tui_app_in_session_destroy_opens_modal_and_renders() {
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

        let mut app = TuiApp::new(registry).with_cwd(cwd);
        assert!(app.destroy_modal.is_none());

        // Press Ctrl+Space (leader) then 'd'
        app.handle_raw_bytes(&[0x00]);
        assert!(app.leader_active);

        app.handle_raw_bytes(b"d");
        assert!(!app.leader_active);
        assert!(app.destroy_modal.is_some());
        assert_eq!(
            app.destroy_modal.as_ref().unwrap().project_name,
            "active-proj"
        );

        // Render with TestBackend and verify confirmation dialog is drawn
        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let theme = Theme::tokyo_night();
                if let Some(modal) = &app.destroy_modal {
                    render_destroy_modal(modal, f.area(), f.buffer_mut(), &theme);
                }
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        let rendered = buffer
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect::<String>();
        assert!(rendered.contains("Confirm Destroy Project"));
        assert!(rendered.contains("active-proj"));
    }

    #[test]
    fn test_tui_app_in_session_destroy_modal_cancel_keys() {
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

        let mut app = TuiApp::new(registry).with_cwd(cwd);

        // 1. Cancel with 'n'
        app.handle_raw_bytes(&[0x00]);
        app.handle_raw_bytes(b"d");
        assert!(app.destroy_modal.is_some());

        app.handle_input_bytes(b"n");
        assert!(app.destroy_modal.is_none());
        assert!(app.destroy_progress_modal.is_none());
        assert_eq!(app.registry.len(), 1);
        assert!(proj_dir.exists());

        // 2. Cancel with Esc (\x1b) after uppercase 'D'
        app.handle_raw_bytes(&[0x00]);
        app.handle_raw_bytes(b"D");
        assert!(app.destroy_modal.is_some());

        app.handle_input_bytes(b"\x1b");
        assert!(app.destroy_modal.is_none());
        assert!(app.destroy_progress_modal.is_none());
        assert_eq!(app.registry.len(), 1);
        assert!(proj_dir.exists());
    }

    #[test]
    fn test_tui_app_in_session_destroy_modal_confirm_triggers_destruction() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let cwd = tmp.path().to_path_buf();

        let proj_dir = cwd.join("destroy-me");
        fs::create_dir(&proj_dir).unwrap();
        fs::write(proj_dir.join("AGENTS.md"), "# Destroy Me").unwrap();

        #[cfg(windows)]
        let (cmd, args) = ("cmd.exe", vec!["/c", "ping -n 10 127.0.0.1 > nul"]);
        #[cfg(not(windows))]
        let (cmd, args) = ("sh", vec!["-c", "sleep 10"]);

        let mut registry = SessionRegistry::new();
        let config = PtyConfig::new(cmd, &proj_dir).with_args(args);
        let pty = PtySession::spawn(config).unwrap();
        let vt = VtScreen::new(80, 24);
        let ps = ProjectSession::new("destroy-me", "ws", "mock");
        registry.add(ps, pty, vt).unwrap();

        let mut app = TuiApp::new(registry).with_cwd(cwd);

        // Trigger destroy modal via Ctrl+Space then 'd'
        app.handle_raw_bytes(&[0x00]);
        app.handle_raw_bytes(b"d");
        assert!(app.destroy_modal.is_some());

        // Confirm with 'y'
        app.process_raw_input(b"y");
        assert!(app.destroy_modal.is_none());
        assert!(app.destroy_progress_modal.is_some());
        assert_eq!(app.registry.len(), 0);

        // Wait for async destruction
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(5) {
            app.poll_destroy_progress();
            if let Some(modal) = &app.destroy_progress_modal {
                if modal.is_finished {
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }

        // Dismiss modal with Enter
        app.handle_destroy_progress_modal_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(app.destroy_progress_modal.is_none());
        assert!(!proj_dir.exists());
    }

    #[test]
    fn test_tui_app_in_session_destroy_multi_session_transitions_to_remaining_session() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let cwd = tmp.path().to_path_buf();

        let dir1 = cwd.join("proj-1");
        fs::create_dir(&dir1).unwrap();
        fs::write(dir1.join("AGENTS.md"), "# Proj 1").unwrap();

        let dir2 = cwd.join("proj-2");
        fs::create_dir(&dir2).unwrap();
        fs::write(dir2.join("AGENTS.md"), "# Proj 2").unwrap();

        #[cfg(windows)]
        let (cmd, args) = ("cmd.exe", vec!["/c", "ping -n 10 127.0.0.1 > nul"]);
        #[cfg(not(windows))]
        let (cmd, args) = ("sh", vec!["-c", "sleep 10"]);

        let mut registry = SessionRegistry::new();
        let config1 = PtyConfig::new(cmd, &dir1).with_args(args.clone());
        let pty1 = PtySession::spawn(config1).unwrap();
        let vt1 = VtScreen::new(80, 24);
        let ps1 = ProjectSession::new("proj-1", "ws", "mock");
        registry.add(ps1, pty1, vt1).unwrap();

        let config2 = PtyConfig::new(cmd, &dir2).with_args(args);
        let pty2 = PtySession::spawn(config2).unwrap();
        let vt2 = VtScreen::new(80, 24);
        let ps2 = ProjectSession::new("proj-2", "ws", "mock");
        registry.add(ps2, pty2, vt2).unwrap();

        let mut app = TuiApp::new(registry).with_cwd(cwd);
        assert_eq!(app.registry.len(), 2);
        assert_eq!(app.registry.active_index(), Some(0));
        assert_eq!(
            app.registry.active_session().unwrap().project_name(),
            "proj-1"
        );

        // Destroy proj-1 from within its session
        app.handle_raw_bytes(&[0x00]);
        app.handle_raw_bytes(b"d");
        assert!(app.destroy_modal.is_some());

        // Confirm with Enter
        app.handle_input_bytes(b"\r");
        assert!(app.destroy_modal.is_none());
        assert!(app.destroy_progress_modal.is_some());

        // Registry should immediately drop to 1 session, focusing proj-2
        assert_eq!(app.registry.len(), 1);
        assert_eq!(app.registry.active_index(), Some(0));
        assert_eq!(
            app.registry.active_session().unwrap().project_name(),
            "proj-2"
        );

        // Wait for async destruction
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(5) {
            app.poll_destroy_progress();
            if let Some(modal) = &app.destroy_progress_modal {
                if modal.is_finished {
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }

        // Dismiss progress modal
        app.handle_destroy_progress_modal_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(app.destroy_progress_modal.is_none());

        // Still 1 session, proj-2 active, not on dashboard
        assert_eq!(app.registry.len(), 1);
        assert_eq!(
            app.registry.active_session().unwrap().project_name(),
            "proj-2"
        );
        assert!(!app.showing_dashboard);
        assert!(!app.is_showing_dashboard());
        assert!(!dir1.exists());
        assert!(dir2.exists());
    }

    #[test]
    fn test_tui_app_in_session_destroy_last_session_transitions_to_dashboard() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let cwd = tmp.path().to_path_buf();

        let proj_dir = cwd.join("lone-proj");
        fs::create_dir(&proj_dir).unwrap();
        fs::write(proj_dir.join("AGENTS.md"), "# Lone Project").unwrap();

        #[cfg(windows)]
        let (cmd, args) = ("cmd.exe", vec!["/c", "ping -n 10 127.0.0.1 > nul"]);
        #[cfg(not(windows))]
        let (cmd, args) = ("sh", vec!["-c", "sleep 10"]);

        let mut registry = SessionRegistry::new();
        let config = PtyConfig::new(cmd, &proj_dir).with_args(args);
        let pty = PtySession::spawn(config).unwrap();
        let vt = VtScreen::new(80, 24);
        let ps = ProjectSession::new("lone-proj", "ws", "mock");
        registry.add(ps, pty, vt).unwrap();

        let dashboard = DashboardState::new(vec!["ws".to_string()], vec!["lone-proj".to_string()]);
        let mut app = TuiApp::new(registry)
            .with_dashboard(dashboard)
            .with_cwd(cwd);

        assert!(!app.showing_dashboard);
        assert!(!app.is_showing_dashboard());

        // In-session destroy
        app.handle_raw_bytes(&[0x00]);
        app.handle_raw_bytes(b"d");
        assert!(app.destroy_modal.is_some());

        app.handle_input_bytes(b"y");
        assert!(app.destroy_modal.is_none());
        assert!(app.destroy_progress_modal.is_some());

        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(5) {
            app.poll_destroy_progress();
            if let Some(modal) = &app.destroy_progress_modal {
                if modal.is_finished {
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }

        app.handle_destroy_progress_modal_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(app.destroy_progress_modal.is_none());

        // Since registry is empty, transitions cleanly to overview dashboard
        assert_eq!(app.registry.len(), 0);
        assert!(app.showing_dashboard);
        assert!(app.is_showing_dashboard());
        assert!(!proj_dir.exists());
    }

    #[test]
    fn test_tui_app_in_session_destroy_last_session_without_dashboard_initializes_dashboard() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let cwd = tmp.path().to_path_buf();

        let proj_dir = cwd.join("standalone-proj");
        fs::create_dir(&proj_dir).unwrap();
        fs::write(proj_dir.join("AGENTS.md"), "# Standalone").unwrap();

        #[cfg(windows)]
        let (cmd, args) = ("cmd.exe", vec!["/c", "ping -n 10 127.0.0.1 > nul"]);
        #[cfg(not(windows))]
        let (cmd, args) = ("sh", vec!["-c", "sleep 10"]);

        let mut registry = SessionRegistry::new();
        let config = PtyConfig::new(cmd, &proj_dir).with_args(args);
        let pty = PtySession::spawn(config).unwrap();
        let vt = VtScreen::new(80, 24);
        let ps = ProjectSession::new("standalone-proj", "ws", "mock");
        registry.add(ps, pty, vt).unwrap();

        // Standalone mode without pre-initialized dashboard
        let mut app = TuiApp::new(registry).with_cwd(cwd);
        assert!(app.dashboard.is_none());

        app.handle_raw_bytes(&[0x00]);
        app.handle_raw_bytes(b"d");
        assert!(app.destroy_modal.is_some());

        app.handle_input_bytes(b"y");
        assert!(app.destroy_modal.is_none());
        assert!(app.destroy_progress_modal.is_some());

        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(5) {
            app.poll_destroy_progress();
            if let Some(modal) = &app.destroy_progress_modal {
                if modal.is_finished {
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }

        app.handle_destroy_progress_modal_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(app.destroy_progress_modal.is_none());

        // Transitions to overview dashboard, automatically initialized
        assert_eq!(app.registry.len(), 0);
        assert!(app.dashboard.is_some());
        assert!(app.showing_dashboard);
        assert!(app.is_showing_dashboard());
        assert!(!proj_dir.exists());
    }

    #[test]
    fn test_tui_app_start_project_creation_success() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let cwd = tmp.path().to_path_buf();
        let repo_dir = tmp.path().join("repo_test");
        create_mock_git_repo(&repo_dir);

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
repos = ['{}']
"#,
            toml_path(&cwd),
            shell_cmd,
            toml_path(&repo_dir)
        );
        let config = Config::load_from_str(&config_toml).unwrap();

        let registry = SessionRegistry::new();
        let mut app = TuiApp::new(registry)
            .with_config(config)
            .with_cwd(cwd.clone());

        app.start_project_creation(
            "test_ws",
            "async-proj",
            None,
            Some("Initial prompt"),
            vec!["repo_test".to_string()],
        );
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
    fn test_tui_app_start_project_creation_with_agent_override() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let cwd = tmp.path().to_path_buf();
        let repo_dir = tmp.path().join("repo_test");
        create_mock_git_repo(&repo_dir);

        #[cfg(windows)]
        let shell_cmd = "cmd.exe";
        #[cfg(not(windows))]
        let shell_cmd = "sh";

        let config_toml = format!(
            r#"
default_agent = "mock_default"
repos_dir = "{}/repos"

[agents.mock_default]
command = "{}"
args = []

[agents.mock_override]
command = "{}"
args = []

[templates.test_ws]
repos = ['{}']
"#,
            toml_path(&cwd),
            shell_cmd,
            shell_cmd,
            toml_path(&repo_dir)
        );
        let config = Config::load_from_str(&config_toml).unwrap();

        let registry = SessionRegistry::new();
        let mut app = TuiApp::new(registry)
            .with_config(config)
            .with_cwd(cwd.clone());

        app.start_project_creation(
            "test_ws",
            "override-proj",
            Some("mock_override"),
            Some("Task instructions"),
            vec!["repo_test".to_string()],
        );
        assert!(app.setup_modal.is_some());

        let start = std::time::Instant::now();
        while app.setup_modal.is_some() && start.elapsed() < Duration::from_secs(5) {
            app.poll_setup_progress();
            std::thread::sleep(Duration::from_millis(20));
        }

        assert!(app.setup_modal.is_none());
        assert_eq!(app.registry.len(), 1);
        let active = app.registry.active_session().unwrap();
        assert_eq!(active.project_name(), "override-proj");
        assert_eq!(active.agent_name(), "mock_override");
        assert_eq!(active.project_session.agent_name, "mock_override");
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
        app.start_project_creation("ws", "active-proj", None, None, vec![]);
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
        let mut vt = VtScreen::new(80, 24);
        for i in 1..=40 {
            vt.process(format!("line {i}\r\n").as_bytes());
        }
        let ps = ProjectSession::new("test-pty-scroll", "ws", "mock");
        registry.add(ps, pty, vt).unwrap();

        let mut app = TuiApp::new(registry);
        assert_eq!(app.registry.len(), 1);

        // 1. Without child mouse mode: ScrollUp scrolls the VtScreen scrollback buffer
        let scroll_up = MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 10,
            row: 10,
            modifiers: KeyModifiers::empty(),
        };
        app.handle_mouse_event(scroll_up);
        assert_eq!(app.registry.active_session().unwrap().vt().scrollback(), 3);

        // 2. Send ScrollDown event: decrements scrollback back to 0
        let scroll_down = MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 10,
            row: 10,
            modifiers: KeyModifiers::empty(),
        };
        app.handle_mouse_event(scroll_down);
        assert_eq!(app.registry.active_session().unwrap().vt().scrollback(), 0);

        // 3. With child mouse mode enabled: sends SGR and marks activity
        app.registry
            .active_session_mut()
            .unwrap()
            .vt_mut()
            .process(b"\x1b[?1000h");
        app.registry
            .active_session_mut()
            .unwrap()
            .project_session
            .set_status(SessionStatus::WaitingForInput);
        app.handle_mouse_event(scroll_up);
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
        use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

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
    fn test_tui_app_handle_mouse_event_child_coordinate_translation_and_status_bar_rejection() {
        use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

        let tmp = tempfile::tempdir().expect("tempdir");
        #[cfg(windows)]
        let (cmd, args) = ("cmd.exe", vec!["/c", "ping -n 10 127.0.0.1 > nul"]);
        #[cfg(not(windows))]
        let (cmd, args) = ("sh", vec!["-c", "sleep 10"]);

        let mut registry = SessionRegistry::new();
        let config = PtyConfig::new(cmd, tmp.path()).with_args(args);
        let pty = PtySession::spawn(config).unwrap();
        let mut vt = VtScreen::new(80, 24);
        vt.process(b"\x1b[?1000h"); // Enable child mouse mode
        let mut ps = ProjectSession::new("test-mouse-coords", "ws", "claude");
        ps.set_status(SessionStatus::WaitingForInput);
        registry.add(ps, pty, vt).unwrap();

        let mut app = TuiApp::new(registry);
        assert!(app
            .registry
            .active_session()
            .unwrap()
            .vt()
            .is_mouse_mode_enabled());

        // 1. Mouse event on row 0 (status bar) is ignored and not forwarded
        let click_row_0 = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 10,
            row: 0,
            modifiers: KeyModifiers::empty(),
        };
        app.handle_mouse_event(click_row_0);
        assert_eq!(
            app.registry.active_session().unwrap().status(),
            SessionStatus::WaitingForInput
        );

        // 2. Mouse event on row 1 (child row 0) is translated and forwarded
        let click_row_1 = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 10,
            row: 1,
            modifiers: KeyModifiers::empty(),
        };
        app.handle_mouse_event(click_row_1);
        assert_eq!(
            app.registry.active_session().unwrap().status(),
            SessionStatus::Busy
        );

        // 3. Mouse event on row N (e.g. row 15, child row 14) is forwarded
        app.registry
            .active_session_mut()
            .unwrap()
            .project_session
            .set_status(SessionStatus::WaitingForInput);
        let click_row_15 = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 10,
            row: 15,
            modifiers: KeyModifiers::empty(),
        };
        app.handle_mouse_event(click_row_15);
        assert_eq!(
            app.registry.active_session().unwrap().status(),
            SessionStatus::Busy
        );

        // 4. Drag event on row N is forwarded
        app.registry
            .active_session_mut()
            .unwrap()
            .project_session
            .set_status(SessionStatus::WaitingForInput);
        let drag_row_10 = MouseEvent {
            kind: MouseEventKind::Drag(MouseButton::Left),
            column: 10,
            row: 10,
            modifiers: KeyModifiers::SHIFT,
        };
        app.handle_mouse_event(drag_row_10);
        assert_eq!(
            app.registry.active_session().unwrap().status(),
            SessionStatus::Busy
        );

        // 5. Up event on row N is forwarded
        app.registry
            .active_session_mut()
            .unwrap()
            .project_session
            .set_status(SessionStatus::WaitingForInput);
        let up_row_10 = MouseEvent {
            kind: MouseEventKind::Up(MouseButton::Left),
            column: 10,
            row: 10,
            modifiers: KeyModifiers::empty(),
        };
        app.handle_mouse_event(up_row_10);
        assert_eq!(
            app.registry.active_session().unwrap().status(),
            SessionStatus::Busy
        );

        // 6. Out of bounds row (row 25 when VT height is 24) is ignored
        app.registry
            .active_session_mut()
            .unwrap()
            .project_session
            .set_status(SessionStatus::WaitingForInput);
        let out_of_bounds = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 10,
            row: 25,
            modifiers: KeyModifiers::empty(),
        };
        app.handle_mouse_event(out_of_bounds);
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
        assert_eq!(app.dashboard.as_ref().unwrap().selected_project_idx, 1);

        // 2. Setup modal routing
        let (_tx, rx) = std::sync::mpsc::channel();
        let mut modal = SetupModalState::new("proj-modal", "tpl", None, rx);
        for i in 1..=10 {
            modal.logs.push(format!("log {i}"));
        }
        app.setup_modal = Some(modal);

        let scroll_up = MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 10,
            row: 10,
            modifiers: KeyModifiers::empty(),
        };
        app.handle_setup_modal_mouse(scroll_up);
        assert_eq!(app.setup_modal.as_ref().unwrap().scroll_offset, 3);
    }

    #[test]
    fn test_tui_app_paste_event_and_scrollback_behavior() {
        let tmp = tempfile::tempdir().expect("tempdir");
        #[cfg(windows)]
        let (cmd, args) = ("cmd.exe", vec!["/c", "ping -n 10 127.0.0.1 > nul"]);
        #[cfg(not(windows))]
        let (cmd, args) = ("sh", vec!["-c", "sleep 10"]);

        let mut registry = SessionRegistry::new();
        let config = PtyConfig::new(cmd, tmp.path()).with_args(args);
        let pty = PtySession::spawn(config).unwrap();
        let mut vt = VtScreen::new(80, 5);
        for i in 1..=15 {
            vt.process(format!("line {i}\r\n").as_bytes());
        }
        let ps = ProjectSession::new("test-paste", "ws", "mock");
        registry.add(ps, pty, vt).unwrap();

        let mut app = TuiApp::new(registry);

        // 1. Scroll up using native scrollback
        let scroll_up = crossterm::event::MouseEvent {
            kind: crossterm::event::MouseEventKind::ScrollUp,
            column: 10,
            row: 2,
            modifiers: crossterm::event::KeyModifiers::empty(),
        };
        app.handle_mouse_event(scroll_up);
        assert_eq!(app.registry.active_session().unwrap().vt().scrollback(), 3);

        // 2. Typing or pasting resets scrollback back to 0
        app.handle_raw_bytes(b"echo 'pasted text'\n");
        assert_eq!(app.registry.active_session().unwrap().vt().scrollback(), 0);

        // 3. Creation modal paste
        let cfg = Config::load_from_str(
            "default_agent = \"claude\"\nrepos_dir = \".botai_repos\"\n[templates.tpl1]\nrepos = []\n",
        )
        .unwrap();
        let mut modal = ProjectCreationModal::new(&cfg, std::path::Path::new("."), None);
        modal.step = crate::tui::CreationStep::ConfigureDetails;
        modal.focus = crate::tui::CreationFocus::ProjectName;
        app.creation_modal = Some(modal);
        app.handle_creation_modal_paste("pasted-project-name");
        assert_eq!(
            app.creation_modal.as_ref().unwrap().project_name_input,
            "pasted-project-name"
        );

        // 4. Dashboard paste
        app.creation_modal = None;
        let mut dashboard = DashboardState::new(vec!["tpl1".to_string()], vec![]);
        dashboard.open_creation_modal();
        dashboard.creation_modal.as_mut().unwrap().step =
            crate::tui::CreationStep::ConfigureDetails;
        app.dashboard = Some(dashboard);
        app.handle_dashboard_paste("dash-project");
        assert_eq!(
            app.dashboard
                .as_ref()
                .unwrap()
                .creation_modal
                .as_ref()
                .unwrap()
                .project_name_input,
            "dash-project"
        );
    }

    #[test]
    fn test_tui_app_handle_raw_bytes_passthrough_to_active_pty() {
        let tmp = tempfile::tempdir().expect("tempdir");
        #[cfg(windows)]
        let (cmd, args) = ("cmd.exe", vec!["/c", "ping -n 10 127.0.0.1 > nul"]);
        #[cfg(not(windows))]
        let (cmd, args) = ("sh", vec!["-c", "sleep 10"]);

        let mut registry = SessionRegistry::new();
        let config = PtyConfig::new(cmd, tmp.path()).with_args(args);
        let pty = PtySession::spawn(config).unwrap();
        let mut vt = VtScreen::new(80, 5);
        for i in 1..=10 {
            vt.process(format!("line {i}\r\n").as_bytes());
        }
        let mut ps = ProjectSession::new("raw-pty-test", "ws", "mock");
        ps.set_status(SessionStatus::WaitingForInput);
        registry.add(ps, pty, vt).unwrap();

        let mut app = TuiApp::new(registry);
        // Scroll up first
        app.registry
            .active_session_mut()
            .unwrap()
            .vt_mut()
            .scroll_up(3);
        assert_eq!(app.registry.active_session().unwrap().vt().scrollback(), 3);

        // Feed raw bytes directly
        app.handle_raw_bytes(b"cargo check\r");
        assert_eq!(app.registry.active_session().unwrap().vt().scrollback(), 0);
        assert_eq!(
            app.registry.active_session().unwrap().status(),
            SessionStatus::Busy
        );
        assert!(!app.leader_active);
    }

    #[test]
    fn test_tui_app_handle_raw_bytes_ctrl_space_0x00_and_switch() {
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

        let mut app = TuiApp::new(registry);
        assert_eq!(app.registry.active_index(), Some(0));

        // 1. Send Ctrl+Space (0x00) -> Enter leader
        app.handle_raw_bytes(&[0x00]);
        assert!(app.leader_active);

        // 2. Send '2' -> Switch to session 2 (index 1)
        app.handle_raw_bytes(b"2");
        assert!(!app.leader_active);
        assert_eq!(app.registry.active_index(), Some(1));
        assert_eq!(
            app.registry.active_session().unwrap().project_name(),
            "proj2"
        );

        // 3. Combined chunk: 0x00 then '1'
        app.handle_raw_bytes(&[0x00, b'1']);
        assert!(!app.leader_active);
        assert_eq!(app.registry.active_index(), Some(0));
        assert_eq!(
            app.registry.active_session().unwrap().project_name(),
            "proj1"
        );
    }

    #[test]
    fn test_tui_app_handle_raw_bytes_kitty_ctrl_space_and_open_launcher() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let cwd = tmp.path().to_path_buf();

        let config_toml = r#"
default_agent = "mock"
repos_dir = "/tmp/repos"

[agents.mock]
command = "sh"
args = ["-c", "exit 0"]

[templates.demo]
repos = []
"#;
        let config = Config::load_from_str(config_toml).unwrap();
        let registry = SessionRegistry::new();
        let mut app = TuiApp::new(registry).with_config(config).with_cwd(cwd);

        assert!(app.creation_modal.is_none());

        // Send Kitty Ctrl+Space (\x1b[32;5u)
        app.handle_raw_bytes(b"\x1b[32;5u");
        assert!(app.leader_active);

        // Send 'c' to open launcher
        app.handle_raw_bytes(b"c");
        assert!(!app.leader_active);
        assert!(app.creation_modal.is_some());
    }

    #[test]
    fn test_tui_app_handle_raw_bytes_literal_ctrl_space() {
        let tmp = tempfile::tempdir().expect("tempdir");
        #[cfg(windows)]
        let (cmd, args) = ("cmd.exe", vec!["/c", "ping -n 10 127.0.0.1 > nul"]);
        #[cfg(not(windows))]
        let (cmd, args) = ("sh", vec!["-c", "sleep 10"]);

        let mut registry = SessionRegistry::new();
        let config = PtyConfig::new(cmd, tmp.path()).with_args(args);
        let pty = PtySession::spawn(config).unwrap();
        let vt = VtScreen::new(80, 24);
        let mut ps = ProjectSession::new("test-literal-null", "ws", "mock");
        ps.set_status(SessionStatus::WaitingForInput);
        registry.add(ps, pty, vt).unwrap();

        let mut app = TuiApp::new(registry);

        // Double Ctrl+Space (0x00 0x00) -> enters leader then forwards 0x00 literal
        app.handle_raw_bytes(&[0x00, 0x00]);
        assert!(!app.leader_active);
        assert_eq!(
            app.registry.active_session().unwrap().status(),
            SessionStatus::Busy
        );

        // Ctrl+Space then Space (0x00 ' ') -> enters leader then forwards 0x00 literal
        app.registry
            .active_session_mut()
            .unwrap()
            .project_session
            .set_status(SessionStatus::WaitingForInput);
        app.handle_raw_bytes(&[0x00, b' ']);
        assert!(!app.leader_active);
        assert_eq!(
            app.registry.active_session().unwrap().status(),
            SessionStatus::Busy
        );
    }

    #[test]
    fn test_tui_app_handle_raw_bytes_errored_session_dismissal() {
        let tmp = tempfile::tempdir().expect("tempdir");
        #[cfg(windows)]
        let (cmd, args) = ("cmd.exe", vec!["/c", "exit 1"]);
        #[cfg(not(windows))]
        let (cmd, args) = ("sh", vec!["-c", "exit 1"]);

        let mut registry = SessionRegistry::new();
        let config = PtyConfig::new(cmd, tmp.path()).with_args(args);
        let pty = PtySession::spawn(config).unwrap();
        let vt = VtScreen::new(80, 24);
        let ps = ProjectSession::new("proj-err", "ws", "mock");
        registry.add(ps, pty, vt).unwrap();

        let mut app = TuiApp::new(registry);
        wait_for(&mut app, |a| {
            a.registry
                .active_session()
                .is_some_and(|s| s.status() == SessionStatus::Error)
        });
        assert_eq!(app.registry.len(), 1);
        assert_eq!(
            app.registry.active_session().unwrap().status(),
            SessionStatus::Error
        );

        // Send \r to dismiss
        app.handle_raw_bytes(b"\r");
        assert_eq!(app.registry.len(), 0);
        assert!(app.should_quit);
    }

    #[test]
    fn test_tui_app_handle_raw_bytes_shift_enter_claude_and_agy() {
        let tmp = tempfile::tempdir().expect("tempdir");
        #[cfg(windows)]
        let (cmd, args) = ("cmd.exe", vec!["/c", "ping -n 10 127.0.0.1 > nul"]);
        #[cfg(not(windows))]
        let (cmd, args) = ("sh", vec!["-c", "sleep 10"]);

        let mut registry = SessionRegistry::new();

        // 1. Claude session
        let config1 = PtyConfig::new(cmd, tmp.path()).with_args(args.clone());
        let pty1 = PtySession::spawn(config1).unwrap();
        let vt1 = VtScreen::new(80, 24);
        let mut ps1 = ProjectSession::new("claude-session", "ws", "claude");
        ps1.set_status(SessionStatus::WaitingForInput);
        registry.add(ps1, pty1, vt1).unwrap();

        // 2. Agy session
        let config2 = PtyConfig::new(cmd, tmp.path()).with_args(args.clone());
        let pty2 = PtySession::spawn(config2).unwrap();
        let vt2 = VtScreen::new(80, 24);
        let mut ps2 = ProjectSession::new("agy-session", "ws", "agy");
        ps2.set_status(SessionStatus::WaitingForInput);
        registry.add(ps2, pty2, vt2).unwrap();

        // 3. Default / Other session
        let config3 = PtyConfig::new(cmd, tmp.path()).with_args(args);
        let pty3 = PtySession::spawn(config3).unwrap();
        let vt3 = VtScreen::new(80, 24);
        let mut ps3 = ProjectSession::new("other-session", "ws", "copilot");
        ps3.set_status(SessionStatus::WaitingForInput);
        registry.add(ps3, pty3, vt3).unwrap();

        let mut app = TuiApp::new(registry);

        // Active: claude -> Shift+Enter (\x1b[13;2u) sets status to Busy
        assert_eq!(app.registry.active_index(), Some(0));
        app.handle_raw_bytes(b"\x1b[13;2u");
        assert_eq!(
            app.registry.active_session().unwrap().status(),
            SessionStatus::Busy
        );

        // Shift+Enter Kitty alternate (\x1b[13:10;2u) and xterm (\x1b[27;2;13~]
        app.registry
            .active_session_mut()
            .unwrap()
            .project_session
            .set_status(SessionStatus::WaitingForInput);
        app.handle_raw_bytes(b"\x1b[13:10;2u");
        assert_eq!(
            app.registry.active_session().unwrap().status(),
            SessionStatus::Busy
        );

        app.registry
            .active_session_mut()
            .unwrap()
            .project_session
            .set_status(SessionStatus::WaitingForInput);
        app.handle_raw_bytes(b"\x1b[27;2;13~");
        assert_eq!(
            app.registry.active_session().unwrap().status(),
            SessionStatus::Busy
        );

        // Switch to agy (index 1)
        app.registry.set_active_index(1).unwrap();
        assert_eq!(
            app.registry.active_session().unwrap().project_name(),
            "agy-session"
        );
        app.handle_raw_bytes(b"\x1b[13;2u");
        assert_eq!(
            app.registry.active_session().unwrap().status(),
            SessionStatus::Busy
        );

        // Switch to other (index 2)
        app.registry.set_active_index(2).unwrap();
        assert_eq!(
            app.registry.active_session().unwrap().project_name(),
            "other-session"
        );
        app.handle_raw_bytes(b"\x1b[13;2u");
        assert_eq!(
            app.registry.active_session().unwrap().status(),
            SessionStatus::Busy
        );

        // Normal Enter (\r) unaffected
        app.registry
            .active_session_mut()
            .unwrap()
            .project_session
            .set_status(SessionStatus::WaitingForInput);
        app.handle_raw_bytes(b"\r");
        assert_eq!(
            app.registry.active_session().unwrap().status(),
            SessionStatus::Busy
        );
    }

    #[test]
    fn test_tui_app_handle_raw_bytes_sgr_mouse_scrolling() {
        let tmp = tempfile::tempdir().expect("tempdir");
        #[cfg(windows)]
        let (cmd, args) = ("cmd.exe", vec!["/c", "ping -n 10 127.0.0.1 > nul"]);
        #[cfg(not(windows))]
        let (cmd, args) = ("sh", vec!["-c", "sleep 10"]);

        let mut registry = SessionRegistry::new();

        // Session 1: mouse tracking disabled
        let config1 = PtyConfig::new(cmd, tmp.path()).with_args(args.clone());
        let pty1 = PtySession::spawn(config1).unwrap();
        let mut vt1 = VtScreen::new(80, 24);
        for i in 1..=30 {
            vt1.process(format!("line {i}\r\n").as_bytes());
        }
        let mut ps1 = ProjectSession::new("no-mouse-session", "ws", "claude");
        ps1.set_status(SessionStatus::WaitingForInput);
        registry.add(ps1, pty1, vt1).unwrap();

        // Session 2: mouse tracking enabled
        let config2 = PtyConfig::new(cmd, tmp.path()).with_args(args);
        let pty2 = PtySession::spawn(config2).unwrap();
        let mut vt2 = VtScreen::new(80, 24);
        vt2.process(b"\x1b[?1000h");
        let mut ps2 = ProjectSession::new("mouse-enabled-session", "ws", "agy");
        ps2.set_status(SessionStatus::WaitingForInput);
        registry.add(ps2, pty2, vt2).unwrap();

        let mut app = TuiApp::new(registry);
        assert_eq!(app.registry.active_index(), Some(0));

        // 1. SGR ScrollUp raw bytes (\x1b[<64;10;5M) scrolls VtScreen scrollback by 3
        app.handle_raw_bytes(b"\x1b[<64;10;5M");
        assert_eq!(app.registry.active_session().unwrap().vt().scrollback(), 3);
        // Does not mark activity / status stays WaitingForInput
        assert_eq!(
            app.registry.active_session().unwrap().status(),
            SessionStatus::WaitingForInput
        );

        // ScrollUp again -> scrollback 6
        app.handle_raw_bytes(b"\x1b[<64;10;5M");
        assert_eq!(app.registry.active_session().unwrap().vt().scrollback(), 6);

        // SGR ScrollDown raw bytes (\x1b[<65;10;5M) scrolls down by 3
        app.handle_raw_bytes(b"\x1b[<65;10;5M");
        assert_eq!(app.registry.active_session().unwrap().vt().scrollback(), 3);

        // 2. Typing any key resets scrollback to 0
        app.handle_raw_bytes(b"a");
        assert_eq!(app.registry.active_session().unwrap().vt().scrollback(), 0);

        // 3. Switch to session 2 (mouse enabled)
        app.registry.set_active_index(1).unwrap();
        assert!(app
            .registry
            .active_session()
            .unwrap()
            .vt()
            .is_mouse_mode_enabled());

        // Forward raw SGR bytes to PTY master and update activity
        app.handle_raw_bytes(b"\x1b[<64;10;5M");
        assert_eq!(
            app.registry.active_session().unwrap().status(),
            SessionStatus::Busy
        );
    }

    #[test]
    fn test_tui_app_standalone_escape_flushed_to_pty() {
        let tmp = tempfile::tempdir().expect("tempdir");
        #[cfg(windows)]
        let (cmd, args) = ("cmd.exe", vec!["/c", "ping -n 10 127.0.0.1 > nul"]);
        #[cfg(not(windows))]
        let (cmd, args) = ("sh", vec!["-c", "sleep 10"]);

        let mut registry = SessionRegistry::new();
        let config = PtyConfig::new(cmd, tmp.path()).with_args(args);
        let pty = PtySession::spawn(config).unwrap();
        let vt = VtScreen::new(80, 24);
        let mut ps = ProjectSession::new("test-escape", "ws", "claude");
        ps.set_status(SessionStatus::WaitingForInput);
        registry.add(ps, pty, vt).unwrap();

        let mut app = TuiApp::new(registry);
        assert_eq!(
            app.registry.active_session().unwrap().status(),
            SessionStatus::WaitingForInput
        );

        // 1. Feeding 0x1b does NOT get trapped in pending_raw_input
        app.handle_raw_bytes(b"\x1b");
        assert!(
            app.pending_raw_input.is_empty(),
            "0x1b must not be trapped in pending_raw_input"
        );
        assert!(
            app.raw_decoder.has_pending(),
            "0x1b should be temporarily held in raw_decoder pending Kitty Ctrl+Space disambiguation"
        );

        // Status remains WaitingForInput before flush
        assert_eq!(
            app.registry.active_session().unwrap().status(),
            SessionStatus::WaitingForInput
        );

        // 2. Calling flush_pending_input flushes the escape byte to active PTY and sets status to Busy
        app.flush_pending_input();
        assert!(!app.raw_decoder.has_pending());
        assert_eq!(
            app.registry.active_session().unwrap().status(),
            SessionStatus::Busy
        );
    }

    #[test]
    fn test_tui_app_standalone_escape_cancels_leader_on_flush() {
        let registry = SessionRegistry::new();
        let mut app = TuiApp::new(registry);

        // Enter leader mode
        app.handle_raw_bytes(&[0x00]);
        assert!(app.leader_active);
        assert!(app.raw_decoder.is_leader_active());

        // Send 0x1b (Esc) while leader is active
        app.handle_raw_bytes(b"\x1b");
        assert!(
            app.pending_raw_input.is_empty(),
            "0x1b must not be trapped in pending_raw_input"
        );
        assert!(app.raw_decoder.has_pending());
        assert!(app.leader_active);

        // Calling flush_pending_input cancels leader mode and resets leader_active to false
        app.flush_pending_input();
        assert!(!app.leader_active);
        assert!(!app.raw_decoder.is_leader_active());
        assert!(!app.raw_decoder.has_pending());
    }

    #[test]
    fn test_tui_app_pending_raw_input_flushed_on_timeout() {
        let tmp = tempfile::tempdir().expect("tempdir");
        #[cfg(windows)]
        let (cmd, args) = ("cmd.exe", vec!["/c", "ping -n 10 127.0.0.1 > nul"]);
        #[cfg(not(windows))]
        let (cmd, args) = ("sh", vec!["-c", "sleep 10"]);

        let mut registry = SessionRegistry::new();
        let config = PtyConfig::new(cmd, tmp.path()).with_args(args);
        let pty = PtySession::spawn(config).unwrap();
        let vt = VtScreen::new(80, 24);
        let mut ps = ProjectSession::new("test-flush-raw", "ws", "claude");
        ps.set_status(SessionStatus::WaitingForInput);
        registry.add(ps, pty, vt).unwrap();

        let mut app = TuiApp::new(registry);

        // Feed incomplete SGR mouse sequence
        app.handle_raw_bytes(b"\x1b[<64;10;");
        assert_eq!(app.pending_raw_input, b"\x1b[<64;10;");

        // Timeout flush drains pending_raw_input and passes it through
        app.flush_pending_input();
        assert!(app.pending_raw_input.is_empty());
        assert_eq!(
            app.registry.active_session().unwrap().status(),
            SessionStatus::Busy
        );
    }

    #[test]
    fn test_terminal_cleanup_guard_emit_tmux_modify_other_keys() {
        let mut buf = Vec::new();
        TerminalCleanupGuard::emit_tmux_modify_other_keys(&mut buf, true)
            .expect("should write enable sequence");
        assert_eq!(buf, b"\x1b[>4;2m");

        buf.clear();
        TerminalCleanupGuard::emit_tmux_modify_other_keys(&mut buf, false)
            .expect("should write disable sequence");
        assert_eq!(buf, b"\x1b[>4;0m");
    }

    #[test]
    fn test_tmux_env_detection_logic() {
        let in_tmux = std::env::var_os("TMUX").is_some();
        let expected = std::env::var_os("TMUX").is_some();
        assert_eq!(in_tmux, expected);

        let detect = |var: Option<std::ffi::OsString>| var.is_some();
        assert!(detect(Some(std::ffi::OsString::from(
            "/tmp/tmux-1000/default,1234,0"
        ))));
        assert!(detect(Some(std::ffi::OsString::from("tmux-session"))));
        assert!(!detect(None));
    }

    #[test]
    fn test_install_panic_hook() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;

        let hook_called = Arc::new(AtomicBool::new(false));
        let hook_called_clone = Arc::clone(&hook_called);

        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |_info| {
            hook_called_clone.store(true, Ordering::SeqCst);
        }));

        install_panic_hook();

        let result = std::panic::catch_unwind(|| {
            panic!("test panic hook trigger");
        });

        assert!(result.is_err());
        assert!(hook_called.load(Ordering::SeqCst));

        // Restore original hook
        std::panic::set_hook(prev);
    }

    #[test]
    fn test_tui_app_in_session_open_creation_modal_reloads_config() {
        let dir = tempfile::tempdir().unwrap();
        let config_file = dir.path().join("config.toml");
        let toml_v1 = r#"
default_agent = "claude"
repos_dir = "/tmp/repos"

[templates.t1]
repos = []
"#;
        std::fs::write(&config_file, toml_v1).unwrap();

        let config = Config::load_from_file(&config_file).unwrap();
        let registry = SessionRegistry::new();
        let mut app = TuiApp::new(registry)
            .with_config(config)
            .with_config_path(&config_file)
            .with_cwd(dir.path().to_path_buf());

        // Update config on disk while app is running
        let toml_v2 = r#"
default_agent = "claude"
repos_dir = "/tmp/repos"

[templates.t1]
repos = []

[templates.t2]
repos = []
"#;
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&config_file, toml_v2).unwrap();

        // In-session open_creation_modal should reload config and include t2
        app.open_creation_modal();
        let modal = app
            .creation_modal
            .as_ref()
            .expect("creation modal must be open");
        assert_eq!(modal.templates, vec!["t1".to_string(), "t2".to_string()]);
    }

    #[test]
    fn test_tui_app_handle_dashboard_key_syncs_reloaded_config() {
        let dir = tempfile::tempdir().unwrap();
        let config_file = dir.path().join("config.toml");
        let toml_v1 = r#"
default_agent = "claude"
repos_dir = "/tmp/repos"

[templates.initial_template]
repos = []
"#;
        std::fs::write(&config_file, toml_v1).unwrap();

        let config = Config::load_from_file(&config_file).unwrap();
        let dashboard =
            DashboardState::from_env(&config, dir.path()).with_config_path(&config_file);
        let registry = SessionRegistry::new();
        let mut app = TuiApp::new(registry)
            .with_dashboard(dashboard)
            .with_config(config)
            .with_config_path(&config_file)
            .with_cwd(dir.path().to_path_buf());

        // Add a new template to config file
        let toml_v2 = r#"
default_agent = "claude"
repos_dir = "/tmp/repos"

[templates.initial_template]
repos = []

[templates.added_template]
repos = []
"#;
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&config_file, toml_v2).unwrap();

        // Simulate pressing 'n' on dashboard
        app.handle_dashboard_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE));

        // Both dashboard and app.config must have the new template
        let dashboard = app.dashboard.as_ref().expect("dashboard must exist");
        assert_eq!(
            dashboard.templates,
            vec!["added_template".to_string(), "initial_template".to_string()]
        );
        let app_cfg = app.config.as_ref().expect("app config must exist");
        assert!(app_cfg.templates.contains_key("added_template"));
        assert!(app_cfg.templates.contains_key("initial_template"));
    }

    #[test]
    fn test_consolidated_pty_polling_process_exit() {
        let tmp = tempfile::tempdir().expect("tempdir");
        #[cfg(windows)]
        let (cmd, args) = ("cmd.exe", vec!["/c", "exit 42"]);
        #[cfg(not(windows))]
        let (cmd, args) = ("sh", vec!["-c", "exit 42"]);

        let mut registry = SessionRegistry::new();
        let config = PtyConfig::new(cmd, tmp.path()).with_args(args);
        let pty = PtySession::spawn(config).unwrap();
        let vt = VtScreen::new(80, 24);
        let ps = ProjectSession::new("test_exit", "ws", "mock");
        let session_id = ps.id.clone();
        registry.add(ps, pty, vt).unwrap();

        let mut app = TuiApp::new(registry);

        wait_for(&mut app, |a| {
            a.registry
                .get(&session_id)
                .map(|s| s.project_session.exit_code == Some(42))
                .unwrap_or(false)
        });

        let s = app.registry.get(&session_id).expect("session exists");
        assert_eq!(s.project_session.exit_code, Some(42));
        assert_eq!(s.project_session.status, SessionStatus::Error);
    }
}
