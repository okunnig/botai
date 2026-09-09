use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Position, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Widget},
};

use crate::config::Config;
use crate::session::SessionStatus;
use crate::tui::creation_modal::ProjectCreationModal;
use crate::tui::theme::Theme;

/// Active top-level tab in the Quick Launcher modal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LauncherTab {
    Create,
    Attach,
}

/// Active focused field or interactive area in the Quick Launcher modal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LauncherFocus {
    TabBar,
    Template,
    ProjectName,
    Prompt,
    ProjectList,
}

/// Actions emitted by user interactions with the Quick Launcher modal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuickLauncherAction {
    None,
    Create {
        template: String,
        project: String,
        agent_override: Option<String>,
        prompt: Option<String>,
    },
    Attach(String),
    Cancel,
}

/// State for the in-session floating Quick Launcher modal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuickLauncherState {
    pub tab: LauncherTab,
    pub focus: LauncherFocus,
    pub templates: Vec<String>,
    pub local_projects: Vec<String>,
    pub running_sessions: HashMap<String, SessionStatus>,
    pub selected_project_idx: usize,
    pub error_message: Option<String>,
    pub creation_modal: ProjectCreationModal,
}

impl QuickLauncherState {
    /// Creates a new `QuickLauncherState` with available templates and local projects.
    /// Defaults selected template to `default_template` if provided and found.
    pub fn new(
        templates: Vec<String>,
        local_projects: Vec<String>,
        default_template: Option<&str>,
    ) -> Self {
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
                    repos: Vec::new(),
                    env: HashMap::new(),
                    post_create: Vec::new(),
                    pre_destroy: Vec::new(),
                });
        }
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let mut creation_modal = ProjectCreationModal::new(&config, &cwd, default_template);
        creation_modal.step = crate::tui::creation_modal::CreationStep::ConfigureDetails;

        Self {
            tab: LauncherTab::Create,
            focus: LauncherFocus::Template,
            templates,
            local_projects,
            running_sessions: HashMap::new(),
            selected_project_idx: 0,
            error_message: None,
            creation_modal,
        }
    }

    /// Creates a `QuickLauncherState` directly from an existing `Config` and working directory.
    pub fn from_config(
        config: &Config,
        cwd: &Path,
        local_projects: Vec<String>,
        default_template: Option<&str>,
    ) -> Self {
        let mut templates: Vec<String> = config.templates.keys().cloned().collect();
        templates.sort();

        let mut creation_modal = ProjectCreationModal::new(config, cwd, default_template);
        creation_modal.step = crate::tui::creation_modal::CreationStep::ConfigureDetails;

        Self {
            tab: LauncherTab::Create,
            focus: LauncherFocus::Template,
            templates,
            local_projects,
            running_sessions: HashMap::new(),
            selected_project_idx: 0,
            error_message: None,
            creation_modal,
        }
    }

    /// Sets the map of currently running sessions.
    pub fn with_running_sessions(mut self, running: HashMap<String, SessionStatus>) -> Self {
        self.running_sessions = running;
        self
    }

    /// Updates the map of currently running sessions in-place.
    pub fn set_running_sessions(&mut self, running: HashMap<String, SessionStatus>) {
        self.running_sessions = running;
    }

    /// Calculates the hardware cursor position when an editable input field is focused.
    pub fn cursor_position(&self, area: Rect) -> Option<Position> {
        let width = 72.min(area.width.saturating_sub(4));
        let height = 16.min(area.height.saturating_sub(2));

        if width < 25 || height < 8 {
            return None;
        }

        let x = area.x + (area.width.saturating_sub(width)) / 2;
        let y = area.y + (area.height.saturating_sub(height)) / 2;
        let modal_area = Rect::new(x, y, width, height);

        let block = Block::default().borders(Borders::ALL);
        let inner = block.inner(modal_area);

        if inner.height < 5 || inner.width < 20 {
            return None;
        }

        let has_error = self.error_message.is_some();
        let constraints = if has_error {
            vec![
                Constraint::Length(1), // TabBar
                Constraint::Length(1), // Divider
                Constraint::Min(4),    // Body
                Constraint::Length(1), // Error banner
                Constraint::Length(1), // Footer hints
            ]
        } else {
            vec![
                Constraint::Length(1), // TabBar
                Constraint::Length(1), // Divider
                Constraint::Min(4),    // Body
                Constraint::Length(1), // Footer hints
            ]
        };

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(inner);

        let body_area = chunks[2];

        if self.tab == LauncherTab::Create {
            match self.focus {
                LauncherFocus::ProjectName => {
                    let prefix_len = 16u16;
                    let total_chars = self.creation_modal.project_name_input.chars().count();
                    let cursor_offset =
                        self.creation_modal.project_name_cursor.min(total_chars) as u16;
                    let cursor_x = body_area.x + prefix_len + cursor_offset;
                    let cursor_y = body_area.y + 2;
                    if cursor_x < body_area.x + body_area.width
                        && cursor_y < body_area.y + body_area.height
                    {
                        Some(Position::new(cursor_x, cursor_y))
                    } else {
                        Some(Position::new(
                            cursor_x.min(body_area.x + body_area.width.saturating_sub(1)),
                            cursor_y,
                        ))
                    }
                }
                LauncherFocus::Prompt => {
                    let prompt_text_width = (body_area.width.saturating_sub(18) as usize).max(10);
                    let (line_idx, col_idx) =
                        self.creation_modal.cursor_to_line_col(prompt_text_width);
                    let scroll_offset = self.creation_modal.prompt_scroll_offset.min(line_idx);
                    let visible_line = line_idx.saturating_sub(scroll_offset);
                    if visible_line < crate::tui::creation_modal::PROMPT_VISIBLE_LINES {
                        let cursor_x = body_area.x + 18 + col_idx as u16;
                        let cursor_y = body_area.y + 4 + visible_line as u16;
                        if cursor_x < body_area.x + body_area.width
                            && cursor_y < body_area.y + body_area.height
                        {
                            Some(Position::new(cursor_x, cursor_y))
                        } else {
                            Some(Position::new(
                                cursor_x.min(body_area.x + body_area.width.saturating_sub(1)),
                                cursor_y.min(body_area.y + body_area.height.saturating_sub(1)),
                            ))
                        }
                    } else {
                        None
                    }
                }
                _ => None,
            }
        } else {
            None
        }
    }

    /// Sets an error message to display in the modal's error banner.
    pub fn set_error(&mut self, err: impl Into<String>) {
        self.error_message = Some(err.into());
    }

    /// Clears any currently displayed error message.
    pub fn clear_error(&mut self) {
        self.error_message = None;
    }

    /// Returns the currently selected template name, if available.
    pub fn selected_template(&self) -> Option<&str> {
        self.creation_modal.selected_template()
    }

    /// Returns the currently selected local project name, if available.
    pub fn selected_project(&self) -> Option<&str> {
        if !self.local_projects.is_empty() && self.selected_project_idx < self.local_projects.len()
        {
            Some(&self.local_projects[self.selected_project_idx])
        } else {
            None
        }
    }

    /// Cycles to the next template.
    pub fn cycle_template_next(&mut self) {
        self.creation_modal.cycle_template_next();
    }

    /// Cycles to the previous template.
    pub fn cycle_template_prev(&mut self) {
        self.creation_modal.cycle_template_prev();
    }

    /// Polls pending background validation checks in the embedded creation modal.
    pub fn poll_validation(&mut self) {
        self.creation_modal.poll_validation();
    }

    /// Advances tick-based state and background validation polling.
    pub fn tick(&mut self) {
        self.poll_validation();
    }

    /// Switches the active modal tab.
    pub fn switch_tab(&mut self, new_tab: LauncherTab) {
        self.tab = new_tab;
        match new_tab {
            LauncherTab::Create => {
                if self.focus == LauncherFocus::ProjectList {
                    self.focus = LauncherFocus::Template;
                }
            }
            LauncherTab::Attach => {
                if self.focus != LauncherFocus::TabBar {
                    self.focus = LauncherFocus::ProjectList;
                }
            }
        }
    }

    /// Moves focus to the next field/tab.
    pub fn next_focus(&mut self) {
        match self.tab {
            LauncherTab::Create => {
                self.focus = match self.focus {
                    LauncherFocus::TabBar => LauncherFocus::Template,
                    LauncherFocus::Template => LauncherFocus::ProjectName,
                    LauncherFocus::ProjectName => LauncherFocus::Prompt,
                    LauncherFocus::Prompt => LauncherFocus::TabBar,
                    LauncherFocus::ProjectList => LauncherFocus::Template,
                };
            }
            LauncherTab::Attach => {
                self.focus = match self.focus {
                    LauncherFocus::TabBar => LauncherFocus::ProjectList,
                    LauncherFocus::ProjectList => LauncherFocus::TabBar,
                    _ => LauncherFocus::ProjectList,
                };
            }
        }
        match self.focus {
            LauncherFocus::Template => {
                self.creation_modal.focus = crate::tui::creation_modal::CreationFocus::Template
            }
            LauncherFocus::ProjectName => {
                self.creation_modal.focus = crate::tui::creation_modal::CreationFocus::ProjectName
            }
            LauncherFocus::Prompt => {
                self.creation_modal.focus = crate::tui::creation_modal::CreationFocus::Prompt
            }
            _ => {}
        }
    }

    /// Moves focus to the previous field/tab.
    pub fn prev_focus(&mut self) {
        match self.tab {
            LauncherTab::Create => {
                self.focus = match self.focus {
                    LauncherFocus::TabBar => LauncherFocus::Prompt,
                    LauncherFocus::Template => LauncherFocus::TabBar,
                    LauncherFocus::ProjectName => LauncherFocus::Template,
                    LauncherFocus::Prompt => LauncherFocus::ProjectName,
                    LauncherFocus::ProjectList => LauncherFocus::TabBar,
                };
            }
            LauncherTab::Attach => {
                self.focus = match self.focus {
                    LauncherFocus::TabBar => LauncherFocus::ProjectList,
                    LauncherFocus::ProjectList => LauncherFocus::TabBar,
                    _ => LauncherFocus::TabBar,
                };
            }
        }
        match self.focus {
            LauncherFocus::Template => {
                self.creation_modal.focus = crate::tui::creation_modal::CreationFocus::Template
            }
            LauncherFocus::ProjectName => {
                self.creation_modal.focus = crate::tui::creation_modal::CreationFocus::ProjectName
            }
            LauncherFocus::Prompt => {
                self.creation_modal.focus = crate::tui::creation_modal::CreationFocus::Prompt
            }
            _ => {}
        }
    }

    /// Handles a keyboard event and returns any triggered `QuickLauncherAction`.
    pub fn handle_key(&mut self, key: KeyEvent) -> QuickLauncherAction {
        self.poll_validation();

        let has_shift = key.modifiers.contains(KeyModifiers::SHIFT);
        let has_alt = key.modifiers.contains(KeyModifiers::ALT);
        let has_ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

        // Cancel / Dismiss modal via Esc or Ctrl+C
        if key.code == KeyCode::Esc
            || (has_ctrl && (key.code == KeyCode::Char('c') || key.code == KeyCode::Char('C')))
        {
            return QuickLauncherAction::Cancel;
        }

        // Special handling when Prompt input is focused on Create tab
        if self.tab == LauncherTab::Create && self.focus == LauncherFocus::Prompt {
            match key.code {
                KeyCode::Tab => {
                    self.next_focus();
                    return QuickLauncherAction::None;
                }
                KeyCode::BackTab => {
                    self.prev_focus();
                    return QuickLauncherAction::None;
                }
                KeyCode::Left => {
                    self.creation_modal.move_prompt_cursor_left();
                    return QuickLauncherAction::None;
                }
                KeyCode::Right => {
                    self.creation_modal.move_prompt_cursor_right();
                    return QuickLauncherAction::None;
                }
                KeyCode::Home => {
                    self.creation_modal.move_prompt_cursor_home();
                    return QuickLauncherAction::None;
                }
                KeyCode::End => {
                    self.creation_modal.move_prompt_cursor_end();
                    return QuickLauncherAction::None;
                }
                KeyCode::Up => {
                    if !self.creation_modal.move_prompt_cursor_up() {
                        self.prev_focus();
                    }
                    return QuickLauncherAction::None;
                }
                KeyCode::Down => {
                    if !self.creation_modal.move_prompt_cursor_down() {
                        self.next_focus();
                    }
                    return QuickLauncherAction::None;
                }
                KeyCode::Backspace => {
                    self.creation_modal.delete_prompt_char_before_cursor();
                    self.clear_error();
                    return QuickLauncherAction::None;
                }
                KeyCode::Delete => {
                    self.creation_modal.delete_prompt_char_at_cursor();
                    self.clear_error();
                    return QuickLauncherAction::None;
                }
                KeyCode::Enter | KeyCode::Char('\n') | KeyCode::Char('\r')
                    if has_shift || has_alt =>
                {
                    self.creation_modal.insert_newline_prompt();
                    self.clear_error();
                    return QuickLauncherAction::None;
                }
                KeyCode::Enter | KeyCode::Char('\n') | KeyCode::Char('\r')
                    if !has_shift && !has_alt =>
                {
                    let trimmed_name = self.creation_modal.project_name_input.trim();
                    if trimmed_name.is_empty() {
                        self.set_error("Project name cannot be empty");
                        return QuickLauncherAction::None;
                    } else if let Some(tpl) = self.selected_template() {
                        let prompt = if self.creation_modal.prompt_input.trim().is_empty() {
                            None
                        } else {
                            Some(self.creation_modal.prompt_input.trim().to_string())
                        };
                        return QuickLauncherAction::Create {
                            template: tpl.to_string(),
                            project: trimmed_name.to_string(),
                            agent_override: Some(self.creation_modal.selected_agent().to_string()),
                            prompt,
                        };
                    } else {
                        self.set_error("No template available");
                        return QuickLauncherAction::None;
                    }
                }
                KeyCode::Char(c) if has_ctrl => {
                    match c {
                        'a' | 'A' => self.creation_modal.move_prompt_cursor_home(),
                        'e' | 'E' => self.creation_modal.move_prompt_cursor_end(),
                        'w' | 'W' => {
                            self.creation_modal.delete_prompt_word_before_cursor();
                            self.clear_error();
                        }
                        'k' | 'K' => {
                            self.creation_modal.kill_to_end_of_prompt_line();
                            self.clear_error();
                        }
                        'u' | 'U' => {
                            self.creation_modal.clear_prompt_line_or_all();
                            self.clear_error();
                        }
                        _ => {}
                    }
                    return QuickLauncherAction::None;
                }
                KeyCode::Char(c) if !has_ctrl && !has_alt => {
                    self.creation_modal.insert_char_prompt(c);
                    self.clear_error();
                    return QuickLauncherAction::None;
                }
                _ => return QuickLauncherAction::None,
            }
        }

        if self.tab == LauncherTab::Create && self.focus == LauncherFocus::ProjectName {
            if (key.code == KeyCode::Left && (has_ctrl || has_alt))
                || ((key.code == KeyCode::Char('b') || key.code == KeyCode::Char('B')) && has_alt)
            {
                self.creation_modal.move_project_name_word_backward();
                return QuickLauncherAction::None;
            }
            if (key.code == KeyCode::Right && (has_ctrl || has_alt))
                || ((key.code == KeyCode::Char('f') || key.code == KeyCode::Char('F')) && has_alt)
            {
                self.creation_modal.move_project_name_word_forward();
                return QuickLauncherAction::None;
            }
            match key.code {
                KeyCode::Left => {
                    self.creation_modal.move_project_name_cursor_left();
                    return QuickLauncherAction::None;
                }
                KeyCode::Right => {
                    self.creation_modal.move_project_name_cursor_right();
                    return QuickLauncherAction::None;
                }
                KeyCode::Home => {
                    self.creation_modal.move_project_name_cursor_home();
                    return QuickLauncherAction::None;
                }
                KeyCode::End => {
                    self.creation_modal.move_project_name_cursor_end();
                    return QuickLauncherAction::None;
                }
                KeyCode::Backspace => {
                    self.creation_modal.delete_project_name_char_backward();
                    self.clear_error();
                    return QuickLauncherAction::None;
                }
                KeyCode::Delete => {
                    self.creation_modal.delete_project_name_char_forward();
                    self.clear_error();
                    return QuickLauncherAction::None;
                }
                KeyCode::Char(c) if has_ctrl => {
                    match c {
                        'a' | 'A' => self.creation_modal.move_project_name_cursor_home(),
                        'e' | 'E' => self.creation_modal.move_project_name_cursor_end(),
                        'u' | 'U' => {
                            self.creation_modal.delete_project_name_to_start();
                            self.clear_error();
                        }
                        'k' | 'K' => {
                            self.creation_modal.delete_project_name_to_end();
                            self.clear_error();
                        }
                        'w' | 'W' => {
                            self.creation_modal.delete_project_name_word_backward();
                            self.clear_error();
                        }
                        _ => {}
                    }
                    return QuickLauncherAction::None;
                }
                KeyCode::Char(c) if !has_ctrl && !has_alt => {
                    self.creation_modal.insert_project_name_char(c);
                    self.clear_error();
                    return QuickLauncherAction::None;
                }
                _ => {}
            }
        }

        match key.code {
            KeyCode::Tab => {
                self.next_focus();
                QuickLauncherAction::None
            }
            KeyCode::BackTab => {
                self.prev_focus();
                QuickLauncherAction::None
            }
            KeyCode::Left => {
                if self.focus == LauncherFocus::TabBar {
                    let next_tab = match self.tab {
                        LauncherTab::Create => LauncherTab::Attach,
                        LauncherTab::Attach => LauncherTab::Create,
                    };
                    self.switch_tab(next_tab);
                } else if self.tab == LauncherTab::Create && self.focus == LauncherFocus::Template {
                    self.cycle_template_prev();
                }
                QuickLauncherAction::None
            }
            KeyCode::Right => {
                if self.focus == LauncherFocus::TabBar {
                    let next_tab = match self.tab {
                        LauncherTab::Create => LauncherTab::Attach,
                        LauncherTab::Attach => LauncherTab::Create,
                    };
                    self.switch_tab(next_tab);
                } else if self.tab == LauncherTab::Create && self.focus == LauncherFocus::Template {
                    self.cycle_template_next();
                }
                QuickLauncherAction::None
            }
            KeyCode::Up => {
                match self.tab {
                    LauncherTab::Create => {
                        self.prev_focus();
                    }
                    LauncherTab::Attach => {
                        if self.focus == LauncherFocus::ProjectList {
                            self.selected_project_idx = self.selected_project_idx.saturating_sub(1);
                        } else {
                            self.focus = LauncherFocus::TabBar;
                        }
                    }
                }
                QuickLauncherAction::None
            }
            KeyCode::Down => {
                match self.tab {
                    LauncherTab::Create => {
                        self.next_focus();
                    }
                    LauncherTab::Attach => {
                        if self.focus == LauncherFocus::TabBar {
                            self.focus = LauncherFocus::ProjectList;
                        } else if self.focus == LauncherFocus::ProjectList
                            && !self.local_projects.is_empty()
                            && self.selected_project_idx + 1 < self.local_projects.len()
                        {
                            self.selected_project_idx += 1;
                        }
                    }
                }
                QuickLauncherAction::None
            }
            KeyCode::Char('k')
                if self.tab == LauncherTab::Attach
                    && self.focus == LauncherFocus::ProjectList
                    && !has_ctrl
                    && !has_alt =>
            {
                self.selected_project_idx = self.selected_project_idx.saturating_sub(1);
                QuickLauncherAction::None
            }
            KeyCode::Char('j')
                if self.tab == LauncherTab::Attach
                    && self.focus == LauncherFocus::ProjectList
                    && !has_ctrl
                    && !has_alt =>
            {
                if !self.local_projects.is_empty()
                    && self.selected_project_idx + 1 < self.local_projects.len()
                {
                    self.selected_project_idx += 1;
                }
                QuickLauncherAction::None
            }
            KeyCode::Char('1') if self.focus == LauncherFocus::TabBar && !has_ctrl && !has_alt => {
                self.switch_tab(LauncherTab::Create);
                QuickLauncherAction::None
            }
            KeyCode::Char('2') if self.focus == LauncherFocus::TabBar && !has_ctrl && !has_alt => {
                self.switch_tab(LauncherTab::Attach);
                QuickLauncherAction::None
            }
            KeyCode::Enter => match self.tab {
                LauncherTab::Create => {
                    let trimmed_name = self.creation_modal.project_name_input.trim();
                    if trimmed_name.is_empty() {
                        self.set_error("Project name cannot be empty");
                        QuickLauncherAction::None
                    } else if let Some(tpl) = self.selected_template() {
                        let prompt = if self.creation_modal.prompt_input.trim().is_empty() {
                            None
                        } else {
                            Some(self.creation_modal.prompt_input.trim().to_string())
                        };
                        QuickLauncherAction::Create {
                            template: tpl.to_string(),
                            project: trimmed_name.to_string(),
                            agent_override: Some(self.creation_modal.selected_agent().to_string()),
                            prompt,
                        }
                    } else {
                        self.set_error("No template available");
                        QuickLauncherAction::None
                    }
                }
                LauncherTab::Attach => {
                    if !self.local_projects.is_empty()
                        && self.selected_project_idx < self.local_projects.len()
                    {
                        let proj = self.local_projects[self.selected_project_idx].clone();
                        QuickLauncherAction::Attach(proj)
                    } else {
                        self.set_error("No local project available to attach");
                        QuickLauncherAction::None
                    }
                }
            },
            _ => QuickLauncherAction::None,
        }
    }

    /// Handles pasted text forwarded from the TUI event loop.
    pub fn handle_paste(&mut self, text: &str) {
        if self.tab == LauncherTab::Create {
            match self.focus {
                LauncherFocus::ProjectName => {
                    self.creation_modal.handle_paste(text);
                    self.clear_error();
                }
                LauncherFocus::Prompt => {
                    self.creation_modal.insert_str_prompt(text);
                    self.clear_error();
                }
                LauncherFocus::Template | LauncherFocus::TabBar | LauncherFocus::ProjectList => {}
            }
        }
    }

    /// Handles a mouse event (e.g. scroll wheel) and returns any triggered `QuickLauncherAction`.
    pub fn handle_mouse(&mut self, mouse: crossterm::event::MouseEvent) -> QuickLauncherAction {
        match mouse.kind {
            crossterm::event::MouseEventKind::ScrollUp => {
                if self.tab == LauncherTab::Attach && self.focus == LauncherFocus::ProjectList {
                    self.selected_project_idx = self.selected_project_idx.saturating_sub(1);
                } else if self.tab == LauncherTab::Create && self.focus == LauncherFocus::Template {
                    self.cycle_template_prev();
                } else if self.tab == LauncherTab::Create
                    && self.focus == LauncherFocus::Prompt
                    && !self.creation_modal.move_prompt_cursor_up()
                {
                    self.prev_focus();
                }
                QuickLauncherAction::None
            }
            crossterm::event::MouseEventKind::ScrollDown => {
                if self.tab == LauncherTab::Attach && self.focus == LauncherFocus::ProjectList {
                    if !self.local_projects.is_empty()
                        && self.selected_project_idx + 1 < self.local_projects.len()
                    {
                        self.selected_project_idx += 1;
                    }
                } else if self.tab == LauncherTab::Create && self.focus == LauncherFocus::Template {
                    self.cycle_template_next();
                } else if self.tab == LauncherTab::Create
                    && self.focus == LauncherFocus::Prompt
                    && !self.creation_modal.move_prompt_cursor_down()
                {
                    self.next_focus();
                }
                QuickLauncherAction::None
            }
            _ => QuickLauncherAction::None,
        }
    }
}

