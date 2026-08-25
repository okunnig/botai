use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Position, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Widget},
};

use crate::config::Config;
use crate::project::{ProjectBuilder, ProjectValidation};
use crate::tui::theme::Theme;

/// Active step within the two-step Project Creation Modal flow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreationStep {
    SelectTemplate,
    ConfigureDetails,
}

/// Active focused field within the unified Project Creation Modal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreationFocus {
    Template,
    ProjectName,
    Agent,
    Prompt,
}

/// Actions emitted by user interactions with the Project Creation Modal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CreationModalAction {
    None,
    Submit {
        template: String,
        project: String,
        agent_override: Option<String>,
        prompt: Option<String>,
    },
    Cancel,
}

/// Unified interactive project creation modal with two-step searchable template workflow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectCreationModal {
    pub config: Config,
    pub cwd: PathBuf,
    pub step: CreationStep,
    pub template_search_query: String,
    pub templates: Vec<String>,
    pub selected_template_idx: usize,
    pub project_name_input: String,
    pub agent_list: Vec<String>,
    pub selected_agent_idx: usize,
    pub prompt_input: String,
    pub focus: CreationFocus,
    pub validation: ProjectValidation,
}

impl ProjectCreationModal {
    /// Creates a new `ProjectCreationModal` initialized with configuration and working directory.
    pub fn new(config: &Config, cwd: &Path, default_template: Option<&str>) -> Self {
        let mut templates: Vec<String> = config.templates.keys().cloned().collect();
        templates.sort();

        // Smart single-template bypass: If config.templates.len() <= 1, initialize in CreationStep::ConfigureDetails.
        // If more than 1 template exists, initialize in CreationStep::SelectTemplate.
        let step = if templates.len() <= 1 {
            CreationStep::ConfigureDetails
        } else {
            CreationStep::SelectTemplate
        };

        let global_selected_idx = default_template
            .and_then(|t| templates.iter().position(|w| w == t))
            .unwrap_or(0);

        // Build list of agents from configuration and standard defaults
        let mut agent_set = BTreeSet::new();
        for standard in &["claude", "agy", "copilot", "aider"] {
            agent_set.insert(standard.to_string());
        }
        for agent_name in config.agents.keys() {
            agent_set.insert(agent_name.clone());
        }
        agent_set.insert(config.default_agent.clone());

        let agent_list: Vec<String> = agent_set.into_iter().collect();

        let target_agent = templates
            .get(global_selected_idx)
            .and_then(|tpl_name| config.get_template(tpl_name))
            .and_then(|tpl| tpl.default_agent.as_ref())
            .unwrap_or(&config.default_agent);

        let selected_agent_idx = agent_list
            .iter()
            .position(|a| a == target_agent)
            .unwrap_or(0);

        let selected_template_idx = global_selected_idx;

        let mut modal = Self {
            config: config.clone(),
            cwd: cwd.to_path_buf(),
            step,
            template_search_query: String::new(),
            templates,
            selected_template_idx,
            project_name_input: String::new(),
            agent_list,
            selected_agent_idx,
            prompt_input: String::new(),
            focus: CreationFocus::ProjectName,
            validation: ProjectValidation::valid(),
        };

        modal.revalidate();
        modal
    }

    /// Sets the active step.
    pub fn with_step(mut self, step: CreationStep) -> Self {
        self.step = step;
        self
    }

    /// Sets the active field focus.
    pub fn with_focus(mut self, focus: CreationFocus) -> Self {
        self.focus = focus;
        self
    }

    /// Sets the template search filter query.
    pub fn with_query(mut self, query: impl Into<String>) -> Self {
        self.template_search_query = query.into();
        self
    }

    /// Returns the indices into `self.templates` matching `self.template_search_query`.
    /// Filter logic: case-insensitive match against template name, description, and repo URLs/names.
    pub fn matching_template_indices(&self) -> Vec<usize> {
        let query = self.template_search_query.trim().to_lowercase();
        self.templates
            .iter()
            .enumerate()
            .filter(|(_idx, tpl_name)| {
                if query.is_empty() {
                    return true;
                }
                if tpl_name.to_lowercase().contains(&query) {
                    return true;
                }
                if let Some(tpl) = self.config.get_template(tpl_name) {
                    if let Some(ref desc) = tpl.description {
                        if desc.to_lowercase().contains(&query) {
                            return true;
                        }
                    }
                    for repo in &tpl.repos {
                        if repo.to_lowercase().contains(&query) {
                            return true;
                        }
                        let derived = crate::config::derive_repo_name(repo);
                        if derived.to_lowercase().contains(&query) {
                            return true;
                        }
                    }
                }
                false
            })
            .map(|(idx, _)| idx)
            .collect()
    }

    /// Returns matching template names according to the current filter query.
    pub fn matching_templates(&self) -> Vec<String> {
        self.matching_template_indices()
            .into_iter()
            .map(|i| self.templates[i].clone())
            .collect()
    }

    /// Returns the currently selected template name, if available.
    pub fn selected_template(&self) -> Option<&str> {
        match self.step {
            CreationStep::SelectTemplate => {
                let matching = self.matching_template_indices();
                if !matching.is_empty() {
                    let clamped = self.selected_template_idx.min(matching.len() - 1);
                    let target_idx = matching[clamped];
                    Some(&self.templates[target_idx])
                } else {
                    None
                }
            }
            CreationStep::ConfigureDetails => {
                if !self.templates.is_empty() && self.selected_template_idx < self.templates.len() {
                    Some(&self.templates[self.selected_template_idx])
                } else {
                    None
                }
            }
        }
    }

    /// Returns the currently selected agent CLI tool name.
    pub fn selected_agent(&self) -> &str {
        if !self.agent_list.is_empty() && self.selected_agent_idx < self.agent_list.len() {
            &self.agent_list[self.selected_agent_idx]
        } else {
            "claude"
        }
    }

    /// Cycles to the next available template and updates default agent override if appropriate.
    pub fn cycle_template_next(&mut self) {
        match self.step {
            CreationStep::SelectTemplate => {
                let matching = self.matching_template_indices();
                if !matching.is_empty() {
                    self.selected_template_idx = (self.selected_template_idx + 1) % matching.len();
                    self.sync_agent_with_template();
                    self.revalidate();
                }
            }
            CreationStep::ConfigureDetails => {
                if !self.templates.is_empty() {
                    self.selected_template_idx =
                        (self.selected_template_idx + 1) % self.templates.len();
                    self.sync_agent_with_template();
                    self.revalidate();
                }
            }
        }
    }

