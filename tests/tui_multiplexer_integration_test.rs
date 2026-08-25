use botai::session::{
    ProjectSession, PtyConfig, PtySession, SessionRegistry, SessionStatus, VtScreen,
};
use botai::tui::events::{parse_raw_events, paste_to_bytes, RawAction, RawLeaderDecoder};
use botai::tui::status_bar::StatusBar;
use botai::tui::ui::{compute_layout, TerminalView};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
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

    // Test Ctrl+Space leader then '2' hotkey mapping and switching via RawLeaderDecoder
    let mut decoder = RawLeaderDecoder::new();
    let actions = decoder.decode(&[0x00]);
    assert_eq!(actions, vec![RawAction::EnterLeader]);
    assert!(decoder.is_leader_active());

    let actions = decoder.decode(b"2");
    assert_eq!(actions, vec![RawAction::SwitchSession(1)]);
    assert!(!decoder.is_leader_active());

    registry
        .set_active_index(1)
        .expect("Failed to switch session");
    assert_eq!(registry.active_index(), Some(1));
    assert_eq!(
        registry.active_session().unwrap().project_name(),
        "frontend-ui"
    );

    // Test Ctrl+Space leader then 'p' (Prev) hotkey
    let actions = decoder.decode(&[0x00]);
    assert_eq!(actions, vec![RawAction::EnterLeader]);
    let actions = decoder.decode(b"p");
    assert_eq!(actions, vec![RawAction::PrevSession]);
    registry.prev_session();
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
        .map(ratatui::buffer::Cell::symbol)
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
    let text = buf.content().iter().map(ratatui::buffer::Cell::symbol).collect::<String>();
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
        .map(ratatui::buffer::Cell::symbol)
        .collect::<String>();
    assert!(text2.contains("interactive-cli"));
    assert!(text2.contains("Ctrl+Space:Leader"));
}

