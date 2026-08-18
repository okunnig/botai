use std::path::Path;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Position, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Widget},
};

use crate::config::Config;

/// Active panel in the two-panel dashboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivePanel {
    Projects,
    Workspaces,
}

/// Actions emitted by user interactions with the dashboard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DashboardAction {
    None,
    Attach(String),
    CreateProject {
        workspace: String,
        project: String,
        prompt: Option<String>,
    },
    Destroy(String),
    Quit,
}

/// State for the floating project creation modal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreationModalState {
    pub workspace_name: String,
    pub project_name_input: String,
    pub prompt_input: String,
    pub active_field: usize, // 0: project name, 1: initial prompt
}

impl CreationModalState {
    /// Creates a new `CreationModalState` targeting the given workspace.
    pub fn new(workspace_name: impl Into<String>) -> Self {
        Self {
            workspace_name: workspace_name.into(),
            project_name_input: String::new(),
            prompt_input: String::new(),
            active_field: 0,
        }
    }

    /// Calculates the hardware cursor position within the given screen area, if visible.
    pub fn cursor_position(&self, area: Rect) -> Option<Position> {
        let width = 64.min(area.width.saturating_sub(4));
        let height = 9.min(area.height.saturating_sub(2));

        if width < 20 || height < 6 {
            return None;
        }

        let x = area.x + (area.width.saturating_sub(width)) / 2;
        let y = area.y + (area.height.saturating_sub(height)) / 2;
        let modal_area = Rect::new(x, y, width, height);

        let block = Block::default().borders(Borders::ALL);
        let inner = block.inner(modal_area);

        if self.active_field == 0 {
            // Label is "▶ Project Name: " (width 16)
            let prefix_len = 16u16;
            let cursor_x = inner.x + prefix_len + self.project_name_input.chars().count() as u16;
            let cursor_y = inner.y;
            if cursor_x < inner.x + inner.width && cursor_y < inner.y + inner.height {
                Some(Position::new(cursor_x, cursor_y))
            } else {
                Some(Position::new(
                    cursor_x.min(inner.x + inner.width.saturating_sub(1)),
                    cursor_y,
                ))
            }
        } else if self.active_field == 1 {
            // Label is "▶ Initial Prompt: " (width 18)
            let prefix_len = 18u16;
            let cursor_x = inner.x + prefix_len + self.prompt_input.chars().count() as u16;
            let cursor_y = inner.y + 2;
            if cursor_x < inner.x + inner.width && cursor_y < inner.y + inner.height {
                Some(Position::new(cursor_x, cursor_y))
            } else {
                Some(Position::new(
                    cursor_x.min(inner.x + inner.width.saturating_sub(1)),
                    cursor_y,
                ))
            }
        } else {
            None
        }
    }

    /// Handles keyboard events within the creation modal.
    pub fn handle_key(&mut self, key: KeyEvent) -> Option<DashboardAction> {
        match key.code {
            KeyCode::Tab | KeyCode::Down => {
                self.active_field = (self.active_field + 1) % 2;
                None
            }
            KeyCode::BackTab | KeyCode::Up => {
                self.active_field = if self.active_field == 0 { 1 } else { 0 };
                None
            }
            KeyCode::Char(c) => {
                if self.active_field == 0 {
                    self.project_name_input.push(c);
                } else {
                    self.prompt_input.push(c);
                }
                None
            }
            KeyCode::Backspace => {
                if self.active_field == 0 {
                    self.project_name_input.pop();
                } else {
                    self.prompt_input.pop();
                }
                None
            }
            KeyCode::Enter => {
                let trimmed_name = self.project_name_input.trim();
                if trimmed_name.is_empty() {
                    None
                } else {
                    let prompt = if self.prompt_input.trim().is_empty() {
                        None
                    } else {
                        Some(self.prompt_input.trim().to_string())
                    };
                    Some(DashboardAction::CreateProject {
                        workspace: self.workspace_name.clone(),
                        project: trimmed_name.to_string(),
                        prompt,
                    })
                }
            }
            _ => None,
        }
    }
}

/// Full state for the interactive two-panel launcher dashboard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DashboardState {
    pub workspaces: Vec<String>,
    pub local_projects: Vec<String>,
    pub active_panel: ActivePanel,
    pub selected_project_idx: usize,
    pub selected_workspace_idx: usize,
    pub creation_modal: Option<CreationModalState>,
}

