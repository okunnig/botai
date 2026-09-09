use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Position, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Widget, Wrap},
};

use crate::config::Config;
use crate::project::{ProjectBuilder, ProjectValidation};
use crate::tui::text_input::TextInput;
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

pub const PROMPT_VISIBLE_LINES: usize = 3;

/// A wrapped line of prompt text with its character offsets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptLine {
    pub text: String,
    pub start_char_idx: usize,
    pub char_count: usize,
}

/// Result payload emitted by background branch validation worker: `(generation, branch_errors)`.
pub type ValidationResult = (u64, Vec<String>);

/// Shared receiver handle for asynchronous background validation results.
pub type ValidationReceiver = Arc<Mutex<mpsc::Receiver<ValidationResult>>>;

/// Unified interactive project creation modal with two-step searchable template workflow.
#[derive(Debug)]
pub struct ProjectCreationModal {
    pub config: Config,
    pub cwd: PathBuf,
    pub step: CreationStep,
    pub template_search_query: String,
    pub templates: Vec<String>,
    pub selected_template_idx: usize,
    pub project_name_input: TextInput,
    pub project_name_cursor: usize,
    pub agent_list: Vec<String>,
    pub selected_agent_idx: usize,
    pub prompt_input: String,
    pub prompt_cursor: usize,
    pub prompt_scroll_offset: usize,
    pub focus: CreationFocus,
    pub validation: ProjectValidation,
    pub validation_generation: u64,
    validation_tx: mpsc::Sender<ValidationResult>,
    validation_rx: ValidationReceiver,
}

impl Clone for ProjectCreationModal {
    fn clone(&self) -> Self {
        let (validation_tx, validation_rx) = mpsc::channel();
        Self {
            config: self.config.clone(),
            cwd: self.cwd.clone(),
            step: self.step,
            template_search_query: self.template_search_query.clone(),
            templates: self.templates.clone(),
            selected_template_idx: self.selected_template_idx,
            project_name_input: self.project_name_input.clone(),
            project_name_cursor: self.project_name_cursor,
            agent_list: self.agent_list.clone(),
            selected_agent_idx: self.selected_agent_idx,
            prompt_input: self.prompt_input.clone(),
            prompt_cursor: self.prompt_cursor,
            prompt_scroll_offset: self.prompt_scroll_offset,
            focus: self.focus,
            validation: self.validation.clone(),
            validation_generation: self.validation_generation,
            validation_tx,
            validation_rx: Arc::new(Mutex::new(validation_rx)),
        }
    }
}

impl PartialEq for ProjectCreationModal {
    fn eq(&self, other: &Self) -> bool {
        self.config == other.config
            && self.cwd == other.cwd
            && self.step == other.step
            && self.template_search_query == other.template_search_query
            && self.templates == other.templates
            && self.selected_template_idx == other.selected_template_idx
            && self.project_name_input == other.project_name_input
            && self.project_name_cursor == other.project_name_cursor
            && self.agent_list == other.agent_list
            && self.selected_agent_idx == other.selected_agent_idx
            && self.prompt_input == other.prompt_input
            && self.prompt_cursor == other.prompt_cursor
            && self.prompt_scroll_offset == other.prompt_scroll_offset
            && self.focus == other.focus
            && self.validation == other.validation
    }
}

impl Eq for ProjectCreationModal {}

