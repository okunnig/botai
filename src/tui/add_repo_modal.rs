use std::fs;
use std::path::Path;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Widget},
};

use crate::config::Config;
use crate::project::extract_template_name_from_agents_md;
use crate::tui::theme::Theme;

/// An item in the repository selection checklist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddRepoItem {
    pub name: String,
    pub selected: bool,
}

/// Actions emitted by the Add Repository modal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddRepoModalAction {
    None,
    Submit {
        project_name: String,
        repos: Vec<String>,
    },
    Cancel,
}

/// State for the Add Repository modal dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddRepoModalState {
    pub project_name: String,
    pub template_name: String,
    pub items: Vec<AddRepoItem>,
    pub selected_idx: usize,
    pub all_included: bool,
    pub warning_message: Option<String>,
}

impl AddRepoModalState {
    /// Creates a new `AddRepoModalState` with explicit items.
    pub fn new(
        project_name: impl Into<String>,
        template_name: impl Into<String>,
        items: Vec<AddRepoItem>,
        all_included: bool,
    ) -> Self {
        Self {
            project_name: project_name.into(),
            template_name: template_name.into(),
            items,
            selected_idx: 0,
            all_included,
            warning_message: None,
        }
    }

    /// Discovers candidate repositories to add from the project workspace and template configuration.
    pub fn from_project(config: &Config, project_dir: &Path) -> anyhow::Result<Self> {
        let project_name = project_dir
            .file_name()
            .and_then(|n| n.to_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow::anyhow!("Invalid project directory name"))?;

        let template_name = extract_template_name_from_agents_md(project_dir).ok_or_else(|| {
            anyhow::anyhow!(
                "Could not determine template from AGENTS.md in {}",
                project_dir.display()
            )
        })?;

        if !config.templates.contains_key(&template_name) {
            anyhow::bail!("Template '{template_name}' not found in configuration");
        }

        let all_template_repos = config.resolve_template_repos(&template_name)?;

        let mut existing_worktree_names = Vec::new();
        if let Ok(entries) = fs::read_dir(project_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() && path.join(".git").exists() {
                    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                        existing_worktree_names.push(name.to_string());
                    }
                }
            }
        }

        let not_included: Vec<AddRepoItem> = all_template_repos
            .into_iter()
            .filter(|r| !existing_worktree_names.contains(&r.name))
            .map(|r| AddRepoItem {
                name: r.name,
                selected: false,
            })
            .collect();

        if not_included.is_empty() {
            Ok(Self::new(project_name, template_name, Vec::new(), true))
        } else {
            Ok(Self::new(project_name, template_name, not_included, false))
        }
    }

    /// Handles a keyboard event and returns any triggered `AddRepoModalAction`.
    pub fn handle_key(&mut self, key: KeyEvent) -> AddRepoModalAction {
        if self.all_included {
            return match key.code {
                KeyCode::Enter | KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char(' ') => {
                    AddRepoModalAction::Cancel
                }
                _ => AddRepoModalAction::None,
            };
        }

        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                if !self.items.is_empty() {
                    self.selected_idx = self.selected_idx.saturating_sub(1);
                }
                AddRepoModalAction::None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if !self.items.is_empty() {
                    self.selected_idx =
                        (self.selected_idx + 1).min(self.items.len().saturating_sub(1));
                }
                AddRepoModalAction::None
            }
            KeyCode::Char(' ') => {
                if let Some(item) = self.items.get_mut(self.selected_idx) {
                    item.selected = !item.selected;
                    self.warning_message = None;
                }
                AddRepoModalAction::None
            }
            KeyCode::Char('a') | KeyCode::Char('A') => {
                let all_sel = self.items.iter().all(|i| i.selected);
                for item in &mut self.items {
                    item.selected = !all_sel;
                }
                self.warning_message = None;
                AddRepoModalAction::None
            }
            KeyCode::Enter => {
                let selected: Vec<String> = self
                    .items
                    .iter()
                    .filter(|i| i.selected)
                    .map(|i| i.name.clone())
                    .collect();

                if selected.is_empty() {
                    self.warning_message =
                        Some("At least one repository must be selected".to_string());
                    AddRepoModalAction::None
                } else {
                    AddRepoModalAction::Submit {
                        project_name: self.project_name.clone(),
                        repos: selected,
                    }
                }
            }
            KeyCode::Esc => AddRepoModalAction::Cancel,
            _ => AddRepoModalAction::None,
        }
    }
}

