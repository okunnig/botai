use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Position, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Widget},
};

use crate::config::{Config, ConfigError};
use crate::session::SessionStatus;
use crate::tui::creation_modal::{CreationModalAction, ProjectCreationModal};
use crate::tui::status_bar::TopBar;
use crate::tui::theme::Theme;

/// Backward compatibility alias for ProjectCreationModal.
pub type CreationModalState = ProjectCreationModal;

/// Actions emitted by user interactions with the dashboard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DashboardAction {
    None,
    Attach(String),
    CreateProject {
        template: String,
        project: String,
        agent_override: Option<String>,
        prompt: Option<String>,
        selected_repos: Vec<String>,
    },
    Destroy(String),
    Quit,
}

/// State for the destroy project confirmation modal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DestroyModalState {
    pub project_name: String,
}

impl DestroyModalState {
    /// Creates a new `DestroyModalState` targeting the given project.
    pub fn new(project_name: impl Into<String>) -> Self {
        Self {
            project_name: project_name.into(),
        }
    }

    /// Handles keyboard events within the destroy confirmation modal.
    pub fn handle_key(&mut self, key: KeyEvent) -> Option<DashboardAction> {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                Some(DashboardAction::Destroy(self.project_name.clone()))
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => Some(DashboardAction::None),
            _ => None,
        }
    }
}

/// Full state for the interactive full-width project dashboard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DashboardState {
    pub templates: Vec<String>,
    pub local_projects: Vec<String>,
    pub running_sessions: HashMap<String, SessionStatus>,
    pub selected_project_idx: usize,
    pub creation_modal: Option<ProjectCreationModal>,
    pub destroy_modal: Option<DestroyModalState>,
    pub config: Config,
    pub cwd: PathBuf,
    pub config_path: Option<PathBuf>,
    pub config_last_modified: Option<std::time::SystemTime>,
    pub config_file_len: Option<u64>,
}

impl DashboardState {
    /// Creates a new `DashboardState` with given templates and local projects.
    pub fn new(templates: Vec<String>, local_projects: Vec<String>) -> Self {
        let mut config =
            Config::load_from_str("default_agent = \"claude\"\nrepos_dir = \".botai_repos\"\n")
                .unwrap_or_else(|_| Config {
                    default_agent: "claude".to_string(),
                    theme: "tokyo-night".to_string(),
                    repos_dir: PathBuf::from(".botai_repos"),
                    repos: HashMap::new(),
                    agents: HashMap::new(),
                    templates: HashMap::new(),
                });

        for tpl in &templates {
            config
                .templates
                .entry(tpl.clone())
                .or_insert_with(|| crate::config::TemplateConfig {
                    description: None,
                    default_agent: None,
                    agents_md_append: None,
                    repos: vec![format!("{tpl}-repo").into()],
                    env: HashMap::new(),
                    post_create: Vec::new(),
                    pre_destroy: Vec::new(),
                });
        }

        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

        Self {
            templates,
            local_projects,
            running_sessions: HashMap::new(),
            selected_project_idx: 0,
            creation_modal: None,
            destroy_modal: None,
            config,
            cwd,
            config_path: None,
            config_last_modified: None,
            config_file_len: None,
        }
    }

    /// Sets the map of currently running sessions.
    pub fn with_running_sessions(mut self, running: HashMap<String, SessionStatus>) -> Self {
        self.running_sessions = running;
        self
    }

    /// Sets the configuration used by the dashboard.
    pub fn with_config(mut self, config: Config) -> Self {
        self.config = config;
        self
    }

    /// Sets the configuration file path used for reloading.
    pub fn with_config_path<P: Into<PathBuf>>(mut self, path: P) -> Self {
        let p = path.into();
        let meta = std::fs::metadata(&p).ok();
        self.config_last_modified = meta.as_ref().and_then(|m| m.modified().ok());
        self.config_file_len = meta.map(|m| m.len());
        self.config_path = Some(p);
        self
    }

    /// Sets the working directory used by the dashboard.
    pub fn with_cwd(mut self, cwd: PathBuf) -> Self {
        self.cwd = cwd;
        self
    }

    /// Updates the map of currently running sessions in-place.
    pub fn set_running_sessions(&mut self, running: HashMap<String, SessionStatus>) {
        self.running_sessions = running;
    }

