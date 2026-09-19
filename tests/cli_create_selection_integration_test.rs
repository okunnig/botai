use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

/// Helper function to create an initialized git repository with an initial commit.
fn create_mock_git_repo(path: &Path, name: &str) {
    fs::create_dir_all(path).expect("Failed to create mock repo dir");

    let init = std::process::Command::new("git")
        .args(["init", "-b", "main"])
        .current_dir(path)
        .status()
        .or_else(|_| {
            std::process::Command::new("git")
                .arg("init")
                .current_dir(path)
                .status()
        })
        .expect("git init failed");
    assert!(init.success());

    let _ = std::process::Command::new("git")
        .args(["config", "user.name", "Botai Test"])
        .current_dir(path)
        .status();
    let _ = std::process::Command::new("git")
        .args(["config", "user.email", "test@botai.dev"])
        .current_dir(path)
        .status();

    fs::write(path.join("README.md"), format!("# {name}\n")).expect("write README");
    let add = std::process::Command::new("git")
        .args(["add", "README.md"])
        .current_dir(path)
        .status()
        .expect("git add failed");
    assert!(add.success());

    let commit = std::process::Command::new("git")
        .args(["commit", "-m", "Initial commit"])
        .current_dir(path)
        .status()
        .expect("git commit failed");
    assert!(commit.success());
}

fn setup_test_environment() -> (TempDir, std::path::PathBuf, std::path::PathBuf) {
    let temp_dir = TempDir::new().expect("create tempdir");
    let repos_dir = temp_dir.path().join("repos");
    let backend_dir = repos_dir.join("backend");
    let frontend_dir = repos_dir.join("frontend");
    let docs_dir = repos_dir.join("docs");

    create_mock_git_repo(&backend_dir, "backend");
    create_mock_git_repo(&frontend_dir, "frontend");
    create_mock_git_repo(&docs_dir, "docs");

    let workspace_cwd = temp_dir.path().join("workspace");
    fs::create_dir_all(&workspace_cwd).expect("create workspace cwd");

    let config_path = temp_dir.path().join("config.toml");
    let config_content = format!(
        r#"
        default_agent = "claude"
        repos_dir = "{}"

        [templates.fullstack]
        description = "Fullstack stack with optional docs"
        repos = [
            "{}",
            "{}",
            {{ repo = "{}", default = false }}
        ]
        "#,
        repos_dir.to_string_lossy().replace('\\', "/"),
        backend_dir.to_string_lossy().replace('\\', "/"),
        frontend_dir.to_string_lossy().replace('\\', "/"),
        docs_dir.to_string_lossy().replace('\\', "/")
    );
    fs::write(&config_path, config_content).expect("write config");

    (temp_dir, config_path, workspace_cwd)
}

#[test]
fn test_cli_create_exclude_flag_omits_repo() {
    let (_temp_dir, config_path, workspace_cwd) = setup_test_environment();

    let mut cmd = Command::cargo_bin("botai").expect("binary exists");
    let assert = cmd
        .current_dir(&workspace_cwd)
        .args([
            "--config",
            config_path.to_str().unwrap(),
            "create",
            "fullstack",
            "proj-backend-only",
            "--exclude",
            "frontend",
        ])
        .assert();

    assert.success().stdout(predicate::str::contains(
        "✓ Project 'proj-backend-only' created successfully",
    ));

    let project_dir = workspace_cwd.join("proj-backend-only");
    assert!(project_dir.exists(), "project root must exist");
    assert!(project_dir.join("AGENTS.md").exists());
    assert!(
        project_dir.join("backend").exists(),
        "backend worktree should exist"
    );
    assert!(
        !project_dir.join("frontend").exists(),
        "excluded frontend worktree must NOT exist"
    );
    assert!(
        !project_dir.join("docs").exists(),
        "default-false docs worktree must NOT exist"
    );

    let agents_md = fs::read_to_string(project_dir.join("AGENTS.md")).expect("read AGENTS.md");
    assert!(agents_md.contains("| backend |"));
    assert!(!agents_md.contains("| frontend |"));
    assert!(!agents_md.contains("| docs |"));
}