    /// Cycles to the previous available template and updates default agent override if appropriate.
    pub fn cycle_template_prev(&mut self) {
        match self.step {
            CreationStep::SelectTemplate => {
                let matching = self.matching_template_indices();
                if !matching.is_empty() {
                    self.selected_template_idx =
                        (self.selected_template_idx + matching.len() - 1) % matching.len();
                    self.sync_agent_with_template();
                    self.revalidate();
                }
            }
            CreationStep::ConfigureDetails => {
                if !self.templates.is_empty() {
                    self.selected_template_idx =
                        (self.selected_template_idx + self.templates.len() - 1) % self.templates.len();
                    self.sync_agent_with_template();
                    self.revalidate();
                }
            }
        }
    }

    /// Cycles to the next available agent tool.
    pub fn cycle_agent_next(&mut self) {
        if !self.agent_list.is_empty() {
            self.selected_agent_idx = (self.selected_agent_idx + 1) % self.agent_list.len();
            self.revalidate();
        }
    }

    /// Cycles to the previous available agent tool.
    pub fn cycle_agent_prev(&mut self) {
        if !self.agent_list.is_empty() {
            self.selected_agent_idx =
                (self.selected_agent_idx + self.agent_list.len() - 1) % self.agent_list.len();
            self.revalidate();
        }
    }

    /// Synchronizes the selected agent with the newly selected template's default agent.
    fn sync_agent_with_template(&mut self) {
        if let Some(tpl_name) = self.selected_template() {
            if let Some(tpl) = self.config.get_template(tpl_name) {
                let target = tpl.default_agent.as_deref().unwrap_or(&self.config.default_agent);
                if let Some(pos) = self.agent_list.iter().position(|a| a == target) {
                    self.selected_agent_idx = pos;
                }
            }
        }
    }

    /// Moves focus to the next input field.
    pub fn next_focus(&mut self) {
        self.focus = match self.focus {
            CreationFocus::ProjectName => CreationFocus::Agent,
            CreationFocus::Agent => CreationFocus::Prompt,
            CreationFocus::Prompt => CreationFocus::ProjectName,
            CreationFocus::Template => CreationFocus::ProjectName,
        };
    }

    /// Moves focus to the previous input field.
    pub fn prev_focus(&mut self) {
        self.focus = match self.focus {
            CreationFocus::ProjectName => CreationFocus::Prompt,
            CreationFocus::Agent => CreationFocus::ProjectName,
            CreationFocus::Prompt => CreationFocus::Agent,
            CreationFocus::Template => CreationFocus::Prompt,
        };
    }

    /// Re-evaluates live pre-validation checks against template repos and CWD directory.
    pub fn revalidate(&mut self) {
        let tpl_name = self.selected_template().unwrap_or("");
        let builder = ProjectBuilder::new(self.config.clone(), tpl_name, &self.project_name_input)
            .with_cwd(&self.cwd)
            .with_agent(self.selected_agent());

        self.validation = builder.validate_detailed();
    }

    /// Calculates the hardware cursor position within the modal area when an editable text input is focused.
    pub fn cursor_position(&self, area: Rect) -> Option<Position> {
        let width = 76.min(area.width.saturating_sub(4));
        let height = 18.min(area.height.saturating_sub(2));

        if width < 25 || height < 10 {
            return None;
        }

        let x = area.x + (area.width.saturating_sub(width)) / 2;
        let y = area.y + (area.height.saturating_sub(height)) / 2;
        let modal_area = Rect::new(x, y, width, height);

        let block = Block::default().borders(Borders::ALL);
        let inner = block.inner(modal_area);

        match self.step {
            CreationStep::SelectTemplate => {
                let prefix_len = 8u16; // "Filter: "
                let cursor_x = inner.x + prefix_len + self.template_search_query.chars().count() as u16;
                let cursor_y = inner.y;
                if cursor_x < inner.x + inner.width && cursor_y < inner.y + inner.height {
                    Some(Position::new(cursor_x, cursor_y))
                } else {
                    Some(Position::new(
                        cursor_x.min(inner.x + inner.width.saturating_sub(1)),
                        cursor_y,
                    ))
                }
            }
            CreationStep::ConfigureDetails => match self.focus {
                CreationFocus::ProjectName => {
                    let prefix_len = 16u16; // "▶ Project Name: "
                    let cursor_x = inner.x + prefix_len + self.project_name_input.chars().count() as u16;
                    let cursor_y = inner.y + 3;
                    if cursor_x < inner.x + inner.width && cursor_y < inner.y + inner.height {
                        Some(Position::new(cursor_x, cursor_y))
                    } else {
                        Some(Position::new(
                            cursor_x.min(inner.x + inner.width.saturating_sub(1)),
                            cursor_y,
                        ))
                    }
                }
                CreationFocus::Prompt => {
                    let prefix_len = 18u16; // "▶ Initial Prompt: "
                    let cursor_x = inner.x + prefix_len + self.prompt_input.chars().count() as u16;
                    let cursor_y = inner.y + 5;
                    if cursor_x < inner.x + inner.width && cursor_y < inner.y + inner.height {
                        Some(Position::new(cursor_x, cursor_y))
                    } else {
                        Some(Position::new(
                            cursor_x.min(inner.x + inner.width.saturating_sub(1)),
                            cursor_y,
                        ))
                    }
                }
                _ => None,
            },
        }
    }

