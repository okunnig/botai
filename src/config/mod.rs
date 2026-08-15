use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Errors that can occur during configuration loading, parsing, and resolution.
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Failed to parse config TOML: {0}")]
    Parse(#[from] toml::de::Error),

    #[error("Config file not found at: {0}")]
    NotFound(PathBuf),

    #[error("Workspace '{0}' not found in configuration")]
    WorkspaceNotFound(String),

    #[error("Agent '{0}' not found in configuration")]
    AgentNotFound(String),

    #[error("Could not determine config directory")]
    ConfigDirNotFound,
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

/// Configuration for a workspace containing multiple repositories.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceConfig {
    /// Optional human-readable description of the workspace.
    #[serde(default)]
    pub description: Option<String>,
    /// Optional workspace-specific default agent override.
    #[serde(default)]
    pub default_agent: Option<String>,
    /// List of local repository paths belonging to this workspace.
    pub repos: Vec<PathBuf>,
    /// Per-workspace environment variables injected into the agent environment.
    #[serde(default)]
    pub env: HashMap<String, String>,
}

/// Root configuration for botai.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    /// Global fallback default agent name.
    pub default_agent: String,
    /// Defined agent tools.
    #[serde(default)]
    pub agents: HashMap<String, AgentConfig>,
    /// Defined workspaces.
    #[serde(default)]
    pub workspaces: HashMap<String, WorkspaceConfig>,
}

impl Config {
    /// Parses configuration from a TOML string.
    pub fn load_from_str(s: &str) -> Result<Self, ConfigError> {
        let config: Config = toml::from_str(s)?;
        Ok(config)
    }

    /// Loads configuration from a file path.
    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Self, ConfigError> {
        let content = std::fs::read_to_string(path)?;
        Self::load_from_str(&content)
    }

    /// Resolves the default configuration file path based on platform standards:
    /// - Unix / macOS: `~/.config/botai/config.toml` (or `$XDG_CONFIG_HOME/botai/config.toml`)
    /// - Windows: `%APPDATA%\botai\config.toml`
    pub fn default_path() -> Option<PathBuf> {
        directories::BaseDirs::new().map(|dirs| dirs.config_dir().join("botai").join("config.toml"))
    }

    /// Retrieves a workspace configuration by name.
    pub fn get_workspace(&self, name: &str) -> Option<&WorkspaceConfig> {
        self.workspaces.get(name)
    }

    /// Retrieves an agent configuration by name.
    pub fn get_agent(&self, name: &str) -> Option<&AgentConfig> {
        self.agents.get(name)
    }

