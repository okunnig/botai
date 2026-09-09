use std::fs;

use botai::config::Config;
use botai::tui::{render_dashboard, CreationFocus, DashboardAction, DashboardState};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::backend::TestBackend;
use ratatui::Terminal;
use tempfile::tempdir;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

#[test]
fn test_integration_dashboard_lifecycle_from_env_to_action() {
    let tmp = tempdir().expect("Failed to create tempdir");
    let cwd = tmp.path();

    // 1. Setup mock existing local project directories with AGENTS.md files
    let proj1 = cwd.join("payment-gateway");
    fs::create_dir(&proj1).expect("Failed to create proj1 dir");
    fs::write(
        proj1.join("AGENTS.md"),
        "# Payment Gateway\nMulti-repo orientation context for payment services.",
    )
    .expect("Failed to write AGENTS.md for proj1");

    let proj2 = cwd.join("user-auth-service");
    fs::create_dir(&proj2).expect("Failed to create proj2 dir");
    fs::write(
        proj2.join("AGENTS.md"),
        "# User Auth Service\nMulti-repo orientation context for auth.",
    )
    .expect("Failed to write AGENTS.md for proj2");

    // Also create a non-project directory that should be ignored by the dashboard scanner
    let random_dir = cwd.join("node_modules");
    fs::create_dir(&random_dir).expect("Failed to create random dir");
    fs::write(
        random_dir.join("package.json"),
        r#"{"name": "some-dependency"}"#,
    )
    .expect("Failed to write package.json");

    // 2. Setup mock Config with multiple templates
    let config_toml = r#"
default_agent = "claude"
repos_dir = "/repos"

[agents.claude]
command = "claude"
args = []

[agents.agy]
command = "agy"
args = ["--verbose"]

[templates.ecommerce]
description = "E-commerce platform services"
default_agent = "claude"
repos = ["/repos/checkout", "/repos/cart"]

[templates.data_pipeline]
description = "Big data processing and ETL"
default_agent = "agy"
repos = ["/repos/spark-jobs", "/repos/kafka-streams"]
"#;

    let config = Config::load_from_str(config_toml).expect("Failed to load mock config");

    // 3. Instantiate DashboardState from environment
    let mut state = DashboardState::from_env(&config, cwd);

    // Verify scanned projects (sorted) and configured templates (sorted)
    assert_eq!(
        state.local_projects,
        vec![
            "payment-gateway".to_string(),
            "user-auth-service".to_string()
        ]
    );
    assert_eq!(
        state.templates,
        vec!["data_pipeline".to_string(), "ecommerce".to_string()]
    );
    assert_eq!(state.selected_project_idx, 0);
    assert!(state.creation_modal.is_none());

    // 4. Test Ratatui TestBackend rendering of full-width dashboard
    let backend = TestBackend::new(120, 30);
    let mut terminal = Terminal::new(backend).expect("Failed to create ratatui test terminal");

    terminal
        .draw(|f| {
            f.render_widget(&state, f.area());
        })
        .expect("Failed to draw frame");

    let buffer = terminal.backend().buffer();
    let rendered_text = buffer
        .content()
        .iter()
        .map(ratatui::buffer::Cell::symbol)
        .collect::<String>();

    assert!(rendered_text.contains("Local Projects"));
    assert!(!rendered_text.contains("Project Templates"));
    assert!(rendered_text.contains("payment-gateway"));
    assert!(rendered_text.contains("user-auth-service"));
    assert!(rendered_text
        .contains("↑/↓: Navigate | Enter: Attach | n/c: New Project | d: Destroy | q: Quit"));

    // 5. Navigate to second project and attach
    state.handle_key(key(KeyCode::Down));
    assert_eq!(state.selected_project_idx, 1);

    let action = state.handle_key(key(KeyCode::Enter));
    assert_eq!(
        action,
        DashboardAction::Attach("user-auth-service".to_string())
    );

    // 6. Open unified creation modal with 'n' shortcut key
    let action = state.handle_key(key(KeyCode::Char('n')));
    assert_eq!(action, DashboardAction::None);
    assert!(state.creation_modal.is_some());

    // 7. Step 1: Select template in creation modal
    assert_eq!(
        state.creation_modal.as_ref().unwrap().step,
        botai::tui::CreationStep::SelectTemplate
    );
    assert_eq!(
        state.creation_modal.as_ref().unwrap().selected_template(),
        Some("data_pipeline")
    );

    // Navigate to "ecommerce" template
    state.handle_key(key(KeyCode::Down));
    assert_eq!(
        state.creation_modal.as_ref().unwrap().selected_template(),
        Some("ecommerce")
    );

    // Press Enter to advance to Step 2 (ConfigureDetails)
    state.handle_key(key(KeyCode::Enter));
    assert_eq!(
        state.creation_modal.as_ref().unwrap().step,
        botai::tui::CreationStep::ConfigureDetails
    );
    assert_eq!(
        state.creation_modal.as_ref().unwrap().focus,
        CreationFocus::ProjectName
    );

    // Type project name into modal
    for c in "stripe-subscriptions".chars() {
        state.handle_key(key(KeyCode::Char(c)));
    }

    // Switch to agent and then prompt input field using Tab
    state.handle_key(key(KeyCode::Tab));
    state.handle_key(key(KeyCode::Tab));
    assert_eq!(
        state.creation_modal.as_ref().unwrap().focus,
        CreationFocus::Prompt
    );

    // Type prompt into modal
    for c in "Build recurring billing engine with webhooks".chars() {
        state.handle_key(key(KeyCode::Char(c)));
    }

    // 8. Render modal in Ratatui TestBackend and verify visual layout
    let theme = config.theme();
    terminal
        .draw(|f| {
            render_dashboard(&state, f.area(), f.buffer_mut(), &theme);
        })
        .expect("Failed to draw frame with modal");

    let modal_rendered = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(ratatui::buffer::Cell::symbol)
        .collect::<String>();

    assert!(modal_rendered.contains("Create New Project"));
    assert!(modal_rendered.contains("ecommerce"));
    assert!(modal_rendered.contains("Project Name:"));
    assert!(modal_rendered.contains("stripe-subscriptions"));
    assert!(modal_rendered.contains("Initial Prompt:"));
    assert!(modal_rendered.contains("Build recurring billing engine with webhooks"));

    // 9. Submit modal by pressing Enter
    let action = state.handle_key(key(KeyCode::Enter));
    assert_eq!(
        action,
        DashboardAction::CreateProject {
            template: "ecommerce".to_string(),
            project: "stripe-subscriptions".to_string(),
            agent_override: Some("claude".to_string()),
            prompt: Some("Build recurring billing engine with webhooks".to_string()),
        }
    );
    assert!(state.creation_modal.is_none());

    // 10. Test Destroy confirmation modal action on selected project
    state.selected_project_idx = 0; // "payment-gateway"

    let action = state.handle_key(key(KeyCode::Char('d')));
    assert_eq!(action, DashboardAction::None);
    assert_eq!(
        state.destroy_modal,
        Some(botai::tui::DestroyModalState::new("payment-gateway"))
    );

    let action = state.handle_key(key(KeyCode::Char('y')));
    assert_eq!(
        action,
        DashboardAction::Destroy("payment-gateway".to_string())
    );
    assert!(state.destroy_modal.is_none());

    // 11. Test Quit action
    let action = state.handle_key(key(KeyCode::Char('q')));
    assert_eq!(action, DashboardAction::Quit);
}

