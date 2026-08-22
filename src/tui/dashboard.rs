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

use crate::config::Config;
use crate::session::SessionStatus;
use crate::tui::creation_modal::{CreationModalAction, ProjectCreationModal};
use crate::tui::theme::Theme;

/// Backward compatibility alias for ProjectCreationModal.
pub type CreationModalState = ProjectCreationModal;

/// Active panel in the two-panel dashboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivePanel {
    Projects,
    Templates,
}

/// Actions emitted by user interactions with the dashboard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DashboardAction {
    None,
    Attach(String),
    CreateProject {
        template: String,
        project: String,
        prompt: Option<String>,
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
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                Some(DashboardAction::None)
            }
            _ => None,
        }
    }
}

/// Full state for the interactive two-panel launcher dashboard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DashboardState {
    pub templates: Vec<String>,
    pub local_projects: Vec<String>,
    pub running_sessions: HashMap<String, SessionStatus>,
    pub active_panel: ActivePanel,
    pub selected_project_idx: usize,
    pub selected_template_idx: usize,
    pub creation_modal: Option<ProjectCreationModal>,
    pub destroy_modal: Option<DestroyModalState>,
    pub config: Config,
    pub cwd: PathBuf,
}

impl DashboardState {
    /// Creates a new `DashboardState` with given templates and local projects.
    pub fn new(templates: Vec<String>, local_projects: Vec<String>) -> Self {
        let mut config = Config::load_from_str("default_agent = \"claude\"\nrepos_dir = \".botai_repos\"\n")
            .unwrap_or_else(|_| Config {
                default_agent: "claude".to_string(),
                theme: "tokyo-night".to_string(),
                repos_dir: PathBuf::from(".botai_repos"),
                agents: HashMap::new(),
                templates: HashMap::new(),
            });

        for tpl in &templates {
            config.templates.entry(tpl.clone()).or_insert_with(|| crate::config::TemplateConfig {
                description: None,
                default_agent: None,
                agents_md_append: None,
                repos: Vec::new(),
                env: HashMap::new(),
            });
        }

        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

        Self {
            templates,
            local_projects,
            running_sessions: HashMap::new(),
            active_panel: ActivePanel::Projects,
            selected_project_idx: 0,
            selected_template_idx: 0,
            creation_modal: None,
            destroy_modal: None,
            config,
            cwd,
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

    /// Toggles active panel between Projects and Templates.
    pub fn switch_panel(&mut self) {
        self.active_panel = match self.active_panel {
            ActivePanel::Projects => ActivePanel::Templates,
            ActivePanel::Templates => ActivePanel::Projects,
        };
    }

    /// Moves selection down to the next item in the currently active panel.
    pub fn next_item(&mut self) {
        match self.active_panel {
            ActivePanel::Projects => {
                if !self.local_projects.is_empty()
                    && self.selected_project_idx + 1 < self.local_projects.len()
                {
                    self.selected_project_idx += 1;
                }
            }
            ActivePanel::Templates => {
                if !self.templates.is_empty()
                    && self.selected_template_idx + 1 < self.templates.len()
                {
                    self.selected_template_idx += 1;
                }
            }
        }
    }

    /// Moves selection up to the previous item in the currently active panel.
    pub fn prev_item(&mut self) {
        match self.active_panel {
            ActivePanel::Projects => {
                self.selected_project_idx = self.selected_project_idx.saturating_sub(1);
            }
            ActivePanel::Templates => {
                self.selected_template_idx = self.selected_template_idx.saturating_sub(1);
            }
        }
    }

    /// Opens the project creation modal for the currently selected template.
    pub fn open_creation_modal(&mut self) {
        if !self.templates.is_empty() {
            let tpl_idx = self
                .selected_template_idx
                .min(self.templates.len().saturating_sub(1));
            let tpl = self.templates[tpl_idx].clone();
            self.creation_modal = Some(ProjectCreationModal::new(&self.config, &self.cwd, Some(&tpl)));
        }
    }

    /// Closes the project creation modal.
    pub fn close_creation_modal(&mut self) {
        self.creation_modal = None;
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
                    prompt,
                    ..
                } => {
                    self.creation_modal = None;
                    return DashboardAction::CreateProject {
                        template,
                        project,
                        prompt,
                    };
                }
                CreationModalAction::None => {
                    self.creation_modal = Some(modal);
                    return DashboardAction::None;
                }
            }
        }