impl DashboardState {
    /// Creates a new `DashboardState` with given workspaces and local projects.
    pub fn new(workspaces: Vec<String>, local_projects: Vec<String>) -> Self {
        Self {
            workspaces,
            local_projects,
            active_panel: ActivePanel::Projects,
            selected_project_idx: 0,
            selected_workspace_idx: 0,
            creation_modal: None,
        }
    }

    /// Scans the CWD for existing project directories containing `AGENTS.md` and loads workspaces from `Config`.
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

        let mut workspaces: Vec<String> = config.workspaces.keys().cloned().collect();
        workspaces.sort();

        Self::new(workspaces, local_projects)
    }

    /// Toggles active panel between Projects and Workspaces.
    pub fn switch_panel(&mut self) {
        self.active_panel = match self.active_panel {
            ActivePanel::Projects => ActivePanel::Workspaces,
            ActivePanel::Workspaces => ActivePanel::Projects,
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
            ActivePanel::Workspaces => {
                if !self.workspaces.is_empty()
                    && self.selected_workspace_idx + 1 < self.workspaces.len()
                {
                    self.selected_workspace_idx += 1;
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
            ActivePanel::Workspaces => {
                self.selected_workspace_idx = self.selected_workspace_idx.saturating_sub(1);
            }
        }
    }

    /// Opens the project creation modal for the currently selected workspace.
    pub fn open_creation_modal(&mut self) {
        if !self.workspaces.is_empty() {
            let ws_idx = self
                .selected_workspace_idx
                .min(self.workspaces.len().saturating_sub(1));
            let ws = self.workspaces[ws_idx].clone();
            self.creation_modal = Some(CreationModalState::new(ws));
        }
    }

    /// Closes the project creation modal.
    pub fn close_creation_modal(&mut self) {
        self.creation_modal = None;
    }

    /// Returns the hardware cursor position if a modal or input field is actively focused.
    pub fn cursor_position(&self, area: Rect) -> Option<Position> {
        self.creation_modal
            .as_ref()
            .and_then(|modal| modal.cursor_position(area))
    }

    /// Handles a keyboard event and returns any triggered `DashboardAction`.
    pub fn handle_key(&mut self, key: KeyEvent) -> DashboardAction {
        if let Some(mut modal) = self.creation_modal.take() {
            if key.code == KeyCode::Esc {
                return DashboardAction::None;
            }
            let res = modal.handle_key(key);
            if let Some(action) = res {
                return action;
            } else {
                self.creation_modal = Some(modal);
                return DashboardAction::None;
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
                    DashboardAction::Destroy(self.local_projects[idx].clone())
                } else {
                    DashboardAction::None
                }
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
                ActivePanel::Workspaces => {
                    self.open_creation_modal();
                    DashboardAction::None
                }
            },
            KeyCode::Char('q') | KeyCode::Esc => DashboardAction::Quit,
            _ => DashboardAction::None,
        }
    }
}

