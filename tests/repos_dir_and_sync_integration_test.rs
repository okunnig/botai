use botai::config::Config;
use botai::context::{generate_agents_md, ProjectContext, RepoMapping};
use botai::git::{clone_repo, create_worktree, fetch_repo, get_current_branch};
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;

/// Helper function to create an initialized git repository with an initial commit.
fn create_mock_upstream_repo(base_dir: &TempDir, name: &str) -> PathBuf {
    let repo_path = base_dir.path().join(name);
    fs::create_dir_all(&repo_path).expect("Failed to create mock repo dir");

    let init_status = Command::new("git")
        .args(["init", "-b", "main"])
        .current_dir(&repo_path)
        .status()
        .or_else(|_| {
            Command::new("git")
                .arg("init")
                .current_dir(&repo_path)
                .status()
        })
        .expect("git init failed");
    assert!(init_status.success());

    let _ = Command::new("git")
        .args(["config", "user.name", "Upstream Tester"])
        .current_dir(&repo_path)
        .status();
    let _ = Command::new("git")
        .args(["config", "user.email", "upstream@botai.dev"])
        .current_dir(&repo_path)
        .status();

    fs::write(repo_path.join("README.md"), format!("# {name}\n")).expect("write file");

    let add = Command::new("git")
        .args(["add", "README.md"])
        .current_dir(&repo_path)
        .status()
        .expect("git add failed");
    assert!(add.success());

    let commit = Command::new("git")
        .args(["commit", "-m", "Initial commit"])
        .current_dir(&repo_path)
        .status()
        .expect("git commit failed");
    assert!(commit.success());

    repo_path
}

#[test]
fn test_integration_repos_dir_auto_cloning_and_sync() {
    let temp_dir = TempDir::new().expect("Failed to create tempdir");
    let upstreams_dir = temp_dir.path().join("upstreams");
    fs::create_dir_all(&upstreams_dir).expect("create upstreams dir");

    let upstream_api = create_mock_upstream_repo(&temp_dir, "upstreams/api-service");
    let upstream_web = create_mock_upstream_repo(&temp_dir, "upstreams/web-frontend");
    let local_shared = create_mock_upstream_repo(&temp_dir, "local-shared");

    let repos_dir = temp_dir.path().join("botai_repos");
    let config_path = temp_dir.path().join("config.toml");

    let toml_content = format!(
        r#"
default_agent = "claude"
repos_dir = "{}"

[agents.claude]
command = "echo"
args = ["started"]

[templates.platform]
description = "Platform template"
repos = [
    "file://{}",
    "file://{}",
    "{}"
]
"#,
        repos_dir.to_string_lossy().replace('\\', "\\\\"),
        upstream_api.to_string_lossy().replace('\\', "/"),
        upstream_web.to_string_lossy().replace('\\', "/"),
        local_shared.to_string_lossy().replace('\\', "\\\\")
    );

    fs::write(&config_path, toml_content).expect("write config");

    let config = Config::load_from_file(&config_path).expect("load config");
    assert_eq!(config.resolved_repos_dir(), repos_dir);

    // 1. Resolve template repos
    let resolved = config
        .resolve_template_repos("platform")
        .expect("resolve platform repos");
    assert_eq!(resolved.len(), 3);

    // Initially, the two remote repos are not yet cached locally in repos_dir
    assert_eq!(resolved[0].name, "api-service");
    assert_eq!(resolved[0].local_path, repos_dir.join("api-service"));
    assert!(!resolved[0].exists_locally());

    assert_eq!(resolved[1].name, "web-frontend");
    assert_eq!(resolved[1].local_path, repos_dir.join("web-frontend"));
    assert!(!resolved[1].exists_locally());

    // Local repo already exists
    assert_eq!(resolved[2].name, "local-shared");
    assert_eq!(resolved[2].local_path, local_shared);
    assert!(resolved[2].exists_locally());

    // 2. Perform sync (auto-clone missing repos into repos_dir)
    for repo in &resolved {
        if !repo.exists_locally() {
            if let Some(url) = &repo.clone_url {
                clone_repo(url, &repo.local_path).expect("clone repo during sync");
            }
        } else {
            let _ = fetch_repo(&repo.local_path);
        }
    }

    // Verify repos_dir now contains both cloned repos
    assert!(repos_dir.join("api-service").join("README.md").exists());
    assert!(repos_dir.join("web-frontend").join("README.md").exists());

    // 3. Create project with worktrees branching off the cloned repos
    let project_dir = temp_dir.path().join("my-project");
    fs::create_dir_all(&project_dir).expect("create project dir");

    let mut repo_mappings = Vec::new();
    for repo in &resolved {
        let worktree_path = project_dir.join(&repo.name);
        create_worktree(&repo.local_path, &worktree_path, "my-project").expect("create worktree");
        assert!(worktree_path.exists());
        assert_eq!(
            get_current_branch(&worktree_path).expect("branch"),
            "my-project"
        );
        repo_mappings.push(RepoMapping::new(
            repo.name.clone(),
            repo.local_path.clone(),
            worktree_path,
        ));
    }

    // 4. Generate AGENTS.md
    let project_context = ProjectContext::new("my-project", "platform")
        .with_description("Platform project")
        .with_repos(repo_mappings);

    let agents_md = generate_agents_md(&project_dir, &project_context).expect("generate agents md");
    assert!(agents_md.exists());
    let md_content = fs::read_to_string(&agents_md).expect("read agents md");
    assert!(md_content.contains("api-service"));
    assert!(md_content.contains("web-frontend"));
    assert!(md_content.contains("local-shared"));
}