        match key.code {
            KeyCode::Tab | KeyCode::Left | KeyCode::Right => {
                self.switch_panel();
                DashboardAction::None
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.prev_item();
                DashboardAction::None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.next_item();
                DashboardAction::None
            }
            KeyCode::Char('n') => {
                self.open_creation_modal();
                DashboardAction::None
            }
            KeyCode::Char('d') => {
                if self.active_panel == ActivePanel::Projects && !self.local_projects.is_empty() {
                    let idx = self
                        .selected_project_idx
                        .min(self.local_projects.len().saturating_sub(1));
                    self.destroy_modal = Some(DestroyModalState::new(&self.local_projects[idx]));
                }
                DashboardAction::None
            }
            KeyCode::Enter => match self.active_panel {
                ActivePanel::Projects => {
                    if !self.local_projects.is_empty() {
                        let idx = self
                            .selected_project_idx
                            .min(self.local_projects.len().saturating_sub(1));
                        DashboardAction::Attach(self.local_projects[idx].clone())
                    } else {
                        DashboardAction::None
                    }
                }
                ActivePanel::Templates => {
                    self.open_creation_modal();
                    DashboardAction::None
                }
            },
            KeyCode::Char('q') | KeyCode::Esc => DashboardAction::Quit,
            _ => DashboardAction::None,
        }
    }

    /// Handles a mouse event (e.g. scroll wheel) and returns any triggered `DashboardAction`.
    pub fn handle_mouse(&mut self, mouse: crossterm::event::MouseEvent) -> DashboardAction {
        if self.destroy_modal.is_some() || self.creation_modal.is_some() {
            return DashboardAction::None;
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


/// Renders the full two-panel dashboard, top header status bar, and optional floating creation modal into the given `Buffer`.
pub fn render_dashboard(state: &DashboardState, area: Rect, buf: &mut Buffer, theme: &Theme) {
    if area.height < 3 || area.width < 10 {
        return;
    }

    // Split area into Header status bar and Main area
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(3)])
        .split(area);

    let header_area = chunks[0];
    let main_area = chunks[1];

    // 1. Render Top Header Status Bar
    let bg_style = Style::default()
        .bg(theme.status_bar_bg)
        .fg(theme.status_bar_fg);
    for x in header_area.left()..header_area.right() {
        for y in header_area.top()..header_area.bottom() {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_style(bg_style).set_char(' ');
            }
        }
    }

    let brand_text = " botai ";
    let brand_len = brand_text.chars().count() as u16;
    let brand_line = Line::from(Span::styled(
        brand_text,
        Style::default()
            .fg(theme.modal_title)
            .bg(theme.status_bar_bg)
            .add_modifier(Modifier::BOLD),
    ));
    buf.set_line(header_area.x, header_area.y, &brand_line, header_area.width);

    let full_hints = "Tab: Switch | ↑/↓: Navigate | Enter: Select | n: New | d: Destroy | q: Quit";
    let compact_hints = "Tab: Switch | Enter: Select | n: New | d: Destroy | q: Quit";
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

    if let Some((hint_str, hint_len)) = hint_to_render {
        let start_x = header_area.right().saturating_sub(hint_len + 1);
        let hint_style = Style::default()
            .fg(theme.key_hint)
            .bg(theme.status_bar_bg);
        let hint_line = Line::from(Span::styled(hint_str, hint_style));
        buf.set_line(start_x, header_area.y, &hint_line, hint_len);
    }

    // Split main area into Left (Projects) and Right (Workspaces)
    let panel_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(main_area);

    let left_area = panel_chunks[0];
    let right_area = panel_chunks[1];

    // 2. Render Left Panel (Local Projects)
    let is_left_active = state.active_panel == ActivePanel::Projects;
    let left_border_style = if is_left_active {
        Style::default()
            .fg(theme.border_focused)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.border_unfocused)
    };

    let left_block = Block::default()
        .borders(Borders::ALL)
        .title(" Local Projects (Existing) ")
        .border_style(left_border_style);

    let left_inner = left_block.inner(left_area);
    left_block.render(left_area, buf);

    if state.local_projects.is_empty() {
        let msg = Paragraph::new(" (No local projects found in CWD) ")
            .style(Style::default().fg(theme.input_placeholder));
        msg.render(left_inner, buf);
    } else {
        let items: Vec<ListItem> = state
            .local_projects
            .iter()
            .enumerate()
            .map(|(i, proj)| {
                let is_selected = i == state.selected_project_idx;
                let (prefix, base_style) = if is_selected && is_left_active {
                    (
                        " > ",
                        Style::default()
                            .fg(theme.selection_fg)
                            .bg(theme.selection_bg)
                            .add_modifier(Modifier::BOLD),
                    )
                } else if is_selected {
                    (" > ", Style::default().fg(theme.selection_indicator))
                } else {
                    ("   ", Style::default().fg(theme.status_bar_fg))
                };

                let mut spans = vec![Span::styled(prefix, base_style)];

                if let Some(status) = state.running_sessions.get(proj) {
                    let dot_color = status.status_color(theme);
                    let mut dot_style = Style::default().fg(dot_color);
                    if is_selected && is_left_active {
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
        list.render(left_inner, buf);
    }

    // 3. Render Right Panel (Project Templates)
    let is_right_active = state.active_panel == ActivePanel::Templates;
    let right_border_style = if is_right_active {
        Style::default()
            .fg(theme.border_focused)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.border_unfocused)
    };

    let right_block = Block::default()
        .borders(Borders::ALL)
        .title(" Project Templates ")
        .border_style(right_border_style);

    let right_inner = right_block.inner(right_area);
    right_block.render(right_area, buf);

    if state.templates.is_empty() {
        let msg = Paragraph::new(" (No templates found in config) ")
            .style(Style::default().fg(theme.input_placeholder));
        msg.render(right_inner, buf);
    } else {
        let items: Vec<ListItem> = state
            .templates
            .iter()
            .enumerate()
            .map(|(i, ws)| {
                let is_selected = i == state.selected_template_idx;
                let (prefix, style) = if is_selected && is_right_active {
                    (
                        " > ",
                        Style::default()
                            .fg(theme.selection_fg)
                            .bg(theme.selection_bg)
                            .add_modifier(Modifier::BOLD),
                    )
                } else if is_selected {
                    (" > ", Style::default().fg(theme.selection_indicator))
                } else {
                    ("   ", Style::default().fg(theme.status_bar_fg))
                };
                ListItem::new(format!("{}{}", prefix, ws)).style(style)
            })
            .collect();

        let list = List::new(items);
        list.render(right_inner, buf);
    }

    // 4. Render Floating Destroy Modal or Creation Modal if active
    if let Some(ref modal) = state.destroy_modal {
        render_destroy_modal(modal, area, buf, theme);
    } else if let Some(ref modal) = state.creation_modal {
        modal.render_modal(area, buf, theme);
    }
}

fn render_destroy_modal(modal: &DestroyModalState, area: Rect, buf: &mut Buffer, theme: &Theme) {
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
    use crate::tui::CreationFocus;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use std::fs;
    use tempfile::{tempdir, TempDir};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn test_render_dashboard_panels_and_header() {
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
            .map(|cell| cell.symbol())
            .collect::<String>();

        assert!(rendered.contains("botai"));
        assert!(rendered.contains("Local Projects (Existing)"));
        assert!(rendered.contains("Project Templates"));
        assert!(rendered.contains("cart-service"));
        assert!(rendered.contains("billing-api"));
        assert!(rendered.contains("payments"));
        assert!(rendered.contains("analytics"));
        assert!(rendered.contains("Tab: Switch"));
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
            .map(|c| c.symbol())
            .collect::<String>();
        assert!(text_wide.contains("botai"));
        assert!(text_wide.contains("Tab: Switch | ↑/↓: Navigate | Enter: Select | n: New | d: Destroy | q: Quit"));

        // 2. Medium screen (75 cols) -> Compact hints
        let area_med = Rect::new(0, 0, 75, 20);
        let mut buf_med = Buffer::empty(area_med);
        render_dashboard(&state, area_med, &mut buf_med, &theme);
        let text_med = buf_med
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect::<String>();
        assert!(text_med.contains("botai"));
        assert!(text_med.contains("Tab: Switch | Enter: Select | n: New | d: Destroy | q: Quit"));
        assert!(!text_med.contains("↑/↓: Navigate"));

        // 3. Narrow screen (20 cols) -> Minimal hints
        let area_narrow = Rect::new(0, 0, 20, 20);
        let mut buf_narrow = Buffer::empty(area_narrow);
        render_dashboard(&state, area_narrow, &mut buf_narrow, &theme);
        let text_narrow = buf_narrow
            .content()
            .iter()
            .map(|c| c.symbol())
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
            modal.project_name_input = "new-checkout".to_string();
            modal.prompt_input = "Implement Stripe".to_string();
        }

        let area = Rect::new(0, 0, 100, 20);
        let mut buf = Buffer::empty(area);
        let theme = Theme::default();

        render_dashboard(&state, area, &mut buf, &theme);

        let rendered = buf
            .content()
            .iter()
            .map(|cell| cell.symbol())
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
            .map(|cell| cell.symbol())
            .collect::<String>();

        assert!(rendered.contains("(No local projects found in CWD)"));
        assert!(rendered.contains("(No templates found in config)"));
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
            .map(|cell| cell.symbol())
            .collect::<String>();

        assert!(rendered.contains("воркспейс-1 🚀"));
        assert!(rendered.contains("проект-тест 💻"));

        state.open_creation_modal();
        for c in "проект-юникод".chars() {
            state.handle_key(key(KeyCode::Char(c)));
        }
        let action = state.handle_key(key(KeyCode::Enter));
        assert_eq!(
            action,
            DashboardAction::CreateProject {
                template: "воркспейс-1 🚀".to_string(),
                project: "проект-юникод".to_string(),
                prompt: None,
            }
        );
    }

    #[test]
    fn test_dashboard_themed_rendering() {
        let state = DashboardState::new(
            vec!["ws-backend".to_string()],
            vec!["proj-api".to_string()],
        );

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

        assert_eq!(state.active_panel, ActivePanel::Projects);
        assert_eq!(state.templates, templates);
        assert_eq!(state.local_projects, local_projects);
        assert_eq!(state.selected_project_idx, 0);
        assert_eq!(state.selected_template_idx, 0);
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
    fn test_panel_switching() {
        let mut state = DashboardState::new(vec!["ws".into()], vec!["proj".into()]);
        assert_eq!(state.active_panel, ActivePanel::Projects);

        // Switch via method
        state.switch_panel();
        assert_eq!(state.active_panel, ActivePanel::Templates);

        state.switch_panel();
        assert_eq!(state.active_panel, ActivePanel::Projects);

        // Switch via key events: Tab, Left, Right
        let action = state.handle_key(key(KeyCode::Tab));
        assert_eq!(action, DashboardAction::None);
        assert_eq!(state.active_panel, ActivePanel::Templates);

        let action = state.handle_key(key(KeyCode::Left));
        assert_eq!(action, DashboardAction::None);
        assert_eq!(state.active_panel, ActivePanel::Projects);

        let action = state.handle_key(key(KeyCode::Right));
        assert_eq!(action, DashboardAction::None);
        assert_eq!(state.active_panel, ActivePanel::Templates);
    }

    #[test]
    fn test_list_item_navigation_and_boundaries() {
        let mut state = DashboardState::new(
            vec!["ws1".into(), "ws2".into(), "ws3".into()],
            vec!["proj1".into(), "proj2".into()],
        );

        // Projects navigation (active_panel = Projects)
        assert_eq!(state.selected_project_idx, 0);
        state.handle_key(key(KeyCode::Down));
        assert_eq!(state.selected_project_idx, 1);

        // Down at boundary stays at max index
        state.handle_key(key(KeyCode::Down));
        assert_eq!(state.selected_project_idx, 1);

        // Down at boundary stays at max index
        state.handle_key(key(KeyCode::Down));
        assert_eq!(state.selected_project_idx, 1);

        // Up moves back
        state.handle_key(key(KeyCode::Up));
        assert_eq!(state.selected_project_idx, 0);

        // Up at boundary stays at 0
        state.handle_key(key(KeyCode::Up));
        assert_eq!(state.selected_project_idx, 0);

        // Switch to Templates
        state.switch_panel();
        assert_eq!(state.selected_template_idx, 0);

        state.handle_key(key(KeyCode::Char('j'))); // vim down
        assert_eq!(state.selected_template_idx, 1);
        state.handle_key(key(KeyCode::Char('j')));
        assert_eq!(state.selected_template_idx, 2);
        state.handle_key(key(KeyCode::Char('j'))); // boundary
        assert_eq!(state.selected_template_idx, 2);

        state.handle_key(key(KeyCode::Char('k'))); // vim up
        assert_eq!(state.selected_template_idx, 1);
    }

    #[test]
    fn test_list_item_navigation_empty_lists() {
        let mut state = DashboardState::new(vec![], vec![]);
        state.next_item();
        assert_eq!(state.selected_project_idx, 0);
        state.prev_item();
        assert_eq!(state.selected_project_idx, 0);

        state.switch_panel();
        state.next_item();
        assert_eq!(state.selected_template_idx, 0);
        state.prev_item();
        assert_eq!(state.selected_template_idx, 0);
    }

    #[test]
    fn test_modal_opening_and_closing() {
        let mut state = DashboardState::new(
            vec!["ecommerce".into(), "analytics".into()],
            vec!["proj1".into()],
        );

        // When in Templates panel, pressing Enter or 'n' opens modal
        state.active_panel = ActivePanel::Templates;
        state.selected_template_idx = 1;

        let action = state.handle_key(key(KeyCode::Enter));
        assert_eq!(action, DashboardAction::None);
        assert!(state.creation_modal.is_some());
        assert_eq!(
            state.creation_modal.as_ref().unwrap().selected_template(),
            Some("analytics")
        );

        // Pressing Esc closes modal
        let action = state.handle_key(key(KeyCode::Esc));
        assert_eq!(action, DashboardAction::None);
        assert!(state.creation_modal.is_none());

        // Pressing 'n' from Projects panel also triggers modal for selected template
        state.active_panel = ActivePanel::Projects;
        state.selected_template_idx = 0;
        let action = state.handle_key(key(KeyCode::Char('n')));
        assert_eq!(action, DashboardAction::None);
        assert!(state.creation_modal.is_some());
        assert_eq!(
            state.creation_modal.as_ref().unwrap().selected_template(),
            Some("ecommerce")
        );

        state.close_creation_modal();
        assert!(state.creation_modal.is_none());
    }

    #[test]
    fn test_modal_opening_empty_templates() {
        let mut state = DashboardState::new(vec![], vec!["proj1".into()]);
        state.open_creation_modal();
        assert!(state.creation_modal.is_none());
    }

    #[test]
    fn test_modal_text_entry_editing_and_field_switching() {
        let config = Config::load_from_str("default_agent = \"claude\"\nrepos_dir = \".repos\"\n").unwrap();
        let temp_dir = TempDir::new().unwrap();
        let mut modal = ProjectCreationModal::new(&config, temp_dir.path(), Some("payment-ws"));
        assert_eq!(modal.focus, CreationFocus::ProjectName);
        assert_eq!(modal.project_name_input, "");
        assert_eq!(modal.prompt_input, "");

        // Type characters into project_name
        for c in "stripe-checkout".chars() {
            modal.handle_key(key(KeyCode::Char(c)));
        }
        assert_eq!(modal.project_name_input, "stripe-checkout");

        // Backspace
        modal.handle_key(key(KeyCode::Backspace));
        assert_eq!(modal.project_name_input, "stripe-checkou");
        modal.handle_key(key(KeyCode::Char('t')));
        assert_eq!(modal.project_name_input, "stripe-checkout");

        // Switch to Agent field with Tab, then Prompt
        modal.handle_key(key(KeyCode::Tab));
        assert_eq!(modal.focus, CreationFocus::Agent);
        modal.handle_key(key(KeyCode::Tab));
        assert_eq!(modal.focus, CreationFocus::Prompt);

        // Type prompt
        for c in "Refactor webhook handlers".chars() {
            modal.handle_key(key(KeyCode::Char(c)));
        }
        assert_eq!(modal.prompt_input, "Refactor webhook handlers");

        // Switch back to Agent with Up arrow
        modal.handle_key(key(KeyCode::Up));
        assert_eq!(modal.focus, CreationFocus::Agent);

        // Switch back to Project Name with Up arrow
        modal.handle_key(key(KeyCode::Up));
        assert_eq!(modal.focus, CreationFocus::ProjectName);

        // Switch with Down arrow
        modal.handle_key(key(KeyCode::Down));
        assert_eq!(modal.focus, CreationFocus::Agent);

        // BackTab
        modal.handle_key(key(KeyCode::BackTab));
        assert_eq!(modal.focus, CreationFocus::ProjectName);
    }

    #[test]
    fn test_modal_submission_produces_create_project_action() {
        let mut state = DashboardState::new(vec!["cloud-infra".into()], vec![]);
        state.open_creation_modal();

        // 1. Trying to submit with empty project name returns DashboardAction::None (stays open)
        let action = state.handle_key(key(KeyCode::Enter));
        assert_eq!(action, DashboardAction::None);
        assert!(state.creation_modal.is_some());

        // 2. Enter project name and prompt
        for c in "k8s-cluster".chars() {
            state.handle_key(key(KeyCode::Char(c)));
        }
        state.handle_key(key(KeyCode::Tab)); // switch to Agent
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
                prompt: Some("Deploy helm chart".to_string()),
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

        for c in "my-project".chars() {
            state.handle_key(key(KeyCode::Char(c)));
        }

        let action = state.handle_key(key(KeyCode::Enter));
        assert_eq!(
            action,
            DashboardAction::CreateProject {
                template: "ws".to_string(),
                project: "my-project".to_string(),
                prompt: None,
            }
        );
    }

    #[test]
    fn test_attach_action_from_left_panel() {
        let mut state = DashboardState::new(
            vec!["ws".into()],
            vec!["proj-alpha".into(), "proj-beta".into()],
        );
        state.active_panel = ActivePanel::Projects;
        state.selected_project_idx = 1;

        let action = state.handle_key(key(KeyCode::Enter));
        assert_eq!(action, DashboardAction::Attach("proj-beta".to_string()));
    }

    #[test]
    fn test_attach_action_on_empty_projects_does_nothing() {
        let mut state = DashboardState::new(vec!["ws".into()], vec![]);
        state.active_panel = ActivePanel::Projects;

        let action = state.handle_key(key(KeyCode::Enter));
        assert_eq!(action, DashboardAction::None);
    }

    #[test]
    fn test_destroy_action_from_left_panel_confirmation_flow() {
        let mut state = DashboardState::new(
            vec!["ws".into()],
            vec!["proj-to-delete".into(), "proj-safe".into()],
        );
        state.active_panel = ActivePanel::Projects;
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

        // Open again and confirm with 'y'
        state.handle_key(key(KeyCode::Char('d')));
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

        // On Templates panel, 'd' key does nothing and doesn't open modal
        state.switch_panel();
        let action = state.handle_key(key(KeyCode::Char('d')));
        assert_eq!(action, DashboardAction::None);
        assert!(state.destroy_modal.is_none());
    }

    #[test]
    fn test_render_destroy_modal() {
        let mut state = DashboardState::new(
            vec!["ws".into()],
            vec!["legacy-app".into()],
        );
        state.destroy_modal = Some(DestroyModalState::new("legacy-app"));

        let area = Rect::new(0, 0, 100, 30);
        let mut buf = Buffer::empty(area);
        let theme = Theme::default();

        render_dashboard(&state, area, &mut buf, &theme);
        let rendered = buf
            .content()
            .iter()
            .map(|cell| cell.symbol())
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
        assert_eq!(state.handle_key(key(KeyCode::Esc)), DashboardAction::Quit);
    }

    #[test]
    fn test_creation_modal_and_dashboard_cursor_position() {
        let config = Config::load_from_str("default_agent = \"claude\"\nrepos_dir = \".repos\"\n").unwrap();
        let temp_dir = TempDir::new().unwrap();
        let area = Rect::new(0, 0, 100, 30);
        let mut modal = ProjectCreationModal::new(&config, temp_dir.path(), Some("ecommerce"));

        // 1. Focus on ProjectName empty
        modal.focus = CreationFocus::ProjectName;
        assert!(modal.cursor_position(area).is_some());

        // Field with text "alpha"
        modal.project_name_input = "alpha".to_string();
        assert!(modal.cursor_position(area).is_some());

        // 2. Switch to Prompt
        modal.focus = CreationFocus::Prompt;
        assert!(modal.cursor_position(area).is_some());

        // 3. Small area returns None
        let small_area = Rect::new(0, 0, 15, 5);
        assert_eq!(modal.cursor_position(small_area), None);

        // 4. DashboardState delegation
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
            .map(|cell| cell.symbol())
            .collect::<String>();

        // Check that project names and status dot indicators are rendered
        assert!(rendered.contains("●"));
        assert!(rendered.contains("proj-busy"));
        assert!(rendered.contains("proj-wait"));
        assert!(rendered.contains("proj-done"));
        assert!(rendered.contains("proj-err"));
        assert!(rendered.contains("proj-inactive"));

        // Verify the status dot colors on specific rows in the left panel
        // Left inner area starts at x = 1, y = 2 (inside borders, since header is at y = 0, panel at y = 1..20)
        // Row 1 (y = 2): proj-busy (selected, active panel) -> prefix is " > ", dot is at x = 4 (index 3 inside inner: 1 + 3 = 4)
        // Let's verify line content and cell colors:
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
        let state = DashboardState::new(vec!["ws".to_string()], projects)
            .with_running_sessions(running);

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
            vec!["ws1".to_string(), "ws2".to_string(), "ws3".to_string()],
            vec!["proj1".to_string(), "proj2".to_string(), "proj3".to_string()],
        );

        assert_eq!(state.active_panel, ActivePanel::Projects);
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

        // ScrollDown in Projects panel
        assert_eq!(state.handle_mouse(scroll_down), DashboardAction::None);
        assert_eq!(state.selected_project_idx, 1);

        assert_eq!(state.handle_mouse(scroll_down), DashboardAction::None);
        assert_eq!(state.selected_project_idx, 2);

        // ScrollUp in Projects panel
        assert_eq!(state.handle_mouse(scroll_up), DashboardAction::None);
        assert_eq!(state.selected_project_idx, 1);

        // Switch panel to Templates
        state.switch_panel();
        assert_eq!(state.active_panel, ActivePanel::Templates);
        assert_eq!(state.selected_template_idx, 0);

        // ScrollDown in Templates panel
        assert_eq!(state.handle_mouse(scroll_down), DashboardAction::None);
        assert_eq!(state.selected_template_idx, 1);

        // ScrollUp in Templates panel
        assert_eq!(state.handle_mouse(scroll_up), DashboardAction::None);
        assert_eq!(state.selected_template_idx, 0);

        // Non-scroll mouse event
        let click = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 10,
            row: 10,
            modifiers: KeyModifiers::empty(),
        };
        assert_eq!(state.handle_mouse(click), DashboardAction::None);
    }
}