/// Renders the full two-panel dashboard, status footer, and optional floating creation modal into the given `Buffer`.
pub fn render_dashboard(state: &DashboardState, area: Rect, buf: &mut Buffer) {
    if area.height < 3 || area.width < 10 {
        return;
    }

    // Split area into Main area and Status Bar footer
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(1)])
        .split(area);

    let main_area = chunks[0];
    let footer_area = chunks[1];

    // Split main area into Left (Projects) and Right (Workspaces)
    let panel_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(main_area);

    let left_area = panel_chunks[0];
    let right_area = panel_chunks[1];

    // 1. Render Left Panel (Local Projects)
    let is_left_active = state.active_panel == ActivePanel::Projects;
    let left_border_style = if is_left_active {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let left_block = Block::default()
        .borders(Borders::ALL)
        .title(" Local Projects (Existing) ")
        .border_style(left_border_style);

    let left_inner = left_block.inner(left_area);
    left_block.render(left_area, buf);

    if state.local_projects.is_empty() {
        let msg = Paragraph::new(" (No local projects found in CWD) ")
            .style(Style::default().fg(Color::DarkGray));
        msg.render(left_inner, buf);
    } else {
        let items: Vec<ListItem> = state
            .local_projects
            .iter()
            .enumerate()
            .map(|(i, proj)| {
                let is_selected = i == state.selected_project_idx;
                let (prefix, style) = if is_selected && is_left_active {
                    (
                        " > ",
                        Style::default()
                            .fg(Color::Black)
                            .bg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    )
                } else if is_selected {
                    (" > ", Style::default().fg(Color::Cyan))
                } else {
                    ("   ", Style::default().fg(Color::White))
                };
                ListItem::new(format!("{}{}", prefix, proj)).style(style)
            })
            .collect();

        let list = List::new(items);
        list.render(left_inner, buf);
    }

    // 2. Render Right Panel (Configured Workspaces)
    let is_right_active = state.active_panel == ActivePanel::Workspaces;
    let right_border_style = if is_right_active {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let right_block = Block::default()
        .borders(Borders::ALL)
        .title(" Configured Workspaces ")
        .border_style(right_border_style);

    let right_inner = right_block.inner(right_area);
    right_block.render(right_area, buf);

    if state.workspaces.is_empty() {
        let msg = Paragraph::new(" (No workspaces found in config) ")
            .style(Style::default().fg(Color::DarkGray));
        msg.render(right_inner, buf);
    } else {
        let items: Vec<ListItem> = state
            .workspaces
            .iter()
            .enumerate()
            .map(|(i, ws)| {
                let is_selected = i == state.selected_workspace_idx;
                let (prefix, style) = if is_selected && is_right_active {
                    (
                        " > ",
                        Style::default()
                            .fg(Color::Black)
                            .bg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    )
                } else if is_selected {
                    (" > ", Style::default().fg(Color::Cyan))
                } else {
                    ("   ", Style::default().fg(Color::White))
                };
                ListItem::new(format!("{}{}", prefix, ws)).style(style)
            })
            .collect();

        let list = List::new(items);
        list.render(right_inner, buf);
    }

    // 3. Render Status Footer
    let footer_style = Style::default().bg(Color::DarkGray).fg(Color::White);
    let footer_text =
        " Tab: Switch | ↑/↓: Navigate | Enter: Select | n: New | d: Destroy | q: Quit ";
    let footer = Paragraph::new(footer_text).style(footer_style);
    footer.render(footer_area, buf);

    // 4. Render Floating Creation Modal if active
    if let Some(ref modal) = state.creation_modal {
        render_creation_modal(modal, area, buf);
    }
}

fn render_creation_modal(modal: &CreationModalState, area: Rect, buf: &mut Buffer) {
    let width = 64.min(area.width.saturating_sub(4));
    let height = 9.min(area.height.saturating_sub(2));

    if width < 20 || height < 6 {
        return;
    }

    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let modal_area = Rect::new(x, y, width, height);

    // Clear background of modal area
    Clear.render(modal_area, buf);

    let title = format!(" Create New Project [{}] ", modal.workspace_name);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        );

    let inner = block.inner(modal_area);
    block.render(modal_area, buf);

    let proj_style = if modal.active_field == 0 {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White)
    };

    let prompt_style = if modal.active_field == 1 {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White)
    };

    let proj_prefix = if modal.active_field == 0 {
        "▶ "
    } else {
        "  "
    };
    let prompt_prefix = if modal.active_field == 1 {
        "▶ "
    } else {
        "  "
    };

    let lines = vec![
        Line::from(vec![
            Span::styled(format!("{}Project Name: ", proj_prefix), proj_style),
            Span::styled(
                &modal.project_name_input,
                Style::default().fg(Color::Yellow),
            ),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled(format!("{}Initial Prompt: ", prompt_prefix), prompt_style),
            Span::styled(&modal.prompt_input, Style::default().fg(Color::Yellow)),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            " [Tab] Next Field  |  [Enter] Create  |  [Esc] Cancel ",
            Style::default().fg(Color::DarkGray),
        )),
    ];

    let paragraph = Paragraph::new(lines);
    paragraph.render(inner, buf);
}

impl Widget for &DashboardState {
    fn render(self, area: Rect, buf: &mut Buffer) {
        render_dashboard(self, area, buf);
    }
}

