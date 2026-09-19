use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use thiserror::Error;

use crate::tui::Theme;

/// Errors that can occur during configuration loading, parsing, and resolution.
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Failed to parse config TOML: {0}")]
    Parse(#[from] toml::de::Error),

    #[error("Config file not found at: {0}")]
    NotFound(PathBuf),

    #[error("Configuration file already exists at '{0}'. Use --force to overwrite.")]
    AlreadyExists(PathBuf),

    #[error("Template '{0}' not found in configuration")]
    TemplateNotFound(String),

    #[error("Agent '{0}' not found in configuration")]
    AgentNotFound(String),

    #[error("Could not determine config directory")]
    ConfigDirNotFound,

    #[error("Repository collision in template '{template}': repository '{name}' is defined multiple times with conflicting sources '{source1}' and '{source2}'")]
    RepoCollision {
        template: String,
        name: String,
        source1: String,
        source2: String,
    },

    #[error("No editor found. Please set $VISUAL or $EDITOR, or pass --editor <cmd>.")]
    NoEditorFound,

    #[error("Editor command is empty")]
    EmptyEditorCommand,

    #[error("Failed to execute editor '{command}': {source}")]
    EditorExecutionFailed {
        command: String,
        #[source]
        source: std::io::Error,
    },
}

pub mod editor;
pub use editor::{
    find_executable_in_path, is_executable_in_path, resolve_editor, resolve_editor_with_platform,
    run_editor, split_command_words, TargetPlatform,
};

/// Backward compatibility alias for ConfigError::TemplateNotFound.
#[allow(non_upper_case_globals)]
pub type WorkspaceNotFound = ConfigError;

/// Default template content for `config.toml` with rich explanatory comments.
pub const DEFAULT_CONFIG_TEMPLATE: &str = r#"# ==============================================================================
# botai - Multi-Repo Workspace Orchestrator Configuration File
# ==============================================================================
# Default locations:
#   - Linux/macOS: ~/.config/botai/config.toml (or $XDG_CONFIG_HOME/botai/config.toml)
#   - Windows:     %APPDATA%\botai\config.toml
# ==============================================================================

# Global default AI agent tool to use when a template does not specify an override.
default_agent = "claude"

# Color theme for the TUI interface ("tokyo-night", "catppuccin", "cyberpunk", "gruvbox", "monochrome", "ansi").
theme = "tokyo-night"

# Central directory where remote repositories are cloned and cached (required).
# Can also be overridden at runtime by setting the BOTAI_REPOS_DIR environment variable.
repos_dir = "~/.local/share/botai/repos"

# ------------------------------------------------------------------------------
# Agent Definitions
# ------------------------------------------------------------------------------
# Configure the AI agent CLI commands, CLI arguments, and environment variables.
# Each entry defines a tool that botai can launch inside a project's root PTY.

[agents.claude]
command = "claude"
args = []
# env = { ANTHROPIC_API_KEY = "sk-ant-..." }

[agents.agy]
command = "agy"
args = []

[agents.copilot]
command = "gh"
args = ["copilot", "suggest"]

[agents.aider]
command = "aider"
args = ["--no-auto-commits"]

# ------------------------------------------------------------------------------
# Reusable Repository Definitions (Optional)
# ------------------------------------------------------------------------------
# Define canonical repository URLs and multi-command setup pipelines once.

[repos.web-frontend]
url = "https://github.com/example/ecommerce-web.git"
setup = [
  ["npm", "install"],
  ["npm", "run", "codegen"]
]

[repos.ecommerce-api]
url = "https://github.com/example/ecommerce-api.git"
setup = [
  ["cargo", "check"]
]

# ------------------------------------------------------------------------------
# Project Templates
# ------------------------------------------------------------------------------
# Define named multi-repo templates. Each template specifies:
# - description: (Optional) Human-readable summary of the template purpose.
# - default_agent: (Optional) Specific agent override for this template.
# - agents_md_append: (Optional) Custom instructions appended to AGENTS.md.
# - repos: (Required) List of registered repo keys, Git URLs, or local paths.
#   Entries can be simple strings (included by default) or inline tables:
#   e.g. { repo = "...", default = true|false } or { name = "...", default = false }
# - env: (Optional) Per-template environment variables injected into the agent.
# - post_create: (Optional) Commands executed at project root after setup.
# - pre_destroy: (Optional) Commands executed at project root before cleanup.

[templates.ecommerce]
description = "E-commerce platform services"
default_agent = "agy"
agents_md_append = """
- Run integration tests with `cargo test --workspace`.
- Follow coding standards documented in the template.
"""
repos = [
    "ecommerce-api",
    "web-frontend",
    "/home/user/code/ecommerce-shared"
]
post_create = [
    ["docker", "compose", "up", "-d"]
]
pre_destroy = [
    ["docker", "compose", "down"]
]
env = { PORT = "8080", NODE_ENV = "development" }
"#;

/// Determines if a string represents a remote Git URL.
pub fn is_git_url(s: &str) -> bool {
    let s = s.trim();
    s.starts_with("http://")
        || s.starts_with("https://")
        || s.starts_with("git@")
        || s.starts_with("ssh://")
        || s.starts_with("git://")
        || s.starts_with("file://")
}

/// Expands a leading tilde (`~`) in a path into the user's home directory.
pub fn expand_tilde<P: AsRef<Path>>(path: P) -> PathBuf {
    let path = path.as_ref();
    let path_str = path.to_string_lossy();
    if path_str.starts_with("~/") || path_str == "~" {
        if let Some(home) = directories::BaseDirs::new().map(|d| d.home_dir().to_path_buf()) {
            if path_str == "~" {
                return home;
            }
            return home.join(path_str.trim_start_matches("~/"));
        }
    }
    path.to_path_buf()
}

/// Derives the clean directory/repo name from a Git URL, local path, or identifier.
pub fn derive_repo_name(entry: &str) -> String {
    let entry = entry.trim();
    if entry.is_empty() {
        return "repo".to_string();
    }

    // Strip trailing slashes
    let cleaned = entry.trim_end_matches(['/', '\\']);

    // If it's a URL with scp syntax (git@host:org/repo.git)
    let path_part = if let Some((_, path)) = cleaned.split_once(':') {
        if !cleaned.starts_with("http://")
            && !cleaned.starts_with("https://")
            && !cleaned.starts_with("ssh://")
            && !cleaned.starts_with("file://")
            && !cleaned.starts_with("git://")
            && !path.starts_with('/')
            && !path.starts_with('\\')
            // Don't treat Windows drive letters like C:\path as scp
            && !cleaned.chars().next().is_some_and(|c| c.is_alphabetic() && cleaned.chars().nth(1) == Some(':'))
        {
            path
        } else {
            cleaned
        }
    } else {
        cleaned
    };

    // Take the last component separated by '/' or '\'
    let last_segment = path_part
        .split(['/', '\\'])
        .rfind(|s| !s.is_empty())
        .unwrap_or(entry);

    // Strip query parameters if any (e.g. ?foo=bar)
    let base = last_segment.split('?').next().unwrap_or(last_segment);

    // Strip trailing .git
    let final_name = base.strip_suffix(".git").unwrap_or(base);

    if final_name.is_empty() {
        "repo".to_string()
    } else {
        final_name.to_string()
    }
}

/// Parses a command hooks specification into a list of command argv arrays (`Vec<Vec<String>>`).
///
/// Accepts:
/// - A list of command argv arrays: `[ ["npm", "install"], ["npm", "run", "codegen"] ]`
/// - A single command argv array: `["cargo", "check"]`
/// - A list of command strings: `["npm install", "npm run codegen"]`
/// - A single command string: `"cargo check"`
pub fn parse_command_hooks_from_value(value: &toml::Value) -> Vec<Vec<String>> {
    match value {
        toml::Value::String(s) => {
            let words = split_command_words(s);
            if words.is_empty() {
                Vec::new()
            } else {
                vec![words]
            }
        }
        toml::Value::Array(arr) => {
            if arr.is_empty() {
                return Vec::new();
            }

            // Check if any element in the array is an array
            let has_sub_array = arr.iter().any(|v| matches!(v, toml::Value::Array(_)));

            if has_sub_array {
                let mut result = Vec::new();
                for item in arr {
                    match item {
                        toml::Value::Array(sub_arr) => {
                            let words: Vec<String> = sub_arr
                                .iter()
                                .filter_map(|v| match v {
                                    toml::Value::String(s) => Some(s.clone()),
                                    _ => None,
                                })
                                .collect();
                            if !words.is_empty() {
                                result.push(words);
                            }
                        }
                        toml::Value::String(s) => {
                            let words = split_command_words(s);
                            if !words.is_empty() {
                                result.push(words);
                            }
                        }
                        _ => {}
                    }
                }
                result
            } else {
                // All items are non-arrays (expected strings)
                let string_items: Vec<String> = arr
                    .iter()
                    .filter_map(|v| match v {
                        toml::Value::String(s) => Some(s.clone()),
                        _ => None,
                    })
                    .collect();

                if string_items.is_empty() {
                    return Vec::new();
                }

                // Check if any string contains multiple words (i.e. spaces/quotes)
                let any_multi_word = string_items
                    .iter()
                    .any(|s| split_command_words(s).len() > 1);

                if any_multi_word {
                    // Each item is a command string: ["npm install", "npm run codegen"]
                    let mut result = Vec::new();
                    for s in string_items {
                        let words = split_command_words(&s);
                        if !words.is_empty() {
                            result.push(words);
                        }
                    }
                    result
                } else {
                    // Single command argv array: ["cargo", "check"]
                    vec![string_items]
                }
            }
        }
        _ => Vec::new(),
    }
}

