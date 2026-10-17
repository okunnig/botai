use std::path::{Path, PathBuf};
use thiserror::Error;

/// Errors that can occur during Git operations and worktree management.
#[derive(Debug, Error)]
pub enum GitError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Git command failed: {command}\nStderr: {stderr}")]
    CommandFailed { command: String, stderr: String },

    #[error("Repository not found at: {0}")]
    RepoNotFound(PathBuf),

    #[error("Branch '{branch}' already exists in repository '{repo}'")]
    BranchAlreadyExists { branch: String, repo: PathBuf },

    #[error(
        "Branch '{branch}' is already checked out at '{worktree_path}' in repository '{repo}'"
    )]
    BranchAlreadyCheckedOut {
        branch: String,
        repo: PathBuf,
        worktree_path: PathBuf,
    },

    #[error("Worktree path already exists at: {0}")]
    WorktreePathExists(PathBuf),

    #[error("Worktree not found at: {0}")]
    WorktreeNotFound(PathBuf),

    #[error("Worktree at '{path}' has uncommitted changes")]
    WorktreeDirty { path: PathBuf },

    #[error("Could not determine default branch for repository '{0}'")]
    DefaultBranchNotFound(PathBuf),

    #[error("Invalid branch name: '{0}'")]
    InvalidBranchName(String),

    #[error("Failed to clone repository from '{url}' to '{path}': {stderr}")]
    CloneFailed {
        url: String,
        path: PathBuf,
        stderr: String,
    },
}

/// Trait defining Git worktree management operations.
pub trait WorktreeManager {
    /// Creates a new git worktree for `repo_path` located at `worktree_path`
    /// checked out to a new branch `branch_name` branched off the default branch.
    fn create_worktree(
        &self,
        repo_path: &Path,
        worktree_path: &Path,
        branch_name: &str,
    ) -> Result<(), GitError>;

    /// Creates a git worktree at `worktree_path` for `branch_name`.
    /// When `checkout_existing` is true, checks out an existing local branch (returning `Ok(false)`),
    /// tracks `origin/<branch_name>` if present (returning `Ok(true)`), or falls back to creating a
    /// new branch off the default branch (returning `Ok(true)`).
    fn create_worktree_with_options(
        &self,
        repo_path: &Path,
        worktree_path: &Path,
        branch_name: &str,
        checkout_existing: bool,
    ) -> Result<bool, GitError>;

    /// Returns the worktree path if `branch_name` is currently checked out in `repo_path` or any of its worktrees.
    fn find_worktree_for_branch(
        &self,
        repo_path: &Path,
        branch_name: &str,
    ) -> Result<Option<PathBuf>, GitError>;

    /// Safely removes a git worktree at `worktree_path`.
    /// If `force` is false, returns `GitError::WorktreeDirty` if uncommitted changes exist.
    /// Preserves the git branch in the repository history.
    fn remove_worktree(
        &self,
        repo_path: &Path,
        worktree_path: &Path,
        force: bool,
    ) -> Result<(), GitError>;

    /// Automatically discovers the main repository backing `worktree_path`.
    fn resolve_main_repo(&self, worktree_path: &Path) -> Result<PathBuf, GitError>;

    /// Automatically resolves the backing repository for `worktree_path` and cleanly removes the worktree.
    /// If `force` is false, returns `GitError::WorktreeDirty` if uncommitted changes exist.
    fn remove_worktree_auto(&self, worktree_path: &Path, force: bool) -> Result<(), GitError>;

    /// Checks if a worktree at `worktree_path` has uncommitted (modified, staged, or untracked) changes.
    fn has_uncommitted_changes(&self, worktree_path: &Path) -> Result<bool, GitError>;

    /// Determines the default branch (`main` or `master`) of the repository.
    fn get_default_branch(&self, repo_path: &Path) -> Result<String, GitError>;

    /// Checks if a branch exists in the repository.
    fn branch_exists(&self, repo_path: &Path, branch_name: &str) -> Result<bool, GitError>;

    /// Force-deletes a branch from the repository.
    fn delete_branch(&self, repo_path: &Path, branch_name: &str) -> Result<(), GitError>;

    /// Gets the currently checked out branch name of a repo or worktree.
    fn get_current_branch(&self, path: &Path) -> Result<String, GitError>;
}

/// Default implementation of git operations using the `git` CLI.
#[derive(Debug, Default, Clone)]
pub struct GitWorktreeManager;

use std::process::Command;

/// Checks whether a given path is an existing, valid Git repository.
pub fn ensure_git_repo(repo_path: &Path) -> Result<(), GitError> {
    if !repo_path.exists() {
        return Err(GitError::RepoNotFound(repo_path.to_path_buf()));
    }
    let output = Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(repo_path)
        .output()?;
    if !output.status.success() {
        return Err(GitError::RepoNotFound(repo_path.to_path_buf()));
    }
    Ok(())
}

impl GitWorktreeManager {
    pub fn new() -> Self {
        Self
    }

    /// Clones a remote repository to `target_path`. If already an existing git repo, succeeds as no-op.
    pub fn clone_repo(url: &str, target_path: &Path) -> Result<(), GitError> {
        if target_path.exists() && ensure_git_repo(target_path).is_ok() {
            return Ok(());
        }

        if let Some(parent) = target_path.parent() {
            if !parent.as_os_str().is_empty() && !parent.exists() {
                std::fs::create_dir_all(parent)?;
            }
        }

        let output = Command::new("git")
            .arg("clone")
            .arg(url)
            .arg(target_path)
            .output()?;

        if !output.status.success() {
            return Err(GitError::CloneFailed {
                url: url.to_string(),
                path: target_path.to_path_buf(),
                stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            });
        }

        Ok(())
    }

    /// Performs a best-effort `git fetch origin` for `repo_path`.
    pub fn fetch_repo(repo_path: &Path) -> Result<(), GitError> {
        ensure_git_repo(repo_path)?;
        let output = Command::new("git")
            .args(["fetch", "origin"])
            .current_dir(repo_path)
            .output()?;

        if !output.status.success() {
            return Err(GitError::CommandFailed {
                command: "git fetch origin".to_string(),
                stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            });
        }

        Ok(())
    }

    /// Checks if a remote branch exists (e.g. `origin/main`).
    pub fn remote_branch_exists(repo_path: &Path, remote_branch: &str) -> Result<bool, GitError> {
        ensure_git_repo(repo_path)?;
        let ref_name = format!("refs/remotes/{remote_branch}");
        let output = Command::new("git")
            .args(["rev-parse", "--verify", &ref_name])
            .current_dir(repo_path)
            .output()?;
        Ok(output.status.success())
    }

