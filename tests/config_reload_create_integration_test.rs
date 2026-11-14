use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::fs;
use std::time::Duration;
use tempfile::tempdir;

use botai::config::Config;
use botai::session::{ProjectSession, PtyConfig, PtySession, SessionRegistry};
use botai::tui::creation_modal::CreationStep;
use botai::tui::dashboard::DashboardState;
use botai::tui::TuiApp;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

#[test]
fn test_integration_add_new_template_and_create_project_without_restarting_botai() {
    let tmp = tempdir().expect("tempdir");
    let cwd = tmp.path().to_path_buf();
    let config_file = tmp.path().join("config.toml");

    #[cfg(windows)]
    let shell_cmd = "cmd.exe";
    #[cfg(not(windows))]
    let shell_cmd = "sh";

    // 1. Initial configuration with only one template (initial_service)
    let initial_toml = format!(
        r#"
default_agent = "mock_agent"
repos_dir = "{}/repos"

[agents.mock_agent]
command = "{}"
args = []

[templates.initial_service]
description = "Initial Service"
repos = []
"#,
        cwd.display().to_string().replace('\\', "/"),
        shell_cmd
    );
    fs::write(&config_file, initial_toml).expect("write initial config");

    let config = Config::load_from_file(&config_file).expect("load config");
    let dashboard = DashboardState::from_env(&config, &cwd).with_config_path(&config_file);
    let registry = SessionRegistry::new();
    let mut app = TuiApp::new(registry)
        .with_dashboard(dashboard)
        .with_config(config)
        .with_config_path(&config_file)
        .with_cwd(cwd.clone());

    // Verify initial state: 1 template
    assert_eq!(
        app.dashboard.as_ref().unwrap().templates,
        vec!["initial_service".to_string()]
    );

    // Initially calling reload_config() when file has NOT been modified returns Ok(false)
    assert!(matches!(app.reload_config(), Ok(false)));

    // 2. User modifies config.toml on disk while botai is running to add a new template
    std::thread::sleep(Duration::from_millis(20));
    let updated_toml = format!(
        r#"
default_agent = "mock_agent"
repos_dir = "{}/repos"

[agents.mock_agent]
command = "{}"
args = []

[templates.initial_service]
description = "Initial Service"
repos = []

[templates.newly_added_service]
description = "Newly Added Service"
repos = []
"#,
        cwd.display().to_string().replace('\\', "/"),
        shell_cmd
    );
    fs::write(&config_file, updated_toml).expect("write updated config");

    // Verify that modifying the file causes reload_config() to return Ok(true)
    assert!(matches!(app.reload_config(), Ok(true)));
    // Verify that calling reload_config() when file has NOT been modified returns Ok(false)
    assert!(matches!(app.reload_config(), Ok(false)));

    // 3. User presses 'n' to open creation modal from dashboard
    app.handle_dashboard_key(key(KeyCode::Char('n')));

    // Verify modal is open and has reloaded configuration containing newly_added_service
    let dashboard = app.dashboard.as_ref().unwrap();
    let modal = dashboard
        .creation_modal
        .as_ref()
        .expect("creation modal must be open");

    assert_eq!(
        modal.templates,
        vec![
            "initial_service".to_string(),
            "newly_added_service".to_string()
        ]
    );
    assert_eq!(
        dashboard.templates,
        vec![
            "initial_service".to_string(),
            "newly_added_service".to_string()
        ]
    );

    // Also verify app.config is synced
    let app_cfg = &app.config;
    assert!(app_cfg.templates.contains_key("newly_added_service"));

    // 4. Select the newly added template (it is at index 1: newly_added_service)
    app.handle_dashboard_key(key(KeyCode::Down));

    // Press Enter to proceed to Step 2 (ConfigureDetails)
    app.handle_dashboard_key(key(KeyCode::Enter));

    let modal = app
        .dashboard
        .as_ref()
        .unwrap()
        .creation_modal
        .as_ref()
        .unwrap();
    assert_eq!(modal.step, CreationStep::ConfigureDetails);
    assert_eq!(modal.selected_template(), Some("newly_added_service"));

    // 5. Type project name "my-new-service"
    for c in "my-new-service".chars() {
        app.handle_dashboard_key(key(KeyCode::Char(c)));
    }

    // 6. Press Enter to submit and create project
    app.handle_dashboard_key(key(KeyCode::Enter));

    // Dashboard creation modal should be closed and setup_modal activated on app
    assert!(app.dashboard.as_ref().unwrap().creation_modal.is_none());
    assert!(app.setup_modal.is_some());
    assert_eq!(
        app.setup_modal.as_ref().unwrap().template_name,
        "newly_added_service"
    );
    assert_eq!(
        app.setup_modal.as_ref().unwrap().project_name,
        "my-new-service"
    );

    // 7. Poll setup modal until completion
    let start = std::time::Instant::now();
    while app.setup_modal.is_some() && start.elapsed() < Duration::from_secs(10) {
        app.poll_setup_progress();
        std::thread::sleep(Duration::from_millis(20));
    }

    assert!(
        app.setup_modal.is_none(),
        "Setup modal should complete successfully"
    );
    assert_eq!(app.registry.len(), 1);

    let session = app.registry.active_session().expect("active session");
    assert_eq!(session.project_name(), "my-new-service");
    assert_eq!(session.template_name(), "newly_added_service");

    // Verify project directory exists on disk and AGENTS.md was created
    let project_dir = cwd.join("my-new-service");
    assert!(project_dir.exists());
    assert!(project_dir.join("AGENTS.md").is_file());
    let agents_md = fs::read_to_string(project_dir.join("AGENTS.md")).unwrap();
    assert!(agents_md.contains("newly_added_service"));
}

