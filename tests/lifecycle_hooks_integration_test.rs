use std::fs;
use std::path::Path;
use tempfile::tempdir;

use botai::config::Config;
use botai::project::{destroy_project_workspace, ProjectBuilder, ProjectSetupEvent};

fn create_mock_git_repo(path: &Path) {
    fs::create_dir_all(path).expect("create repo dir");
    let _ = std::process::Command::new("git")
        .args(["init", "-b", "main"])
        .current_dir(path)
        .status()
        .or_else(|_| {
            std::process::Command::new("git")
                .arg("init")
                .current_dir(path)
                .status()
        })
        .expect("git init");

    let _ = std::process::Command::new("git")
        .args(["config", "user.name", "Lifecycle Tester"])
        .current_dir(path)
        .status();
    let _ = std::process::Command::new("git")
        .args(["config", "user.email", "lifecycle@botai.dev"])
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
fn test_parallel_per_repo_setup_hooks_and_post_create_lifecycle() {
    let tmp = tempdir().expect("tempdir");
    let repos_dir = tmp.path().join("repos");
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
            ["sh", "-c", "echo step1 > backend_order.log"],
            ["sh", "-c", "echo step2 >> backend_order.log && echo $REPO_SECRET > backend_env.log"]
        ]
        env = {{ REPO_SECRET = "backend_secret_xyz" }}

        [repos.frontend]
        setup = [
            "sh -c 'echo frontend_done > frontend_status.log && echo $TEMPLATE_SHARED > frontend_env.log'"
        ]

        [templates.fullstack]
        repos = ["backend", "frontend"]
        post_create = [
            ["sh", "-c", "echo post_create_done > project_root.log && echo $TEMPLATE_SHARED >> project_root.log"]
        ]
        env = {{ TEMPLATE_SHARED = "shared_template_val" }}
        "#,
        repos_dir.to_string_lossy().replace('\\', "/")
    );

    let config = Config::load_from_str(&toml_str).expect("Valid config");
    let workspaces_dir = tmp.path().join("workspaces");
    fs::create_dir_all(&workspaces_dir).expect("create workspaces dir");

    let builder =
        ProjectBuilder::new(config, "fullstack", "feat-lifecycle").with_cwd(&workspaces_dir);

    let (project_dir, mappings) = builder
        .create_workspace(|_| {})
        .expect("create_workspace must succeed");

    assert_eq!(mappings.len(), 2);
    assert!(project_dir.exists());

    // 1. Verify multi-command sequential execution in backend worktree
    let backend_order_path = project_dir.join("backend").join("backend_order.log");
    assert!(backend_order_path.exists(), "backend_order.log must exist");
    let backend_order_content = fs::read_to_string(backend_order_path).unwrap();
    assert_eq!(backend_order_content.trim(), "step1\nstep2");

    // 2. Verify repo-specific environment variable in backend worktree
    let backend_env_path = project_dir.join("backend").join("backend_env.log");
    assert!(backend_env_path.exists(), "backend_env.log must exist");
    let backend_env_content = fs::read_to_string(backend_env_path).unwrap();
    assert_eq!(backend_env_content.trim(), "backend_secret_xyz");

    // 3. Verify frontend worktree setup hook and inherited template env
    let frontend_status_path = project_dir.join("frontend").join("frontend_status.log");
    assert!(
        frontend_status_path.exists(),
        "frontend_status.log must exist"
    );
    assert_eq!(
        fs::read_to_string(frontend_status_path).unwrap().trim(),
        "frontend_done"
    );

    let frontend_env_path = project_dir.join("frontend").join("frontend_env.log");
    assert!(frontend_env_path.exists(), "frontend_env.log must exist");
    assert_eq!(
        fs::read_to_string(frontend_env_path).unwrap().trim(),
        "shared_template_val"
    );

    // 4. Verify post_create hook executed at project root
    let root_log_path = project_dir.join("project_root.log");
    assert!(root_log_path.exists(), "project_root.log must exist");
    let root_log_content = fs::read_to_string(root_log_path).unwrap();
    assert_eq!(
        root_log_content.trim(),
        "post_create_done\nshared_template_val"
    );
}

