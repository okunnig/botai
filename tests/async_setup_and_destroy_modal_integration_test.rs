use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use std::fs;
use std::time::Duration;
use tempfile::tempdir;

use botai::config::Config;
use botai::session::SessionRegistry;
use botai::tui::dashboard::{DashboardAction, DashboardState, DestroyModalState};
use botai::tui::destroy_progress_modal::render_destroy_progress_modal;
use botai::tui::setup_modal::render_setup_modal;
use botai::tui::theme::Theme;
use botai::tui::TuiApp;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

#[test]
fn test_integration_destroy_confirmation_modal_lifecycle() {
    let tmp = tempdir().expect("tempdir");
    let cwd = tmp.path().to_path_buf();

    // Create mock projects
    let proj_keep = cwd.join("service-keep");
    let proj_del = cwd.join("service-del");
    fs::create_dir(&proj_keep).unwrap();
    fs::write(proj_keep.join("AGENTS.md"), "# Keep").unwrap();
    fs::create_dir(&proj_del).unwrap();
    fs::write(proj_del.join("AGENTS.md"), "# Delete").unwrap();

    let mut state = DashboardState::new(
        vec!["template-a".into()],
        vec!["service-del".into(), "service-keep".into()],
    );
    state.selected_project_idx = 0; // service-del

    // 1. Press 'd' -> Opens destroy confirmation modal without emitting Destroy action
    let action = state.handle_key(key(KeyCode::Char('d')));
    assert_eq!(action, DashboardAction::None);
    assert_eq!(
        state.destroy_modal,
        Some(DestroyModalState::new("service-del"))
    );

    // 2. Render while confirmation modal is open
    let area = Rect::new(0, 0, 100, 30);
    let mut buf = Buffer::empty(area);
    let theme = Theme::default();
    botai::tui::render_dashboard(&state, area, &mut buf, &theme);
    let rendered = buf
        .content()
        .iter()
        .map(ratatui::buffer::Cell::symbol)
        .collect::<String>();
    assert!(rendered.contains("Confirm Destroy Project"));
    assert!(rendered.contains("service-del"));
    assert!(rendered.contains("[y] / [Enter] Confirm"));
    assert_eq!(state.cursor_position(area), None);

    // 3. Press 'n' -> Cancels modal
    let action = state.handle_key(key(KeyCode::Char('n')));
    assert_eq!(action, DashboardAction::None);
    assert!(state.destroy_modal.is_none());

    // 4. Press 'd' again and cancel with Esc
    state.handle_key(key(KeyCode::Char('d')));
    assert!(state.destroy_modal.is_some());
    let action = state.handle_key(key(KeyCode::Esc));
    assert_eq!(action, DashboardAction::None);
    assert!(state.destroy_modal.is_none());

    // 5. Press 'd' again and confirm with Enter
    state.handle_key(key(KeyCode::Char('d')));
    assert!(state.destroy_modal.is_some());
    let action = state.handle_key(key(KeyCode::Enter));
    assert_eq!(action, DashboardAction::Destroy("service-del".to_string()));
    assert!(state.destroy_modal.is_none());

    // Execute destroy on TuiApp
    let registry = SessionRegistry::new();
    let mut app = TuiApp::new(registry)
        .with_dashboard(state)
        .with_cwd(cwd.clone());

    app.handle_dashboard_action(action);
    assert!(app.destroy_progress_modal.is_some());

    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        app.poll_destroy_progress();
        if let Some(modal) = &app.destroy_progress_modal {
            if modal.is_finished {
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    app.handle_destroy_progress_modal_key(key(KeyCode::Enter));
    assert!(app.destroy_progress_modal.is_none());
    assert!(!proj_del.exists());
    assert!(proj_keep.exists());
}

#[test]
fn test_integration_destroy_project_with_git_worktrees_cleans_central_repo() {
    let tmp = tempdir().expect("tempdir");
    let cwd = tmp.path().to_path_buf();

    // 1. Setup mock central repo
    let central_repo = cwd.join("repos").join("my_service");
    fs::create_dir_all(&central_repo).expect("create repo dir");

    let init = std::process::Command::new("git")
        .args(["init", "-b", "main"])
        .current_dir(&central_repo)
        .status()
        .or_else(|_| {
            std::process::Command::new("git")
                .arg("init")
                .current_dir(&central_repo)
                .status()
        })
        .expect("git init");
    assert!(init.success());

    let _ = std::process::Command::new("git")
        .args(["config", "user.name", "Tester"])
        .current_dir(&central_repo)
        .status();
    let _ = std::process::Command::new("git")
        .args(["config", "user.email", "tester@botai.dev"])
        .current_dir(&central_repo)
        .status();

    fs::write(central_repo.join("README.md"), "# Central Service\n").unwrap();
    let _ = std::process::Command::new("git")
        .args(["add", "README.md"])
        .current_dir(&central_repo)
        .status();
    let _ = std::process::Command::new("git")
        .args(["commit", "-m", "init"])
        .current_dir(&central_repo)
        .status();

    // 2. Setup project with worktree
    let project_name = "test-project-destroy";
    let project_dir = cwd.join(project_name);
    fs::create_dir_all(&project_dir).expect("create project dir");
    fs::write(project_dir.join("AGENTS.md"), "# Project").unwrap();

    let worktree_path = project_dir.join("my_service");
    botai::git::create_worktree(&central_repo, &worktree_path, project_name)
        .expect("create worktree");

    // Verify central repo tracks worktree
    let list_before = std::process::Command::new("git")
        .args(["worktree", "list", "--porcelain"])
        .current_dir(&central_repo)
        .output()
        .expect("worktree list");
    let str_before = String::from_utf8_lossy(&list_before.stdout);
    assert!(str_before.contains(project_name));

    // 3. Destroy project via TuiApp::destroy_project
    let registry = SessionRegistry::new();
    let mut app = TuiApp::new(registry).with_cwd(cwd.clone());

    app.destroy_project(project_name).expect("destroy_project");

    // 4. Verify project directory removed and central repo metadata cleaned up
    assert!(!project_dir.exists(), "Project directory must be removed");

    let list_after = std::process::Command::new("git")
        .args(["worktree", "list", "--porcelain"])
        .current_dir(&central_repo)
        .output()
        .expect("worktree list after");
    let str_after = String::from_utf8_lossy(&list_after.stdout);
    assert!(
        !str_after.contains(project_name),
        "Central repo must not list destroyed worktree"
    );

    let wt_meta = central_repo
        .join(".git")
        .join("worktrees")
        .join("my_service");
    assert!(
        !wt_meta.exists(),
        "Central repo worktree metadata must be pruned"
    );

    assert!(
        botai::git::branch_exists(&central_repo, project_name).expect("branch exists"),
        "Branch must be preserved in central repo history"
    );
}

#[test]
fn test_integration_async_project_setup_progress_streaming_and_success() {
    let tmp = tempdir().expect("tempdir");
    let cwd = tmp.path().to_path_buf();

    #[cfg(windows)]
    let shell_cmd = "cmd.exe";
    #[cfg(not(windows))]
    let shell_cmd = "sh";

    let config_toml = format!(
        r#"
default_agent = "mock_agent"
repos_dir = "{}/repos"

[agents.mock_agent]
command = "{}"
args = []

[templates.cloud_ws]
repos = []
"#,
        cwd.display().to_string().replace('\\', "/"),
        shell_cmd
    );
    let config = Config::load_from_str(&config_toml).unwrap();

    let registry = SessionRegistry::new();
    let dashboard = DashboardState::from_env(&config, &cwd);
    let mut app = TuiApp::new(registry)
        .with_dashboard(dashboard)
        .with_config(config)
        .with_cwd(cwd.clone());

    // Trigger project creation from dashboard action
    app.handle_dashboard_action(DashboardAction::CreateProject {
        template: "cloud_ws".to_string(),
        project: "microservice-alpha".to_string(),
        agent_override: Some("mock_agent".to_string()),
        prompt: Some("Initialize API endpoints".to_string()),
    });

    assert!(app.setup_modal.is_some());
    let initial_modal = app.setup_modal.as_ref().unwrap();
    assert_eq!(initial_modal.project_name, "microservice-alpha");
    assert_eq!(initial_modal.template_name, "cloud_ws");
    assert_eq!(
        initial_modal.prompt.as_deref(),
        Some("Initialize API endpoints")
    );

    // Poll until completion
    let start = std::time::Instant::now();
    while app.setup_modal.is_some() && start.elapsed() < Duration::from_secs(5) {
        app.poll_setup_progress();
        std::thread::sleep(Duration::from_millis(20));
    }

    assert!(app.setup_modal.is_none());
    assert_eq!(app.registry.len(), 1);
    let active = app.registry.active_session().unwrap();
    assert_eq!(active.project_name(), "microservice-alpha");
    assert_eq!(active.template_name(), "cloud_ws");
    assert_eq!(active.agent_name(), "mock_agent");
    assert_eq!(active.project_session.agent_name, "mock_agent");
    assert_eq!(
        active.project_session.prompt.as_deref(),
        Some("Initialize API endpoints")
    );

    let created_dir = cwd.join("microservice-alpha");
    assert!(created_dir.exists());
    assert!(created_dir.join("AGENTS.md").is_file());
}

#[test]
fn test_integration_async_project_setup_agent_override_applied() {
    let tmp = tempdir().expect("tempdir");
    let cwd = tmp.path().to_path_buf();

    #[cfg(windows)]
    let shell_cmd = "cmd.exe";
    #[cfg(not(windows))]
    let shell_cmd = "sh";

    let config_toml = format!(
        r#"
default_agent = "mock_default"
repos_dir = "{}/repos"

[agents.mock_default]
command = "{}"
args = []

[agents.mock_custom]
command = "{}"
args = []

[templates.cloud_ws]
repos = []
"#,
        cwd.display().to_string().replace('\\', "/"),
        shell_cmd,
        shell_cmd
    );
    let config = Config::load_from_str(&config_toml).unwrap();

    let registry = SessionRegistry::new();
    let dashboard = DashboardState::from_env(&config, &cwd);
    let mut app = TuiApp::new(registry)
        .with_dashboard(dashboard)
        .with_config(config)
        .with_cwd(cwd.clone());

    // Trigger project creation with explicit agent override
    app.handle_dashboard_action(DashboardAction::CreateProject {
        template: "cloud_ws".to_string(),
        project: "custom-agent-proj".to_string(),
        agent_override: Some("mock_custom".to_string()),
        prompt: None,
    });

    let start = std::time::Instant::now();
    while app.setup_modal.is_some() && start.elapsed() < Duration::from_secs(5) {
        app.poll_setup_progress();
        std::thread::sleep(Duration::from_millis(20));
    }

    assert!(app.setup_modal.is_none());
    assert_eq!(app.registry.len(), 1);
    let active = app.registry.active_session().unwrap();
    assert_eq!(active.project_name(), "custom-agent-proj");
    assert_eq!(active.agent_name(), "mock_custom");
    assert_eq!(active.project_session.agent_name, "mock_custom");
}

#[test]
fn test_integration_async_project_setup_error_retention_and_dismissal() {
    let tmp = tempdir().expect("tempdir");
    let cwd = tmp.path().to_path_buf();

    let config_toml = format!(
        r#"
default_agent = "mock"
repos_dir = "{}/repos"

[agents.mock]
command = "sh"
args = []

[templates.bad_ws]
repos = ["nonexistent-repo-that-does-not-exist"]
"#,
        cwd.display().to_string().replace('\\', "/"),
    );
    let config = Config::load_from_str(&config_toml).unwrap();

    let registry = SessionRegistry::new();
    let mut app = TuiApp::new(registry)
        .with_config(config)
        .with_cwd(cwd.clone());

    app.start_project_creation("bad_ws", "failing-proj", None, None);
    assert!(app.setup_modal.is_some());

    // Wait for worker to encounter error and stream failure event
    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        app.poll_setup_progress();
        if let Some(modal) = &app.setup_modal {
            if modal.is_finished {
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }

    assert!(app.setup_modal.is_some(), "Modal must remain open on error");
    let modal = app.setup_modal.as_ref().unwrap();
    assert!(modal.is_finished);
    assert!(modal.error.is_some());

    // Render the error modal
    let area = Rect::new(0, 0, 100, 30);
    let mut buf = Buffer::empty(area);
    let theme = Theme::default();
    render_setup_modal(modal, area, &mut buf, &theme);
    let text = buf
        .content()
        .iter()
        .map(ratatui::buffer::Cell::symbol)
        .collect::<String>();
    assert!(text.contains("Project Setup Failed: failing-proj"));
    assert!(text.contains("[Esc] / [Enter] Close"));

    // Pressing a character key should keep modal open
    app.handle_setup_modal_key(key(KeyCode::Char('x')));
    assert!(app.setup_modal.is_some());

    // Pressing Enter should dismiss the modal
    app.handle_setup_modal_key(key(KeyCode::Enter));
    assert!(app.setup_modal.is_none());

    // Failing project directory should not remain
    assert!(!cwd.join("failing-proj").exists());
}

#[test]
fn test_integration_async_project_destroy_progress_streaming_and_success() {
    use crossterm::event::{MouseEvent, MouseEventKind};

    let tmp = tempdir().expect("tempdir");
    let cwd = tmp.path().to_path_buf();

    // 1. Setup mock central repo
    let central_repo = cwd.join("repos").join("worker_service");
    fs::create_dir_all(&central_repo).expect("create repo dir");

    let init = std::process::Command::new("git")
        .args(["init", "-b", "main"])
        .current_dir(&central_repo)
        .status()
        .or_else(|_| {
            std::process::Command::new("git")
                .arg("init")
                .current_dir(&central_repo)
                .status()
        })
        .expect("git init");
    assert!(init.success());

    let _ = std::process::Command::new("git")
        .args(["config", "user.name", "Tester"])
        .current_dir(&central_repo)
        .status();
    let _ = std::process::Command::new("git")
        .args(["config", "user.email", "tester@botai.dev"])
        .current_dir(&central_repo)
        .status();

    fs::write(central_repo.join("README.md"), "# Central Worker\n").unwrap();
    let _ = std::process::Command::new("git")
        .args(["add", "README.md"])
        .current_dir(&central_repo)
        .status();
    let _ = std::process::Command::new("git")
        .args(["commit", "-m", "init"])
        .current_dir(&central_repo)
        .status();

    // 2. Setup project with worktree and pre_destroy marker
    let project_name = "async-del-service";
    let project_dir = cwd.join(project_name);
    fs::create_dir_all(&project_dir).expect("create project dir");

    let pre_destroy_marker = cwd.join("pre_destroy_ran.log");

    let config_toml = format!(
        r#"
default_agent = "mock"
repos_dir = "{}/repos"

[agents.mock]
command = "sh"
args = []

[templates.del_tpl]
repos = ["worker_service"]
pre_destroy = [
    ["sh", "-c", "echo executed_pre_destroy > {}"]
]
"#,
        cwd.display().to_string().replace('\\', "/"),
        pre_destroy_marker.display().to_string().replace('\\', "/")
    );
    let config = Config::load_from_str(&config_toml).unwrap();

    fs::write(
        project_dir.join("AGENTS.md"),
        "# Multi-Repo Project\n\nTemplate: **del_tpl**\n",
    )
    .unwrap();

    let worktree_path = project_dir.join("worker_service");
    botai::git::create_worktree(&central_repo, &worktree_path, project_name)
        .expect("create worktree");

    let registry = SessionRegistry::new();
    let dashboard = DashboardState::from_env(&config, &cwd);
    let mut app = TuiApp::new(registry)
        .with_dashboard(dashboard)
        .with_config(config)
        .with_cwd(cwd.clone());

    // Trigger destruction action
    app.handle_dashboard_action(DashboardAction::Destroy(project_name.to_string()));

    assert!(app.destroy_progress_modal.is_some());
    let initial_modal = app.destroy_progress_modal.as_ref().unwrap();
    assert_eq!(initial_modal.project_name, project_name);

    // Poll until completion
    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        app.poll_destroy_progress();
        if let Some(modal) = &app.destroy_progress_modal {
            if modal.is_finished {
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    assert!(
        app.destroy_progress_modal.is_some(),
        "Modal remains open upon completion"
    );
    let modal = app.destroy_progress_modal.as_ref().unwrap();
    assert!(modal.is_finished);
    assert!(modal.is_success);
    assert!(modal.error.is_none());
    assert_eq!(modal.current_step, 4);

    // Render the finished modal
    let area = Rect::new(0, 0, 100, 30);
    let mut buf = Buffer::empty(area);
    let theme = Theme::default();
    render_destroy_progress_modal(modal, area, &mut buf, &theme);
    let rendered = buf
        .content()
        .iter()
        .map(ratatui::buffer::Cell::symbol)
        .collect::<String>();
    assert!(rendered.contains("Project Destroyed: async-del-service"));
    assert!(rendered.contains("Destroyed Successfully"));
    assert!(rendered.contains("Destruction Logs"));
    assert!(rendered.contains("[Enter] / [Esc] Return to dashboard"));

    // Test mouse scrolling
    let scroll_up = MouseEvent {
        kind: MouseEventKind::ScrollUp,
        column: 10,
        row: 10,
        modifiers: KeyModifiers::empty(),
    };
    app.handle_destroy_progress_modal_mouse(scroll_up);
    assert_eq!(
        app.destroy_progress_modal.as_ref().unwrap().scroll_offset,
        3
    );

    // Test key scrolling
    app.handle_destroy_progress_modal_key(key(KeyCode::Up));
    assert_eq!(
        app.destroy_progress_modal.as_ref().unwrap().scroll_offset,
        4
    );
    app.handle_destroy_progress_modal_key(key(KeyCode::Down));
    assert_eq!(
        app.destroy_progress_modal.as_ref().unwrap().scroll_offset,
        3
    );

    // Non-dismiss key does not dismiss
    app.handle_destroy_progress_modal_key(key(KeyCode::Char('x')));
    assert!(app.destroy_progress_modal.is_some());

    // Enter dismisses modal and refreshes dashboard
    app.handle_destroy_progress_modal_key(key(KeyCode::Enter));
    assert!(app.destroy_progress_modal.is_none());

    // Verify project deleted on disk
    assert!(!project_dir.exists());
    assert!(pre_destroy_marker.exists());
    assert_eq!(
        fs::read_to_string(&pre_destroy_marker).unwrap().trim(),
        "executed_pre_destroy"
    );

    // Verify dashboard refreshed
    assert!(!app
        .dashboard
        .as_ref()
        .unwrap()
        .local_projects
        .contains(&project_name.to_string()));
}

#[test]
fn test_integration_async_project_destroy_error_retention_and_dismissal() {
    let tmp = tempdir().expect("tempdir");
    let cwd = tmp.path().to_path_buf();

    let registry = SessionRegistry::new();
    let mut app = TuiApp::new(registry).with_cwd(cwd.clone());

    // Start destruction of non-existent project
    app.start_project_destruction("nonexistent-dir");
    assert!(app.destroy_progress_modal.is_some());

    // Poll until failure event arrives
    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        app.poll_destroy_progress();
        if let Some(modal) = &app.destroy_progress_modal {
            if modal.is_finished {
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    let modal = app.destroy_progress_modal.as_ref().unwrap();
    assert!(modal.is_finished);
    assert!(!modal.is_success);
    assert!(modal.error.is_some());

    // Render error modal
    let area = Rect::new(0, 0, 100, 30);
    let mut buf = Buffer::empty(area);
    let theme = Theme::default();
    render_destroy_progress_modal(modal, area, &mut buf, &theme);
    let text = buf
        .content()
        .iter()
        .map(ratatui::buffer::Cell::symbol)
        .collect::<String>();
    assert!(text.contains("Project Destruction Failed: nonexistent-dir"));
    assert!(text.contains("Reason:"));
    assert!(text.contains("[Enter] / [Esc] Return to dashboard"));

    // Character key keeps modal open
    app.handle_destroy_progress_modal_key(key(KeyCode::Char('k')));
    assert!(app.destroy_progress_modal.is_some());

    // Esc dismisses modal
    app.handle_destroy_progress_modal_key(key(KeyCode::Esc));
    assert!(app.destroy_progress_modal.is_none());
}
