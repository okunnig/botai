use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use thiserror::Error;

use crate::config::{Config, ConfigError, ResolvedRepo};
use crate::context::{generate_agents_md, ContextError, ProjectContext, RepoMapping};
use crate::git::{
    create_worktree, delete_branch, remove_worktree_auto, GitError, GitWorktreeManager,
    WorktreeManager,
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

    #[error("At least one repository must be selected")]
    NoRepositoriesSelected,

    #[error("Repository '{name}' not found in template '{template}'")]
    RepositoryNotInTemplate { name: String, template: String },

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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectBuilder {
    pub config: Config,
    pub template_name: String,
    pub project_name: String,
    pub selected_repos: Option<Vec<String>>,
    pub agent_override: Option<String>,
    pub prompt: Option<String>,
    pub cwd: PathBuf,
    pub viewport_size: (u16, u16),
}

/// Undoes worktrees created for `project_name`, and the branches that came with them.
///
/// `git worktree remove` keeps the branch, so without this a failed setup leaves one behind
/// in every repo and the next attempt at the same name fails with `BranchExists`.
/// Best effort: this runs on a path that is already failing.
fn rollback_worktrees(mappings: &[RepoMapping], project_name: &str) {
    for mapping in mappings {
        let _ = remove_worktree_auto(&mapping.worktree_path, true);
        let _ = delete_branch(&mapping.original_path, project_name);
    }
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
            selected_repos: None,
            agent_override: None,
            prompt: None,
            cwd,
            viewport_size: (100, 30),
        }
    }

    /// Sets the list of selected repository names to checkout in the project.
    pub fn with_selected_repos<I, S>(mut self, repos: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.selected_repos = Some(repos.into_iter().map(Into::into).collect());
        self
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

    /// Resolves the effective repositories to provision for this project:
    /// - If `selected_repos` is `Some(...)`, validates each name against the template's declared
    ///   repositories (returning `RepositoryNotInTemplate` if unknown) and filters `all_repos`.
    /// - If `selected_repos` is `None`, filters `all_repos` keeping only repositories with `default_include = true`.
    /// - Returns `NoRepositoriesSelected` if the effective list is empty.
    pub fn resolve_effective_repos(&self) -> Result<Vec<ResolvedRepo>, ProjectError> {
        let all_repos = self.config.resolve_template_repos(&self.template_name)?;
        let has_configured_repos = !all_repos.is_empty();
        let effective: Vec<ResolvedRepo> = if let Some(ref selected) = self.selected_repos {
            for name in selected {
                if !all_repos.iter().any(|r| &r.name == name) {
                    return Err(ProjectError::RepositoryNotInTemplate {
                        name: name.clone(),
                        template: self.template_name.clone(),
                    });
                }
            }
            all_repos
                .into_iter()
                .filter(|r| selected.iter().any(|s| s == &r.name))
                .collect()
        } else {
            all_repos
                .into_iter()
                .filter(|r| r.default_include)
                .collect()
        };

        if has_configured_repos && effective.is_empty() {
            return Err(ProjectError::NoRepositoriesSelected);
        }

        Ok(effective)
    }

    /// Validates preconditions for project creation:
    /// - Non-empty project name
    /// - Template existence in configuration
    /// - Project directory does not already exist in CWD
    /// - Branch `<project_name>` does not collide in any locally existing effective repositories
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

        let resolved_repos = self.resolve_effective_repos()?;
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
    /// - Selected repository list is not empty (if explicitly specified)
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

        if let Some(tpl) = self.config.get_template(&self.template_name) {
            if !tpl.repos.is_empty() && self.selected_repos.as_ref().is_some_and(|r| r.is_empty()) {
                validation.add_error("At least one repository must be selected");
                return validation;
            }
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
        match self.resolve_effective_repos() {
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

        let resolved_repos = match self.resolve_effective_repos() {
            Ok(repos) => repos,
            Err(e) => {
                validation.add_error(e.to_string());
                return validation;
            }
        };

        let trimmed_name = self.project_name.trim();
        let git = GitWorktreeManager::new();
        for repo in resolved_repos {
            if repo.exists_locally() {
                if let Ok(exists) = git.branch_exists(&repo.local_path, trimmed_name) {
                    if exists {
                        validation.add_error(format!(
                            "Branch '{}' already exists in repository '{}'",
                            trimmed_name, repo.name
                        ));
                    }
                }
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

            let resolved_repos: Vec<ResolvedRepo> = self.resolve_effective_repos()?;

            // Step 2: Auto-cloning missing repos & fetching updates
            progress_sink(ProjectSetupEvent::Step {
                current: 2,
                total: 4,
                message: "Cloning and updating repositories...".to_string(),
            });

            if !resolved_repos.is_empty() {
                let (tx, rx) = std::sync::mpsc::channel();
                let mut errors = Vec::new();

                std::thread::scope(|s| {
                    for repo in &resolved_repos {
                        let thread_tx = tx.clone();
                        s.spawn(move || {
                            if !repo.exists_locally() {
                                if let Some(url) = &repo.clone_url {
                                    let _ = thread_tx.send(ProjectSetupEvent::Log(format!(
                                        "[2/4] Cloning repository '{}' from {}...",
                                        repo.name, url
                                    )));
                                    if let Err(err) = crate::git::clone_repo(url, &repo.local_path) {
                                        let _ = thread_tx.send(ProjectSetupEvent::Error(format!(
                                            "Failed to clone repository '{}': {}",
                                            repo.name, err
                                        )));
                                    }
                                } else {
                                    let _ = thread_tx.send(ProjectSetupEvent::Error(format!(
                                        "Repository '{}' does not exist locally and has no remote URL configured",
                                        repo.name
                                    )));
                                }
                            } else {
                                let _ = thread_tx.send(ProjectSetupEvent::Log(format!(
                                    "[2/4] Fetching latest updates for repository '{}'...",
                                    repo.name
                                )));
                                let _ = crate::git::fetch_repo(&repo.local_path);
                            }
                        });
                    }

                    drop(tx);
                    while let Ok(event) = rx.recv() {
                        if let ProjectSetupEvent::Error(ref err) = event {
                            errors.push(err.clone());
                        }
                        progress_sink(event);
                    }
                });

                if !errors.is_empty() {
                    anyhow::bail!("{}", errors.join("; "));
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
            // `dunce`: every worktree path derives from this and is passed to
            // `git worktree add` as a positional argument, which rejects a `\\?\` path.
            let project_dir_canon =
                dunce::canonicalize(&project_dir).unwrap_or_else(|_| project_dir.clone());

            // Group repositories by backing repository path (`repo.local_path`) so distinct
            // backing repos run concurrently in std::thread::scope, while multiple worktrees
            // sharing the same backing repo run sequentially to prevent git index.lock collisions.
            let mut repo_groups: HashMap<PathBuf, Vec<(usize, &ResolvedRepo)>> = HashMap::new();
            for (idx, repo) in resolved_repos.iter().enumerate() {
                repo_groups
                    .entry(repo.local_path.clone())
                    .or_default()
                    .push((idx, repo));
            }

            enum WorktreeMsg {
                Log(String),
                Success(usize, RepoMapping),
                Failure {
                    index: usize,
                    name: String,
                    local_path: PathBuf,
                    error: GitError,
                },
            }

            let (tx, rx) = std::sync::mpsc::channel();
            let mut created_mappings_indexed: Vec<(usize, RepoMapping)> = Vec::new();
            let mut failed_creations: Vec<(usize, String, PathBuf, GitError)> = Vec::new();

            std::thread::scope(|s| {
                for repos in repo_groups.values() {
                    let thread_tx = tx.clone();
                    let project_dir_canon = &project_dir_canon;
                    s.spawn(move || {
                        for (idx, repo) in repos {
                            let worktree_path = project_dir_canon.join(&repo.name);
                            let _ = thread_tx.send(WorktreeMsg::Log(format!(
                                "[3/4] Creating git worktree for '{}' at {}...",
                                repo.name,
                                worktree_path.display()
                            )));

                            match create_worktree(
                                &repo.local_path,
                                &worktree_path,
                                trimmed_project_name,
                            ) {
                                Ok(()) => {
                                    let mapping = RepoMapping::new(
                                        repo.name.clone(),
                                        repo.local_path.clone(),
                                        worktree_path,
                                    );
                                    let _ = thread_tx.send(WorktreeMsg::Success(*idx, mapping));
                                }
                                Err(err) => {
                                    let _ = thread_tx.send(WorktreeMsg::Failure {
                                        index: *idx,
                                        name: repo.name.clone(),
                                        local_path: repo.local_path.clone(),
                                        error: err,
                                    });
                                    // Stop creating further worktrees for this backing repository
                                    break;
                                }
                            }
                        }
                    });
                }

                drop(tx);

                while let Ok(msg) = rx.recv() {
                    match msg {
                        WorktreeMsg::Log(log) => {
                            progress_sink(ProjectSetupEvent::Log(log));
                        }
                        WorktreeMsg::Success(idx, mapping) => {
                            created_mappings_indexed.push((idx, mapping));
                        }
                        WorktreeMsg::Failure {
                            index,
                            name,
                            local_path,
                            error,
                        } => {
                            failed_creations.push((index, name, local_path, error));
                        }
                    }
                }
            });

            if !failed_creations.is_empty() {
                let created_mappings: Vec<RepoMapping> = created_mappings_indexed
                    .into_iter()
                    .map(|(_, m)| m)
                    .collect();
                rollback_worktrees(&created_mappings, trimmed_project_name);

                for (_, name, local_path, err) in &failed_creations {
                    if !matches!(err, GitError::BranchAlreadyExists { .. }) {
                        let _ = delete_branch(local_path, trimmed_project_name);
                    }
                    let _ = remove_worktree_auto(&project_dir_canon.join(name), true);
                }

                let _ = fs::remove_dir_all(&project_dir);

                failed_creations.sort_by_key(|f| f.0);
                let error_msgs: Vec<String> = failed_creations
                    .iter()
                    .map(|(_, name, _, err)| {
                        format!("Failed to create worktree for '{name}': {err}")
                    })
                    .collect();
                anyhow::bail!("{}", error_msgs.join("; "));
            }

            created_mappings_indexed.sort_by_key(|(idx, _)| *idx);
            let created_mappings: Vec<RepoMapping> = created_mappings_indexed
                .into_iter()
                .map(|(_, m)| m)
                .collect();

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

                                let mut command = prepare_hook_command(prog, args);
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
                rollback_worktrees(&created_mappings, trimmed_project_name);
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

                let mut command = prepare_hook_command(prog, args);
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
                rollback_worktrees(&created_mappings, self.project_name.trim());
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

                        let mut command = prepare_hook_command(prog, args);
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

        if !force && !child_paths.is_empty() {
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::scope(|s| {
                for child_path in &child_paths {
                    let thread_tx = tx.clone();
                    let path = child_path.clone();
                    s.spawn(move || {
                        let repo_name = path
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("unknown")
                            .to_string();
                        let result = crate::git::has_uncommitted_changes(&path);
                        let _ = thread_tx.send((repo_name, path, result));
                    });
                }
                drop(tx);

                let mut dirty_worktrees = Vec::new();
                let mut check_errors = Vec::new();

                while let Ok((repo_name, path, result)) = rx.recv() {
                    match result {
                        Ok(true) => {
                            dirty_worktrees.push((repo_name, path));
                        }
                        Ok(false) => {}
                        Err(err) => {
                            check_errors.push(format!("'{repo_name}': {err}"));
                        }
                    }
                }

                if !check_errors.is_empty() {
                    anyhow::bail!(
                        "Failed to check uncommitted changes: {}",
                        check_errors.join("; ")
                    );
                }

                if !dirty_worktrees.is_empty() {
                    dirty_worktrees.sort_by(|a, b| a.0.cmp(&b.0));
                    let dirty_list: Vec<String> = dirty_worktrees
                        .iter()
                        .map(|(name, path)| format!("'{name}' ({})", path.display()))
                        .collect();
                    anyhow::bail!(
                        "Worktree(s) have uncommitted changes: {}. Use --force to destroy anyway.",
                        dirty_list.join(", ")
                    );
                }

                Ok::<(), anyhow::Error>(())
            })?;
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

        if !child_paths.is_empty() {
            let mut groups: HashMap<PathBuf, Vec<PathBuf>> = HashMap::new();
            for child_path in &child_paths {
                let main_repo = crate::git::resolve_main_repo(child_path)
                    .unwrap_or_else(|_| child_path.clone());
                groups
                    .entry(main_repo)
                    .or_default()
                    .push(child_path.clone());
            }

            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::scope(|s| {
                for group in groups.into_values() {
                    let thread_tx = tx.clone();
                    s.spawn(move || {
                        for child_path in group {
                            let repo_name = child_path
                                .file_name()
                                .and_then(|n| n.to_str())
                                .unwrap_or("unknown")
                                .to_string();
                            let _ = thread_tx.send(ProjectDestroyEvent::Log(format!(
                                "Removing worktree '{repo_name}'..."
                            )));
                            match crate::git::remove_worktree_auto(&child_path, force) {
                                Ok(_) => {
                                    let _ = thread_tx.send(ProjectDestroyEvent::Log(format!(
                                        "✓ Removed worktree '{repo_name}' and pruned metadata."
                                    )));
                                }
                                Err(err) => {
                                    let _ = thread_tx.send(ProjectDestroyEvent::Log(format!(
                                        "⚠ Warning removing worktree '{repo_name}': {err}"
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

/// Resolves an executable candidate against PATH and PATHEXT extensions.
///
/// On Windows, `CreateProcess` does not resolve extensions from `%PATHEXT%` (e.g. `.cmd`, `.bat`).
/// When launching hooks (such as `npm`, `yarn`, `pnpm`), this function finds the exact
/// executable file so Rust's `std::process::Command` can invoke it correctly.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn resolve_executable_in_paths(
    prog: &str,
    paths: impl IntoIterator<Item = std::path::PathBuf>,
    pathexts: &[&str],
) -> std::path::PathBuf {
    let prog_path = std::path::Path::new(prog);
    if prog_path.is_absolute() || prog.contains('/') || prog.contains('\\') {
        if prog_path.is_file() {
            return prog_path.to_path_buf();
        }
        for ext in pathexts {
            let clean = ext.trim_start_matches('.');
            let lower = clean.to_lowercase();
            let upper = clean.to_uppercase();
            for e in [&lower, clean, &upper] {
                let cand = prog_path.with_extension(e);
                if cand.is_file() {
                    return cand;
                }
                let cand = std::path::PathBuf::from(format!("{prog}.{e}"));
                if cand.is_file() {
                    return cand;
                }
            }
        }
        return prog_path.to_path_buf();
    }

    for dir in paths {
        if prog_path.extension().is_some() {
            let candidate = dir.join(prog);
            if candidate.is_file() {
                return candidate;
            }
            if let Some(file_name) = prog_path.file_name().and_then(|n| n.to_str()) {
                let lower = dir.join(file_name.to_lowercase());
                if lower.is_file() {
                    return lower;
                }
                let upper = dir.join(file_name.to_uppercase());
                if upper.is_file() {
                    return upper;
                }
            }
        }
        for ext in pathexts {
            let clean = ext.trim_start_matches('.');
            let lower = clean.to_lowercase();
            let upper = clean.to_uppercase();
            for e in [&lower, clean, &upper] {
                let candidate = dir.join(format!("{prog}.{e}"));
                if candidate.is_file() {
                    return candidate;
                }
            }
        }
    }

    prog_path.to_path_buf()
}

/// Prepares a `std::process::Command` for a lifecycle hook.
///
/// On Windows, probes `%PATH%` and `%PATHEXT%` (`.COM`, `.EXE`, `.BAT`, `.CMD`)
/// to resolve scripts (e.g. `npm`, `yarn`, `pnpm`) before spawning.
#[cfg(windows)]
pub(crate) fn prepare_hook_command(prog: &str, args: &[String]) -> std::process::Command {
    let pathext_var = std::env::var("PATHEXT").unwrap_or_default();
    let parsed_pathexts: Vec<&str> = pathext_var
        .split(';')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();
    let pathexts = if parsed_pathexts.is_empty() {
        vec![".COM", ".EXE", ".BAT", ".CMD"]
    } else {
        parsed_pathexts
    };
    let path_var = std::env::var_os("PATH").unwrap_or_default();
    let split_paths = std::env::split_paths(&path_var);
    let resolved = resolve_executable_in_paths(prog, split_paths, &pathexts);
    let mut command = std::process::Command::new(resolved);
    command.args(args);
    command
}

/// Prepares a `std::process::Command` for a lifecycle hook on non-Windows platforms.
#[cfg(not(windows))]
pub(crate) fn prepare_hook_command(prog: &str, args: &[String]) -> std::process::Command {
    let mut command = std::process::Command::new(prog);
    command.args(args);
    command
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

    fn toml_path(p: &std::path::Path) -> String {
        p.to_string_lossy().replace('\\', "/")
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
            toml_path(temp_dir.path()),
            toml_path(&repo_dir)
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
            toml_path(temp_dir.path()),
            toml_path(&repo_dir)
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
            toml_path(temp_dir.path()),
            toml_path(&repo_dir)
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
        let expected_project_dir = dunce::canonicalize(project_cwd.join("checkout-feat")).unwrap();
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
            toml_path(temp_dir.path()),
            toml_path(&repo_dir)
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
            toml_path(temp_dir.path()),
            toml_path(&repo_dir)
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
            toml_path(temp_dir.path()),
            toml_path(&repo_dir)
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
            toml_path(&repos_dir)
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
            toml_path(&repos_dir)
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
            toml_path(&repos_dir),
            toml_path(&cleanup_marker)
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
            toml_path(&repos_dir),
            toml_path(&cleanup_marker)
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
            toml_path(&repos_dir),
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

    #[test]
    fn test_resolve_executable_bare_command_to_cmd_or_bat_in_mock_path() {
        let temp_dir = TempDir::new().unwrap();
        let path1 = temp_dir.path().join("dir1");
        let path2 = temp_dir.path().join("dir2");
        fs::create_dir_all(&path1).unwrap();
        fs::create_dir_all(&path2).unwrap();

        // Create npm.cmd in dir2 and yarn.bat in dir1
        fs::write(path2.join("npm.cmd"), "@echo off\necho npm").unwrap();
        fs::write(path1.join("yarn.bat"), "@echo off\necho yarn").unwrap();

        let pathexts = [".COM", ".EXE", ".BAT", ".CMD"];

        // Resolving bare "npm" should find npm.cmd in dir2
        let resolved_npm =
            resolve_executable_in_paths("npm", vec![path1.clone(), path2.clone()], &pathexts);
        assert_eq!(resolved_npm, path2.join("npm.cmd"));

        // Resolving bare "yarn" should find yarn.bat in dir1
        let resolved_yarn =
            resolve_executable_in_paths("yarn", vec![path1.clone(), path2.clone()], &pathexts);
        assert_eq!(resolved_yarn, path1.join("yarn.bat"));

        // Test with extension without leading dot
        let pathexts_no_dot = ["COM", "EXE", "BAT", "CMD"];
        let resolved_npm_no_dot = resolve_executable_in_paths(
            "npm",
            vec![path1.clone(), path2.clone()],
            &pathexts_no_dot,
        );
        assert_eq!(resolved_npm_no_dot, path2.join("npm.cmd"));

        // Test PATH directory precedence: path1 comes before path2
        fs::write(path1.join("pnpm.cmd"), "@echo off\necho pnpm1").unwrap();
        fs::write(path2.join("pnpm.bat"), "@echo off\necho pnpm2").unwrap();
        let resolved_pnpm =
            resolve_executable_in_paths("pnpm", vec![path1.clone(), path2.clone()], &pathexts);
        assert_eq!(resolved_pnpm, path1.join("pnpm.cmd"));

        // Test PATHEXT extension priority within the same directory: .BAT before .CMD
        fs::write(path1.join("tool.bat"), "@echo off\necho tool bat").unwrap();
        fs::write(path1.join("tool.cmd"), "@echo off\necho tool cmd").unwrap();
        let resolved_tool =
            resolve_executable_in_paths("tool", vec![path1.clone()], &[".BAT", ".CMD"]);
        assert_eq!(resolved_tool, path1.join("tool.bat"));
    }

    #[test]
    fn test_resolve_executable_existing_file_with_extension() {
        let temp_dir = TempDir::new().unwrap();
        let path1 = temp_dir.path().join("bin");
        fs::create_dir_all(&path1).unwrap();

        fs::write(path1.join("mvn.cmd"), "@echo off\necho mvn").unwrap();
        fs::write(path1.join("git.exe"), "binary").unwrap();

        let pathexts = [".COM", ".EXE", ".BAT", ".CMD"];

        let resolved_mvn = resolve_executable_in_paths("mvn.cmd", vec![path1.clone()], &pathexts);
        assert_eq!(resolved_mvn, path1.join("mvn.cmd"));

        let resolved_git = resolve_executable_in_paths("git.exe", vec![path1.clone()], &pathexts);
        assert_eq!(resolved_git, path1.join("git.exe"));
    }

    #[test]
    fn test_resolve_executable_fallback_when_not_found() {
        let temp_dir = TempDir::new().unwrap();
        let path1 = temp_dir.path().join("bin");
        fs::create_dir_all(&path1).unwrap();

        let pathexts = [".COM", ".EXE", ".BAT", ".CMD"];

        // Bare command not in PATH falls back to original PathBuf
        let resolved_missing =
            resolve_executable_in_paths("missing_tool", vec![path1.clone()], &pathexts);
        assert_eq!(resolved_missing, std::path::PathBuf::from("missing_tool"));

        // Command with extension not in PATH falls back to original PathBuf
        let resolved_missing_ext =
            resolve_executable_in_paths("missing.exe", vec![path1.clone()], &pathexts);
        assert_eq!(
            resolved_missing_ext,
            std::path::PathBuf::from("missing.exe")
        );

        // Empty paths list falls back immediately
        let resolved_empty_paths = resolve_executable_in_paths("echo", Vec::new(), &pathexts);
        assert_eq!(resolved_empty_paths, std::path::PathBuf::from("echo"));
    }

    #[test]
    fn test_resolve_executable_path_prefixed_commands() {
        let temp_dir = TempDir::new().unwrap();

        // 1. Absolute path with existing file
        let abs_file = temp_dir.path().join("script.bat");
        fs::write(&abs_file, "@echo off").unwrap();
        let resolved_abs =
            resolve_executable_in_paths(abs_file.to_str().unwrap(), vec![], &[".BAT", ".CMD"]);
        assert_eq!(resolved_abs, abs_file);

        // 2. Absolute path without extension resolving to .cmd
        let abs_tool = temp_dir.path().join("my_tool");
        let abs_tool_cmd = temp_dir.path().join("my_tool.cmd");
        fs::write(&abs_tool_cmd, "@echo off").unwrap();
        let resolved_abs_tool =
            resolve_executable_in_paths(abs_tool.to_str().unwrap(), vec![], &[".BAT", ".CMD"]);
        assert_eq!(resolved_abs_tool, abs_tool_cmd);

        // 3. Absolute path nonexistent fallback
        let abs_missing = temp_dir.path().join("nonexistent_script");
        let resolved_abs_missing =
            resolve_executable_in_paths(abs_missing.to_str().unwrap(), vec![], &[".BAT", ".CMD"]);
        assert_eq!(resolved_abs_missing, abs_missing);

        // 4. Relative path with '/' resolving extension
        let unique_id = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let rel_dir = std::path::PathBuf::from(format!("target/test_probe_{unique_id}"));
        fs::create_dir_all(&rel_dir).unwrap();
        let rel_script_cmd = rel_dir.join("build.cmd");
        fs::write(&rel_script_cmd, "@echo off").unwrap();

        let rel_query = format!("target/test_probe_{unique_id}/build");
        let resolved_rel = resolve_executable_in_paths(&rel_query, vec![], &[".BAT", ".CMD"]);
        assert_eq!(resolved_rel, rel_script_cmd);

        // Clean up temporary relative directory
        let _ = fs::remove_dir_all(&rel_dir);

        // 5. Relative path nonexistent fallback with '/' and '\'
        let rel_missing_slash = "./does/not/exist/cmd";
        assert_eq!(
            resolve_executable_in_paths(rel_missing_slash, vec![], &[".CMD"]),
            std::path::PathBuf::from(rel_missing_slash)
        );

        let rel_missing_bs = r"does\not\exist\cmd";
        assert_eq!(
            resolve_executable_in_paths(rel_missing_bs, vec![], &[".CMD"]),
            std::path::PathBuf::from(rel_missing_bs)
        );
    }

    #[test]
    fn test_prepare_hook_command_spawns_correct_args() {
        let args = vec!["arg1".to_string(), "arg2".to_string()];
        let cmd = prepare_hook_command("echo", &args);

        #[cfg(not(windows))]
        {
            assert_eq!(cmd.get_program(), "echo");
        }

        let cmd_args: Vec<&std::ffi::OsStr> = cmd.get_args().collect();
        assert_eq!(cmd_args, vec!["arg1", "arg2"]);
    }

    /// A failed setup must not leave behind the branches it created.
    ///
    /// `create_worktree` makes the branch as part of `git worktree add -b`, and removing a
    /// worktree does not delete its branch — so without explicit cleanup the first repo's
    /// branch survived a rollback and the retry failed with `BranchAlreadyExists` instead.
    ///
    /// Note the TOML is built with *literal* (single-quoted) strings. Windows paths contain
    /// backslashes, and in a TOML basic string `\U` is a unicode escape.
    #[test]
    fn test_create_workspace_rollback_deletes_branches_it_created() {
        let temp_dir = TempDir::new().unwrap();
        let good_repo = temp_dir.path().join("good_repo");
        create_mock_git_repo(&good_repo);
        // Exists, but is not a git repository, so create_worktree fails for it.
        let bad_repo = temp_dir.path().join("not_a_repo");
        fs::create_dir_all(&bad_repo).unwrap();

        let toml_str = format!(
            r#"
            default_agent = "claude"
            repos_dir = '{}'
            [templates.tpl]
            repos = ['{}', '{}']
            "#,
            toml_path(temp_dir.path()),
            toml_path(&good_repo),
            toml_path(&bad_repo)
        );
        let config = Config::load_from_str(&toml_str).unwrap();

        let project_cwd = temp_dir.path().join("workspaces");
        fs::create_dir_all(&project_cwd).unwrap();

        let result = ProjectBuilder::new(config, "tpl", "rollback-proj")
            .with_cwd(&project_cwd)
            .create_workspace(|_ev| {});
        assert!(result.is_err(), "setup should fail on the second repo");

        // The branch created for the repo that succeeded must be gone, so the same project
        // name can be retried.
        assert!(
            !crate::git::branch_exists(&good_repo, "rollback-proj").unwrap(),
            "rollback must delete the branch it created in the first repo"
        );
        assert!(
            !project_cwd.join("rollback-proj").exists(),
            "rollback must remove the project directory"
        );
    }

    #[test]
    fn test_resolve_effective_repos_none_selection_filters_default_false() {
        let toml_str = r#"
            default_agent = "claude"
            repos_dir = ".repos"
            [templates.tpl]
            repos = [
                "repo_a",
                { repo = "repo_b", default = true },
                { repo = "repo_c", default = false }
            ]
        "#;
        let config = Config::load_from_str(toml_str).unwrap();
        let builder = ProjectBuilder::new(config, "tpl", "proj");
        assert!(builder.selected_repos.is_none());

        let effective = builder.resolve_effective_repos().unwrap();
        assert_eq!(effective.len(), 2);
        assert_eq!(effective[0].name, "repo_a");
        assert!(effective[0].default_include);
        assert_eq!(effective[1].name, "repo_b");
        assert!(effective[1].default_include);
        assert!(!effective.iter().any(|r| r.name == "repo_c"));
    }

    #[test]
    fn test_resolve_effective_repos_with_selected_repos_explicit_subset_including_default_false() {
        let toml_str = r#"
            default_agent = "claude"
            repos_dir = ".repos"
            [templates.tpl]
            repos = [
                "repo_a",
                { repo = "repo_b", default = true },
                { repo = "repo_c", default = false }
            ]
        "#;
        let config = Config::load_from_str(toml_str).unwrap();
        let builder = ProjectBuilder::new(config, "tpl", "proj")
            .with_selected_repos(vec!["repo_b", "repo_c"]);

        assert_eq!(
            builder.selected_repos,
            Some(vec!["repo_b".to_string(), "repo_c".to_string()])
        );

        let effective = builder.resolve_effective_repos().unwrap();
        assert_eq!(effective.len(), 2);
        assert_eq!(effective[0].name, "repo_b");
        assert_eq!(effective[1].name, "repo_c");
        assert!(!effective.iter().any(|r| r.name == "repo_a"));
    }

    #[test]
    fn test_resolve_effective_repos_nonexistent_repo_returns_error() {
        let toml_str = r#"
            default_agent = "claude"
            repos_dir = ".repos"
            [templates.tpl]
            repos = ["repo_a", "repo_b"]
        "#;
        let config = Config::load_from_str(toml_str).unwrap();
        let builder = ProjectBuilder::new(config, "tpl", "proj")
            .with_selected_repos(vec!["repo_a", "repo_missing"]);

        let err = builder.resolve_effective_repos().unwrap_err();
        match err {
            ProjectError::RepositoryNotInTemplate { name, template } => {
                assert_eq!(name, "repo_missing");
                assert_eq!(template, "tpl");
            }
            other => panic!("Expected RepositoryNotInTemplate, got: {other:?}"),
        }
    }

    #[test]
    fn test_resolve_effective_repos_empty_selection_returns_no_repositories_selected() {
        // Case A: explicitly selected empty vec
        let toml_str = r#"
            default_agent = "claude"
            repos_dir = ".repos"
            [templates.tpl]
            repos = ["repo_a", "repo_b"]
        "#;
        let config = Config::load_from_str(toml_str).unwrap();
        let builder =
            ProjectBuilder::new(config, "tpl", "proj").with_selected_repos(Vec::<String>::new());

        let err = builder.resolve_effective_repos().unwrap_err();
        assert!(matches!(err, ProjectError::NoRepositoriesSelected));

        // Case B: selected_repos is None, but all template repos have default = false
        let toml_str_all_false = r#"
            default_agent = "claude"
            repos_dir = ".repos"
            [templates.tpl]
            repos = [
                { repo = "repo_a", default = false },
                { repo = "repo_b", default = false }
            ]
        "#;
        let config_all_false = Config::load_from_str(toml_str_all_false).unwrap();
        let builder_all_false = ProjectBuilder::new(config_all_false, "tpl", "proj");
        let err_all_false = builder_all_false.resolve_effective_repos().unwrap_err();
        assert!(matches!(
            err_all_false,
            ProjectError::NoRepositoriesSelected
        ));
    }

    #[test]
    fn test_validate_and_branch_collisions_ignore_excluded_repos() {
        let temp_dir = TempDir::new().unwrap();
        let repo_included = temp_dir.path().join("repo_inc");
        create_mock_git_repo(&repo_included);

        let repo_excluded = temp_dir.path().join("repo_exc");
        create_mock_git_repo(&repo_excluded);

        // Create conflicting branch in repo_excluded ONLY
        let _ = std::process::Command::new("git")
            .args(["branch", "feat-collision"])
            .current_dir(&repo_excluded)
            .status();

        let toml_str = format!(
            r#"
            default_agent = "claude"
            repos_dir = "{}"
            [templates.tpl]
            repos = [
                "{}",
                {{ repo = "{}", default = false }}
            ]
            "#,
            toml_path(temp_dir.path()),
            toml_path(&repo_included),
            toml_path(&repo_excluded)
        );
        let config = Config::load_from_str(&toml_str).unwrap();

        // 1. With selected_repos = None (repo_exc is default = false, so excluded)
        let builder =
            ProjectBuilder::new(config.clone(), "tpl", "feat-collision").with_cwd(temp_dir.path());

        // Validate succeeds because repo_excluded is ignored!
        assert!(builder.validate().is_ok());

        // check_branch_collisions returns empty list
        let collisions = builder.check_branch_collisions();
        assert!(
            collisions.is_empty(),
            "Collisions should be empty: {collisions:?}"
        );

        // validate_detailed succeeds
        let detailed = builder.validate_detailed();
        assert!(detailed.is_valid);
        assert!(detailed.errors.is_empty());

        // 2. When repo_excluded is explicitly selected via with_selected_repos:
        let builder_with_exc = ProjectBuilder::new(config, "tpl", "feat-collision")
            .with_cwd(temp_dir.path())
            .with_selected_repos(vec!["repo_inc", "repo_exc"]);

        // Validate now fails with BranchExists in repo_exc
        let err = builder_with_exc.validate().unwrap_err();
        match err {
            ProjectError::BranchExists { repo, branch } => {
                assert_eq!(branch, "feat-collision");
                assert_eq!(repo, repo_excluded);
            }
            other => panic!("Expected BranchExists, got: {other:?}"),
        }

        // check_branch_collisions reports the collision in repo_exc
        let collisions = builder_with_exc.check_branch_collisions();
        assert_eq!(collisions.len(), 1);
        assert!(collisions[0].contains("repo_exc"));

        // validate_detailed fails
        let detailed = builder_with_exc.validate_detailed();
        assert!(!detailed.is_valid);
        assert!(detailed.primary_message().unwrap().contains("repo_exc"));
    }

    #[test]
    fn test_create_workspace_only_creates_worktrees_for_effective_repos_and_agents_md() {
        let temp_dir = TempDir::new().unwrap();
        let repo_backend = temp_dir.path().join("backend");
        create_mock_git_repo(&repo_backend);

        let repo_frontend = temp_dir.path().join("frontend");
        create_mock_git_repo(&repo_frontend);

        let repo_docs = temp_dir.path().join("docs");
        create_mock_git_repo(&repo_docs);

        let toml_str = format!(
            r#"
            default_agent = "claude"
            repos_dir = "{}"
            [templates.fullstack]
            description = "Fullstack application services"
            repos = [
                "{}",
                "{}",
                {{ repo = "{}", default = false }}
            ]
            "#,
            toml_path(temp_dir.path()),
            toml_path(&repo_backend),
            toml_path(&repo_frontend),
            toml_path(&repo_docs)
        );
        let config = Config::load_from_str(&toml_str).unwrap();

        let project_cwd = temp_dir.path().join("workspaces");
        fs::create_dir_all(&project_cwd).unwrap();

        // Select only backend and docs (omitting frontend, and including optional docs)
        let builder = ProjectBuilder::new(config, "fullstack", "selective-proj")
            .with_cwd(&project_cwd)
            .with_selected_repos(vec!["backend", "docs"]);

        let (project_dir, mappings) = builder
            .create_workspace(|_event| {})
            .expect("create_workspace should succeed");

        // 1. Check mappings
        assert_eq!(mappings.len(), 2);
        assert_eq!(mappings[0].name, "backend");
        assert_eq!(mappings[1].name, "docs");

        // 2. Check filesystem: backend and docs worktrees exist, frontend does NOT exist
        assert!(project_dir.join("backend").exists());
        assert!(project_dir.join("backend").join("README.md").exists());
        assert!(project_dir.join("docs").exists());
        assert!(project_dir.join("docs").join("README.md").exists());
        assert!(
            !project_dir.join("frontend").exists(),
            "frontend should NOT be checked out because it was not in selected_repos"
        );

        // 3. Verify AGENTS.md context orientation file
        let agents_md_path = project_dir.join("AGENTS.md");
        assert!(agents_md_path.is_file());
        let agents_md_content = fs::read_to_string(&agents_md_path).unwrap();

        // Must list backend and docs
        assert!(agents_md_content.contains("backend"));
        assert!(agents_md_content.contains("docs"));
        // Must NOT list frontend
        assert!(
            !agents_md_content.contains("frontend"),
            "AGENTS.md should only list effective repositories"
        );
    }

    #[test]
    fn test_validate_fast_empty_selected_repos() {
        let toml_str = r#"
            default_agent = "claude"
            repos_dir = ".repos"
            [templates.tpl]
            repos = ["repo_a"]
        "#;
        let config = Config::load_from_str(toml_str).unwrap();
        let builder = ProjectBuilder::new(config, "tpl", "my-project")
            .with_selected_repos(Vec::<String>::new());

        let fast_val = builder.validate_fast();
        assert!(!fast_val.is_valid);
        assert_eq!(
            fast_val.primary_message(),
            Some("At least one repository must be selected")
        );
    }

    #[test]
    fn test_step2_parallel_cloning_and_fetching() {
        let temp_dir = TempDir::new().unwrap();
        let upstream1 = temp_dir.path().join("upstream1");
        create_mock_git_repo(&upstream1);

        let upstream2 = temp_dir.path().join("upstream2");
        create_mock_git_repo(&upstream2);

        let local_repo = temp_dir.path().join("local_repo");
        create_mock_git_repo(&local_repo);

        let repos_dir = temp_dir.path().join("cached_repos");
        let project_cwd = temp_dir.path().join("workspaces");
        fs::create_dir_all(&project_cwd).unwrap();

        let toml_str = format!(
            r#"
            default_agent = "claude"
            repos_dir = "{}"
            [templates.parallel_tpl]
            description = "Parallel cloning test template"
            repos = [
                "file://{}",
                "file://{}",
                "{}"
            ]
            "#,
            toml_path(&repos_dir),
            toml_path(&upstream1),
            toml_path(&upstream2),
            toml_path(&local_repo)
        );
        let config = Config::load_from_str(&toml_str).unwrap();
        let builder =
            ProjectBuilder::new(config, "parallel_tpl", "parallel-proj").with_cwd(&project_cwd);

        let events = Arc::new(Mutex::new(Vec::new()));
        let events_clone = Arc::clone(&events);

        let (project_dir, mappings) = builder
            .create_workspace(move |event| {
                events_clone.lock().unwrap().push(event);
            })
            .expect("create_workspace should succeed with parallel clone and fetch");

        assert_eq!(mappings.len(), 3);
        assert!(project_dir.join("upstream1").exists());
        assert!(project_dir.join("upstream2").exists());
        assert!(project_dir.join("local_repo").exists());

        // Verify cached repos were cloned into repos_dir
        assert!(repos_dir.join("upstream1").join("README.md").exists());
        assert!(repos_dir.join("upstream2").join("README.md").exists());

        // Verify events logged
        let evs = events.lock().unwrap();
        let log_messages: Vec<String> = evs
            .iter()
            .filter_map(|e| match e {
                ProjectSetupEvent::Log(msg) => Some(msg.clone()),
                _ => None,
            })
            .collect();

        assert!(
            log_messages
                .iter()
                .any(|m| m.contains("Cloning repository 'upstream1'")),
            "Expected cloning log for upstream1"
        );
        assert!(
            log_messages
                .iter()
                .any(|m| m.contains("Cloning repository 'upstream2'")),
            "Expected cloning log for upstream2"
        );
        assert!(
            log_messages
                .iter()
                .any(|m| m.contains("Fetching latest updates for repository 'local_repo'")),
            "Expected fetch log for local_repo"
        );
    }

    #[test]
    fn test_step2_missing_repo_without_url_error_propagation() {
        let temp_dir = TempDir::new().unwrap();
        let project_cwd = temp_dir.path().join("workspaces");
        fs::create_dir_all(&project_cwd).unwrap();

        let toml_str = format!(
            r#"
            default_agent = "claude"
            repos_dir = "{}"
            [templates.missing_tpl]
            repos = ["nonexistent_bare_repo"]
            "#,
            toml_path(temp_dir.path())
        );
        let config = Config::load_from_str(&toml_str).unwrap();
        let builder =
            ProjectBuilder::new(config, "missing_tpl", "missing-proj").with_cwd(&project_cwd);

        let events = Arc::new(Mutex::new(Vec::new()));
        let events_clone = Arc::clone(&events);

        let res = builder.create_workspace(move |event| {
            events_clone.lock().unwrap().push(event);
        });

        assert!(
            res.is_err(),
            "create_workspace must fail for missing repo with no URL"
        );
        let err_msg = res.unwrap_err().to_string();
        assert!(
            err_msg.contains("does not exist locally and has no remote URL configured"),
            "Error must mention missing remote URL, got: {err_msg}"
        );

        let evs = events.lock().unwrap();
        let has_error_event = evs.iter().any(|e| match e {
            ProjectSetupEvent::Error(msg) => {
                msg.contains("does not exist locally and has no remote URL configured")
            }
            _ => false,
        });
        assert!(
            has_error_event,
            "Sink must receive ProjectSetupEvent::Error for missing remote URL"
        );
    }

    #[test]
    fn test_step2_clone_failure_error_propagation() {
        let temp_dir = TempDir::new().unwrap();
        let project_cwd = temp_dir.path().join("workspaces");
        fs::create_dir_all(&project_cwd).unwrap();

        let invalid_url = "file:///nonexistent/repo_invalid.git";
        let toml_str = format!(
            r#"
            default_agent = "claude"
            repos_dir = "{}"
            [templates.invalid_clone_tpl]
            repos = ["{}"]
            "#,
            toml_path(temp_dir.path()),
            invalid_url
        );
        let config = Config::load_from_str(&toml_str).unwrap();
        let builder =
            ProjectBuilder::new(config, "invalid_clone_tpl", "invalid-proj").with_cwd(&project_cwd);

        let events = Arc::new(Mutex::new(Vec::new()));
        let events_clone = Arc::clone(&events);

        let res = builder.create_workspace(move |event| {
            events_clone.lock().unwrap().push(event);
        });

        assert!(
            res.is_err(),
            "create_workspace must fail when git clone fails"
        );
        let err_msg = res.unwrap_err().to_string();
        assert!(
            err_msg.contains("Failed to clone repository"),
            "Error message must indicate clone failure, got: {err_msg}"
        );

        let evs = events.lock().unwrap();
        let has_error_event = evs.iter().any(|e| match e {
            ProjectSetupEvent::Error(msg) => msg.contains("Failed to clone repository"),
            _ => false,
        });
        assert!(
            has_error_event,
            "Sink must receive ProjectSetupEvent::Error when clone fails"
        );
    }

    #[test]
    fn test_step2_multiple_missing_repos_errors_propagated() {
        let temp_dir = TempDir::new().unwrap();
        let project_cwd = temp_dir.path().join("workspaces");
        fs::create_dir_all(&project_cwd).unwrap();

        let toml_str = format!(
            r#"
            default_agent = "claude"
            repos_dir = "{}"
            [templates.multi_fail_tpl]
            repos = ["missing_alpha", "missing_beta"]
            "#,
            toml_path(temp_dir.path())
        );
        let config = Config::load_from_str(&toml_str).unwrap();
        let builder =
            ProjectBuilder::new(config, "multi_fail_tpl", "multi-fail-proj").with_cwd(&project_cwd);

        let events = Arc::new(Mutex::new(Vec::new()));
        let events_clone = Arc::clone(&events);

        let res = builder.create_workspace(move |event| {
            events_clone.lock().unwrap().push(event);
        });

        assert!(res.is_err());
        let err_msg = res.unwrap_err().to_string();
        assert!(err_msg.contains("missing_alpha") || err_msg.contains("missing_beta"));

        let evs = events.lock().unwrap();
        let error_events: Vec<String> = evs
            .iter()
            .filter_map(|e| match e {
                ProjectSetupEvent::Error(msg) => Some(msg.clone()),
                _ => None,
            })
            .collect();

        assert!(
            error_events.iter().any(|e| e.contains("missing_alpha")),
            "Expected missing_alpha error event"
        );
        assert!(
            error_events.iter().any(|e| e.contains("missing_beta")),
            "Expected missing_beta error event"
        );
    }

    #[test]
    fn test_concurrent_worktree_creation_multiple_repositories() {
        let temp_dir = TempDir::new().unwrap();
        let repos_dir = temp_dir.path().join("repos");
        let repo_a = repos_dir.join("repo_a");
        let repo_b = repos_dir.join("repo_b");
        let repo_c = repos_dir.join("repo_c");
        create_mock_git_repo(&repo_a);
        create_mock_git_repo(&repo_b);
        create_mock_git_repo(&repo_c);

        let toml_str = format!(
            r#"
            default_agent = "claude"
            repos_dir = "{}"

            [templates.multi_tpl]
            repos = ["repo_a", "repo_b", "repo_c"]
            "#,
            toml_path(&repos_dir)
        );
        let config = Config::load_from_str(&toml_str).unwrap();
        let project_cwd = temp_dir.path().join("workspaces");
        fs::create_dir_all(&project_cwd).unwrap();

        let builder =
            ProjectBuilder::new(config, "multi_tpl", "parallel-proj").with_cwd(&project_cwd);

        let events = Arc::new(Mutex::new(Vec::new()));
        let events_clone = Arc::clone(&events);

        let res = builder.create_workspace(move |ev| {
            events_clone.lock().unwrap().push(ev);
        });

        assert!(
            res.is_ok(),
            "create_workspace should succeed for multiple repos"
        );
        let (project_dir, mappings) = res.unwrap();
        assert_eq!(mappings.len(), 3);
        assert_eq!(mappings[0].name, "repo_a");
        assert_eq!(mappings[1].name, "repo_b");
        assert_eq!(mappings[2].name, "repo_c");

        for m in &mappings {
            assert!(m.worktree_path.exists(), "worktree must exist on disk");
            assert!(
                crate::git::branch_exists(&m.original_path, "parallel-proj").unwrap(),
                "branch parallel-proj must exist in backing repo"
            );
        }

        assert!(project_dir.join("AGENTS.md").exists());

        let evs = events.lock().unwrap();
        let log_count = evs
            .iter()
            .filter(|e| match e {
                ProjectSetupEvent::Log(msg) => msg.contains("Creating git worktree for"),
                _ => false,
            })
            .count();
        assert_eq!(log_count, 3, "must stream creation logs for all 3 repos");
    }

    #[test]
    fn test_concurrent_worktree_creation_failure_rollback_and_branch_deletion() {
        let temp_dir = TempDir::new().unwrap();
        let repos_dir = temp_dir.path().join("repos");
        let repo_ok1 = repos_dir.join("repo_ok1");
        let repo_ok2 = repos_dir.join("repo_ok2");
        let repo_bad = repos_dir.join("repo_bad");
        create_mock_git_repo(&repo_ok1);
        create_mock_git_repo(&repo_ok2);
        // Exists on disk, but not initialized as a git repo -> create_worktree fails
        fs::create_dir_all(&repo_bad).unwrap();

        let toml_str = format!(
            r#"
            default_agent = "claude"
            repos_dir = "{}"

            [templates.fail_tpl]
            repos = ["repo_ok1", "repo_ok2", "repo_bad"]
            "#,
            toml_path(&repos_dir)
        );
        let config = Config::load_from_str(&toml_str).unwrap();
        let project_cwd = temp_dir.path().join("workspaces");
        fs::create_dir_all(&project_cwd).unwrap();

        let builder = ProjectBuilder::new(config, "fail_tpl", "fail-proj").with_cwd(&project_cwd);

        let res = builder.create_workspace(|_| {});
        assert!(
            res.is_err(),
            "create_workspace must fail when a repo cannot be created"
        );

        let err_msg = res.unwrap_err().to_string();
        assert!(err_msg.contains("repo_bad"));

        // Verify atomic rollback:
        // 1. Branches created in repo_ok1 and repo_ok2 must be deleted
        assert!(
            !crate::git::branch_exists(&repo_ok1, "fail-proj").unwrap(),
            "rollback must delete branch in repo_ok1"
        );
        assert!(
            !crate::git::branch_exists(&repo_ok2, "fail-proj").unwrap(),
            "rollback must delete branch in repo_ok2"
        );

        // 2. Project directory must be completely removed
        assert!(
            !project_cwd.join("fail-proj").exists(),
            "rollback must remove project directory"
        );
    }

    #[test]
    fn test_two_phase_concurrent_teardown_clean_repos() {
        let temp_dir = TempDir::new().unwrap();
        let repos_dir = temp_dir.path().join("repos");
        let repo_1 = repos_dir.join("repo_1");
        let repo_2 = repos_dir.join("repo_2");
        let repo_3 = repos_dir.join("repo_3");
        create_mock_git_repo(&repo_1);
        create_mock_git_repo(&repo_2);
        create_mock_git_repo(&repo_3);

        let toml_str = format!(
            r#"
            default_agent = "claude"
            repos_dir = "{}"

            [templates.clean_tpl]
            repos = ["repo_1", "repo_2", "repo_3"]
            "#,
            toml_path(&repos_dir)
        );
        let config = Config::load_from_str(&toml_str).unwrap();
        let project_cwd = temp_dir.path().join("workspaces");
        fs::create_dir_all(&project_cwd).unwrap();

        let builder =
            ProjectBuilder::new(config.clone(), "clean_tpl", "clean-proj").with_cwd(&project_cwd);
        let (project_dir, _) = builder.create_workspace(|_| {}).unwrap();
        assert!(project_dir.exists());

        let events = Arc::new(Mutex::new(Vec::new()));
        let events_clone = Arc::clone(&events);

        let res = destroy_project_workspace_with_progress(
            Some(&config),
            &project_cwd,
            "clean-proj",
            false,
            move |ev| {
                events_clone.lock().unwrap().push(ev);
            },
        );

        assert!(
            res.is_ok(),
            "destroying clean workspace without force must succeed"
        );
        assert!(!project_dir.exists(), "project directory must be removed");

        let evs = events.lock().unwrap();
        // Validation step passed
        assert!(evs.iter().any(|e| match e {
            ProjectDestroyEvent::Log(msg) => msg.contains("Worktree validation passed"),
            _ => false,
        }));
        // All 3 worktrees removed in Step 3
        for r in &["repo_1", "repo_2", "repo_3"] {
            assert!(evs.iter().any(|e| match e {
                ProjectDestroyEvent::Log(msg) => msg.contains(&format!("Removed worktree '{r}'")),
                _ => false,
            }));
        }
        assert!(evs
            .iter()
            .any(|e| matches!(e, ProjectDestroyEvent::Success)));
    }

    #[test]
    fn test_two_phase_concurrent_teardown_aborts_and_reports_multiple_dirty_repos() {
        let temp_dir = TempDir::new().unwrap();
        let repos_dir = temp_dir.path().join("repos");
        let repo_clean = repos_dir.join("repo_clean");
        let repo_dirty_a = repos_dir.join("repo_dirty_a");
        let repo_dirty_b = repos_dir.join("repo_dirty_b");
        create_mock_git_repo(&repo_clean);
        create_mock_git_repo(&repo_dirty_a);
        create_mock_git_repo(&repo_dirty_b);

        let toml_str = format!(
            r#"
            default_agent = "claude"
            repos_dir = "{}"

            [templates.dirty_tpl]
            repos = ["repo_clean", "repo_dirty_a", "repo_dirty_b"]
            "#,
            toml_path(&repos_dir)
        );
        let config = Config::load_from_str(&toml_str).unwrap();
        let project_cwd = temp_dir.path().join("workspaces");
        fs::create_dir_all(&project_cwd).unwrap();

        let builder =
            ProjectBuilder::new(config.clone(), "dirty_tpl", "dirty-proj").with_cwd(&project_cwd);
        let (project_dir, _) = builder.create_workspace(|_| {}).unwrap();
        assert!(project_dir.exists());

        // Introduce uncommitted changes in both dirty_a and dirty_b
        fs::write(
            project_dir.join("repo_dirty_a").join("uncommitted.txt"),
            "changes in a",
        )
        .unwrap();
        fs::write(
            project_dir.join("repo_dirty_b").join("README.md"),
            "# Modified dirty b\n",
        )
        .unwrap();

        let events = Arc::new(Mutex::new(Vec::new()));
        let events_clone = Arc::clone(&events);

        // Teardown without force must abort at validation phase
        let res = destroy_project_workspace_with_progress(
            Some(&config),
            &project_cwd,
            "dirty-proj",
            false,
            move |ev| {
                events_clone.lock().unwrap().push(ev);
            },
        );

        assert!(
            res.is_err(),
            "must abort when dirty repos exist without force"
        );
        let err_msg = res.unwrap_err().to_string();

        // Safety Barrier: must report both dirty repos in aggregated message
        assert!(err_msg.contains("Worktree(s) have uncommitted changes:"));
        assert!(err_msg.contains("'repo_dirty_a'"));
        assert!(err_msg.contains("'repo_dirty_b'"));
        assert!(!err_msg.contains("'repo_clean'"));
        assert!(err_msg.contains("Use --force to destroy anyway"));

        // Safety Barrier: NONE of the worktrees should have been removed!
        assert!(project_dir.exists());
        assert!(project_dir.join("repo_clean").exists());
        assert!(project_dir.join("repo_dirty_a").exists());
        assert!(project_dir.join("repo_dirty_b").exists());

        // Now destroy with force -> must succeed and remove everything
        let res_force = destroy_project_workspace_with_progress(
            Some(&config),
            &project_cwd,
            "dirty-proj",
            true,
            |_| {},
        );
        assert!(res_force.is_ok(), "destroy with force must succeed");
        assert!(
            !project_dir.exists(),
            "project directory must be removed after forced destroy"
        );
    }
}