    /// Scans the CWD for existing project directories containing `AGENTS.md` and loads templates from `Config`.
    pub fn from_env(config: &Config, cwd: &Path) -> Self {
        let mut local_projects = Vec::new();
        if let Ok(entries) = std::fs::read_dir(cwd) {
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

        let mut templates: Vec<String> = config.templates.keys().cloned().collect();
        templates.sort();

        Self::new(templates, local_projects)
            .with_config(config.clone())
            .with_cwd(cwd.to_path_buf())
    }

    /// Reloads configuration from disk if a configuration file path is known and valid.
    /// Updates `self.config` and `self.templates` to reflect newly added or modified templates.
    pub fn reload_config(&mut self) -> Result<bool, ConfigError> {
        if let Some(ref path) = self.config_path {
            if path.exists() {
                let meta = std::fs::metadata(path)?;
                let curr_len = meta.len();
                if let (Some(last), Ok(curr)) = (self.config_last_modified, meta.modified()) {
                    if curr == last && self.config_file_len == Some(curr_len) {
                        return Ok(false);
                    }
                }
                self.config_last_modified = meta.modified().ok();
                self.config_file_len = Some(curr_len);
                self.config = Config::load_from_file(path)?;
                let mut templates: Vec<String> = self.config.templates.keys().cloned().collect();
                templates.sort();
                self.templates = templates;
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Moves selection down to the next project item.
    pub fn next_item(&mut self) {
        if !self.local_projects.is_empty()
            && self.selected_project_idx + 1 < self.local_projects.len()
        {
            self.selected_project_idx += 1;
        }
    }

    /// Moves selection up to the previous project item.
    pub fn prev_item(&mut self) {
        self.selected_project_idx = self.selected_project_idx.saturating_sub(1);
    }

    /// Moves selection to the first project item.
    pub fn first_item(&mut self) {
        self.selected_project_idx = 0;
    }

    /// Moves selection to the last project item.
    pub fn last_item(&mut self) {
        if !self.local_projects.is_empty() {
            self.selected_project_idx = self.local_projects.len().saturating_sub(1);
        }
    }

    /// Opens the project creation modal, reloading potentially updated configuration first.
    pub fn open_creation_modal(&mut self) {
        let _ = self.reload_config();
        let default_tpl = self.templates.first().map(|s| s.as_str());
        self.creation_modal = Some(ProjectCreationModal::new(
            &self.config,
            &self.cwd,
            default_tpl,
        ));
    }

    /// Closes the project creation modal.
    pub fn close_creation_modal(&mut self) {
        self.creation_modal = None;
    }

    /// Polls pending background validation checks in the creation modal, if active.
    pub fn poll_validation(&mut self) {
        if let Some(modal) = &mut self.creation_modal {
            modal.poll_validation();
        }
    }

    /// Advances tick-based state and background validation polling for active modals.
    pub fn tick(&mut self) {
        self.poll_validation();
    }

    /// Returns the hardware cursor position if a modal or input field is actively focused.
    pub fn cursor_position(&self, area: Rect) -> Option<Position> {
        if self.destroy_modal.is_some() {
            return None;
        }
        self.creation_modal
            .as_ref()
            .and_then(|modal| modal.cursor_position(area))
    }

    /// Handles a keyboard event and returns any triggered `DashboardAction`.
    pub fn handle_key(&mut self, key: KeyEvent) -> DashboardAction {
        if let Some(mut modal) = self.destroy_modal.take() {
            if let Some(action) = modal.handle_key(key) {
                return action;
            } else {
                self.destroy_modal = Some(modal);
                return DashboardAction::None;
            }
        }

        if let Some(mut modal) = self.creation_modal.take() {
            match modal.handle_key(key) {
                CreationModalAction::Cancel => {
                    self.creation_modal = None;
                    return DashboardAction::None;
                }
                CreationModalAction::Submit {
                    template,
                    project,
                    agent_override,
                    prompt,
                    selected_repos,
                } => {
                    self.creation_modal = None;
                    return DashboardAction::CreateProject {
                        template,
                        project,
                        agent_override,
                        prompt,
                        selected_repos,
                    };
                }
                CreationModalAction::None => {
                    self.creation_modal = Some(modal);
                    return DashboardAction::None;
                }
            }
        }

        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.prev_item();
                DashboardAction::None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.next_item();
                DashboardAction::None
            }
            KeyCode::Home => {
                self.first_item();
                DashboardAction::None
            }
            KeyCode::End => {
                self.last_item();
                DashboardAction::None
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Char('c') | KeyCode::Char('C') => {
                self.open_creation_modal();
                DashboardAction::None
            }
            KeyCode::Char('d') | KeyCode::Char('D') => {
                if !self.local_projects.is_empty() {
                    let idx = self
                        .selected_project_idx
                        .min(self.local_projects.len().saturating_sub(1));
                    self.destroy_modal = Some(DestroyModalState::new(&self.local_projects[idx]));
                }
                DashboardAction::None
            }
            KeyCode::Enter => {
                if !self.local_projects.is_empty() {
                    let idx = self
                        .selected_project_idx
                        .min(self.local_projects.len().saturating_sub(1));
                    DashboardAction::Attach(self.local_projects[idx].clone())
                } else {
                    DashboardAction::None
                }
            }
            KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => DashboardAction::Quit,
            _ => DashboardAction::None,
        }
    }

    /// Handles pasted text into the dashboard or any active creation modal.
    pub fn handle_paste(&mut self, text: &str) {
        if let Some(modal) = &mut self.creation_modal {
            modal.handle_paste(text);
        }
    }

    /// Handles a mouse event (e.g. scroll wheel) and returns any triggered `DashboardAction`.
    pub fn handle_mouse(&mut self, mouse: crossterm::event::MouseEvent) -> DashboardAction {
        if self.destroy_modal.is_some() {
            return DashboardAction::None;
        }

        if let Some(ref mut modal) = self.creation_modal {
            match modal.handle_mouse(mouse) {
                CreationModalAction::Cancel => {
                    self.creation_modal = None;
                    return DashboardAction::None;
                }
                CreationModalAction::Submit {
                    template,
                    project,
                    agent_override,
                    prompt,
                    selected_repos,
                } => {
                    self.creation_modal = None;
                    return DashboardAction::CreateProject {
                        template,
                        project,
                        agent_override,
                        prompt,
                        selected_repos,
                    };
                }
                CreationModalAction::None => return DashboardAction::None,
            }
        }

        match mouse.kind {
            crossterm::event::MouseEventKind::ScrollUp => {
                self.prev_item();
                DashboardAction::None
            }
            crossterm::event::MouseEventKind::ScrollDown => {
                self.next_item();
                DashboardAction::None
            }
            _ => DashboardAction::None,
        }
    }
}

/// Renders the full-width local projects dashboard, top header status bar, and optional floating modals into the given `Buffer`.
pub fn render_dashboard(state: &DashboardState, area: Rect, buf: &mut Buffer, theme: &Theme) {
    if area.height < 3 || area.width < 10 {
        return;
    }

    // Split area into Header status bar and Main area
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(2)])
        .split(area);

    let header_area = chunks[0];
    let main_area = chunks[1];

    // 1. Render Top Header Status Bar using unified TopBar component
    let brand_text = " botai ";
    let brand_len = brand_text.chars().count() as u16;
    let brand_line = Line::from(Span::styled(
        brand_text,
        Style::default()
            .fg(theme.modal_title)
            .bg(theme.status_bar_bg)
            .add_modifier(Modifier::BOLD),
    ));

    let full_hints = "↑/↓: Navigate | Enter: Attach | n/c: New Project | d: Destroy | q: Quit";
    let compact_hints = "Enter: Attach | n: New | d: Destroy | q: Quit";
    let minimal_hints = "q: Quit";

    let full_len = full_hints.chars().count() as u16;
    let compact_len = compact_hints.chars().count() as u16;
    let minimal_len = minimal_hints.chars().count() as u16;

    let hint_to_render = if header_area.width >= brand_len + full_len + 2 {
        Some((full_hints, full_len))
    } else if header_area.width >= brand_len + compact_len + 2 {
        Some((compact_hints, compact_len))
    } else if header_area.width >= brand_len + minimal_len + 2 {
        Some((minimal_hints, minimal_len))
    } else {
        None
    };

    let mut top_bar = TopBar::new(brand_line).with_theme(theme);
    if let Some((hint_str, hint_len)) = hint_to_render {
        let hint_style = Style::default().fg(theme.key_hint).bg(theme.status_bar_bg);
        let hint_line = Line::from(Span::styled(hint_str, hint_style));
        top_bar = top_bar.with_right(hint_line, hint_len);
    }
    top_bar.render(header_area, buf);

    // 2. Render Full-Width Main Panel (Local Projects)
    let border_style = Style::default()
        .fg(theme.border_focused)
        .add_modifier(Modifier::BOLD);

    let main_block = Block::default()
        .borders(Borders::ALL)
        .title(" Local Projects ")
        .border_style(border_style);

    let inner = main_block.inner(main_area);
    main_block.render(main_area, buf);

    if state.local_projects.is_empty() {
        let empty_msg = "No local projects found in CWD. Press [n] or [c] to create a new project.";
        let msg = Paragraph::new(empty_msg).style(Style::default().fg(theme.input_placeholder));
        msg.render(inner, buf);
    } else {
        let items: Vec<ListItem> = state
            .local_projects
            .iter()
            .enumerate()
            .map(|(i, proj)| {
                let is_selected = i == state.selected_project_idx;
                let (prefix, base_style) = if is_selected {
                    (
                        " > ",
                        Style::default()
                            .fg(theme.selection_fg)
                            .bg(theme.selection_bg)
                            .add_modifier(Modifier::BOLD),
                    )
                } else {
                    ("   ", Style::default().fg(theme.status_bar_fg))
                };

                let mut spans = vec![Span::styled(prefix, base_style)];

                if let Some(status) = state.running_sessions.get(proj) {
                    let dot_color = status.status_color(theme);
                    let mut dot_style = Style::default().fg(dot_color);
                    if is_selected {
                        dot_style = dot_style
                            .bg(theme.selection_bg)
                            .add_modifier(Modifier::BOLD);
                    }
                    spans.push(Span::styled("● ", dot_style));
                } else {
                    spans.push(Span::styled("  ", base_style));
                }

                spans.push(Span::styled(proj.clone(), base_style));

                ListItem::new(Line::from(spans)).style(base_style)
            })
            .collect();

        let list = List::new(items);
        list.render(inner, buf);
    }

    // 3. Render Floating Destroy Modal or Creation Modal if active
    if let Some(ref modal) = state.destroy_modal {
        render_destroy_modal(modal, area, buf, theme);
    } else if let Some(ref modal) = state.creation_modal {
        modal.render_modal(area, buf, theme);
    }
}

pub fn render_destroy_modal(
    modal: &DestroyModalState,
    area: Rect,
    buf: &mut Buffer,
    theme: &Theme,
) {
    let width = 76.min(area.width.saturating_sub(4));
    let height = 8.min(area.height.saturating_sub(2));

    if width < 20 || height < 5 {
        return;
    }

    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let modal_area = Rect::new(x, y, width, height);

    // Clear background of modal area
    Clear.render(modal_area, buf);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Confirm Destroy Project ")
        .border_style(
            Style::default()
                .fg(theme.status_error)
                .add_modifier(Modifier::BOLD),
        );

    let inner = block.inner(modal_area);
    block.render(modal_area, buf);

    let lines = vec![
        Line::from(vec![
            Span::raw("Are you sure you want to destroy project '"),
            Span::styled(
                &modal.project_name,
                Style::default()
                    .fg(theme.modal_title)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("'?"),
        ]),
        Line::from(Span::styled(
            "This will permanently remove the project directory and its git worktrees.",
            Style::default().fg(theme.status_bar_fg),
        )),
        Line::from(""),
        Line::from(Span::styled(
            " [y] / [Enter] Confirm  |  [n] / [Esc] Cancel ",
            Style::default()
                .fg(theme.key_hint)
                .add_modifier(Modifier::BOLD),
        )),
    ];

    let paragraph = Paragraph::new(lines);
    paragraph.render(inner, buf);
}

impl Widget for &DashboardState {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let theme = Theme::default();
        render_dashboard(self, area, buf, &theme);
    }
}

impl Widget for DashboardState {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let theme = Theme::default();
        render_dashboard(&self, area, buf, &theme);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use std::fs;
    use tempfile::tempdir;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn test_render_dashboard_full_width_and_header() {
        let state = DashboardState::new(
            vec!["payments".to_string(), "analytics".to_string()],
            vec!["cart-service".to_string(), "billing-api".to_string()],
        );

        let area = Rect::new(0, 0, 100, 20);
        let mut buf = Buffer::empty(area);
        let theme = Theme::default();

        render_dashboard(&state, area, &mut buf, &theme);

        let rendered = buf
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect::<String>();

        assert!(rendered.contains("botai"));
        assert!(rendered.contains("Local Projects"));
        assert!(rendered.contains("cart-service"));
        assert!(rendered.contains("billing-api"));
        assert!(!rendered.contains("Project Templates"));
        assert!(rendered.contains("↑/↓: Navigate"));
        assert!(rendered.contains("Enter: Attach"));
        assert!(rendered.contains("n/c: New Project"));
        assert!(rendered.contains("d: Destroy"));
        assert!(rendered.contains("q: Quit"));
    }

