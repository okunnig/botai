use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use thiserror::Error;

use crate::config::{Config, ConfigError, ResolvedRepo};
use crate::context::{generate_agents_md, ContextError, ProjectContext, RepoMapping};
use crate::git::{
    create_worktree, remove_worktree_auto, GitError, GitWorktreeManager, WorktreeManager,
};
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

/// Structured events emitted during the project destruction pipeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectDestroyEvent {
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
        self.errors
            .first()
            .or_else(|| self.warnings.first())
            .map(|s| s.as_str())
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

    /// Resolves the effective arguments for the selected agent tool, including adapted prompt arguments.
    pub fn resolve_agent_args(&self) -> Vec<String> {
        let agent_name = self.resolved_agent_name();
        let base_args = if let Some(agent) = self.config.get_agent(&agent_name) {
            agent.args.clone()
        } else {
            Vec::new()
        };
        adapt_agent_args(&agent_name, &base_args, self.prompt.as_deref())
    }

    /// Resolves the command, arguments, and environment variables for the selected agent tool.
    ///
    /// If an initial prompt is configured, agent CLI arguments are adapted automatically
    /// for known agents (`agy`, `aider`, `claude`, `copilot`).
    pub fn resolve_agent_command(&self) -> (String, Vec<String>, HashMap<String, String>) {
        let agent_name = self.resolved_agent_name();
        let (cmd, base_args, env) = if let Some(agent) = self.config.get_agent(&agent_name) {
            (agent.command.clone(), agent.args.clone(), agent.env.clone())
        } else {
            #[cfg(windows)]
            let default_shell = "cmd.exe".to_string();
            #[cfg(not(windows))]
            let default_shell = "sh".to_string();
            (default_shell, Vec::new(), HashMap::new())
        };

        let adapted_args = adapt_agent_args(&agent_name, &base_args, self.prompt.as_deref());
        (cmd, adapted_args, env)
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

    /// Executes fast local pre-validation without disk/git I/O across template repositories.
    /// Checks:
    /// - Non-empty project name
    /// - Template existence in config
    /// - Project directory collision in CWD
    pub fn validate_fast(&self) -> ProjectValidation {
        let mut validation = ProjectValidation::valid();
        let trimmed_name = self.project_name.trim();

        if trimmed_name.is_empty() {
            validation.is_valid = false;
            validation.add_warning("Project name cannot be empty");
            return validation;
        }

        if !self.config.templates.contains_key(&self.template_name) {
            validation.add_error(format!(
                "Template '{}' not found in config",
                self.template_name
            ));
            return validation;
        }

        let project_dir = self.cwd.join(trimmed_name);
        if project_dir.exists() {
            validation.add_error(format!(
                "Directory '{trimmed_name}' already exists in current directory"
            ));
            return validation;
        }

        validation
    }

    /// Checks for git branch collisions across template repositories.
    /// Returns a list of collision error messages.
    pub fn check_branch_collisions(&self) -> Vec<String> {
        let trimmed_name = self.project_name.trim();
        if trimmed_name.is_empty() {
            return Vec::new();
        }

        let mut errors = Vec::new();
        match self.config.resolve_template_repos(&self.template_name) {
            Ok(resolved_repos) => {
                let git = GitWorktreeManager::new();
                for repo in resolved_repos {
                    if repo.exists_locally() {
                        if let Ok(exists) = git.branch_exists(&repo.local_path, trimmed_name) {
                            if exists {
                                errors.push(format!(
                                    "Branch '{}' already exists in repository '{}'",
                                    trimmed_name, repo.name
                                ));
                            }
                        }
                    }
                }
            }
            Err(e) => {
                errors.push(format!("Repository resolution error: {e}"));
            }
        }
        errors
    }

    /// Executes detailed pre-validation returning structured errors and warnings for TUI display.
    /// Combines fast local validation and repository branch collision checking.
    pub fn validate_detailed(&self) -> ProjectValidation {
        let mut validation = self.validate_fast();
        if !validation.is_valid {
            return validation;
        }

        let branch_errors = self.check_branch_collisions();
        for err in branch_errors {
            validation.add_error(err);
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
    pub fn create_workspace<F>(
        &self,
        progress_sink: F,
    ) -> anyhow::Result<(PathBuf, Vec<RepoMapping>)>
    where
        F: Fn(ProjectSetupEvent),
    {
        let res = (|| -> anyhow::Result<(PathBuf, Vec<RepoMapping>)> {
            // Step 1: Pre-validation & resolving repositories
            progress_sink(ProjectSetupEvent::Step {
                current: 1,
                total: 4,
                message: format!(
                    "Checking repositories for template '{}'...",
                    self.template_name
                ),
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

            let resolved_repos: Vec<ResolvedRepo> =
                self.config.resolve_template_repos(&self.template_name)?;

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

                if let Err(err) =
                    create_worktree(&repo.local_path, &worktree_path, trimmed_project_name)
                {
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

            // Step 4: Execute per-repository setup hooks in parallel across worktrees
            let repos_with_hooks: Vec<(&ResolvedRepo, PathBuf)> = resolved_repos
                .iter()
                .filter(|r| !r.setup_hooks.is_empty())
                .map(|r| (r, project_dir_canon.join(&r.name)))
                .collect();

            if !repos_with_hooks.is_empty() {
                let (tx, rx) = std::sync::mpsc::channel();
                std::thread::scope(|s| {
                    for (repo, worktree_path) in &repos_with_hooks {
                        let thread_tx = tx.clone();
                        let tpl_env = &tpl.env;
                        let repo_env = &repo.env;
                        let setup_hooks = &repo.setup_hooks;
                        let repo_name = &repo.name;
                        let wt_path = worktree_path.clone();

                        s.spawn(move || {
                            for cmd in setup_hooks {
                                if cmd.is_empty() {
                                    continue;
                                }
                                let prog = &cmd[0];
                                let args = &cmd[1..];
                                let cmd_display = cmd.join(" ");
                                let _ = thread_tx.send(ProjectSetupEvent::Log(format!(
                                    "Running setup hook for '{repo_name}': {cmd_display}"
                                )));

                                let mut command = std::process::Command::new(prog);
                                command.args(args);
                                command.current_dir(&wt_path);
                                for (k, v) in tpl_env {
                                    command.env(k, v);
                                }
                                for (k, v) in repo_env {
                                    command.env(k, v);
                                }

                                match command.output() {
                                    Ok(output) => {
                                        if !output.status.success() {
                                            let exit_code = output
                                                .status
                                                .code()
                                                .map(|c| c.to_string())
                                                .unwrap_or_else(|| "signal".to_string());
                                            let stderr_snippet =
                                                String::from_utf8_lossy(&output.stderr);
                                            let trimmed_err = stderr_snippet.trim();
                                            let msg = if trimmed_err.is_empty() {
                                                format!("exit code {exit_code}")
                                            } else {
                                                format!("exit code {exit_code}: {trimmed_err}")
                                            };
                                            let _ = thread_tx.send(ProjectSetupEvent::Log(format!(
                                                "⚠ Warning: Setup hook for '{repo_name}' failed: {msg}"
                                            )));
                                        } else {
                                            let _ = thread_tx.send(ProjectSetupEvent::Log(format!(
                                                "✓ Setup hook for '{repo_name}' completed: {cmd_display}"
                                            )));
                                        }
                                    }
                                    Err(err) => {
                                        let _ = thread_tx.send(ProjectSetupEvent::Log(format!(
                                            "⚠ Warning: Setup hook for '{repo_name}' failed: {err}"
                                        )));
                                    }
                                }
                            }
                        });
                    }
                    drop(tx);
                    while let Ok(event) = rx.recv() {
                        progress_sink(event);
                    }
                });
            }

            // Step 5: Generating AGENTS.md context
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

            // Step 6: Execute template post_create hooks sequentially at project root
            for cmd in &tpl.post_create {
                if cmd.is_empty() {
                    continue;
                }
                let prog = &cmd[0];
                let args = &cmd[1..];
                let cmd_display = cmd.join(" ");
                progress_sink(ProjectSetupEvent::Log(format!(
                    "Running post_create hook: {cmd_display}"
                )));

                let mut command = std::process::Command::new(prog);
                command.args(args);
                command.current_dir(&project_dir_canon);
                for (k, v) in &tpl.env {
                    command.env(k, v);
                }

                match command.output() {
                    Ok(output) => {
                        if !output.status.success() {
                            let exit_code = output
                                .status
                                .code()
                                .map(|c| c.to_string())
                                .unwrap_or_else(|| "signal".to_string());
                            let stderr_snippet = String::from_utf8_lossy(&output.stderr);
                            let trimmed_err = stderr_snippet.trim();
                            let msg = if trimmed_err.is_empty() {
                                format!("exit code {exit_code}")
                            } else {
                                format!("exit code {exit_code}: {trimmed_err}")
                            };
                            progress_sink(ProjectSetupEvent::Log(format!(
                                "⚠ Warning: Post-create hook failed: {msg}"
                            )));
                        } else {
                            progress_sink(ProjectSetupEvent::Log(format!(
                                "✓ Post-create hook completed: {cmd_display}"
                            )));
                        }
                    }
                    Err(err) => {
                        progress_sink(ProjectSetupEvent::Log(format!(
                            "⚠ Warning: Post-create hook failed: {err}"
                        )));
                    }
                }
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
    pub fn build<F>(
        &self,
        progress_sink: F,
    ) -> anyhow::Result<(ProjectSession, PtySession, VtScreen)>
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

/// Adapts AI agent CLI arguments by appending the initial prompt according to each agent tool's
/// CLI specifications.
///
/// Supported agents (case-insensitive / normalized):
/// - `agy`: Appends `"-i"` followed by the prompt string.
/// - `aider`: Appends `"--message"` followed by the prompt string.
/// - `claude`: Appends the prompt string as a positional argument.
/// - `copilot`: Appends the prompt string as a positional argument.
/// - Unknown / custom agents / fallback shells: Leaves arguments unchanged.
///
/// If `prompt` is `None` or contains only whitespace, arguments remain unchanged.
pub fn adapt_agent_args<S: AsRef<str>>(
    agent_name: &str,
    base_args: &[S],
    prompt: Option<&str>,
) -> Vec<String> {
    let mut args: Vec<String> = base_args.iter().map(|s| s.as_ref().to_string()).collect();

    let prompt = match prompt {
        Some(p) if !p.trim().is_empty() => p,
        _ => return args,
    };

    let normalized = agent_name.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "agy" => {
            args.push("-i".to_string());
            args.push(prompt.to_string());
        }
        "aider" => {
            args.push("--message".to_string());
            args.push(prompt.to_string());
        }
        "claude" => {
            args.push(prompt.to_string());
        }
        "copilot" => {
            args.push(prompt.to_string());
        }
        _ => {}
    }

    args
}

/// Extracts the template name recorded in a project's `AGENTS.md` orientation file, if present.
pub fn extract_template_name_from_agents_md<P: AsRef<std::path::Path>>(
    project_dir: P,
) -> Option<String> {
    let agents_path = project_dir.as_ref().join("AGENTS.md");
    let content = fs::read_to_string(agents_path).ok()?;
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("Template: **") {
            if let Some(tpl) = rest.strip_suffix("**") {
                let tpl = tpl.trim();
                if !tpl.is_empty() {
                    return Some(tpl.to_string());
                }
            }
        }
    }
    None
}

/// Safely tears down a project workspace with streaming progress events:
/// 1. Executes template `pre_destroy` lifecycle hooks if template is configured.
/// 2. Validates uncommitted changes across worktrees (unless `force` is true).
/// 3. Removes git worktrees and prunes central repository metadata.
/// 4. Removes the project directory.
pub fn destroy_project_workspace_with_progress<F>(
    config: Option<&Config>,
    cwd: &std::path::Path,
    project_name: &str,
    force: bool,
    mut progress_sink: F,
) -> anyhow::Result<()>
where
    F: FnMut(ProjectDestroyEvent),
{
    use anyhow::Context;
    let project_dir = cwd.join(project_name);

    let res = (|| -> anyhow::Result<()> {
        if !project_dir.exists() {
            anyhow::bail!("Project directory {project_dir:?} does not exist.");
        }

        // Step 1: Running pre-destroy hooks
        progress_sink(ProjectDestroyEvent::Step {
            current: 1,
            total: 4,
            message: "Running pre-destroy hooks...".to_string(),
        });

        let mut ran_hooks = false;
        if let Some(cfg) = config {
            if let Some(tpl_name) = extract_template_name_from_agents_md(&project_dir) {
                if let Some(tpl) = cfg.get_template(&tpl_name) {
                    for cmd in &tpl.pre_destroy {
                        if cmd.is_empty() {
                            continue;
                        }
                        ran_hooks = true;
                        let prog = &cmd[0];
                        let args = &cmd[1..];
                        let cmd_display = cmd.join(" ");
                        progress_sink(ProjectDestroyEvent::Log(format!(
                            "Running pre_destroy hook: {cmd_display}"
                        )));

                        let mut command = std::process::Command::new(prog);
                        command.args(args);
                        command.current_dir(&project_dir);
                        for (k, v) in &tpl.env {
                            command.env(k, v);
                        }

                        match command.output() {
                            Ok(output) => {
                                if !output.status.success() {
                                    let exit_code = output
                                        .status
                                        .code()
                                        .map(|c| c.to_string())
                                        .unwrap_or_else(|| "signal".to_string());
                                    let stderr_snippet = String::from_utf8_lossy(&output.stderr);
                                    let trimmed_err = stderr_snippet.trim();
                                    let msg = if trimmed_err.is_empty() {
                                        format!("exit code {exit_code}")
                                    } else {
                                        format!("exit code {exit_code}: {trimmed_err}")
                                    };
                                    progress_sink(ProjectDestroyEvent::Log(format!(
                                        "⚠ Warning: Pre-destroy hook failed: {msg}"
                                    )));
                                } else {
                                    progress_sink(ProjectDestroyEvent::Log(format!(
                                        "✓ Pre-destroy hook completed: {cmd_display}"
                                    )));
                                }
                            }
                            Err(err) => {
                                progress_sink(ProjectDestroyEvent::Log(format!(
                                    "⚠ Warning: Pre-destroy hook failed: {err}"
                                )));
                            }
                        }
                    }
                }
            }
        }
        if !ran_hooks {
            progress_sink(ProjectDestroyEvent::Log(
                "No pre-destroy hooks configured.".to_string(),
            ));
        }

        // Step 2: Validating worktrees
        progress_sink(ProjectDestroyEvent::Step {
            current: 2,
            total: 4,
            message: "Validating worktrees...".to_string(),
        });

        let child_paths: Vec<_> = if let Ok(entries) = fs::read_dir(&project_dir) {
            entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir() && p.join(".git").exists())
                .collect()
        } else {
            Vec::new()
        };

        if child_paths.is_empty() {
            progress_sink(ProjectDestroyEvent::Log(
                "No child git worktrees found.".to_string(),
            ));
        } else {
            progress_sink(ProjectDestroyEvent::Log(format!(
                "Found {} child worktree(s) to inspect.",
                child_paths.len()
            )));
        }

        if !force {
            for child_path in &child_paths {
                let repo_name = child_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("unknown");
                if crate::git::has_uncommitted_changes(child_path)? {
                    let err_msg = format!(
                        "Worktree '{repo_name}' ({child_path:?}) has uncommitted changes. Use --force to destroy anyway."
                    );
                    anyhow::bail!("{err_msg}");
                }
            }
        }
        progress_sink(ProjectDestroyEvent::Log(
            "Worktree validation passed.".to_string(),
        ));

        // Step 3: Removing git worktrees
        progress_sink(ProjectDestroyEvent::Step {
            current: 3,
            total: 4,
            message: "Removing git worktrees...".to_string(),
        });

        for child_path in &child_paths {
            let repo_name = child_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown");
            progress_sink(ProjectDestroyEvent::Log(format!(
                "Removing worktree '{repo_name}'..."
            )));
            match crate::git::remove_worktree_auto(child_path, force) {
                Ok(_) => {
                    progress_sink(ProjectDestroyEvent::Log(format!(
                        "✓ Removed worktree '{repo_name}' and pruned metadata."
                    )));
                }
                Err(err) => {
                    progress_sink(ProjectDestroyEvent::Log(format!(
                        "⚠ Warning removing worktree '{repo_name}': {err}"
                    )));
                }
            }
        }

        // Step 4: Removing project directory
        progress_sink(ProjectDestroyEvent::Step {
            current: 4,
            total: 4,
            message: "Removing project directory...".to_string(),
        });

        if project_dir.exists() {
            progress_sink(ProjectDestroyEvent::Log(format!(
                "Deleting project directory {project_dir:?}..."
            )));
            fs::remove_dir_all(&project_dir)
                .with_context(|| format!("Failed to remove project directory {project_dir:?}"))?;
            progress_sink(ProjectDestroyEvent::Log(
                "✓ Project directory removed successfully.".to_string(),
            ));
        }

        Ok(())
    })();

    match res {
        Ok(()) => {
            progress_sink(ProjectDestroyEvent::Success);
            Ok(())
        }
        Err(err) => {
            let err_msg = err.to_string();
            progress_sink(ProjectDestroyEvent::Log(format!("✗ Error: {err_msg}")));
            progress_sink(ProjectDestroyEvent::Error(err_msg));
            Err(err)
        }
    }
}

/// Safely tears down a project workspace:
/// 1. Executes template `pre_destroy` lifecycle hooks if template is configured.
/// 2. Validates uncommitted changes across worktrees (unless `force` is true).
/// 3. Removes git worktrees and prunes central repository metadata.
/// 4. Removes the project directory.
pub fn destroy_project_workspace(
    config: Option<&Config>,
    cwd: &std::path::Path,
    project_name: &str,
    force: bool,
) -> anyhow::Result<()> {
    destroy_project_workspace_with_progress(config, cwd, project_name, force, |_| {})
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
        let config =
            Config::load_from_str("default_agent = \"claude\"\nrepos_dir = \".repos\"\n").unwrap();
        let builder = ProjectBuilder::new(config, "ecommerce", "   ");
        let val = builder.validate_detailed();
        assert!(!val.is_valid);
        assert!(val.primary_message().unwrap().contains("empty"));
    }

    #[test]
    fn test_project_builder_validation_template_not_found() {
        let config =
            Config::load_from_str("default_agent = \"claude\"\nrepos_dir = \".repos\"\n").unwrap();
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
    fn test_project_builder_validate_fast_vs_detailed() {
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

        // Fast validation only checks name, template, and directory existence -> passes!
        let fast_val = builder.validate_fast();
        assert!(fast_val.is_valid);
        assert!(fast_val.errors.is_empty());

        // check_branch_collisions explicitly detects collision
        let branch_collisions = builder.check_branch_collisions();
        assert_eq!(branch_collisions.len(), 1);
        assert!(branch_collisions[0].contains("already exists in repository"));

        // Detailed validation combines fast validation + branch checks -> fails!
        let detailed_val = builder.validate_detailed();
        assert!(!detailed_val.is_valid);
        assert!(detailed_val
            .primary_message()
            .unwrap()
            .contains("already exists in repository"));
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
        assert!(val
            .primary_message()
            .unwrap()
            .contains("already exists in repository"));
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

        assert!(
            res.is_ok(),
            "create_workspace should succeed: {:?}",
            res.err()
        );
        let (project_dir_canon, mappings) = res.unwrap();
        let expected_project_dir = project_cwd.join("checkout-feat").canonicalize().unwrap();
        assert_eq!(project_dir_canon, expected_project_dir);
        assert_eq!(mappings.len(), 1);
        assert_eq!(mappings[0].name, "backend");
        assert_eq!(
            mappings[0].worktree_path,
            expected_project_dir.join("backend")
        );

        assert!(expected_project_dir.exists());
        assert!(expected_project_dir.join("AGENTS.md").exists());
        assert_eq!(
            fs::read_to_string(expected_project_dir.join("CLAUDE.md")).expect("CLAUDE.md"),
            "@AGENTS.md\n"
        );
        assert!(expected_project_dir.join("backend").exists());
        assert!(expected_project_dir
            .join("backend")
            .join("README.md")
            .exists());

        let ev_list = events.lock().unwrap();
        assert!(ev_list
            .iter()
            .any(|e| matches!(e, ProjectSetupEvent::Step { current: 1, .. })));
        assert!(ev_list
            .iter()
            .any(|e| matches!(e, ProjectSetupEvent::Step { current: 4, .. })));
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

        let builder =
            ProjectBuilder::new(config, "failing_tpl", "clashing_project").with_cwd(&project_cwd);

        let events = Arc::new(Mutex::new(Vec::new()));
        let events_clone = Arc::clone(&events);

        let res = builder.create_workspace(move |event| {
            events_clone.lock().unwrap().push(event);
        });

        assert!(res.is_err());
        let project_dir = project_cwd.join("clashing_project");
        assert!(
            !project_dir.exists(),
            "project_dir must be cleaned up on rollback"
        );

        let ev_list = events.lock().unwrap();
        assert!(ev_list
            .iter()
            .any(|e| matches!(e, ProjectSetupEvent::Error(_))));
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
        assert_eq!(
            fs::read_to_string(project_dir.join("CLAUDE.md")).expect("CLAUDE.md"),
            "@AGENTS.md\n"
        );
        assert!(project_dir.join("backend").exists());
        assert!(project_dir.join("backend").join("README.md").exists());

        let ev_list = events.lock().unwrap();
        assert!(ev_list
            .iter()
            .any(|e| matches!(e, ProjectSetupEvent::Success)));
        assert!(ev_list
            .iter()
            .any(|e| matches!(e, ProjectSetupEvent::Step { current: 1, .. })));
        assert!(ev_list
            .iter()
            .any(|e| matches!(e, ProjectSetupEvent::Step { current: 4, .. })));
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

        let builder =
            ProjectBuilder::new(config, "failing_tpl", "clashing_project").with_cwd(&project_cwd);

        let events = Arc::new(Mutex::new(Vec::new()));
        let events_clone = Arc::clone(&events);

        let res = builder.build(move |event| {
            events_clone.lock().unwrap().push(event);
        });

        assert!(res.is_err());
        let project_dir = project_cwd.join("clashing_project");
        assert!(
            !project_dir.exists(),
            "project_dir must be cleaned up on rollback"
        );

        let ev_list = events.lock().unwrap();
        assert!(ev_list
            .iter()
            .any(|e| matches!(e, ProjectSetupEvent::Error(_))));
    }

    #[test]
    fn test_adapt_agent_args_agy_with_prompt() {
        let base = vec!["--verbose".to_string()];
        let res = adapt_agent_args("agy", &base, Some("my prompt"));
        assert_eq!(res, vec!["--verbose", "-i", "my prompt"]);

        let empty_base: Vec<String> = vec![];
        let res2 = adapt_agent_args("AGY", &empty_base, Some("my prompt"));
        assert_eq!(res2, vec!["-i", "my prompt"]);

        let res3 = adapt_agent_args("Agy", &["--debug".to_string()], Some("Fix bug"));
        assert_eq!(res3, vec!["--debug", "-i", "Fix bug"]);
    }

    #[test]
    fn test_adapt_agent_args_aider_with_prompt() {
        let base = vec!["--no-auto-commits".to_string()];
        let res = adapt_agent_args("aider", &base, Some("my prompt"));
        assert_eq!(res, vec!["--no-auto-commits", "--message", "my prompt"]);

        let empty_base: Vec<String> = vec![];
        let res2 = adapt_agent_args("AIDER", &empty_base, Some("my prompt"));
        assert_eq!(res2, vec!["--message", "my prompt"]);
    }

    #[test]
    fn test_adapt_agent_args_claude_with_prompt() {
        let empty_base: Vec<String> = vec![];
        let res = adapt_agent_args("claude", &empty_base, Some("my prompt"));
        assert_eq!(res, vec!["my prompt"]);

        let base = vec!["--print".to_string()];
        let res2 = adapt_agent_args("Claude", &base, Some("my prompt"));
        assert_eq!(res2, vec!["--print", "my prompt"]);
    }

    #[test]
    fn test_adapt_agent_args_copilot_with_prompt() {
        let base = vec!["copilot".to_string(), "suggest".to_string()];
        let res = adapt_agent_args("copilot", &base, Some("my prompt"));
        assert_eq!(res, vec!["copilot", "suggest", "my prompt"]);

        let empty_base: Vec<String> = vec![];
        let res2 = adapt_agent_args("Copilot", &empty_base, Some("my prompt"));
        assert_eq!(res2, vec!["my prompt"]);
    }

    #[test]
    fn test_adapt_agent_args_custom_unknown_agent_with_prompt() {
        let base = vec!["--custom-flag".to_string()];
        let res = adapt_agent_args("custom", &base, Some("my prompt"));
        assert_eq!(res, vec!["--custom-flag"]);

        let empty_base: Vec<String> = vec![];
        let res2 = adapt_agent_args("shell", &empty_base, Some("my prompt"));
        assert_eq!(res2, empty_base);
    }

    #[test]
    fn test_adapt_agent_args_none_or_empty_prompt() {
        let base = vec!["--verbose".to_string()];
        assert_eq!(adapt_agent_args("agy", &base, None), base);
        assert_eq!(adapt_agent_args("agy", &base, Some("")), base);
        assert_eq!(adapt_agent_args("agy", &base, Some("   \n\t  ")), base);

        let aider_base = vec!["--no-auto-commits".to_string()];
        assert_eq!(adapt_agent_args("aider", &aider_base, None), aider_base);
        assert_eq!(
            adapt_agent_args("aider", &aider_base, Some("  ")),
            aider_base
        );

        let claude_base: Vec<String> = vec![];
        assert_eq!(adapt_agent_args("claude", &claude_base, None), claude_base);
        assert_eq!(
            adapt_agent_args("claude", &claude_base, Some("")),
            claude_base
        );

        let copilot_base = vec!["copilot".to_string(), "suggest".to_string()];
        assert_eq!(
            adapt_agent_args("copilot", &copilot_base, None),
            copilot_base
        );
        assert_eq!(
            adapt_agent_args("copilot", &copilot_base, Some("  \t  ")),
            copilot_base
        );
    }

    #[test]
    fn test_project_builder_resolve_agent_args_and_command() {
        let toml_str = r#"
            default_agent = "claude"
            repos_dir = ".repos"
            [agents.claude]
            command = "claude"
            args = []
            [agents.agy]
            command = "agy"
            args = ["--verbose"]
            [agents.aider]
            command = "aider"
            args = ["--no-auto-commits"]
            [agents.copilot]
            command = "gh"
            args = ["copilot", "suggest"]
            [agents.custom]
            command = "custom-agent"
            args = ["--run"]
            [templates.tpl]
            repos = []
        "#;
        let config = Config::load_from_str(toml_str).unwrap();

        // 1. agy with prompt -> ["--verbose", "-i", "Fix checkout bug"]
        let b_agy = ProjectBuilder::new(config.clone(), "tpl", "p1")
            .with_agent("agy")
            .with_prompt("Fix checkout bug");
        let (cmd, args, _) = b_agy.resolve_agent_command();
        assert_eq!(cmd, "agy");
        assert_eq!(args, vec!["--verbose", "-i", "Fix checkout bug"]);
        assert_eq!(
            b_agy.resolve_agent_args(),
            vec!["--verbose", "-i", "Fix checkout bug"]
        );

        // 2. aider with prompt -> ["--no-auto-commits", "--message", "Fix backend tests"]
        let b_aider = ProjectBuilder::new(config.clone(), "tpl", "p2")
            .with_agent("aider")
            .with_prompt("Fix backend tests");
        let (cmd, args, _) = b_aider.resolve_agent_command();
        assert_eq!(cmd, "aider");
        assert_eq!(
            args,
            vec!["--no-auto-commits", "--message", "Fix backend tests"]
        );
        assert_eq!(
            b_aider.resolve_agent_args(),
            vec!["--no-auto-commits", "--message", "Fix backend tests"]
        );

        // 3. claude with prompt -> ["Implement new login"]
        let b_claude = ProjectBuilder::new(config.clone(), "tpl", "p3")
            .with_agent("claude")
            .with_prompt("Implement new login");
        let (cmd, args, _) = b_claude.resolve_agent_command();
        assert_eq!(cmd, "claude");
        assert_eq!(args, vec!["Implement new login"]);
        assert_eq!(b_claude.resolve_agent_args(), vec!["Implement new login"]);

        // 4. copilot with prompt -> ["copilot", "suggest", "Suggest git command"]
        let b_copilot = ProjectBuilder::new(config.clone(), "tpl", "p4")
            .with_agent("copilot")
            .with_prompt("Suggest git command");
        let (cmd, args, _) = b_copilot.resolve_agent_command();
        assert_eq!(cmd, "gh");
        assert_eq!(args, vec!["copilot", "suggest", "Suggest git command"]);
        assert_eq!(
            b_copilot.resolve_agent_args(),
            vec!["copilot", "suggest", "Suggest git command"]
        );

        // 5. custom agent with prompt -> unchanged args ["--run"]
        let b_custom = ProjectBuilder::new(config.clone(), "tpl", "p5")
            .with_agent("custom")
            .with_prompt("Do task");
        let (cmd, args, _) = b_custom.resolve_agent_command();
        assert_eq!(cmd, "custom-agent");
        assert_eq!(args, vec!["--run"]);
        assert_eq!(b_custom.resolve_agent_args(), vec!["--run"]);

        // 6. agy without prompt -> unchanged args ["--verbose"]
        let b_agy_no_prompt = ProjectBuilder::new(config.clone(), "tpl", "p6").with_agent("agy");
        assert_eq!(b_agy_no_prompt.resolve_agent_args(), vec!["--verbose"]);

        // 7. claude with empty whitespace prompt -> unchanged empty args []
        let b_claude_empty = ProjectBuilder::new(config, "tpl", "p7")
            .with_agent("claude")
            .with_prompt("   ");
        assert_eq!(b_claude_empty.resolve_agent_args(), Vec::<String>::new());
    }

    #[test]
    fn test_extract_template_name_from_agents_md() {
        let temp_dir = TempDir::new().unwrap();
        let proj_dir = temp_dir.path().join("proj");
        fs::create_dir_all(&proj_dir).unwrap();

        // 1. Missing AGENTS.md
        assert_eq!(extract_template_name_from_agents_md(&proj_dir), None);

        // 2. AGENTS.md without template line
        fs::write(
            proj_dir.join("AGENTS.md"),
            "# Project: proj\n\nNo template line\n",
        )
        .unwrap();
        assert_eq!(extract_template_name_from_agents_md(&proj_dir), None);

        // 3. Valid AGENTS.md
        fs::write(
            proj_dir.join("AGENTS.md"),
            "# Project: proj\n\nTemplate: **ecommerce**\n\n## Repository Layout\n",
        )
        .unwrap();
        assert_eq!(
            extract_template_name_from_agents_md(&proj_dir),
            Some("ecommerce".to_string())
        );
    }

    #[test]
    fn test_setup_and_post_create_hooks_execution_success() {
        let temp_dir = TempDir::new().unwrap();
        let repos_dir = temp_dir.path().join("repos");
        let repo_backend = repos_dir.join("backend");
        let repo_frontend = repos_dir.join("frontend");
        create_mock_git_repo(&repo_backend);
        create_mock_git_repo(&repo_frontend);

        let toml_str = format!(
            r#"
            default_agent = "claude"
            repos_dir = "{}"

            [repos.backend]
            setup = [
                ["git", "status"],
                ["sh", "-c", "echo 'backend-setup-ok' > setup.marker"]
            ]
            env = {{ BACKEND_ENV = "backend_val" }}

            [repos.frontend]
            setup = ["sh -c 'echo frontend-setup-ok > setup.marker'"]

            [templates.fullstack]
            repos = ["backend", "frontend"]
            post_create = [
                ["sh", "-c", "echo 'post-create-ok' > post_create.marker"]
            ]
            env = {{ TEMPLATE_ENV = "tpl_val" }}
            "#,
            repos_dir.display()
        );

        let config = Config::load_from_str(&toml_str).unwrap();
        let project_cwd = temp_dir.path().join("workspaces");
        fs::create_dir_all(&project_cwd).unwrap();

        let builder = ProjectBuilder::new(config, "fullstack", "my-project").with_cwd(&project_cwd);

        let events = Arc::new(Mutex::new(Vec::new()));
        let events_clone = Arc::clone(&events);

        let res = builder.create_workspace(move |ev| {
            events_clone.lock().unwrap().push(ev);
        });

        assert!(
            res.is_ok(),
            "create_workspace should succeed: {:?}",
            res.err()
        );
        let (project_dir, mappings) = res.unwrap();
        assert_eq!(mappings.len(), 2);

        // Verify setup hooks created markers in worktrees
        assert!(project_dir.join("backend").join("setup.marker").exists());
        assert_eq!(
            fs::read_to_string(project_dir.join("backend").join("setup.marker"))
                .unwrap()
                .trim(),
            "backend-setup-ok"
        );

        assert!(project_dir.join("frontend").join("setup.marker").exists());
        assert_eq!(
            fs::read_to_string(project_dir.join("frontend").join("setup.marker"))
                .unwrap()
                .trim(),
            "frontend-setup-ok"
        );

        // Verify post_create hook created marker at project root
        assert!(project_dir.join("post_create.marker").exists());
        assert_eq!(
            fs::read_to_string(project_dir.join("post_create.marker"))
                .unwrap()
                .trim(),
            "post-create-ok"
        );

        let ev_list = events.lock().unwrap();
        assert!(ev_list.iter().any(|e| match e {
            ProjectSetupEvent::Log(msg) => msg.contains("Setup hook for 'backend' completed"),
            _ => false,
        }));
        assert!(ev_list.iter().any(|e| match e {
            ProjectSetupEvent::Log(msg) => msg.contains("Post-create hook completed"),
            _ => false,
        }));
    }

    #[test]
    fn test_setup_hooks_warn_and_continue_on_failure() {
        let temp_dir = TempDir::new().unwrap();
        let repos_dir = temp_dir.path().join("repos");
        let repo_backend = repos_dir.join("backend");
        create_mock_git_repo(&repo_backend);

        let toml_str = format!(
            r#"
            default_agent = "claude"
            repos_dir = "{}"

            [repos.backend]
            setup = [
                ["non_existent_binary_xyz_12345", "--arg"],
                ["sh", "-c", "exit 42"],
                ["sh", "-c", "echo 'survived' > after_fail.marker"]
            ]

            [templates.failing_setup_tpl]
            repos = ["backend"]
            post_create = [
                ["non_existent_post_create_cmd"]
            ]
            "#,
            repos_dir.display()
        );

        let config = Config::load_from_str(&toml_str).unwrap();
        let project_cwd = temp_dir.path().join("workspaces");
        fs::create_dir_all(&project_cwd).unwrap();

        let builder = ProjectBuilder::new(config, "failing_setup_tpl", "resilient-project")
            .with_cwd(&project_cwd);

        let events = Arc::new(Mutex::new(Vec::new()));
        let events_clone = Arc::clone(&events);

        let res = builder.create_workspace(move |ev| {
            events_clone.lock().unwrap().push(ev);
        });

        // Warn-and-continue: must succeed without rollback!
        assert!(
            res.is_ok(),
            "create_workspace should succeed despite hook failure"
        );
        let (project_dir, _) = res.unwrap();
        assert!(project_dir.exists(), "project_dir must exist");
        assert!(project_dir.join("backend").exists());
        assert!(project_dir.join("AGENTS.md").exists());
        assert!(project_dir
            .join("backend")
            .join("after_fail.marker")
            .exists());

        let ev_list = events.lock().unwrap();
        assert!(ev_list.iter().any(|e| match e {
            ProjectSetupEvent::Log(msg) =>
                msg.contains("⚠ Warning: Setup hook for 'backend' failed"),
            _ => false,
        }));
        assert!(ev_list.iter().any(|e| match e {
            ProjectSetupEvent::Log(msg) => msg.contains("⚠ Warning: Post-create hook failed"),
            _ => false,
        }));
    }

    #[test]
    fn test_destroy_project_workspace_runs_pre_destroy_hooks() {
        let temp_dir = TempDir::new().unwrap();
        let repos_dir = temp_dir.path().join("repos");
        let repo_backend = repos_dir.join("backend");
        create_mock_git_repo(&repo_backend);

        let cleanup_marker = temp_dir.path().join("pre_destroy_ran.marker");

        let toml_str = format!(
            r#"
            default_agent = "claude"
            repos_dir = "{}"

            [templates.with_pre_destroy]
            repos = ["backend"]
            pre_destroy = [
                ["sh", "-c", "echo 'cleanup-ok' > {}"]
            ]
            "#,
            repos_dir.display(),
            cleanup_marker.display()
        );

        let config = Config::load_from_str(&toml_str).unwrap();
        let project_cwd = temp_dir.path().join("workspaces");
        fs::create_dir_all(&project_cwd).unwrap();

        let builder = ProjectBuilder::new(config.clone(), "with_pre_destroy", "proj-to-destroy")
            .with_cwd(&project_cwd);

        let (project_dir, _) = builder.create_workspace(|_| {}).unwrap();
        assert!(project_dir.exists());
        assert!(project_dir.join("AGENTS.md").exists());

        // Destroy project workspace
        let destroy_res =
            destroy_project_workspace(Some(&config), &project_cwd, "proj-to-destroy", false);
        assert!(destroy_res.is_ok(), "destroy must succeed");
        assert!(!project_dir.exists(), "project_dir must be deleted");

        // Verify pre_destroy hook ran
        assert!(
            cleanup_marker.exists(),
            "pre_destroy hook must have created cleanup marker"
        );
        assert_eq!(
            fs::read_to_string(&cleanup_marker).unwrap().trim(),
            "cleanup-ok"
        );
    }

    #[test]
    fn test_destroy_project_workspace_with_progress_streams_all_steps_and_events() {
        let temp_dir = TempDir::new().unwrap();
        let repos_dir = temp_dir.path().join("repos");
        let repo_backend = repos_dir.join("backend");
        create_mock_git_repo(&repo_backend);

        let cleanup_marker = temp_dir.path().join("pre_destroy_stream.marker");

        let toml_str = format!(
            r#"
            default_agent = "claude"
            repos_dir = "{}"

            [templates.stream_tpl]
            repos = ["backend"]
            pre_destroy = [
                ["sh", "-c", "echo 'stream-cleanup' > {}"]
            ]
            "#,
            repos_dir.display(),
            cleanup_marker.display()
        );

        let config = Config::load_from_str(&toml_str).unwrap();
        let project_cwd = temp_dir.path().join("workspaces");
        fs::create_dir_all(&project_cwd).unwrap();

        let builder =
            ProjectBuilder::new(config.clone(), "stream_tpl", "proj-stream").with_cwd(&project_cwd);

        let (project_dir, _) = builder.create_workspace(|_| {}).unwrap();
        assert!(project_dir.exists());

        let events = Arc::new(Mutex::new(Vec::new()));
        let events_clone = Arc::clone(&events);

        let res = destroy_project_workspace_with_progress(
            Some(&config),
            &project_cwd,
            "proj-stream",
            false,
            move |ev| {
                events_clone.lock().unwrap().push(ev);
            },
        );

        assert!(res.is_ok());
        assert!(!project_dir.exists());
        assert!(cleanup_marker.exists());

        let evs = events.lock().unwrap();
        let steps: Vec<usize> = evs
            .iter()
            .filter_map(|e| match e {
                ProjectDestroyEvent::Step { current, .. } => Some(*current),
                _ => None,
            })
            .collect();
        assert_eq!(steps, vec![1, 2, 3, 4]);

        assert!(evs
            .iter()
            .any(|e| matches!(e, ProjectDestroyEvent::Success)));
        assert!(evs.iter().any(|e| match e {
            ProjectDestroyEvent::Log(msg) => msg.contains("pre_destroy hook"),
            _ => false,
        }));
        assert!(evs.iter().any(|e| match e {
            ProjectDestroyEvent::Log(msg) => msg.contains("backend"),
            _ => false,
        }));
    }

    #[test]
    fn test_destroy_project_workspace_with_progress_error_on_missing_dir() {
        let temp_dir = TempDir::new().unwrap();
        let project_cwd = temp_dir.path().join("workspaces");
        fs::create_dir_all(&project_cwd).unwrap();

        let events = Arc::new(Mutex::new(Vec::new()));
        let events_clone = Arc::clone(&events);

        let res = destroy_project_workspace_with_progress(
            None,
            &project_cwd,
            "nonexistent-proj",
            false,
            move |ev| {
                events_clone.lock().unwrap().push(ev);
            },
        );

        assert!(res.is_err());
        let evs = events.lock().unwrap();
        assert!(evs
            .iter()
            .any(|e| matches!(e, ProjectDestroyEvent::Error(_))));
    }

    #[test]
    fn test_destroy_project_workspace_with_progress_uncommitted_changes_prevented() {
        let temp_dir = TempDir::new().unwrap();
        let repos_dir = temp_dir.path().join("repos");
        let repo_backend = repos_dir.join("backend");
        create_mock_git_repo(&repo_backend);

        let toml_str = format!(
            r#"
            default_agent = "claude"
            repos_dir = "{}"

            [templates.safety_tpl]
            repos = ["backend"]
            "#,
            repos_dir.display(),
        );

        let config = Config::load_from_str(&toml_str).unwrap();
        let project_cwd = temp_dir.path().join("workspaces");
        fs::create_dir_all(&project_cwd).unwrap();

        let builder =
            ProjectBuilder::new(config.clone(), "safety_tpl", "proj-safety").with_cwd(&project_cwd);

        let (project_dir, _) = builder.create_workspace(|_| {}).unwrap();
        assert!(project_dir.exists());

        // Introduce uncommitted change
        fs::write(project_dir.join("backend").join("new_file.txt"), "dirty").unwrap();

        let events = Arc::new(Mutex::new(Vec::new()));
        let events_clone = Arc::clone(&events);

        // Attempt destruction without force -> must fail
        let res = destroy_project_workspace_with_progress(
            Some(&config),
            &project_cwd,
            "proj-safety",
            false,
            move |ev| {
                events_clone.lock().unwrap().push(ev);
            },
        );

        assert!(res.is_err());
        assert!(
            project_dir.exists(),
            "Directory must NOT be deleted if uncommitted changes exist"
        );

        let evs = events.lock().unwrap();
        assert!(evs.iter().any(|e| match e {
            ProjectDestroyEvent::Error(err) => err.contains("uncommitted changes"),
            _ => false,
        }));

        // Now destroy with force -> must succeed
        let res_force = destroy_project_workspace_with_progress(
            Some(&config),
            &project_cwd,
            "proj-safety",
            true,
            |_| {},
        );
        assert!(res_force.is_ok());
        assert!(!project_dir.exists());
    }
}
