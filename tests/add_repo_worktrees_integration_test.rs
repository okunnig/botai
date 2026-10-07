use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::fs;
use std::time::Duration;
use tempfile::tempdir;

use botai::config::Config;
use botai::project::add_worktrees_to_project;
use botai::project::builder::ProjectBuilder;
use botai::session::SessionRegistry;
use botai::tui::dashboard::DashboardAction;
use botai::tui::events::RawAction;
use botai::tui::TuiApp;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn create_mock_git_repo(path: &std::path::Path) {
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
        .args(["commit", "-m", "init"])
        .current_dir(path)
        .status();
}

#[test]
fn test_integration_add_worktrees_backend_pipeline() {
    let tmp = tempdir().expect("tempdir");
    let cwd = tmp.path().to_path_buf();
    let repos_dir = cwd.join(".botai_repos");
    fs::create_dir_all(&repos_dir).unwrap();

    let repo_a_path = repos_dir.join("repo-a");
    let repo_b_path = repos_dir.join("repo-b");
    let repo_c_path = repos_dir.join("repo-c");
    create_mock_git_repo(&repo_a_path);
    create_mock_git_repo(&repo_b_path);
    create_mock_git_repo(&repo_c_path);

    #[cfg(windows)]
    let setup_cmd = "[\"cmd.exe\", \"/c\", \"echo ran > hook_ran.txt\"]";
    #[cfg(not(windows))]
    let setup_cmd = "[\"touch\", \"hook_ran.txt\"]";

    let config_toml = format!(
        r#"
default_agent = "mock"
repos_dir = "{}"

[agents.mock]
command = "sh"
args = ["-c", "exit 0"]

[repos.repo-b]
setup = [{}]

[templates.multi]
description = "Multi repo template"
repos = ["repo-a", "repo-b", "repo-c"]
"#,
        repos_dir.display().to_string().replace('\\', "/"),
        setup_cmd
    );

    let config = Config::load_from_str(&config_toml).unwrap();

    // 1. Initially create project with repo-a only
    let builder = ProjectBuilder::new(config.clone(), "multi", "my-proj")
        .with_cwd(cwd.clone())
        .with_selected_repos(vec!["repo-a".to_string()])
        .with_prompt("Fix crucial billing issue in repo-a");
    let (ps, mut pty, vt) = builder.build(|_| {}).unwrap();
    let _ = pty.kill();

    let proj_dir = cwd.join("my-proj");
    assert!(proj_dir.join("repo-a").join(".git").exists());
    assert!(!proj_dir.join("repo-b").exists());
    assert!(!proj_dir.join("repo-c").exists());

    let initial_agents_md = fs::read_to_string(proj_dir.join("AGENTS.md")).unwrap();
    assert!(initial_agents_md.contains("repo-a"));
    assert!(!initial_agents_md.contains("repo-b"));
    assert!(initial_agents_md.contains("Fix crucial billing issue in repo-a"));

    // 2. Add repo-b and repo-c using add_worktrees_to_project
    let (tx, _rx) = std::sync::mpsc::channel();
    let added = add_worktrees_to_project(
        &config,
        &proj_dir,
        &["repo-b".to_string(), "repo-c".to_string()],
        move |ev| {
            let _ = tx.send(ev);
        },
    )
    .unwrap();

    let added_names: Vec<String> = added.iter().map(|r| r.name.clone()).collect();
    assert_eq!(
        added_names,
        vec!["repo-b".to_string(), "repo-c".to_string()]
    );
    assert!(proj_dir.join("repo-b").join(".git").exists());
    assert!(proj_dir.join("repo-c").join(".git").exists());

    // 3. Verify setup hook ran in repo-b
    assert!(proj_dir.join("repo-b").join("hook_ran.txt").exists());

    // 4. Verify AGENTS.md re-synthesis
    let updated_agents_md = fs::read_to_string(proj_dir.join("AGENTS.md")).unwrap();
    assert!(updated_agents_md.contains("repo-a"));
    assert!(updated_agents_md.contains("repo-b"));
    assert!(updated_agents_md.contains("repo-c"));
    // Verify template order preserved
    let pos_a = updated_agents_md.find("repo-a").unwrap();
    let pos_b = updated_agents_md.find("repo-b").unwrap();
    let pos_c = updated_agents_md.find("repo-c").unwrap();
    assert!(pos_a < pos_b);
    assert!(pos_b < pos_c);
    // Verify original prompt preserved
    assert!(updated_agents_md.contains("Fix crucial billing issue in repo-a"));

    // Cleanup session
    drop(ps);
    drop(vt);
}