#[test]
fn test_cli_create_include_flag_adds_default_false_repo() {
    let (_temp_dir, config_path, workspace_cwd) = setup_test_environment();

    let mut cmd = Command::cargo_bin("botai").expect("binary exists");
    let assert = cmd
        .current_dir(&workspace_cwd)
        .args([
            "--config",
            config_path.to_str().unwrap(),
            "create",
            "fullstack",
            "proj-with-docs",
            "--include",
            "docs",
        ])
        .assert();

    assert.success().stdout(predicate::str::contains(
        "✓ Project 'proj-with-docs' created successfully",
    ));

    let project_dir = workspace_cwd.join("proj-with-docs");
    assert!(project_dir.exists(), "project root must exist");
    assert!(project_dir.join("backend").exists());
    assert!(project_dir.join("frontend").exists());
    assert!(
        project_dir.join("docs").exists(),
        "included docs worktree must exist"
    );

    let agents_md = fs::read_to_string(project_dir.join("AGENTS.md")).expect("read AGENTS.md");
    assert!(agents_md.contains("| backend |"));
    assert!(agents_md.contains("| frontend |"));
    assert!(agents_md.contains("| docs |"));
}

#[test]
fn test_cli_create_all_flag_includes_all_repos() {
    let (_temp_dir, config_path, workspace_cwd) = setup_test_environment();

    let mut cmd = Command::cargo_bin("botai").expect("binary exists");
    let assert = cmd
        .current_dir(&workspace_cwd)
        .args([
            "--config",
            config_path.to_str().unwrap(),
            "create",
            "fullstack",
            "proj-all-repos",
            "--all",
        ])
        .assert();

    assert.success().stdout(predicate::str::contains(
        "✓ Project 'proj-all-repos' created successfully",
    ));

    let project_dir = workspace_cwd.join("proj-all-repos");
    assert!(project_dir.exists(), "project root must exist");
    assert!(project_dir.join("backend").exists());
    assert!(project_dir.join("frontend").exists());
    assert!(project_dir.join("docs").exists());

    let agents_md = fs::read_to_string(project_dir.join("AGENTS.md")).expect("read AGENTS.md");
    assert!(agents_md.contains("| backend |"));
    assert!(agents_md.contains("| frontend |"));
    assert!(agents_md.contains("| docs |"));
}

#[test]
fn test_cli_create_conflict_between_include_and_exclude_fails() {
    let (_temp_dir, config_path, workspace_cwd) = setup_test_environment();

    let mut cmd = Command::cargo_bin("botai").expect("binary exists");
    let assert = cmd
        .current_dir(&workspace_cwd)
        .args([
            "--config",
            config_path.to_str().unwrap(),
            "create",
            "fullstack",
            "proj-conflict",
            "--include",
            "docs",
            "--exclude",
            "docs",
        ])
        .assert();

    assert.failure().stderr(predicate::str::contains(
        "Cannot both include and exclude repository 'docs'",
    ));

    let project_dir = workspace_cwd.join("proj-conflict");
    assert!(
        !project_dir.exists(),
        "project directory must not be created on validation failure"
    );
}

#[test]
fn test_cli_create_unknown_repo_fails() {
    let (_temp_dir, config_path, workspace_cwd) = setup_test_environment();

    let mut cmd = Command::cargo_bin("botai").expect("binary exists");
    let assert = cmd
        .current_dir(&workspace_cwd)
        .args([
            "--config",
            config_path.to_str().unwrap(),
            "create",
            "fullstack",
            "proj-unknown",
            "--include",
            "unknown-repo",
        ])
        .assert();

    assert.failure().stderr(predicate::str::contains(
        "Repository 'unknown-repo' not found in template. Available repositories:",
    ));

    let project_dir = workspace_cwd.join("proj-unknown");
    assert!(
        !project_dir.exists(),
        "project directory must not be created on validation failure"
    );
}

#[test]
fn test_cli_create_empty_selection_fails() {
    let (_temp_dir, config_path, workspace_cwd) = setup_test_environment();

    let mut cmd = Command::cargo_bin("botai").expect("binary exists");
    let assert = cmd
        .current_dir(&workspace_cwd)
        .args([
            "--config",
            config_path.to_str().unwrap(),
            "create",
            "fullstack",
            "proj-empty",
            "--exclude",
            "backend,frontend",
        ])
        .assert();

    assert.failure().stderr(predicate::str::contains(
        "Cannot create project: at least one repository must be selected.",
    ));

    let project_dir = workspace_cwd.join("proj-empty");
    assert!(
        !project_dir.exists(),
        "project directory must not be created on empty selection failure"
    );
}