    /// Returns the worktree path if `branch_name` is currently checked out in `repo_path` or any of its worktrees.
    pub fn find_worktree_for_branch(
        repo_path: &Path,
        branch_name: &str,
    ) -> Result<Option<PathBuf>, GitError> {
        ensure_git_repo(repo_path)?;
        if branch_name.trim().is_empty() {
            return Ok(None);
        }

        // Best-effort prune of stale worktree metadata first
        let _ = Command::new("git")
            .args(["worktree", "prune"])
            .current_dir(repo_path)
            .output();

        let output = Command::new("git")
            .args(["worktree", "list", "--porcelain"])
            .current_dir(repo_path)
            .output()?;

        if !output.status.success() {
            return Err(GitError::CommandFailed {
                command: "git worktree list --porcelain".to_string(),
                stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            });
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let target_ref = format!("refs/heads/{branch_name}");
        let mut current_wt: Option<PathBuf> = None;

        for line in stdout.lines() {
            let line = line.trim_end_matches('\r');
            if line.is_empty() {
                current_wt = None;
            } else if let Some(wt_str) = line.strip_prefix("worktree ") {
                current_wt = Some(PathBuf::from(wt_str.trim()));
            } else if let Some(branch_ref) = line.strip_prefix("branch ") {
                if branch_ref.trim() == target_ref {
                    if let Some(wt) = current_wt {
                        return Ok(Some(wt));
                    }
                }
            }
        }

        Ok(None)
    }

    /// Creates a worktree at `worktree_path` checked out to `branch_name` branched off default branch (or upstream default branch).
    pub fn create_worktree(
        repo_path: &Path,
        worktree_path: &Path,
        branch_name: &str,
    ) -> Result<(), GitError> {
        Self::create_worktree_with_options(repo_path, worktree_path, branch_name, false).map(|_| ())
    }

    /// Creates a worktree at `worktree_path` for `branch_name` with optional existing branch checkout.
    ///
    /// Returns `Ok(true)` if a new local branch (`refs/heads/<branch_name>`) was created by this call,
    /// or `Ok(false)` if a pre-existing local branch was checked out as-is.
    pub fn create_worktree_with_options(
        repo_path: &Path,
        worktree_path: &Path,
        branch_name: &str,
        checkout_existing: bool,
    ) -> Result<bool, GitError> {
        ensure_git_repo(repo_path)?;

        // Prune stale worktree metadata in case directories were manually deleted
        let _ = Command::new("git")
            .args(["worktree", "prune"])
            .current_dir(repo_path)
            .output();

        if branch_name.trim().is_empty() {
            return Err(GitError::InvalidBranchName(branch_name.to_string()));
        }

        if worktree_path.exists() {
            return Err(GitError::WorktreePathExists(worktree_path.to_path_buf()));
        }

        if let Some(existing_wt) = Self::find_worktree_for_branch(repo_path, branch_name)? {
            return Err(GitError::BranchAlreadyCheckedOut {
                branch: branch_name.to_string(),
                repo: repo_path.to_path_buf(),
                worktree_path: existing_wt,
            });
        }

        if let Some(parent) = worktree_path.parent() {
            if !parent.as_os_str().is_empty() && !parent.exists() {
                std::fs::create_dir_all(parent)?;
            }
        }

        if checkout_existing {
            // Case A: Local branch exists -> checkout existing local branch as-is
            if Self::branch_exists(repo_path, branch_name)? {
                let output = Command::new("git")
                    .args(["worktree", "add"])
                    .arg(worktree_path)
                    .arg(branch_name)
                    .current_dir(repo_path)
                    .output()?;

                if !output.status.success() {
                    return Err(GitError::CommandFailed {
                        command: format!("git worktree add {worktree_path:?} {branch_name}"),
                        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
                    });
                }

                return Ok(false);
            }

            // Case B: Local branch does not exist, but origin/<branch_name> exists -> track remote branch
            let remote_ref = format!("origin/{branch_name}");
            if Self::remote_branch_exists(repo_path, &remote_ref).unwrap_or(false) {
                let output = Command::new("git")
                    .args(["worktree", "add", "--track", "-b", branch_name])
                    .arg(worktree_path)
                    .arg(&remote_ref)
                    .current_dir(repo_path)
                    .output()?;

                if !output.status.success() {
                    return Err(GitError::CommandFailed {
                        command: format!(
                            "git worktree add --track -b {branch_name} {worktree_path:?} {remote_ref}"
                        ),
                        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
                    });
                }

                return Ok(true);
            }
        } else if Self::branch_exists(repo_path, branch_name)? {
            return Err(GitError::BranchAlreadyExists {
                branch: branch_name.to_string(),
                repo: repo_path.to_path_buf(),
            });
        }

        // Case C (or !checkout_existing): Create new branch off default branch (or origin/<default_branch>)
        let default_branch = Self::get_default_branch(repo_path)?;
        let origin_ref = format!("origin/{default_branch}");
        let branch_target = if Self::remote_branch_exists(repo_path, &origin_ref).unwrap_or(false) {
            origin_ref
        } else {
            default_branch
        };

        let output = Command::new("git")
            .args(["worktree", "add", "-b", branch_name])
            .arg(worktree_path)
            .arg(&branch_target)
            .current_dir(repo_path)
            .output()?;

        if !output.status.success() {
            return Err(GitError::CommandFailed {
                command: format!(
                    "git worktree add -b {branch_name} {worktree_path:?} {branch_target}"
                ),
                stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            });
        }

        Ok(true)
    }

    /// Safely removes a worktree. Errors if dirty unless `force` is true. Preserves the branch.
    pub fn remove_worktree(
        repo_path: &Path,
        worktree_path: &Path,
        force: bool,
    ) -> Result<(), GitError> {
        ensure_git_repo(repo_path)?;

        if !worktree_path.exists() {
            return Err(GitError::WorktreeNotFound(worktree_path.to_path_buf()));
        }

        if !force && Self::has_uncommitted_changes(worktree_path)? {
            return Err(GitError::WorktreeDirty {
                path: worktree_path.to_path_buf(),
            });
        }

        let abs_worktree_path = if worktree_path.is_absolute() {
            worktree_path.to_path_buf()
        } else {
            std::env::current_dir()
                .map(|cwd| cwd.join(worktree_path))
                .unwrap_or_else(|_| worktree_path.to_path_buf())
        };
        // `dunce`: a positional git argument, which rejects a `\\?\` path.
        let wt_arg = dunce::canonicalize(&abs_worktree_path).unwrap_or(abs_worktree_path);

        let mut cmd = Command::new("git");
        cmd.arg("worktree").arg("remove");
        if force {
            cmd.arg("--force");
        }
        cmd.arg(&wt_arg);
        cmd.current_dir(repo_path);

        let output = cmd.output()?;

        // Always prune stale worktree metadata on repo_path after git worktree remove
        let _ = Command::new("git")
            .args(["worktree", "prune"])
            .current_dir(repo_path)
            .output();

        if !output.status.success() {
            return Err(GitError::CommandFailed {
                command: format!(
                    "git worktree remove{}{:?}",
                    if force { " --force " } else { " " },
                    wt_arg
                ),
                stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            });
        }

        if worktree_path.exists() {
            let _ = std::fs::remove_dir_all(worktree_path);
        }

        Ok(())
    }

    /// Resolves the main repository path backing a worktree directory.
    pub fn resolve_main_repo(worktree_path: &Path) -> Result<PathBuf, GitError> {
        if !worktree_path.exists() {
            return Err(GitError::WorktreeNotFound(worktree_path.to_path_buf()));
        }

        // 1. Try running `git rev-parse --git-common-dir`
        if let Ok(output) = Command::new("git")
            .args(["rev-parse", "--git-common-dir"])
            .current_dir(worktree_path)
            .output()
        {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !stdout.is_empty() {
                    let common_path = PathBuf::from(&stdout);
                    let abs_common = if common_path.is_absolute() {
                        common_path
                    } else {
                        worktree_path.join(common_path)
                    };
                    let canon_common = dunce::canonicalize(&abs_common).unwrap_or(abs_common);

                    // If common dir ends with .git, its parent is the repo root
                    if canon_common.file_name().and_then(|n| n.to_str()) == Some(".git") {
                        if let Some(parent) = canon_common.parent() {
                            let parent_buf = parent.to_path_buf();
                            if ensure_git_repo(&parent_buf).is_ok() {
                                return Ok(parent_buf);
                            }
                        }
                    }

                    // Otherwise if the common dir itself is a valid git repo (e.g. bare repo)
                    if ensure_git_repo(&canon_common).is_ok() {
                        return Ok(canon_common);
                    }
                }
            }
        }

        // 2. Fallback: read .git file in worktree_path
        let git_file = worktree_path.join(".git");
        if git_file.is_file() {
            if let Ok(content) = std::fs::read_to_string(&git_file) {
                for line in content.lines() {
                    let trimmed = line.trim();
                    if let Some(gitdir_str) = trimmed.strip_prefix("gitdir:") {
                        let gitdir_str = gitdir_str.trim();
                        let gitdir_path = PathBuf::from(gitdir_str);
                        let abs_gitdir = if gitdir_path.is_absolute() {
                            gitdir_path
                        } else {
                            worktree_path.join(gitdir_path)
                        };
                        let canon_gitdir = dunce::canonicalize(&abs_gitdir).unwrap_or(abs_gitdir);

                        // Walk up ancestors looking for main repo root
                        let mut current = Some(canon_gitdir.as_path());
                        while let Some(p) = current {
                            if p.file_name().and_then(|n| n.to_str()) == Some(".git") {
                                if let Some(parent) = p.parent() {
                                    let parent_buf = parent.to_path_buf();
                                    if ensure_git_repo(&parent_buf).is_ok() {
                                        return Ok(parent_buf);
                                    }
                                }
                            }
                            if ensure_git_repo(p).is_ok() {
                                return Ok(p.to_path_buf());
                            }
                            current = p.parent();
                        }
                    }
                }
            }
        }

        // 3. Fallback: check if worktree_path itself is a git repository
        if ensure_git_repo(worktree_path).is_ok() {
            let canon =
                dunce::canonicalize(worktree_path).unwrap_or_else(|_| worktree_path.to_path_buf());
            return Ok(canon);
        }

        Err(GitError::RepoNotFound(worktree_path.to_path_buf()))
    }

