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

#[test]
fn test_cli_create_without_attach_e2e() {
    let temp_dir = TempDir::new().expect("create tempdir");
    let repos_dir = temp_dir.path().join("repos");
    let backend_dir = repos_dir.join("backend");
    let frontend_dir = repos_dir.join("frontend");

    create_mock_git_repo(&backend_dir, "backend");
    create_mock_git_repo(&frontend_dir, "frontend");

    let workspace_cwd = temp_dir.path().join("workspace");
    fs::create_dir_all(&workspace_cwd).expect("create workspace cwd");

    let config_path = temp_dir.path().join("config.toml");
    let config_content = format!(
        r#"
        default_agent = "claude"
        repos_dir = "{}"
        [templates.fullstack]
        description = "Fullstack application stack"
        repos = ["{}", "{}"]
        "#,
        repos_dir.display(),
        backend_dir.display(),
        frontend_dir.display()
    );
    fs::write(&config_path, config_content).expect("write config");

    let mut cmd = Command::cargo_bin("botai").expect("binary exists");
    let assert = cmd
        .current_dir(&workspace_cwd)
        .args([
            "--config",
            config_path.to_str().unwrap(),
            "create",
            "fullstack",
            "feat-checkout",
        ])
        .assert();

    assert
        .success()
        .stdout(predicate::str::contains("[1/4] Checking repositories for template 'fullstack'..."))
        .stdout(predicate::str::contains("[3/4] Creating git worktree"))
        .stdout(predicate::str::contains("[4/4] Generating AGENTS.md"))
        .stdout(predicate::str::contains("✓ Project 'feat-checkout' created successfully at"))
        .stdout(predicate::str::contains("Hint: Run 'botai attach feat-checkout' to launch the agent session or run 'botai' to open the interactive dashboard."));

    let project_dir = workspace_cwd.join("feat-checkout");
    assert!(project_dir.exists(), "project root directory must exist");
    assert!(
        project_dir.join("AGENTS.md").exists(),
        "AGENTS.md must exist"
    );
    assert!(
        project_dir.join("backend").exists(),
        "backend worktree must exist"
    );
    assert!(
        project_dir.join("frontend").exists(),
        "frontend worktree must exist"
    );
    assert!(project_dir.join("backend").join("README.md").exists());
    assert!(project_dir.join("frontend").join("README.md").exists());

    // Check git branch in worktree
    let branch_output = std::process::Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(project_dir.join("backend"))
        .output()
        .expect("git rev-parse");
    let branch_name = String::from_utf8_lossy(&branch_output.stdout)
        .trim()
        .to_string();
    assert_eq!(branch_name, "feat-checkout");
}

#[test]
fn test_cli_create_prompt_without_attach_rejected_by_clap() {
    let temp_dir = TempDir::new().expect("create tempdir");
    let workspace_cwd = temp_dir.path().join("workspace");
    fs::create_dir_all(&workspace_cwd).expect("create workspace cwd");

    let mut cmd = Command::cargo_bin("botai").expect("binary exists");
    let assert = cmd
        .current_dir(&workspace_cwd)
        .args([
            "create",
            "fullstack",
            "feat-prompt-err",
            "--prompt",
            "Initial task prompt without attach",
        ])
        .assert();

    assert.failure();
    assert!(!workspace_cwd.join("feat-prompt-err").exists());
}

#[test]
fn test_cli_create_short_prompt_without_attach_rejected_by_clap() {
    let temp_dir = TempDir::new().expect("create tempdir");
    let workspace_cwd = temp_dir.path().join("workspace");
    fs::create_dir_all(&workspace_cwd).expect("create workspace cwd");

    let mut cmd = Command::cargo_bin("botai").expect("binary exists");
    let assert = cmd
        .current_dir(&workspace_cwd)
        .args(["create", "fullstack", "feat-short-err", "-p", "Task prompt"])
        .assert();

    assert.failure();
    assert!(!workspace_cwd.join("feat-short-err").exists());
}

#[test]
fn test_cli_start_subcommand_is_rejected_as_unknown() {
    let mut cmd = Command::cargo_bin("botai").expect("binary exists");
    let assert = cmd.args(["start", "fullstack", "feat-old-cmd"]).assert();

    assert.failure();
}

#[test]
fn test_cli_create_with_agent_flag_without_attach() {
    let temp_dir = TempDir::new().expect("create tempdir");
    let repos_dir = temp_dir.path().join("repos");
    let service_dir = repos_dir.join("service");
    create_mock_git_repo(&service_dir, "service");

    let workspace_cwd = temp_dir.path().join("workspace");
    fs::create_dir_all(&workspace_cwd).expect("create workspace cwd");

    let config_path = temp_dir.path().join("config.toml");
    let config_content = format!(
        r#"
        default_agent = "claude"
        repos_dir = "{}"
        [agents.custom]
        command = "echo"
        args = ["custom agent"]
        [templates.microservice]
        description = "Microservice stack"
        repos = ["{}"]
        "#,
        repos_dir.display(),
        service_dir.display()
    );
    fs::write(&config_path, config_content).expect("write config");

    let mut cmd = Command::cargo_bin("botai").expect("binary exists");
    let assert = cmd
        .current_dir(&workspace_cwd)
        .args([
            "--config",
            config_path.to_str().unwrap(),
            "create",
            "microservice",
            "feat-agent-override",
            "-A",
            "custom",
        ])
        .assert();

    assert
        .success()
        .stdout(predicate::str::contains("✓ Project 'feat-agent-override' created successfully at"))
        .stdout(predicate::str::contains("Hint: Run 'botai attach feat-agent-override' to launch the agent session or run 'botai' to open the interactive dashboard."));

    assert!(workspace_cwd
        .join("feat-agent-override")
        .join("service")
        .exists());
}

#[test]
fn test_project_builder_create_and_attach_lifecycle() {
    let temp_dir = TempDir::new().expect("create tempdir");
    let repos_dir = temp_dir.path().join("repos");
    let backend_dir = repos_dir.join("backend");
    create_mock_git_repo(&backend_dir, "backend");

    let workspace_cwd = temp_dir.path().join("workspace");
    fs::create_dir_all(&workspace_cwd).expect("create workspace cwd");

    let config_content = format!(
        r#"
        default_agent = "claude"
        repos_dir = "{}"
        [agents.claude]
        command = "echo"
        args = ["test-agent-run"]
        [templates.app]
        description = "App template"
        repos = ["{}"]
        "#,
        repos_dir.display(),
        backend_dir.display()
    );
    let config = botai::config::Config::load_from_str(&config_content).expect("load config");

    let builder = botai::project::ProjectBuilder::new(config, "app", "feat-attached")
        .with_cwd(&workspace_cwd)
        .with_prompt("Build user auth");

    let (ps, mut pty, vt) = builder
        .build(|_| {})
        .expect("build with attach should succeed");

    assert_eq!(ps.project_name, "feat-attached");
    assert_eq!(ps.template_name, "app");
    assert_eq!(ps.prompt.as_deref(), Some("Build user auth"));
    assert!(pty.is_alive() || pty.try_wait().is_ok());
    assert_eq!(vt.size(), (100, 30));

    let project_dir = workspace_cwd.join("feat-attached");
    assert!(project_dir.exists());
    assert!(project_dir.join("AGENTS.md").exists());
    assert!(project_dir.join("backend").exists());
}