    /// Handles keyboard events and returns any triggered `CreationModalAction`.
    pub fn handle_key(&mut self, key: KeyEvent) -> CreationModalAction {
        let has_alt = key.modifiers.contains(KeyModifiers::ALT);
        let has_ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

        // Ctrl+C always cancels
        if has_ctrl && (key.code == KeyCode::Char('c') || key.code == KeyCode::Char('C')) {
            return CreationModalAction::Cancel;
        }

        match self.step {
            CreationStep::SelectTemplate => match key.code {
                KeyCode::Esc => CreationModalAction::Cancel,
                KeyCode::Up => {
                    self.cycle_template_prev();
                    CreationModalAction::None
                }
                KeyCode::Down => {
                    self.cycle_template_next();
                    CreationModalAction::None
                }
                KeyCode::Left => {
                    self.cycle_template_prev();
                    CreationModalAction::None
                }
                KeyCode::Right => {
                    self.cycle_template_next();
                    CreationModalAction::None
                }
                KeyCode::Char('k') if has_ctrl => {
                    self.cycle_template_prev();
                    CreationModalAction::None
                }
                KeyCode::Char('j') if has_ctrl => {
                    self.cycle_template_next();
                    CreationModalAction::None
                }
                KeyCode::Char(c) if !has_ctrl && !has_alt => {
                    self.template_search_query.push(c);
                    let matching = self.matching_template_indices();
                    if matching.is_empty() {
                        self.selected_template_idx = 0;
                    } else if self.selected_template_idx >= matching.len() {
                        self.selected_template_idx = matching.len() - 1;
                    }
                    self.sync_agent_with_template();
                    self.revalidate();
                    CreationModalAction::None
                }
                KeyCode::Backspace => {
                    self.template_search_query.pop();
                    let matching = self.matching_template_indices();
                    if matching.is_empty() {
                        self.selected_template_idx = 0;
                    } else if self.selected_template_idx >= matching.len() {
                        self.selected_template_idx = matching.len() - 1;
                    }
                    self.sync_agent_with_template();
                    self.revalidate();
                    CreationModalAction::None
                }
                KeyCode::Enter => {
                    let matching = self.matching_template_indices();
                    if !matching.is_empty() {
                        let target_idx = matching[self.selected_template_idx.min(matching.len() - 1)];
                        self.selected_template_idx = target_idx;
                        self.sync_agent_with_template();
                        self.revalidate();
                        self.step = CreationStep::ConfigureDetails;
                        self.focus = CreationFocus::ProjectName;
                    }
                    CreationModalAction::None
                }
                _ => CreationModalAction::None,
            },
            CreationStep::ConfigureDetails => match key.code {
                KeyCode::Esc => {
                    if self.templates.len() > 1 {
                        let cur_tpl = self.selected_template().map(str::to_string);
                        self.step = CreationStep::SelectTemplate;
                        let matching = self.matching_template_indices();
                        if let Some(tpl) = cur_tpl {
                            self.selected_template_idx = matching
                                .iter()
                                .position(|&i| self.templates.get(i).map(String::as_str) == Some(&tpl))
                                .unwrap_or(0);
                        } else {
                            self.selected_template_idx = 0;
                        }
                        CreationModalAction::None
                    } else {
                        CreationModalAction::Cancel
                    }
                }
                KeyCode::Tab => {
                    self.next_focus();
                    CreationModalAction::None
                }
                KeyCode::BackTab => {
                    self.prev_focus();
                    CreationModalAction::None
                }
                KeyCode::Up => {
                    self.prev_focus();
                    CreationModalAction::None
                }
                KeyCode::Down => {
                    self.next_focus();
                    CreationModalAction::None
                }
                KeyCode::Left => {
                    if self.focus == CreationFocus::Agent {
                        self.cycle_agent_prev();
                    }
                    CreationModalAction::None
                }
                KeyCode::Right => {
                    if self.focus == CreationFocus::Agent {
                        self.cycle_agent_next();
                    }
                    CreationModalAction::None
                }
                KeyCode::Char(c) if !has_ctrl && !has_alt => {
                    match self.focus {
                        CreationFocus::ProjectName => {
                            self.project_name_input.push(c);
                            self.revalidate();
                        }
                        CreationFocus::Prompt => {
                            self.prompt_input.push(c);
                        }
                        CreationFocus::Agent => {
                            if c == 'h' || c == 'k' {
                                self.cycle_agent_prev();
                            } else if c == 'l' || c == 'j' {
                                self.cycle_agent_next();
                            }
                        }
                        CreationFocus::Template => {}
                    }
                    CreationModalAction::None
                }
                KeyCode::Backspace => {
                    match self.focus {
                        CreationFocus::ProjectName => {
                            if self.project_name_input.is_empty() {
                                if self.templates.len() > 1 {
                                    let cur_tpl = self.selected_template().map(str::to_string);
                                    self.step = CreationStep::SelectTemplate;
                                    let matching = self.matching_template_indices();
                                    if let Some(tpl) = cur_tpl {
                                        self.selected_template_idx = matching
                                            .iter()
                                            .position(|&i| {
                                                self.templates.get(i).map(String::as_str)
                                                    == Some(&tpl)
                                            })
                                            .unwrap_or(0);
                                    } else {
                                        self.selected_template_idx = 0;
                                    }
                                }
                            } else {
                                self.project_name_input.pop();
                                self.revalidate();
                            }
                        }
                        CreationFocus::Prompt => {
                            self.prompt_input.pop();
                        }
                        _ => {}
                    }
                    CreationModalAction::None
                }
                KeyCode::Enter => {
                    self.revalidate();
                    if self.validation.is_valid {
                        let trimmed = self.project_name_input.trim();
                        if let Some(tpl) = self.selected_template() {
                            let prompt = if self.prompt_input.trim().is_empty() {
                                None
                            } else {
                                Some(self.prompt_input.trim().to_string())
                            };
                            CreationModalAction::Submit {
                                template: tpl.to_string(),
                                project: trimmed.to_string(),
                                agent_override: Some(self.selected_agent().to_string()),
                                prompt,
                            }
                        } else {
                            CreationModalAction::None
                        }
                    } else {
                        CreationModalAction::None
                    }
                }
                _ => CreationModalAction::None,
            },
        }
    }

    /// Handles pasted text into the active focused text input field or search filter.
    pub fn handle_paste(&mut self, text: &str) {
        match self.step {
            CreationStep::SelectTemplate => {
                for c in text.chars() {
                    if !c.is_control() {
                        self.template_search_query.push(c);
                    }
                }
                let matching = self.matching_template_indices();
                if matching.is_empty() {
                    self.selected_template_idx = 0;
                } else if self.selected_template_idx >= matching.len() {
                    self.selected_template_idx = matching.len() - 1;
                }
                self.sync_agent_with_template();
                self.revalidate();
            }
            CreationStep::ConfigureDetails => match self.focus {
                CreationFocus::ProjectName => {
                    for c in text.chars() {
                        if !c.is_control() {
                            self.project_name_input.push(c);
                        }
                    }
                    self.revalidate();
                }
                CreationFocus::Prompt => {
                    for c in text.chars() {
                        if c == '\n' || c == '\r' || !c.is_control() {
                            self.prompt_input.push(c);
                        }
                    }
                }
                CreationFocus::Agent | CreationFocus::Template => {}
            },
        }
    }