#[test]
fn test_integration_tui_app_in_session_add_repo_lifecycle() {
    let tmp = tempdir().expect("tempdir");
    let cwd = tmp.path().to_path_buf();
    let repos_dir = cwd.join(".botai_repos");
    fs::create_dir_all(&repos_dir).unwrap();

    let repo_a_path = repos_dir.join("repo-a");
    let repo_b_path = repos_dir.join("repo-b");
    create_mock_git_repo(&repo_a_path);
    create_mock_git_repo(&repo_b_path);

    let config_toml = format!(
        r#"
default_agent = "mock"
repos_dir = "{}"

[agents.mock]
command = "sh"
args = ["-c", "exit 0"]

[templates.app_tpl]
repos = ["repo-a", "repo-b"]
"#,
        repos_dir.display().to_string().replace('\\', "/")
    );
    let config = Config::load_from_str(&config_toml).unwrap();

    let builder = ProjectBuilder::new(config.clone(), "app_tpl", "session-proj")
        .with_cwd(cwd.clone())
        .with_selected_repos(vec!["repo-a".to_string()]);
    let (ps, pty, vt) = builder.build(|_| {}).unwrap();

    let mut registry = SessionRegistry::new();
    registry.add(ps, pty, vt).unwrap();

    let mut app = TuiApp::new(registry).with_config(config).with_cwd(cwd);

    // 1. In active session, trigger Add Repo via leader key 'a'
    app.handle_raw_action(RawAction::AddRepo);
    assert!(app.add_repo_modal.is_some());
    let modal = app.add_repo_modal.as_ref().unwrap();
    assert_eq!(modal.items.len(), 1);
    assert_eq!(modal.items[0].name, "repo-b");
    assert!(!modal.items[0].selected);

    // 2. Select repo-b and submit
    app.handle_add_repo_modal_key(key(KeyCode::Char(' ')));
    assert!(app.add_repo_modal.as_ref().unwrap().items[0].selected);
    app.handle_add_repo_modal_key(key(KeyCode::Enter));

    assert!(app.add_repo_modal.is_none());
    assert!(app.add_repo_progress_modal.is_some());

    // 3. Poll worker thread until finished
    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_secs(10) {
        let _ = app.poll_add_repo_progress();
        if app
            .add_repo_progress_modal
            .as_ref()
            .map(|m| m.is_finished)
            .unwrap_or(false)
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    assert!(app.add_repo_progress_modal.as_ref().unwrap().is_finished);
    assert!(app.add_repo_progress_modal.as_ref().unwrap().is_success);

    // 4. Dismiss progress modal with Enter
    app.handle_add_repo_progress_modal_key(key(KeyCode::Enter));
    assert!(app.add_repo_progress_modal.is_none());

    // 5. Verify status notification is active in session
    assert_eq!(
        app.status_notification.as_deref(),
        Some("✓ Added repository 'repo-b' to project 'session-proj'")
    );

    // 6. Typing into session clears notification
    app.handle_raw_action(RawAction::Passthrough(b"clear\n".to_vec()));
    assert!(app.status_notification.is_none());
}

#[test]
fn test_integration_tui_app_dashboard_add_repo_action() {
    let tmp = tempdir().expect("tempdir");
    let cwd = tmp.path().to_path_buf();
    let repos_dir = cwd.join(".botai_repos");
    fs::create_dir_all(&repos_dir).unwrap();

    let repo_a_path = repos_dir.join("repo-a");
    let repo_b_path = repos_dir.join("repo-b");
    create_mock_git_repo(&repo_a_path);
    create_mock_git_repo(&repo_b_path);

    let config_toml = format!(
        r#"
default_agent = "mock"
repos_dir = "{}"

[templates.dash_tpl]
repos = ["repo-a", "repo-b"]
"#,
        repos_dir.display().to_string().replace('\\', "/")
    );
    let config = Config::load_from_str(&config_toml).unwrap();

    let proj_dir = cwd.join("dash-proj");
    fs::create_dir_all(proj_dir.join("repo-a").join(".git")).unwrap();
    fs::write(
        proj_dir.join("AGENTS.md"),
        "# Project: dash-proj\n\nTemplate: **dash_tpl**\n",
    )
    .unwrap();

    let mut app = TuiApp::new(SessionRegistry::new())
        .with_config(config)
        .with_cwd(cwd);
    app.refresh_dashboard();
    app.showing_dashboard = true;

    // Trigger Add Repo from dashboard
    app.handle_dashboard_action(DashboardAction::AddRepo("dash-proj".to_string()));
    assert!(app.add_repo_modal.is_some());

    // Select and submit
    app.handle_add_repo_modal_key(key(KeyCode::Char(' ')));
    app.handle_add_repo_modal_key(key(KeyCode::Enter));

    assert!(app.add_repo_modal.is_none());
    assert!(app.add_repo_progress_modal.is_some());

    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_secs(10) {
        let _ = app.poll_add_repo_progress();
        if app
            .add_repo_progress_modal
            .as_ref()
            .map(|m| m.is_finished)
            .unwrap_or(false)
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    assert!(app.add_repo_progress_modal.as_ref().unwrap().is_finished);

    // Dismiss modal on dashboard -> refreshes dashboard without session notification
    app.handle_add_repo_progress_modal_key(key(KeyCode::Enter));
    assert!(app.add_repo_progress_modal.is_none());
    assert!(app.status_notification.is_none());
    assert!(app.is_showing_dashboard());
}