    #[test]
    fn test_render_dashboard_responsive_header_hints() {
        let state = DashboardState::new(vec!["ws".into()], vec!["proj".into()]);
        let theme = Theme::default();

        // 1. Wide screen (100 cols) -> Full hints
        let area_wide = Rect::new(0, 0, 100, 20);
        let mut buf_wide = Buffer::empty(area_wide);
        render_dashboard(&state, area_wide, &mut buf_wide, &theme);
        let text_wide = buf_wide
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect::<String>();
        assert!(text_wide.contains("botai"));
        assert!(text_wide
            .contains("↑/↓: Navigate | Enter: Attach | n/c: New Project | d: Destroy | q: Quit"));

        // 2. Medium screen (60 cols) -> Compact hints
        let area_med = Rect::new(0, 0, 60, 20);
        let mut buf_med = Buffer::empty(area_med);
        render_dashboard(&state, area_med, &mut buf_med, &theme);
        let text_med = buf_med
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect::<String>();
        assert!(text_med.contains("botai"));
        assert!(text_med.contains("Enter: Attach | n: New | d: Destroy | q: Quit"));
        assert!(!text_med.contains("↑/↓: Navigate"));

        // 3. Narrow screen (20 cols) -> Minimal hints
        let area_narrow = Rect::new(0, 0, 20, 20);
        let mut buf_narrow = Buffer::empty(area_narrow);
        render_dashboard(&state, area_narrow, &mut buf_narrow, &theme);
        let text_narrow = buf_narrow
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect::<String>();
        assert!(text_narrow.contains("botai"));
        assert!(text_narrow.contains("q: Quit"));
    }

