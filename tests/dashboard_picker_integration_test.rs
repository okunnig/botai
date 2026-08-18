use std::fs;

use botai::config::Config;
use botai::tui::{render_dashboard, ActivePanel, DashboardAction, DashboardState};
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

    // 2. Setup mock Config with multiple workspaces
    let config_toml = r#"
default_agent = "claude"
repos_dir = "/repos"

[agents.claude]
command = "claude"
args = []

[agents.agy]
command = "agy"
args = ["--verbose"]

[workspaces.ecommerce]
description = "E-commerce platform services"
default_agent = "claude"
repos = ["/repos/checkout", "/repos/cart"]

[workspaces.data_pipeline]
description = "Big data processing and ETL"
default_agent = "agy"
repos = ["/repos/spark-jobs", "/repos/kafka-streams"]
"#;

    let config = Config::load_from_str(config_toml).expect("Failed to load mock config");

    // 3. Instantiate DashboardState from environment
    let mut state = DashboardState::from_env(&config, cwd);

    // Verify scanned projects (sorted) and configured workspaces (sorted)
    assert_eq!(
        state.local_projects,
        vec![
            "payment-gateway".to_string(),
            "user-auth-service".to_string()
        ]
    );
    assert_eq!(
        state.workspaces,
        vec!["data_pipeline".to_string(), "ecommerce".to_string()]
    );
    assert_eq!(state.active_panel, ActivePanel::Projects);
    assert_eq!(state.selected_project_idx, 0);
    assert_eq!(state.selected_workspace_idx, 0);
    assert!(state.creation_modal.is_none());

    // 4. Test Ratatui TestBackend rendering of initial dashboard
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
        .map(|cell| cell.symbol())
        .collect::<String>();

    assert!(rendered_text.contains("Local Projects (Existing)"));
    assert!(rendered_text.contains("Configured Workspaces"));
    assert!(rendered_text.contains("payment-gateway"));
    assert!(rendered_text.contains("user-auth-service"));
    assert!(rendered_text.contains("data_pipeline"));
    assert!(rendered_text.contains("ecommerce"));
    assert!(rendered_text.contains("Tab: Switch | ↑/↓: Navigate"));

    // 5. Navigate to second project and attach
    state.handle_key(key(KeyCode::Down));
    assert_eq!(state.selected_project_idx, 1);

    let action = state.handle_key(key(KeyCode::Enter));
    assert_eq!(
        action,
        DashboardAction::Attach("user-auth-service".to_string())
    );

    // 6. Switch panel to Workspaces
    state.handle_key(key(KeyCode::Tab));
    assert_eq!(state.active_panel, ActivePanel::Workspaces);
    assert_eq!(state.selected_workspace_idx, 0); // "data_pipeline"

    // Navigate to "ecommerce" workspace
    state.handle_key(key(KeyCode::Down));
    assert_eq!(state.selected_workspace_idx, 1);

    // 7. Open creation modal with 'n' key
    let action = state.handle_key(key(KeyCode::Char('n')));
    assert_eq!(action, DashboardAction::None);
    assert!(state.creation_modal.is_some());

    let modal = state.creation_modal.as_ref().unwrap();
    assert_eq!(modal.workspace_name, "ecommerce");
    assert_eq!(modal.active_field, 0);

    // 8. Type project name into modal
    for c in "stripe-subscriptions".chars() {
        state.handle_key(key(KeyCode::Char(c)));
    }

    // Switch to prompt input field using Tab
    state.handle_key(key(KeyCode::Tab));
    assert_eq!(state.creation_modal.as_ref().unwrap().active_field, 1);

    // Type prompt into modal
    for c in "Build recurring billing engine with webhooks".chars() {
        state.handle_key(key(KeyCode::Char(c)));
    }

    // 9. Render modal in Ratatui TestBackend and verify visual layout
    terminal
        .draw(|f| {
            render_dashboard(&state, f.area(), f.buffer_mut());
        })
        .expect("Failed to draw frame with modal");

    let modal_rendered = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();

    assert!(modal_rendered.contains("Create New Project [ecommerce]"));
    assert!(modal_rendered.contains("Project Name:"));
    assert!(modal_rendered.contains("stripe-subscriptions"));
    assert!(modal_rendered.contains("Initial Prompt:"));
    assert!(modal_rendered.contains("Build recurring billing engine with webhooks"));

    // 10. Submit modal by pressing Enter
    let action = state.handle_key(key(KeyCode::Enter));
    assert_eq!(
        action,
        DashboardAction::CreateProject {
            workspace: "ecommerce".to_string(),
            project: "stripe-subscriptions".to_string(),
            prompt: Some("Build recurring billing engine with webhooks".to_string()),
        }
    );
    assert!(state.creation_modal.is_none());

    // 11. Test Destroy action on Projects panel
    state.handle_key(key(KeyCode::Tab)); // Switch back to Projects
    assert_eq!(state.active_panel, ActivePanel::Projects);
    state.selected_project_idx = 0; // "payment-gateway"

    let action = state.handle_key(key(KeyCode::Char('d')));
    assert_eq!(
        action,
        DashboardAction::Destroy("payment-gateway".to_string())
    );

    // 12. Test Quit action
    let action = state.handle_key(key(KeyCode::Char('q')));
    assert_eq!(action, DashboardAction::Quit);
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

[workspaces.test_ws]
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