#[test]
fn test_integration_dashboard_empty_state_rendering() {
    let tmp = tempdir().expect("Failed to create tempdir");
    let cwd = tmp.path();

    let config =
        Config::load_from_str("default_agent = \"claude\"\nrepos_dir = \"/repos\"\n").unwrap();
    let state = DashboardState::from_env(&config, cwd);

    let backend = TestBackend::new(100, 20);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal
        .draw(|f| {
            f.render_widget(&state, f.area());
        })
        .unwrap();

    let text = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(ratatui::buffer::Cell::symbol)
        .collect::<String>();

    assert!(text.contains("Local Projects"));
    assert!(
        text.contains("No local projects found in CWD. Press [n] or [c] to create a new project.")
    );
}

#[test]
fn test_integration_tui_app_with_dashboard_actions() {
    let tmp = tempdir().expect("Failed to create tempdir");
    let cwd = tmp.path();

    // Create a mock project
    let proj1 = cwd.join("mock-service");
    fs::create_dir(&proj1).expect("create proj dir");
    fs::write(proj1.join("AGENTS.md"), "# Mock Service").expect("write agents md");

    let config_toml = r#"
default_agent = "mock_shell"
repos_dir = "/tmp/repos"

[agents.mock_shell]
command = "sh"
args = ["-c", "exit 0"]

[templates.test_ws]
repos = []
"#;
    let config = Config::load_from_str(config_toml).expect("load config");
    let dashboard = DashboardState::from_env(&config, cwd);

    let registry = botai::session::SessionRegistry::new();
    let mut app = botai::tui::TuiApp::new(registry)
        .with_dashboard(dashboard)
        .with_config(config)
        .with_cwd(cwd.to_path_buf());

    assert!(app.registry.is_empty());
    assert!(app.dashboard.is_some());
    assert!(!app.should_quit);

    // 1. Trigger Attach action
    app.handle_dashboard_action(DashboardAction::Attach("mock-service".to_string()));
    assert_eq!(app.registry.len(), 1);
    assert_eq!(
        app.registry.active_session().unwrap().project_name(),
        "mock-service"
    );

    // 2. Trigger Destroy action on another directory
    let to_destroy = cwd.join("destroy-me");
    fs::create_dir(&to_destroy).expect("create destroy-me");
    fs::write(to_destroy.join("AGENTS.md"), "# Destroy me").expect("write file");
    app.refresh_dashboard();
    assert!(app
        .dashboard
        .as_ref()
        .unwrap()
        .local_projects
        .contains(&"destroy-me".to_string()));

    app.handle_dashboard_action(DashboardAction::Destroy("destroy-me".to_string()));
    assert!(app.destroy_progress_modal.is_some());
    let start = std::time::Instant::now();
    while start.elapsed() < std::time::Duration::from_secs(5) {
        app.poll_destroy_progress();
        if let Some(modal) = &app.destroy_progress_modal {
            if modal.is_finished {
                break;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    app.handle_destroy_progress_modal_key(key(KeyCode::Enter));
    assert!(app.destroy_progress_modal.is_none());
    assert!(!to_destroy.exists());
    assert!(!app
        .dashboard
        .as_ref()
        .unwrap()
        .local_projects
        .contains(&"destroy-me".to_string()));

    // 3. Trigger Quit action
    app.handle_dashboard_action(DashboardAction::Quit);
    assert!(app.should_quit);
}

#[test]
fn test_integration_detach_returns_to_dashboard_and_reattach() {
    let tmp = tempdir().expect("Failed to create tempdir");
    let cwd = tmp.path();

    let proj1 = cwd.join("service-a");
    fs::create_dir(&proj1).expect("create proj dir");
    fs::write(proj1.join("AGENTS.md"), "# Service A").expect("write agents md");

    let config_toml = r#"
default_agent = "mock_shell"
repos_dir = "/tmp/repos"

[agents.mock_shell]
command = "sh"
args = ["-c", "sleep 10"]

[templates.test_ws]
repos = []
"#;
    let config = Config::load_from_str(config_toml).expect("load config");
    let dashboard = DashboardState::from_env(&config, cwd);

    let registry = botai::session::SessionRegistry::new();
    let mut app = botai::tui::TuiApp::new(registry)
        .with_dashboard(dashboard)
        .with_config(config)
        .with_cwd(cwd.to_path_buf());

    // 1. Initially showing dashboard
    assert!(app.is_showing_dashboard());

    // 2. Attach project service-a
    app.handle_dashboard_action(DashboardAction::Attach("service-a".to_string()));
    assert_eq!(app.registry.len(), 1);
    assert!(!app.showing_dashboard);
    assert!(!app.is_showing_dashboard());

    // 3. Send leader Ctrl+Space then Detach ('d')
    app.handle_raw_bytes(&[0x00]);
    assert!(app.leader_active);
    app.handle_raw_bytes(b"d");

    // Should return to dashboard without quitting, keeping session running
    assert!(!app.leader_active);
    assert!(!app.should_quit);
    assert!(app.showing_dashboard);
    assert!(app.is_showing_dashboard());
    assert_eq!(app.registry.len(), 1);

    // 4. Re-attach to service-a from dashboard
    app.handle_dashboard_action(DashboardAction::Attach("service-a".to_string()));
    assert!(!app.showing_dashboard);
    assert!(!app.is_showing_dashboard());
    assert_eq!(app.registry.active_index(), Some(0));
    assert_eq!(app.registry.len(), 1);
}

#[test]
fn test_integration_dashboard_running_session_indicator_lifecycle() {
    let tmp = tempdir().expect("tempdir");
    let cwd = tmp.path();

    let proj1 = cwd.join("svc-running");
    fs::create_dir(&proj1).expect("create proj1");
    fs::write(proj1.join("AGENTS.md"), "# Running Service").expect("write agents md");

    let proj2 = cwd.join("svc-stopped");
    fs::create_dir(&proj2).expect("create proj2");
    fs::write(proj2.join("AGENTS.md"), "# Stopped Service").expect("write agents md");

    let config_toml = r#"
default_agent = "mock_shell"
repos_dir = "/tmp/repos"

[agents.mock_shell]
command = "sh"
args = ["-c", "sleep 10"]

[templates.test_ws]
repos = []
"#;
    let config = Config::load_from_str(config_toml).expect("load config");
    let dashboard = DashboardState::from_env(&config, cwd);

    let registry = botai::session::SessionRegistry::new();
    let mut app = botai::tui::TuiApp::new(registry)
        .with_dashboard(dashboard)
        .with_config(config)
        .with_cwd(cwd.to_path_buf());

    // 1. Attach to svc-running
    app.handle_dashboard_action(DashboardAction::Attach("svc-running".to_string()));
    assert_eq!(app.registry.len(), 1);

    // 2. Detach back to dashboard
    app.handle_raw_bytes(&[0x00]);
    app.handle_raw_bytes(b"d");

    assert!(app.is_showing_dashboard());

    // 3. Render dashboard with app's synchronized running sessions
    let dashboard = app.dashboard.as_ref().unwrap();
    assert_eq!(
        dashboard.running_sessions.get("svc-running"),
        Some(&botai::session::SessionStatus::Initializing)
    );
    assert_eq!(dashboard.running_sessions.get("svc-stopped"), None);

    let backend = TestBackend::new(120, 30);
    let mut terminal = Terminal::new(backend).unwrap();
    let theme = botai::tui::Theme::tokyo_night();

    terminal
        .draw(|f| {
            render_dashboard(dashboard, f.area(), f.buffer_mut(), &theme);
        })
        .unwrap();

    let buffer = terminal.backend().buffer();
    let rendered = buffer
        .content()
        .iter()
        .map(ratatui::buffer::Cell::symbol)
        .collect::<String>();

    assert!(rendered.contains("svc-running"));
    assert!(rendered.contains("svc-stopped"));
    assert!(rendered.contains("●"));
}
