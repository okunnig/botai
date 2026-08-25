use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use thiserror::Error;

use crate::config::{Config, ConfigError, ResolvedRepo};
use crate::context::{generate_agents_md, ContextError, ProjectContext, RepoMapping};
use crate::git::{create_worktree, remove_worktree_auto, GitError, GitWorktreeManager, WorktreeManager};
use crate::session::{ProjectSession, PtyConfig, PtySession, SessionError, VtScreen};

/// Errors that can occur during project creation, validation, or rollback.
#[derive(Debug, Error)]
pub enum ProjectError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Git error: {0}")]
    Git(#[from] GitError),

    #[error("Config error: {0}")]
    Config(#[from] ConfigError),

    #[error("Context error: {0}")]
    Context(#[from] ContextError),

    #[error("Session error: {0}")]
    Session(#[from] SessionError),

    #[error("Project directory '{0}' already exists")]
    DirectoryExists(PathBuf),

    #[error("Project name cannot be empty")]
    EmptyProjectName,

    #[error("Template '{0}' not found in configuration")]
    TemplateNotFound(String),

    #[error("Agent '{0}' not found in configuration")]
    AgentNotFound(String),

    #[error("Branch '{branch}' already exists in repository '{repo}'")]
    BranchExists { repo: PathBuf, branch: String },

    #[error("Repository '{name}' does not exist locally and has no remote URL configured")]
    MissingRepository { name: String, path: PathBuf },

    #[error("General project error: {0}")]
    Other(String),
}

/// Structured events emitted during the project creation pipeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectSetupEvent {
    Step {
        current: usize,
        total: usize,
        message: String,
    },
    Log(String),
    Success,
    Error(String),
}

/// Detailed pre-validation result for project creation inputs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectValidation {
    pub is_valid: bool,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

impl ProjectValidation {
    /// Creates a valid validation outcome with no errors or warnings.
    pub fn valid() -> Self {
        Self {
            is_valid: true,
            errors: Vec::new(),
            warnings: Vec::new(),
        }
    }

    /// Creates an invalid validation outcome with a single error message.
    pub fn invalid(err: impl Into<String>) -> Self {
        Self {
            is_valid: false,
            errors: vec![err.into()],
            warnings: Vec::new(),
        }
    }

    /// Appends an error message and marks the validation outcome as invalid.
    pub fn add_error(&mut self, err: impl Into<String>) {
        self.is_valid = false;
        self.errors.push(err.into());
    }

    /// Appends a warning message without necessarily marking the validation as invalid.
    pub fn add_warning(&mut self, warn: impl Into<String>) {
        self.warnings.push(warn.into());
    }

    /// Returns the primary error or warning message for single-line display.
    pub fn primary_message(&self) -> Option<&str> {
        self.errors.first().or_else(|| self.warnings.first()).map(|s| s.as_str())
    }
}

/// Unified builder pipeline for multi-repo project creation, worktree orchestration,
/// context generation, and PTY process spawning.
#[derive(Debug, Clone)]
pub struct ProjectBuilder {
    pub config: Config,
    pub template_name: String,
    pub project_name: String,
    pub agent_override: Option<String>,
    pub prompt: Option<String>,
    pub cwd: PathBuf,
    pub viewport_size: (u16, u16),
}

impl ProjectBuilder {
    /// Creates a new `ProjectBuilder` with default working directory and terminal dimensions.
    pub fn new(
        config: Config,
        template_name: impl Into<String>,
        project_name: impl Into<String>,
    ) -> Self {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        Self {
            config,
            template_name: template_name.into(),
            project_name: project_name.into(),
            agent_override: None,
            prompt: None,
            cwd,
            viewport_size: (100, 30),
        }
    }

    /// Sets the AI agent CLI tool override.
    pub fn with_agent(mut self, agent: impl Into<String>) -> Self {
        let a = agent.into();
        self.agent_override = if a.trim().is_empty() { None } else { Some(a) };
        self
    }

    /// Sets an optional initial task prompt.
    pub fn with_prompt(mut self, prompt: impl Into<String>) -> Self {
        let p = prompt.into();
        self.prompt = if p.trim().is_empty() { None } else { Some(p) };
        self
    }

    /// Sets the current working directory where the project directory will be created.
    pub fn with_cwd(mut self, cwd: impl Into<PathBuf>) -> Self {
        self.cwd = cwd.into();
        self
    }

    /// Sets the pseudo-terminal and virtual terminal viewport dimensions (cols, rows).
    pub fn with_viewport(mut self, cols: u16, rows: u16) -> Self {
        self.viewport_size = (cols.max(1), rows.max(1));
        self
    }

    /// Resolves the effective agent name for this project.
    pub fn resolved_agent_name(&self) -> String {
        if let Some(ref override_name) = self.agent_override {
            return override_name.clone();
        }
        if let Some(tpl) = self.config.get_template(&self.template_name) {
            if let Some(ref tpl_agent) = tpl.default_agent {
                return tpl_agent.clone();
            }
        }
        self.config.default_agent.clone()
    }

    /// Resolves the command, arguments, and environment variables for the selected agent tool.
    pub fn resolve_agent_command(&self) -> (String, Vec<String>, HashMap<String, String>) {
        let agent_name = self.resolved_agent_name();
        if let Some(agent) = self.config.get_agent(&agent_name) {
            (agent.command.clone(), agent.args.clone(), agent.env.clone())
        } else {
            #[cfg(windows)]
            let default_shell = "cmd.exe".to_string();
            #[cfg(not(windows))]
            let default_shell = "sh".to_string();
            (default_shell, Vec::new(), HashMap::new())
        }
    }

    /// Validates preconditions for project creation:
    /// - Non-empty project name
    /// - Template existence in configuration
    /// - Project directory does not already exist in CWD
    /// - Branch `<project_name>` does not collide in any locally existing template repositories
    pub fn validate(&self) -> Result<(), ProjectError> {
        let trimmed_name = self.project_name.trim();
        if trimmed_name.is_empty() {
            return Err(ProjectError::EmptyProjectName);
        }

        if !self.config.templates.contains_key(&self.template_name) {
            return Err(ProjectError::TemplateNotFound(self.template_name.clone()));
        }

        let project_dir = self.cwd.join(trimmed_name);
        if project_dir.exists() {
            return Err(ProjectError::DirectoryExists(project_dir));
        }

        let resolved_repos = self.config.resolve_template_repos(&self.template_name)?;
        let git = GitWorktreeManager::new();

        for repo in resolved_repos {
            if repo.exists_locally() {
                if let Ok(exists) = git.branch_exists(&repo.local_path, trimmed_name) {
                    if exists {
                        return Err(ProjectError::BranchExists {
                            repo: repo.local_path,
                            branch: trimmed_name.to_string(),
                        });
                    }
                }
            }
        }

        Ok(())
    }

    /// Executes detailed pre-validation returning structured errors and warnings for TUI display.
    pub fn validate_detailed(&self) -> ProjectValidation {
        let mut validation = ProjectValidation::valid();
        let trimmed_name = self.project_name.trim();

        if trimmed_name.is_empty() {
            validation.is_valid = false;
            validation.add_warning("Project name cannot be empty");
            return validation;
        }

        if !self.config.templates.contains_key(&self.template_name) {
            validation.add_error(format!("Template '{}' not found in config", self.template_name));
            return validation;
        }

        let project_dir = self.cwd.join(trimmed_name);
        if project_dir.exists() {
            validation.add_error(format!(
                "Directory '{trimmed_name}' already exists in current directory"
            ));
            return validation;
        }

        match self.config.resolve_template_repos(&self.template_name) {
            Ok(resolved_repos) => {
                let git = GitWorktreeManager::new();
                for repo in resolved_repos {
                    if repo.exists_locally() {
                        if let Ok(exists) = git.branch_exists(&repo.local_path, trimmed_name) {
                            if exists {
                                validation.add_error(format!(
                                    "Branch '{}' already exists in repository '{}'",
                                    trimmed_name, repo.name
                                ));
                                return validation;
                            }
                        }
                    }
                }
            }
            Err(e) => {
                validation.add_error(format!("Repository resolution error: {e}"));
                return validation;
            }
        }

        validation
    }

    /// Executes workspace initialization steps 1-4 without spawning an AI agent PTY session:
    /// 1. Pre-validation & repo resolution
    /// 2. Auto-cloning missing repositories & upstream fetching
    /// 3. Project directory creation & git worktree checkout
    /// 4. `AGENTS.md` context synthesis
    ///
    /// Returns the canonical project root directory and repository mappings.
    /// On any error, triggers atomic rollback by removing any created worktrees
    /// with `remove_worktree_auto` and deleting the project directory.
    pub fn create_workspace<F>(&self, progress_sink: F) -> anyhow::Result<(PathBuf, Vec<RepoMapping>)>
    where
        F: Fn(ProjectSetupEvent),
    {
        let res = (|| -> anyhow::Result<(PathBuf, Vec<RepoMapping>)> {
            // Step 1: Pre-validation & resolving repositories
            progress_sink(ProjectSetupEvent::Step {
                current: 1,
                total: 4,
                message: format!("Checking repositories for template '{}'...", self.template_name),
            });
            progress_sink(ProjectSetupEvent::Log(format!(
                "[1/4] Checking repositories for template '{}'...",
                self.template_name
            )));

            self.validate()?;

            let tpl = self
                .config
                .get_template(&self.template_name)
                .ok_or_else(|| ProjectError::TemplateNotFound(self.template_name.clone()))?;

            let resolved_repos: Vec<ResolvedRepo> = self
                .config
                .resolve_template_repos(&self.template_name)?;

            // Step 2: Auto-cloning missing repos & fetching updates
            progress_sink(ProjectSetupEvent::Step {
                current: 2,
                total: 4,
                message: "Cloning and updating repositories...".to_string(),
            });

            for repo in &resolved_repos {
                if !repo.exists_locally() {
                    if let Some(url) = &repo.clone_url {
                        progress_sink(ProjectSetupEvent::Log(format!(
                            "[2/4] Cloning repository '{}' from {}...",
                            repo.name, url
                        )));
                        crate::git::clone_repo(url, &repo.local_path)?;
                    } else {
                        anyhow::bail!(
                            "Repository '{}' does not exist locally and has no remote URL configured",
                            repo.name
                        );
                    }
                } else {
                    progress_sink(ProjectSetupEvent::Log(format!(
                        "[2/4] Fetching latest updates for repository '{}'...",
                        repo.name
                    )));
                    let _ = crate::git::fetch_repo(&repo.local_path);
                }
            }

            // Step 3: Creating project directory and git worktrees
            progress_sink(ProjectSetupEvent::Step {
                current: 3,
                total: 4,
                message: "Creating project directory and git worktrees...".to_string(),
            });

            let trimmed_project_name = self.project_name.trim();
            let project_dir = self.cwd.join(trimmed_project_name);
            fs::create_dir_all(&project_dir)?;
            let project_dir_canon = project_dir
                .canonicalize()
                .unwrap_or_else(|_| project_dir.clone());

            let mut created_mappings: Vec<RepoMapping> = Vec::new();
            for repo in &resolved_repos {
                let worktree_path = project_dir_canon.join(&repo.name);
                progress_sink(ProjectSetupEvent::Log(format!(
                    "[3/4] Creating git worktree for '{}' at {}...",
                    repo.name,
                    worktree_path.display()
                )));

                if let Err(err) = create_worktree(&repo.local_path, &worktree_path, trimmed_project_name) {
                    // Rollback partially created worktrees & project_dir
                    for mapping in &created_mappings {
                        let _ = remove_worktree_auto(&mapping.worktree_path, true);
                    }
                    let _ = fs::remove_dir_all(&project_dir);
                    anyhow::bail!("Failed to create worktree for '{}': {}", repo.name, err);
                }

                created_mappings.push(RepoMapping::new(
                    repo.name.clone(),
                    repo.local_path.clone(),
                    worktree_path,
                ));
            }

            // Step 4: Generating AGENTS.md context
            progress_sink(ProjectSetupEvent::Step {
                current: 4,
                total: 4,
                message: "Generating AGENTS.md context orientation file...".to_string(),
            });
            progress_sink(ProjectSetupEvent::Log(
                "[4/4] Generating AGENTS.md context orientation file...".to_string(),
            ));

            let project_context = ProjectContext {
                project_name: trimmed_project_name.to_string(),
                template_name: self.template_name.clone(),
                description: tpl.description.clone(),
                agents_md_append: tpl.agents_md_append.clone(),
                repos: created_mappings.clone(),
                env: tpl.env.clone(),
                initial_prompt: self.prompt.clone(),
            };

            if let Err(err) = generate_agents_md(&project_dir_canon, &project_context) {
                for mapping in &created_mappings {
                    let _ = remove_worktree_auto(&mapping.worktree_path, true);
                }
                let _ = fs::remove_dir_all(&project_dir);
                anyhow::bail!("Failed to generate AGENTS.md: {err}");
            }

            Ok((project_dir_canon, created_mappings))
        })();

        match res {
            Ok(val) => Ok(val),
            Err(err) => {
                let err_msg = err.to_string();
                progress_sink(ProjectSetupEvent::Log(format!("✗ Error: {err_msg}")));
                progress_sink(ProjectSetupEvent::Error(err_msg));
                Err(err)
            }
        }
    }

    /// Executes the full end-to-end project creation pipeline:
    /// 1. Pre-validation & repo resolution
    /// 2. Auto-cloning missing repositories & upstream fetching
    /// 3. Project directory creation & git worktree checkout
    /// 4. `AGENTS.md` context synthesis
    /// 5. PTY process spawning & virtual screen initialization
    ///
    /// On any error during steps 1-5, triggers atomic rollback by removing any created
    /// worktrees with `remove_worktree_auto` and deleting the project directory.
    pub fn build<F>(&self, progress_sink: F) -> anyhow::Result<(ProjectSession, PtySession, VtScreen)>
    where
        F: Fn(ProjectSetupEvent) + Send + Sync + 'static,
    {
        let (project_dir_canon, created_mappings) = self.create_workspace(&progress_sink)?;

        let tpl = self
            .config
            .get_template(&self.template_name)
            .ok_or_else(|| ProjectError::TemplateNotFound(self.template_name.clone()))?;

        let agent_name = self.resolved_agent_name();
        let (agent_cmd, agent_args, agent_env) = self.resolve_agent_command();

        // Step 5: Spawning agent CLI in pseudo-terminal
        let (cols, rows) = self.viewport_size;
        progress_sink(ProjectSetupEvent::Log(format!(
            "Spawning AI agent CLI '{agent_cmd}' in pseudo-terminal ({cols}x{rows})..."
        )));

        let mut pty_config = PtyConfig::new(&agent_cmd, &project_dir_canon)
            .with_args(agent_args)
            .with_dimensions(cols, rows);

        for (k, v) in &agent_env {
            pty_config = pty_config.with_env(k, v);
        }
        for (k, v) in &tpl.env {
            pty_config = pty_config.with_env(k, v);
        }

        let pty = match PtySession::spawn(pty_config) {
            Ok(p) => p,
            Err(err) => {
                for mapping in &created_mappings {
                    let _ = remove_worktree_auto(&mapping.worktree_path, true);
                }
                let _ = fs::remove_dir_all(&project_dir_canon);
                let err_msg = format!("Failed to spawn agent process '{agent_cmd}': {err}");
                progress_sink(ProjectSetupEvent::Log(format!("✗ Error: {err_msg}")));
                progress_sink(ProjectSetupEvent::Error(err_msg.clone()));
                anyhow::bail!("{err_msg}");
            }
        };

        let vt = VtScreen::new(cols, rows);
        let trimmed_project_name = self.project_name.trim();
        let mut ps = ProjectSession::new(trimmed_project_name, &self.template_name, &agent_name);
        if let Some(ref p) = self.prompt {
            ps = ps.with_prompt(p);
        }

        progress_sink(ProjectSetupEvent::Log(
            "✓ Project setup completed successfully!".to_string(),
        ));
        progress_sink(ProjectSetupEvent::Success);

        Ok((ps, pty, vt))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::sync::{Arc, Mutex};
    use tempfile::TempDir;

    fn create_mock_git_repo(path: &Path) {
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
        fs::write(path.join("README.md"), "# Test Repo\n").unwrap();
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
    fn test_project_builder_validation_empty_name() {
        let config = Config::load_from_str("default_agent = \"claude\"\nrepos_dir = \".repos\"\n").unwrap();
        let builder = ProjectBuilder::new(config, "ecommerce", "   ");
        let val = builder.validate_detailed();
        assert!(!val.is_valid);
        assert!(val.primary_message().unwrap().contains("empty"));
    }

    #[test]
    fn test_project_builder_validation_template_not_found() {
        let config = Config::load_from_str("default_agent = \"claude\"\nrepos_dir = \".repos\"\n").unwrap();
        let builder = ProjectBuilder::new(config, "nonexistent", "my-project");
        let val = builder.validate_detailed();
        assert!(!val.is_valid);
        assert!(val.primary_message().unwrap().contains("not found"));
    }

    #[test]
    fn test_project_builder_validation_directory_collision() {
        let temp_dir = TempDir::new().unwrap();
        let cwd = temp_dir.path();
        fs::create_dir_all(cwd.join("existing-proj")).unwrap();

        let toml_str = r#"
            default_agent = "claude"
            repos_dir = ".repos"
            [templates.ecommerce]
            repos = []
        "#;
        let config = Config::load_from_str(toml_str).unwrap();
        let builder = ProjectBuilder::new(config, "ecommerce", "existing-proj").with_cwd(cwd);

        let val = builder.validate_detailed();
        assert!(!val.is_valid);
        assert!(val.primary_message().unwrap().contains("already exists"));
    }

    #[test]
    fn test_project_builder_validation_branch_collision() {
        let temp_dir = TempDir::new().unwrap();
        let repo_dir = temp_dir.path().join("my_service");
        create_mock_git_repo(&repo_dir);

        // Create branch "collision-branch"
        let _ = std::process::Command::new("git")
            .args(["branch", "collision-branch"])
            .current_dir(&repo_dir)
            .status();

        let toml_str = format!(
            r#"
            default_agent = "claude"
            repos_dir = "{}"
            [templates.service_tpl]
            repos = ["{}"]
            "#,
            temp_dir.path().display(),
            repo_dir.display()
        );
        let config = Config::load_from_str(&toml_str).unwrap();
        let builder = ProjectBuilder::new(config, "service_tpl", "collision-branch")
            .with_cwd(temp_dir.path());

        let val = builder.validate_detailed();
        assert!(!val.is_valid);
        assert!(val.primary_message().unwrap().contains("already exists in repository"));
    }

    #[test]
    fn test_project_builder_create_workspace_success() {
        let temp_dir = TempDir::new().unwrap();
        let repo_dir = temp_dir.path().join("backend");
        create_mock_git_repo(&repo_dir);

        let toml_str = format!(
            r#"
            default_agent = "claude"
            repos_dir = "{}"
            [templates.web_app]
            description = "Web App Services"
            repos = ["{}"]
            "#,
            temp_dir.path().display(),
            repo_dir.display()
        );
        let config = Config::load_from_str(&toml_str).unwrap();
        let project_cwd = temp_dir.path().join("workspaces");
        fs::create_dir_all(&project_cwd).unwrap();

        let builder = ProjectBuilder::new(config, "web_app", "checkout-feat")
            .with_cwd(&project_cwd)
            .with_prompt("Build payment integration");

        let events = Arc::new(Mutex::new(Vec::new()));
        let events_clone = Arc::clone(&events);

        let res = builder.create_workspace(move |event| {
            events_clone.lock().unwrap().push(event);
        });

        assert!(res.is_ok(), "create_workspace should succeed: {:?}", res.err());
        let (project_dir_canon, mappings) = res.unwrap();
        let expected_project_dir = project_cwd.join("checkout-feat").canonicalize().unwrap();
        assert_eq!(project_dir_canon, expected_project_dir);
        assert_eq!(mappings.len(), 1);
        assert_eq!(mappings[0].name, "backend");
        assert_eq!(mappings[0].worktree_path, expected_project_dir.join("backend"));

        assert!(expected_project_dir.exists());
        assert!(expected_project_dir.join("AGENTS.md").exists());
        assert!(expected_project_dir.join("backend").exists());
        assert!(expected_project_dir.join("backend").join("README.md").exists());

        let ev_list = events.lock().unwrap();
        assert!(ev_list.iter().any(|e| matches!(e, ProjectSetupEvent::Step { current: 1, .. })));
        assert!(ev_list.iter().any(|e| matches!(e, ProjectSetupEvent::Step { current: 4, .. })));
    }

    #[test]
    fn test_project_builder_create_workspace_rollback_on_failure() {
        let temp_dir = TempDir::new().unwrap();
        let repo_dir = temp_dir.path().join("failing_repo");
        create_mock_git_repo(&repo_dir);

        // Pre-create branch so create_worktree fails
        let _ = std::process::Command::new("git")
            .args(["branch", "clashing_project"])
            .current_dir(&repo_dir)
            .status();

        let toml_str = format!(
            r#"
            default_agent = "claude"
            repos_dir = "{}"
            [templates.failing_tpl]
            repos = ["{}"]
            "#,
            temp_dir.path().display(),
            repo_dir.display()
        );
        let config = Config::load_from_str(&toml_str).unwrap();
        let project_cwd = temp_dir.path().join("workspaces");
        fs::create_dir_all(&project_cwd).unwrap();

        let builder = ProjectBuilder::new(config, "failing_tpl", "clashing_project")
            .with_cwd(&project_cwd);

        let events = Arc::new(Mutex::new(Vec::new()));
        let events_clone = Arc::clone(&events);

        let res = builder.create_workspace(move |event| {
            events_clone.lock().unwrap().push(event);
        });

        assert!(res.is_err());
        let project_dir = project_cwd.join("clashing_project");
        assert!(!project_dir.exists(), "project_dir must be cleaned up on rollback");

        let ev_list = events.lock().unwrap();
        assert!(ev_list.iter().any(|e| matches!(e, ProjectSetupEvent::Error(_))));
    }

    #[test]
    fn test_project_builder_build_pipeline_success() {
        let temp_dir = TempDir::new().unwrap();
        let repo_dir = temp_dir.path().join("backend");
        create_mock_git_repo(&repo_dir);

        let toml_str = format!(
            r#"
            default_agent = "claude"
            repos_dir = "{}"
            [agents.claude]
            command = "echo"
            args = ["agent running"]
            [templates.web_app]
            description = "Web App Services"
            repos = ["{}"]
            "#,
            temp_dir.path().display(),
            repo_dir.display()
        );
        let config = Config::load_from_str(&toml_str).unwrap();
        let project_cwd = temp_dir.path().join("workspaces");
        fs::create_dir_all(&project_cwd).unwrap();

        let builder = ProjectBuilder::new(config, "web_app", "checkout-feat")
            .with_cwd(&project_cwd)
            .with_prompt("Build payment integration");

        let events = Arc::new(Mutex::new(Vec::new()));
        let events_clone = Arc::clone(&events);

        let res = builder.build(move |event| {
            events_clone.lock().unwrap().push(event);
        });

        assert!(res.is_ok(), "build should succeed: {:?}", res.err());
        let (ps, mut pty, vt) = res.unwrap();
        assert_eq!(ps.project_name, "checkout-feat");
        assert_eq!(ps.template_name, "web_app");
        assert_eq!(ps.agent_name, "claude");
        assert_eq!(ps.prompt.as_deref(), Some("Build payment integration"));
        assert!(pty.is_alive() || pty.try_wait().is_ok());
        assert_eq!(vt.size(), (100, 30));

        let project_dir = project_cwd.join("checkout-feat");
        assert!(project_dir.exists());
        assert!(project_dir.join("AGENTS.md").exists());
        assert!(project_dir.join("backend").exists());
        assert!(project_dir.join("backend").join("README.md").exists());

        let ev_list = events.lock().unwrap();
        assert!(ev_list.iter().any(|e| matches!(e, ProjectSetupEvent::Success)));
        assert!(ev_list.iter().any(|e| matches!(e, ProjectSetupEvent::Step { current: 1, .. })));
        assert!(ev_list.iter().any(|e| matches!(e, ProjectSetupEvent::Step { current: 4, .. })));
    }

    #[test]
    fn test_project_builder_build_pipeline_rollback_on_worktree_failure() {
        let temp_dir = TempDir::new().unwrap();
        let repo_dir = temp_dir.path().join("failing_repo");
        create_mock_git_repo(&repo_dir);

        // Pre-create branch so create_worktree fails
        let _ = std::process::Command::new("git")
            .args(["branch", "clashing_project"])
            .current_dir(&repo_dir)
            .status();

        let toml_str = format!(
            r#"
            default_agent = "claude"
            repos_dir = "{}"
            [templates.failing_tpl]
            repos = ["{}"]
            "#,
            temp_dir.path().display(),
            repo_dir.display()
        );
        let config = Config::load_from_str(&toml_str).unwrap();
        let project_cwd = temp_dir.path().join("workspaces");
        fs::create_dir_all(&project_cwd).unwrap();

        let builder = ProjectBuilder::new(config, "failing_tpl", "clashing_project")
            .with_cwd(&project_cwd);

        let events = Arc::new(Mutex::new(Vec::new()));
        let events_clone = Arc::clone(&events);

        let res = builder.build(move |event| {
            events_clone.lock().unwrap().push(event);
        });

        assert!(res.is_err());
        let project_dir = project_cwd.join("clashing_project");
        assert!(!project_dir.exists(), "project_dir must be cleaned up on rollback");

        let ev_list = events.lock().unwrap();
        assert!(ev_list.iter().any(|e| matches!(e, ProjectSetupEvent::Error(_))));
    }
}