#[test]
fn test_integration_tui_multiplexer_mouse_scroll_and_scrollback_end_to_end() {
    use botai::tui::app::TuiApp;
    use botai::tui::events::{mouse_event_to_bytes, mouse_event_to_sgr_bytes, mouse_scroll_to_bytes};
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
    let mut vt1 = VtScreen::new(80, 24);
    for i in 1..=30 {
        vt1.process(format!("backend log line {i}\r\n").as_bytes());
    }
    let mut ps1 = ProjectSession::new("backend-api", "workspace", "claude");
    ps1.set_status(SessionStatus::WaitingForInput);
    registry.add(ps1, pty1, vt1).unwrap();

    // Session 2: frontend-ui (with mouse tracking enabled)
    let config2 = PtyConfig::new(cmd, tmp.path()).with_args(args);
    let pty2 = PtySession::spawn(config2).unwrap();
    let mut vt2 = VtScreen::new(80, 24);
    vt2.process(b"\x1b[?1000h"); // Enable mouse mode
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

    // 2. Session 1 (mouse mode disabled): ScrollUp scrolls native VtScreen scrollback buffer
    let scroll_up = MouseEvent {
        kind: MouseEventKind::ScrollUp,
        column: 15,
        row: 10,
        modifiers: KeyModifiers::empty(),
    };
    app.handle_mouse_event(scroll_up);
    assert_eq!(
        app.registry.active_session().unwrap().vt().scrollback(),
        3
    );
    assert_eq!(
        app.registry.active_session().unwrap().project_name(),
        "backend-api"
    );

    // 3. Switch to session 2 (mouse mode enabled)
    app.registry.set_active_index(1).unwrap();
    assert_eq!(app.registry.active_index(), Some(1));
    assert_eq!(
        app.registry.active_session().unwrap().project_name(),
        "frontend-ui"
    );
    assert!(app.registry.active_session().unwrap().vt().is_mouse_mode_enabled());

    // Forward ScrollDown to active session 2 (emits SGR and marks activity)
    let scroll_down = MouseEvent {
        kind: MouseEventKind::ScrollDown,
        column: 20,
        row: 12,
        modifiers: KeyModifiers::empty(),
    };
    assert_eq!(
        mouse_event_to_sgr_bytes(&scroll_down),
        b"\x1b[<65;21;13M".to_vec()
    );
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

    // 5. Non-scroll events in mouse disabled mode do not produce activity
    app.registry.set_active_index(0).unwrap();
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

#[test]
fn test_integration_tui_multiplexer_raw_events_decoding_and_paste() {
    fn make_key(code: KeyCode, modifiers: KeyModifiers) -> Event {
        Event::Key(KeyEvent::new(code, modifiers))
    }

    // 1. Shift+Enter multiline input format for agy / modern TUIs (\x1b[13;2u)
    assert_eq!(
        parse_raw_events(b"\x1b[13;2u"),
        vec![make_key(KeyCode::Enter, KeyModifiers::SHIFT)]
    );

    // 2. Ctrl+Enter (\x1b[13;5u), Alt+Enter (\x1b[13;3u), Ctrl+Shift+Enter (\x1b[13;6u)
    assert_eq!(
        parse_raw_events(b"\x1b[13;5u"),
        vec![make_key(KeyCode::Enter, KeyModifiers::CONTROL)]
    );
    assert_eq!(
        parse_raw_events(b"\x1b[13;3u"),
        vec![make_key(KeyCode::Enter, KeyModifiers::ALT)]
    );
    assert_eq!(
        parse_raw_events(b"\x1b[13;6u"),
        vec![make_key(KeyCode::Enter, KeyModifiers::CONTROL | KeyModifiers::SHIFT)]
    );

    // 3. Tab, BackTab, Enter, Backspace, Esc
    assert_eq!(
        parse_raw_events(b"\t"),
        vec![make_key(KeyCode::Tab, KeyModifiers::empty())]
    );
    assert_eq!(
        parse_raw_events(b"\x1b[Z"),
        vec![make_key(KeyCode::BackTab, KeyModifiers::SHIFT)]
    );
    assert_eq!(
        parse_raw_events(b"\r"),
        vec![make_key(KeyCode::Enter, KeyModifiers::empty())]
    );
    assert_eq!(
        parse_raw_events(b"\n"),
        vec![make_key(KeyCode::Enter, KeyModifiers::empty())]
    );
    assert_eq!(
        parse_raw_events(&[0x7f]),
        vec![make_key(KeyCode::Backspace, KeyModifiers::empty())]
    );
    assert_eq!(
        parse_raw_events(b"\x1b"),
        vec![make_key(KeyCode::Esc, KeyModifiers::empty())]
    );

    // 4. Modified arrows and navigation
    assert_eq!(
        parse_raw_events(b"\x1b[1;5A"),
        vec![make_key(KeyCode::Up, KeyModifiers::CONTROL)]
    );
    assert_eq!(
        parse_raw_events(b"\x1b[1;2B"),
        vec![make_key(KeyCode::Down, KeyModifiers::SHIFT)]
    );
    assert_eq!(
        parse_raw_events(b"\x1b[1;5H"),
        vec![make_key(KeyCode::Home, KeyModifiers::CONTROL)]
    );

    // 5. Bracketed paste encoding and decoding
    let pasted = paste_to_bytes("def test():\n    return 42\n");
    assert_eq!(
        pasted,
        b"\x1b[200~def test():\n    return 42\n\x1b[201~".to_vec()
    );
    assert_eq!(
        parse_raw_events(&pasted),
        vec![Event::Paste("def test():\n    return 42\n".to_string())]
    );
}

#[test]
fn test_integration_raw_leader_decoder_and_pty_byte_forwarding() {
    use botai::tui::app::TuiApp;
    use botai::tui::events::{RawAction, RawLeaderDecoder};

    let mut decoder = RawLeaderDecoder::new();

    // 1. Raw passthrough streaming
    assert_eq!(
        decoder.decode(b"echo 'hello world'\r\n"),
        vec![RawAction::Passthrough(b"echo 'hello world'\r\n".to_vec())]
    );
    assert!(!decoder.is_leader_active());

    // 2. Ctrl+Space interception via 0x00
    assert_eq!(decoder.decode(&[0x00]), vec![RawAction::EnterLeader]);
    assert!(decoder.is_leader_active());

    // 3. Subsequent session switch hotkey '2'
    assert_eq!(decoder.decode(b"2"), vec![RawAction::SwitchSession(1)]);
    assert!(!decoder.is_leader_active());

    // 4. Kitty protocol Ctrl+Space interception \x1b[32;5u
    assert_eq!(
        decoder.decode(b"\x1b[32;5u"),
        vec![RawAction::EnterLeader]
    );
    assert!(decoder.is_leader_active());

    // 5. Subsequent launcher hotkey 'c'
    assert_eq!(decoder.decode(b"c"), vec![RawAction::OpenLauncher]);
    assert!(!decoder.is_leader_active());

    // 6. Double Ctrl+Space forwards literal 0x00
    assert_eq!(
        decoder.decode(&[0x00, 0x00]),
        vec![RawAction::EnterLeader, RawAction::Passthrough(vec![0x00])]
    );
    assert!(!decoder.is_leader_active());

    // 7. End-to-end PTY raw forwarding in TuiApp
    let mut registry = SessionRegistry::new();
    let (mut ps1, pty1, vt1) = create_mock_session("app1", "ws", "claude");
    ps1.set_status(SessionStatus::WaitingForInput);
    registry.add(ps1, pty1, vt1).unwrap();

    let (mut ps2, pty2, vt2) = create_mock_session("app2", "ws", "agy");
    ps2.set_status(SessionStatus::WaitingForInput);
    registry.add(ps2, pty2, vt2).unwrap();

    let mut app = TuiApp::new(registry);
    assert_eq!(app.registry.active_index(), Some(0));

    // Stream raw bytes to active session 1
    app.handle_raw_bytes(b"pytest\n");
    assert_eq!(
        app.registry.active_session().unwrap().status(),
        SessionStatus::Busy
    );

    // Switch to session 2 via raw bytes: 0x00 followed by '2'
    app.handle_raw_bytes(&[0x00, b'2']);
    assert_eq!(app.registry.active_index(), Some(1));
    assert_eq!(
        app.registry.active_session().unwrap().project_name(),
        "app2"
    );

    // Next / Prev via raw bytes: 0x00 followed by 'p'
    app.handle_raw_bytes(&[0x00, b'p']);
    assert_eq!(app.registry.active_index(), Some(0));
    assert_eq!(
        app.registry.active_session().unwrap().project_name(),
        "app1"
    );
}