    #[test]
    fn test_render_dashboard_with_creation_modal() {
        let mut state = DashboardState::new(
            vec!["payments".to_string()],
            vec!["cart-service".to_string()],
        );
        state.open_creation_modal();
        if let Some(ref mut modal) = state.creation_modal {
            modal.step = crate::tui::CreationStep::ConfigureDetails;
            modal.set_project_name("new-checkout");
            modal.prompt_input = "Implement Stripe".to_string();
        }

        let area = Rect::new(0, 0, 100, 20);
        let mut buf = Buffer::empty(area);
        let theme = Theme::default();

        render_dashboard(&state, area, &mut buf, &theme);

        let rendered = buf
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect::<String>();

        assert!(rendered.contains("Create New Project"));
        assert!(rendered.contains("payments"));
        assert!(rendered.contains("Project Name:"));
        assert!(rendered.contains("new-checkout"));
        assert!(rendered.contains("Initial Prompt:"));
        assert!(rendered.contains("Implement Stripe"));
        assert!(rendered.contains("[Enter] Create Project"));
    }

    #[test]
    fn test_render_empty_state() {
        let state = DashboardState::new(vec![], vec![]);

        let area = Rect::new(0, 0, 100, 20);
        let mut buf = Buffer::empty(area);
        let theme = Theme::default();

        render_dashboard(&state, area, &mut buf, &theme);

        let rendered = buf
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect::<String>();

        assert!(rendered
            .contains("No local projects found in CWD. Press [n] or [c] to create a new project."));
    }

    #[test]
    fn test_unicode_and_special_character_support() {
        let mut state = DashboardState::new(
            vec!["воркспейс-1 🚀".to_string()],
            vec!["проект-тест 💻".to_string()],
        );

        let area = Rect::new(0, 0, 100, 20);
        let mut buf = Buffer::empty(area);
        let theme = Theme::default();
        render_dashboard(&state, area, &mut buf, &theme);

        let rendered = buf
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect::<String>();

        assert!(rendered.contains("проект-тест 💻"));

        state.open_creation_modal();
        state.handle_key(key(KeyCode::Enter));
        for c in "проект-юникод".chars() {
            state.handle_key(key(KeyCode::Char(c)));
        }
        let action = state.handle_key(key(KeyCode::Enter));
        assert_eq!(
            action,
            DashboardAction::CreateProject {
                template: "воркспейс-1 🚀".to_string(),
                project: "проект-юникод".to_string(),
                agent_override: Some("claude".to_string()),
                prompt: None,
                selected_repos: vec!["воркспейс-1 🚀-repo".to_string()],
            }
        );
    }

    #[test]
    fn test_dashboard_themed_rendering() {
        let state =
            DashboardState::new(vec!["ws-backend".to_string()], vec!["proj-api".to_string()]);

        let area = Rect::new(0, 0, 100, 20);
        let mut buf = Buffer::empty(area);
        let theme = Theme::gruvbox();

        render_dashboard(&state, area, &mut buf, &theme);

        // Check top header status bar cell background matches theme.status_bar_bg
        let header_cell = buf.cell((0, 0)).unwrap();
        assert_eq!(header_cell.bg, theme.status_bar_bg);
    }