impl Widget for DashboardState {
    fn render(self, area: Rect, buf: &mut Buffer) {
        render_dashboard(&self, area, buf);
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
    fn test_dashboard_state_initialization() {
        let workspaces = vec!["ecommerce".to_string(), "analytics".to_string()];
        let local_projects = vec!["feature-checkout".to_string(), "hotfix-auth".to_string()];

        let state = DashboardState::new(workspaces.clone(), local_projects.clone());

        assert_eq!(state.active_panel, ActivePanel::Projects);
        assert_eq!(state.workspaces, workspaces);
        assert_eq!(state.local_projects, local_projects);
        assert_eq!(state.selected_project_idx, 0);
        assert_eq!(state.selected_workspace_idx, 0);
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

[workspaces.backend]
repos = ["/path/repo1"]

[workspaces.frontend]
repos = ["/path/repo2"]
"#;
        let config = Config::load_from_str(toml_content).unwrap();
        let state = DashboardState::from_env(&config, cwd);

        assert_eq!(
            state.local_projects,
            vec!["alpha-service".to_string(), "beta-service".to_string()]
        );
        assert_eq!(
            state.workspaces,
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
        assert!(state.workspaces.is_empty());

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
        assert_eq!(state.active_panel, ActivePanel::Workspaces);

        state.switch_panel();
        assert_eq!(state.active_panel, ActivePanel::Projects);

        // Switch via key events: Tab, Left, Right
        let action = state.handle_key(key(KeyCode::Tab));
        assert_eq!(action, DashboardAction::None);
        assert_eq!(state.active_panel, ActivePanel::Workspaces);

        let action = state.handle_key(key(KeyCode::Left));
        assert_eq!(action, DashboardAction::None);
        assert_eq!(state.active_panel, ActivePanel::Projects);

        let action = state.handle_key(key(KeyCode::Right));
        assert_eq!(action, DashboardAction::None);
        assert_eq!(state.active_panel, ActivePanel::Workspaces);
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

        // Up moves back
        state.handle_key(key(KeyCode::Up));
        assert_eq!(state.selected_project_idx, 0);

        // Up at boundary stays at 0
        state.handle_key(key(KeyCode::Up));
        assert_eq!(state.selected_project_idx, 0);

        // Switch to Workspaces
        state.switch_panel();
        assert_eq!(state.selected_workspace_idx, 0);

        state.handle_key(key(KeyCode::Char('j'))); // vim down
        assert_eq!(state.selected_workspace_idx, 1);
        state.handle_key(key(KeyCode::Char('j')));
        assert_eq!(state.selected_workspace_idx, 2);
        state.handle_key(key(KeyCode::Char('j'))); // boundary
        assert_eq!(state.selected_workspace_idx, 2);

        state.handle_key(key(KeyCode::Char('k'))); // vim up
        assert_eq!(state.selected_workspace_idx, 1);
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
        assert_eq!(state.selected_workspace_idx, 0);
        state.prev_item();
        assert_eq!(state.selected_workspace_idx, 0);
    }

    #[test]
    fn test_modal_opening_and_closing() {
        let mut state = DashboardState::new(
            vec!["ecommerce".into(), "analytics".into()],
            vec!["proj1".into()],
        );

        // When in Workspaces panel, pressing Enter or 'n' opens modal
        state.active_panel = ActivePanel::Workspaces;
        state.selected_workspace_idx = 1;

        let action = state.handle_key(key(KeyCode::Enter));
        assert_eq!(action, DashboardAction::None);
        assert!(state.creation_modal.is_some());
        assert_eq!(
            state.creation_modal.as_ref().unwrap().workspace_name,
            "analytics"
        );

        // Pressing Esc closes modal
        let action = state.handle_key(key(KeyCode::Esc));
        assert_eq!(action, DashboardAction::None);
        assert!(state.creation_modal.is_none());

        // Pressing 'n' from Projects panel also triggers modal for selected workspace
        state.active_panel = ActivePanel::Projects;
        state.selected_workspace_idx = 0;
        let action = state.handle_key(key(KeyCode::Char('n')));
        assert_eq!(action, DashboardAction::None);
        assert!(state.creation_modal.is_some());
        assert_eq!(
            state.creation_modal.as_ref().unwrap().workspace_name,
            "ecommerce"
        );

        state.close_creation_modal();
        assert!(state.creation_modal.is_none());
    }

    #[test]
    fn test_modal_opening_empty_workspaces() {
        let mut state = DashboardState::new(vec![], vec!["proj1".into()]);
        state.open_creation_modal();
        assert!(state.creation_modal.is_none());
    }

    #[test]
    fn test_modal_text_entry_editing_and_field_switching() {
        let mut modal = CreationModalState::new("payment-ws");
        assert_eq!(modal.active_field, 0);
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

        // Switch to Prompt field with Tab
        modal.handle_key(key(KeyCode::Tab));
        assert_eq!(modal.active_field, 1);

        // Type prompt
        for c in "Refactor webhook handlers".chars() {
            modal.handle_key(key(KeyCode::Char(c)));
        }
        assert_eq!(modal.prompt_input, "Refactor webhook handlers");

        // Switch back to Project Name with Up arrow
        modal.handle_key(key(KeyCode::Up));
        assert_eq!(modal.active_field, 0);

        // Switch with Down arrow
        modal.handle_key(key(KeyCode::Down));
        assert_eq!(modal.active_field, 1);

        // BackTab
        modal.handle_key(key(KeyCode::BackTab));
        assert_eq!(modal.active_field, 0);
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
        state.handle_key(key(KeyCode::Tab)); // switch to prompt
        for c in "Deploy helm chart".chars() {
            state.handle_key(key(KeyCode::Char(c)));
        }

        let action = state.handle_key(key(KeyCode::Enter));
        assert_eq!(
            action,
            DashboardAction::CreateProject {
                workspace: "cloud-infra".to_string(),
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
                workspace: "ws".to_string(),
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
    fn test_destroy_action_from_left_panel() {
        let mut state = DashboardState::new(
            vec!["ws".into()],
            vec!["proj-to-delete".into(), "proj-safe".into()],
        );
        state.active_panel = ActivePanel::Projects;
        state.selected_project_idx = 0;

        let action = state.handle_key(key(KeyCode::Char('d')));
        assert_eq!(
            action,
            DashboardAction::Destroy("proj-to-delete".to_string())
        );

        // On Workspaces panel, 'd' key does nothing
        state.switch_panel();
        let action = state.handle_key(key(KeyCode::Char('d')));
        assert_eq!(action, DashboardAction::None);
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
    fn test_render_dashboard_panels_and_footer() {
        let state = DashboardState::new(
            vec!["payments".to_string(), "analytics".to_string()],
            vec!["cart-service".to_string(), "billing-api".to_string()],
        );

        let area = Rect::new(0, 0, 100, 20);
        let mut buf = Buffer::empty(area);

        render_dashboard(&state, area, &mut buf);

        let rendered = buf
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();

        assert!(rendered.contains("Local Projects (Existing)"));
        assert!(rendered.contains("Configured Workspaces"));
        assert!(rendered.contains("cart-service"));
        assert!(rendered.contains("billing-api"));
        assert!(rendered.contains("payments"));
        assert!(rendered.contains("analytics"));
        assert!(rendered.contains("Tab: Switch"));
        assert!(rendered.contains("q: Quit"));
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

        render_dashboard(&state, area, &mut buf);

        let rendered = buf
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();

        assert!(rendered.contains("Create New Project [payments]"));
        assert!(rendered.contains("Project Name:"));
        assert!(rendered.contains("new-checkout"));
        assert!(rendered.contains("Initial Prompt:"));
        assert!(rendered.contains("Implement Stripe"));
        assert!(rendered.contains("[Enter] Create"));
    }

    #[test]
    fn test_render_empty_state() {
        let state = DashboardState::new(vec![], vec![]);

        let area = Rect::new(0, 0, 100, 20);
        let mut buf = Buffer::empty(area);

        render_dashboard(&state, area, &mut buf);

        let rendered = buf
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();

        assert!(rendered.contains("(No local projects found in CWD)"));
        assert!(rendered.contains("(No workspaces found in config)"));
    }

    #[test]
    fn test_unicode_and_special_character_support() {
        let mut state = DashboardState::new(
            vec!["воркспейс-1 🚀".to_string()],
            vec!["проект-тест 💻".to_string()],
        );

        let area = Rect::new(0, 0, 100, 20);
        let mut buf = Buffer::empty(area);
        render_dashboard(&state, area, &mut buf);

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
                workspace: "воркспейс-1 🚀".to_string(),
                project: "проект-юникод".to_string(),
                prompt: None,
            }
        );
    }

    #[test]
    fn test_creation_modal_and_dashboard_cursor_position() {
        let area = Rect::new(0, 0, 100, 30);
        let mut modal = CreationModalState::new("ecommerce");

        // 1. Field 0 (Project Name) empty
        assert_eq!(modal.cursor_position(area), Some(Position::new(35, 11)));

        // Field 0 with text "alpha"
        modal.project_name_input = "alpha".to_string();
        assert_eq!(modal.cursor_position(area), Some(Position::new(40, 11)));

        // 2. Switch to Field 1 (Prompt) empty
        modal.active_field = 1;
        assert_eq!(modal.cursor_position(area), Some(Position::new(37, 13)));

        // Field 1 with text "do something"
        modal.prompt_input = "do something".to_string();
        assert_eq!(modal.cursor_position(area), Some(Position::new(49, 13)));

        // 3. Small area returns None
        let small_area = Rect::new(0, 0, 15, 5);
        assert_eq!(modal.cursor_position(small_area), None);

        // 4. DashboardState delegation
        let mut dashboard = DashboardState::new(vec!["ecommerce".into()], vec![]);
        assert_eq!(dashboard.cursor_position(area), None);

        dashboard.open_creation_modal();
        assert_eq!(
            dashboard.cursor_position(area),
            Some(Position::new(35, 11))
        );
    }
}