    /// Handles mouse events (e.g. scroll wheel) and returns any triggered `CreationModalAction`.
    pub fn handle_mouse(&mut self, mouse: MouseEvent) -> CreationModalAction {
        match mouse.kind {
            MouseEventKind::ScrollUp => {
                match self.step {
                    CreationStep::SelectTemplate => {
                        self.cycle_template_prev();
                    }
                    CreationStep::ConfigureDetails => {
                        if self.focus == CreationFocus::Agent {
                            self.cycle_agent_prev();
                        } else {
                            self.prev_focus();
                        }
                    }
                }
                CreationModalAction::None
            }
            MouseEventKind::ScrollDown => {
                match self.step {
                    CreationStep::SelectTemplate => {
                        self.cycle_template_next();
                    }
                    CreationStep::ConfigureDetails => {
                        if self.focus == CreationFocus::Agent {
                            self.cycle_agent_next();
                        } else {
                            self.next_focus();
                        }
                    }
                }
                CreationModalAction::None
            }
            _ => CreationModalAction::None,
        }
    }

    /// Renders the unified Project Creation Modal into the given `Buffer`.
    pub fn render_modal(&self, area: Rect, buf: &mut Buffer, theme: &Theme) {
        let width = 76.min(area.width.saturating_sub(4));
        let height = 18.min(area.height.saturating_sub(2));

        if width < 25 || height < 10 {
            return;
        }

        let x = area.x + (area.width.saturating_sub(width)) / 2;
        let y = area.y + (area.height.saturating_sub(height)) / 2;
        let modal_area = Rect::new(x, y, width, height);

        Clear.render(modal_area, buf);

        match self.step {
            CreationStep::SelectTemplate => {
                self.render_step1(modal_area, buf, theme);
            }
            CreationStep::ConfigureDetails => {
                self.render_step2(modal_area, buf, theme);
            }
        }
    }

