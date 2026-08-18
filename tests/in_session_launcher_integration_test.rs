use std::fs;

use botai::config::Config;
use botai::session::SessionRegistry;
use botai::tui::{render_quick_launcher, LauncherFocus, LauncherTab, TuiApp};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::backend::TestBackend;
use ratatui::Terminal;
use tempfile::tempdir;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn ctrl_space_key() -> KeyEvent {
    KeyEvent::new(KeyCode::Char(' '), KeyModifiers::CONTROL)
}

#[test]
fn test_integration_in_session_quick_launcher_lifecycle() {
    let tmp = tempdir().expect("tempdir");
    let cwd = tmp.path().to_path_buf();

    #[cfg(windows)]
    let (cmd, args) = ("cmd.exe", r#"["/c", "exit 0"]"#);
    #[cfg(not(windows))]
    let (cmd, args) = ("sh", r#"["-c", "exit 0"]"#);

    let config_toml = format!(
        r#"
default_agent = "mock_agent"
repos_dir = "/tmp/repos"

[agents.mock_agent]
command = "{}"
args = {}

[workspaces.payments]
description = "Payment processing services"
repos = []

[workspaces.notifications]
description = "Notification delivery services"
repos = []
"#,
        cmd, args
    );
    let config = Config::load_from_str(&config_toml).expect("load config");

    // Setup an existing local project
    let existing_proj = cwd.join("legacy-service");
    fs::create_dir(&existing_proj).unwrap();
    fs::write(
        existing_proj.join("AGENTS.md"),
        "# Legacy Service\nOrientation context.",
    )
    .unwrap();

    let registry = SessionRegistry::new();
    let mut app = TuiApp::new(registry)
        .with_config(config)
        .with_cwd(cwd.clone());

    // 1. Initially no active sessions and no launcher
    assert!(app.registry.is_empty());
    assert!(app.quick_launcher.is_none());

    // 2. Open quick launcher via Ctrl+Space then 'c'
    app.handle_key_event(ctrl_space_key());
    assert!(app.leader_active);
    app.handle_key_event(key(KeyCode::Char('c')));
    assert!(!app.leader_active);
    assert!(app.quick_launcher.is_some());

    let launcher = app.quick_launcher.as_ref().unwrap();
    assert_eq!(
        launcher.workspaces,
        vec!["notifications".to_string(), "payments".to_string()]
    );
    assert_eq!(launcher.local_projects, vec!["legacy-service".to_string()]);
    assert_eq!(launcher.tab, LauncherTab::Create);
    assert_eq!(launcher.focus, LauncherFocus::Workspace);
    assert_eq!(launcher.selected_workspace(), Some("notifications"));

    // 3. Test Ratatui TestBackend rendering of launcher modal
    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).expect("ratatui test terminal");

    terminal
        .draw(|f| {
            if let Some(l) = &app.quick_launcher {
                let theme = app.config.as_ref().map(|c| c.theme()).unwrap_or_default();
                render_quick_launcher(l, f.area(), f.buffer_mut(), &theme);
            }
        })
        .expect("draw launcher");

    let rendered = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();

    assert!(rendered.contains("Quick Project Launcher (Ctrl+Space c)"));
    assert!(rendered.contains("1. Create New Project"));
    assert!(rendered.contains("2. Attach Existing Project"));
    assert!(rendered.contains("Workspace:"));
    assert!(rendered.contains("notifications"));

    // 4. Cycle workspace with Right arrow to "payments"
    app.handle_launcher_key(key(KeyCode::Right));
    assert_eq!(
        app.quick_launcher.as_ref().unwrap().selected_workspace(),
        Some("payments")
    );

    // 5. Navigate to Project Name input with Tab
    app.handle_launcher_key(key(KeyCode::Tab));
    assert_eq!(
        app.quick_launcher.as_ref().unwrap().focus,
        LauncherFocus::ProjectName
    );

    // 6. Type project name
    for c in "stripe-webhooks".chars() {
        app.handle_launcher_key(key(KeyCode::Char(c)));
    }
    assert_eq!(
        app.quick_launcher.as_ref().unwrap().project_name_input,
        "stripe-webhooks"
    );

    // 7. Navigate to Prompt input with Tab
    app.handle_launcher_key(key(KeyCode::Tab));
    assert_eq!(
        app.quick_launcher.as_ref().unwrap().focus,
        LauncherFocus::Prompt
    );

    // 8. Type prompt
    for c in "Implement idempotency keys".chars() {
        app.handle_launcher_key(key(KeyCode::Char(c)));
    }
    assert_eq!(
        app.quick_launcher.as_ref().unwrap().prompt_input,
        "Implement idempotency keys"
    );

    // 9. Submit creation with Enter
    app.handle_launcher_key(key(KeyCode::Enter));

    // Quick launcher closes and setup modal opens
    assert!(app.quick_launcher.is_none());
    assert!(app.setup_modal.is_some());

    let start = std::time::Instant::now();
    while app.setup_modal.is_some() && start.elapsed() < std::time::Duration::from_secs(5) {
        app.poll_setup_progress();
        std::thread::sleep(std::time::Duration::from_millis(20));
    }

    // Modal closes and session is active
    assert!(app.setup_modal.is_none());
    assert_eq!(app.registry.len(), 1);
    assert_eq!(
        app.registry.active_session().unwrap().project_name(),
        "stripe-webhooks"
    );
    assert_eq!(
        app.registry.active_session().unwrap().workspace_name(),
        "payments"
    );
    assert!(cwd.join("stripe-webhooks").join("AGENTS.md").is_file());


    // 10. Open launcher again via Ctrl+Space then 'C' from inside the active session
    app.handle_key_event(ctrl_space_key());
    assert!(app.leader_active);
    app.handle_key_event(make_key(KeyCode::Char('C'), KeyModifiers::SHIFT));
    assert!(!app.leader_active);
    assert!(app.quick_launcher.is_some());
    // Active workspace should default to "payments" (the active session's workspace)
    assert_eq!(
        app.quick_launcher.as_ref().unwrap().selected_workspace(),
        Some("payments")
    );

    // 11. Switch to TabBar and switch to Attach tab
    app.handle_launcher_key(make_key(KeyCode::BackTab, KeyModifiers::SHIFT)); // from Workspace -> TabBar
    assert_eq!(
        app.quick_launcher.as_ref().unwrap().focus,
        LauncherFocus::TabBar
    );

    app.handle_launcher_key(key(KeyCode::Right));
    assert_eq!(
        app.quick_launcher.as_ref().unwrap().tab,
        LauncherTab::Attach
    );

    // 12. Move down into project list and attach "legacy-service"
    app.handle_launcher_key(key(KeyCode::Down));
    assert_eq!(
        app.quick_launcher.as_ref().unwrap().focus,
        LauncherFocus::ProjectList
    );

    app.handle_launcher_key(key(KeyCode::Enter));

    // Modal closes, legacy-service is attached as session 2 and focused
    assert!(app.quick_launcher.is_none());
    assert_eq!(app.registry.len(), 2);
    assert_eq!(
        app.registry.active_session().unwrap().project_name(),
        "legacy-service"
    );
    assert_eq!(app.registry.active_index(), Some(1));

    // 13. Open launcher and attach "stripe-webhooks" (which is already active as session 0)
    app.handle_key_event(ctrl_space_key());
    app.handle_key_event(key(KeyCode::Char('c')));
    let mut launcher = app.quick_launcher.take().unwrap();
    launcher.switch_tab(LauncherTab::Attach);
    // Find index of stripe-webhooks in local_projects
    if let Some(pos) = launcher
        .local_projects
        .iter()
        .position(|p| p == "stripe-webhooks")
    {
        launcher.selected_project_idx = pos;
    }
    app.quick_launcher = Some(launcher);

    app.handle_launcher_key(key(KeyCode::Enter));

    // Modal closes and active session switches back to session 0 ("stripe-webhooks")
    assert!(app.quick_launcher.is_none());
    assert_eq!(app.registry.active_index(), Some(0));
    assert_eq!(
        app.registry.active_session().unwrap().project_name(),
        "stripe-webhooks"
    );

    // 14. Open launcher and test Dismiss / Cancel via Esc
    app.handle_key_event(ctrl_space_key());
    app.handle_key_event(key(KeyCode::Char('c')));
    assert!(app.quick_launcher.is_some());
    app.handle_launcher_key(key(KeyCode::Esc));
    assert!(app.quick_launcher.is_none());
}

fn make_key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
    KeyEvent::new(code, modifiers)
}