    /// Automatically resolves the backing repository for `worktree_path` and cleanly removes the worktree.
    /// Errors if uncommitted changes exist and `force` is false.
    pub fn remove_worktree_auto(worktree_path: &Path, force: bool) -> Result<(), GitError> {
        if !worktree_path.exists() {
            return Err(GitError::WorktreeNotFound(worktree_path.to_path_buf()));
        }

        if !force && Self::has_uncommitted_changes(worktree_path)? {
            return Err(GitError::WorktreeDirty {
                path: worktree_path.to_path_buf(),
            });
        }

        let main_repo = Self::resolve_main_repo(worktree_path)?;

        let remove_res = Self::remove_worktree(&main_repo, worktree_path, true);
        if remove_res.is_err() || worktree_path.exists() {
            if worktree_path.exists() {
                let _ = std::fs::remove_dir_all(worktree_path);
            }
            let _ = Command::new("git")
                .args(["worktree", "prune"])
                .current_dir(&main_repo)
                .output();
        }

        Ok(())
    }

    /// Returns true if `worktree_path` has uncommitted, untracked, or staged changes.
    pub fn has_uncommitted_changes(worktree_path: &Path) -> Result<bool, GitError> {
        if !worktree_path.exists() {
            return Err(GitError::WorktreeNotFound(worktree_path.to_path_buf()));
        }

        let output = Command::new("git")
            .args(["status", "--porcelain"])
            .current_dir(worktree_path)
            .output()?;

        if !output.status.success() {
            return Err(GitError::CommandFailed {
                command: "git status --porcelain".to_string(),
                stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            });
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        Ok(!stdout.trim().is_empty())
    }

    /// Determines the default branch (`main`, `master`, etc.) of `repo_path`.
    pub fn get_default_branch(repo_path: &Path) -> Result<String, GitError> {
        ensure_git_repo(repo_path)?;

        // 1. Check remote HEAD: git symbolic-ref --short refs/remotes/origin/HEAD
        if let Ok(output) = Command::new("git")
            .args(["symbolic-ref", "--short", "refs/remotes/origin/HEAD"])
            .current_dir(repo_path)
            .output()
        {
            if output.status.success() {
                let ref_name = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !ref_name.is_empty() {
                    let branch = ref_name.strip_prefix("origin/").unwrap_or(&ref_name).trim();
                    if !branch.is_empty() {
                        return Ok(branch.to_string());
                    }
                }
            }
        }

        // 2. Check refs/heads/main
        if let Ok(output) = Command::new("git")
            .args(["rev-parse", "--verify", "refs/heads/main"])
            .current_dir(repo_path)
            .output()
        {
            if output.status.success() {
                return Ok("main".to_string());
            }
        }

        // 3. Check refs/heads/master
        if let Ok(output) = Command::new("git")
            .args(["rev-parse", "--verify", "refs/heads/master"])
            .current_dir(repo_path)
            .output()
        {
            if output.status.success() {
                return Ok("master".to_string());
            }
        }

        // 4. Check current HEAD symbolic ref
        if let Ok(output) = Command::new("git")
            .args(["symbolic-ref", "--short", "HEAD"])
            .current_dir(repo_path)
            .output()
        {
            if output.status.success() {
                let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !branch.is_empty() {
                    return Ok(branch);
                }
            }
        }

        Err(GitError::DefaultBranchNotFound(repo_path.to_path_buf()))
    }

    /// Returns true if `branch_name` already exists in `repo_path`.
    pub fn branch_exists(repo_path: &Path, branch_name: &str) -> Result<bool, GitError> {
        ensure_git_repo(repo_path)?;
        let ref_name = format!("refs/heads/{branch_name}");
        let output = Command::new("git")
            .args(["rev-parse", "--verify", &ref_name])
            .current_dir(repo_path)
            .output()?;
        Ok(output.status.success())
    }

    /// Force-deletes `branch_name` from `repo_path`.
    ///
    /// Destructive, so callers must only pass a branch they created: `create_worktree`
    /// returns `BranchAlreadyExists` before running git, so a later failure proves the branch
    /// did not exist beforehand. `-D` because a setup hook may already have committed to it.
    pub fn delete_branch(repo_path: &Path, branch_name: &str) -> Result<(), GitError> {
        ensure_git_repo(repo_path)?;
        let output = Command::new("git")
            .args(["branch", "-D", branch_name])
            .current_dir(repo_path)
            .output()?;

        if !output.status.success() {
            return Err(GitError::CommandFailed {
                command: format!("git branch -D {branch_name}"),
                stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            });
        }

        Ok(())
    }

    /// Returns the current checked out branch name for `path`.
    pub fn get_current_branch(path: &Path) -> Result<String, GitError> {
        if !path.exists() {
            return Err(GitError::RepoNotFound(path.to_path_buf()));
        }
        let output = Command::new("git")
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .current_dir(path)
            .output()?;
        if !output.status.success() {
            return Err(GitError::CommandFailed {
                command: "git rev-parse --abbrev-ref HEAD".to_string(),
                stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            });
        }
        let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Ok(branch)
    }
}

impl WorktreeManager for GitWorktreeManager {
    fn create_worktree(
        &self,
        repo_path: &Path,
        worktree_path: &Path,
        branch_name: &str,
    ) -> Result<(), GitError> {
        Self::create_worktree(repo_path, worktree_path, branch_name)
    }