    /// Renders Step 1: Searchable Template Selection with master-detail split layout.
    fn render_step1(&self, modal_area: Rect, buf: &mut Buffer, theme: &Theme) {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Create New Project ")
            .border_style(
                Style::default()
                    .fg(theme.modal_title)
                    .add_modifier(Modifier::BOLD),
            );

        let inner = block.inner(modal_area);
        block.render(modal_area, buf);

        if inner.height < 6 || inner.width < 20 {
            return;
        }

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // Top filter row
                Constraint::Length(1), // Divider
                Constraint::Min(4),    // Body (Left: template list, Right: preview)
                Constraint::Length(1), // Divider
                Constraint::Length(1), // Footer hint
            ])
            .split(inner);

        let filter_area = chunks[0];
        let top_divider_area = chunks[1];
        let body_area = chunks[2];
        let bottom_divider_area = chunks[3];
        let footer_area = chunks[4];

        // 1. Render Filter Bar
        let filter_line = Line::from(vec![
            Span::styled(
                "Filter: ",
                Style::default()
                    .fg(theme.modal_label)
                    .add_modifier(Modifier::BOLD),
            ),
            if self.template_search_query.is_empty() {
                Span::styled(
                    "<type to filter templates>",
                    Style::default().fg(theme.input_placeholder),
                )
            } else {
                Span::styled(
                    &self.template_search_query,
                    Style::default()
                        .fg(theme.input_text)
                        .add_modifier(Modifier::BOLD),
                )
            },
        ]);
        buf.set_line(filter_area.x, filter_area.y, &filter_line, filter_area.width);

        // 2. Top Divider
        let divider_str = "─".repeat(top_divider_area.width as usize);
        let divider_line = Line::from(Span::styled(
            divider_str,
            Style::default().fg(theme.border_unfocused),
        ));
        buf.set_line(
            top_divider_area.x,
            top_divider_area.y,
            &divider_line,
            top_divider_area.width,
        );

        // 3. Body Split: Left List vs Right Preview
        let left_width = 28.min(body_area.width.saturating_sub(20));
        let body_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(left_width), Constraint::Min(20)])
            .split(body_area);

        let left_area = body_chunks[0];
        let right_area = body_chunks[1];

        // Left: Matching Templates List
        let matching = self.matching_template_indices();
        let mut left_lines = vec![
            Line::from(Span::styled(
                format!(" Templates ({})", matching.len()),
                Style::default()
                    .fg(theme.modal_label)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
        ];

        if matching.is_empty() {
            left_lines.push(Line::from(Span::styled(
                "  (No matches)",
                Style::default().fg(theme.input_placeholder),
            )));
        } else {
            let max_display = (left_area.height as usize).saturating_sub(2);
            let start_idx = if self.selected_template_idx >= max_display {
                self.selected_template_idx.saturating_sub(max_display.saturating_sub(1))
            } else {
                0
            };

            for (match_i, &tpl_idx) in matching.iter().enumerate().skip(start_idx).take(max_display) {
                let tpl_name = &self.templates[tpl_idx];
                let is_selected = match_i == self.selected_template_idx;
                if is_selected {
                    left_lines.push(Line::from(vec![
                        Span::styled(
                            " > ",
                            Style::default()
                                .fg(theme.selection_indicator)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            tpl_name,
                            Style::default()
                                .fg(theme.selection_indicator)
                                .add_modifier(Modifier::BOLD),
                        ),
                    ]));
                } else {
                    left_lines.push(Line::from(vec![
                        Span::raw("   "),
                        Span::styled(tpl_name, Style::default().fg(theme.status_bar_fg)),
                    ]));
                }
            }
        }
        let left_par = Paragraph::new(left_lines);
        left_par.render(left_area, buf);

        // Right: Template Preview
        let right_block = Block::default()
            .borders(Borders::LEFT)
            .border_style(Style::default().fg(theme.border_unfocused))
            .title(" Preview ");
        let right_inner = right_block.inner(right_area);
        right_block.render(right_area, buf);

        let mut right_lines = Vec::new();
        if let Some(tpl_name) = self.selected_template() {
            if let Some(tpl) = self.config.get_template(tpl_name) {
                right_lines.push(Line::from(vec![
                    Span::styled(
                        "Template: ",
                        Style::default()
                            .fg(theme.modal_label)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        tpl_name,
                        Style::default()
                            .fg(theme.input_text)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]));

                let desc = tpl.description.as_deref().unwrap_or("None");
                right_lines.push(Line::from(vec![
                    Span::styled("Description: ", Style::default().fg(theme.modal_label)),
                    Span::styled(desc, Style::default().fg(theme.input_text)),
                ]));

                let agent_name = tpl
                    .default_agent
                    .as_deref()
                    .unwrap_or(&self.config.default_agent);
                right_lines.push(Line::from(vec![
                    Span::styled("Default Agent: ", Style::default().fg(theme.modal_label)),
                    Span::styled(agent_name, Style::default().fg(theme.input_text)),
                ]));

                right_lines.push(Line::from(vec![
                    Span::styled(
                        format!("Repositories ({}):", tpl.repos.len()),
                        Style::default()
                            .fg(theme.modal_label)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]));

                if tpl.repos.is_empty() {
                    right_lines.push(Line::from(Span::styled(
                        "  (none)",
                        Style::default().fg(theme.input_placeholder),
                    )));
                } else {
                    for repo in &tpl.repos {
                        let name = crate::config::derive_repo_name(repo);
                        let display_str = if repo != &name {
                            format!("  • {name} ({repo})")
                        } else {
                            format!("  • {name}")
                        };
                        right_lines.push(Line::from(Span::styled(
                            display_str,
                            Style::default().fg(theme.status_bar_fg),
                        )));
                    }
                }

                if let Some(ref custom) = tpl.agents_md_append {
                    let trimmed = custom.trim();
                    if !trimmed.is_empty() {
                        right_lines.push(Line::from(vec![
                            Span::styled(
                                "Custom Instructions:",
                                Style::default()
                                    .fg(theme.modal_label)
                                    .add_modifier(Modifier::BOLD),
                            ),
                        ]));
                        for line in trimmed.lines().take(2) {
                            right_lines.push(Line::from(Span::styled(
                                format!("  {line}"),
                                Style::default().fg(theme.input_placeholder),
                            )));
                        }
                    }
                }
            }
        } else {
            right_lines.push(Line::from(Span::styled(
                "No template selected",
                Style::default().fg(theme.input_placeholder),
            )));
        }
        let right_par = Paragraph::new(right_lines);
        right_par.render(right_inner, buf);

        // 4. Bottom Divider
        let bottom_divider_str = "─".repeat(bottom_divider_area.width as usize);
        let bottom_divider_line = Line::from(Span::styled(
            bottom_divider_str,
            Style::default().fg(theme.border_unfocused),
        ));
        buf.set_line(
            bottom_divider_area.x,
            bottom_divider_area.y,
            &bottom_divider_line,
            bottom_divider_area.width,
        );

        // 5. Footer Hints
        let footer_line = Line::from(Span::styled(
            " [↑/↓/j/k] Select | [Enter] Next (Configure) | [Esc] Cancel ",
            Style::default()
                .fg(theme.key_hint)
                .add_modifier(Modifier::BOLD),
        ));
        buf.set_line(footer_area.x, footer_area.y, &footer_line, footer_area.width);
    }

    /// Renders Step 2: Project Details & Configuration.
    fn render_step2(&self, modal_area: Rect, buf: &mut Buffer, theme: &Theme) {
        let tpl_name = self.selected_template().unwrap_or("default");
        let title = format!(" Create New Project (Template: {tpl_name}) ");

        let block = Block::default()
            .borders(Borders::ALL)
            .title(title)
            .border_style(
                Style::default()
                    .fg(theme.modal_title)
                    .add_modifier(Modifier::BOLD),
            );

        let inner = block.inner(modal_area);
        block.render(modal_area, buf);

        if inner.height < 6 || inner.width < 20 {
            return;
        }

        let proj_focused = self.focus == CreationFocus::ProjectName;
        let agent_focused = self.focus == CreationFocus::Agent;
        let prompt_focused = self.focus == CreationFocus::Prompt;

        let proj_label_style = if proj_focused {
            Style::default()
                .fg(theme.modal_label)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.status_bar_fg)
        };

        let agent_label_style = if agent_focused {
            Style::default()
                .fg(theme.modal_label)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.status_bar_fg)
        };

        let prompt_label_style = if prompt_focused {
            Style::default()
                .fg(theme.modal_label)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.status_bar_fg)
        };

        let proj_prefix = if proj_focused { "▶ " } else { "  " };
        let agent_prefix = if agent_focused { "▶ " } else { "  " };
        let prompt_prefix = if prompt_focused { "▶ " } else { "  " };

        // 1. Template summary info
        let (repo_preview_str, tpl_desc_str) = if let Some(tpl) = self.config.get_template(tpl_name) {
            let repo_count = tpl.repos.len();
            let repo_names = tpl
                .repos
                .iter()
                .map(|r| crate::config::derive_repo_name(r))
                .collect::<Vec<_>>()
                .join(", ");
            let r_str = format!(
                "  Template: {} | Repos ({}): {}",
                tpl_name,
                repo_count,
                if repo_names.is_empty() { "none" } else { &repo_names }
            );
            let d_str = tpl
                .description
                .as_deref()
                .map(|d| format!("    Description: {d}"))
                .unwrap_or_default();
            (r_str, d_str)
        } else {
            (format!("  Template: {tpl_name}"), String::new())
        };

        // 2. Project Name input with validation badge
        let proj_display = if self.project_name_input.is_empty() && !proj_focused {
            Span::styled(
                "<enter project name>",
                Style::default().fg(theme.input_placeholder),
            )
        } else {
            let cursor = if proj_focused { " " } else { "" };
            Span::styled(
                format!("{}{}", self.project_name_input, cursor),
                Style::default()
                    .fg(theme.input_text)
                    .add_modifier(Modifier::BOLD),
            )
        };

        let badge_span = if self.project_name_input.trim().is_empty() {
            Span::styled(" [Required] ", Style::default().fg(theme.input_placeholder))
        } else if self.validation.is_valid {
            Span::styled(
                " [✓ Ready] ",
                Style::default()
                    .fg(theme.status_done)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            Span::styled(
                " [✗ Collision] ",
                Style::default()
                    .fg(theme.status_error)
                    .add_modifier(Modifier::BOLD),
            )
        };

        // 3. Agent Tool Selector
        let agent_name = self.selected_agent();
        let agent_count_str = format!(
            " ◀ {} ▶  ({}/{})",
            agent_name,
            self.selected_agent_idx + 1,
            self.agent_list.len()
        );
        let agent_display = Span::styled(
            agent_count_str,
            Style::default()
                .fg(if agent_focused {
                    theme.selection_indicator
                } else {
                    theme.input_text
                })
                .add_modifier(Modifier::BOLD),
        );

        // 4. Initial Prompt
        let prompt_display = if self.prompt_input.is_empty() && !prompt_focused {
            Span::styled(
                "<optional task instruction prompt>",
                Style::default().fg(theme.input_placeholder),
            )
        } else {
            let cursor = if prompt_focused { " " } else { "" };
            Span::styled(
                format!("{}{}", self.prompt_input, cursor),
                Style::default().fg(theme.input_text),
            )
        };

        // 5. Validation Status Banner
        let banner_line = if let Some(msg) = self.validation.primary_message() {
            if self.validation.is_valid {
                Line::from(vec![
                    Span::styled(" ✓ ", Style::default().fg(theme.status_done).add_modifier(Modifier::BOLD)),
                    Span::styled(msg, Style::default().fg(theme.status_done)),
                ])
            } else if self.project_name_input.trim().is_empty() {
                Line::from(vec![
                    Span::styled(" ℹ ", Style::default().fg(theme.modal_label)),
                    Span::styled(msg, Style::default().fg(theme.input_placeholder)),
                ])
            } else {
                Line::from(vec![
                    Span::styled(" ✗ ", Style::default().fg(theme.status_error).add_modifier(Modifier::BOLD)),
                    Span::styled(msg, Style::default().fg(theme.status_error)),
                ])
            }
        } else if self.validation.is_valid {
            Line::from(vec![
                Span::styled(" ✓ ", Style::default().fg(theme.status_done).add_modifier(Modifier::BOLD)),
                Span::styled("Ready to initialize project worktrees and spawn agent", Style::default().fg(theme.status_done)),
            ])
        } else {
            Line::from("")
        };

        // 6. Footer Hints
        let footer_text = if self.templates.len() > 1 {
            " [Tab] Next | [←/→] Agent | [Enter] Create Project | [Esc/Bksp] Back "
        } else {
            " [Tab] Next | [←/→] Agent | [Enter] Create Project | [Esc] Cancel "
        };
        let footer_line = Line::from(Span::styled(
            footer_text,
            Style::default()
                .fg(theme.key_hint)
                .add_modifier(Modifier::BOLD),
        ));

        let mut lines = vec![
            // Row 0: Template summary
            Line::from(Span::styled(
                repo_preview_str,
                Style::default().fg(theme.input_placeholder),
            )),
        ];

        if !tpl_desc_str.is_empty() && inner.height >= 14 {
            lines.push(Line::from(Span::styled(
                tpl_desc_str,
                Style::default().fg(theme.input_placeholder),
            )));
        } else {
            lines.push(Line::from(""));
        }

        // Row 2: Empty
        lines.push(Line::from(""));

        // Row 3: Project Name
        lines.push(Line::from(vec![
            Span::styled(format!("{proj_prefix}Project Name: "), proj_label_style),
            proj_display,
            Span::raw(" "),
            badge_span,
        ]));

        // Row 4: Agent
        lines.push(Line::from(vec![
            Span::styled(format!("{agent_prefix}Agent CLI: "), agent_label_style),
            agent_display,
        ]));

        // Row 5: Prompt
        lines.push(Line::from(vec![
            Span::styled(format!("{prompt_prefix}Initial Prompt: "), prompt_label_style),
            prompt_display,
        ]));

        lines.push(Line::from(""));
        lines.push(banner_line);
        lines.push(Line::from(""));
        lines.push(footer_line);

        let paragraph = Paragraph::new(lines);
        paragraph.render(inner, buf);
    }
}