    #[test]
    fn test_dashboard_state_initialization() {
        let templates = vec!["ecommerce".to_string(), "analytics".to_string()];
        let local_projects = vec!["feature-checkout".to_string(), "hotfix-auth".to_string()];

        let state = DashboardState::new(templates.clone(), local_projects.clone());

        assert_eq!(state.templates, templates);
        assert_eq!(state.local_projects, local_projects);
        assert_eq!(state.selected_project_idx, 0);
        assert!(state.creation_modal.is_none());
    }

    #[test]
    fn test_dashboard_state_from_env_scanning() {
        let tmp = tempdir().expect("Failed to create tempdir");
        let cwd = tmp.path();

        // 1. Create a directory with AGENTS.md (valid project)
        let valid_proj_1 = cwd.join("alpha-service");
        fs::create_dir(&valid_proj_1).unwrap();
        fs::write(valid_proj_1.join("AGENTS.md"), "# Project Context").unwrap();

        // 2. Create another directory with AGENTS.md
        let valid_proj_2 = cwd.join("beta-service");
        fs::create_dir(&valid_proj_2).unwrap();
        fs::write(valid_proj_2.join("AGENTS.md"), "# Project Context").unwrap();

        // 3. Create a regular directory without AGENTS.md (not a project)
        let regular_dir = cwd.join("some-random-folder");
        fs::create_dir(&regular_dir).unwrap();
        fs::write(regular_dir.join("README.md"), "# Not a project").unwrap();

        // 4. Create a standalone AGENTS.md file in root
        fs::write(cwd.join("AGENTS.md"), "# Root context").unwrap();

        let toml_content = r#"
default_agent = "claude"
repos_dir = "/repos"

[agents.claude]
command = "claude"

[templates.backend]
repos = ["/path/repo1"]

[templates.frontend]
repos = ["/path/repo2"]
"#;
        let config = Config::load_from_str(toml_content).unwrap();
        let state = DashboardState::from_env(&config, cwd);

        assert_eq!(
            state.local_projects,
            vec!["alpha-service".to_string(), "beta-service".to_string()]
        );
        assert_eq!(
            state.templates,
            vec!["backend".to_string(), "frontend".to_string()]
        );
    }

    #[test]
    fn test_dashboard_state_from_env_empty_and_nonexistent_dirs() {
        let tmp = tempdir().expect("Failed to create tempdir");
        let empty_cwd = tmp.path().join("empty_folder");
        fs::create_dir(&empty_cwd).unwrap();

        let config = Config::load_from_str(
            r#"
default_agent = "claude"
repos_dir = "/repos"
[agents.claude]
command = "claude"
"#,
        )
        .unwrap();

        let state = DashboardState::from_env(&config, &empty_cwd);
        assert!(state.local_projects.is_empty());
        assert!(state.templates.is_empty());

        let non_existent_cwd = tmp.path().join("does_not_exist");
        let state_nonexistent = DashboardState::from_env(&config, &non_existent_cwd);
        assert!(state_nonexistent.local_projects.is_empty());
    }

    #[test]
    fn test_list_item_navigation_and_boundaries() {
        let mut state = DashboardState::new(
            vec!["ws1".into()],
            vec!["proj1".into(), "proj2".into(), "proj3".into()],
        );

        // Initial index 0
        assert_eq!(state.selected_project_idx, 0);

        // Down / j moves down
        state.handle_key(key(KeyCode::Down));
        assert_eq!(state.selected_project_idx, 1);

        state.handle_key(key(KeyCode::Char('j')));
        assert_eq!(state.selected_project_idx, 2);

        // Down at boundary stays at max index (2)
        state.handle_key(key(KeyCode::Down));
        assert_eq!(state.selected_project_idx, 2);

        // Up / k moves up
        state.handle_key(key(KeyCode::Up));
        assert_eq!(state.selected_project_idx, 1);

        state.handle_key(key(KeyCode::Char('k')));
        assert_eq!(state.selected_project_idx, 0);

        // Up at boundary stays at 0
        state.handle_key(key(KeyCode::Up));
        assert_eq!(state.selected_project_idx, 0);

        // End moves to last item
        state.handle_key(key(KeyCode::End));
        assert_eq!(state.selected_project_idx, 2);

        // Home moves to first item
        state.handle_key(key(KeyCode::Home));
        assert_eq!(state.selected_project_idx, 0);
    }

    #[test]
    fn test_list_item_navigation_empty_lists() {
        let mut state = DashboardState::new(vec![], vec![]);
        state.next_item();
        assert_eq!(state.selected_project_idx, 0);
        state.prev_item();
        assert_eq!(state.selected_project_idx, 0);
        state.first_item();
        assert_eq!(state.selected_project_idx, 0);
        state.last_item();
        assert_eq!(state.selected_project_idx, 0);
    }

    #[test]
    fn test_modal_opening_shortcuts_and_closing() {
        let mut state = DashboardState::new(
            vec!["ecommerce".into(), "analytics".into()],
            vec!["proj1".into()],
        );

        // 1. 'n' opens creation modal
        let action = state.handle_key(key(KeyCode::Char('n')));
        assert_eq!(action, DashboardAction::None);
        assert!(state.creation_modal.is_some());

        // Pressing Esc closes modal
        let action = state.handle_key(key(KeyCode::Esc));
        assert_eq!(action, DashboardAction::None);
        assert!(state.creation_modal.is_none());

        // 2. 'c' opens creation modal
        let action = state.handle_key(key(KeyCode::Char('c')));
        assert_eq!(action, DashboardAction::None);
        assert!(state.creation_modal.is_some());
        state.close_creation_modal();
        assert!(state.creation_modal.is_none());

        // 3. 'N' and 'C' uppercase shortcuts
        state.handle_key(key(KeyCode::Char('N')));
        assert!(state.creation_modal.is_some());
        state.close_creation_modal();

        state.handle_key(key(KeyCode::Char('C')));
        assert!(state.creation_modal.is_some());
        state.close_creation_modal();
    }