#[test]
fn test_integration_in_session_launcher_picks_up_new_template() {
    let tmp = tempdir().expect("tempdir");
    let cwd = tmp.path().to_path_buf();
    let config_file = tmp.path().join("config.toml");

    #[cfg(windows)]
    let shell_cmd = "cmd.exe";
    #[cfg(not(windows))]
    let shell_cmd = "sh";

    let initial_toml = format!(
        r#"
default_agent = "mock_agent"
repos_dir = "{}/repos"

[agents.mock_agent]
command = "{}"
args = []

[templates.base_tpl]
repos = []
"#,
        cwd.display().to_string().replace('\\', "/"),
        shell_cmd
    );
    fs::write(&config_file, initial_toml).unwrap();

    let config = Config::load_from_file(&config_file).unwrap();

    // Spawn an initial active session in registry
    #[cfg(windows)]
    let (init_cmd, init_args) = ("cmd.exe", vec!["/c", "exit 0"]);
    #[cfg(not(windows))]
    let (init_cmd, init_args) = ("sh", vec!["-c", "true"]);

    let pty_config = PtyConfig::new(init_cmd, cwd.clone())
        .with_args(init_args)
        .with_dimensions(80, 24);
    let pty = PtySession::spawn(pty_config).expect("spawn initial pty");
    let vt = botai::session::VtScreen::new(80, 24);
    let ps = ProjectSession::new("active-session", "base_tpl", "mock_agent");

    let mut registry = SessionRegistry::new();
    registry.add(ps, pty, vt).expect("add session");
    registry.set_active_index(0).expect("set active");

    let mut app = TuiApp::new(registry)
        .with_config(config)
        .with_config_path(&config_file)
        .with_cwd(cwd.clone());

    // Initially unmodified returns Ok(false)
    assert!(matches!(app.reload_config(), Ok(false)));

    // Update config on disk while inside the session
    std::thread::sleep(Duration::from_millis(20));
    let updated_toml = format!(
        r#"
default_agent = "mock_agent"
repos_dir = "{}/repos"

[agents.mock_agent]
command = "{}"
args = []

[templates.base_tpl]
repos = []

[templates.session_added_tpl]
repos = []
"#,
        cwd.display().to_string().replace('\\', "/"),
        shell_cmd
    );
    fs::write(&config_file, updated_toml).unwrap();

    // Trigger in-session launcher (internally calls reload_config)
    app.open_creation_modal();

    let modal = app.creation_modal.as_ref().expect("modal should be opened");
    assert_eq!(
        modal.templates,
        vec!["base_tpl".to_string(), "session_added_tpl".to_string()]
    );

    // Calling reload_config() again without disk modification returns Ok(false)
    assert!(matches!(app.reload_config(), Ok(false)));
}