    /// Resolves the effective agent configuration for a given workspace.
    ///
    /// If the workspace specifies a `default_agent`, that agent is looked up.
    /// Otherwise, falls back to the global `default_agent`.
    ///
    /// Returns `ConfigError::WorkspaceNotFound` if the workspace does not exist.
    /// Returns `ConfigError::AgentNotFound` if the target agent definition does not exist.
    pub fn resolve_agent_for_workspace(
        &self,
        workspace_name: &str,
    ) -> Result<&AgentConfig, ConfigError> {
        let workspace = self
            .get_workspace(workspace_name)
            .ok_or_else(|| ConfigError::WorkspaceNotFound(workspace_name.to_string()))?;

        let agent_name = workspace
            .default_agent
            .as_deref()
            .unwrap_or(&self.default_agent);

        self.get_agent(agent_name)
            .ok_or_else(|| ConfigError::AgentNotFound(agent_name.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    const SAMPLE_VALID_TOML: &str = r#"
default_agent = "claude"

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

[workspaces.ecommerce]
description = "E-commerce platform services"
default_agent = "agy"
repos = [
    "/home/user/code/ecommerce-api",
    "/home/user/code/ecommerce-web",
    "/home/user/code/ecommerce-shared"
]
env = { PORT = "8080", NODE_ENV = "development" }

[workspaces.analytics]
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
        assert_eq!(config.agents.len(), 3);
        assert_eq!(config.workspaces.len(), 2);

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

        // Verify workspace 'ecommerce'
        let ecommerce = config
            .get_workspace("ecommerce")
            .expect("Workspace 'ecommerce' should exist");
        assert_eq!(
            ecommerce.description.as_deref(),
            Some("E-commerce platform services")
        );
        assert_eq!(ecommerce.default_agent.as_deref(), Some("agy"));
        assert_eq!(
            ecommerce.repos,
            vec![
                PathBuf::from("/home/user/code/ecommerce-api"),
                PathBuf::from("/home/user/code/ecommerce-web"),
                PathBuf::from("/home/user/code/ecommerce-shared"),
            ]
        );
        assert_eq!(ecommerce.env.get("PORT"), Some(&"8080".to_string()));
        assert_eq!(
            ecommerce.env.get("NODE_ENV"),
            Some(&"development".to_string())
        );

        // Verify workspace 'analytics' (omits default_agent and env)
        let analytics = config
            .get_workspace("analytics")
            .expect("Workspace 'analytics' should exist");
        assert_eq!(analytics.description.as_deref(), Some("Analytics pipeline"));
        assert_eq!(analytics.default_agent, None);
        assert_eq!(
            analytics.repos,
            vec![PathBuf::from("/home/user/code/analytics-core")]
        );
        assert!(analytics.env.is_empty());
    }

    #[test]
    fn test_parse_minimal_valid_config() {
        let minimal_toml = r#"
default_agent = "basic"

[agents.basic]
command = "echo"

[workspaces.mini]
repos = ["/tmp/repo"]
"#;
        let config = Config::load_from_str(minimal_toml)
            .expect("Minimal TOML should parse successfully with defaults");

        assert_eq!(config.default_agent, "basic");

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

        let workspace = config
            .get_workspace("mini")
            .expect("Workspace 'mini' must exist");
        assert_eq!(
            workspace.description, None,
            "Omitted description should be None"
        );
        assert_eq!(
            workspace.default_agent, None,
            "Omitted default_agent should be None"
        );
        assert_eq!(workspace.repos, vec![PathBuf::from("/tmp/repo")]);
        assert!(
            workspace.env.is_empty(),
            "Omitted env should default to empty map"
        );
    }

    #[test]
    fn test_get_workspace_and_agent_not_found() {
        let config = Config::load_from_str(SAMPLE_VALID_TOML).expect("Config should parse");

        assert!(config.get_workspace("non_existent_workspace").is_none());
        assert!(config.get_agent("non_existent_agent").is_none());
    }

    #[test]
    fn test_resolve_agent_for_workspace_explicit_override() {
        let config = Config::load_from_str(SAMPLE_VALID_TOML).expect("Config should parse");

        // ecommerce workspace specifies default_agent = "agy"
        let resolved_agent = config
            .resolve_agent_for_workspace("ecommerce")
            .expect("Should resolve explicit agent 'agy' for workspace 'ecommerce'");
        assert_eq!(resolved_agent.command, "agy");
        assert_eq!(resolved_agent.args, vec!["--verbose".to_string()]);
    }

    #[test]
    fn test_resolve_agent_for_workspace_fallback_to_global() {
        let config = Config::load_from_str(SAMPLE_VALID_TOML).expect("Config should parse");

        // analytics workspace does not specify default_agent, should fall back to global default "claude"
        let resolved_agent = config
            .resolve_agent_for_workspace("analytics")
            .expect("Should fallback to global agent 'claude' for workspace 'analytics'");
        assert_eq!(resolved_agent.command, "claude");
    }

    #[test]
    fn test_resolve_agent_for_workspace_workspace_not_found() {
        let config = Config::load_from_str(SAMPLE_VALID_TOML).expect("Config should parse");

        let err = config
            .resolve_agent_for_workspace("ghost_workspace")
            .expect_err("Should return error for missing workspace");

        match err {
            ConfigError::WorkspaceNotFound(name) => assert_eq!(name, "ghost_workspace"),
            other => panic!("Expected ConfigError::WorkspaceNotFound, got {:?}", other),
        }
    }

    #[test]
    fn test_resolve_agent_for_workspace_agent_not_found_on_workspace_override() {
        let toml = r#"
default_agent = "claude"

[agents.claude]
command = "claude"

[workspaces.broken]
default_agent = "nonexistent_agent"
repos = ["/path/to/repo"]
"#;
        let config = Config::load_from_str(toml).expect("Config should parse");

        let err = config
            .resolve_agent_for_workspace("broken")
            .expect_err("Should return error when workspace references missing agent");

        match err {
            ConfigError::AgentNotFound(agent) => assert_eq!(agent, "nonexistent_agent"),
            other => panic!("Expected ConfigError::AgentNotFound, got {:?}", other),
        }
    }

    #[test]
    fn test_resolve_agent_for_workspace_agent_not_found_on_global_fallback() {
        let toml = r#"
default_agent = "missing_global_agent"

[agents.other]
command = "other"

[workspaces.test_ws]
repos = ["/path/to/repo"]
"#;
        let config = Config::load_from_str(toml).expect("Config should parse");

        let err = config
            .resolve_agent_for_workspace("test_ws")
            .expect_err("Should return error when global default agent is not in agents table");

        match err {
            ConfigError::AgentNotFound(agent) => assert_eq!(agent, "missing_global_agent"),
            other => panic!("Expected ConfigError::AgentNotFound, got {:?}", other),
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
            "Expected default config path to end with 'botai/config.toml', got: {:?}",
            path
        );

        #[cfg(unix)]
        {
            let path_str = path.to_string_lossy();
            assert!(
                path_str.contains(".config/botai/config.toml")
                    || path_str.contains("botai/config.toml"),
                "On Unix, default path should follow XDG or ~/.config conventions: {}",
                path_str
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
    fn test_error_malformed_toml_syntax() {
        let malformed_toml = r#"
default_agent = "claude"
[agents.claude
command = "claude"
"#;
        let err = Config::load_from_str(malformed_toml)
            .expect_err("Malformed TOML syntax should produce a Parse error");

        match err {
            ConfigError::Parse(_) => (),
            other => panic!("Expected ConfigError::Parse, got: {:?}", other),
        }
    }

    #[test]
    fn test_error_missing_required_default_agent() {
        let toml_missing_default_agent = r#"
[agents.claude]
command = "claude"

[workspaces.ecommerce]
repos = ["/path/to/repo"]
"#;
        let err = Config::load_from_str(toml_missing_default_agent)
            .expect_err("Config missing required default_agent field should produce a Parse error");

        match err {
            ConfigError::Parse(_) => (),
            other => panic!("Expected ConfigError::Parse, got: {:?}", other),
        }
    }

    #[test]
    fn test_error_missing_agent_command() {
        let toml_missing_command = r#"
default_agent = "claude"

[agents.claude]
args = ["--some-arg"]

[workspaces.ecommerce]
repos = ["/path/to/repo"]
"#;
        let err = Config::load_from_str(toml_missing_command)
            .expect_err("Agent missing required 'command' field should produce a Parse error");

        match err {
            ConfigError::Parse(_) => (),
            other => panic!("Expected ConfigError::Parse, got: {:?}", other),
        }
    }

    #[test]
    fn test_error_missing_workspace_repos() {
        let toml_missing_repos = r#"
default_agent = "claude"

[agents.claude]
command = "claude"

[workspaces.ecommerce]
description = "Missing repos field"
"#;
        let err = Config::load_from_str(toml_missing_repos)
            .expect_err("Workspace missing required 'repos' field should produce a Parse error");

        match err {
            ConfigError::Parse(_) => (),
            other => panic!("Expected ConfigError::Parse, got: {:?}", other),
        }
    }

    #[test]
    fn test_error_invalid_field_type() {
        let toml_invalid_type = r#"
default_agent = "claude"

[agents.claude]
command = "claude"

[workspaces.ecommerce]
repos = "not-a-list"
"#;
        let err = Config::load_from_str(toml_invalid_type)
            .expect_err("Invalid type for repos should produce a Parse error");

        match err {
            ConfigError::Parse(_) => (),
            other => panic!("Expected ConfigError::Parse, got: {:?}", other),
        }
    }

    #[test]
    fn test_error_empty_string_input() {
        let err = Config::load_from_str("")
            .expect_err("Empty string should produce a Parse error due to missing default_agent");

        match err {
            ConfigError::Parse(_) => (),
            other => panic!("Expected ConfigError::Parse, got: {:?}", other),
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
        assert_eq!(config.workspaces.len(), 2);
    }

    #[test]
    fn test_load_from_file_not_found() {
        let non_existent_path = PathBuf::from("/non/existent/path/to/botai_test_config.toml");
        let err = Config::load_from_file(&non_existent_path)
            .expect_err("Loading from non-existent file should produce an error");

        match err {
            ConfigError::Io(e) => assert_eq!(e.kind(), std::io::ErrorKind::NotFound),
            ConfigError::NotFound(p) => assert_eq!(p, non_existent_path),
            other => panic!(
                "Expected ConfigError::Io or ConfigError::NotFound, got: {:?}",
                other
            ),
        }
    }

    #[test]
    fn test_cross_platform_repo_paths() {
        let toml_paths = r#"
default_agent = "claude"

[agents.claude]
command = "claude"

[workspaces.cross_platform]
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
            .get_workspace("cross_platform")
            .expect("Workspace must exist");
        assert_eq!(ws.repos.len(), 4);
        assert_eq!(ws.repos[0], PathBuf::from(r"C:\Users\User\Projects\repo1"));
        assert_eq!(ws.repos[1], PathBuf::from("/home/user/projects/repo2"));
        assert_eq!(ws.repos[2], PathBuf::from("relative/path/to/repo3"));
        assert_eq!(ws.repos[3], PathBuf::from("./another/relative/repo4"));
    }

    #[test]
    fn test_unicode_in_workspace_and_paths() {
        let toml_unicode = r#"
default_agent = "claude"

[agents.claude]
command = "claude"

[workspaces."проект-тест"]
description = "Тестовый воркспейс с юникодом 🚀"
repos = [
    "/home/пользователь/проекты/тест-репо",
    "/home/user/日本語/リポジトリ"
]
env = { GREETING = "こんにちは" }
"#;
        let config = Config::load_from_str(toml_unicode).expect(
            "Unicode workspace names, descriptions, paths, and env vars should parse cleanly",
        );

        let ws = config
            .get_workspace("проект-тест")
            .expect("Unicode workspace should be found");
        assert_eq!(
            ws.description.as_deref(),
            Some("Тестовый воркспейс с юникодом 🚀")
        );
        assert_eq!(
            ws.repos[0],
            PathBuf::from("/home/пользователь/проекты/тест-репо")
        );
        assert_eq!(ws.repos[1], PathBuf::from("/home/user/日本語/リポジトリ"));
        assert_eq!(ws.env.get("GREETING"), Some(&"こんにちは".to_string()));
    }
}