    #[test]
    fn test_modal_submission_produces_create_project_action() {
        let mut state = DashboardState::new(vec!["cloud-infra".into()], vec![]);
        state.open_creation_modal();

        // 1. Advance from SelectTemplate to ConfigureDetails
        let action = state.handle_key(key(KeyCode::Enter));
        assert_eq!(action, DashboardAction::None);
        assert!(state.creation_modal.is_some());
        assert_eq!(
            state.creation_modal.as_ref().unwrap().step,
            crate::tui::CreationStep::ConfigureDetails
        );

        // In ConfigureDetails, trying to submit with empty project name returns DashboardAction::None (stays open)
        let action = state.handle_key(key(KeyCode::Enter));
        assert_eq!(action, DashboardAction::None);
        assert!(state.creation_modal.is_some());

        // 2. Enter project name and prompt
        for c in "k8s-cluster".chars() {
            state.handle_key(key(KeyCode::Char(c)));
        }
        state.handle_key(key(KeyCode::Tab)); // switch to Agent
        state.handle_key(key(KeyCode::Tab)); // switch to Repositories
        state.handle_key(key(KeyCode::Tab)); // switch to Prompt
        for c in "Deploy helm chart".chars() {
            state.handle_key(key(KeyCode::Char(c)));
        }

        let action = state.handle_key(key(KeyCode::Enter));
        assert_eq!(
            action,
            DashboardAction::CreateProject {
                template: "cloud-infra".to_string(),
                project: "k8s-cluster".to_string(),
                agent_override: Some("claude".to_string()),
                prompt: Some("Deploy helm chart".to_string()),
                selected_repos: vec!["cloud-infra-repo".to_string()],
            }
        );
        assert!(
            state.creation_modal.is_none(),
            "Modal should close after successful submit"
        );
    }

    #[test]
    fn test_modal_submission_with_empty_prompt_is_none() {
        let mut state = DashboardState::new(vec!["ws".into()], vec![]);
        state.open_creation_modal();
        state.handle_key(key(KeyCode::Enter));

        for c in "my-project".chars() {
            state.handle_key(key(KeyCode::Char(c)));
        }

        let action = state.handle_key(key(KeyCode::Enter));
        assert_eq!(
            action,
            DashboardAction::CreateProject {
                template: "ws".to_string(),
                project: "my-project".to_string(),
                agent_override: Some("claude".to_string()),
                prompt: None,
                selected_repos: vec!["ws-repo".to_string()],
            }
        );
    }

    #[test]
    fn test_modal_submission_with_agent_override() {
        let mut state = DashboardState::new(vec!["ws".into()], vec![]);
        state.open_creation_modal();
        state.handle_key(key(KeyCode::Enter)); // Advance from SelectTemplate to ConfigureDetails

        for c in "my-agent-project".chars() {
            state.handle_key(key(KeyCode::Char(c)));
        }

        // Tab to agent selector
        state.handle_key(key(KeyCode::Tab));
        assert_eq!(
            state.creation_modal.as_ref().unwrap().focus,
            crate::tui::creation_modal::CreationFocus::Agent
        );

        // Cycle agent
        state.handle_key(key(KeyCode::Right));
        let selected_agent = state
            .creation_modal
            .as_ref()
            .unwrap()
            .selected_agent()
            .to_string();

        let action = state.handle_key(key(KeyCode::Enter));
        assert_eq!(
            action,
            DashboardAction::CreateProject {
                template: "ws".to_string(),
                project: "my-agent-project".to_string(),
                agent_override: Some(selected_agent),
                prompt: None,
                selected_repos: vec!["ws-repo".to_string()],
            }
        );
    }

    #[test]
    fn test_attach_action_from_project_list() {
        let mut state = DashboardState::new(
            vec!["ws".into()],
            vec!["proj-alpha".into(), "proj-beta".into()],
        );
        state.selected_project_idx = 1;

        let action = state.handle_key(key(KeyCode::Enter));
        assert_eq!(action, DashboardAction::Attach("proj-beta".to_string()));
    }

    #[test]
    fn test_attach_action_on_empty_projects_does_nothing() {
        let mut state = DashboardState::new(vec!["ws".into()], vec![]);

        let action = state.handle_key(key(KeyCode::Enter));
        assert_eq!(action, DashboardAction::None);
    }

    #[test]
    fn test_destroy_action_confirmation_flow() {
        let mut state = DashboardState::new(
            vec!["ws".into()],
            vec!["proj-to-delete".into(), "proj-safe".into()],
        );
        state.selected_project_idx = 0;

        // Press 'd' -> opens destroy modal, returns None
        let action = state.handle_key(key(KeyCode::Char('d')));
        assert_eq!(action, DashboardAction::None);
        assert_eq!(
            state.destroy_modal,
            Some(DestroyModalState {
                project_name: "proj-to-delete".to_string()
            })
        );

        // Cancel with 'n'
        let action = state.handle_key(key(KeyCode::Char('n')));
        assert_eq!(action, DashboardAction::None);
        assert!(state.destroy_modal.is_none());

        // Open again and cancel with Esc
        state.handle_key(key(KeyCode::Char('d')));
        assert!(state.destroy_modal.is_some());
        let action = state.handle_key(key(KeyCode::Esc));
        assert_eq!(action, DashboardAction::None);
        assert!(state.destroy_modal.is_none());

        // Open again with 'D' and confirm with 'y'
        state.handle_key(key(KeyCode::Char('D')));
        assert!(state.destroy_modal.is_some());
        let action = state.handle_key(key(KeyCode::Char('y')));
        assert_eq!(
            action,
            DashboardAction::Destroy("proj-to-delete".to_string())
        );
        assert!(state.destroy_modal.is_none());

        // Open again and confirm with Enter
        state.handle_key(key(KeyCode::Char('d')));
        assert!(state.destroy_modal.is_some());
        let action = state.handle_key(key(KeyCode::Enter));
        assert_eq!(
            action,
            DashboardAction::Destroy("proj-to-delete".to_string())
        );
        assert!(state.destroy_modal.is_none());

        // On empty project list, 'd' does nothing
        let mut empty_state = DashboardState::new(vec![], vec![]);
        let action = empty_state.handle_key(key(KeyCode::Char('d')));
        assert_eq!(action, DashboardAction::None);
        assert!(empty_state.destroy_modal.is_none());
    }

