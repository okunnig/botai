use std::time::Duration;
use tempfile::tempdir;

use botai::session::{
    ProjectSession, PtyConfig, PtySession, SessionRegistry, SessionStatus, VtScreen,
};
use botai::tui::app::render_error_banner;
use botai::tui::dashboard::DashboardState;
use botai::tui::ui::compute_layout;
use botai::tui::ui::TerminalView;
use botai::tui::TuiApp;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::backend::TestBackend;
use ratatui::Terminal;

fn create_process_session(
    name: &str,
    ws: &str,
    agent: &str,
    script: &str,
) -> (ProjectSession, PtySession, VtScreen) {
    let tmp = tempdir().expect("Failed to create tempdir");
    #[cfg(windows)]
    let (cmd, args) = ("cmd.exe", vec!["/c", script]);
    #[cfg(not(windows))]
    let (cmd, args) = ("sh", vec!["-c", script]);

    let config = PtyConfig::new(cmd, tmp.path())
        .with_args(args)
        .with_dimensions(80, 24);
    let pty = PtySession::spawn(config).expect("Failed to spawn test pty");
    let vt = VtScreen::new(80, 24);
    let ps = ProjectSession::new(name, ws, agent);

    (ps, pty, vt)
}

#[test]
fn test_integration_session_exit_normal_multi_session_focus_transition() {
    let mut registry = SessionRegistry::new();

    // Session 1 will exit immediately with code 0
    let (ps1, pty1, vt1) =
        create_process_session("task-fast-complete", "ecommerce", "claude", "exit 0");
    // Session 2 is a long running task
    #[cfg(windows)]
    let wait_cmd = "ping -n 10 127.0.0.1 > nul";
    #[cfg(not(windows))]
    let wait_cmd = "sleep 10";

    let (ps2, pty2, vt2) =
        create_process_session("task-long-running", "ecommerce", "agy", wait_cmd);

    registry.add(ps1, pty1, vt1).unwrap();
    registry.add(ps2, pty2, vt2).unwrap();

    let mut app = TuiApp::new(registry);
    assert_eq!(app.registry.len(), 2);
    assert_eq!(app.registry.active_index(), Some(0));

    // Wait for session 1 to terminate
    std::thread::sleep(Duration::from_millis(150));

    // Process streams
    app.process_pty_streams();

    // Session 1 is closed automatically, leaving Session 2 active
    assert_eq!(app.registry.len(), 1);
    assert_eq!(app.registry.active_index(), Some(0));
    assert_eq!(
        app.registry.active_session().unwrap().project_name(),
        "task-long-running"
    );
    assert!(!app.should_quit);
}

#[test]
fn test_integration_session_exit_normal_last_session_returns_to_dashboard() {
    let mut registry = SessionRegistry::new();
    let (ps, pty, vt) =
        create_process_session("single-project", "workspace1", "claude", "exit 0");
    registry.add(ps, pty, vt).unwrap();

    let dashboard = DashboardState::new(vec!["workspace1".to_string()], vec![]);
    let mut app = TuiApp::new(registry).with_dashboard(dashboard);

    std::thread::sleep(Duration::from_millis(150));
    app.process_pty_streams();

    assert_eq!(app.registry.len(), 0);
    assert!(app.dashboard.is_some());
    assert!(!app.should_quit);
}

#[test]
fn test_integration_session_exit_normal_last_session_cli_mode_exits_shell() {
    let mut registry = SessionRegistry::new();
    let (ps, pty, vt) =
        create_process_session("single-cli-project", "workspace1", "claude", "exit 0");
    registry.add(ps, pty, vt).unwrap();

    let mut app = TuiApp::new(registry);
    assert!(app.dashboard.is_none());

    std::thread::sleep(Duration::from_millis(150));
    app.process_pty_streams();

    assert_eq!(app.registry.len(), 0);
    assert!(app.should_quit);
}

#[test]
fn test_integration_session_exit_error_retention_and_banner_dismissal() {
    let mut registry = SessionRegistry::new();
    let (ps, pty, vt) = create_process_session(
        "failing-agent",
        "workspace1",
        "claude",
        "echo 'Fatal Error: failed to compile' && exit 1",
    );
    registry.add(ps, pty, vt).unwrap();

    let mut app = TuiApp::new(registry);

    // Wait for command output and exit
    std::thread::sleep(Duration::from_millis(200));
    app.process_pty_streams();

    // Session must remain in registry with status Error and exit code 1
    assert_eq!(app.registry.len(), 1);
    let active = app.registry.active_session().unwrap();
    assert_eq!(active.status(), SessionStatus::Error);
    assert_eq!(active.project_session.exit_code, Some(1));
    assert!(!app.should_quit);

    // Verify error banner is rendered on terminal draw
    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal
        .draw(|f| {
            let (_, term_area) = compute_layout(f.area());
            let active = app.registry.active_session().unwrap();
            f.render_widget(TerminalView::new(active.vt()), term_area);
            render_error_banner(active.project_session.exit_code, term_area, f.buffer_mut());
        })
        .unwrap();

    let buf = terminal.backend().buffer();
    let text = buf.content().iter().map(|c| c.symbol()).collect::<String>();
    assert!(text.contains("Fatal Error: failed to compile"));
    assert!(text.contains("[Process exited with code 1 - Press Enter or Ctrl+Space w to dismiss]"));

    // Dismiss with Enter
    let enter_key = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
    app.handle_key_event(enter_key);

    assert_eq!(app.registry.len(), 0);
    assert!(app.should_quit);
}