    fn create_worktree_with_options(
        &self,
        repo_path: &Path,
        worktree_path: &Path,
        branch_name: &str,
        checkout_existing: bool,
    ) -> Result<bool, GitError> {
        Self::create_worktree_with_options(repo_path, worktree_path, branch_name, checkout_existing)
    }

    fn find_worktree_for_branch(
        &self,
        repo_path: &Path,
        branch_name: &str,
    ) -> Result<Option<PathBuf>, GitError> {
        Self::find_worktree_for_branch(repo_path, branch_name)
    }

    fn remove_worktree(
        &self,
        repo_path: &Path,
        worktree_path: &Path,
        force: bool,
    ) -> Result<(), GitError> {
        Self::remove_worktree(repo_path, worktree_path, force)
    }

    fn resolve_main_repo(&self, worktree_path: &Path) -> Result<PathBuf, GitError> {
        Self::resolve_main_repo(worktree_path)
    }

    fn remove_worktree_auto(&self, worktree_path: &Path, force: bool) -> Result<(), GitError> {
        Self::remove_worktree_auto(worktree_path, force)
    }

    fn has_uncommitted_changes(&self, worktree_path: &Path) -> Result<bool, GitError> {
        Self::has_uncommitted_changes(worktree_path)
    }

    fn get_default_branch(&self, repo_path: &Path) -> Result<String, GitError> {
        Self::get_default_branch(repo_path)
    }

    fn branch_exists(&self, repo_path: &Path, branch_name: &str) -> Result<bool, GitError> {
        Self::branch_exists(repo_path, branch_name)
    }

    fn delete_branch(&self, repo_path: &Path, branch_name: &str) -> Result<(), GitError> {
        Self::delete_branch(repo_path, branch_name)
    }

