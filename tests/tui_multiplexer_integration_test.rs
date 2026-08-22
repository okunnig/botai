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

    // Test Ctrl+Space leader then '2' hotkey mapping and switching
    let ctrl_space = KeyEvent::new(KeyCode::Char(' '), KeyModifiers::CONTROL);
    assert_eq!(map_key_event(ctrl_space, false), TuiAction::EnterLeader);

    let key_2_event = KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE);
    let action = map_key_event(key_2_event, true);
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

    // Test Ctrl+Space leader then 'p' (Prev) hotkey
    assert_eq!(map_key_event(ctrl_space, false), TuiAction::EnterLeader);
    let p_event = KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE);
    let action = map_key_event(p_event, true);
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

    assert!(rendered_text.contains("● 1: backend-api"));
    assert!(rendered_text.contains("● 2: frontend-ui"));
    assert!(rendered_text.contains("API server running on port 8080"));
}

#[test]
fn test_integration_tui_multiplexer_dynamic_resizing_and_cursor() {
    let mut registry = SessionRegistry::new();
    let (mut ps, pty, mut vt) = create_mock_session("interactive-cli", "workspace", "agy");
    ps.set_status(SessionStatus::WaitingForInput);
    // Write prompt and move cursor
    vt.process(b"agy-prompt> user input\x1b[1;23H");

    registry.add(ps, pty, vt).unwrap();

    // 1. Initial 120x40 viewport
    let backend = TestBackend::new(120, 40);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal
        .draw(|f| {
            let (status_area, term_area) = compute_layout(f.area());
            f.render_widget(StatusBar::new(&registry), status_area);
            let active = registry.active_session().unwrap();
            f.render_widget(TerminalView::new(active.vt()), term_area);

            if !active.vt().hide_cursor() {
                let (col, row) = active.vt().cursor_position();
                if col < term_area.width && row < term_area.height {
                    f.set_cursor_position(ratatui::layout::Position::new(
                        term_area.x + col,
                        term_area.y + row,
                    ));
                }
            }
        })
        .unwrap();

    let buf = terminal.backend().buffer();
    let text = buf.content().iter().map(|c| c.symbol()).collect::<String>();
    assert!(text.contains("interactive-cli"));
    assert!(text.contains("agy-prompt> user input"));

    // 2. Resize all sessions to new terminal size 160x50
    registry.resize_all(160, 49);
    assert_eq!(registry.active_session().unwrap().vt().size(), (160, 49));

    let backend2 = TestBackend::new(160, 50);
    let mut terminal2 = Terminal::new(backend2).unwrap();

    terminal2
        .draw(|f| {
            let (status_area, term_area) = compute_layout(f.area());
            f.render_widget(StatusBar::new(&registry), status_area);
            let active = registry.active_session().unwrap();
            f.render_widget(TerminalView::new(active.vt()), term_area);
        })
        .unwrap();

    let text2 = terminal2
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|c| c.symbol())
        .collect::<String>();
    assert!(text2.contains("interactive-cli"));
    assert!(text2.contains("Ctrl+Space:Leader"));
}

#[test]
fn test_integration_tui_multiplexer_mouse_scroll_forwarding_end_to_end() {
    use botai::tui::app::TuiApp;
    use botai::tui::events::{mouse_event_to_bytes, mouse_scroll_to_bytes};
    use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

    let tmp = tempdir().expect("Failed to create tempdir");
    #[cfg(windows)]
    let (cmd, args) = ("cmd.exe", vec!["/c", "ping -n 10 127.0.0.1 > nul"]);
    #[cfg(not(windows))]
    let (cmd, args) = ("sh", vec!["-c", "sleep 10"]);

    let mut registry = SessionRegistry::new();

    // Session 1: backend-api
    let config1 = PtyConfig::new(cmd, tmp.path()).with_args(args.clone());
    let pty1 = PtySession::spawn(config1).unwrap();
    let vt1 = VtScreen::new(80, 24);
    let mut ps1 = ProjectSession::new("backend-api", "workspace", "claude");
    ps1.set_status(SessionStatus::WaitingForInput);
    registry.add(ps1, pty1, vt1).unwrap();

    // Session 2: frontend-ui
    let config2 = PtyConfig::new(cmd, tmp.path()).with_args(args);
    let pty2 = PtySession::spawn(config2).unwrap();
    let vt2 = VtScreen::new(80, 24);
    let mut ps2 = ProjectSession::new("frontend-ui", "workspace", "agy");
    ps2.set_status(SessionStatus::WaitingForInput);
    registry.add(ps2, pty2, vt2).unwrap();

    let mut app = TuiApp::new(registry);
    assert_eq!(app.registry.len(), 2);
    assert_eq!(app.registry.active_index(), Some(0));

    // 1. Verify mouse escape sequence conversions
    assert_eq!(
        mouse_scroll_to_bytes(MouseEventKind::ScrollUp),
        b"\x1b[A\x1b[A\x1b[A".to_vec()
    );
    assert_eq!(
        mouse_scroll_to_bytes(MouseEventKind::ScrollDown),
        b"\x1b[B\x1b[B\x1b[B".to_vec()
    );
    assert_eq!(
        mouse_scroll_to_bytes(MouseEventKind::ScrollLeft),
        b"\x1b[D".to_vec()
    );
    assert_eq!(
        mouse_scroll_to_bytes(MouseEventKind::ScrollRight),
        b"\x1b[C".to_vec()
    );

    // 2. Forward ScrollUp to active session 1
    let scroll_up = MouseEvent {
        kind: MouseEventKind::ScrollUp,
        column: 15,
        row: 10,
        modifiers: KeyModifiers::empty(),
    };
    app.handle_mouse_event(scroll_up);
    assert_eq!(
        app.registry.active_session().unwrap().status(),
        SessionStatus::Busy
    );
    assert_eq!(
        app.registry.active_session().unwrap().project_name(),
        "backend-api"
    );

    // 3. Switch to session 2
    app.registry.set_active_index(1).unwrap();
    assert_eq!(app.registry.active_index(), Some(1));
    assert_eq!(
        app.registry.active_session().unwrap().project_name(),
        "frontend-ui"
    );

    // Forward ScrollDown to active session 2
    let scroll_down = MouseEvent {
        kind: MouseEventKind::ScrollDown,
        column: 20,
        row: 12,
        modifiers: KeyModifiers::empty(),
    };
    app.handle_mouse_event(scroll_down);
    assert_eq!(
        app.registry.active_session().unwrap().status(),
        SessionStatus::Busy
    );

    // 4. When leader is active, mouse scrolling is suppressed
    app.leader_active = true;
    app.registry.active_session_mut().unwrap().project_session.set_status(SessionStatus::WaitingForInput);
    app.handle_mouse_event(scroll_up);
    assert_eq!(
        app.registry.active_session().unwrap().status(),
        SessionStatus::WaitingForInput
    );
    app.leader_active = false;

    // 5. Non-scroll events do not produce activity
    let click = MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 5,
        row: 5,
        modifiers: KeyModifiers::empty(),
    };
    assert!(mouse_event_to_bytes(&click).is_empty());
    app.handle_mouse_event(click);
    assert_eq!(
        app.registry.active_session().unwrap().status(),
        SessionStatus::WaitingForInput
    );
}