    #[test]
    fn test_render_destroy_modal() {
        let mut state = DashboardState::new(vec!["ws".into()], vec!["legacy-app".into()]);
        state.destroy_modal = Some(DestroyModalState::new("legacy-app"));

        let area = Rect::new(0, 0, 100, 30);
        let mut buf = Buffer::empty(area);
        let theme = Theme::default();

        render_dashboard(&state, area, &mut buf, &theme);
        let rendered = buf
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect::<String>();

        assert!(rendered.contains("Confirm Destroy Project"));
        assert!(rendered.contains("Are you sure you want to destroy project 'legacy-app'?"));
        assert!(rendered.contains("This will permanently remove the project directory"));
        assert!(rendered.contains("[y] / [Enter] Confirm"));
        assert!(rendered.contains("[n] / [Esc] Cancel"));

        // Cursor should be hidden when destroy modal is active
        assert_eq!(state.cursor_position(area), None);
    }

    #[test]
    fn test_quit_actions() {
        let mut state = DashboardState::new(vec!["ws".into()], vec!["proj".into()]);

        assert_eq!(
            state.handle_key(key(KeyCode::Char('q'))),
            DashboardAction::Quit
        );
        assert_eq!(
            state.handle_key(key(KeyCode::Char('Q'))),
            DashboardAction::Quit
        );
        assert_eq!(state.handle_key(key(KeyCode::Esc)), DashboardAction::Quit);
    }

    #[test]
    fn test_creation_modal_and_dashboard_cursor_position() {
        let area = Rect::new(0, 0, 100, 30);
        let mut dashboard = DashboardState::new(vec!["ecommerce".into()], vec![]);
        assert_eq!(dashboard.cursor_position(area), None);

        dashboard.open_creation_modal();
        assert!(dashboard.cursor_position(area).is_some());
    }

    #[test]
    fn test_dashboard_running_sessions_state_methods() {
        let mut running = HashMap::new();
        running.insert("proj-1".to_string(), SessionStatus::Busy);
        running.insert("proj-2".to_string(), SessionStatus::WaitingForInput);

        let state = DashboardState::new(vec!["ws".to_string()], vec!["proj-1".to_string()])
            .with_running_sessions(running.clone());

        assert_eq!(state.running_sessions.len(), 2);
        assert_eq!(
            state.running_sessions.get("proj-1"),
            Some(&SessionStatus::Busy)
        );
        assert_eq!(
            state.running_sessions.get("proj-2"),
            Some(&SessionStatus::WaitingForInput)
        );

        let mut state2 = DashboardState::new(vec![], vec![]);
        state2.set_running_sessions(running);
        assert_eq!(state2.running_sessions.len(), 2);
    }

    #[test]
    fn test_render_dashboard_running_session_indicators_and_colors() {
        let mut running = HashMap::new();
        running.insert("proj-busy".to_string(), SessionStatus::Busy);
        running.insert("proj-wait".to_string(), SessionStatus::WaitingForInput);
        running.insert("proj-done".to_string(), SessionStatus::Done);
        running.insert("proj-err".to_string(), SessionStatus::Error);
        // "proj-inactive" is not in running_sessions

        let projects = vec![
            "proj-busy".to_string(),
            "proj-wait".to_string(),
            "proj-done".to_string(),
            "proj-err".to_string(),
            "proj-inactive".to_string(),
        ];

        let state = DashboardState::new(vec!["template-a".to_string()], projects)
            .with_running_sessions(running);

        let area = Rect::new(0, 0, 100, 20);
        let mut buf = Buffer::empty(area);
        let theme = Theme::tokyo_night();

        render_dashboard(&state, area, &mut buf, &theme);

        let rendered = buf
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect::<String>();

        // Check that project names and status dot indicators are rendered
        assert!(rendered.contains("●"));
        assert!(rendered.contains("proj-busy"));
        assert!(rendered.contains("proj-wait"));
        assert!(rendered.contains("proj-done"));
        assert!(rendered.contains("proj-err"));
        assert!(rendered.contains("proj-inactive"));

        // Inner area starts at x = 1, y = 2 (inside borders, since header is at y = 0, panel at y = 1..20)
        // Row 1 (y = 2): proj-busy (selected) -> prefix is " > ", dot is at x = 4
        let cell_busy_dot = buf.cell((4, 2)).unwrap();
        assert_eq!(cell_busy_dot.symbol(), "●");
        assert_eq!(cell_busy_dot.fg, theme.status_busy);

        // Row 2 (y = 3): proj-wait (unselected) -> prefix "   ", dot at x = 4
        let cell_wait_dot = buf.cell((4, 3)).unwrap();
        assert_eq!(cell_wait_dot.symbol(), "●");
        assert_eq!(cell_wait_dot.fg, theme.status_waiting);

        // Row 3 (y = 4): proj-done -> dot at x = 4
        let cell_done_dot = buf.cell((4, 4)).unwrap();
        assert_eq!(cell_done_dot.symbol(), "●");
        assert_eq!(cell_done_dot.fg, theme.status_done);

        // Row 4 (y = 5): proj-err -> dot at x = 4
        let cell_err_dot = buf.cell((4, 5)).unwrap();
        assert_eq!(cell_err_dot.symbol(), "●");
        assert_eq!(cell_err_dot.fg, theme.status_error);

        // Row 5 (y = 6): proj-inactive -> no dot, spaces at x = 4 and x = 5, project name starts at x = 6
        let cell_inactive_space1 = buf.cell((4, 6)).unwrap();
        let cell_inactive_space2 = buf.cell((5, 6)).unwrap();
        let cell_inactive_p = buf.cell((6, 6)).unwrap();
        assert_eq!(cell_inactive_space1.symbol(), " ");
        assert_eq!(cell_inactive_space2.symbol(), " ");
        assert_eq!(cell_inactive_p.symbol(), "p");
    }