    fn get_current_branch(&self, path: &Path) -> Result<String, GitError> {
        Self::get_current_branch(path)
    }
}

/// Type alias for GitWorktreeManager.
pub type GitManager = GitWorktreeManager;

/// Module-level convenience function to clone a repository.
pub fn clone_repo(url: &str, target_path: &Path) -> Result<(), GitError> {
    GitWorktreeManager::clone_repo(url, target_path)
}

/// Module-level convenience function to fetch remote updates.
pub fn fetch_repo(repo_path: &Path) -> Result<(), GitError> {
    GitWorktreeManager::fetch_repo(repo_path)
}

/// Module-level convenience function to check if a remote branch exists.
pub fn remote_branch_exists(repo_path: &Path, remote_branch: &str) -> Result<bool, GitError> {
    GitWorktreeManager::remote_branch_exists(repo_path, remote_branch)
}

/// Module-level convenience function to find the worktree where `branch_name` is currently checked out.
pub fn find_worktree_for_branch(
    repo_path: &Path,
    branch_name: &str,
) -> Result<Option<PathBuf>, GitError> {
    GitWorktreeManager::find_worktree_for_branch(repo_path, branch_name)
}

/// Module-level convenience function to create a worktree.
pub fn create_worktree(
    repo_path: &Path,
    worktree_path: &Path,
    branch_name: &str,
) -> Result<(), GitError> {
    GitWorktreeManager::create_worktree(repo_path, worktree_path, branch_name)
}

/// Module-level convenience function to create a worktree with options (`checkout_existing`).
pub fn create_worktree_with_options(
    repo_path: &Path,
    worktree_path: &Path,
    branch_name: &str,
    checkout_existing: bool,
) -> Result<bool, GitError> {
    GitWorktreeManager::create_worktree_with_options(
        repo_path,
        worktree_path,
        branch_name,
        checkout_existing,
    )
}

/// Module-level convenience function to remove a worktree.
pub fn remove_worktree(
    repo_path: &Path,
    worktree_path: &Path,
    force: bool,
) -> Result<(), GitError> {
    GitWorktreeManager::remove_worktree(repo_path, worktree_path, force)
}

/// Module-level convenience function to resolve the main repository backing a worktree.
pub fn resolve_main_repo(worktree_path: &Path) -> Result<PathBuf, GitError> {
    GitWorktreeManager::resolve_main_repo(worktree_path)
}

/// Module-level convenience function to remove a worktree automatically resolving its main repository.
pub fn remove_worktree_auto(worktree_path: &Path, force: bool) -> Result<(), GitError> {
    GitWorktreeManager::remove_worktree_auto(worktree_path, force)
}

/// Module-level convenience function to check for uncommitted changes.
pub fn has_uncommitted_changes(worktree_path: &Path) -> Result<bool, GitError> {
    GitWorktreeManager::has_uncommitted_changes(worktree_path)
}

/// Module-level convenience function to get the default branch.
pub fn get_default_branch(repo_path: &Path) -> Result<String, GitError> {
    GitWorktreeManager::get_default_branch(repo_path)
}

/// Module-level convenience function to check if a branch exists.
pub fn branch_exists(repo_path: &Path, branch_name: &str) -> Result<bool, GitError> {
    GitWorktreeManager::branch_exists(repo_path, branch_name)
}

/// Module-level convenience function to force-delete a branch.
pub fn delete_branch(repo_path: &Path, branch_name: &str) -> Result<(), GitError> {
    GitWorktreeManager::delete_branch(repo_path, branch_name)
}

/// Module-level convenience function to get current checked out branch.
pub fn get_current_branch(path: &Path) -> Result<String, GitError> {
    GitWorktreeManager::get_current_branch(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::process::Command;
    use tempfile::TempDir;

    /// Helper to initialize a mock git repository with an initial commit on a default branch.
    fn init_mock_repo(default_branch: &str) -> (TempDir, PathBuf) {
        let temp_dir = TempDir::new().expect("Failed to create tempdir");
        let repo_path = temp_dir.path().join("mock_repo");
        fs::create_dir_all(&repo_path).expect("Failed to create repo dir");

        // Run git init -b <default_branch>
        let init_status = Command::new("git")
            .args(["init", "-b", default_branch])
            .current_dir(&repo_path)
            .status()
            .or_else(|_| {
                // Fallback for older git versions
                Command::new("git")
                    .arg("init")
                    .current_dir(&repo_path)
                    .status()
            })
            .expect("Failed to run git init");
        assert!(init_status.success(), "git init failed");

        // Configure git user for commits in test
        let _ = Command::new("git")
            .args(["config", "user.name", "Botai Test"])
            .current_dir(&repo_path)
            .status();
        let _ = Command::new("git")
            .args(["config", "user.email", "test@botai.dev"])
            .current_dir(&repo_path)
            .status();

        // Create an initial commit
        fs::write(repo_path.join("README.md"), "# Mock Repo\n").expect("Failed to write README");
        let add_status = Command::new("git")
            .args(["add", "README.md"])
            .current_dir(&repo_path)
            .status()
            .expect("git add failed");
        assert!(add_status.success());

        let commit_status = Command::new("git")
            .args(["commit", "-m", "Initial commit"])
            .current_dir(&repo_path)
            .status()
            .expect("git commit failed");
        assert!(commit_status.success());

        (temp_dir, repo_path)
    }

    #[test]
    fn test_create_worktree_happy_path() {
        let (_dir, repo_path) = init_mock_repo("main");
        let worktree_dir = TempDir::new().expect("Failed to create worktree tempdir");
        let worktree_path = worktree_dir.path().join("worktree_main");

        // Should create worktree on branch "feature-abc" branching off "main"
        create_worktree(&repo_path, &worktree_path, "feature-abc")
            .expect("create_worktree should succeed");

        assert!(worktree_path.exists());
        assert!(worktree_path.join("README.md").exists());
        assert_eq!(
            get_current_branch(&worktree_path).expect("get current branch"),
            "feature-abc"
        );
    }

    #[test]
    fn test_create_worktree_branch_collision_error() {
        let (_dir, repo_path) = init_mock_repo("main");

        // Create a pre-existing branch "existing-feat"
        let branch_status = Command::new("git")
            .args(["branch", "existing-feat"])
            .current_dir(&repo_path)
            .status()
            .expect("git branch failed");
        assert!(branch_status.success());

        let worktree_dir = TempDir::new().expect("Failed to create tempdir");
        let worktree_path = worktree_dir.path().join("worktree_collision");

        // Attempting to create worktree with existing branch name must fail
        let result = create_worktree(&repo_path, &worktree_path, "existing-feat");
        match result {
            Err(GitError::BranchAlreadyExists { branch, repo }) => {
                assert_eq!(branch, "existing-feat");
                assert_eq!(repo, repo_path);
            }
            other => panic!("Expected BranchAlreadyExists error, got: {other:?}"),
        }
        assert!(
            !worktree_path.exists(),
            "Worktree path should not exist after collision failure"
        );
    }

    #[test]
    fn test_create_worktree_path_collision_error() {
        let (_dir, repo_path) = init_mock_repo("main");
        let worktree_dir = TempDir::new().expect("Failed to create tempdir");
        let worktree_path = worktree_dir.path().join("already_exists");

        // Pre-create directory on disk
        fs::create_dir_all(&worktree_path).expect("Failed to create pre-existing dir");

        let result = create_worktree(&repo_path, &worktree_path, "new-branch");
        match result {
            Err(GitError::WorktreePathExists(p)) => {
                assert_eq!(p, worktree_path);
            }
            other => panic!("Expected WorktreePathExists error, got: {other:?}"),
        }
    }

    #[test]
    fn test_create_worktree_nonexistent_repo_error() {
        let nonexistent_repo = PathBuf::from("/path/to/nonexistent/repo");
        let worktree_dir = TempDir::new().expect("Failed to create tempdir");
        let worktree_path = worktree_dir.path().join("worktree");

        let result = create_worktree(&nonexistent_repo, &worktree_path, "branch-1");
        match result {
            Err(GitError::RepoNotFound(p)) => {
                assert_eq!(p, nonexistent_repo);
            }
            other => panic!("Expected RepoNotFound error, got: {other:?}"),
        }
    }

    #[test]
    fn test_create_worktree_with_master_default_branch() {
        let (_dir, repo_path) = init_mock_repo("master");
        let worktree_dir = TempDir::new().expect("Failed to create tempdir");
        let worktree_path = worktree_dir.path().join("worktree_master");

        create_worktree(&repo_path, &worktree_path, "feature-from-master")
            .expect("Should branch off master when default");

        assert!(worktree_path.join("README.md").exists());
        assert_eq!(
            get_current_branch(&worktree_path).expect("current branch"),
            "feature-from-master"
        );
    }

    #[test]
    fn test_has_uncommitted_changes_clean() {
        let (_dir, repo_path) = init_mock_repo("main");
        let worktree_dir = TempDir::new().expect("Failed to create tempdir");
        let worktree_path = worktree_dir.path().join("clean_wt");

        create_worktree(&repo_path, &worktree_path, "clean-feat").expect("create worktree");

        assert!(!has_uncommitted_changes(&worktree_path).expect("check clean"));
    }

    #[test]
    fn test_has_uncommitted_changes_modified_file() {
        let (_dir, repo_path) = init_mock_repo("main");
        let worktree_dir = TempDir::new().expect("Failed to create tempdir");
        let worktree_path = worktree_dir.path().join("modified_wt");

        create_worktree(&repo_path, &worktree_path, "mod-feat").expect("create worktree");

        // Modify tracked file
        fs::write(worktree_path.join("README.md"), "# Modified Content\n")
            .expect("Failed to modify README");

        assert!(has_uncommitted_changes(&worktree_path).expect("check modified"));
    }

    #[test]
    fn test_has_uncommitted_changes_untracked_file() {
        let (_dir, repo_path) = init_mock_repo("main");
        let worktree_dir = TempDir::new().expect("Failed to create tempdir");
        let worktree_path = worktree_dir.path().join("untracked_wt");

        create_worktree(&repo_path, &worktree_path, "untracked-feat").expect("create worktree");

        // Create untracked file
        fs::write(worktree_path.join("new_file.txt"), "hello\n").expect("write untracked");

        assert!(has_uncommitted_changes(&worktree_path).expect("check untracked"));
    }

    #[test]
    fn test_has_uncommitted_changes_staged_file() {
        let (_dir, repo_path) = init_mock_repo("main");
        let worktree_dir = TempDir::new().expect("Failed to create tempdir");
        let worktree_path = worktree_dir.path().join("staged_wt");

        create_worktree(&repo_path, &worktree_path, "staged-feat").expect("create worktree");

        fs::write(worktree_path.join("staged.txt"), "staged\n").expect("write staged");
        let add = Command::new("git")
            .args(["add", "staged.txt"])
            .current_dir(&worktree_path)
            .status()
            .expect("git add");
        assert!(add.success());

        assert!(has_uncommitted_changes(&worktree_path).expect("check staged"));
    }

    #[test]
    fn test_has_uncommitted_changes_deleted_file() {
        let (_dir, repo_path) = init_mock_repo("main");
        let worktree_dir = TempDir::new().expect("Failed to create tempdir");
        let worktree_path = worktree_dir.path().join("deleted_wt");

        create_worktree(&repo_path, &worktree_path, "del-feat").expect("create worktree");

        // Delete tracked file
        fs::remove_file(worktree_path.join("README.md")).expect("remove tracked file");

        assert!(has_uncommitted_changes(&worktree_path).expect("check deleted"));
    }

    #[test]
    fn test_remove_worktree_clean_preserves_branch() {
        let (_dir, repo_path) = init_mock_repo("main");
        let worktree_dir = TempDir::new().expect("Failed to create tempdir");
        let worktree_path = worktree_dir.path().join("remove_clean_wt");

        create_worktree(&repo_path, &worktree_path, "safe-removal-branch")
            .expect("create worktree");
        assert!(worktree_path.exists());

        // Remove worktree with force = false
        remove_worktree(&repo_path, &worktree_path, false).expect("remove clean worktree");

        // Worktree directory should no longer exist
        assert!(!worktree_path.exists(), "Worktree dir must be deleted");

        // The branch must still exist in git repository
        assert!(
            branch_exists(&repo_path, "safe-removal-branch").expect("branch exists"),
            "Git branch must be preserved after worktree removal"
        );
    }

    #[test]
    fn test_remove_worktree_dirty_without_force_fails() {
        let (_dir, repo_path) = init_mock_repo("main");
        let worktree_dir = TempDir::new().expect("Failed to create tempdir");
        let worktree_path = worktree_dir.path().join("dirty_wt");

        create_worktree(&repo_path, &worktree_path, "dirty-branch").expect("create worktree");

        // Make dirty
        fs::write(worktree_path.join("dirty.txt"), "dirty").expect("write dirty file");

        // Attempt removal without force
        let result = remove_worktree(&repo_path, &worktree_path, false);
        match result {
            Err(GitError::WorktreeDirty { path }) => {
                assert_eq!(path, worktree_path);
            }
            other => panic!("Expected WorktreeDirty error, got: {other:?}"),
        }

        // Worktree directory must still exist
        assert!(
            worktree_path.exists(),
            "Dirty worktree must not be deleted when force=false"
        );
    }

    #[test]
    fn test_remove_worktree_dirty_with_force_succeeds() {
        let (_dir, repo_path) = init_mock_repo("main");
        let worktree_dir = TempDir::new().expect("Failed to create tempdir");
        let worktree_path = worktree_dir.path().join("force_dirty_wt");

        create_worktree(&repo_path, &worktree_path, "force-branch").expect("create worktree");

        // Make dirty
        fs::write(worktree_path.join("dirty.txt"), "dirty").expect("write dirty file");

        // Force removal
        remove_worktree(&repo_path, &worktree_path, true).expect("force remove dirty worktree");

        // Directory must be removed
        assert!(!worktree_path.exists());

        // Branch must be preserved
        assert!(
            branch_exists(&repo_path, "force-branch").expect("branch exists"),
            "Branch must be preserved even on forced removal"
        );
    }

    #[test]
    fn test_remove_nonexistent_worktree_error() {
        let (_dir, repo_path) = init_mock_repo("main");
        let nonexistent_wt = PathBuf::from("/path/to/nonexistent/wt");

        let result = remove_worktree(&repo_path, &nonexistent_wt, false);
        match result {
            Err(GitError::WorktreeNotFound(p)) => {
                assert_eq!(p, nonexistent_wt);
            }
            other => panic!("Expected WorktreeNotFound error, got: {other:?}"),
        }
    }

    #[test]
    fn test_spaces_and_unicode_in_paths() {
        let temp_dir = TempDir::new().expect("tempdir");
        let repo_path = temp_dir.path().join("repo with spaces 🚀");
        fs::create_dir_all(&repo_path).expect("create dir");

        let _ = Command::new("git")
            .args(["init", "-b", "main"])
            .current_dir(&repo_path)
            .status();
        let _ = Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(&repo_path)
            .status();
        let _ = Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(&repo_path)
            .status();
        fs::write(repo_path.join("README.md"), "# Space & Unicode Repo\n").unwrap();
        let _ = Command::new("git")
            .args(["add", "README.md"])
            .current_dir(&repo_path)
            .status();
        let _ = Command::new("git")
            .args(["commit", "-m", "init"])
            .current_dir(&repo_path)
            .status();

        let wt_path = temp_dir.path().join("wt with spaces 🛠️");
        create_worktree(&repo_path, &wt_path, "feat-unicode").expect("create worktree");

        assert!(wt_path.join("README.md").exists());
        remove_worktree(&repo_path, &wt_path, false).expect("remove worktree");
        assert!(!wt_path.exists());
    }

    #[test]
    fn test_clone_repo_local_and_fetch() {
        let (_dir, origin_repo) = init_mock_repo("main");
        let client_temp = TempDir::new().expect("tempdir");
        let cloned_path = client_temp.path().join("cloned_repo");

        // Clone repo
        let origin_url = origin_repo.to_string_lossy().to_string();
        clone_repo(&origin_url, &cloned_path).expect("clone local repo");
        assert!(cloned_path.exists());
        assert!(cloned_path.join("README.md").exists());

        // Clone again (should be no-op success)
        clone_repo(&origin_url, &cloned_path).expect("clone existing repo is no-op");

        // Fetch from origin
        fetch_repo(&cloned_path).expect("fetch from origin");

        // Check remote branch exists
        assert!(remote_branch_exists(&cloned_path, "origin/main").expect("check remote branch"));

        // Create worktree on cloned repo - should branch off origin/main
        let wt_path = client_temp.path().join("wt_from_clone");
        create_worktree(&cloned_path, &wt_path, "feature-cloned")
            .expect("create worktree from clone");
        assert!(wt_path.exists());
        assert_eq!(
            get_current_branch(&wt_path).expect("current branch"),
            "feature-cloned"
        );
    }

    #[test]
    fn test_clone_repo_invalid_url_fails() {
        let temp = TempDir::new().expect("tempdir");
        let target = temp.path().join("invalid");
        let err = clone_repo(
            "https://invalid.domain.that.does.not.exist.botai/repo.git",
            &target,
        )
        .expect_err("Cloning invalid URL should fail");

        match err {
            GitError::CloneFailed { url, path, .. } => {
                assert!(url.contains("invalid.domain"));
                assert_eq!(path, target);
            }
            other => panic!("Expected CloneFailed error, got: {other:?}"),
        }
    }

    #[test]
    fn test_resolve_main_repo_from_worktree() {
        let (_dir, repo_path) = init_mock_repo("main");
        let worktree_dir = TempDir::new().expect("tempdir");
        let worktree_path = worktree_dir.path().join("wt_resolve");

        create_worktree(&repo_path, &worktree_path, "feat-resolve").expect("create worktree");

        let resolved = resolve_main_repo(&worktree_path).expect("resolve main repo");
        assert_eq!(
            dunce::canonicalize(&resolved).expect("canon resolved"),
            dunce::canonicalize(&repo_path).expect("canon repo")
        );
    }

    #[test]
    fn test_resolve_main_repo_from_main_repo_itself() {
        let (_dir, repo_path) = init_mock_repo("main");
        let resolved = resolve_main_repo(&repo_path).expect("resolve repo itself");
        assert_eq!(
            dunce::canonicalize(&resolved).expect("canon resolved"),
            dunce::canonicalize(&repo_path).expect("canon repo")
        );
    }

    #[test]
    fn test_resolve_main_repo_nonexistent_and_invalid() {
        let nonexistent = PathBuf::from("/nonexistent/path/that/does/not/exist");
        assert!(matches!(
            resolve_main_repo(&nonexistent),
            Err(GitError::WorktreeNotFound(_))
        ));

        let empty_temp = TempDir::new().expect("tempdir");
        assert!(matches!(
            resolve_main_repo(empty_temp.path()),
            Err(GitError::RepoNotFound(_))
        ));
    }

    #[test]
    fn test_remove_worktree_auto_clean_removes_metadata_and_prunes() {
        let (_dir, repo_path) = init_mock_repo("main");
        let worktree_dir = TempDir::new().expect("tempdir");
        let worktree_path = worktree_dir.path().join("wt_auto_clean");

        create_worktree(&repo_path, &worktree_path, "feat-auto-clean").expect("create worktree");
        assert!(worktree_path.exists());

        // Verify git worktree list contains the worktree before removal
        let wt_list_before = Command::new("git")
            .args(["worktree", "list", "--porcelain"])
            .current_dir(&repo_path)
            .output()
            .expect("git worktree list");
        let list_str_before = String::from_utf8_lossy(&wt_list_before.stdout);
        assert!(list_str_before.contains("feat-auto-clean"));

        // Remove worktree automatically
        remove_worktree_auto(&worktree_path, false).expect("remove_worktree_auto");

        // Directory should be gone
        assert!(!worktree_path.exists());

        // git worktree list should no longer list the worktree
        let wt_list_after = Command::new("git")
            .args(["worktree", "list", "--porcelain"])
            .current_dir(&repo_path)
            .output()
            .expect("git worktree list");
        let list_str_after = String::from_utf8_lossy(&wt_list_after.stdout);
        assert!(!list_str_after.contains("feat-auto-clean"));

        // Branch is preserved
        assert!(branch_exists(&repo_path, "feat-auto-clean").expect("branch exists"));

        // Worktrees metadata directory should not contain the pruned entry
        let wt_meta_dir = repo_path
            .join(".git")
            .join("worktrees")
            .join("wt_auto_clean");
        assert!(!wt_meta_dir.exists());
    }

    #[test]
    fn test_remove_worktree_auto_dirty_without_force_fails() {
        let (_dir, repo_path) = init_mock_repo("main");
        let worktree_dir = TempDir::new().expect("tempdir");
        let worktree_path = worktree_dir.path().join("wt_auto_dirty");

        create_worktree(&repo_path, &worktree_path, "feat-auto-dirty").expect("create worktree");
        fs::write(worktree_path.join("uncommitted.txt"), "dirty").expect("write dirty file");

        let err = remove_worktree_auto(&worktree_path, false).expect_err("should fail on dirty");
        match err {
            GitError::WorktreeDirty { path } => {
                assert_eq!(path, worktree_path);
            }
            other => panic!("Expected WorktreeDirty, got {other:?}"),
        }

        assert!(worktree_path.exists());
    }

    #[test]
    fn test_remove_worktree_auto_dirty_with_force_succeeds() {
        let (_dir, repo_path) = init_mock_repo("main");
        let worktree_dir = TempDir::new().expect("tempdir");
        let worktree_path = worktree_dir.path().join("wt_auto_force_dirty");

        create_worktree(&repo_path, &worktree_path, "feat-force-dirty").expect("create worktree");
        fs::write(worktree_path.join("uncommitted.txt"), "dirty").expect("write dirty file");

        remove_worktree_auto(&worktree_path, true).expect("force remove should succeed");
        assert!(!worktree_path.exists());

        let wt_list_after = Command::new("git")
            .args(["worktree", "list", "--porcelain"])
            .current_dir(&repo_path)
            .output()
            .expect("git worktree list");
        let list_str_after = String::from_utf8_lossy(&wt_list_after.stdout);
        assert!(!list_str_after.contains("feat-force-dirty"));
        assert!(branch_exists(&repo_path, "feat-force-dirty").expect("branch exists"));
    }

    #[test]
    fn test_remove_worktree_auto_recreate_worktree_at_same_path() {
        let (_dir, repo_path) = init_mock_repo("main");
        let worktree_dir = TempDir::new().expect("tempdir");
        let worktree_path = worktree_dir.path().join("wt_recreate");

        create_worktree(&repo_path, &worktree_path, "branch-1").expect("create branch-1");
        assert_eq!(get_current_branch(&worktree_path).unwrap(), "branch-1");

        remove_worktree_auto(&worktree_path, false).expect("remove worktree");
        assert!(!worktree_path.exists());

        create_worktree(&repo_path, &worktree_path, "branch-2").expect("create branch-2");
        assert_eq!(get_current_branch(&worktree_path).unwrap(), "branch-2");

        remove_worktree_auto(&worktree_path, false).expect("remove worktree again");
        assert!(!worktree_path.exists());
    }

    #[test]
    fn test_create_worktree_prunes_stale_worktree_metadata() {
        let (_dir, repo_path) = init_mock_repo("main");
        let worktree_dir = TempDir::new().expect("Failed to create worktree tempdir");
        let wt_path = worktree_dir.path().join("worktree_stale");

        // Create initial worktree with branch1
        create_worktree(&repo_path, &wt_path, "branch1")
            .expect("initial create_worktree should succeed");
        assert!(wt_path.exists());
        assert_eq!(
            get_current_branch(&wt_path).expect("current branch"),
            "branch1"
        );

        // Manually delete the worktree directory without git worktree remove
        fs::remove_dir_all(&wt_path).expect("Failed to remove worktree directory");
        assert!(!wt_path.exists());

        // Calling create_worktree at the same path with branch2 should succeed because
        // it runs git worktree prune prior to creating the worktree
        create_worktree(&repo_path, &wt_path, "branch2")
            .expect("create_worktree should succeed after pruning stale metadata");
        assert!(wt_path.exists());
        assert_eq!(
            get_current_branch(&wt_path).expect("current branch"),
            "branch2"
        );
    }

    /// Regression test: the worktree destination must never be a Windows verbatim (`\\?\`)
    /// path.
    ///
    /// `ProjectBuilder::create_workspace` canonicalizes the project directory and derives
    /// every worktree path from it. With `Path::canonicalize` that yielded `\\?\C:\...`, which
    /// git rewrites to `//?/C:/...` and then rejects:
    /// `fatal: could not create leading directories of '//?/...': Invalid argument`.
    /// `dunce::canonicalize` returns a plain path instead.
    ///
    /// The assertion is cross-platform: on unix `dunce::canonicalize` is `canonicalize`, and
    /// no path acquires the prefix, so the check is simply always true there.
    #[test]
    fn test_create_worktree_accepts_canonicalized_project_dir() {
        let (temp_dir, repo_path) = init_mock_repo("main");

        let project_dir = temp_dir.path().join("project");
        fs::create_dir_all(&project_dir).expect("create project dir");
        let project_dir_canon = dunce::canonicalize(&project_dir).expect("canonicalize");

        assert!(
            !project_dir_canon.to_string_lossy().starts_with(r"\\?\"),
            "canonicalized project dir must not be a verbatim path, got {}",
            project_dir_canon.display()
        );

        // The exact shape create_workspace builds: <project_dir_canon>/<repo name>.
        let wt_path = project_dir_canon.join("backend");
        create_worktree(&repo_path, &wt_path, "feature-x")
            .expect("create_worktree should succeed for a canonicalized project dir");

        assert!(wt_path.exists());
        assert_eq!(
            get_current_branch(&wt_path).expect("current branch"),
            "feature-x"
        );
    }

    #[test]
    fn test_delete_branch_removes_branch_and_errors_when_missing() {
        let (_temp_dir, repo_path) = init_mock_repo("main");

        let wt_dir = _temp_dir.path().join("wt");
        create_worktree(&repo_path, &wt_dir, "doomed").expect("create_worktree");
        assert!(branch_exists(&repo_path, "doomed").expect("branch_exists"));

        // The branch is checked out by the worktree, so it has to go first.
        remove_worktree(&repo_path, &wt_dir, true).expect("remove_worktree");
        // Removing a worktree leaves the branch behind - that is why rollback needs this.
        assert!(
            branch_exists(&repo_path, "doomed").expect("branch_exists"),
            "removing a worktree must not delete its branch"
        );

        delete_branch(&repo_path, "doomed").expect("delete_branch");
        assert!(!branch_exists(&repo_path, "doomed").expect("branch_exists"));

        // Deleting a branch that is not there is an error, not a silent success - callers on
        // the rollback path swallow it with `let _ =`.
        assert!(delete_branch(&repo_path, "never-existed").is_err());
    }

    #[test]
    fn test_find_worktree_for_branch_detects_main_and_linked_worktrees() {
        let (_temp_dir, repo_path) = init_mock_repo("main");

        // "main" is checked out in the main repository
        let main_wt = find_worktree_for_branch(&repo_path, "main")
            .expect("find_worktree_for_branch should succeed")
            .expect("main should be checked out");
        assert_eq!(
            dunce::canonicalize(&main_wt).unwrap(),
            dunce::canonicalize(&repo_path).unwrap()
        );

        // Create a local branch that is NOT checked out anywhere
        let status = Command::new("git")
            .args(["branch", "unattached-branch"])
            .current_dir(&repo_path)
            .status()
            .expect("git branch");
        assert!(status.success());

        assert_eq!(
            find_worktree_for_branch(&repo_path, "unattached-branch").unwrap(),
            None
        );

        // Create a linked worktree on "linked-branch"
        let wt_dir = TempDir::new().unwrap();
        let wt_path = wt_dir.path().join("linked_wt");
        create_worktree(&repo_path, &wt_path, "linked-branch").expect("create_worktree");

        let linked_wt = find_worktree_for_branch(&repo_path, "linked-branch")
            .expect("find_worktree_for_branch")
            .expect("linked-branch should be checked out");
        assert_eq!(
            dunce::canonicalize(&linked_wt).unwrap(),
            dunce::canonicalize(&wt_path).unwrap()
        );

        // Empty branch name returns Ok(None)
        assert_eq!(find_worktree_for_branch(&repo_path, "   ").unwrap(), None);
    }

    #[test]
    fn test_create_worktree_with_options_checkout_existing_local_branch_preserves_commits() {
        let (_temp_dir, repo_path) = init_mock_repo("main");

        // Create a branch with an extra commit, then switch back to main
        let wt_setup_dir = TempDir::new().unwrap();
        let wt_setup_path = wt_setup_dir.path().join("setup_wt");
        create_worktree(&repo_path, &wt_setup_path, "existing-local").expect("create setup wt");
        fs::write(wt_setup_path.join("feature.txt"), "existing work\n").unwrap();
        let _ = Command::new("git")
            .args(["add", "feature.txt"])
            .current_dir(&wt_setup_path)
            .status();
        let _ = Command::new("git")
            .args(["commit", "-m", "Feature commit"])
            .current_dir(&wt_setup_path)
            .status();
        remove_worktree(&repo_path, &wt_setup_path, false).expect("remove setup wt");

        // Now "existing-local" exists locally with "feature.txt" and is not checked out
        let wt_dir = TempDir::new().unwrap();
        let wt_path = wt_dir.path().join("reused_wt");

        let created_new =
            create_worktree_with_options(&repo_path, &wt_path, "existing-local", true)
                .expect("checkout_existing should succeed for local branch");
        assert!(
            !created_new,
            "Checking out pre-existing local branch must return created_new_branch = false"
        );
        assert!(wt_path.join("feature.txt").exists());
        assert_eq!(
            fs::read_to_string(wt_path.join("feature.txt")).unwrap(),
            "existing work\n"
        );
        assert_eq!(get_current_branch(&wt_path).unwrap(), "existing-local");
    }

    #[test]
    fn test_create_worktree_with_options_checkout_existing_remote_branch() {
        let (_origin_dir, origin_repo) = init_mock_repo("main");

        // Create a branch on origin with a commit (via a temporary worktree on origin so main stays checked out)
        let origin_wt_dir = TempDir::new().unwrap();
        let origin_wt_path = origin_wt_dir.path().join("origin_feat_wt");
        create_worktree(&origin_repo, &origin_wt_path, "remote-only-feat")
            .expect("create origin feature branch");
        fs::write(origin_wt_path.join("remote.txt"), "from origin\n").unwrap();
        let _ = Command::new("git")
            .args(["add", "remote.txt"])
            .current_dir(&origin_wt_path)
            .status();
        let _ = Command::new("git")
            .args(["commit", "-m", "Remote commit"])
            .current_dir(&origin_wt_path)
            .status();
        remove_worktree(&origin_repo, &origin_wt_path, false).expect("remove origin wt");

        // Clone origin_repo to client_repo
        let client_temp = TempDir::new().unwrap();
        let client_repo = client_temp.path().join("client_repo");
        clone_repo(&origin_repo.to_string_lossy(), &client_repo).expect("clone");

        // Verify "remote-only-feat" does not exist as a local branch in client_repo, but origin/remote-only-feat exists
        assert!(!branch_exists(&client_repo, "remote-only-feat").unwrap());
        assert!(remote_branch_exists(&client_repo, "origin/remote-only-feat").unwrap());

        let wt_path = client_temp.path().join("wt_remote_feat");
        let created_new =
            create_worktree_with_options(&client_repo, &wt_path, "remote-only-feat", true)
                .expect("checkout_existing should track remote branch");
        assert!(
            created_new,
            "Creating local tracking branch from remote must return created_new_branch = true"
        );
        assert!(wt_path.join("remote.txt").exists());
        assert_eq!(get_current_branch(&wt_path).unwrap(), "remote-only-feat");
        assert!(branch_exists(&client_repo, "remote-only-feat").unwrap());
    }

    #[test]
    fn test_create_worktree_with_options_checkout_existing_fallback_new_branch() {
        let (_temp_dir, repo_path) = init_mock_repo("main");
        let wt_dir = TempDir::new().unwrap();
        let wt_path = wt_dir.path().join("wt_brand_new");

        let created_new =
            create_worktree_with_options(&repo_path, &wt_path, "brand-new-branch", true)
                .expect("checkout_existing should fall back to creating new branch");
        assert!(
            created_new,
            "Fallback new branch creation must return created_new_branch = true"
        );
        assert!(wt_path.exists());
        assert_eq!(get_current_branch(&wt_path).unwrap(), "brand-new-branch");
    }

    #[test]
    fn test_create_worktree_with_options_rejects_already_checked_out_branch() {
        let (_temp_dir, repo_path) = init_mock_repo("main");
        let wt_dir = TempDir::new().unwrap();
        let wt1_path = wt_dir.path().join("wt1");
        let wt2_path = wt_dir.path().join("wt2");

        create_worktree(&repo_path, &wt1_path, "active-branch").expect("create wt1");

        // Even with checkout_existing = true, checking out an already checked-out branch must fail with BranchAlreadyCheckedOut
        let err = create_worktree_with_options(&repo_path, &wt2_path, "active-branch", true)
            .expect_err("Should fail when branch is already checked out in wt1");
        match err {
            GitError::BranchAlreadyCheckedOut {
                branch,
                repo,
                worktree_path,
            } => {
                assert_eq!(branch, "active-branch");
                assert_eq!(repo, repo_path);
                assert_eq!(
                    dunce::canonicalize(&worktree_path).unwrap(),
                    dunce::canonicalize(&wt1_path).unwrap()
                );
            }
            other => panic!("Expected BranchAlreadyCheckedOut, got: {other:?}"),
        }

        // Also fails when trying to check out "main" which is checked out in repo_path itself
        let err_main = create_worktree_with_options(&repo_path, &wt2_path, "main", true)
            .expect_err("Should fail when branch is checked out in main repo");
        assert!(matches!(err_main, GitError::BranchAlreadyCheckedOut { .. }));
    }
}
