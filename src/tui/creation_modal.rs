use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use ratatui::{
    buffer::Buffer,
    layout::{Position, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Widget},
};

use crate::config::Config;
use crate::project::{ProjectBuilder, ProjectValidation};
use crate::tui::theme::Theme;

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

/// Unified interactive project creation modal used across Dashboard and In-Session Quick Launcher.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectCreationModal {
    pub config: Config,
    pub cwd: PathBuf,
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

        let selected_template_idx = default_template
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
            .get(selected_template_idx)
            .and_then(|tpl_name| config.get_template(tpl_name))
            .and_then(|tpl| tpl.default_agent.as_ref())
            .unwrap_or(&config.default_agent);

        let selected_agent_idx = agent_list
            .iter()
            .position(|a| a == target_agent)
            .unwrap_or(0);

        let mut modal = Self {
            config: config.clone(),
            cwd: cwd.to_path_buf(),
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

    /// Sets the active field focus.
    pub fn with_focus(mut self, focus: CreationFocus) -> Self {
        self.focus = focus;
        self
    }

    /// Returns the currently selected template name, if available.
    pub fn selected_template(&self) -> Option<&str> {
        if !self.templates.is_empty() && self.selected_template_idx < self.templates.len() {
            Some(&self.templates[self.selected_template_idx])
        } else {
            None
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
        if !self.templates.is_empty() {
            self.selected_template_idx =
                (self.selected_template_idx + 1) % self.templates.len();
            self.sync_agent_with_template();
            self.revalidate();
        }
    }

    /// Cycles to the previous available template and updates default agent override if appropriate.
    pub fn cycle_template_prev(&mut self) {
        if !self.templates.is_empty() {
            self.selected_template_idx =
                (self.selected_template_idx + self.templates.len() - 1) % self.templates.len();
            self.sync_agent_with_template();
            self.revalidate();
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
            CreationFocus::Template => CreationFocus::ProjectName,
            CreationFocus::ProjectName => CreationFocus::Agent,
            CreationFocus::Agent => CreationFocus::Prompt,
            CreationFocus::Prompt => CreationFocus::Template,
        };
    }

    /// Moves focus to the previous input field.
    pub fn prev_focus(&mut self) {
        self.focus = match self.focus {
            CreationFocus::Template => CreationFocus::Prompt,
            CreationFocus::ProjectName => CreationFocus::Template,
            CreationFocus::Agent => CreationFocus::ProjectName,
            CreationFocus::Prompt => CreationFocus::Agent,
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

        // Layout rows:
        // row 0: Template selector (height 1)
        // row 1: Template preview info (height 2)
        // row 2: Empty line (height 1)
        // row 3: Project Name input (height 1)
        // row 4: Agent selector (height 1)
        // row 5: Initial Prompt input (height 1)

        match self.focus {
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
        }
    }

    /// Handles keyboard events and returns any triggered `CreationModalAction`.
    pub fn handle_key(&mut self, key: KeyEvent) -> CreationModalAction {
        let has_alt = key.modifiers.contains(KeyModifiers::ALT);
        let has_ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

        if key.code == KeyCode::Esc
            || (has_ctrl && (key.code == KeyCode::Char('c') || key.code == KeyCode::Char('C')))
        {
            return CreationModalAction::Cancel;
        }

        match key.code {
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
                match self.focus {
                    CreationFocus::Template => {
                        self.cycle_template_prev();
                    }
                    CreationFocus::Agent => {
                        self.cycle_agent_prev();
                    }
                    _ => {}
                }
                CreationModalAction::None
            }
            KeyCode::Right => {
                match self.focus {
                    CreationFocus::Template => {
                        self.cycle_template_next();
                    }
                    CreationFocus::Agent => {
                        self.cycle_agent_next();
                    }
                    _ => {}
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
                    CreationFocus::Template => {
                        if c == 'h' || c == 'k' {
                            self.cycle_template_prev();
                        } else if c == 'l' || c == 'j' {
                            self.cycle_template_next();
                        }
                    }
                    CreationFocus::Agent => {
                        if c == 'h' || c == 'k' {
                            self.cycle_agent_prev();
                        } else if c == 'l' || c == 'j' {
                            self.cycle_agent_next();
                        }
                    }
                }
                CreationModalAction::None
            }
            KeyCode::Backspace => {
                match self.focus {
                    CreationFocus::ProjectName => {
                        self.project_name_input.pop();
                        self.revalidate();
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
        }
    }

    /// Handles mouse events (e.g. scroll wheel) and returns any triggered `CreationModalAction`.
    pub fn handle_mouse(&mut self, mouse: MouseEvent) -> CreationModalAction {
        match mouse.kind {
            MouseEventKind::ScrollUp => {
                match self.focus {
                    CreationFocus::Template => self.cycle_template_prev(),
                    CreationFocus::Agent => self.cycle_agent_prev(),
                    _ => self.prev_focus(),
                }
                CreationModalAction::None
            }
            MouseEventKind::ScrollDown => {
                match self.focus {
                    CreationFocus::Template => self.cycle_template_next(),
                    CreationFocus::Agent => self.cycle_agent_next(),
                    _ => self.next_focus(),
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

        let tpl_focused = self.focus == CreationFocus::Template;
        let proj_focused = self.focus == CreationFocus::ProjectName;
        let agent_focused = self.focus == CreationFocus::Agent;
        let prompt_focused = self.focus == CreationFocus::Prompt;

        let tpl_label_style = if tpl_focused {
            Style::default()
                .fg(theme.modal_label)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.status_bar_fg)
        };

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

        let tpl_prefix = if tpl_focused { "▶ " } else { "  " };
        let proj_prefix = if proj_focused { "▶ " } else { "  " };
        let agent_prefix = if agent_focused { "▶ " } else { "  " };
        let prompt_prefix = if prompt_focused { "▶ " } else { "  " };

        // 1. Template Row
        let tpl_display = if self.templates.is_empty() {
            Span::styled(
                "(No templates configured)",
                Style::default().fg(theme.input_placeholder),
            )
        } else {
            let tpl_name = self.selected_template().unwrap_or("default");
            let count_str = format!(
                " ◀ {} ▶  ({}/{})",
                tpl_name,
                self.selected_template_idx + 1,
                self.templates.len()
            );
            Span::styled(
                count_str,
                Style::default()
                    .fg(if tpl_focused {
                        theme.selection_indicator
                    } else {
                        theme.input_text
                    })
                    .add_modifier(Modifier::BOLD),
            )
        };

        // 1b. Template Preview Info
        let (repo_preview_str, tpl_desc_str) = if let Some(tpl_name) = self.selected_template() {
            if let Some(tpl) = self.config.get_template(tpl_name) {
                let repo_count = tpl.repos.len();
                let repo_names = tpl
                    .repos
                    .iter()
                    .map(|r| crate::config::derive_repo_name(r))
                    .collect::<Vec<_>>()
                    .join(", ");
                let r_str = format!(
                    "    Repos ({}): {}",
                    repo_count,
                    if repo_names.is_empty() { "none" } else { &repo_names }
                );
                let d_str = tpl
                    .description
                    .as_deref()
                    .map(|d| format!("    Description: {}", d))
                    .unwrap_or_default();
                (r_str, d_str)
            } else {
                (String::new(), String::new())
            }
        } else {
            (String::new(), String::new())
        };

        // 2. Project Name Row with Inline Validation Badge
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

        // 3. Agent Tool Selector Row
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

        // 4. Initial Prompt Row
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
        let footer_line = Line::from(Span::styled(
            " [Tab] Next | [←/→] Cycle | [Enter] Create Project | [Esc] Cancel ",
            Style::default()
                .fg(theme.key_hint)
                .add_modifier(Modifier::BOLD),
        ));

        let mut lines = vec![
            // Row 0: Template
            Line::from(vec![
                Span::styled(format!("{}Template: ", tpl_prefix), tpl_label_style),
                tpl_display,
            ]),
            // Row 1: Preview repos
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

        // Row 3: Project Name
        lines.push(Line::from(vec![
            Span::styled(format!("{}Project Name: ", proj_prefix), proj_label_style),
            proj_display,
            Span::raw(" "),
            badge_span,
        ]));

        // Row 4: Agent
        lines.push(Line::from(vec![
            Span::styled(format!("{}Agent CLI: ", agent_prefix), agent_label_style),
            agent_display,
        ]));

        // Row 5: Prompt
        lines.push(Line::from(vec![
            Span::styled(format!("{}Initial Prompt: ", prompt_prefix), prompt_label_style),
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
    fn test_creation_modal_navigation_and_input_lifecycle() {
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
            [templates.analytics]
            repos = ["data-pipeline"]
        "#;
        let config = Config::load_from_str(toml_str).unwrap();
        let temp_dir = TempDir::new().unwrap();
        let cwd = temp_dir.path();

        let mut modal = ProjectCreationModal::new(&config, cwd, Some("ecommerce"));

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
            other => panic!("Expected Submit action, got: {:?}", other),
        }
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

    #[test]
    fn test_creation_modal_rendering() {
        let toml_str = r#"
            default_agent = "claude"
            repos_dir = ".repos"
            [templates.billing]
            description = "Billing & Payments"
            repos = ["billing-api", "billing-web"]
        "#;
        let config = Config::load_from_str(toml_str).unwrap();
        let temp_dir = TempDir::new().unwrap();
        let modal = ProjectCreationModal::new(&config, temp_dir.path(), Some("billing"));

        let area = Rect::new(0, 0, 100, 30);
        let mut buf = Buffer::empty(area);
        let theme = Theme::default();

        modal.render_modal(area, &mut buf, &theme);

        let text = buf
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect::<String>();

        assert!(text.contains("Create New Project"));
        assert!(text.contains("billing"));
        assert!(text.contains("Repos (2)"));
        assert!(text.contains("billing-api"));
        assert!(text.contains("Project Name:"));
        assert!(text.contains("Agent CLI:"));
        assert!(text.contains("Initial Prompt:"));
        assert!(text.contains("[Tab] Next"));
    }
}