    #[test]
    fn test_render_dashboard_running_session_colors_with_gruvbox_theme() {
        let mut running = HashMap::new();
        running.insert("proj-busy".to_string(), SessionStatus::Busy);

        let projects = vec!["proj-busy".to_string()];
        let state =
            DashboardState::new(vec!["ws".to_string()], projects).with_running_sessions(running);

        let area = Rect::new(0, 0, 100, 20);
        let mut buf = Buffer::empty(area);
        let theme = Theme::gruvbox();

        render_dashboard(&state, area, &mut buf, &theme);

        let cell_busy_dot = buf.cell((4, 2)).unwrap();
        assert_eq!(cell_busy_dot.symbol(), "●");
        assert_eq!(cell_busy_dot.fg, theme.status_busy);
    }

    #[test]
    fn test_dashboard_mouse_scrolling() {
        use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

        let mut state = DashboardState::new(
            vec!["ws1".to_string()],
            vec![
                "proj1".to_string(),
                "proj2".to_string(),
                "proj3".to_string(),
            ],
        );

        assert_eq!(state.selected_project_idx, 0);

        let scroll_down = MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 10,
            row: 10,
            modifiers: KeyModifiers::empty(),
        };
        let scroll_up = MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 10,
            row: 10,
            modifiers: KeyModifiers::empty(),
        };

        // ScrollDown in project list
        assert_eq!(state.handle_mouse(scroll_down), DashboardAction::None);
        assert_eq!(state.selected_project_idx, 1);

        assert_eq!(state.handle_mouse(scroll_down), DashboardAction::None);
        assert_eq!(state.selected_project_idx, 2);

        // ScrollUp in project list
        assert_eq!(state.handle_mouse(scroll_up), DashboardAction::None);
        assert_eq!(state.selected_project_idx, 1);

        // Non-scroll mouse event
        let click = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 10,
            row: 10,
            modifiers: KeyModifiers::empty(),
        };
        assert_eq!(state.handle_mouse(click), DashboardAction::None);
    }

    #[test]
    fn test_dashboard_open_creation_modal_reloads_config() {
        let dir = tempfile::tempdir().unwrap();
        let config_file = dir.path().join("config.toml");
        let toml_v1 = r#"
default_agent = "claude"
repos_dir = "/tmp/repos"

[templates.base_tpl]
repos = ["repo1"]
"#;
        std::fs::write(&config_file, toml_v1).unwrap();

        let config = Config::load_from_file(&config_file).unwrap();
        let mut state =
            DashboardState::from_env(&config, dir.path()).with_config_path(&config_file);
        assert_eq!(state.templates, vec!["base_tpl".to_string()]);

        // Add a new template while dashboard is running
        let toml_v2 = r#"
default_agent = "claude"
repos_dir = "/tmp/repos"

[templates.base_tpl]
repos = ["repo1"]

[templates.new_tpl]
repos = ["repo2"]
"#;
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&config_file, toml_v2).unwrap();

        // Opening creation modal reloads config and updates template list
        state.open_creation_modal();

        assert_eq!(
            state.templates,
            vec!["base_tpl".to_string(), "new_tpl".to_string()]
        );
        let modal = state
            .creation_modal
            .as_ref()
            .expect("creation modal must be open");
        assert_eq!(
            modal.templates,
            vec!["base_tpl".to_string(), "new_tpl".to_string()]
        );
    }

    #[test]
    fn test_dashboard_reload_config_syntax_error_preserves_existing() {
        let dir = tempfile::tempdir().unwrap();
        let config_file = dir.path().join("config.toml");
        let toml_v1 = r#"
default_agent = "claude"
repos_dir = "/tmp/repos"

[templates.base_tpl]
repos = ["repo1"]
"#;
        std::fs::write(&config_file, toml_v1).unwrap();

        let config = Config::load_from_file(&config_file).unwrap();
        let mut state =
            DashboardState::from_env(&config, dir.path()).with_config_path(&config_file);

        // Write broken TOML
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&config_file, "invalid = [unterminated").unwrap();

        // reload_config returns error but keeps previous config
        let res = state.reload_config();
        assert!(res.is_err());
        assert_eq!(state.templates, vec!["base_tpl".to_string()]);

        // open_creation_modal does not crash and uses existing valid config
        state.open_creation_modal();
        assert!(state.creation_modal.is_some());
        assert_eq!(
            state.creation_modal.unwrap().templates,
            vec!["base_tpl".to_string()]
        );
    }
}
