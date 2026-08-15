use botai::session::{
    ProjectSession, PtyConfig, PtySession, SessionRegistry, SessionStatus, VtScreen,
};
use botai::tui::events::{map_key_event, TuiAction};
use botai::tui::status_bar::StatusBar;
use botai::tui::ui::{compute_layout, TerminalView};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::backend::TestBackend;
use ratatui::Terminal;
use tempfile::tempdir;

fn create_mock_session(
    name: &str,
    ws: &str,
    agent: &str,
) -> (ProjectSession, PtySession, VtScreen) {
    let tmp = tempdir().expect("Failed to create tempdir");
    #[cfg(windows)]
    let (cmd, args) = ("cmd.exe", vec!["/c", "exit 0"]);
    #[cfg(not(windows))]
    let (cmd, args) = ("sh", vec!["-c", "true"]);

    let config = PtyConfig::new(cmd, tmp.path())
        .with_args(args)
        .with_dimensions(80, 24);
    let pty = PtySession::spawn(config).expect("Failed to spawn test pty");
    let vt = VtScreen::new(80, 24);
    let ps = ProjectSession::new(name, ws, agent);

    (ps, pty, vt)
}

#[test]
fn test_integration_tui_multiplexer_navigation_and_rendering() {
    let mut registry = SessionRegistry::new();

    let (mut ps1, pty1, mut vt1) = create_mock_session("backend-api", "payments", "claude");
    ps1.set_status(SessionStatus::Busy);
    vt1.process(b"API server running on port 8080...\n");

    let (mut ps2, pty2, mut vt2) = create_mock_session("frontend-ui", "payments", "agy");
    ps2.set_status(SessionStatus::WaitingForInput);
    vt2.process(b"Ready for prompt> ");

    registry
        .add(ps1, pty1, vt1)
        .expect("Failed to add session 1");
    registry
        .add(ps2, pty2, vt2)
        .expect("Failed to add session 2");

    assert_eq!(registry.len(), 2);
    assert_eq!(registry.active_index(), Some(0));

    // Test Alt+2 hotkey mapping and switching
    let alt_2_event = KeyEvent::new(KeyCode::Char('2'), KeyModifiers::ALT);
    let action = map_key_event(alt_2_event);
    assert_eq!(action, TuiAction::SwitchSession(1));

    if let TuiAction::SwitchSession(idx) = action {
        registry
            .set_active_index(idx)
            .expect("Failed to switch session");
    }
    assert_eq!(registry.active_index(), Some(1));
    assert_eq!(
        registry.active_session().unwrap().project_name(),
        "frontend-ui"
    );

    // Test Alt+P (Prev) hotkey
    let alt_p_event = KeyEvent::new(KeyCode::Char('p'), KeyModifiers::ALT);
    let action = map_key_event(alt_p_event);
    assert_eq!(action, TuiAction::PrevSession);
    if let TuiAction::PrevSession = action {
        registry.prev_session();
    }
    assert_eq!(registry.active_index(), Some(0));
    assert_eq!(
        registry.active_session().unwrap().project_name(),
        "backend-api"
    );

    // Test Ratatui TestBackend rendering
    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).expect("Failed to create test terminal");

    terminal
        .draw(|f| {
            let (status_area, terminal_area) = compute_layout(f.area());
            f.render_widget(StatusBar::new(&registry), status_area);

            if let Some(active) = registry.active_session() {
                f.render_widget(TerminalView::new(active.vt()), terminal_area);
            }
        })
        .expect("Failed to draw frame");

    let buffer = terminal.backend().buffer();
    let rendered_text = buffer
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();

    assert!(rendered_text.contains("1: backend-api ⏳"));
    assert!(rendered_text.contains("2: frontend-ui 💬"));
    assert!(rendered_text.contains("API server running on port 8080"));
}