/// Renders the floating Quick Launcher modal on top of the given area in the buffer.
pub fn render_quick_launcher(
    state: &QuickLauncherState,
    area: Rect,
    buf: &mut Buffer,
    theme: &Theme,
) {
    let width = 72.min(area.width.saturating_sub(4));
    let height = 16.min(area.height.saturating_sub(2));

    if width < 25 || height < 8 {
        return;
    }

    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let modal_area = Rect::new(x, y, width, height);

    // Clear background
    Clear.render(modal_area, buf);

    // Outer container block
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Quick Project Launcher (Ctrl+Space c) ")
        .border_style(
            Style::default()
                .fg(theme.border_focused)
                .add_modifier(Modifier::BOLD),
        );

    let inner = block.inner(modal_area);
    block.render(modal_area, buf);

    if inner.height < 5 || inner.width < 20 {
        return;
    }

    // Split inner area into: TabBar, Spacing/Divider, Body, Error banner (if present), Footer hints
    let has_error = state.error_message.is_some();
    let constraints = if has_error {
        vec![
            Constraint::Length(1), // TabBar
            Constraint::Length(1), // Divider
            Constraint::Min(4),    // Body
            Constraint::Length(1), // Error banner
            Constraint::Length(1), // Footer hints
        ]
    } else {
        vec![
            Constraint::Length(1), // TabBar
            Constraint::Length(1), // Divider
            Constraint::Min(4),    // Body
            Constraint::Length(1), // Footer hints
        ]
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(inner);

    let tab_area = chunks[0];
    let divider_area = chunks[1];
    let body_area = chunks[2];
    let (error_area, footer_area) = if has_error {
        (Some(chunks[3]), chunks[4])
    } else {
        (None, chunks[3])
    };

    // 1. Render Tab Bar
    let is_tab_focused = state.focus == LauncherFocus::TabBar;
    let is_create_tab = state.tab == LauncherTab::Create;

    let create_style = if is_create_tab {
        if is_tab_focused {
            Style::default()
                .fg(theme.leader_fg)
                .bg(theme.leader_bg)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(theme.tab_active_fg)
                .bg(theme.tab_active_bg)
                .add_modifier(Modifier::BOLD)
        }
    } else {
        Style::default().fg(theme.tab_inactive_fg)
    };

    let attach_style = if !is_create_tab {
        if is_tab_focused {
            Style::default()
                .fg(theme.leader_fg)
                .bg(theme.leader_bg)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(theme.tab_active_fg)
                .bg(theme.tab_active_bg)
                .add_modifier(Modifier::BOLD)
        }
    } else {
        Style::default().fg(theme.tab_inactive_fg)
    };

    let tab_focus_indicator = if is_tab_focused { "▶ " } else { "  " };

    let tab_line = Line::from(vec![
        Span::styled(
            tab_focus_indicator,
            Style::default()
                .fg(theme.leader_bg)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" 1. Create New Project ", create_style),
        Span::raw("   "),
        Span::styled(" 2. Attach Existing Project ", attach_style),
    ]);
    buf.set_line(tab_area.x, tab_area.y, &tab_line, tab_area.width);

    // 2. Render Divider
    let divider_char = "─";
    let divider_text = divider_char.repeat(divider_area.width as usize);
    let divider_line = Line::from(Span::styled(
        divider_text,
        Style::default().fg(theme.border_unfocused),
    ));
    buf.set_line(
        divider_area.x,
        divider_area.y,
        &divider_line,
        divider_area.width,
    );

    // 3. Render Body
    match state.tab {
        LauncherTab::Create => {
            let tpl_focused = state.focus == LauncherFocus::Template;
            let proj_focused = state.focus == LauncherFocus::ProjectName;
            let prompt_focused = state.focus == LauncherFocus::Prompt;

            let tpl_style = if tpl_focused {
                Style::default()
                    .fg(theme.modal_label)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.status_bar_fg)
            };

            let proj_style = if proj_focused {
                Style::default()
                    .fg(theme.modal_label)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.status_bar_fg)
            };

            let prompt_style = if prompt_focused {
                Style::default()
                    .fg(theme.modal_label)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.status_bar_fg)
            };

            let tpl_prefix = if tpl_focused { "▶ " } else { "  " };
            let proj_prefix = if proj_focused { "▶ " } else { "  " };
            let prompt_prefix = if prompt_focused { "▶ " } else { "  " };

            let tpl_display = if state.templates.is_empty() {
                Span::styled(
                    "(No templates found)",
                    Style::default().fg(theme.input_placeholder),
                )
            } else {
                let tpl_name = state.selected_template().unwrap_or("default");
                let count_str = format!(
                    " ◀ {} ▶  ({}/{})",
                    tpl_name,
                    state.creation_modal.selected_template_idx + 1,
                    state.templates.len()
                );
                Span::styled(
                    count_str,
                    Style::default()
                        .fg(theme.input_text)
                        .add_modifier(Modifier::BOLD),
                )
            };

            let proj_display =
                if state.creation_modal.project_name_input.is_empty() && !proj_focused {
                    Span::styled(
                        "<enter project name>",
                        Style::default().fg(theme.input_placeholder),
                    )
                } else {
                    Span::styled(
                        state.creation_modal.project_name_input.value(),
                        Style::default()
                            .fg(theme.input_text)
                            .add_modifier(Modifier::BOLD),
                    )
                };

            let prompt_text_width = (body_area.width.saturating_sub(18) as usize).max(10);
            let prompt_lines = state
                .creation_modal
                .compute_wrapped_prompt_lines(prompt_text_width);
            let visible_count = crate::tui::creation_modal::PROMPT_VISIBLE_LINES;
            let scroll_offset = state
                .creation_modal
                .prompt_scroll_offset
                .min(prompt_lines.len().saturating_sub(1));
            let is_empty_and_unfocused =
                state.creation_modal.prompt_input.is_empty() && !prompt_focused;

            let mut prompt_row_spans = Vec::new();
            for i in 0..visible_count {
                let line_idx = scroll_offset + i;
                if i == 0 {
                    let content_span = if is_empty_and_unfocused {
                        Span::styled(
                            "<optional task instruction prompt>",
                            Style::default().fg(theme.input_placeholder),
                        )
                    } else if line_idx < prompt_lines.len() {
                        Span::styled(
                            &prompt_lines[line_idx].text,
                            Style::default().fg(theme.input_text),
                        )
                    } else {
                        Span::raw("")
                    };
                    prompt_row_spans.push(Line::from(vec![
                        Span::styled(format!("{prompt_prefix}Initial Prompt: "), prompt_style),
                        content_span,
                    ]));
                } else {
                    let content_span = if is_empty_and_unfocused {
                        Span::raw("")
                    } else if line_idx < prompt_lines.len() {
                        Span::styled(
                            &prompt_lines[line_idx].text,
                            Style::default().fg(theme.input_text),
                        )
                    } else {
                        Span::raw("")
                    };
                    prompt_row_spans.push(Line::from(vec![
                        Span::raw("                  "),
                        content_span,
                    ]));
                }
            }

            let mut lines = vec![
                Line::from(vec![
                    Span::styled(format!("{tpl_prefix}Template: "), tpl_style),
                    tpl_display,
                ]),
                Line::from(""),
                Line::from(vec![
                    Span::styled(format!("{proj_prefix}Project Name: "), proj_style),
                    proj_display,
                ]),
                Line::from(""),
            ];
            lines.extend(prompt_row_spans);

            let paragraph = Paragraph::new(lines);
            paragraph.render(body_area, buf);
        }
        LauncherTab::Attach => {
            let is_list_focused = state.focus == LauncherFocus::ProjectList;

            if state.local_projects.is_empty() {
                let empty_lines = vec![
                    Line::from(Span::styled(
                        " (No local project directories with AGENTS.md found in CWD)",
                        Style::default().fg(theme.input_placeholder),
                    )),
                    Line::from(""),
                    Line::from(Span::styled(
                        " Switch to 'Create New Project' tab [Tab / ←] to initialize a new project.",
                        Style::default().fg(theme.input_placeholder),
                    )),
                ];
                let msg = Paragraph::new(empty_lines);
                msg.render(body_area, buf);
            } else {
                let items: Vec<ListItem> = state
                    .local_projects
                    .iter()
                    .enumerate()
                    .map(|(i, proj)| {
                        let is_selected = i == state.selected_project_idx;
                        let (prefix, base_style) = if is_selected && is_list_focused {
                            (
                                " ▶ ",
                                Style::default()
                                    .fg(theme.selection_fg)
                                    .bg(theme.selection_bg)
                                    .add_modifier(Modifier::BOLD),
                            )
                        } else if is_selected {
                            (" ▶ ", Style::default().fg(theme.selection_indicator))
                        } else {
                            ("   ", Style::default().fg(theme.status_bar_fg))
                        };

                        let mut spans = vec![Span::styled(prefix, base_style)];

                        if let Some(status) = state.running_sessions.get(proj) {
                            let dot_color = status.status_color(theme);
                            let mut dot_style = Style::default().fg(dot_color);
                            if is_selected && is_list_focused {
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
                list.render(body_area, buf);
            }
        }
    }

    // 4. Render Error Banner (if present)
    if let Some(err_area) = error_area {
        if let Some(ref err) = state.error_message {
            let err_line = Line::from(Span::styled(
                format!(" ✖ Error: {err}"),
                Style::default()
                    .fg(theme.error_banner_bg)
                    .add_modifier(Modifier::BOLD),
            ));
            buf.set_line(err_area.x, err_area.y, &err_line, err_area.width);
        }
    }

    // 5. Render Footer Hints
    let footer_text = match state.tab {
        LauncherTab::Create => {
            " [Tab/Shift+Tab] Nav | [←/→] Tab/Template | [Enter] Create | [Esc] Cancel "
        }
        LauncherTab::Attach => {
            " [Tab/Shift+Tab] Nav | [↑/↓/j/k] Select | [Enter] Attach | [Esc] Cancel "
        }
    };
    let footer_line = Line::from(Span::styled(
        footer_text,
        Style::default().fg(theme.key_hint),
    ));
    buf.set_line(
        footer_area.x,
        footer_area.y,
        &footer_line,
        footer_area.width,
    );
}

impl Widget for &QuickLauncherState {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let theme = Theme::default();
        render_quick_launcher(self, area, buf, &theme);
    }
}

impl Widget for QuickLauncherState {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let theme = Theme::default();
        render_quick_launcher(&self, area, buf, &theme);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

    fn make_key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            state: KeyEventState::empty(),
        }
    }

    fn key(code: KeyCode) -> KeyEvent {
        make_key(code, KeyModifiers::NONE)
    }

    #[test]
    fn test_quick_launcher_initialization() {
        let templates = vec!["ecommerce".to_string(), "analytics".to_string()];
        let projects = vec!["feature-cart".to_string(), "fix-auth".to_string()];

        let launcher =
            QuickLauncherState::new(templates.clone(), projects.clone(), Some("analytics"));

        assert_eq!(launcher.tab, LauncherTab::Create);
        assert_eq!(launcher.focus, LauncherFocus::Template);
        assert_eq!(launcher.templates, templates);
        assert_eq!(launcher.local_projects, projects);
        assert_eq!(launcher.selected_template(), Some("analytics"));
        assert_eq!(launcher.selected_project(), Some("feature-cart"));
        assert!(launcher.error_message.is_none());
    }

    #[test]
    fn test_quick_launcher_initialization_default_fallback() {
        let templates = vec!["backend".to_string()];
        let launcher = QuickLauncherState::new(templates, vec![], Some("unknown"));
        assert_eq!(launcher.creation_modal.selected_template_idx, 0);
        assert_eq!(launcher.selected_template(), Some("backend"));
    }

    #[test]
    fn test_template_cycling() {
        let templates = vec!["ws1".to_string(), "ws2".to_string(), "ws3".to_string()];
        let mut launcher = QuickLauncherState::new(templates, vec![], None);
        assert_eq!(launcher.creation_modal.selected_template_idx, 0);

        launcher.cycle_template_next();
        assert_eq!(launcher.creation_modal.selected_template_idx, 1);
        launcher.cycle_template_next();
        assert_eq!(launcher.creation_modal.selected_template_idx, 2);
        launcher.cycle_template_next();
        assert_eq!(launcher.creation_modal.selected_template_idx, 0);

        launcher.cycle_template_prev();
        assert_eq!(launcher.creation_modal.selected_template_idx, 2);
        launcher.cycle_template_prev();
        assert_eq!(launcher.creation_modal.selected_template_idx, 1);
    }

    #[test]
    fn test_template_cycling_empty() {
        let mut launcher = QuickLauncherState::new(vec![], vec![], None);
        assert_eq!(launcher.selected_template(), None);
        launcher.cycle_template_next();
        assert_eq!(launcher.creation_modal.selected_template_idx, 0);
        launcher.cycle_template_prev();
        assert_eq!(launcher.creation_modal.selected_template_idx, 0);
    }

    #[test]
    fn test_tab_switching_and_focus_transitions() {
        let mut launcher = QuickLauncherState::new(vec!["ws".into()], vec!["proj".into()], None);
        assert_eq!(launcher.tab, LauncherTab::Create);

        // Switch to TabBar focus
        launcher.focus = LauncherFocus::TabBar;

        // Left/Right switches tab
        let action = launcher.handle_key(key(KeyCode::Right));
        assert_eq!(action, QuickLauncherAction::None);
        assert_eq!(launcher.tab, LauncherTab::Attach);
        assert_eq!(launcher.focus, LauncherFocus::TabBar);

        let action = launcher.handle_key(key(KeyCode::Left));
        assert_eq!(action, QuickLauncherAction::None);
        assert_eq!(launcher.tab, LauncherTab::Create);
        assert_eq!(launcher.focus, LauncherFocus::TabBar);

        // Press '2' on TabBar switches to Attach
        let action = launcher.handle_key(key(KeyCode::Char('2')));
        assert_eq!(action, QuickLauncherAction::None);
        assert_eq!(launcher.tab, LauncherTab::Attach);

        // Press '1' on TabBar switches to Create
        let action = launcher.handle_key(key(KeyCode::Char('1')));
        assert_eq!(action, QuickLauncherAction::None);
        assert_eq!(launcher.tab, LauncherTab::Create);
    }

    #[test]
    fn test_focus_navigation_in_create_tab() {
        let mut launcher = QuickLauncherState::new(vec!["ws".into()], vec![], None);
        launcher.focus = LauncherFocus::TabBar;

        // Tab moves forward: TabBar -> Template -> ProjectName -> Prompt -> TabBar
        launcher.handle_key(key(KeyCode::Tab));
        assert_eq!(launcher.focus, LauncherFocus::Template);

        launcher.handle_key(key(KeyCode::Tab));
        assert_eq!(launcher.focus, LauncherFocus::ProjectName);

        launcher.handle_key(key(KeyCode::Tab));
        assert_eq!(launcher.focus, LauncherFocus::Prompt);

        launcher.handle_key(key(KeyCode::Tab));
        assert_eq!(launcher.focus, LauncherFocus::TabBar);

        // BackTab moves backward: TabBar -> Prompt -> ProjectName -> Template -> TabBar
        launcher.handle_key(make_key(KeyCode::BackTab, KeyModifiers::SHIFT));
        assert_eq!(launcher.focus, LauncherFocus::Prompt);

        launcher.handle_key(make_key(KeyCode::BackTab, KeyModifiers::SHIFT));
        assert_eq!(launcher.focus, LauncherFocus::ProjectName);

        launcher.handle_key(make_key(KeyCode::BackTab, KeyModifiers::SHIFT));
        assert_eq!(launcher.focus, LauncherFocus::Template);

        launcher.handle_key(make_key(KeyCode::BackTab, KeyModifiers::SHIFT));
        assert_eq!(launcher.focus, LauncherFocus::TabBar);

        // Up/Down navigation
        launcher.handle_key(key(KeyCode::Down));
        assert_eq!(launcher.focus, LauncherFocus::Template);

        launcher.handle_key(key(KeyCode::Up));
        assert_eq!(launcher.focus, LauncherFocus::TabBar);
    }

    #[test]
    fn test_text_input_and_backspace_editing() {
        let mut launcher = QuickLauncherState::new(vec!["ws".into()], vec![], None);
        launcher.focus = LauncherFocus::ProjectName;

        // Type project name
        for c in "billing-service".chars() {
            launcher.handle_key(key(KeyCode::Char(c)));
        }
        assert_eq!(
            launcher.creation_modal.project_name_input,
            "billing-service"
        );

        // Backspace
        launcher.handle_key(key(KeyCode::Backspace));
        assert_eq!(launcher.creation_modal.project_name_input, "billing-servic");
        launcher.handle_key(key(KeyCode::Char('e')));
        assert_eq!(
            launcher.creation_modal.project_name_input,
            "billing-service"
        );

        // Switch to Prompt
        launcher.handle_key(key(KeyCode::Tab));
        assert_eq!(launcher.focus, LauncherFocus::Prompt);

        for c in "Implement stripe checkout".chars() {
            launcher.handle_key(key(KeyCode::Char(c)));
        }
        assert_eq!(
            launcher.creation_modal.prompt_input,
            "Implement stripe checkout"
        );

        launcher.handle_key(key(KeyCode::Backspace));
        assert_eq!(
            launcher.creation_modal.prompt_input,
            "Implement stripe checkou"
        );
    }

    #[test]
    fn test_submission_create_project() {
        let mut launcher = QuickLauncherState::new(
            vec!["ecommerce".into(), "analytics".into()],
            vec![],
            Some("analytics"),
        );
        launcher.focus = LauncherFocus::ProjectName;

        // 1. Submit with empty name sets error
        let action = launcher.handle_key(key(KeyCode::Enter));
        assert_eq!(action, QuickLauncherAction::None);
        assert_eq!(
            launcher.error_message,
            Some("Project name cannot be empty".to_string())
        );

        // 2. Typing clears error
        launcher.handle_key(key(KeyCode::Char('m')));
        assert!(launcher.error_message.is_none());

        for c in "y-feature".chars() {
            launcher.handle_key(key(KeyCode::Char(c)));
        }
        launcher.handle_key(key(KeyCode::Tab)); // focus Prompt
        for c in "Initial task prompt".chars() {
            launcher.handle_key(key(KeyCode::Char(c)));
        }

        // 3. Submit with valid name and prompt
        let action = launcher.handle_key(key(KeyCode::Enter));
        assert_eq!(
            action,
            QuickLauncherAction::Create {
                template: "analytics".to_string(),
                project: "my-feature".to_string(),
                agent_override: Some("claude".to_string()),
                prompt: Some("Initial task prompt".to_string()),
            }
        );
    }

    #[test]
    fn test_submission_create_project_empty_prompt_is_none() {
        let mut launcher = QuickLauncherState::new(vec!["ws1".into()], vec![], None);
        launcher.focus = LauncherFocus::ProjectName;
        for c in "proj-without-prompt".chars() {
            launcher.handle_key(key(KeyCode::Char(c)));
        }

        let action = launcher.handle_key(key(KeyCode::Enter));
        assert_eq!(
            action,
            QuickLauncherAction::Create {
                template: "ws1".to_string(),
                project: "proj-without-prompt".to_string(),
                agent_override: Some("claude".to_string()),
                prompt: None,
            }
        );
    }

    #[test]
    fn test_submission_create_project_with_custom_template_agent() {
        let config_toml = r#"
default_agent = "claude"
repos_dir = ".botai_repos"

[agents.claude]
command = "claude"
args = []

[agents.agy]
command = "agy"
args = []

[templates.agy_template]
description = "Template using agy"
default_agent = "agy"
repos = []
"#;
        let config = Config::load_from_str(config_toml).unwrap();
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let mut launcher = QuickLauncherState::from_config(&config, &cwd, vec![], Some("agy_template"));
        launcher.focus = LauncherFocus::ProjectName;
        for c in "new-agy-proj".chars() {
            launcher.handle_key(key(KeyCode::Char(c)));
        }

        let action = launcher.handle_key(key(KeyCode::Enter));
        assert_eq!(
            action,
            QuickLauncherAction::Create {
                template: "agy_template".to_string(),
                project: "new-agy-proj".to_string(),
                agent_override: Some("agy".to_string()),
                prompt: None,
            }
        );
    }

    #[test]
    fn test_submission_create_project_empty_templates() {
        let mut launcher = QuickLauncherState::new(vec![], vec![], None);
        launcher.focus = LauncherFocus::ProjectName;
        for c in "my-proj".chars() {
            launcher.handle_key(key(KeyCode::Char(c)));
        }

        let action = launcher.handle_key(key(KeyCode::Enter));
        assert_eq!(action, QuickLauncherAction::None);
        assert_eq!(
            launcher.error_message,
            Some("No template available".to_string())
        );
    }

    #[test]
    fn test_attach_tab_navigation_and_submission() {
        let projects = vec![
            "proj-alpha".to_string(),
            "proj-beta".to_string(),
            "proj-gamma".to_string(),
        ];
        let mut launcher = QuickLauncherState::new(vec!["ws".into()], projects, None);
        launcher.switch_tab(LauncherTab::Attach);
        assert_eq!(launcher.focus, LauncherFocus::ProjectList);
        assert_eq!(launcher.selected_project_idx, 0);

        // Down / j moves down
        launcher.handle_key(key(KeyCode::Down));
        assert_eq!(launcher.selected_project_idx, 1);

        launcher.handle_key(key(KeyCode::Char('j')));
        assert_eq!(launcher.selected_project_idx, 2);

        // Boundary down
        launcher.handle_key(key(KeyCode::Down));
        assert_eq!(launcher.selected_project_idx, 2);

        // Up / k moves up
        launcher.handle_key(key(KeyCode::Char('k')));
        assert_eq!(launcher.selected_project_idx, 1);

        launcher.handle_key(key(KeyCode::Up));
        assert_eq!(launcher.selected_project_idx, 0);

        // Submit attach
        let action = launcher.handle_key(key(KeyCode::Enter));
        assert_eq!(
            action,
            QuickLauncherAction::Attach("proj-alpha".to_string())
        );
    }

    #[test]
    fn test_attach_tab_empty_projects_submission() {
        let mut launcher = QuickLauncherState::new(vec!["ws".into()], vec![], None);
        launcher.switch_tab(LauncherTab::Attach);

        let action = launcher.handle_key(key(KeyCode::Enter));
        assert_eq!(action, QuickLauncherAction::None);
        assert_eq!(
            launcher.error_message,
            Some("No local project available to attach".to_string())
        );
    }

    #[test]
    fn test_esc_and_ctrl_c_cancels() {
        let mut launcher = QuickLauncherState::new(vec!["ws".into()], vec![], None);

        let action = launcher.handle_key(key(KeyCode::Esc));
        assert_eq!(action, QuickLauncherAction::Cancel);

        let action = launcher.handle_key(make_key(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert_eq!(action, QuickLauncherAction::Cancel);

        let action = launcher.handle_key(make_key(KeyCode::Char('C'), KeyModifiers::CONTROL));
        assert_eq!(action, QuickLauncherAction::Cancel);
    }

    #[test]
    fn test_render_quick_launcher_create_tab() {
        let launcher = QuickLauncherState::new(
            vec!["ecommerce".into(), "backend".into()],
            vec!["local-app".into()],
            Some("ecommerce"),
        );

        let mut buffer = Buffer::empty(Rect::new(0, 0, 80, 20));
        let theme = Theme::default();
        render_quick_launcher(&launcher, Rect::new(0, 0, 80, 20), &mut buffer, &theme);

        let rendered = buffer
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect::<String>();

        assert!(rendered.contains("Quick Project Launcher (Ctrl+Space c)"));
        assert!(rendered.contains("1. Create New Project"));
        assert!(rendered.contains("2. Attach Existing Project"));
        assert!(rendered.contains("Template:"));
        assert!(rendered.contains("ecommerce"));
        assert!(rendered.contains("Project Name:"));
        assert!(rendered.contains("Initial Prompt:"));
    }

    #[test]
    fn test_render_quick_launcher_with_error_banner() {
        let mut launcher = QuickLauncherState::new(vec!["ws1".into()], vec![], None);
        launcher.set_error("Branch collision: 'test' already exists");

        let mut buffer = Buffer::empty(Rect::new(0, 0, 80, 20));
        let theme = Theme::default();
        render_quick_launcher(&launcher, Rect::new(0, 0, 80, 20), &mut buffer, &theme);

        let rendered = buffer
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect::<String>();

        assert!(rendered.contains("✖ Error: Branch collision: 'test' already exists"));
    }

    #[test]
    fn test_render_quick_launcher_attach_tab() {
        let mut launcher = QuickLauncherState::new(
            vec!["ws1".into()],
            vec!["my-backend".into(), "my-frontend".into()],
            None,
        );
        launcher.switch_tab(LauncherTab::Attach);

        let mut buffer = Buffer::empty(Rect::new(0, 0, 80, 20));
        let theme = Theme::default();
        render_quick_launcher(&launcher, Rect::new(0, 0, 80, 20), &mut buffer, &theme);

        let rendered = buffer
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect::<String>();

        assert!(rendered.contains("my-backend"));
        assert!(rendered.contains("my-frontend"));
        assert!(rendered.contains("Attach"));
    }

    #[test]
    fn test_quick_launcher_themed_rendering() {
        let launcher = QuickLauncherState::new(
            vec!["ws-main".into()],
            vec!["proj-local".into()],
            Some("ws-main"),
        );

        let theme = Theme::cyberpunk();
        let mut buffer = Buffer::empty(Rect::new(0, 0, 80, 20));
        render_quick_launcher(&launcher, Rect::new(0, 0, 80, 20), &mut buffer, &theme);

        // Check container border top-left cell
        let border_cell = buffer.cell((4, 2)).unwrap();
        assert_eq!(border_cell.fg, theme.border_focused);
    }

    #[test]
    fn test_quick_launcher_cursor_position() {
        let area = Rect::new(0, 0, 100, 30);
        let mut launcher = QuickLauncherState::new(vec!["ws1".into()], vec!["proj1".into()], None);

        // 1. Initial focus is Template -> None
        assert_eq!(launcher.focus, LauncherFocus::Template);
        assert_eq!(launcher.cursor_position(area), None);

        // 2. Focus on ProjectName, empty input
        launcher.focus = LauncherFocus::ProjectName;
        assert_eq!(launcher.cursor_position(area), Some(Position::new(31, 12)));

        // Focus on ProjectName, with input "demo-api"
        launcher.creation_modal.set_project_name("demo-api");
        assert_eq!(launcher.cursor_position(area), Some(Position::new(39, 12)));

        // 3. Focus on Prompt, empty input
        launcher.focus = LauncherFocus::Prompt;
        assert_eq!(launcher.cursor_position(area), Some(Position::new(33, 14)));

        // Focus on Prompt, with input "hello"
        launcher.creation_modal.prompt_input = "hello".to_string();
        launcher.creation_modal.prompt_cursor = 5;
        assert_eq!(launcher.cursor_position(area), Some(Position::new(38, 14)));

        launcher.creation_modal.prompt_cursor = 0;
        assert_eq!(launcher.cursor_position(area), Some(Position::new(33, 14)));

        // 4. Focus on TabBar -> None
        launcher.focus = LauncherFocus::TabBar;
        assert_eq!(launcher.cursor_position(area), None);

        // 5. Attach tab -> None
        launcher.switch_tab(LauncherTab::Attach);
        assert_eq!(launcher.cursor_position(area), None);
        launcher.focus = LauncherFocus::ProjectList;
        assert_eq!(launcher.cursor_position(area), None);

        // 6. Too small area -> None
        let small_area = Rect::new(0, 0, 20, 5);
        assert_eq!(launcher.cursor_position(small_area), None);
    }

    #[test]
    fn test_quick_launcher_running_sessions_methods() {
        let mut running = HashMap::new();
        running.insert("proj-a".to_string(), SessionStatus::Busy);
        running.insert("proj-b".to_string(), SessionStatus::Done);

        let launcher =
            QuickLauncherState::new(vec!["ws".to_string()], vec!["proj-a".to_string()], None)
                .with_running_sessions(running.clone());

        assert_eq!(launcher.running_sessions.len(), 2);
        assert_eq!(
            launcher.running_sessions.get("proj-a"),
            Some(&SessionStatus::Busy)
        );

        let mut launcher2 = QuickLauncherState::new(vec![], vec![], None);
        launcher2.set_running_sessions(running);
        assert_eq!(launcher2.running_sessions.len(), 2);
    }

    #[test]
    fn test_render_quick_launcher_attach_tab_running_session_indicators_and_colors() {
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

        let mut launcher = QuickLauncherState::new(vec!["ws".to_string()], projects, None)
            .with_running_sessions(running);
        launcher.switch_tab(LauncherTab::Attach);

        let area = Rect::new(0, 0, 80, 20);
        let mut buffer = Buffer::empty(area);
        let theme = Theme::tokyo_night();

        render_quick_launcher(&launcher, area, &mut buffer, &theme);

        let rendered = buffer
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect::<String>();

        assert!(rendered.contains("●"));
        assert!(rendered.contains("proj-busy"));
        assert!(rendered.contains("proj-wait"));
        assert!(rendered.contains("proj-done"));
        assert!(rendered.contains("proj-err"));
        assert!(rendered.contains("proj-inactive"));

        // In 80x20 area: width = 72, height = 16.
        // x = (80 - 72)/2 = 4, y = (20 - 16)/2 = 2.
        // Outer block inner: x = 5, y = 3, width = 70, height = 14.
        // Chunks: TabBar (y=3, h=1), Divider (y=4, h=1), Body (y=5, h=10), Footer (y=15, h=1).
        // Body starts at y = 5, x = 5.
        // Row 1 (y = 5): proj-busy -> prefix is " ▶ " (len 3: x=5,6,7), dot is at x = 8 (symbol "●"), space at x = 9, project name starts at x = 10.
        let cell_busy_dot = buffer.cell((8, 5)).unwrap();
        assert_eq!(cell_busy_dot.symbol(), "●");
        assert_eq!(cell_busy_dot.fg, theme.status_busy);

        // Row 2 (y = 6): proj-wait -> dot at x = 8
        let cell_wait_dot = buffer.cell((8, 6)).unwrap();
        assert_eq!(cell_wait_dot.symbol(), "●");
        assert_eq!(cell_wait_dot.fg, theme.status_waiting);

        // Row 3 (y = 7): proj-done -> dot at x = 8
        let cell_done_dot = buffer.cell((8, 7)).unwrap();
        assert_eq!(cell_done_dot.symbol(), "●");
        assert_eq!(cell_done_dot.fg, theme.status_done);

        // Row 4 (y = 8): proj-err -> dot at x = 8
        let cell_err_dot = buffer.cell((8, 8)).unwrap();
        assert_eq!(cell_err_dot.symbol(), "●");
        assert_eq!(cell_err_dot.fg, theme.status_error);

        // Row 5 (y = 9): proj-inactive -> spaces at x = 8 and x = 9, project name 'p' at x = 10
        let cell_inactive_space1 = buffer.cell((8, 9)).unwrap();
        let cell_inactive_space2 = buffer.cell((9, 9)).unwrap();
        let cell_inactive_p = buffer.cell((10, 9)).unwrap();
        assert_eq!(cell_inactive_space1.symbol(), " ");
        assert_eq!(cell_inactive_space2.symbol(), " ");
        assert_eq!(cell_inactive_p.symbol(), "p");
    }

    #[test]
    fn test_quick_launcher_mouse_scrolling() {
        use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

        let mut state = QuickLauncherState::new(
            vec!["tpl1".to_string(), "tpl2".to_string(), "tpl3".to_string()],
            vec![
                "proj1".to_string(),
                "proj2".to_string(),
                "proj3".to_string(),
            ],
            None,
        );

        assert_eq!(state.tab, LauncherTab::Create);
        assert_eq!(state.focus, LauncherFocus::Template);
        assert_eq!(state.creation_modal.selected_template_idx, 0);

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

        // ScrollDown cycles template forward
        assert_eq!(state.handle_mouse(scroll_down), QuickLauncherAction::None);
        assert_eq!(state.creation_modal.selected_template_idx, 1);

        // ScrollUp cycles template backward
        assert_eq!(state.handle_mouse(scroll_up), QuickLauncherAction::None);
        assert_eq!(state.creation_modal.selected_template_idx, 0);

        // Switch to Attach tab
        state.switch_tab(LauncherTab::Attach);
        assert_eq!(state.tab, LauncherTab::Attach);
        assert_eq!(state.focus, LauncherFocus::ProjectList);
        assert_eq!(state.selected_project_idx, 0);

        // ScrollDown navigates projects down
        assert_eq!(state.handle_mouse(scroll_down), QuickLauncherAction::None);
        assert_eq!(state.selected_project_idx, 1);

        // ScrollUp navigates projects up
        assert_eq!(state.handle_mouse(scroll_up), QuickLauncherAction::None);
        assert_eq!(state.selected_project_idx, 0);

        // Non-scroll event
        let click = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 10,
            row: 10,
            modifiers: KeyModifiers::empty(),
        };
        assert_eq!(state.handle_mouse(click), QuickLauncherAction::None);
    }

    #[test]
    fn test_quick_launcher_prompt_cursor_navigation_and_editing() {
        let mut state =
            QuickLauncherState::new(vec!["tpl1".to_string()], vec!["proj1".to_string()], None);

        state.focus = LauncherFocus::Prompt;
        state.creation_modal.focus = crate::tui::creation_modal::CreationFocus::Prompt;

        // 1. Type characters
        state.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        state.handle_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE));
        state.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));
        assert_eq!(state.creation_modal.prompt_input, "abc");
        assert_eq!(state.creation_modal.prompt_cursor, 3);

        // 2. Cursor Left, Right, Home, End
        state.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        assert_eq!(state.creation_modal.prompt_cursor, 2);
        state.handle_key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE));
        assert_eq!(state.creation_modal.prompt_cursor, 0);
        state.handle_key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE));
        assert_eq!(state.creation_modal.prompt_cursor, 3);
        state.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(state.creation_modal.prompt_cursor, 3);

        // 3. Shift+Enter / Alt+Enter inserts newline
        state.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT));
        assert_eq!(state.creation_modal.prompt_input, "abc\n");
        assert_eq!(state.creation_modal.prompt_cursor, 4);

        state.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));
        state.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT));
        assert_eq!(state.creation_modal.prompt_input, "abc\nd\n");

        // 4. Vertical Up/Down navigation within prompt
        assert_eq!(state.creation_modal.cursor_to_line_col(56), (2, 0));
        state.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        assert_eq!(state.creation_modal.cursor_to_line_col(56), (1, 0));
        state.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        assert_eq!(state.creation_modal.cursor_to_line_col(56), (0, 0));

        // Up on top line transitions focus to ProjectName (prev_focus)
        state.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        assert_eq!(state.focus, LauncherFocus::ProjectName);

        // Down from ProjectName goes to Prompt
        state.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(state.focus, LauncherFocus::Prompt);

        // 5. Emacs shortcuts: Ctrl+A, Ctrl+E, Ctrl+W, Ctrl+K, Ctrl+U
        state.handle_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::CONTROL));
        assert_eq!(state.creation_modal.prompt_cursor, 3); // end of first line
        state.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL));
        assert_eq!(state.creation_modal.prompt_cursor, 0); // start of first line

        // 6. Delete & Backspace
        state.handle_key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));
        assert_eq!(state.creation_modal.prompt_input, "bc\nd\n");
        state.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        state.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        assert_eq!(state.creation_modal.prompt_input, "c\nd\n");
    }

    #[test]
    fn test_quick_launcher_prompt_paste_and_multiline_render() {
        let mut state =
            QuickLauncherState::new(vec!["tpl1".to_string()], vec!["proj1".to_string()], None);

        state.focus = LauncherFocus::Prompt;
        state.creation_modal.focus = crate::tui::creation_modal::CreationFocus::Prompt;

        // Paste multiline text
        state.handle_paste("First line of instructions\r\nSecond line of instructions");
        assert_eq!(
            state.creation_modal.prompt_input,
            "First line of instructions\nSecond line of instructions"
        );

        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        let theme = Theme::default();

        render_quick_launcher(&state, area, &mut buf, &theme);
        let content = buf
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect::<String>();
        assert!(content.contains("First line of instructions"));
        assert!(content.contains("Second line of instructions"));
    }
}
