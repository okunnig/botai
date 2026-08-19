use botai::context::{generate_agents_md, ProjectContext, RepoMapping};
use botai::git::{
    branch_exists, create_worktree, get_current_branch, has_uncommitted_changes, remove_worktree,
    GitError,
};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;

/// Helper function to create an initialized git repository with an initial commit.
fn create_mock_repo(base_dir: &TempDir, name: &str, sample_file: &str, content: &str) -> PathBuf {
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
        .args(["config", "user.name", "Integration Tester"])
        .current_dir(&repo_path)
        .status();
    let _ = Command::new("git")
        .args(["config", "user.email", "integration@botai.dev"])
        .current_dir(&repo_path)
        .status();

    fs::write(repo_path.join(sample_file), content).expect("Failed to write initial file");

    let add = Command::new("git")
        .args(["add", sample_file])
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
fn test_full_git_worktree_and_context_synthesis_lifecycle() {
    let temp_dir = TempDir::new().expect("Failed to create integration tempdir");

    // 1. Setup two mock repos (frontend and backend)
    let frontend_repo = create_mock_repo(
        &temp_dir,
        "repo_frontend",
        "package.json",
        r#"{"name": "frontend", "version": "1.0.0"}"#,
    );
    let backend_repo = create_mock_repo(
        &temp_dir,
        "repo_backend",
        "Cargo.toml",
        "[package]\nname = \"backend\"\nversion = \"0.1.0\"\n",
    );

    // 2. Define project structure and context
    let project_name = "my-feature";
    let template_name = "fullstack";
    let project_root = temp_dir.path().join("projects").join(project_name);
    fs::create_dir_all(&project_root).expect("Failed to create project root");

    let frontend_wt = project_root.join("frontend");
    let backend_wt = project_root.join("backend");

    let mut env = HashMap::new();
    env.insert("PORT".to_string(), "3000".to_string());
    env.insert("API_URL".to_string(), "http://localhost:8080".to_string());

    let repo_mappings = vec![
        RepoMapping::new("frontend", &frontend_repo, &frontend_wt),
        RepoMapping::new("backend", &backend_repo, &backend_wt),
    ];

    let context = ProjectContext::new(project_name, template_name)
        .with_description("Fullstack web app with Rust backend and React frontend")
        .with_repos(repo_mappings)
        .with_env(env)
        .with_initial_prompt("Implement user authentication with JWT tokens.");

    // 3. Create worktrees for both repos
    create_worktree(&frontend_repo, &frontend_wt, project_name)
        .expect("Failed to create frontend worktree");
    create_worktree(&backend_repo, &backend_wt, project_name)
        .expect("Failed to create backend worktree");

    // Verify worktrees exist and have the original files
    assert!(frontend_wt.join("package.json").exists());
    assert!(backend_wt.join("Cargo.toml").exists());

    // Verify checked out branch
    assert_eq!(
        get_current_branch(&frontend_wt).expect("frontend branch"),
        project_name
    );
    assert_eq!(
        get_current_branch(&backend_wt).expect("backend branch"),
        project_name
    );

    // 4. Synthesize and generate AGENTS.md
    let agents_md_path = generate_agents_md(&project_root, &context)
        .expect("Failed to generate AGENTS.md in project root");
    assert_eq!(agents_md_path, project_root.join("AGENTS.md"));
    assert!(agents_md_path.exists());

    let agents_content = fs::read_to_string(&agents_md_path).expect("Failed to read AGENTS.md");
    assert!(agents_content.contains(project_name));
    assert!(agents_content.contains(template_name));
    assert!(agents_content.contains("Fullstack web app with Rust backend and React frontend"));
    assert!(agents_content.contains("frontend"));
    assert!(agents_content.contains("backend"));
    assert!(agents_content.contains("PORT"));
    assert!(agents_content.contains("3000"));
    assert!(agents_content.contains("Implement user authentication with JWT tokens."));

    // 5. Verify initial clean state
    assert!(!has_uncommitted_changes(&frontend_wt).expect("frontend clean check"));
    assert!(!has_uncommitted_changes(&backend_wt).expect("backend clean check"));

    // 6. Introduce modification in frontend worktree
    fs::write(
        frontend_wt.join("src_App.js"),
        "console.log('auth started');",
    )
    .expect("write untracked file");

    assert!(has_uncommitted_changes(&frontend_wt).expect("frontend dirty check"));
    assert!(!has_uncommitted_changes(&backend_wt).expect("backend dirty check"));

    // 7. Verify safe deletion refusal on dirty worktree
    let remove_dirty_res = remove_worktree(&frontend_repo, &frontend_wt, false);
    match remove_dirty_res {
        Err(GitError::WorktreeDirty { path }) => {
            assert_eq!(path, frontend_wt);
        }
        other => panic!("Expected WorktreeDirty error, got {:?}", other),
    }
    assert!(frontend_wt.exists(), "Dirty worktree must not be removed");

    // 8. Clean up worktrees
    remove_worktree(&frontend_repo, &frontend_wt, true)
        .expect("Failed to force remove frontend worktree");
    remove_worktree(&backend_repo, &backend_wt, false).expect("Failed to remove backend worktree");

    assert!(!frontend_wt.exists(), "Frontend worktree must be removed");
    assert!(!backend_wt.exists(), "Backend worktree must be removed");

    // 9. Verify branches are preserved in source repositories
    assert!(
        branch_exists(&frontend_repo, project_name).expect("frontend branch check"),
        "Git branch must remain in frontend repo"
    );
    assert!(
        branch_exists(&backend_repo, project_name).expect("backend branch check"),
        "Git branch must remain in backend repo"
    );
}

#[test]
fn test_template_agents_md_append_integration() {
    let temp_dir = TempDir::new().expect("Failed to create integration tempdir");

    let repo = create_mock_repo(
        &temp_dir,
        "service_repo",
        "README.md",
        "# Service Repository\n",
    );

    let project_name = "custom-instructions-feature";
    let template_name = "dev-template";
    let project_root = temp_dir.path().join("projects").join(project_name);
    fs::create_dir_all(&project_root).expect("Failed to create project root");

    let service_wt = project_root.join("service");
    create_worktree(&repo, &service_wt, project_name).expect("Failed to create worktree");

    let custom_instructions = "### Template Rules\n- Strictly adhere to TDD.\n- Run `cargo clippy --all-targets` before submitting.";

    let context = ProjectContext::new(project_name, template_name)
        .with_description("Template with custom agent rules")
        .with_repos(vec![RepoMapping::new("service", &repo, &service_wt)])
        .with_agents_md_append(custom_instructions);

    let agents_md_path = generate_agents_md(&project_root, &context)
        .expect("Failed to generate AGENTS.md");
    assert!(agents_md_path.exists());

    let content = fs::read_to_string(&agents_md_path).expect("Failed to read AGENTS.md");
    assert!(content.contains(project_name));
    assert!(content.contains(template_name));
    assert!(content.contains("## Template Instructions"));
    assert!(content.contains("### Template Rules"));
    assert!(content.contains("- Strictly adhere to TDD."));
    assert!(content.contains("- Run `cargo clippy --all-targets` before submitting."));
}
