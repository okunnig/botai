use botai::config::Config;
use botai::session::{ProjectSession, PtyConfig, PtySession, SessionRegistry};
use botai::tui::{CreationFocus, CreationStep, TuiApp};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::backend::TestBackend;
use ratatui::Terminal;
use tempfile::tempdir;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn create_mock_git_repo(path: &std::path::Path) {
    std::fs::create_dir_all(path).unwrap();
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
    std::fs::write(path.join("README.md"), "# Test Repo\n").unwrap();
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
fn test_integration_in_session_creation_modal_lifecycle() {
    let tmp = tempdir().expect("tempdir");
    let cwd = tmp.path().to_path_buf();
    let repo_dir = tmp.path().join("repo_shared");
    create_mock_git_repo(&repo_dir);

    #[cfg(windows)]
    let (cmd, args) = ("cmd.exe", r#"["/c", "exit 0"]"#);
    #[cfg(not(windows))]
    let (cmd, args) = ("sh", r#"["-c", "exit 0"]"#);

    let repo_path_str = repo_dir.to_string_lossy().replace('\\', "/");
    let config_toml = format!(
        r#"
default_agent = "mock_agent"
repos_dir = "{}/repos"

[agents.mock_agent]
command = "{cmd}"
args = {args}

[templates.payments]
description = "Payment processing services"
repos = ['{repo_path_str}']

[templates.notifications]
description = "Notification delivery services"
repos = ['{repo_path_str}']
"#,
        cwd.display().to_string().replace('\\', "/")
    );
    let config = Config::load_from_str(&config_toml).expect("load config");

    // 1. Start an initial active session in the registry
    #[cfg(windows)]
    let (init_cmd, init_args) = ("cmd.exe", vec!["/c", "exit 0"]);
    #[cfg(not(windows))]
    let (init_cmd, init_args) = ("sh", vec!["-c", "true"]);

    let pty_config = PtyConfig::new(init_cmd, cwd.clone())
        .with_args(init_args)
        .with_dimensions(80, 24);
    let pty = PtySession::spawn(pty_config).expect("spawn initial pty");
    let vt = botai::session::VtScreen::new(80, 24);
    let ps = ProjectSession::new("initial-service", "notifications", "mock_agent");

    let mut registry = SessionRegistry::new();
    registry.add(ps, pty, vt).expect("add session to registry");
    registry.set_active_index(0).expect("set active session");

    let mut app = TuiApp::new(registry)
        .with_config(config)
        .with_cwd(cwd.clone());

    // Verify initial active session and that creation_modal is not open
    assert_eq!(app.registry.len(), 1);
    assert!(app.creation_modal.is_none());

    // 2. Open creation modal via Ctrl+Space (0x00) then 'c'
    app.handle_raw_bytes(&[0x00]);
    assert!(app.leader_active);
    app.handle_raw_bytes(b"c");
    assert!(!app.leader_active);
    assert!(app.creation_modal.is_some());

    let modal = app.creation_modal.as_ref().unwrap();
    assert_eq!(modal.step, CreationStep::SelectTemplate);
    assert_eq!(
        modal.templates,
        vec!["notifications".to_string(), "payments".to_string()]
    );

    // 3. Renders creation_modal with render_modal -> verify it contains Step 1 template list/preview
    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).expect("ratatui test terminal");

    terminal
        .draw(|f| {
            if let Some(m) = &app.creation_modal {
                let theme = app.config.as_ref().map(|c| c.theme()).unwrap_or_default();
                m.render_modal(f.area(), f.buffer_mut(), &theme);
            }
        })
        .expect("draw creation modal");

    let rendered = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(ratatui::buffer::Cell::symbol)
        .collect::<String>();

    assert!(rendered.contains("Create New Project"));
    assert!(rendered.contains("Templates"));
    assert!(rendered.contains("notifications"));
    assert!(rendered.contains("payments"));

    // 4. Advances to Step 2 (Enter)
    app.handle_creation_modal_key(key(KeyCode::Enter));
    assert!(app.creation_modal.is_some());
    assert_eq!(
        app.creation_modal.as_ref().unwrap().step,
        CreationStep::ConfigureDetails
    );

    // 5. Enters project name and prompt
    for c in "stripe-webhooks".chars() {
        app.handle_creation_modal_key(key(KeyCode::Char(c)));
    }
    assert_eq!(
        app.creation_modal.as_ref().unwrap().project_name_input,
        "stripe-webhooks"
    );

    // Tab to Agent
    app.handle_creation_modal_key(key(KeyCode::Tab));
    assert_eq!(
        app.creation_modal.as_ref().unwrap().focus,
        CreationFocus::Agent
    );

    // Tab to Prompt
    app.handle_creation_modal_key(key(KeyCode::Tab));
    assert_eq!(
        app.creation_modal.as_ref().unwrap().focus,
        CreationFocus::Prompt
    );

    for c in "Implement idempotency keys".chars() {
        app.handle_creation_modal_key(key(KeyCode::Char(c)));
    }
    assert_eq!(
        app.creation_modal.as_ref().unwrap().prompt_input,
        "Implement idempotency keys"
    );

    // 6. Submits (Enter) -> app.setup_modal.is_some()
    app.handle_creation_modal_key(key(KeyCode::Enter));
    assert!(app.creation_modal.is_none());
    assert!(app.setup_modal.is_some());

    // 7. Polls setup progress until complete
    let start = std::time::Instant::now();
    while app.setup_modal.is_some() && start.elapsed() < std::time::Duration::from_secs(5) {
        app.poll_setup_progress();
        std::thread::sleep(std::time::Duration::from_millis(20));
    }

    // 8. Session is created and active!
    assert!(app.setup_modal.is_none());
    assert_eq!(app.registry.len(), 2);
    let active = app.registry.active_session().unwrap();
    assert_eq!(active.project_name(), "stripe-webhooks");
    assert_eq!(active.template_name(), "notifications");
    assert!(cwd.join("stripe-webhooks").join("AGENTS.md").is_file());

    // 9. Tests dismiss via Esc:
    // From active session, open creation modal
    app.handle_raw_bytes(&[0x00]);
    assert!(app.leader_active);
    app.handle_raw_bytes(b"c");
    assert!(!app.leader_active);
    assert!(app.creation_modal.is_some());
    assert_eq!(
        app.creation_modal.as_ref().unwrap().step,
        CreationStep::SelectTemplate
    );

    // In Step 1, Esc dismisses the modal
    app.handle_creation_modal_key(key(KeyCode::Esc));
    assert!(app.creation_modal.is_none());

    // Open again, advance to Step 2, Esc returns to Step 1, then Esc dismisses
    app.handle_raw_bytes(&[0x00]);
    app.handle_raw_bytes(b"c");
    assert!(app.creation_modal.is_some());
    assert_eq!(
        app.creation_modal.as_ref().unwrap().step,
        CreationStep::SelectTemplate
    );

    app.handle_creation_modal_key(key(KeyCode::Enter));
    assert_eq!(
        app.creation_modal.as_ref().unwrap().step,
        CreationStep::ConfigureDetails
    );

    app.handle_creation_modal_key(key(KeyCode::Esc));
    assert!(app.creation_modal.is_some());
    assert_eq!(
        app.creation_modal.as_ref().unwrap().step,
        CreationStep::SelectTemplate
    );

    app.handle_creation_modal_key(key(KeyCode::Esc));
    assert!(app.creation_modal.is_none());
}