impl Widget for &ProjectCreationModal {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let theme = Theme::default();
        self.render_modal(area, buf, &theme);
    }
}

impl Widget for ProjectCreationModal {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let theme = Theme::default();
        self.render_modal(area, buf, &theme);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use std::fs;
    use tempfile::TempDir;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn init_mock_repo(path: &Path) {
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
        fs::write(path.join("README.md"), "# Mock Repo\n").unwrap();
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
    fn test_single_template_bypass_and_lifecycle() {
        let toml_str = r#"
            default_agent = "claude"
            repos_dir = ".repos"
            [agents.claude]
            command = "claude"
            [agents.agy]
            command = "agy"
            [templates.ecommerce]
            description = "E-Commerce Services"
            default_agent = "agy"
            repos = ["api-service", "web-frontend"]
        "#;
        let config = Config::load_from_str(toml_str).unwrap();
        let temp_dir = TempDir::new().unwrap();
        let cwd = temp_dir.path();

        let mut modal = ProjectCreationModal::new(&config, cwd, Some("ecommerce"));

        // Single template bypass should initialize in ConfigureDetails
        assert_eq!(modal.step, CreationStep::ConfigureDetails);
        assert_eq!(modal.selected_template(), Some("ecommerce"));
        assert_eq!(modal.selected_agent(), "agy");
        assert_eq!(modal.focus, CreationFocus::ProjectName);
        assert!(!modal.validation.is_valid); // Name is empty

        // Type project name: "feat-pay"
        for c in "feat-pay".chars() {
            let act = modal.handle_key(key(KeyCode::Char(c)));
            assert_eq!(act, CreationModalAction::None);
        }
        assert_eq!(modal.project_name_input, "feat-pay");
        assert!(modal.validation.is_valid);

        // Tab to Agent selector
        modal.handle_key(key(KeyCode::Tab));
        assert_eq!(modal.focus, CreationFocus::Agent);

        // Cycle agent with Right arrow
        let initial_agent = modal.selected_agent().to_string();
        modal.handle_key(key(KeyCode::Right));
        assert_ne!(modal.selected_agent(), &initial_agent);

        // Tab to Prompt
        modal.handle_key(key(KeyCode::Tab));
        assert_eq!(modal.focus, CreationFocus::Prompt);

        // Type initial prompt
        for c in "Do task".chars() {
            modal.handle_key(key(KeyCode::Char(c)));
        }
        assert_eq!(modal.prompt_input, "Do task");

        // Submit via Enter
        let submit_action = modal.handle_key(key(KeyCode::Enter));
        match submit_action {
            CreationModalAction::Submit {
                template,
                project,
                agent_override,
                prompt,
            } => {
                assert_eq!(template, "ecommerce");
                assert_eq!(project, "feat-pay");
                assert!(agent_override.is_some());
                assert_eq!(prompt.as_deref(), Some("Do task"));
            }
            other => panic!("Expected Submit action, got: {other:?}"),
        }
    }

    #[test]
    fn test_step1_search_filtering_by_name_description_and_repo_urls() {
        let toml_str = r#"
            default_agent = "claude"
            repos_dir = ".repos"
            [templates.ecommerce]
            description = "Online shop microservices"
            repos = ["https://github.com/example/shop-api.git", "shop-web"]

            [templates.analytics]
            description = "Data warehouse and ETL pipelines"
            repos = ["data-stream", "spark-jobs"]

            [templates.infra]
            description = "Kubernetes terraform modules"
            repos = ["https://github.com/example/k8s-terraform.git"]
        "#;
        let config = Config::load_from_str(toml_str).unwrap();
        let temp_dir = TempDir::new().unwrap();
        let cwd = temp_dir.path();

        let mut modal = ProjectCreationModal::new(&config, cwd, None);
        assert_eq!(modal.step, CreationStep::SelectTemplate);
        assert_eq!(modal.templates.len(), 3);
        assert_eq!(modal.matching_templates().len(), 3);

        // 1. Filter by template name substring "ecom"
        modal.template_search_query = "ecom".to_string();
        assert_eq!(modal.matching_templates(), vec!["ecommerce".to_string()]);

        // 2. Case-insensitive match "ANALYTICS"
        modal.template_search_query = "ANALYTICS".to_string();
        assert_eq!(modal.matching_templates(), vec!["analytics".to_string()]);

        // 3. Match against description "warehouse"
        modal.template_search_query = "warehouse".to_string();
        assert_eq!(modal.matching_templates(), vec!["analytics".to_string()]);

        // 4. Match against repo URL "k8s-terraform"
        modal.template_search_query = "k8s-terraform".to_string();
        assert_eq!(modal.matching_templates(), vec!["infra".to_string()]);

        // 5. Match against derived repo name "shop-api"
        modal.template_search_query = "shop-api".to_string();
        assert_eq!(modal.matching_templates(), vec!["ecommerce".to_string()]);

        // 6. Non-matching query
        modal.template_search_query = "nonexistent-xyz".to_string();
        assert!(modal.matching_templates().is_empty());
        assert_eq!(modal.selected_template(), None);
    }

    #[test]
    fn test_step1_navigation_and_search_query_key_handling() {
        let toml_str = r#"
            default_agent = "claude"
            repos_dir = ".repos"
            [templates.alpha]
            repos = ["repo-a"]
            [templates.beta]
            repos = ["repo-b"]
            [templates.gamma]
            repos = ["repo-c"]
        "#;
        let config = Config::load_from_str(toml_str).unwrap();
        let temp_dir = TempDir::new().unwrap();
        let cwd = temp_dir.path();

        let mut modal = ProjectCreationModal::new(&config, cwd, Some("alpha"));
        assert_eq!(modal.step, CreationStep::SelectTemplate);
        assert_eq!(modal.selected_template(), Some("alpha"));

        // Navigate down
        modal.handle_key(key(KeyCode::Down));
        assert_eq!(modal.selected_template(), Some("beta"));

        modal.handle_key(key(KeyCode::Down));
        assert_eq!(modal.selected_template(), Some("gamma"));

        // Wraps around
        modal.handle_key(key(KeyCode::Down));
        assert_eq!(modal.selected_template(), Some("alpha"));

        // Navigate up wraps
        modal.handle_key(key(KeyCode::Up));
        assert_eq!(modal.selected_template(), Some("gamma"));

        // Typing query narrows selection
        modal.handle_key(key(KeyCode::Char('b')));
        modal.handle_key(key(KeyCode::Char('e')));
        assert_eq!(modal.template_search_query, "be");
        assert_eq!(modal.matching_templates(), vec!["beta".to_string()]);
        assert_eq!(modal.selected_template(), Some("beta"));

        // Backspace widens selection
        modal.handle_key(key(KeyCode::Backspace));
        modal.handle_key(key(KeyCode::Backspace));
        assert_eq!(modal.template_search_query, "");
        assert_eq!(modal.matching_templates().len(), 3);
    }

    #[test]
    fn test_two_step_transitions_and_backtracking() {
        let toml_str = r#"
            default_agent = "claude"
            repos_dir = ".repos"
            [templates.billing]
            description = "Billing services"
            default_agent = "agy"
            repos = ["billing-core"]
            [templates.catalog]
            description = "Product catalog"
            default_agent = "claude"
            repos = ["catalog-core"]
        "#;
        let config = Config::load_from_str(toml_str).unwrap();
        let temp_dir = TempDir::new().unwrap();
        let cwd = temp_dir.path();

        let mut modal = ProjectCreationModal::new(&config, cwd, Some("billing"));
        assert_eq!(modal.step, CreationStep::SelectTemplate);
        assert_eq!(modal.selected_template(), Some("billing"));

        // Navigate to catalog
        modal.handle_key(key(KeyCode::Down));
        assert_eq!(modal.selected_template(), Some("catalog"));

        // Enter advances to Step 2: ConfigureDetails
        let act = modal.handle_key(key(KeyCode::Enter));
        assert_eq!(act, CreationModalAction::None);
        assert_eq!(modal.step, CreationStep::ConfigureDetails);
        assert_eq!(modal.selected_template(), Some("catalog"));
        assert_eq!(modal.selected_agent(), "claude");
        assert_eq!(modal.focus, CreationFocus::ProjectName);

        // Esc in Step 2 backtracks to Step 1
        let act = modal.handle_key(key(KeyCode::Esc));
        assert_eq!(act, CreationModalAction::None);
        assert_eq!(modal.step, CreationStep::SelectTemplate);
        assert_eq!(modal.selected_template(), Some("catalog"));

        // Enter again to advance to Step 2
        modal.handle_key(key(KeyCode::Enter));
        assert_eq!(modal.step, CreationStep::ConfigureDetails);

        // Backspace on empty project name also backtracks to Step 1
        assert!(modal.project_name_input.is_empty());
        let act = modal.handle_key(key(KeyCode::Backspace));
        assert_eq!(act, CreationModalAction::None);
        assert_eq!(modal.step, CreationStep::SelectTemplate);

        // Esc in Step 1 cancels the modal
        let act = modal.handle_key(key(KeyCode::Esc));
        assert_eq!(act, CreationModalAction::Cancel);
    }

    #[test]
    fn test_step1_cursor_position_on_filter_bar() {
        let toml_str = r#"
            default_agent = "claude"
            repos_dir = ".repos"
            [templates.t1]
            repos = ["r1"]
            [templates.t2]
            repos = ["r2"]
        "#;
        let config = Config::load_from_str(toml_str).unwrap();
        let temp_dir = TempDir::new().unwrap();
        let mut modal = ProjectCreationModal::new(&config, temp_dir.path(), None);
        assert_eq!(modal.step, CreationStep::SelectTemplate);

        let area = Rect::new(0, 0, 100, 30);
        let pos = modal.cursor_position(area);
        // inner.x = 13, prefix_len = 8 ("Filter: "), query len = 0 -> 21, inner.y = 7
        assert_eq!(pos, Some(Position::new(21, 7)));

        modal.template_search_query = "xyz".to_string();
        let pos = modal.cursor_position(area);
        assert_eq!(pos, Some(Position::new(24, 7)));
    }

    #[test]
    fn test_step1_and_step2_rendering_master_detail() {
        let toml_str = r#"
            default_agent = "claude"
            repos_dir = ".repos"
            [templates.billing]
            description = "Billing & Payments"
            default_agent = "agy"
            agents_md_append = "Run cargo test before commit."
            repos = ["https://github.com/example/billing-api.git", "billing-web"]
            [templates.shipping]
            description = "Logistics service"
            repos = ["shipping-core"]
        "#;
        let config = Config::load_from_str(toml_str).unwrap();
        let temp_dir = TempDir::new().unwrap();
        let mut modal = ProjectCreationModal::new(&config, temp_dir.path(), Some("billing"));

        let area = Rect::new(0, 0, 100, 30);
        let theme = Theme::default();

        // 1. Render Step 1
        let mut buf1 = Buffer::empty(area);
        modal.render_modal(area, &mut buf1, &theme);
        let text1 = buf1
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect::<String>();

        assert!(text1.contains("Create New Project"));
        assert!(text1.contains("Filter:"));
        assert!(text1.contains("Templates (2)"));
        assert!(text1.contains("billing"));
        assert!(text1.contains("shipping"));
        assert!(text1.contains("Preview"));
        assert!(text1.contains("Billing & Payments"));
        assert!(text1.contains("Repositories (2):"));
        assert!(text1.contains("billing-api"));
        assert!(text1.contains("Run cargo test"));
        assert!(text1.contains("[Enter] Next (Configure)"));

        // 2. Advance to Step 2 and render
        modal.handle_key(key(KeyCode::Enter));
        assert_eq!(modal.step, CreationStep::ConfigureDetails);

        let mut buf2 = Buffer::empty(area);
        modal.render_modal(area, &mut buf2, &theme);
        let text2 = buf2
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect::<String>();

        assert!(text2.contains("Create New Project (Template: billing)"));
        assert!(text2.contains("Project Name:"));
        assert!(text2.contains("Agent CLI:"));
        assert!(text2.contains("Initial Prompt:"));
        assert!(text2.contains("[Enter] Create Project"));
    }

    #[test]
    fn test_creation_modal_live_validation_and_branch_collision() {
        let temp_dir = TempDir::new().unwrap();
        let repo_dir = temp_dir.path().join("core_service");
        init_mock_repo(&repo_dir);

        // Create collision branch "existing-branch"
        let _ = std::process::Command::new("git")
            .args(["branch", "existing-branch"])
            .current_dir(&repo_dir)
            .status();

        let toml_str = format!(
            r#"
            default_agent = "claude"
            repos_dir = "{}"
            [templates.core_tpl]
            repos = ["{}"]
            "#,
            temp_dir.path().display(),
            repo_dir.display()
        );
        let config = Config::load_from_str(&toml_str).unwrap();
        let mut modal = ProjectCreationModal::new(&config, temp_dir.path(), Some("core_tpl"));

        // Type colliding name
        for c in "existing-branch".chars() {
            modal.handle_key(key(KeyCode::Char(c)));
        }

        assert!(!modal.validation.is_valid);
        assert!(modal.validation.primary_message().unwrap().contains("already exists in repository"));

        // Pressing Enter on invalid input should return None
        let act = modal.handle_key(key(KeyCode::Enter));
        assert_eq!(act, CreationModalAction::None);

        // Clear and type valid name
        for _ in 0.."existing-branch".len() {
            modal.handle_key(key(KeyCode::Backspace));
        }
        for c in "clean-branch".chars() {
            modal.handle_key(key(KeyCode::Char(c)));
        }

        assert!(modal.validation.is_valid);
        let act = modal.handle_key(key(KeyCode::Enter));
        assert!(matches!(act, CreationModalAction::Submit { project, .. } if project == "clean-branch"));
    }

    #[test]
    fn test_creation_modal_cursor_position_calculation() {
        let config = Config::load_from_str("default_agent = \"claude\"\nrepos_dir = \".repos\"\n").unwrap();
        let temp_dir = TempDir::new().unwrap();
        let mut modal = ProjectCreationModal::new(&config, temp_dir.path(), None);

        let area = Rect::new(0, 0, 100, 30);
        modal.focus = CreationFocus::ProjectName;
        modal.project_name_input = "my-proj".to_string();

        let pos = modal.cursor_position(area);
        assert_eq!(pos, Some(Position::new(36, 10)));

        modal.focus = CreationFocus::Prompt;
        modal.prompt_input = "fix bug".to_string();
        let pos = modal.cursor_position(area);
        assert_eq!(pos, Some(Position::new(38, 12)));

        modal.focus = CreationFocus::Template;
        assert_eq!(modal.cursor_position(area), None);

        modal.focus = CreationFocus::Agent;
        assert_eq!(modal.cursor_position(area), None);
    }
}