/// Custom serde deserializer for flexible command hook lists.
pub fn deserialize_command_hooks<'de, D>(deserializer: D) -> Result<Vec<Vec<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;
    let value = toml::Value::deserialize(deserializer)?;
    Ok(parse_command_hooks_from_value(&value))
}

/// Represents an entry in a template's `repos` list.
///
/// Can either be a simple string (e.g., `"ecommerce-api"`) which defaults to included,
/// or an inline table specifying `{ name = "...", default = false }` or `{ repo = "...", default = false }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TemplateRepoEntry {
    Simple(String),
    Detailed {
        #[serde(alias = "name")]
        repo: String,
        #[serde(default = "default_repo_included")]
        default: bool,
    },
}

pub fn default_repo_included() -> bool {
    true
}

impl TemplateRepoEntry {
    pub fn entry_str(&self) -> &str {
        match self {
            Self::Simple(s) => s.as_str(),
            Self::Detailed { repo, .. } => repo.as_str(),
        }
    }

    pub fn is_default_included(&self) -> bool {
        match self {
            Self::Simple(_) => true,
            Self::Detailed { default, .. } => *default,
        }
    }
}

impl From<&str> for TemplateRepoEntry {
    fn from(s: &str) -> Self {
        Self::Simple(s.to_string())
    }
}

impl From<String> for TemplateRepoEntry {
    fn from(s: String) -> Self {
        Self::Simple(s)
    }
}

impl AsRef<str> for TemplateRepoEntry {
    fn as_ref(&self) -> &str {
        self.entry_str()
    }
}

impl std::fmt::Display for TemplateRepoEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.entry_str())
    }
}

impl PartialEq<str> for TemplateRepoEntry {
    fn eq(&self, other: &str) -> bool {
        self.entry_str() == other
    }
}

impl PartialEq<&str> for TemplateRepoEntry {
    fn eq(&self, other: &&str) -> bool {
        self.entry_str() == *other
    }
}

impl PartialEq<String> for TemplateRepoEntry {
    fn eq(&self, other: &String) -> bool {
        self.entry_str() == other.as_str()
    }
}

impl PartialEq<TemplateRepoEntry> for String {
    fn eq(&self, other: &TemplateRepoEntry) -> bool {
        self.as_str() == other.entry_str()
    }
}

impl PartialEq<TemplateRepoEntry> for &str {
    fn eq(&self, other: &TemplateRepoEntry) -> bool {
        *self == other.entry_str()
    }
}

/// Information about a resolved repository within a template.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedRepo {
    /// Directory / repository name (e.g. "ecommerce-api").
    pub name: String,
    /// Absolute or resolved path where the repository is or should be located on disk.
    pub local_path: PathBuf,
    /// Remote clone URL if the repository is declared as a remote Git URL.
    pub clone_url: Option<String>,
    /// Original entry string from configuration.
    pub original_entry: String,
    /// Setup commands to execute in this worktree after checkout.
    #[serde(default)]
    pub setup_hooks: Vec<Vec<String>>,
    /// Per-repository environment variables applied during setup hook execution.
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// Whether this repository is included/selected by default during project creation.
    #[serde(default = "default_repo_included")]
    pub default_include: bool,
}

impl ResolvedRepo {
    /// Returns true if the repository currently exists on the local filesystem.
    pub fn exists_locally(&self) -> bool {
        self.local_path.exists()
    }
}

/// Configuration for a repository in the central `[repos]` registry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RepoConfig {
    /// Remote Git clone URL or local directory path.
    #[serde(default)]
    pub url: Option<String>,
    /// Setup commands to execute in the repository's worktree upon creation.
    #[serde(default, deserialize_with = "deserialize_command_hooks")]
    pub setup: Vec<Vec<String>>,
    /// Per-repository environment variables applied during setup hook execution.
    #[serde(default)]
    pub env: HashMap<String, String>,
}

/// Configuration for an AI agent tool (e.g., Claude Code, agy, Copilot CLI).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentConfig {
    /// Command to execute the agent CLI.
    pub command: String,
    /// Arguments to pass to the agent CLI command.
    #[serde(default)]
    pub args: Vec<String>,
    /// Optional environment variables injected when launching the agent.
    #[serde(default)]
    pub env: HashMap<String, String>,
}

/// Configuration for a project template containing multiple repositories.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateConfig {
    /// Optional human-readable description of the template.
    #[serde(default)]
    pub description: Option<String>,
    /// Optional template-specific default agent override.
    #[serde(default)]
    pub default_agent: Option<String>,
    /// Optional custom template instructions appended to AGENTS.md.
    #[serde(default)]
    pub agents_md_append: Option<String>,
    /// List of Git URLs, relative repo names, or local repository paths (strings or inline tables).
    pub repos: Vec<TemplateRepoEntry>,
    /// Per-template environment variables injected into the agent environment.
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// Post-creation lifecycle commands executed at the project root after worktrees and setup hooks finish.
    #[serde(default, deserialize_with = "deserialize_command_hooks")]
    pub post_create: Vec<Vec<String>>,
    /// Pre-destruction lifecycle commands executed at the project root before removing worktrees.
    #[serde(default, deserialize_with = "deserialize_command_hooks")]
    pub pre_destroy: Vec<Vec<String>>,
}

impl TemplateConfig {
    /// Returns the list of repository entry strings (URLs, keys, or paths) configured for this template.
    pub fn repo_names(&self) -> Vec<String> {
        self.repos
            .iter()
            .map(|entry| entry.entry_str().to_string())
            .collect()
    }

    /// Returns the list of default-included repository entry strings for this template.
    pub fn default_repo_names(&self) -> Vec<String> {
        self.repos
            .iter()
            .filter(|entry| entry.is_default_included())
            .map(|entry| entry.entry_str().to_string())
            .collect()
    }
}

/// Backward compatibility alias for `TemplateConfig`.
pub type WorkspaceConfig = TemplateConfig;

fn default_theme() -> String {
    "tokyo-night".to_string()
}

/// Root configuration for botai.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    /// Global fallback default agent name.
    pub default_agent: String,
    /// Color theme for the TUI interface ("tokyo-night", "catppuccin", "cyberpunk", "gruvbox", "monochrome", "ansi").
    #[serde(default = "default_theme")]
    pub theme: String,
    /// Central directory where remote repositories are cloned and cached (required).
    pub repos_dir: PathBuf,
    /// Central repository registry definitions.
    #[serde(default)]
    pub repos: HashMap<String, RepoConfig>,
    /// Defined agent tools.
    #[serde(default)]
    pub agents: HashMap<String, AgentConfig>,
    /// Defined project templates.
    #[serde(default, alias = "workspaces")]
    pub templates: HashMap<String, TemplateConfig>,
}

impl Config {
    /// Backward compatibility accessor for `templates`.
    #[inline]
    pub fn workspaces(&self) -> &HashMap<String, TemplateConfig> {
        &self.templates
    }
    /// Returns the resolved `Theme` palette configured for the application.
    pub fn theme(&self) -> Theme {
        Theme::from_name(&self.theme)
    }