#[test]
fn test_setup_hooks_warn_and_continue_policy() {
    let tmp = tempdir().expect("tempdir");
    let repos_dir = tmp.path().join("repos");
    let repo_backend = repos_dir.join("backend");
    create_mock_git_repo(&repo_backend);

    let toml_str = format!(
        r#"
        default_agent = "claude"
        repos_dir = "{}"

        [repos.backend]
        setup = [
            ["non_existent_binary_for_testing_warn_and_continue"],
            ["sh", "-c", "exit 101"],
            ["sh", "-c", "echo completed_subsequent_step > survivor.log"]
        ]

        [templates.resilient]
        repos = ["backend"]
        post_create = [
            ["non_existent_post_create_tool"]
        ]
        "#,
        repos_dir.to_string_lossy().replace('\\', "/")
    );

    let config = Config::load_from_str(&toml_str).expect("Valid config");
    let workspaces_dir = tmp.path().join("workspaces");
    fs::create_dir_all(&workspaces_dir).expect("create workspaces dir");

    let builder =
        ProjectBuilder::new(config, "resilient", "feat-resilient").with_cwd(&workspaces_dir);

    let logs = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let logs_clone = std::sync::Arc::clone(&logs);

    let res = builder.create_workspace(move |ev| {
        if let ProjectSetupEvent::Log(msg) = ev {
            logs_clone.lock().unwrap().push(msg);
        }
    });

    // Must NOT roll back or fail project creation
    assert!(
        res.is_ok(),
        "create_workspace must succeed under warn-and-continue policy"
    );
    let (project_dir, _) = res.unwrap();
    assert!(project_dir.exists());
    assert!(project_dir.join("AGENTS.md").exists());
    assert!(project_dir.join("backend").exists());
    assert!(project_dir.join("backend").join("survivor.log").exists());

    // Verify warning log events were emitted
    let log_entries = logs.lock().unwrap();
    assert!(
        log_entries
            .iter()
            .any(|l| l.contains("⚠ Warning: Setup hook for 'backend' failed")),
        "Must log setup hook failure warning"
    );
    assert!(
        log_entries
            .iter()
            .any(|l| l.contains("⚠ Warning: Post-create hook failed")),
        "Must log post-create hook failure warning"
    );
}

#[test]
fn test_pre_destroy_hook_lifecycle_on_destroy() {
    let tmp = tempdir().expect("tempdir");
    let repos_dir = tmp.path().join("repos");
    let repo_backend = repos_dir.join("backend");
    create_mock_git_repo(&repo_backend);

    let pre_destroy_marker = tmp.path().join("pre_destroy_executed.log");

    let toml_str = format!(
        r#"
        default_agent = "claude"
        repos_dir = "{}"

        [templates.microservice]
        repos = ["backend"]
        pre_destroy = [
            ["sh", "-c", "echo pre_destroy_ok > {}"]
        ]
        "#,
        repos_dir.to_string_lossy().replace('\\', "/"),
        pre_destroy_marker.to_string_lossy().replace('\\', "/")
    );

    let config = Config::load_from_str(&toml_str).expect("Valid config");
    let workspaces_dir = tmp.path().join("workspaces");
    fs::create_dir_all(&workspaces_dir).expect("create workspaces dir");

    let project_name = "feat-pre-destroy";
    let builder =
        ProjectBuilder::new(config.clone(), "microservice", project_name).with_cwd(&workspaces_dir);

    let (project_dir, _) = builder.create_workspace(|_| {}).expect("Workspace created");
    assert!(project_dir.exists());
    assert!(project_dir.join("AGENTS.md").exists());
    assert!(!pre_destroy_marker.exists());

    // Destroy project workspace
    destroy_project_workspace(Some(&config), &workspaces_dir, project_name, true)
        .expect("destroy_project_workspace must succeed");

    // Verify project dir was deleted
    assert!(!project_dir.exists(), "project_dir must be deleted");

    // Verify pre_destroy hook was executed
    assert!(
        pre_destroy_marker.exists(),
        "pre_destroy marker must exist after project destruction"
    );
    assert_eq!(
        fs::read_to_string(&pre_destroy_marker).unwrap().trim(),
        "pre_destroy_ok"
    );
}