/// Renders the Add Repository modal into the given `Buffer`.
pub fn render_add_repo_modal(
    state: &AddRepoModalState,
    area: Rect,
    buf: &mut Buffer,
    theme: &Theme,
) {
    let width = 64.min(area.width.saturating_sub(4));
    let height = if state.all_included {
        10.min(area.height.saturating_sub(2))
    } else {
        14.min(area.height.saturating_sub(2))
    };

    if width < 24 || height < 6 {
        return;
    }

    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let modal_area = Rect::new(x, y, width, height);

    Clear.render(modal_area, buf);

    let title = format!(" Add Repositories: {} ", state.project_name);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border_focused))
        .title(Span::styled(
            title,
            Style::default()
                .fg(theme.modal_title)
                .add_modifier(Modifier::BOLD),
        ));

    let inner = block.inner(modal_area);
    block.render(modal_area, buf);

    if state.all_included {
        let text = vec![
            Line::from(Span::styled(
                format!("Template: {}", state.template_name),
                Style::default().fg(theme.modal_label).add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(Span::styled(
                format!(
                    "All repositories defined in template '{}' are already included in project '{}'.",
                    state.template_name, state.project_name
                ),
                Style::default().fg(theme.status_bar_fg),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "[Enter] / [Esc] Dismiss",
                Style::default().fg(theme.key_hint).add_modifier(Modifier::BOLD),
            )),
        ];
        let p = Paragraph::new(text);
        p.render(inner, buf);
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // Template header
            Constraint::Length(1), // Spacer / Divider
            Constraint::Min(3),    // Repo list
            Constraint::Length(1), // Warning or blank
            Constraint::Length(1), // Footer hint
        ])
        .split(inner);

    // Template header
    let tpl_line = Line::from(vec![
        Span::styled(
            "Template: ",
            Style::default()
                .fg(theme.modal_label)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            &state.template_name,
            Style::default().fg(theme.status_bar_fg),
        ),
    ]);
    Paragraph::new(tpl_line).render(chunks[0], buf);

    // Divider
    let divider = Line::from(Span::styled(
        "─".repeat(chunks[1].width as usize),
        Style::default().fg(theme.border_unfocused),
    ));
    Paragraph::new(divider).render(chunks[1], buf);

    // Items list
    let mut item_lines = Vec::new();
    for (idx, item) in state.items.iter().enumerate() {
        let is_focused = idx == state.selected_idx;
        let pointer = if is_focused { "▶ " } else { "  " };
        let checkbox = if item.selected { "[x] " } else { "[ ] " };

        let mut item_spans = Vec::new();
        item_spans.push(Span::styled(
            pointer,
            if is_focused {
                Style::default()
                    .fg(theme.selection_indicator)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.status_bar_fg)
            },
        ));

        item_spans.push(Span::styled(
            checkbox,
            if item.selected {
                Style::default()
                    .fg(theme.status_done)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.status_bar_fg)
            },
        ));

        item_spans.push(Span::styled(
            &item.name,
            if is_focused {
                Style::default()
                    .fg(theme.input_text)
                    .add_modifier(Modifier::BOLD)
            } else if item.selected {
                Style::default()
                    .fg(theme.status_bar_fg)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.status_bar_fg)
            },
        ));

        item_lines.push(Line::from(item_spans));
    }
    Paragraph::new(item_lines).render(chunks[2], buf);

    // Warning message
    if let Some(ref warn) = state.warning_message {
        let warn_line = Line::from(Span::styled(
            format!("✗ {warn}"),
            Style::default()
                .fg(theme.status_error)
                .add_modifier(Modifier::BOLD),
        ));
        Paragraph::new(warn_line).render(chunks[3], buf);
    }

    // Footer
    let footer_line = Line::from(Span::styled(
        "[Space] Toggle | [a] All | [↑/↓] Navigate | [Enter] Add | [Esc] Cancel",
        Style::default()
            .fg(theme.key_hint)
            .add_modifier(Modifier::BOLD),
    ));
    Paragraph::new(footer_line).render(chunks[4], buf);
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, crossterm::event::KeyModifiers::empty())
    }

    #[test]
    fn test_add_repo_modal_state_key_navigation_and_toggle() {
        let items = vec![
            AddRepoItem {
                name: "repo_a".to_string(),
                selected: false,
            },
            AddRepoItem {
                name: "repo_b".to_string(),
                selected: false,
            },
        ];
        let mut state = AddRepoModalState::new("my-proj", "tpl", items, false);

        // Move down
        assert_eq!(
            state.handle_key(key(KeyCode::Down)),
            AddRepoModalAction::None
        );
        assert_eq!(state.selected_idx, 1);

        // Toggle space
        assert_eq!(
            state.handle_key(key(KeyCode::Char(' '))),
            AddRepoModalAction::None
        );
        assert!(state.items[1].selected);
        assert!(!state.items[0].selected);

        // Move up
        assert_eq!(
            state.handle_key(key(KeyCode::Char('k'))),
            AddRepoModalAction::None
        );
        assert_eq!(state.selected_idx, 0);

        // Submit
        let action = state.handle_key(key(KeyCode::Enter));
        assert_eq!(
            action,
            AddRepoModalAction::Submit {
                project_name: "my-proj".to_string(),
                repos: vec!["repo_b".to_string()],
            }
        );
    }

    #[test]
    fn test_add_repo_modal_empty_selection_shows_warning() {
        let items = vec![AddRepoItem {
            name: "repo_a".to_string(),
            selected: false,
        }];
        let mut state = AddRepoModalState::new("my-proj", "tpl", items, false);

        // Submit with 0 selected
        let action = state.handle_key(key(KeyCode::Enter));
        assert_eq!(action, AddRepoModalAction::None);
        assert_eq!(
            state.warning_message,
            Some("At least one repository must be selected".to_string())
        );

        // Cancel with Esc
        let action = state.handle_key(key(KeyCode::Esc));
        assert_eq!(action, AddRepoModalAction::Cancel);
    }

    #[test]
    fn test_add_repo_modal_all_included_dialog() {
        let mut state = AddRepoModalState::new("my-proj", "tpl", Vec::new(), true);

        // Enter or Esc dismisses
        assert_eq!(
            state.handle_key(key(KeyCode::Enter)),
            AddRepoModalAction::Cancel
        );
        assert_eq!(
            state.handle_key(key(KeyCode::Esc)),
            AddRepoModalAction::Cancel
        );
    }

    #[test]
    fn test_add_repo_modal_bulk_toggle() {
        let items = vec![
            AddRepoItem {
                name: "repo_a".to_string(),
                selected: false,
            },
            AddRepoItem {
                name: "repo_b".to_string(),
                selected: false,
            },
        ];
        let mut state = AddRepoModalState::new("my-proj", "tpl", items, false);

        // 'a' selects all
        state.handle_key(key(KeyCode::Char('a')));
        assert!(state.items.iter().all(|i| i.selected));

        // 'a' again deselects all
        state.handle_key(key(KeyCode::Char('a')));
        assert!(state.items.iter().all(|i| !i.selected));
    }

    #[test]
    fn test_add_repo_modal_render() {
        let items = vec![
            AddRepoItem {
                name: "api-service".to_string(),
                selected: true,
            },
            AddRepoItem {
                name: "web-client".to_string(),
                selected: false,
            },
        ];
        let state = AddRepoModalState::new("my-proj", "microservices", items, false);
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        let theme = Theme::default();

        render_add_repo_modal(&state, area, &mut buf, &theme);

        let content = buf
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect::<String>();
        assert!(content.contains("Add Repositories: my-proj"));
        assert!(content.contains("Template: microservices"));
        assert!(content.contains("api-service"));
        assert!(content.contains("web-client"));
    }

    #[test]
    fn test_add_repo_modal_from_project() {
        let temp_dir = TempDir::new().unwrap();
        let proj_dir = temp_dir.path().join("my-project");
        fs::create_dir_all(&proj_dir).unwrap();

        // Write AGENTS.md
        fs::write(
            proj_dir.join("AGENTS.md"),
            "# Project: my-project\n\nTemplate: **tpl**\n\n## Repository Layout\n",
        )
        .unwrap();

        // Create one existing worktree directory
        fs::create_dir_all(proj_dir.join("repo_a").join(".git")).unwrap();

        let toml_str = r#"
        default_agent = "claude"
        repos_dir = "/tmp/repos"
        [templates.tpl]
        repos = ["repo_a", "repo_b"]
        "#;
        let config = Config::load_from_str(toml_str).unwrap();

        let modal = AddRepoModalState::from_project(&config, &proj_dir).unwrap();
        assert_eq!(modal.project_name, "my-project");
        assert_eq!(modal.template_name, "tpl");
        assert!(!modal.all_included);
        assert_eq!(modal.items.len(), 1);
        assert_eq!(modal.items[0].name, "repo_b");
    }
}