    /// Returns the default annotated configuration template.
    pub fn default_template() -> &'static str {
        DEFAULT_CONFIG_TEMPLATE
    }

    /// Initializes a configuration file at the specified path.
    ///
    /// If the target file already exists and `force` is false, returns `ConfigError::AlreadyExists`.
    /// Automatically creates parent directories if needed.
    pub fn init_file<P: AsRef<Path>>(path: P, force: bool) -> Result<PathBuf, ConfigError> {
        let path = path.as_ref();
        if path.exists() && !force {
            return Err(ConfigError::AlreadyExists(path.to_path_buf()));
        }

        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }

        std::fs::write(path, Self::default_template())?;
        Ok(path.to_path_buf())
    }

    /// Parses configuration from a TOML string.
    pub fn load_from_str(s: &str) -> Result<Self, ConfigError> {
        let config: Config = toml::from_str(s)?;
        Ok(config)
    }

    /// Loads configuration from a file path.
    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Self, ConfigError> {
        let content = std::fs::read_to_string(&path)?;
        Self::load_from_str(&content)
    }

    /// Resolves the default configuration file path based on platform standards:
    /// - Unix / macOS: `~/.config/botai/config.toml` (or `$XDG_CONFIG_HOME/botai/config.toml`)
    /// - Windows: `%APPDATA%\botai\config.toml`
    pub fn default_path() -> Option<PathBuf> {
        directories::BaseDirs::new().map(|dirs| dirs.config_dir().join("botai").join("config.toml"))
    }

    /// Resolves the effective central repository directory, respecting:
    /// 1. `BOTAI_REPOS_DIR` environment variable (if set)
    /// 2. `repos_dir` configuration in `config.toml`
    pub fn resolved_repos_dir(&self) -> PathBuf {
        if let Ok(env_val) = std::env::var("BOTAI_REPOS_DIR") {
            if !env_val.trim().is_empty() {
                return expand_tilde(env_val.trim());
            }
        }
        expand_tilde(&self.repos_dir)
    }

    /// Resolves a single repository entry string into a `ResolvedRepo`.
    pub fn resolve_repo(&self, entry: &str) -> ResolvedRepo {
        let repos_dir = self.resolved_repos_dir();
        let trimmed = entry.trim();

        if let Some(repo_config) = self.repos.get(trimmed) {
            let name = trimmed.to_string();
            let setup_hooks = repo_config.setup.clone();
            let env = repo_config.env.clone();
            let original_entry = trimmed.to_string();

            if let Some(url_str) = &repo_config.url {
                let url_trimmed = url_str.trim();
                if is_git_url(url_trimmed) {
                    ResolvedRepo {
                        name,
                        local_path: repos_dir.join(trimmed),
                        clone_url: Some(url_trimmed.to_string()),
                        original_entry,
                        setup_hooks,
                        env,
                        default_include: true,
                    }
                } else {
                    let path_obj = Path::new(url_trimmed);
                    let local_path = if path_obj.is_absolute() || url_trimmed.starts_with('~') {
                        expand_tilde(url_trimmed)
                    } else if path_obj.exists() {
                        dunce::canonicalize(path_obj).unwrap_or_else(|_| path_obj.to_path_buf())
                    } else {
                        repos_dir.join(url_trimmed)
                    };
                    ResolvedRepo {
                        name,
                        local_path,
                        clone_url: None,
                        original_entry,
                        setup_hooks,
                        env,
                        default_include: true,
                    }
                }
            } else {
                ResolvedRepo {
                    name,
                    local_path: repos_dir.join(trimmed),
                    clone_url: None,
                    original_entry,
                    setup_hooks,
                    env,
                    default_include: true,
                }
            }
        } else {
            let name = derive_repo_name(trimmed);
            if is_git_url(trimmed) {
                let local_path = repos_dir.join(&name);
                ResolvedRepo {
                    name,
                    local_path,
                    clone_url: Some(trimmed.to_string()),
                    original_entry: trimmed.to_string(),
                    setup_hooks: Vec::new(),
                    env: HashMap::new(),
                    default_include: true,
                }
            } else {
                let path_obj = Path::new(trimmed);
                let local_path = if path_obj.is_absolute() || trimmed.starts_with('~') {
                    expand_tilde(trimmed)
                } else if path_obj.exists() {
                    dunce::canonicalize(path_obj).unwrap_or_else(|_| path_obj.to_path_buf())
                } else {
                    repos_dir.join(trimmed)
                };
                ResolvedRepo {
                    name,
                    local_path,
                    clone_url: None,
                    original_entry: trimmed.to_string(),
                    setup_hooks: Vec::new(),
                    env: HashMap::new(),
                    default_include: true,
                }
            }
        }
    }

    /// Resolves and validates all repositories declared in a given template.
    pub fn resolve_template_repos(
        &self,
        template_name: &str,
    ) -> Result<Vec<ResolvedRepo>, ConfigError> {
        let tpl = self
            .get_template(template_name)
            .ok_or_else(|| ConfigError::TemplateNotFound(template_name.to_string()))?;

        let mut resolved = Vec::new();
        let mut seen_names: HashMap<String, String> = HashMap::new();

        for repo_entry in &tpl.repos {
            let mut res = self.resolve_repo(repo_entry.entry_str());
            res.default_include = repo_entry.is_default_included();
            let entry_str = repo_entry.entry_str();
            if let Some(prev_entry) = seen_names.get(&res.name) {
                if prev_entry != entry_str {
                    return Err(ConfigError::RepoCollision {
                        template: template_name.to_string(),
                        name: res.name.clone(),
                        source1: prev_entry.clone(),
                        source2: entry_str.to_string(),
                    });
                }
            } else {
                seen_names.insert(res.name.clone(), entry_str.to_string());
            }
            resolved.push(res);
        }

        Ok(resolved)
    }

    /// Alias for `resolve_template_repos` for backward compatibility.
    pub fn resolve_workspace_repos(
        &self,
        workspace_name: &str,
    ) -> Result<Vec<ResolvedRepo>, ConfigError> {
        self.resolve_template_repos(workspace_name)
    }

    /// Retrieves a template configuration by name.
    pub fn get_template(&self, name: &str) -> Option<&TemplateConfig> {
        self.templates.get(name)
    }

    /// Alias for `get_template` for backward compatibility.
    pub fn get_workspace(&self, name: &str) -> Option<&TemplateConfig> {
        self.get_template(name)
    }

    /// Retrieves an agent configuration by name (exact match with case-insensitive fallback).
    pub fn get_agent(&self, name: &str) -> Option<&AgentConfig> {
        self.agents.get(name).or_else(|| {
            let trimmed = name.trim();
            self.agents
                .iter()
                .find(|(k, _)| k.trim().eq_ignore_ascii_case(trimmed))
                .map(|(_, v)| v)
        })
    }

    /// Resolves the effective agent configuration for a given template.
    ///
    /// If the template specifies a `default_agent`, that agent is looked up.
    /// Otherwise, falls back to the global `default_agent`.
    ///
    /// Returns `ConfigError::TemplateNotFound` if the template does not exist.
    /// Returns `ConfigError::AgentNotFound` if the target agent definition does not exist.
    pub fn resolve_agent_for_template(
        &self,
        template_name: &str,
    ) -> Result<&AgentConfig, ConfigError> {
        let template = self
            .get_template(template_name)
            .ok_or_else(|| ConfigError::TemplateNotFound(template_name.to_string()))?;

        let agent_name = template
            .default_agent
            .as_deref()
            .unwrap_or(&self.default_agent);

        self.get_agent(agent_name)
            .ok_or_else(|| ConfigError::AgentNotFound(agent_name.to_string()))
    }

    /// Alias for `resolve_agent_for_template` for backward compatibility.
    pub fn resolve_agent_for_workspace(
        &self,
        workspace_name: &str,
    ) -> Result<&AgentConfig, ConfigError> {
        self.resolve_agent_for_template(workspace_name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    const SAMPLE_VALID_TOML: &str = r#"
default_agent = "claude"
repos_dir = "/tmp/botai_repos"

[agents.claude]
command = "claude"
args = []
env = { ANTHROPIC_API_KEY = "test_key" }

[agents.agy]
command = "agy"
args = ["--verbose"]

[agents.copilot]
command = "gh"
args = ["copilot", "suggest"]

[templates.ecommerce]
description = "E-commerce platform services"
default_agent = "agy"
repos = [
    "/home/user/code/ecommerce-api",
    "/home/user/code/ecommerce-web",
    "/home/user/code/ecommerce-shared"
]
env = { PORT = "8080", NODE_ENV = "development" }

[templates.analytics]
description = "Analytics pipeline"
repos = [
    "/home/user/code/analytics-core"
]
"#;

    #[test]
    fn test_parse_valid_full_config() {
        let config = Config::load_from_str(SAMPLE_VALID_TOML)
            .expect("Valid TOML config should parse successfully");

        assert_eq!(config.default_agent, "claude");
        assert_eq!(config.repos_dir, PathBuf::from("/tmp/botai_repos"));
        assert_eq!(config.agents.len(), 3);
        assert_eq!(config.templates.len(), 2);

        // Verify agent 'claude'
        let claude = config
            .get_agent("claude")
            .expect("Agent 'claude' should exist");
        assert_eq!(claude.command, "claude");
        assert!(claude.args.is_empty());
        assert_eq!(
            claude.env.get("ANTHROPIC_API_KEY"),
            Some(&"test_key".to_string())
        );

        // Verify agent 'agy'
        let agy = config.get_agent("agy").expect("Agent 'agy' should exist");
        assert_eq!(agy.command, "agy");
        assert_eq!(agy.args, vec!["--verbose".to_string()]);
        assert!(agy.env.is_empty());

        // Verify template 'ecommerce'
        let ecommerce = config
            .get_template("ecommerce")
            .expect("Template 'ecommerce' should exist");
        assert_eq!(
            ecommerce.description.as_deref(),
            Some("E-commerce platform services")
        );
        assert_eq!(ecommerce.default_agent.as_deref(), Some("agy"));
        assert_eq!(
            ecommerce.repos,
            vec![
                "/home/user/code/ecommerce-api".to_string(),
                "/home/user/code/ecommerce-web".to_string(),
                "/home/user/code/ecommerce-shared".to_string(),
            ]
        );
        assert_eq!(ecommerce.env.get("PORT"), Some(&"8080".to_string()));
        assert_eq!(
            ecommerce.env.get("NODE_ENV"),
            Some(&"development".to_string())
        );

        // Verify template 'analytics' (omits default_agent and env)
        let analytics = config
            .get_template("analytics")
            .expect("Template 'analytics' should exist");
        assert_eq!(analytics.description.as_deref(), Some("Analytics pipeline"));
        assert_eq!(analytics.default_agent, None);
        assert_eq!(
            analytics.repos,
            vec!["/home/user/code/analytics-core".to_string()]
        );
        assert!(analytics.env.is_empty());
    }

    #[test]
    fn test_parse_backward_compatible_workspaces_alias() {
        let legacy_toml = r#"
default_agent = "claude"
repos_dir = "/tmp/botai_repos"

[workspaces.legacy]
description = "Legacy workspace entry"
repos = ["/tmp/legacy-repo"]
"#;
        let config = Config::load_from_str(legacy_toml)
            .expect("Legacy config with [workspaces] should parse via alias");

        assert_eq!(config.templates.len(), 1);
        let tpl = config
            .get_template("legacy")
            .expect("Template 'legacy' should be found");
        assert_eq!(tpl.description.as_deref(), Some("Legacy workspace entry"));
        assert_eq!(tpl.repos, vec!["/tmp/legacy-repo".to_string()]);
    }

    #[test]
    fn test_parse_minimal_valid_config() {
        let minimal_toml = r#"
default_agent = "basic"
repos_dir = "/tmp/repos"

[agents.basic]
command = "echo"

[templates.mini]
repos = ["/tmp/repo"]
"#;
        let config = Config::load_from_str(minimal_toml)
            .expect("Minimal TOML should parse successfully with defaults");

        assert_eq!(config.default_agent, "basic");
        assert_eq!(config.repos_dir, PathBuf::from("/tmp/repos"));

        let agent = config.get_agent("basic").expect("Agent 'basic' must exist");
        assert_eq!(agent.command, "echo");
        assert!(
            agent.args.is_empty(),
            "Omitted args should default to empty vec"
        );
        assert!(
            agent.env.is_empty(),
            "Omitted env should default to empty map"
        );

        let template = config
            .get_template("mini")
            .expect("Template 'mini' must exist");
        assert_eq!(
            template.description, None,
            "Omitted description should be None"
        );
        assert_eq!(
            template.default_agent, None,
            "Omitted default_agent should be None"
        );
        assert_eq!(template.repos, vec!["/tmp/repo".to_string()]);
        assert!(
            template.env.is_empty(),
            "Omitted env should default to empty map"
        );
    }

    #[test]
    fn test_get_template_and_agent_not_found() {
        let config = Config::load_from_str(SAMPLE_VALID_TOML).expect("Config should parse");

        assert!(config.get_template("non_existent_template").is_none());
        assert!(config.get_agent("non_existent_agent").is_none());
    }

    #[test]
    fn test_get_agent_case_insensitive() {
        let config = Config::load_from_str(SAMPLE_VALID_TOML).expect("Config should parse");

        assert!(config.get_agent("claude").is_some());
        assert!(config.get_agent("Claude").is_some());
        assert!(config.get_agent("CLAUDE").is_some());
        assert!(config.get_agent("agy").is_some());
        assert!(config.get_agent("Agy").is_some());
        assert!(config.get_agent("AGY").is_some());
        assert_eq!(
            config.get_agent("Claude").unwrap().command,
            config.get_agent("claude").unwrap().command
        );
    }

    #[test]
    fn test_resolve_agent_for_template_explicit_override() {
        let config = Config::load_from_str(SAMPLE_VALID_TOML).expect("Config should parse");

        // ecommerce template specifies default_agent = "agy"
        let resolved_agent = config
            .resolve_agent_for_template("ecommerce")
            .expect("Should resolve explicit agent 'agy' for template 'ecommerce'");
        assert_eq!(resolved_agent.command, "agy");
        assert_eq!(resolved_agent.args, vec!["--verbose".to_string()]);
    }

    #[test]
    fn test_resolve_agent_for_template_fallback_to_global() {
        let config = Config::load_from_str(SAMPLE_VALID_TOML).expect("Config should parse");

        // analytics template does not specify default_agent, should fall back to global default "claude"
        let resolved_agent = config
            .resolve_agent_for_template("analytics")
            .expect("Should fallback to global agent 'claude' for template 'analytics'");
        assert_eq!(resolved_agent.command, "claude");
    }

    #[test]
    fn test_resolve_agent_for_template_template_not_found() {
        let config = Config::load_from_str(SAMPLE_VALID_TOML).expect("Config should parse");

        let err = config
            .resolve_agent_for_template("ghost_template")
            .expect_err("Should return error for missing template");

        match err {
            ConfigError::TemplateNotFound(name) => assert_eq!(name, "ghost_template"),
            other => panic!("Expected ConfigError::TemplateNotFound, got {other:?}"),
        }
    }

    #[test]
    fn test_resolve_agent_for_template_agent_not_found_on_template_override() {
        let toml = r#"
default_agent = "claude"
repos_dir = "/tmp/repos"

[agents.claude]
command = "claude"

[templates.broken]
default_agent = "nonexistent_agent"
repos = ["/path/to/repo"]
"#;
        let config = Config::load_from_str(toml).expect("Config should parse");

        let err = config
            .resolve_agent_for_template("broken")
            .expect_err("Should return error when template references missing agent");

        match err {
            ConfigError::AgentNotFound(agent) => assert_eq!(agent, "nonexistent_agent"),
            other => panic!("Expected ConfigError::AgentNotFound, got {other:?}"),
        }
    }

    #[test]
    fn test_resolve_agent_for_template_agent_not_found_on_global_fallback() {
        let toml = r#"
default_agent = "missing_global_agent"
repos_dir = "/tmp/repos"

[agents.other]
command = "other"

[templates.test_ws]
repos = ["/path/to/repo"]
"#;
        let config = Config::load_from_str(toml).expect("Config should parse");

        let err = config
            .resolve_agent_for_template("test_ws")
            .expect_err("Should return error when global default agent is not in agents table");

        match err {
            ConfigError::AgentNotFound(agent) => assert_eq!(agent, "missing_global_agent"),
            other => panic!("Expected ConfigError::AgentNotFound, got {other:?}"),
        }
    }

    #[test]
    fn test_default_path_resolution() {
        let default_path = Config::default_path();
        assert!(
            default_path.is_some(),
            "Config::default_path() should resolve on supported platforms"
        );

        let path = default_path.unwrap();
        assert!(
            path.ends_with("botai/config.toml") || path.ends_with(r"botai\config.toml"),
            "Expected default config path to end with 'botai/config.toml', got: {path:?}"
        );

        #[cfg(unix)]
        {
            let path_str = path.to_string_lossy();
            assert!(
                path_str.contains(".config/botai/config.toml")
                    || path_str.contains("botai/config.toml"),
                "On Unix, default path should follow XDG or ~/.config conventions: {path_str}"
            );
        }

        #[cfg(windows)]
        {
            let path_str = path.to_string_lossy();
            assert!(
                path_str.contains(r"botai\config.toml"),
                "On Windows, default path should contain botai\\config.toml: {}",
                path_str
            );
        }
    }

    #[test]
    fn test_resolved_repos_dir_custom_and_env() {
        let toml_custom = r#"
default_agent = "claude"
repos_dir = "/custom/repos/storage"
"#;
        let config = Config::load_from_str(toml_custom).expect("Config with repos_dir");
        assert_eq!(
            config.resolved_repos_dir(),
            PathBuf::from("/custom/repos/storage")
        );
    }

    #[test]
    fn test_is_git_url_detection() {
        assert!(is_git_url("https://github.com/org/repo.git"));
        assert!(is_git_url("http://gitlab.com/org/repo"));
        assert!(is_git_url("git@github.com:org/repo.git"));
        assert!(is_git_url("ssh://git@git.myhost.com:2222/org/repo.git"));
        assert!(is_git_url("git://github.com/org/repo.git"));
        assert!(is_git_url("file:///tmp/repo.git"));

        assert!(!is_git_url("/home/user/code/my-repo"));
        assert!(!is_git_url("C:\\Users\\User\\code\\repo"));
        assert!(!is_git_url("relative-repo-name"));
    }

    #[test]
    fn test_derive_repo_name() {
        assert_eq!(
            derive_repo_name("https://github.com/example/ecommerce-api.git"),
            "ecommerce-api"
        );
        assert_eq!(
            derive_repo_name("https://github.com/example/ecommerce-web/"),
            "ecommerce-web"
        );
        assert_eq!(
            derive_repo_name("git@github.com:org/auth-service.git"),
            "auth-service"
        );
        assert_eq!(
            derive_repo_name("ssh://git@github.com/org/payment-core.git"),
            "payment-core"
        );
        assert_eq!(derive_repo_name("/home/user/code/shared-lib"), "shared-lib");
        assert_eq!(derive_repo_name("custom-tool"), "custom-tool");
        assert_eq!(
            derive_repo_name("https://example.com/repo.git?token=123"),
            "repo"
        );
    }

    #[test]
    fn test_resolve_template_repos_hybrid() {
        let toml = r#"
default_agent = "claude"
repos_dir = "/var/botai_repos"

[templates.hybrid]
repos = [
    "https://github.com/org/api-service.git",
    "git@github.com:org/web-frontend.git",
    "/opt/local/shared-db",
    "cached-worker"
]
"#;
        let config = Config::load_from_str(toml).expect("Parse hybrid config");
        let resolved = config
            .resolve_template_repos("hybrid")
            .expect("Resolve template repos");

        assert_eq!(resolved.len(), 4);

        // Remote HTTPS URL
        assert_eq!(resolved[0].name, "api-service");
        assert_eq!(
            resolved[0].local_path,
            PathBuf::from("/var/botai_repos/api-service")
        );
        assert_eq!(
            resolved[0].clone_url,
            Some("https://github.com/org/api-service.git".to_string())
        );

        // Remote SSH URL
        assert_eq!(resolved[1].name, "web-frontend");
        assert_eq!(
            resolved[1].local_path,
            PathBuf::from("/var/botai_repos/web-frontend")
        );
        assert_eq!(
            resolved[1].clone_url,
            Some("git@github.com:org/web-frontend.git".to_string())
        );

        // Absolute path
        assert_eq!(resolved[2].name, "shared-db");
        assert_eq!(
            resolved[2].local_path,
            PathBuf::from("/opt/local/shared-db")
        );
        assert_eq!(resolved[2].clone_url, None);

        // Relative name resolved into repos_dir
        assert_eq!(resolved[3].name, "cached-worker");
        assert_eq!(
            resolved[3].local_path,
            PathBuf::from("/var/botai_repos/cached-worker")
        );
        assert_eq!(resolved[3].clone_url, None);
    }

    #[test]
    fn test_resolve_template_repos_collision_error() {
        let toml = r#"
default_agent = "claude"
repos_dir = "/tmp/repos"

[templates.conflicted]
repos = [
    "https://github.com/orgA/auth-service.git",
    "https://github.com/orgB/auth-service.git"
]
"#;
        let config = Config::load_from_str(toml).expect("Parse conflicted config");
        let err = config
            .resolve_template_repos("conflicted")
            .expect_err("Should error on name collision");

        match err {
            ConfigError::RepoCollision {
                template,
                name,
                source1,
                source2,
            } => {
                assert_eq!(template, "conflicted");
                assert_eq!(name, "auth-service");
                assert_eq!(source1, "https://github.com/orgA/auth-service.git");
                assert_eq!(source2, "https://github.com/orgB/auth-service.git");
            }
            other => panic!("Expected ConfigError::RepoCollision, got {other:?}"),
        }
    }

    #[test]
    fn test_error_missing_required_repos_dir() {
        let toml_missing_repos_dir = r#"
default_agent = "claude"

[agents.claude]
command = "claude"
"#;
        let err = Config::load_from_str(toml_missing_repos_dir)
            .expect_err("Config missing required repos_dir field should produce a Parse error");

        match err {
            ConfigError::Parse(_) => (),
            other => panic!("Expected ConfigError::Parse, got: {other:?}"),
        }
    }

    #[test]
    fn test_error_malformed_toml_syntax() {
        let malformed_toml = r#"
default_agent = "claude"
repos_dir = "/tmp/repos"
[agents.claude
command = "claude"
"#;
        let err = Config::load_from_str(malformed_toml)
            .expect_err("Malformed TOML syntax should produce a Parse error");

        match err {
            ConfigError::Parse(_) => (),
            other => panic!("Expected ConfigError::Parse, got: {other:?}"),
        }
    }

    #[test]
    fn test_error_missing_required_default_agent() {
        let toml_missing_default_agent = r#"
repos_dir = "/tmp/repos"

[agents.claude]
command = "claude"

[templates.ecommerce]
repos = ["/path/to/repo"]
"#;
        let err = Config::load_from_str(toml_missing_default_agent)
            .expect_err("Config missing required default_agent field should produce a Parse error");

        match err {
            ConfigError::Parse(_) => (),
            other => panic!("Expected ConfigError::Parse, got: {other:?}"),
        }
    }

    #[test]
    fn test_error_missing_agent_command() {
        let toml_missing_command = r#"
default_agent = "claude"
repos_dir = "/tmp/repos"

[agents.claude]
args = ["--some-arg"]

[templates.ecommerce]
repos = ["/path/to/repo"]
"#;
        let err = Config::load_from_str(toml_missing_command)
            .expect_err("Agent missing required 'command' field should produce a Parse error");

        match err {
            ConfigError::Parse(_) => (),
            other => panic!("Expected ConfigError::Parse, got: {other:?}"),
        }
    }

    #[test]
    fn test_error_missing_template_repos() {
        let toml_missing_repos = r#"
default_agent = "claude"
repos_dir = "/tmp/repos"

[agents.claude]
command = "claude"

[templates.ecommerce]
description = "Missing repos field"
"#;
        let err = Config::load_from_str(toml_missing_repos)
            .expect_err("Template missing required 'repos' field should produce a Parse error");

        match err {
            ConfigError::Parse(_) => (),
            other => panic!("Expected ConfigError::Parse, got: {other:?}"),
        }
    }

    #[test]
    fn test_error_invalid_field_type() {
        let toml_invalid_type = r#"
default_agent = "claude"
repos_dir = "/tmp/repos"

[agents.claude]
command = "claude"

[templates.ecommerce]
repos = "not-a-list"
"#;
        let err = Config::load_from_str(toml_invalid_type)
            .expect_err("Invalid type for repos should produce a Parse error");

        match err {
            ConfigError::Parse(_) => (),
            other => panic!("Expected ConfigError::Parse, got: {other:?}"),
        }
    }

    #[test]
    fn test_error_empty_string_input() {
        let err = Config::load_from_str("")
            .expect_err("Empty string should produce a Parse error due to missing default_agent");

        match err {
            ConfigError::Parse(_) => (),
            other => panic!("Expected ConfigError::Parse, got: {other:?}"),
        }
    }

    #[test]
    fn test_load_from_file_success() {
        let mut temp_file = NamedTempFile::new().expect("Failed to create temp file");
        temp_file
            .write_all(SAMPLE_VALID_TOML.as_bytes())
            .expect("Failed to write to temp file");

        let config = Config::load_from_file(temp_file.path())
            .expect("Config::load_from_file should load and parse valid file");

        assert_eq!(config.default_agent, "claude");
        assert_eq!(config.templates.len(), 2);
    }

    #[test]
    fn test_load_from_file_not_found() {
        let non_existent_path = PathBuf::from("/non/existent/path/to/botai_test_config.toml");
        let err = Config::load_from_file(&non_existent_path)
            .expect_err("Loading from non-existent file should produce an error");

        match err {
            ConfigError::Io(e) => assert_eq!(e.kind(), std::io::ErrorKind::NotFound),
            ConfigError::NotFound(p) => assert_eq!(p, non_existent_path),
            other => panic!("Expected ConfigError::Io or ConfigError::NotFound, got: {other:?}"),
        }
    }

    #[test]
    fn test_cross_platform_repo_paths() {
        let toml_paths = r#"
default_agent = "claude"
repos_dir = "/tmp/repos"

[agents.claude]
command = "claude"

[templates.cross_platform]
repos = [
    "C:\\Users\\User\\Projects\\repo1",
    "/home/user/projects/repo2",
    "relative/path/to/repo3",
    "./another/relative/repo4"
]
"#;
        let config = Config::load_from_str(toml_paths)
            .expect("Config with various cross-platform path representations should parse");

        let ws = config
            .get_template("cross_platform")
            .expect("Template must exist");
        assert_eq!(ws.repos.len(), 4);
        assert_eq!(ws.repos[0], "C:\\Users\\User\\Projects\\repo1");
        assert_eq!(ws.repos[1], "/home/user/projects/repo2");
        assert_eq!(ws.repos[2], "relative/path/to/repo3");
        assert_eq!(ws.repos[3], "./another/relative/repo4");
    }

    #[test]
    fn test_unicode_in_template_and_paths() {
        let toml_unicode = r#"
default_agent = "claude"
repos_dir = "/home/пользователь/botai_repos"

[agents.claude]
command = "claude"

[templates."проект-тест"]
description = "Тестовый воркспейс с юникодом 🚀"
repos = [
    "/home/пользователь/проекты/тест-репо",
    "/home/user/日本語/リポジトリ"
]
env = { GREETING = "こんにちは" }
"#;
        let config = Config::load_from_str(toml_unicode).expect(
            "Unicode template names, descriptions, paths, and env vars should parse cleanly",
        );

        let ws = config
            .get_template("проект-тест")
            .expect("Unicode template should be found");
        assert_eq!(
            ws.description.as_deref(),
            Some("Тестовый воркспейс с юникодом 🚀")
        );
        assert_eq!(ws.repos[0], "/home/пользователь/проекты/тест-репо");
        assert_eq!(ws.repos[1], "/home/user/日本語/リポジトリ");
        assert_eq!(ws.env.get("GREETING"), Some(&"こんにちは".to_string()));
    }

    #[test]
    fn test_default_template_is_valid_config() {
        let template = Config::default_template();
        assert!(!template.is_empty(), "Template must not be empty");
        assert!(
            template.contains("default_agent = \"claude\""),
            "Template must contain default_agent"
        );
        assert!(
            template.contains("theme = \"tokyo-night\""),
            "Template must define theme"
        );
        assert!(
            template.contains("repos_dir = \"~/.local/share/botai/repos\""),
            "Template must define repos_dir"
        );
        assert!(
            template.contains("[agents.claude]"),
            "Template must define claude agent"
        );
        assert!(
            template.contains("[agents.agy]"),
            "Template must define agy agent"
        );
        assert!(
            template.contains("[agents.copilot]"),
            "Template must define copilot agent"
        );
        assert!(
            template.contains("[agents.aider]"),
            "Template must define aider agent"
        );
        assert!(
            template.contains("[templates.ecommerce]"),
            "Template must define sample template"
        );

        let parsed = Config::load_from_str(template)
            .expect("Default template must parse cleanly into Config struct");

        assert_eq!(parsed.default_agent, "claude");
        assert_eq!(parsed.theme, "tokyo-night");
        assert_eq!(parsed.theme(), Theme::tokyo_night());
        assert_eq!(
            parsed.repos_dir,
            PathBuf::from("~/.local/share/botai/repos")
        );
        assert_eq!(parsed.agents.len(), 4);
        assert!(parsed.get_agent("claude").is_some());
        assert!(parsed.get_agent("agy").is_some());
        assert!(parsed.get_agent("copilot").is_some());
        assert!(parsed.get_agent("aider").is_some());

        let ws = parsed
            .get_template("ecommerce")
            .expect("ecommerce template must exist in template");
        assert_eq!(ws.default_agent.as_deref(), Some("agy"));
        assert!(ws.agents_md_append.is_some());
        assert!(ws
            .agents_md_append
            .as_deref()
            .unwrap()
            .contains("cargo test --workspace"));
        assert_eq!(ws.repos.len(), 3);
        assert_eq!(ws.env.get("PORT"), Some(&"8080".to_string()));
    }

    #[test]
    fn test_config_theme_resolution_and_defaults() {
        // 1. Missing theme field defaults to "tokyo-night"
        let toml_no_theme = r#"
default_agent = "claude"
repos_dir = "/tmp/repos"
"#;
        let config_default = Config::load_from_str(toml_no_theme).unwrap();
        assert_eq!(config_default.theme, "tokyo-night");
        assert_eq!(config_default.theme(), Theme::tokyo_night());

        // 2. Explicit theme
        let toml_catppuccin = r#"
default_agent = "claude"
theme = "catppuccin"
repos_dir = "/tmp/repos"
"#;
        let config_cat = Config::load_from_str(toml_catppuccin).unwrap();
        assert_eq!(config_cat.theme, "catppuccin");
        assert_eq!(config_cat.theme(), Theme::catppuccin());

        // 3. Cyberpunk alias
        let toml_cyber = r#"
default_agent = "claude"
theme = "synthwave"
repos_dir = "/tmp/repos"
"#;
        let config_cyber = Config::load_from_str(toml_cyber).unwrap();
        assert_eq!(config_cyber.theme(), Theme::cyberpunk());

        // 4. Gruvbox alias
        let toml_gruvbox = r#"
default_agent = "claude"
theme = "gruvbox-dark"
repos_dir = "/tmp/repos"
"#;
        let config_gruv = Config::load_from_str(toml_gruvbox).unwrap();
        assert_eq!(config_gruv.theme(), Theme::gruvbox());

        // 5. Monochrome alias
        let toml_mono = r#"
default_agent = "claude"
theme = "zinc"
repos_dir = "/tmp/repos"
"#;
        let config_mono = Config::load_from_str(toml_mono).unwrap();
        assert_eq!(config_mono.theme(), Theme::monochrome());

        // 6. ANSI alias
        let toml_ansi = r#"
default_agent = "claude"
theme = "classic"
repos_dir = "/tmp/repos"
"#;
        let config_ansi = Config::load_from_str(toml_ansi).unwrap();
        assert_eq!(config_ansi.theme(), Theme::ansi());
    }

    #[test]
    fn test_init_file_creates_new_file_and_directories() {
        let temp_dir = tempfile::tempdir().expect("Failed to create tempdir");
        let nested_config = temp_dir
            .path()
            .join("nested")
            .join("sub")
            .join("config.toml");

        assert!(!nested_config.exists());

        let written_path = Config::init_file(&nested_config, false)
            .expect("init_file should succeed when file does not exist");
        assert_eq!(written_path, nested_config);
        assert!(nested_config.exists());

        let loaded = Config::load_from_file(&nested_config)
            .expect("Generated config file must be valid and readable");
        assert_eq!(loaded.default_agent, "claude");
        assert_eq!(loaded.theme, "tokyo-night");
        assert_eq!(loaded.theme(), Theme::tokyo_night());
    }

    #[test]
    fn test_init_file_already_exists_fails_without_force() {
        let temp_dir = tempfile::tempdir().expect("Failed to create tempdir");
        let config_path = temp_dir.path().join("config.toml");

        // First initialization
        Config::init_file(&config_path, false).expect("First init should succeed");
        assert!(config_path.exists());

        // Second initialization without force
        let err = Config::init_file(&config_path, false)
            .expect_err("Second init without force should fail");

        match err {
            ConfigError::AlreadyExists(p) => assert_eq!(p, config_path),
            other => panic!("Expected ConfigError::AlreadyExists, got: {other:?}"),
        }
    }

    #[test]
    fn test_init_file_overwrites_with_force() {
        let temp_dir = tempfile::tempdir().expect("Failed to create tempdir");
        let config_path = temp_dir.path().join("config.toml");

        std::fs::write(&config_path, "corrupted toml content")
            .expect("Failed to write dummy content");

        // Initialization with force should overwrite
        let written_path = Config::init_file(&config_path, true)
            .expect("init_file with force=true must succeed and overwrite");
        assert_eq!(written_path, config_path);

        let content = std::fs::read_to_string(&config_path).expect("Failed to read file");
        assert!(content.contains("default_agent = \"claude\""));
    }

    #[test]
    fn test_parse_template_agents_md_append_present() {
        let toml = r#"
default_agent = "claude"
repos_dir = "/tmp/repos"

[templates.custom_instructions]
description = "Template with instructions"
agents_md_append = """
### Important Rules
- Never push directly to main.
- Run `cargo fmt` before submitting PRs.
"""
repos = ["/path/to/repo"]
"#;
        let config =
            Config::load_from_str(toml).expect("Config with agents_md_append should parse");
        let ws = config
            .get_template("custom_instructions")
            .expect("Template should exist");

        assert!(ws.agents_md_append.is_some());
        let instructions = ws.agents_md_append.as_deref().unwrap();
        assert!(instructions.contains("### Important Rules"));
        assert!(instructions.contains("Never push directly to main."));
        assert!(instructions.contains("Run `cargo fmt` before submitting PRs."));
    }

    #[test]
    fn test_parse_template_agents_md_append_omitted() {
        let toml = r#"
default_agent = "claude"
repos_dir = "/tmp/repos"

[templates.no_instructions]
repos = ["/path/to/repo"]
"#;
        let config = Config::load_from_str(toml).expect("Config should parse");
        let ws = config
            .get_template("no_instructions")
            .expect("Template should exist");

        assert_eq!(ws.agents_md_append, None);
    }

    #[test]
    fn test_flexible_command_hooks_parsing_formats() {
        // 1. Array of command argv arrays
        let toml1 = r#"
default_agent = "claude"
repos_dir = "/tmp/repos"

[repos.web]
setup = [
    ["npm", "install"],
    ["npm", "run", "codegen"]
]

[templates.demo]
repos = ["web"]
post_create = [
    ["docker", "compose", "up", "-d"]
]
pre_destroy = [
    ["docker", "compose", "down"]
]
"#;
        let cfg1 = Config::load_from_str(toml1).expect("Parse array of argv arrays");
        let repo1 = cfg1.repos.get("web").unwrap();
        assert_eq!(
            repo1.setup,
            vec![
                vec!["npm".to_string(), "install".to_string()],
                vec!["npm".to_string(), "run".to_string(), "codegen".to_string()]
            ]
        );
        let tpl1 = cfg1.get_template("demo").unwrap();
        assert_eq!(
            tpl1.post_create,
            vec![vec![
                "docker".to_string(),
                "compose".to_string(),
                "up".to_string(),
                "-d".to_string()
            ]]
        );
        assert_eq!(
            tpl1.pre_destroy,
            vec![vec![
                "docker".to_string(),
                "compose".to_string(),
                "down".to_string()
            ]]
        );

        // 2. Single command argv array: setup = ["cargo", "check"]
        let toml2 = r#"
default_agent = "claude"
repos_dir = "/tmp/repos"

[repos.backend]
setup = ["cargo", "check", "--workspace"]

[templates.demo]
repos = ["backend"]
post_create = ["echo", "ready"]
"#;
        let cfg2 = Config::load_from_str(toml2).expect("Parse single argv array");
        let repo2 = cfg2.repos.get("backend").unwrap();
        assert_eq!(
            repo2.setup,
            vec![vec![
                "cargo".to_string(),
                "check".to_string(),
                "--workspace".to_string()
            ]]
        );
        let tpl2 = cfg2.get_template("demo").unwrap();
        assert_eq!(
            tpl2.post_create,
            vec![vec!["echo".to_string(), "ready".to_string()]]
        );

        // 3. List of command strings: setup = ["npm install", "npm run codegen"]
        let toml3 = r#"
default_agent = "claude"
repos_dir = "/tmp/repos"

[repos.web]
setup = ["npm install", "npm run codegen"]

[templates.demo]
repos = ["web"]
post_create = ["docker compose up -d", "cargo check"]
"#;
        let cfg3 = Config::load_from_str(toml3).expect("Parse list of command strings");
        let repo3 = cfg3.repos.get("web").unwrap();
        assert_eq!(
            repo3.setup,
            vec![
                vec!["npm".to_string(), "install".to_string()],
                vec!["npm".to_string(), "run".to_string(), "codegen".to_string()]
            ]
        );
        let tpl3 = cfg3.get_template("demo").unwrap();
        assert_eq!(
            tpl3.post_create,
            vec![
                vec![
                    "docker".to_string(),
                    "compose".to_string(),
                    "up".to_string(),
                    "-d".to_string()
                ],
                vec!["cargo".to_string(), "check".to_string()]
            ]
        );

        // 4. Single command string: setup = "cargo check"
        let toml4 = r#"
default_agent = "claude"
repos_dir = "/tmp/repos"

[repos.backend]
setup = "cargo check --release"
"#;
        let cfg4 = Config::load_from_str(toml4).expect("Parse single string");
        let repo4 = cfg4.repos.get("backend").unwrap();
        assert_eq!(
            repo4.setup,
            vec![vec![
                "cargo".to_string(),
                "check".to_string(),
                "--release".to_string()
            ]]
        );

        // 5. Empty array and omitted fields
        let toml5 = r#"
default_agent = "claude"
repos_dir = "/tmp/repos"

[repos.empty_setup]
setup = []

[repos.no_setup]

[templates.demo]
repos = ["empty_setup", "no_setup"]
"#;
        let cfg5 = Config::load_from_str(toml5).expect("Parse empty and omitted");
        assert_eq!(
            cfg5.repos.get("empty_setup").unwrap().setup,
            Vec::<Vec<String>>::new()
        );
        assert_eq!(
            cfg5.repos.get("no_setup").unwrap().setup,
            Vec::<Vec<String>>::new()
        );
        let tpl5 = cfg5.get_template("demo").unwrap();
        assert_eq!(tpl5.post_create, Vec::<Vec<String>>::new());
        assert_eq!(tpl5.pre_destroy, Vec::<Vec<String>>::new());
    }

    #[test]
    fn test_central_repos_registry_resolution() {
        let toml = r#"
default_agent = "claude"
repos_dir = "/tmp/botai_repos"

[repos.web-frontend]
url = "https://github.com/example/ecommerce-web.git"
setup = [
    ["npm", "install"],
    ["npm", "run", "codegen"]
]
env = { NODE_ENV = "test" }

[repos.local-service]
url = "/home/user/code/local-service"
setup = ["cargo check"]
env = { RUST_BACKTRACE = "1" }

[repos.implicit-cached]
setup = ["make build"]

[templates.hybrid]
repos = [
    "web-frontend",
    "local-service",
    "implicit-cached",
    "https://github.com/example/unregistered-api.git",
    "relative-unregistered",
    "/absolute/unregistered"
]
"#;
        let config = Config::load_from_str(toml).expect("Parse hybrid config");
        assert_eq!(config.repos.len(), 3);

        let resolved = config
            .resolve_template_repos("hybrid")
            .expect("Resolve template repos");
        assert_eq!(resolved.len(), 6);

        // 1. Registered repo with Git URL
        let r0 = &resolved[0];
        assert_eq!(r0.name, "web-frontend");
        assert_eq!(
            r0.clone_url,
            Some("https://github.com/example/ecommerce-web.git".to_string())
        );
        assert_eq!(
            r0.local_path,
            PathBuf::from("/tmp/botai_repos/web-frontend")
        );
        assert_eq!(
            r0.setup_hooks,
            vec![
                vec!["npm".to_string(), "install".to_string()],
                vec!["npm".to_string(), "run".to_string(), "codegen".to_string()]
            ]
        );
        assert_eq!(r0.env.get("NODE_ENV"), Some(&"test".to_string()));

        // 2. Registered repo with local path URL
        let r1 = &resolved[1];
        assert_eq!(r1.name, "local-service");
        assert_eq!(r1.clone_url, None);
        assert_eq!(
            r1.local_path,
            PathBuf::from("/home/user/code/local-service")
        );
        assert_eq!(
            r1.setup_hooks,
            vec![vec!["cargo".to_string(), "check".to_string()]]
        );
        assert_eq!(r1.env.get("RUST_BACKTRACE"), Some(&"1".to_string()));

        // 3. Registered repo with None URL (defaults to repos_dir/key)
        let r2 = &resolved[2];
        assert_eq!(r2.name, "implicit-cached");
        assert_eq!(r2.clone_url, None);
        assert_eq!(
            r2.local_path,
            PathBuf::from("/tmp/botai_repos/implicit-cached")
        );
        assert_eq!(
            r2.setup_hooks,
            vec![vec!["make".to_string(), "build".to_string()]]
        );

        // 4. Unregistered Git URL
        let r3 = &resolved[3];
        assert_eq!(r3.name, "unregistered-api");
        assert_eq!(
            r3.clone_url,
            Some("https://github.com/example/unregistered-api.git".to_string())
        );
        assert_eq!(
            r3.local_path,
            PathBuf::from("/tmp/botai_repos/unregistered-api")
        );
        assert!(r3.setup_hooks.is_empty());
        assert!(r3.env.is_empty());

        // 5. Unregistered relative name
        let r4 = &resolved[4];
        assert_eq!(r4.name, "relative-unregistered");
        assert_eq!(r4.clone_url, None);
        assert_eq!(
            r4.local_path,
            PathBuf::from("/tmp/botai_repos/relative-unregistered")
        );
        assert!(r4.setup_hooks.is_empty());

        // 6. Unregistered absolute path
        let r5 = &resolved[5];
        assert_eq!(r5.name, "unregistered");
        assert_eq!(r5.clone_url, None);
        assert_eq!(r5.local_path, PathBuf::from("/absolute/unregistered"));
        assert!(r5.setup_hooks.is_empty());
    }

    #[test]
    fn test_template_repo_entry_string_array_backward_compatibility() {
        let toml_str = r#"
default_agent = "claude"
repos_dir = "/tmp/repos"

[templates.simple]
description = "Simple string array"
repos = ["service-a", "service-b"]
"#;
        let config =
            Config::load_from_str(toml_str).expect("Valid string array config should parse");
        let tpl = config.get_template("simple").expect("Template must exist");
        assert_eq!(tpl.repos.len(), 2);
        assert_eq!(
            tpl.repos[0],
            TemplateRepoEntry::Simple("service-a".to_string())
        );
        assert_eq!(
            tpl.repos[1],
            TemplateRepoEntry::Simple("service-b".to_string())
        );
        assert_eq!(tpl.repos[0].entry_str(), "service-a");
        assert_eq!(tpl.repos[1].entry_str(), "service-b");
        assert!(tpl.repos[0].is_default_included());
        assert!(tpl.repos[1].is_default_included());
        assert_eq!(tpl.repo_names(), vec!["service-a", "service-b"]);
        assert_eq!(tpl.default_repo_names(), vec!["service-a", "service-b"]);
    }

    #[test]
    fn test_template_repo_entry_inline_table_default_false() {
        let toml_str = r#"
default_agent = "claude"
repos_dir = "/tmp/repos"

[templates.optional_repo]
description = "Template with optional repository"
repos = [
    { repo = "docs", default = false }
]
"#;
        let config = Config::load_from_str(toml_str)
            .expect("Inline table with default = false should parse");
        let tpl = config
            .get_template("optional_repo")
            .expect("Template must exist");
        assert_eq!(tpl.repos.len(), 1);
        match &tpl.repos[0] {
            TemplateRepoEntry::Detailed { repo, default } => {
                assert_eq!(repo, "docs");
                assert!(!default);
            }
            _ => panic!("Expected Detailed variant"),
        }
        assert_eq!(tpl.repos[0].entry_str(), "docs");
        assert!(!tpl.repos[0].is_default_included());
        assert_eq!(tpl.repo_names(), vec!["docs"]);
        assert!(tpl.default_repo_names().is_empty());
    }

    #[test]
    fn test_template_repo_entry_name_and_repo_aliases() {
        let toml_str = r#"
default_agent = "claude"
repos_dir = "/tmp/repos"

[templates.aliased]
repos = [
    { name = "billing-service", default = false },
    { repo = "auth-service", default = false }
]
"#;
        let config = Config::load_from_str(toml_str)
            .expect("Aliases 'name' and 'repo' should parse cleanly");
        let tpl = config.get_template("aliased").expect("Template must exist");
        assert_eq!(tpl.repos.len(), 2);
        assert_eq!(tpl.repos[0].entry_str(), "billing-service");
        assert!(!tpl.repos[0].is_default_included());
        assert_eq!(tpl.repos[1].entry_str(), "auth-service");
        assert!(!tpl.repos[1].is_default_included());
        assert_eq!(tpl.repo_names(), vec!["billing-service", "auth-service"]);
        assert!(tpl.default_repo_names().is_empty());
    }

    #[test]
    fn test_template_repo_entry_omitted_default_defaults_to_true() {
        let toml_str = r#"
default_agent = "claude"
repos_dir = "/tmp/repos"

[templates.implicit_default]
repos = [
    { repo = "core-api" },
    { name = "frontend" }
]
"#;
        let config =
            Config::load_from_str(toml_str).expect("Omitted default field should parse as true");
        let tpl = config
            .get_template("implicit_default")
            .expect("Template must exist");
        assert_eq!(tpl.repos.len(), 2);
        assert_eq!(tpl.repos[0].entry_str(), "core-api");
        assert!(tpl.repos[0].is_default_included());
        assert_eq!(tpl.repos[1].entry_str(), "frontend");
        assert!(tpl.repos[1].is_default_included());
        assert_eq!(tpl.default_repo_names(), vec!["core-api", "frontend"]);
    }

    #[test]
    fn test_template_repo_entry_mixed_array() {
        let toml_str = r#"
default_agent = "claude"
repos_dir = "/tmp/repos"

[templates.mixed]
repos = [
    "service-api",
    { repo = "web-frontend", default = true },
    { name = "docs-site", default = false },
    "/local/shared-lib",
    { repo = "extra-tools", default = false }
]
"#;
        let config = Config::load_from_str(toml_str)
            .expect("Mixed array of strings and tables should parse");
        let tpl = config.get_template("mixed").expect("Template must exist");
        assert_eq!(tpl.repos.len(), 5);

        assert_eq!(tpl.repos[0].entry_str(), "service-api");
        assert!(tpl.repos[0].is_default_included());

        assert_eq!(tpl.repos[1].entry_str(), "web-frontend");
        assert!(tpl.repos[1].is_default_included());

        assert_eq!(tpl.repos[2].entry_str(), "docs-site");
        assert!(!tpl.repos[2].is_default_included());

        assert_eq!(tpl.repos[3].entry_str(), "/local/shared-lib");
        assert!(tpl.repos[3].is_default_included());

        assert_eq!(tpl.repos[4].entry_str(), "extra-tools");
        assert!(!tpl.repos[4].is_default_included());

        assert_eq!(
            tpl.repo_names(),
            vec![
                "service-api",
                "web-frontend",
                "docs-site",
                "/local/shared-lib",
                "extra-tools"
            ]
        );
        assert_eq!(
            tpl.default_repo_names(),
            vec!["service-api", "web-frontend", "/local/shared-lib"]
        );
    }

    #[test]
    fn test_resolve_template_repos_with_default_inclusion() {
        let toml_str = r#"
default_agent = "claude"
repos_dir = "/tmp/repos"

[repos.registered-service]
url = "https://github.com/example/registered-service.git"

[templates.multi]
repos = [
    "registered-service",
    { repo = "docs", default = false },
    { name = "analytics", default = true },
    { repo = "https://github.com/example/extra.git", default = false }
]
"#;
        let config = Config::load_from_str(toml_str).expect("Config should parse");
        let resolved = config
            .resolve_template_repos("multi")
            .expect("Template repos should resolve cleanly");

        assert_eq!(resolved.len(), 4);

        assert_eq!(resolved[0].name, "registered-service");
        assert!(resolved[0].default_include);

        assert_eq!(resolved[1].name, "docs");
        assert!(!resolved[1].default_include);

        assert_eq!(resolved[2].name, "analytics");
        assert!(resolved[2].default_include);

        assert_eq!(resolved[3].name, "extra");
        assert!(!resolved[3].default_include);
    }

    #[test]
    fn test_template_repo_entry_conversions_and_display() {
        let entry1: TemplateRepoEntry = "my-repo".into();
        assert_eq!(entry1.entry_str(), "my-repo");
        assert!(entry1.is_default_included());
        assert_eq!(format!("{entry1}"), "my-repo");
        assert_eq!(entry1.as_ref(), "my-repo");
        assert_eq!(entry1, "my-repo");
        assert_eq!("my-repo", entry1);
        assert_eq!(entry1, "my-repo".to_string());
        assert_eq!("my-repo".to_string(), entry1);

        let entry2: TemplateRepoEntry = "another-repo".to_string().into();
        assert_eq!(entry2.entry_str(), "another-repo");
        assert_eq!(entry2, "another-repo");

        let entry3 = TemplateRepoEntry::Detailed {
            repo: "opt-in".to_string(),
            default: false,
        };
        assert_eq!(entry3.entry_str(), "opt-in");
        assert!(!entry3.is_default_included());
        assert_eq!(format!("{entry3}"), "opt-in");
        assert_eq!(entry3, "opt-in");
    }
}
