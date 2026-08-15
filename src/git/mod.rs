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

    /// Removes a git worktree at `worktree_path`.
    /// If `force` is false, returns `GitError::WorktreeDirty` if uncommitted changes exist.
    /// Preserves the git branch in the repository history.
    fn remove_worktree(
        &self,
        repo_path: &Path,
        worktree_path: &Path,
        force: bool,
    ) -> Result<(), GitError>;

    /// Checks if a worktree at `worktree_path` has uncommitted (modified, staged, or untracked) changes.
    fn has_uncommitted_changes(&self, worktree_path: &Path) -> Result<bool, GitError>;

    /// Determines the default branch (`main` or `master`) of the repository.
    fn get_default_branch(&self, repo_path: &Path) -> Result<String, GitError>;

    /// Checks if a branch exists in the repository.
    fn branch_exists(&self, repo_path: &Path, branch_name: &str) -> Result<bool, GitError>;

    /// Gets the currently checked out branch name of a repo or worktree.
    fn get_current_branch(&self, path: &Path) -> Result<String, GitError>;
}

/// Default implementation of git operations using the `git` CLI.
#[derive(Debug, Default, Clone)]
pub struct GitWorktreeManager;

use std::process::Command;

/// Checks whether a given path is an existing, valid Git repository.
fn ensure_git_repo(repo_path: &Path) -> Result<(), GitError> {
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

    /// Creates a worktree at `worktree_path` checked out to `branch_name` branched off default branch.
    pub fn create_worktree(
        repo_path: &Path,
        worktree_path: &Path,
        branch_name: &str,
    ) -> Result<(), GitError> {
        ensure_git_repo(repo_path)?;

        if branch_name.trim().is_empty() {
            return Err(GitError::InvalidBranchName(branch_name.to_string()));
        }

        if Self::branch_exists(repo_path, branch_name)? {
            return Err(GitError::BranchAlreadyExists {
                branch: branch_name.to_string(),
                repo: repo_path.to_path_buf(),
            });
        }

        if worktree_path.exists() {
            return Err(GitError::WorktreePathExists(worktree_path.to_path_buf()));
        }

        let default_branch = Self::get_default_branch(repo_path)?;

        if let Some(parent) = worktree_path.parent() {
            if !parent.as_os_str().is_empty() && !parent.exists() {
                std::fs::create_dir_all(parent)?;
            }
        }

        let output = Command::new("git")
            .args(["worktree", "add", "-b", branch_name])
            .arg(worktree_path)
            .arg(&default_branch)
            .current_dir(repo_path)
            .output()?;

        if !output.status.success() {
            return Err(GitError::CommandFailed {
                command: format!(
                    "git worktree add -b {} {:?} {}",
                    branch_name, worktree_path, default_branch
                ),
                stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            });
        }

        Ok(())
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

        let mut cmd = Command::new("git");
        cmd.arg("worktree").arg("remove");
        if force {
            cmd.arg("--force");
        }
        cmd.arg(worktree_path);
        cmd.current_dir(repo_path);

        let output = cmd.output()?;
        if !output.status.success() {
            return Err(GitError::CommandFailed {
                command: format!(
                    "git worktree remove{}{:?}",
                    if force { " --force " } else { " " },
                    worktree_path
                ),
                stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            });
        }

        // Prune stale worktree metadata
        let _ = Command::new("git")
            .args(["worktree", "prune"])
            .current_dir(repo_path)
            .output();

        if worktree_path.exists() {
            let _ = std::fs::remove_dir_all(worktree_path);
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
        let ref_name = format!("refs/heads/{}", branch_name);
        let output = Command::new("git")
            .args(["rev-parse", "--verify", &ref_name])
            .current_dir(repo_path)
            .output()?;
        Ok(output.status.success())
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

    fn remove_worktree(
        &self,
        repo_path: &Path,
        worktree_path: &Path,
        force: bool,
    ) -> Result<(), GitError> {
        Self::remove_worktree(repo_path, worktree_path, force)
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

    fn get_current_branch(&self, path: &Path) -> Result<String, GitError> {
        Self::get_current_branch(path)
    }
}

/// Type alias for GitWorktreeManager.
pub type GitManager = GitWorktreeManager;

/// Module-level convenience function to create a worktree.
pub fn create_worktree(
    repo_path: &Path,
    worktree_path: &Path,
    branch_name: &str,
) -> Result<(), GitError> {
    GitWorktreeManager::create_worktree(repo_path, worktree_path, branch_name)
}

/// Module-level convenience function to remove a worktree.
pub fn remove_worktree(
    repo_path: &Path,
    worktree_path: &Path,
    force: bool,
) -> Result<(), GitError> {
    GitWorktreeManager::remove_worktree(repo_path, worktree_path, force)
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
            other => panic!("Expected BranchAlreadyExists error, got: {:?}", other),
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
            other => panic!("Expected WorktreePathExists error, got: {:?}", other),
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
            other => panic!("Expected RepoNotFound error, got: {:?}", other),
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
            other => panic!("Expected WorktreeDirty error, got: {:?}", other),
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
            other => panic!("Expected WorktreeNotFound error, got: {:?}", other),
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
}