#[test]
fn test_integration_reload_config_gracefully_handles_invalid_toml() {
    let tmp = tempdir().expect("tempdir");
    let cwd = tmp.path().to_path_buf();
    let config_file = tmp.path().join("config.toml");

    let valid_toml = r#"
default_agent = "claude"
repos_dir = "/tmp/repos"

[templates.stable_tpl]
repos = []
"#;
    fs::write(&config_file, valid_toml).unwrap();

    let config = Config::load_from_file(&config_file).unwrap();
    let dashboard = DashboardState::from_env(&config, &cwd).with_config_path(&config_file);
    let registry = SessionRegistry::new();
    let mut app = TuiApp::new(registry)
        .with_dashboard(dashboard)
        .with_config(config)
        .with_config_path(&config_file)
        .with_cwd(cwd);

    assert!(matches!(app.reload_config(), Ok(false)));

    // Corrupt config file
    fs::write(&config_file, "default_agent = [unclosed syntax").unwrap();

    // Press 'n' to open creation modal
    app.handle_dashboard_key(key(KeyCode::Char('n')));

    // Should not crash and should still have stable_tpl
    let modal = app
        .dashboard
        .as_ref()
        .unwrap()
        .creation_modal
        .as_ref()
        .expect("modal should still open");
    assert_eq!(modal.templates, vec!["stable_tpl".to_string()]);
}

#[test]
fn test_integration_modify_existing_template_reloads_on_project_creation() {
    let tmp = tempdir().expect("tempdir");
    let cwd = tmp.path().to_path_buf();
    let config_file = tmp.path().join("config.toml");

    let v1_toml = r#"
default_agent = "claude"
repos_dir = "/tmp/repos"

[templates.my_tpl]
description = "Version 1"
repos = []
"#;
    fs::write(&config_file, v1_toml).unwrap();

    let config = Config::load_from_file(&config_file).unwrap();
    let mut dashboard = DashboardState::from_env(&config, &cwd).with_config_path(&config_file);

    assert_eq!(
        dashboard
            .config
            .get_template("my_tpl")
            .unwrap()
            .description
            .as_deref(),
        Some("Version 1")
    );

    // Initial check: calling reload_config when file has NOT been modified returns Ok(false)
    assert!(matches!(dashboard.reload_config(), Ok(false)));

    // Modify existing template description on disk
    std::thread::sleep(Duration::from_millis(20));
    let v2_toml = r#"
default_agent = "claude"
repos_dir = "/tmp/repos"

[templates.my_tpl]
description = "Version 2 Updated"
repos = []
"#;
    fs::write(&config_file, v2_toml).unwrap();

    // Verify modifying file causes reload_config() to return Ok(true)
    assert!(matches!(dashboard.reload_config(), Ok(true)));
    // Verify calling reload_config() without modification returns Ok(false)
    assert!(matches!(dashboard.reload_config(), Ok(false)));

    // Open creation modal
    dashboard.open_creation_modal();

    let modal = dashboard.creation_modal.as_ref().unwrap();
    assert_eq!(
        modal
            .config
            .get_template("my_tpl")
            .unwrap()
            .description
            .as_deref(),
        Some("Version 2 Updated")
    );
}

#[test]
fn test_integration_reload_config_mtime_skips_when_unchanged_and_reloads_when_modified() {
    let tmp = tempdir().expect("tempdir");
    let cwd = tmp.path().to_path_buf();
    let config_file = tmp.path().join("config.toml");

    let toml_v1 = r#"
default_agent = "claude"
repos_dir = "/tmp/repos"

[templates.t1]
repos = []
"#;
    fs::write(&config_file, toml_v1).unwrap();

    let config = Config::load_from_file(&config_file).unwrap();
    let mut dashboard = DashboardState::from_env(&config, &cwd).with_config_path(&config_file);
    let mut app = TuiApp::new(SessionRegistry::new())
        .with_config(config)
        .with_config_path(&config_file)
        .with_cwd(cwd);

    // 1. Initially unmodified - both return Ok(false)
    assert!(matches!(dashboard.reload_config(), Ok(false)));
    assert!(matches!(app.reload_config(), Ok(false)));

    // 2. Modify config file
    std::thread::sleep(Duration::from_millis(20));
    let toml_v2 = r#"
default_agent = "claude"
repos_dir = "/tmp/repos"

[templates.t1]
repos = []

[templates.t2]
repos = []
"#;
    fs::write(&config_file, toml_v2).unwrap();

    // 3. Reload config on dashboard - returns Ok(true)
    assert!(matches!(dashboard.reload_config(), Ok(true)));
    assert_eq!(
        dashboard.templates,
        vec!["t1".to_string(), "t2".to_string()]
    );
    // Calling reload again without disk modification returns Ok(false)
    assert!(matches!(dashboard.reload_config(), Ok(false)));

    // 4. Reload config on app - returns Ok(true)
    assert!(matches!(app.reload_config(), Ok(true)));
    assert!(app.config.templates.contains_key("t2"));
    // Calling reload again without disk modification returns Ok(false)
    assert!(matches!(app.reload_config(), Ok(false)));
}