impl ProjectCreationModal {
    /// Creates a new `ProjectCreationModal` initialized with configuration and working directory.
    pub fn new(config: &Config, cwd: &Path, default_template: Option<&str>) -> Self {
        let mut templates: Vec<String> = config.templates.keys().cloned().collect();
        templates.sort();

        // The creation modal always starts in CreationStep::SelectTemplate regardless of template count,
        // ensuring consistent template selection, configuration review, and preview.
        let step = CreationStep::SelectTemplate;

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

        let (validation_tx, validation_rx) = mpsc::channel();

        let mut modal = Self {
            config: config.clone(),
            cwd: cwd.to_path_buf(),
            step,
            template_search_query: String::new(),
            templates,
            selected_template_idx,
            project_name_input: TextInput::new().with_placeholder("<enter project name>"),
            project_name_cursor: 0,
            agent_list,
            selected_agent_idx,
            prompt_input: String::new(),
            prompt_cursor: 0,
            prompt_scroll_offset: 0,
            focus: CreationFocus::ProjectName,
            validation: ProjectValidation::valid(),
            validation_generation: 0,
            validation_tx,
            validation_rx: Arc::new(Mutex::new(validation_rx)),
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
                        (self.selected_template_idx + self.templates.len() - 1)
                            % self.templates.len();
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
                let target = tpl
                    .default_agent
                    .as_deref()
                    .unwrap_or(&self.config.default_agent);
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

    /// Re-evaluates fast pre-validation immediately (0ms) and dispatches background
    /// git branch collision checks across template repositories if fast checks pass.
    pub fn revalidate(&mut self) {
        let tpl_name = self.selected_template().unwrap_or("");
        let builder = ProjectBuilder::new(
            self.config.clone(),
            tpl_name,
            self.project_name_input.value(),
        )
        .with_cwd(&self.cwd)
        .with_agent(self.selected_agent());

        self.validation = builder.validate_fast();
        self.validation_generation = self.validation_generation.wrapping_add(1);

        if self.validation.is_valid && !self.project_name_input.trim().is_empty() {
            let gen = self.validation_generation;
            let tx = self.validation_tx.clone();
            let builder_clone = builder.clone();

            std::thread::spawn(move || {
                let collisions = builder_clone.check_branch_collisions();
                let _ = tx.send((gen, collisions));
            });
        }
    }

    /// Drains pending asynchronous branch validation results and updates `self.validation`
    /// if the generation matches the current active input.
    pub fn poll_validation(&mut self) {
        if let Ok(rx) = self.validation_rx.lock() {
            while let Ok((gen, branch_errors)) = rx.try_recv() {
                if gen == self.validation_generation && self.validation.is_valid {
                    for err in branch_errors {
                        self.validation.add_error(err);
                    }
                }
            }
        }
    }

    /// Advances tick-based state and drains pending validation results.
    pub fn tick(&mut self) {
        self.poll_validation();
    }

    /// Default inner width for initial prompt wrapping.
    pub fn prompt_width(&self) -> usize {
        56
    }

    /// Computes wrapped lines of prompt text according to the given width,
    /// preserving explicit newlines (`\n`) and word-wrapping long lines.
    pub fn compute_wrapped_prompt_lines(&self, width: usize) -> Vec<PromptLine> {
        let width = width.max(1);
        let chars: Vec<char> = self.prompt_input.chars().collect();

        let mut logical_lines: Vec<(usize, Vec<char>, bool)> = Vec::new();
        let mut current_line = Vec::new();
        let mut line_start = 0;

        for (i, &c) in chars.iter().enumerate() {
            if c == '\n' {
                logical_lines.push((line_start, current_line, true));
                current_line = Vec::new();
                line_start = i + 1;
            } else {
                current_line.push(c);
            }
        }
        logical_lines.push((line_start, current_line, false));

        let mut result = Vec::new();

        for (start_idx, line_chars, has_trailing_newline) in logical_lines {
            if line_chars.is_empty() {
                let char_count = if has_trailing_newline { 1 } else { 0 };
                result.push(PromptLine {
                    text: String::new(),
                    start_char_idx: start_idx,
                    char_count,
                });
                continue;
            }

            let mut i = 0;
            while i < line_chars.len() {
                let line_start = i;
                let remaining = line_chars.len() - line_start;
                if remaining <= width {
                    let text: String = line_chars[line_start..].iter().collect();
                    let char_count = remaining + if has_trailing_newline { 1 } else { 0 };
                    result.push(PromptLine {
                        text,
                        start_char_idx: start_idx + line_start,
                        char_count,
                    });
                    break;
                } else {
                    let search_end = line_start + width;
                    let mut split_at = None;
                    for pos in (line_start + 1..=search_end).rev() {
                        if line_chars[pos] == ' ' {
                            split_at = Some(pos);
                            break;
                        }
                    }

                    if let Some(space_pos) = split_at {
                        let text: String = line_chars[line_start..space_pos].iter().collect();
                        let char_count = space_pos - line_start + 1;
                        result.push(PromptLine {
                            text,
                            start_char_idx: start_idx + line_start,
                            char_count,
                        });
                        i = space_pos + 1;
                    } else {
                        let text: String =
                            line_chars[line_start..line_start + width].iter().collect();
                        result.push(PromptLine {
                            text,
                            start_char_idx: start_idx + line_start,
                            char_count: width,
                        });
                        i = line_start + width;
                    }
                }
            }
        }

        if result.is_empty() {
            result.push(PromptLine {
                text: String::new(),
                start_char_idx: 0,
                char_count: 0,
            });
        }

        result
    }

    /// Converts the current `prompt_cursor` (character index) into `(line_idx, col_idx)` in wrapped coordinates.
    pub fn cursor_to_line_col(&self, width: usize) -> (usize, usize) {
        let lines = self.compute_wrapped_prompt_lines(width);
        if lines.is_empty() {
            return (0, 0);
        }
        let total_chars = self.prompt_input.chars().count();
        let cursor = self.prompt_cursor.min(total_chars);

        for (line_idx, line) in lines.iter().enumerate() {
            if cursor >= line.start_char_idx && cursor < line.start_char_idx + line.char_count {
                let col = cursor - line.start_char_idx;
                let max_col = line.text.chars().count();
                return (line_idx, col.min(max_col));
            }
        }

        let last_idx = lines.len() - 1;
        let col = cursor.saturating_sub(lines[last_idx].start_char_idx);
        let max_col = lines[last_idx].text.chars().count();
        (last_idx, col.min(max_col))
    }

    /// Converts wrapped coordinates `(line_idx, col_idx)` back into a `prompt_cursor` character index.
    pub fn line_col_to_cursor(&self, line_idx: usize, col_idx: usize, width: usize) -> usize {
        let lines = self.compute_wrapped_prompt_lines(width);
        if lines.is_empty() {
            return 0;
        }
        let line_idx = line_idx.min(lines.len() - 1);
        let line = &lines[line_idx];
        let line_len = line.text.chars().count();
        let col = col_idx.min(line_len);
        line.start_char_idx + col
    }

    /// Synchronizes `prompt_scroll_offset` so that the line containing `prompt_cursor` is visible.
    pub fn sync_prompt_scroll(&mut self, visible_lines: usize, width: usize) {
        let lines = self.compute_wrapped_prompt_lines(width);
        if self.prompt_scroll_offset >= lines.len() {
            self.prompt_scroll_offset = lines.len().saturating_sub(1);
        }
        let (line_idx, _) = self.cursor_to_line_col(width);
        if line_idx < self.prompt_scroll_offset {
            self.prompt_scroll_offset = line_idx;
        } else if line_idx >= self.prompt_scroll_offset + visible_lines {
            self.prompt_scroll_offset = line_idx.saturating_sub(visible_lines.saturating_sub(1));
        }
    }

    /// Inserts a single character into `prompt_input` at `prompt_cursor`.
    pub fn insert_char_prompt(&mut self, c: char) {
        let mut chars: Vec<char> = self.prompt_input.chars().collect();
        let idx = self.prompt_cursor.min(chars.len());
        chars.insert(idx, c);
        self.prompt_input = chars.into_iter().collect();
        self.prompt_cursor = idx + 1;
        self.sync_prompt_scroll(PROMPT_VISIBLE_LINES, self.prompt_width());
    }

    /// Inserts a string into `prompt_input` at `prompt_cursor`, normalizing `\r\n` and `\r` to `\n` and stripping unwanted control characters.
    pub fn insert_str_prompt(&mut self, s: &str) {
        let normalized = s.replace("\r\n", "\n").replace('\r', "\n");
        let mut cleaned = String::new();
        for c in normalized.chars() {
            if c == '\n' || c == '\t' || !c.is_control() {
                cleaned.push(c);
            }
        }
        if cleaned.is_empty() {
            return;
        }
        let mut chars: Vec<char> = self.prompt_input.chars().collect();
        let idx = self.prompt_cursor.min(chars.len());
        let insert_chars: Vec<char> = cleaned.chars().collect();
        let count = insert_chars.len();
        chars.splice(idx..idx, insert_chars);
        self.prompt_input = chars.into_iter().collect();
        self.prompt_cursor = idx + count;
        self.sync_prompt_scroll(PROMPT_VISIBLE_LINES, self.prompt_width());
    }

    /// Inserts a newline character (`\n`) into `prompt_input` at `prompt_cursor`.
    pub fn insert_newline_prompt(&mut self) {
        self.insert_char_prompt('\n');
    }

    /// Deletes the character before `prompt_cursor` (Backspace).
    pub fn delete_prompt_char_before_cursor(&mut self) {
        if self.prompt_cursor > 0 {
            let mut chars: Vec<char> = self.prompt_input.chars().collect();
            let idx = self.prompt_cursor.min(chars.len());
            if idx > 0 {
                chars.remove(idx - 1);
                self.prompt_input = chars.into_iter().collect();
                self.prompt_cursor = idx - 1;
                self.sync_prompt_scroll(PROMPT_VISIBLE_LINES, self.prompt_width());
            }
        }
    }

    /// Deletes the character at `prompt_cursor` (Delete).
    pub fn delete_prompt_char_at_cursor(&mut self) {
        let mut chars: Vec<char> = self.prompt_input.chars().collect();
        let idx = self.prompt_cursor;
        if idx < chars.len() {
            chars.remove(idx);
            self.prompt_input = chars.into_iter().collect();
            self.sync_prompt_scroll(PROMPT_VISIBLE_LINES, self.prompt_width());
        }
    }

    /// Moves `prompt_cursor` left by one character.
    pub fn move_prompt_cursor_left(&mut self) {
        self.prompt_cursor = self.prompt_cursor.saturating_sub(1);
        self.sync_prompt_scroll(PROMPT_VISIBLE_LINES, self.prompt_width());
    }

    /// Moves `prompt_cursor` right by one character.
    pub fn move_prompt_cursor_right(&mut self) {
        let total = self.prompt_input.chars().count();
        if self.prompt_cursor < total {
            self.prompt_cursor += 1;
        }
        self.sync_prompt_scroll(PROMPT_VISIBLE_LINES, self.prompt_width());
    }

    /// Moves `prompt_cursor` up one line in the wrapped prompt. Returns `true` if moved, `false` if already on the top line.
    pub fn move_prompt_cursor_up(&mut self) -> bool {
        self.move_prompt_cursor_up_with_width(self.prompt_width())
    }

    /// Moves `prompt_cursor` up one line using the specified wrapping width.
    pub fn move_prompt_cursor_up_with_width(&mut self, width: usize) -> bool {
        let (line_idx, col_idx) = self.cursor_to_line_col(width);
        if line_idx == 0 {
            false
        } else {
            self.prompt_cursor = self.line_col_to_cursor(line_idx - 1, col_idx, width);
            self.sync_prompt_scroll(PROMPT_VISIBLE_LINES, width);
            true
        }
    }

    /// Moves `prompt_cursor` down one line in the wrapped prompt. Returns `true` if moved, `false` if already on the bottom line.
    pub fn move_prompt_cursor_down(&mut self) -> bool {
        self.move_prompt_cursor_down_with_width(self.prompt_width())
    }

    /// Moves `prompt_cursor` down one line using the specified wrapping width.
    pub fn move_prompt_cursor_down_with_width(&mut self, width: usize) -> bool {
        let (line_idx, col_idx) = self.cursor_to_line_col(width);
        let lines = self.compute_wrapped_prompt_lines(width);
        if line_idx + 1 >= lines.len() {
            false
        } else {
            self.prompt_cursor = self.line_col_to_cursor(line_idx + 1, col_idx, width);
            self.sync_prompt_scroll(PROMPT_VISIBLE_LINES, width);
            true
        }
    }

    /// Moves `prompt_cursor` to the start of the current wrapped line.
    pub fn move_prompt_cursor_home(&mut self) {
        let (line_idx, _) = self.cursor_to_line_col(self.prompt_width());
        self.prompt_cursor = self.line_col_to_cursor(line_idx, 0, self.prompt_width());
        self.sync_prompt_scroll(PROMPT_VISIBLE_LINES, self.prompt_width());
    }

    /// Moves `prompt_cursor` to the end of the current wrapped line.
    pub fn move_prompt_cursor_end(&mut self) {
        let lines = self.compute_wrapped_prompt_lines(self.prompt_width());
        let (line_idx, _) = self.cursor_to_line_col(self.prompt_width());
        if line_idx < lines.len() {
            let line_len = lines[line_idx].text.chars().count();
            self.prompt_cursor = self.line_col_to_cursor(line_idx, line_len, self.prompt_width());
        } else {
            self.prompt_cursor = self.prompt_input.chars().count();
        }
        self.sync_prompt_scroll(PROMPT_VISIBLE_LINES, self.prompt_width());
    }

    /// Deletes the word before `prompt_cursor` (Ctrl+W).
    pub fn delete_prompt_word_before_cursor(&mut self) {
        let chars: Vec<char> = self.prompt_input.chars().collect();
        let idx = self.prompt_cursor.min(chars.len());
        if idx == 0 {
            return;
        }
        let mut pos = idx;
        while pos > 0 && chars[pos - 1].is_whitespace() {
            pos -= 1;
        }
        while pos > 0 && !chars[pos - 1].is_whitespace() {
            pos -= 1;
        }
        let mut new_chars = Vec::new();
        new_chars.extend_from_slice(&chars[..pos]);
        new_chars.extend_from_slice(&chars[idx..]);
        self.prompt_input = new_chars.into_iter().collect();
        self.prompt_cursor = pos;
        self.sync_prompt_scroll(PROMPT_VISIBLE_LINES, self.prompt_width());
    }

    /// Kills text from `prompt_cursor` to the end of the logical line (Ctrl+K).
    pub fn kill_to_end_of_prompt_line(&mut self) {
        let chars: Vec<char> = self.prompt_input.chars().collect();
        let idx = self.prompt_cursor.min(chars.len());
        if idx >= chars.len() {
            return;
        }
        if chars[idx] == '\n' {
            let mut new_chars = Vec::new();
            new_chars.extend_from_slice(&chars[..idx]);
            new_chars.extend_from_slice(&chars[idx + 1..]);
            self.prompt_input = new_chars.into_iter().collect();
        } else {
            let next_newline = chars[idx..]
                .iter()
                .position(|&c| c == '\n')
                .map(|p| idx + p)
                .unwrap_or(chars.len());
            let mut new_chars = Vec::new();
            new_chars.extend_from_slice(&chars[..idx]);
            new_chars.extend_from_slice(&chars[next_newline..]);
            self.prompt_input = new_chars.into_iter().collect();
        }
        self.sync_prompt_scroll(PROMPT_VISIBLE_LINES, self.prompt_width());
    }

    /// Clears text before `prompt_cursor` on the current logical line, or the entire line/prompt (Ctrl+U).
    pub fn clear_prompt_line_or_all(&mut self) {
        let chars: Vec<char> = self.prompt_input.chars().collect();
        let idx = self.prompt_cursor.min(chars.len());
        let start_pos = chars[..idx]
            .iter()
            .rposition(|&c| c == '\n')
            .map(|p| p + 1)
            .unwrap_or(0);
        if start_pos < idx {
            let mut new_chars = Vec::new();
            new_chars.extend_from_slice(&chars[..start_pos]);
            new_chars.extend_from_slice(&chars[idx..]);
            self.prompt_input = new_chars.into_iter().collect();
            self.prompt_cursor = start_pos;
        } else if start_pos == 0 && idx == 0 {
            self.prompt_input.clear();
            self.prompt_cursor = 0;
        } else if start_pos > 0 {
            let mut new_chars = Vec::new();
            new_chars.extend_from_slice(&chars[..start_pos - 1]);
            new_chars.extend_from_slice(&chars[idx..]);
            self.prompt_input = new_chars.into_iter().collect();
            self.prompt_cursor = start_pos - 1;
        }
        self.sync_prompt_scroll(PROMPT_VISIBLE_LINES, self.prompt_width());
    }

    /// Returns the current project name string slice.
    pub fn project_name(&self) -> &str {
        self.project_name_input.value()
    }

    /// Sets the project name value, moves the cursor to the end, and triggers revalidation.
    pub fn set_project_name(&mut self, name: impl Into<String>) {
        self.project_name_input.set_value(name);
        self.project_name_input.move_cursor_end();
        self.project_name_cursor = self.project_name_input.cursor;
        self.revalidate();
    }

    fn sync_project_name_cursor(&mut self) {
        self.project_name_input.cursor = self
            .project_name_cursor
            .min(self.project_name_input.chars().count());
    }

    /// Inserts a character into the project name input at the current cursor offset.
    pub fn insert_project_name_char(&mut self, c: char) {
        self.sync_project_name_cursor();
        self.project_name_input.insert_char(c);
        self.project_name_cursor = self.project_name_input.cursor;
        self.revalidate();
    }

    /// Deletes the character before the project name cursor (Backspace).
    /// If the project name is empty, returns to template selection step.
    pub fn delete_project_name_char_backward(&mut self) {
        self.sync_project_name_cursor();
        if self.project_name_input.is_empty() {
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
            return;
        }
        if self.project_name_input.delete_char_backward() {
            self.project_name_cursor = self.project_name_input.cursor;
            self.revalidate();
        }
    }

    /// Deletes the character under the project name cursor (Delete).
    pub fn delete_project_name_char_forward(&mut self) {
        self.sync_project_name_cursor();
        if self.project_name_input.delete_char_forward() {
            self.project_name_cursor = self.project_name_input.cursor;
            self.revalidate();
        }
    }

    /// Deletes characters from the project name cursor to the beginning of the line (Ctrl+U).
    pub fn delete_project_name_to_start(&mut self) {
        self.sync_project_name_cursor();
        if self.project_name_input.delete_to_start() {
            self.project_name_cursor = self.project_name_input.cursor;
            self.revalidate();
        }
    }

    /// Deletes characters from the project name cursor to the end of the line (Ctrl+K).
    pub fn delete_project_name_to_end(&mut self) {
        self.sync_project_name_cursor();
        if self.project_name_input.delete_to_end() {
            self.project_name_cursor = self.project_name_input.cursor;
            self.revalidate();
        }
    }

    /// Deletes the word immediately preceding the project name cursor (Ctrl+W).
    pub fn delete_project_name_word_backward(&mut self) {
        self.sync_project_name_cursor();
        if self.project_name_input.delete_word_backward() {
            self.project_name_cursor = self.project_name_input.cursor;
            self.revalidate();
        }
    }

    /// Moves the project name cursor backward one word (Ctrl+Left / Alt+B).
    pub fn move_project_name_word_backward(&mut self) {
        self.sync_project_name_cursor();
        self.project_name_input.move_word_backward();
        self.project_name_cursor = self.project_name_input.cursor;
    }

    /// Moves the project name cursor forward one word (Ctrl+Right / Alt+F).
    pub fn move_project_name_word_forward(&mut self) {
        self.sync_project_name_cursor();
        self.project_name_input.move_word_forward();
        self.project_name_cursor = self.project_name_input.cursor;
    }

    /// Moves the project name cursor left by one character.
    pub fn move_project_name_cursor_left(&mut self) {
        self.sync_project_name_cursor();
        self.project_name_input.move_cursor_left();
        self.project_name_cursor = self.project_name_input.cursor;
    }

    /// Moves the project name cursor right by one character.
    pub fn move_project_name_cursor_right(&mut self) {
        self.sync_project_name_cursor();
        self.project_name_input.move_cursor_right();
        self.project_name_cursor = self.project_name_input.cursor;
    }

    /// Moves the project name cursor to the start of the line (Home / Ctrl+A).
    pub fn move_project_name_cursor_home(&mut self) {
        self.project_name_input.move_cursor_home();
        self.project_name_cursor = self.project_name_input.cursor;
    }

    /// Moves the project name cursor to the end of the line (End / Ctrl+E).
    pub fn move_project_name_cursor_end(&mut self) {
        self.project_name_input.move_cursor_end();
        self.project_name_cursor = self.project_name_input.cursor;
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
                let cursor_x =
                    inner.x + prefix_len + self.template_search_query.chars().count() as u16;
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
                    let total_chars = self.project_name_input.chars().count();
                    let cursor_offset = self.project_name_cursor.min(total_chars) as u16;
                    let cursor_x = inner.x + prefix_len + cursor_offset;
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
                    let prompt_text_width = (inner.width.saturating_sub(18) as usize).max(10);
                    let (line_idx, col_idx) = self.cursor_to_line_col(prompt_text_width);
                    let scroll_offset = self.prompt_scroll_offset.min(line_idx);
                    let visible_line = line_idx.saturating_sub(scroll_offset);
                    if visible_line < PROMPT_VISIBLE_LINES {
                        let cursor_x = inner.x + 18 + col_idx as u16;
                        let cursor_y = inner.y + 5 + visible_line as u16;
                        if cursor_x < inner.x + inner.width && cursor_y < inner.y + inner.height {
                            Some(Position::new(cursor_x, cursor_y))
                        } else {
                            Some(Position::new(
                                cursor_x.min(inner.x + inner.width.saturating_sub(1)),
                                cursor_y.min(inner.y + inner.height.saturating_sub(1)),
                            ))
                        }
                    } else {
                        None
                    }
                }
                _ => None,
            },
        }
    }

    /// Handles keyboard events and returns any triggered `CreationModalAction`.
    pub fn handle_key(&mut self, key: KeyEvent) -> CreationModalAction {
        self.poll_validation();

        let has_shift = key.modifiers.contains(KeyModifiers::SHIFT);
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
                        let target_idx =
                            matching[self.selected_template_idx.min(matching.len() - 1)];
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
            CreationStep::ConfigureDetails => {
                if self.focus == CreationFocus::Prompt {
                    match key.code {
                        KeyCode::Esc => self.handle_back_or_cancel(),
                        KeyCode::Tab => {
                            self.next_focus();
                            CreationModalAction::None
                        }
                        KeyCode::BackTab => {
                            self.prev_focus();
                            CreationModalAction::None
                        }
                        KeyCode::Up => {
                            if !self.move_prompt_cursor_up() {
                                self.prev_focus();
                            }
                            CreationModalAction::None
                        }
                        KeyCode::Down => {
                            if !self.move_prompt_cursor_down() {
                                self.next_focus();
                            }
                            CreationModalAction::None
                        }
                        KeyCode::Left => {
                            self.move_prompt_cursor_left();
                            CreationModalAction::None
                        }
                        KeyCode::Right => {
                            self.move_prompt_cursor_right();
                            CreationModalAction::None
                        }
                        KeyCode::Home => {
                            self.move_prompt_cursor_home();
                            CreationModalAction::None
                        }
                        KeyCode::End => {
                            self.move_prompt_cursor_end();
                            CreationModalAction::None
                        }
                        KeyCode::Backspace => {
                            self.delete_prompt_char_before_cursor();
                            CreationModalAction::None
                        }
                        KeyCode::Delete => {
                            self.delete_prompt_char_at_cursor();
                            CreationModalAction::None
                        }
                        KeyCode::Enter | KeyCode::Char('\n') | KeyCode::Char('\r')
                            if has_shift || has_alt =>
                        {
                            self.insert_newline_prompt();
                            CreationModalAction::None
                        }
                        KeyCode::Enter | KeyCode::Char('\n') | KeyCode::Char('\r')
                            if !has_shift && !has_alt =>
                        {
                            self.try_submit()
                        }
                        KeyCode::Char(c) if has_ctrl => {
                            match c {
                                'a' | 'A' => self.move_prompt_cursor_home(),
                                'e' | 'E' => self.move_prompt_cursor_end(),
                                'w' | 'W' => self.delete_prompt_word_before_cursor(),
                                'k' | 'K' => self.kill_to_end_of_prompt_line(),
                                'u' | 'U' => self.clear_prompt_line_or_all(),
                                _ => {}
                            }
                            CreationModalAction::None
                        }
                        KeyCode::Char(c) if !has_ctrl && !has_alt => {
                            self.insert_char_prompt(c);
                            CreationModalAction::None
                        }
                        _ => CreationModalAction::None,
                    }
                } else if self.focus == CreationFocus::ProjectName {
                    if (key.code == KeyCode::Left && (has_ctrl || has_alt))
                        || ((key.code == KeyCode::Char('b') || key.code == KeyCode::Char('B'))
                            && has_alt)
                    {
                        self.move_project_name_word_backward();
                        return CreationModalAction::None;
                    }

                    if (key.code == KeyCode::Right && (has_ctrl || has_alt))
                        || ((key.code == KeyCode::Char('f') || key.code == KeyCode::Char('F'))
                            && has_alt)
                    {
                        self.move_project_name_word_forward();
                        return CreationModalAction::None;
                    }

                    match key.code {
                        KeyCode::Esc => self.handle_back_or_cancel(),
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
                            self.move_project_name_cursor_left();
                            CreationModalAction::None
                        }
                        KeyCode::Right => {
                            self.move_project_name_cursor_right();
                            CreationModalAction::None
                        }
                        KeyCode::Home => {
                            self.move_project_name_cursor_home();
                            CreationModalAction::None
                        }
                        KeyCode::End => {
                            self.move_project_name_cursor_end();
                            CreationModalAction::None
                        }
                        KeyCode::Backspace => {
                            self.delete_project_name_char_backward();
                            CreationModalAction::None
                        }
                        KeyCode::Delete => {
                            self.delete_project_name_char_forward();
                            CreationModalAction::None
                        }
                        KeyCode::Enter => self.try_submit(),
                        KeyCode::Char(c) if has_ctrl => {
                            match c {
                                'a' | 'A' => self.move_project_name_cursor_home(),
                                'e' | 'E' => self.move_project_name_cursor_end(),
                                'u' | 'U' => self.delete_project_name_to_start(),
                                'k' | 'K' => self.delete_project_name_to_end(),
                                'w' | 'W' => self.delete_project_name_word_backward(),
                                _ => {}
                            }
                            CreationModalAction::None
                        }
                        KeyCode::Char(c) if !has_ctrl && !has_alt => {
                            self.insert_project_name_char(c);
                            CreationModalAction::None
                        }
                        _ => CreationModalAction::None,
                    }
                } else {
                    match key.code {
                        KeyCode::Esc => self.handle_back_or_cancel(),
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
                            if self.focus == CreationFocus::Agent {
                                if c == 'h' || c == 'k' {
                                    self.cycle_agent_prev();
                                } else if c == 'l' || c == 'j' {
                                    self.cycle_agent_next();
                                }
                            }
                            CreationModalAction::None
                        }
                        KeyCode::Enter => self.try_submit(),
                        _ => CreationModalAction::None,
                    }
                }
            }
        }
    }

    /// Attempts to validate and submit the project creation modal.
    pub fn try_submit(&mut self) -> CreationModalAction {
        self.poll_validation();
        if self.validation.is_valid {
            let tpl_name = self.selected_template().unwrap_or("");
            let builder = ProjectBuilder::new(
                self.config.clone(),
                tpl_name,
                self.project_name_input.value(),
            )
            .with_cwd(&self.cwd)
            .with_agent(self.selected_agent());
            self.validation = builder.validate_detailed();
        }
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

    /// Handles backtracking to template selection.
    pub fn handle_back_or_cancel(&mut self) -> CreationModalAction {
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
                    self.sync_project_name_cursor();
                    if self.project_name_input.handle_paste(text) {
                        self.project_name_cursor = self.project_name_input.cursor;
                        self.revalidate();
                    }
                }
                CreationFocus::Prompt => {
                    self.insert_str_prompt(text);
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
                        } else if self.focus == CreationFocus::Prompt {
                            if !self.move_prompt_cursor_up() {
                                self.prev_focus();
                            }
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
                        } else if self.focus == CreationFocus::Prompt {
                            if !self.move_prompt_cursor_down() {
                                self.next_focus();
                            }
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
        buf.set_line(
            filter_area.x,
            filter_area.y,
            &filter_line,
            filter_area.width,
        );

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
            if self.templates.is_empty() {
                left_lines.push(Line::from(Span::styled(
                    "  (No templates)",
                    Style::default().fg(theme.input_placeholder),
                )));
            } else {
                left_lines.push(Line::from(Span::styled(
                    "  (No matches)",
                    Style::default().fg(theme.input_placeholder),
                )));
            }
        } else {
            let max_display = (left_area.height as usize).saturating_sub(2);
            let start_idx = if self.selected_template_idx >= max_display {
                self.selected_template_idx
                    .saturating_sub(max_display.saturating_sub(1))
            } else {
                0
            };

            for (match_i, &tpl_idx) in matching
                .iter()
                .enumerate()
                .skip(start_idx)
                .take(max_display)
            {
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

                right_lines.push(Line::from(vec![Span::styled(
                    format!("Repositories ({}):", tpl.repos.len()),
                    Style::default()
                        .fg(theme.modal_label)
                        .add_modifier(Modifier::BOLD),
                )]));

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
                        right_lines.push(Line::from(vec![Span::styled(
                            "Custom Instructions:",
                            Style::default()
                                .fg(theme.modal_label)
                                .add_modifier(Modifier::BOLD),
                        )]));
                        for line in trimmed.lines().take(2) {
                            right_lines.push(Line::from(Span::styled(
                                format!("  {line}"),
                                Style::default().fg(theme.input_placeholder),
                            )));
                        }
                    }
                }
            }
        } else if self.templates.is_empty() {
            right_lines.push(Line::from(Span::styled(
                "No templates found in config.toml.",
                Style::default().fg(theme.input_placeholder),
            )));
            right_lines.push(Line::from(Span::styled(
                "Run 'botai edit' to configure templates.",
                Style::default().fg(theme.input_placeholder),
            )));
        } else {
            right_lines.push(Line::from(Span::styled(
                "No template selected",
                Style::default().fg(theme.input_placeholder),
            )));
        }
        let right_par = Paragraph::new(right_lines).wrap(Wrap { trim: false });
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
        let footer_text = if self.templates.is_empty() {
            " [Esc] Cancel "
        } else {
            " [↑/↓/j/k] Select | [Enter] Next (Configure) | [Esc] Cancel "
        };
        let footer_line = Line::from(Span::styled(
            footer_text,
            Style::default()
                .fg(theme.key_hint)
                .add_modifier(Modifier::BOLD),
        ));
        buf.set_line(
            footer_area.x,
            footer_area.y,
            &footer_line,
            footer_area.width,
        );
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
        let (repo_preview_str, tpl_desc_str) = if let Some(tpl) = self.config.get_template(tpl_name)
        {
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
                if repo_names.is_empty() {
                    "none"
                } else {
                    &repo_names
                }
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
            Span::styled(
                self.project_name_input.value(),
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

        // 4. Initial Prompt (multi-line box with visible lines)
        let prompt_text_width = (inner.width.saturating_sub(18) as usize).max(10);
        let prompt_lines = self.compute_wrapped_prompt_lines(prompt_text_width);
        let visible_count = PROMPT_VISIBLE_LINES;
        let scroll_offset = self
            .prompt_scroll_offset
            .min(prompt_lines.len().saturating_sub(1));
        let is_empty_and_unfocused = self.prompt_input.is_empty() && !prompt_focused;

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
                    Span::styled(
                        format!("{prompt_prefix}Initial Prompt: "),
                        prompt_label_style,
                    ),
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

        // 5. Validation Status Banner
        let banner_line = if let Some(msg) = self.validation.primary_message() {
            if self.validation.is_valid {
                Line::from(vec![
                    Span::styled(
                        " ✓ ",
                        Style::default()
                            .fg(theme.status_done)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(msg, Style::default().fg(theme.status_done)),
                ])
            } else if self.project_name_input.trim().is_empty() {
                Line::from(vec![
                    Span::styled(" ℹ ", Style::default().fg(theme.modal_label)),
                    Span::styled(msg, Style::default().fg(theme.input_placeholder)),
                ])
            } else {
                Line::from(vec![
                    Span::styled(
                        " ✗ ",
                        Style::default()
                            .fg(theme.status_error)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(msg, Style::default().fg(theme.status_error)),
                ])
            }
        } else if self.validation.is_valid {
            Line::from(vec![
                Span::styled(
                    " ✓ ",
                    Style::default()
                        .fg(theme.status_done)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    "Ready to initialize project worktrees and spawn agent",
                    Style::default().fg(theme.status_done),
                ),
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

        // Rows 5..=7: Prompt
        lines.extend(prompt_row_spans);

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
    fn test_modal_full_workflow_and_submission() {
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

        // Modal starts in SelectTemplate even with a single template configured
        assert_eq!(modal.step, CreationStep::SelectTemplate);
        assert_eq!(modal.selected_template(), Some("ecommerce"));
        assert_eq!(modal.selected_agent(), "agy");

        // Advance to ConfigureDetails via Enter
        let act = modal.handle_key(key(KeyCode::Enter));
        assert_eq!(act, CreationModalAction::None);
        assert_eq!(modal.step, CreationStep::ConfigureDetails);
        assert_eq!(modal.focus, CreationFocus::ProjectName);
        assert!(!modal.validation.is_valid); // Name is empty

        // Esc backtracks to SelectTemplate, and Enter returns to ConfigureDetails
        let act = modal.handle_key(key(KeyCode::Esc));
        assert_eq!(act, CreationModalAction::None);
        assert_eq!(modal.step, CreationStep::SelectTemplate);
        let act = modal.handle_key(key(KeyCode::Enter));
        assert_eq!(act, CreationModalAction::None);
        assert_eq!(modal.step, CreationStep::ConfigureDetails);

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
        modal.step = CreationStep::ConfigureDetails;

        // Type colliding name
        for c in "existing-branch".chars() {
            modal.handle_key(key(KeyCode::Char(c)));
        }

        // Wait for background branch validation to complete and poll
        let start = std::time::Instant::now();
        while modal.validation.is_valid && start.elapsed() < std::time::Duration::from_secs(3) {
            modal.poll_validation();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        assert!(!modal.validation.is_valid);
        assert!(modal
            .validation
            .primary_message()
            .unwrap()
            .contains("already exists in repository"));

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

        let start = std::time::Instant::now();
        while start.elapsed() < std::time::Duration::from_millis(100) {
            modal.poll_validation();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        assert!(modal.validation.is_valid);
        let act = modal.handle_key(key(KeyCode::Enter));
        assert!(
            matches!(act, CreationModalAction::Submit { project, .. } if project == "clean-branch")
        );
    }

    #[test]
    fn test_creation_modal_background_validation_generation_and_debouncing() {
        let temp_dir = TempDir::new().unwrap();
        let repo_dir = temp_dir.path().join("service_alpha");
        init_mock_repo(&repo_dir);

        let _ = std::process::Command::new("git")
            .args(["branch", "collision-target"])
            .current_dir(&repo_dir)
            .status();

        let toml_str = format!(
            r#"
            default_agent = "claude"
            repos_dir = "{}"
            [templates.alpha_tpl]
            repos = ["{}"]
            "#,
            temp_dir.path().display(),
            repo_dir.display()
        );
        let config = Config::load_from_str(&toml_str).unwrap();
        let mut modal = ProjectCreationModal::new(&config, temp_dir.path(), Some("alpha_tpl"));
        modal.step = CreationStep::ConfigureDetails;

        // Fast validation immediately reflects non-empty input
        modal.set_project_name("collision-target");
        let gen1 = modal.validation_generation;

        // Immediate state before background thread finishes is fast-valid
        assert!(modal.validation.is_valid);

        // Simulate typing more characters before gen1 finishes
        modal.set_project_name("collision-target-extra");
        let gen2 = modal.validation_generation;
        assert_ne!(gen1, gen2);

        // Let background threads run
        let start = std::time::Instant::now();
        while start.elapsed() < std::time::Duration::from_millis(200) {
            modal.poll_validation();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        // gen1 result (collision on "collision-target") was discarded because active generation is gen2 ("collision-target-extra" which is clean)
        assert!(modal.validation.is_valid);
        assert!(modal.validation.errors.is_empty());
    }

    #[test]
    fn test_creation_modal_cursor_position_calculation() {
        let config =
            Config::load_from_str("default_agent = \"claude\"\nrepos_dir = \".repos\"\n").unwrap();
        let temp_dir = TempDir::new().unwrap();
        let mut modal = ProjectCreationModal::new(&config, temp_dir.path(), None);
        modal.step = CreationStep::ConfigureDetails;

        let area = Rect::new(0, 0, 100, 30);
        modal.focus = CreationFocus::ProjectName;
        modal.set_project_name("my-proj");

        let pos = modal.cursor_position(area);
        assert_eq!(pos, Some(Position::new(36, 10)));

        modal.focus = CreationFocus::Prompt;
        modal.prompt_input = "fix bug".to_string();
        modal.prompt_cursor = 7;
        let pos = modal.cursor_position(area);
        assert_eq!(pos, Some(Position::new(38, 12)));

        modal.prompt_cursor = 0;
        let pos = modal.cursor_position(area);
        assert_eq!(pos, Some(Position::new(31, 12)));

        modal.focus = CreationFocus::Template;
        assert_eq!(modal.cursor_position(area), None);

        modal.focus = CreationFocus::Agent;
        assert_eq!(modal.cursor_position(area), None);
    }

    #[test]
    fn test_prompt_helper_methods_cursor_editing() {
        let config =
            Config::load_from_str("default_agent = \"claude\"\nrepos_dir = \".repos\"\n").unwrap();
        let temp_dir = TempDir::new().unwrap();
        let mut modal = ProjectCreationModal::new(&config, temp_dir.path(), None);

        // 1. Insert characters
        modal.insert_char_prompt('h');
        modal.insert_char_prompt('i');
        assert_eq!(modal.prompt_input, "hi");
        assert_eq!(modal.prompt_cursor, 2);

        // 2. Move cursor and insert in middle
        modal.move_prompt_cursor_left();
        assert_eq!(modal.prompt_cursor, 1);
        modal.insert_char_prompt('e');
        assert_eq!(modal.prompt_input, "hei");
        assert_eq!(modal.prompt_cursor, 2);

        // 3. Insert string with \r\n normalization and control characters
        modal.insert_str_prompt("llo\r\nworld\x07!");
        assert_eq!(modal.prompt_input, "hello\nworld!i");
        assert_eq!(modal.prompt_cursor, 12);

        // 4. Backspace before cursor (deletes '!')
        modal.delete_prompt_char_before_cursor();
        assert_eq!(modal.prompt_input, "hello\nworldi");
        assert_eq!(modal.prompt_cursor, 11);

        // 5. Delete at cursor
        modal.move_prompt_cursor_home();
        assert_eq!(modal.prompt_cursor, 6); // start of 2nd line
        modal.delete_prompt_char_at_cursor(); // deletes 'w'
        assert_eq!(modal.prompt_input, "hello\norldi");
        assert_eq!(modal.prompt_cursor, 6);

        // 6. Home and End
        modal.move_prompt_cursor_end();
        assert_eq!(modal.prompt_cursor, 11);
        modal.move_prompt_cursor_home();
        assert_eq!(modal.prompt_cursor, 6);

        // 7. Word deletion (Ctrl+W)
        modal.move_prompt_cursor_end();
        modal.insert_str_prompt(" test case");
        assert_eq!(modal.prompt_input, "hello\norldi test case");
        modal.delete_prompt_word_before_cursor();
        assert_eq!(modal.prompt_input, "hello\norldi test ");
        modal.delete_prompt_word_before_cursor();
        assert_eq!(modal.prompt_input, "hello\norldi ");

        // 8. Kill to end of line (Ctrl+K)
        modal.prompt_cursor = 6;
        modal.kill_to_end_of_prompt_line();
        assert_eq!(modal.prompt_input, "hello\n");
        modal.prompt_cursor = 5; // positioned at '\n'
        modal.kill_to_end_of_prompt_line(); // kill newline
        assert_eq!(modal.prompt_input, "hello");

        // 9. Clear line / prompt (Ctrl+U)
        modal.insert_str_prompt("\nsecond line");
        modal.clear_prompt_line_or_all();
        assert_eq!(modal.prompt_input, "hello\n");
        modal.clear_prompt_line_or_all();
        assert_eq!(modal.prompt_input, "hello");
        modal.move_prompt_cursor_home();
        modal.clear_prompt_line_or_all();
        assert_eq!(modal.prompt_input, "");
        assert_eq!(modal.prompt_cursor, 0);
    }

    #[test]
    fn test_prompt_word_wrapping_and_coordinate_mapping() {
        let config =
            Config::load_from_str("default_agent = \"claude\"\nrepos_dir = \".repos\"\n").unwrap();
        let temp_dir = TempDir::new().unwrap();
        let mut modal = ProjectCreationModal::new(&config, temp_dir.path(), None);

        // 1. Empty input
        let lines = modal.compute_wrapped_prompt_lines(20);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text, "");
        assert_eq!(modal.cursor_to_line_col(20), (0, 0));
        assert_eq!(modal.line_col_to_cursor(0, 0, 20), 0);

        // 2. Explicit newlines
        modal.prompt_input = "first\nsecond\nthird".to_string();
        modal.prompt_cursor = 8; // 'e' in second
        let lines = modal.compute_wrapped_prompt_lines(20);
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].text, "first");
        assert_eq!(lines[1].text, "second");
        assert_eq!(lines[2].text, "third");
        assert_eq!(modal.cursor_to_line_col(20), (1, 2));
        assert_eq!(modal.line_col_to_cursor(1, 2, 20), 8);

        // 3. Word wrapping with spaces
        modal.prompt_input = "the quick brown fox jumps".to_string();
        let lines = modal.compute_wrapped_prompt_lines(10);
        // "the quick" (9), "brown fox" (9), "jumps" (5)
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].text, "the quick");
        assert_eq!(lines[1].text, "brown fox");
        assert_eq!(lines[2].text, "jumps");

        // 4. Hard break without spaces
        modal.prompt_input = "abcdefghijklmnopqrst".to_string();
        let lines = modal.compute_wrapped_prompt_lines(8);
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].text, "abcdefgh");
        assert_eq!(lines[1].text, "ijklmnop");
        assert_eq!(lines[2].text, "qrst");
    }

    #[test]
    fn test_prompt_vertical_cursor_navigation_and_focus_transitions() {
        let config =
            Config::load_from_str("default_agent = \"claude\"\nrepos_dir = \".repos\"\n").unwrap();
        let temp_dir = TempDir::new().unwrap();
        let mut modal = ProjectCreationModal::new(&config, temp_dir.path(), None);
        modal.step = CreationStep::ConfigureDetails;
        modal.focus = CreationFocus::Prompt;
        modal.prompt_input = "line 1\nline 2\nline 3".to_string();
        modal.prompt_cursor = 0;

        // On top line, move_prompt_cursor_up returns false
        assert!(!modal.move_prompt_cursor_up());

        // Pressing Up when on top line transitions focus to prev field (Agent)
        modal.handle_key(key(KeyCode::Up));
        assert_eq!(modal.focus, CreationFocus::Agent);

        // Down from Agent returns to Prompt
        modal.handle_key(key(KeyCode::Down));
        assert_eq!(modal.focus, CreationFocus::Prompt);

        // Down within prompt moves across lines
        let moved = modal.move_prompt_cursor_down();
        assert!(moved);
        assert_eq!(modal.cursor_to_line_col(56), (1, 0));

        let moved = modal.move_prompt_cursor_down();
        assert!(moved);
        assert_eq!(modal.cursor_to_line_col(56), (2, 0));

        // On bottom line, move_prompt_cursor_down returns false
        assert!(!modal.move_prompt_cursor_down());

        // Pressing Down when on bottom line transitions focus to next field (ProjectName)
        modal.handle_key(key(KeyCode::Down));
        assert_eq!(modal.focus, CreationFocus::ProjectName);
    }

    #[test]
    fn test_prompt_vertical_scrolling_and_scroll_offset_sync() {
        let config =
            Config::load_from_str("default_agent = \"claude\"\nrepos_dir = \".repos\"\n").unwrap();
        let temp_dir = TempDir::new().unwrap();
        let mut modal = ProjectCreationModal::new(&config, temp_dir.path(), None);
        modal.focus = CreationFocus::Prompt;
        modal.prompt_input = "1\n2\n3\n4\n5\n6".to_string();
        modal.prompt_cursor = 0;

        assert_eq!(modal.prompt_scroll_offset, 0);

        // Move down line by line
        modal.move_prompt_cursor_down(); // Line 1
        assert_eq!(modal.prompt_scroll_offset, 0);
        modal.move_prompt_cursor_down(); // Line 2
        assert_eq!(modal.prompt_scroll_offset, 0);
        modal.move_prompt_cursor_down(); // Line 3 (exceeds 3 visible lines [0, 1, 2])
        assert_eq!(modal.prompt_scroll_offset, 1);
        modal.move_prompt_cursor_down(); // Line 4
        assert_eq!(modal.prompt_scroll_offset, 2);
        modal.move_prompt_cursor_down(); // Line 5
        assert_eq!(modal.prompt_scroll_offset, 3);

        // Move up line by line
        modal.move_prompt_cursor_up(); // Line 4
        assert_eq!(modal.prompt_scroll_offset, 3);
        modal.move_prompt_cursor_up(); // Line 3
        assert_eq!(modal.prompt_scroll_offset, 3);
        modal.move_prompt_cursor_up(); // Line 2
        assert_eq!(modal.prompt_scroll_offset, 2);
        modal.move_prompt_cursor_up(); // Line 1
        assert_eq!(modal.prompt_scroll_offset, 1);
        modal.move_prompt_cursor_up(); // Line 0
        assert_eq!(modal.prompt_scroll_offset, 0);
    }

    #[test]
    fn test_prompt_shift_alt_enter_and_emacs_shortcuts() {
        let config =
            Config::load_from_str("default_agent = \"claude\"\nrepos_dir = \".repos\"\n").unwrap();
        let temp_dir = TempDir::new().unwrap();
        let mut modal = ProjectCreationModal::new(&config, temp_dir.path(), None);
        modal.step = CreationStep::ConfigureDetails;
        modal.focus = CreationFocus::Prompt;

        // Shift+Enter inserts newline without submitting
        let shift_enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT);
        let action = modal.handle_key(shift_enter);
        assert_eq!(action, CreationModalAction::None);
        assert_eq!(modal.prompt_input, "\n");
        assert_eq!(modal.prompt_cursor, 1);

        // Alt+Enter inserts newline without submitting
        let alt_enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT);
        let action = modal.handle_key(alt_enter);
        assert_eq!(action, CreationModalAction::None);
        assert_eq!(modal.prompt_input, "\n\n");
        assert_eq!(modal.prompt_cursor, 2);

        // Type text
        modal.handle_key(key(KeyCode::Char('t')));
        modal.handle_key(key(KeyCode::Char('e')));
        modal.handle_key(key(KeyCode::Char('s')));
        modal.handle_key(key(KeyCode::Char('t')));
        assert_eq!(modal.prompt_input, "\n\ntest");

        // Ctrl+A (Home)
        modal.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL));
        assert_eq!(modal.prompt_cursor, 2);

        // Ctrl+E (End)
        modal.handle_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::CONTROL));
        assert_eq!(modal.prompt_cursor, 6);

        // Ctrl+W (Delete word)
        modal.handle_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL));
        assert_eq!(modal.prompt_input, "\n\n");

        // Paste multiline
        modal.handle_paste("line A\r\nline B");
        assert_eq!(modal.prompt_input, "\n\nline A\nline B");
    }

    #[test]
    fn test_prompt_multiline_rendering_and_placeholder() {
        let config =
            Config::load_from_str("default_agent = \"claude\"\nrepos_dir = \".repos\"\n").unwrap();
        let temp_dir = TempDir::new().unwrap();
        let mut modal = ProjectCreationModal::new(&config, temp_dir.path(), None);
        modal.step = CreationStep::ConfigureDetails;

        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        let theme = Theme::default();

        // 1. Render empty & unfocused -> shows placeholder
        modal.focus = CreationFocus::ProjectName;
        modal.render_modal(area, &mut buf, &theme);
        let content = buf
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect::<String>();
        assert!(content.contains("<optional task instruction prompt>"));

        // 2. Render multi-line prompt
        modal.focus = CreationFocus::Prompt;
        modal.insert_str_prompt(
            "First instruction line\nSecond instruction line\nThird instruction line",
        );
        let mut buf2 = Buffer::empty(area);
        modal.render_modal(area, &mut buf2, &theme);
        let content2 = buf2
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect::<String>();
        assert!(content2.contains("First instruction line"));
        assert!(content2.contains("Second instruction line"));
        assert!(content2.contains("Third instruction line"));
    }

    #[test]
    fn test_project_name_inline_cursor_movement_and_field_isolation() {
        let config =
            Config::load_from_str("default_agent = \"claude\"\nrepos_dir = \".repos\"\n").unwrap();
        let temp_dir = TempDir::new().unwrap();
        let mut modal = ProjectCreationModal::new(&config, temp_dir.path(), None);
        modal.step = CreationStep::ConfigureDetails;
        modal.focus = CreationFocus::ProjectName;

        // Set initial project name
        modal.set_project_name("alpha-beta-gamma");
        assert_eq!(modal.project_name_cursor, 16);

        // Left arrow moves cursor left by 1 without changing focus
        modal.handle_key(key(KeyCode::Left));
        assert_eq!(modal.focus, CreationFocus::ProjectName);
        assert_eq!(modal.project_name_cursor, 15);

        // Right arrow moves cursor right by 1 without changing focus
        modal.handle_key(key(KeyCode::Right));
        assert_eq!(modal.focus, CreationFocus::ProjectName);
        assert_eq!(modal.project_name_cursor, 16);

        // Right arrow at end does not overflow
        modal.handle_key(key(KeyCode::Right));
        assert_eq!(modal.project_name_cursor, 16);

        // Home key moves cursor to 0
        modal.handle_key(key(KeyCode::Home));
        assert_eq!(modal.project_name_cursor, 0);

        // Left arrow at start does not underflow
        modal.handle_key(key(KeyCode::Left));
        assert_eq!(modal.project_name_cursor, 0);

        // End key moves cursor to end
        modal.handle_key(key(KeyCode::End));
        assert_eq!(modal.project_name_cursor, 16);

        // Ctrl+A moves to start
        modal.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL));
        assert_eq!(modal.project_name_cursor, 0);

        // Ctrl+E moves to end
        modal.handle_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::CONTROL));
        assert_eq!(modal.project_name_cursor, 16);

        // Ctrl+Left moves back one word
        modal.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::CONTROL));
        assert_eq!(modal.project_name_cursor, 11); // Start of "gamma"

        // Alt+B moves back one word
        modal.handle_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::ALT));
        assert_eq!(modal.project_name_cursor, 6); // Start of "beta"

        modal.handle_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::ALT));
        assert_eq!(modal.project_name_cursor, 0); // Start of "alpha"

        // Ctrl+Right moves forward one word
        modal.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::CONTROL));
        assert_eq!(modal.project_name_cursor, 5); // End of "alpha"

        // Alt+F moves forward one word
        modal.handle_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::ALT));
        assert_eq!(modal.project_name_cursor, 10); // End of "beta"

        modal.handle_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::ALT));
        assert_eq!(modal.project_name_cursor, 16); // End of "gamma"

        // Field navigation isolation: Up / Down or Tab switch focus
        modal.handle_key(key(KeyCode::Down));
        assert_eq!(modal.focus, CreationFocus::Agent);

        modal.handle_key(key(KeyCode::Up));
        assert_eq!(modal.focus, CreationFocus::ProjectName);

        modal.handle_key(key(KeyCode::Tab));
        assert_eq!(modal.focus, CreationFocus::Agent);

        modal.handle_key(key(KeyCode::BackTab));
        assert_eq!(modal.focus, CreationFocus::ProjectName);
    }

    #[test]
    fn test_project_name_readline_kills() {
        let config =
            Config::load_from_str("default_agent = \"claude\"\nrepos_dir = \".repos\"\n").unwrap();
        let temp_dir = TempDir::new().unwrap();
        let mut modal = ProjectCreationModal::new(&config, temp_dir.path(), None);
        modal.step = CreationStep::ConfigureDetails;
        modal.focus = CreationFocus::ProjectName;

        // 1. Ctrl+W (delete word backward)
        modal.set_project_name("billing-api-service");
        assert_eq!(modal.project_name_cursor, 19);

        modal.handle_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL));
        assert_eq!(modal.project_name(), "billing-api-");
        assert_eq!(modal.project_name_cursor, 12);

        modal.handle_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL));
        assert_eq!(modal.project_name(), "billing-");
        assert_eq!(modal.project_name_cursor, 8);

        // 2. Ctrl+K (delete to end)
        modal.set_project_name("customer-portal-web");
        modal.project_name_cursor = 8; // At '-' before portal
        modal.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL));
        assert_eq!(modal.project_name(), "customer");
        assert_eq!(modal.project_name_cursor, 8);

        // 3. Ctrl+U (delete to start)
        modal.set_project_name("customer-portal-web");
        modal.project_name_cursor = 9; // At 'p' in portal
        modal.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        assert_eq!(modal.project_name(), "portal-web");
        assert_eq!(modal.project_name_cursor, 0);
    }

    #[test]
    fn test_project_name_insertions_and_deletions() {
        let config =
            Config::load_from_str("default_agent = \"claude\"\nrepos_dir = \".repos\"\n").unwrap();
        let temp_dir = TempDir::new().unwrap();
        let mut modal = ProjectCreationModal::new(&config, temp_dir.path(), None);
        modal.step = CreationStep::ConfigureDetails;
        modal.focus = CreationFocus::ProjectName;

        // Type characters
        for c in "pay-svc".chars() {
            modal.handle_key(key(KeyCode::Char(c)));
        }
        assert_eq!(modal.project_name(), "pay-svc");
        assert_eq!(modal.project_name_cursor, 7);

        // Move to position 3 ("pay|-svc") and insert 's'
        modal.project_name_cursor = 3;
        modal.handle_key(key(KeyCode::Char('s')));
        assert_eq!(modal.project_name(), "pays-svc");
        assert_eq!(modal.project_name_cursor, 4);

        // Delete key deletes char under cursor ('-')
        modal.handle_key(key(KeyCode::Delete));
        assert_eq!(modal.project_name(), "payssvc");
        assert_eq!(modal.project_name_cursor, 4);

        // Backspace deletes char before cursor ('s')
        modal.handle_key(key(KeyCode::Backspace));
        assert_eq!(modal.project_name(), "paysvc");
        assert_eq!(modal.project_name_cursor, 3);

        // Move to start (pos 0) and Backspace does not delete or crash
        modal.handle_key(key(KeyCode::Home));
        assert_eq!(modal.project_name_cursor, 0);
        modal.handle_key(key(KeyCode::Backspace));
        assert_eq!(modal.project_name(), "paysvc");
        assert_eq!(modal.project_name_cursor, 0);
    }

    #[test]
    fn test_project_name_paste_sanitization_at_cursor() {
        let config =
            Config::load_from_str("default_agent = \"claude\"\nrepos_dir = \".repos\"\n").unwrap();
        let temp_dir = TempDir::new().unwrap();
        let mut modal = ProjectCreationModal::new(&config, temp_dir.path(), None);
        modal.step = CreationStep::ConfigureDetails;
        modal.focus = CreationFocus::ProjectName;

        modal.set_project_name("my-app");
        modal.project_name_cursor = 3; // Between '-' and 'a'

        // Paste text containing \r\n and control characters
        modal.handle_paste("super\r\n-\x01cool-");
        assert_eq!(modal.project_name(), "my-super-cool-app");
        assert_eq!(modal.project_name_cursor, 14);
    }

    #[test]
    fn test_project_name_hardware_cursor_position() {
        let config =
            Config::load_from_str("default_agent = \"claude\"\nrepos_dir = \".repos\"\n").unwrap();
        let temp_dir = TempDir::new().unwrap();
        let mut modal = ProjectCreationModal::new(&config, temp_dir.path(), None);
        modal.step = CreationStep::ConfigureDetails;
        modal.focus = CreationFocus::ProjectName;

        let area = Rect::new(0, 0, 100, 30);
        // inner.x = 13, prefix_len = 16 -> start x = 29, inner.y = 7 -> y = 10
        assert_eq!(modal.cursor_position(area), Some(Position::new(29, 10)));

        modal.set_project_name("test-project");
        assert_eq!(modal.cursor_position(area), Some(Position::new(41, 10)));

        modal.move_project_name_cursor_home();
        assert_eq!(modal.cursor_position(area), Some(Position::new(29, 10)));

        modal.project_name_cursor = 5;
        assert_eq!(modal.cursor_position(area), Some(Position::new(34, 10)));
    }

    #[test]
    fn test_single_template_workflow_and_backtrack() {
        let toml_str = r#"
            default_agent = "claude"
            repos_dir = ".repos"
            [templates.solo_service]
            description = "Solo service template"
            repos = ["repo-one"]
        "#;
        let config = Config::load_from_str(toml_str).unwrap();
        let temp_dir = TempDir::new().unwrap();
        let mut modal = ProjectCreationModal::new(&config, temp_dir.path(), None);

        // 1. Modal starts in CreationStep::SelectTemplate
        assert_eq!(modal.step, CreationStep::SelectTemplate);

        // 2. Single template is pre-selected and visible in preview
        assert_eq!(modal.selected_template(), Some("solo_service"));
        let area = Rect::new(0, 0, 100, 30);
        let mut buf = Buffer::empty(area);
        let theme = Theme::default();
        modal.render_modal(area, &mut buf, &theme);
        let rendered = buf
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect::<String>();
        assert!(rendered.contains("solo_service"));
        assert!(rendered.contains("Solo service template"));

        // 3. Enter advances to CreationStep::ConfigureDetails
        let act = modal.handle_key(key(KeyCode::Enter));
        assert_eq!(act, CreationModalAction::None);
        assert_eq!(modal.step, CreationStep::ConfigureDetails);
        assert_eq!(modal.focus, CreationFocus::ProjectName);

        // 4. Esc returns to CreationStep::SelectTemplate
        let act = modal.handle_key(key(KeyCode::Esc));
        assert_eq!(act, CreationModalAction::None);
        assert_eq!(modal.step, CreationStep::SelectTemplate);
        assert_eq!(modal.selected_template(), Some("solo_service"));

        // 5. Enter advances again
        let act = modal.handle_key(key(KeyCode::Enter));
        assert_eq!(act, CreationModalAction::None);
        assert_eq!(modal.step, CreationStep::ConfigureDetails);

        // 6. Backspace when project name is empty returns to CreationStep::SelectTemplate
        assert!(modal.project_name_input.is_empty());
        let act = modal.handle_key(key(KeyCode::Backspace));
        assert_eq!(act, CreationModalAction::None);
        assert_eq!(modal.step, CreationStep::SelectTemplate);

        // 7. Esc in CreationStep::SelectTemplate cancels with CreationModalAction::Cancel
        let act = modal.handle_key(key(KeyCode::Esc));
        assert_eq!(act, CreationModalAction::Cancel);
    }

    #[test]
    fn test_zero_templates_empty_state_and_cancel() {
        let toml_str = r#"
            default_agent = "claude"
            repos_dir = ".repos"
        "#;
        let config = Config::load_from_str(toml_str).unwrap();
        let temp_dir = TempDir::new().unwrap();
        let mut modal = ProjectCreationModal::new(&config, temp_dir.path(), None);

        // 1. Modal starts in CreationStep::SelectTemplate
        assert_eq!(modal.step, CreationStep::SelectTemplate);
        assert_eq!(modal.selected_template(), None);

        // 2. render_modal contains empty-state message, (No templates), and [Esc] Cancel
        let area = Rect::new(0, 0, 100, 30);
        let mut buf = Buffer::empty(area);
        let theme = Theme::default();
        modal.render_modal(area, &mut buf, &theme);
        let rendered = buf
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect::<String>();
        assert!(rendered.contains("No templates found in config.toml."));
        assert!(rendered.contains("Run 'botai edit' to configure templates."));
        assert!(rendered.contains("(No templates)"));
        assert!(rendered.contains("[Esc] Cancel"));

        // 3. Enter is a no-op (remains in SelectTemplate, does not advance)
        let act = modal.handle_key(key(KeyCode::Enter));
        assert_eq!(act, CreationModalAction::None);
        assert_eq!(modal.step, CreationStep::SelectTemplate);

        // 4. Esc cancels modal (CreationModalAction::Cancel)
        let act = modal.handle_key(key(KeyCode::Esc));
        assert_eq!(act, CreationModalAction::Cancel);
    }
}
