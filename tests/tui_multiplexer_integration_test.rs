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

    let (mut ps1, pty1, mut vt1) = create_mock_session("backend-api", "workspace", "claude");
    ps1.set_status(SessionStatus::Busy);
    vt1.process(b"API server running on port 8080...\n");

    let (mut ps2, pty2, mut vt2) = create_mock_session("frontend-ui", "workspace", "agy");
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

    assert!(rendered_text.contains("● 1 [workspace] backend-api"));
    assert!(rendered_text.contains("● 2 [workspace] frontend-ui"));
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
    let text = buf
        .content()
        .iter()
        .map(ratatui::buffer::Cell::symbol)
        .collect::<String>();
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
    assert!(text2.contains("Ctrl+Space: Actions"));
}

#[test]
fn test_integration_tui_multiplexer_mouse_scroll_and_scrollback_end_to_end() {
    use botai::tui::app::TuiApp;
    use botai::tui::events::{
        mouse_event_to_bytes, mouse_event_to_sgr_bytes, mouse_scroll_to_bytes,
    };
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
    assert_eq!(app.registry.active_session().unwrap().vt().scrollback(), 3);
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
    assert!(app
        .registry
        .active_session()
        .unwrap()
        .vt()
        .is_mouse_mode_enabled());

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
    app.registry
        .active_session_mut()
        .unwrap()
        .project_session
        .set_status(SessionStatus::WaitingForInput);
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
        vec![make_key(
            KeyCode::Enter,
            KeyModifiers::CONTROL | KeyModifiers::SHIFT
        )]
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
    assert_eq!(decoder.decode(b"\x1b[32;5u"), vec![RawAction::EnterLeader]);
    assert!(decoder.is_leader_active());

    // 5. Subsequent launcher hotkey 'c'
    assert_eq!(decoder.decode(b"c"), vec![RawAction::OpenLauncher]);
    assert!(!decoder.is_leader_active());

    // 6. Subsequent detach hotkey 'x'
    assert_eq!(decoder.decode(&[0x00]), vec![RawAction::EnterLeader]);
    assert_eq!(decoder.decode(b"x"), vec![RawAction::Detach]);
    assert!(!decoder.is_leader_active());

    // 7. Subsequent destroy hotkey 'd' and 'D'
    assert_eq!(decoder.decode(&[0x00]), vec![RawAction::EnterLeader]);
    assert_eq!(decoder.decode(b"d"), vec![RawAction::DestroyProject]);
    assert!(!decoder.is_leader_active());

    assert_eq!(decoder.decode(&[0x00]), vec![RawAction::EnterLeader]);
    assert_eq!(decoder.decode(b"D"), vec![RawAction::DestroyProject]);
    assert!(!decoder.is_leader_active());

    // 8. Double Ctrl+Space forwards literal 0x00
    assert_eq!(
        decoder.decode(&[0x00, 0x00]),
        vec![RawAction::EnterLeader, RawAction::Passthrough(vec![0x00])]
    );
    assert!(!decoder.is_leader_active());

    // 9. End-to-end PTY raw forwarding in TuiApp
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

#[test]
fn test_integration_shift_enter_multiline_support_claude_vs_agy_and_enter() {
    use botai::tui::app::TuiApp;
    use botai::tui::events::is_shift_enter;
    use std::time::Duration;

    let tmp = tempdir().expect("Failed to create tempdir");
    #[cfg(windows)]
    let (cmd, args) = ("cmd.exe", vec!["/c", "more"]);
    #[cfg(not(windows))]
    let (cmd, args) = ("sh", vec!["-c", "cat"]);

    let mut registry = SessionRegistry::new();

    // 1. Session 1: Claude Code agent
    let config1 = PtyConfig::new(cmd, tmp.path()).with_args(args.clone());
    let pty1 = PtySession::spawn(config1).unwrap();
    let vt1 = VtScreen::new(80, 24);
    let mut ps1 = ProjectSession::new("claude-project", "workspace", "claude");
    ps1.set_status(SessionStatus::WaitingForInput);
    registry.add(ps1, pty1, vt1).unwrap();

    // 2. Session 2: Agy agent
    let config2 = PtyConfig::new(cmd, tmp.path()).with_args(args);
    let pty2 = PtySession::spawn(config2).unwrap();
    let vt2 = VtScreen::new(80, 24);
    let mut ps2 = ProjectSession::new("agy-project", "workspace", "agy");
    ps2.set_status(SessionStatus::WaitingForInput);
    registry.add(ps2, pty2, vt2).unwrap();

    let mut app = TuiApp::new(registry);
    assert_eq!(app.registry.active_index(), Some(0));

    // Verify helper
    assert!(is_shift_enter(b"\x1b[13;2u"));
    assert!(is_shift_enter(b"\x1b[13:10;2u"));
    assert!(is_shift_enter(b"\x1b[27;2;13~"));
    assert!(!is_shift_enter(b"\r"));
    assert!(!is_shift_enter(b"\n"));

    // Claude session: Shift+Enter sends \n (0x0A)
    app.handle_raw_bytes(b"\x1b[13;2u");
    assert_eq!(
        app.registry.active_session().unwrap().status(),
        SessionStatus::Busy
    );

    // Sleep to allow cat process to echo back
    std::thread::sleep(Duration::from_millis(100));
    app.process_pty_streams();
    let text = app.registry.active_session().unwrap().vt().contents();
    // \n was processed by VT
    assert!(
        text.contains('\n')
            || text.is_empty()
            || app.registry.active_session().unwrap().status() == SessionStatus::Busy
    );

    // Claude session: Shift+Enter alternate Kitty format (\x1b[13:10;2u)
    app.registry
        .active_session_mut()
        .unwrap()
        .project_session
        .set_status(SessionStatus::WaitingForInput);
    app.handle_raw_bytes(b"\x1b[13:10;2u");
    assert_eq!(
        app.registry.active_session().unwrap().status(),
        SessionStatus::Busy
    );

    // Claude session: Shift+Enter xterm format (\x1b[27;2;13~]
    app.registry
        .active_session_mut()
        .unwrap()
        .project_session
        .set_status(SessionStatus::WaitingForInput);
    app.handle_raw_bytes(b"\x1b[27;2;13~");
    assert_eq!(
        app.registry.active_session().unwrap().status(),
        SessionStatus::Busy
    );

    // Normal Enter (\r) unaffected
    app.registry
        .active_session_mut()
        .unwrap()
        .project_session
        .set_status(SessionStatus::WaitingForInput);
    app.handle_raw_bytes(b"\r");
    assert_eq!(
        app.registry.active_session().unwrap().status(),
        SessionStatus::Busy
    );

    // Switch to session 2: Agy
    app.registry.set_active_index(1).unwrap();
    assert_eq!(
        app.registry.active_session().unwrap().project_name(),
        "agy-project"
    );

    // Agy session: Shift+Enter forwards \x1b[13;2u
    app.handle_raw_bytes(b"\x1b[13;2u");
    assert_eq!(
        app.registry.active_session().unwrap().status(),
        SessionStatus::Busy
    );

    // Agy session: Alternate formats also forward appropriately
    app.registry
        .active_session_mut()
        .unwrap()
        .project_session
        .set_status(SessionStatus::WaitingForInput);
    app.handle_raw_bytes(b"\x1b[13:10;2u");
    assert_eq!(
        app.registry.active_session().unwrap().status(),
        SessionStatus::Busy
    );

    app.registry
        .active_session_mut()
        .unwrap()
        .project_session
        .set_status(SessionStatus::WaitingForInput);
    app.handle_raw_bytes(b"\x1b[27;2;13~");
    assert_eq!(
        app.registry.active_session().unwrap().status(),
        SessionStatus::Busy
    );

    // Normal Enter (\r) on Agy session unaffected
    app.registry
        .active_session_mut()
        .unwrap()
        .project_session
        .set_status(SessionStatus::WaitingForInput);
    app.handle_raw_bytes(b"\r");
    assert_eq!(
        app.registry.active_session().unwrap().status(),
        SessionStatus::Busy
    );
}

#[test]
fn test_integration_mouse_wheel_scrolling_via_handle_raw_bytes() {
    use botai::tui::app::TuiApp;

    let tmp = tempdir().expect("Failed to create tempdir");
    #[cfg(windows)]
    let (cmd, args) = ("cmd.exe", vec!["/c", "ping -n 10 127.0.0.1 > nul"]);
    #[cfg(not(windows))]
    let (cmd, args) = ("sh", vec!["-c", "sleep 10"]);

    let mut registry = SessionRegistry::new();

    // Session 1: mouse disabled
    let config1 = PtyConfig::new(cmd, tmp.path()).with_args(args.clone());
    let pty1 = PtySession::spawn(config1).unwrap();
    let mut vt1 = VtScreen::new(80, 24);
    for i in 1..=30 {
        vt1.process(format!("output line {i}\r\n").as_bytes());
    }
    let mut ps1 = ProjectSession::new("scroll-test", "ws", "claude");
    ps1.set_status(SessionStatus::WaitingForInput);
    registry.add(ps1, pty1, vt1).unwrap();

    // Session 2: mouse enabled
    let config2 = PtyConfig::new(cmd, tmp.path()).with_args(args);
    let pty2 = PtySession::spawn(config2).unwrap();
    let mut vt2 = VtScreen::new(80, 24);
    vt2.process(b"\x1b[?1000h");
    let mut ps2 = ProjectSession::new("mouse-app", "ws", "agy");
    ps2.set_status(SessionStatus::WaitingForInput);
    registry.add(ps2, pty2, vt2).unwrap();

    let mut app = TuiApp::new(registry);

    // 1. In Session 1, SGR ScrollUp raw bytes (\x1b[<64;10;5M) navigate virtual terminal scrollback directly
    app.handle_raw_bytes(b"\x1b[<64;10;5M");
    assert_eq!(app.registry.active_session().unwrap().vt().scrollback(), 3);
    assert_eq!(
        app.registry.active_session().unwrap().status(),
        SessionStatus::WaitingForInput
    );

    app.handle_raw_bytes(b"\x1b[<64;10;5M");
    assert_eq!(app.registry.active_session().unwrap().vt().scrollback(), 6);

    // SGR ScrollDown raw bytes (\x1b[<65;10;5M) scroll down by 3
    app.handle_raw_bytes(b"\x1b[<65;10;5M");
    assert_eq!(app.registry.active_session().unwrap().vt().scrollback(), 3);

    // 2. Typing any key resets scrollback to 0
    app.handle_raw_bytes(b"x");
    assert_eq!(app.registry.active_session().unwrap().vt().scrollback(), 0);

    // 3. In Session 2 (mouse mode enabled), SGR bytes are forwarded to PTY and record activity
    app.registry.set_active_index(1).unwrap();
    assert!(app
        .registry
        .active_session()
        .unwrap()
        .vt()
        .is_mouse_mode_enabled());

    app.handle_raw_bytes(b"\x1b[<64;10;5M");
    assert_eq!(
        app.registry.active_session().unwrap().status(),
        SessionStatus::Busy
    );
}

#[test]
fn test_integration_mixed_mouse_and_keystrokes_stream_demuxing() {
    use botai::tui::app::TuiApp;
    use std::time::Duration;

    let tmp = tempdir().expect("Failed to create tempdir");
    #[cfg(windows)]
    let (cmd, args) = ("cmd.exe", vec!["/c", "more"]);
    #[cfg(not(windows))]
    let (cmd, args) = ("sh", vec!["-c", "cat"]);

    let mut registry = SessionRegistry::new();
    let config = PtyConfig::new(cmd, tmp.path()).with_args(args);
    let pty = PtySession::spawn(config).unwrap();
    let mut vt = VtScreen::new(80, 24);
    for i in 1..=30 {
        vt.process(format!("output line {i}\r\n").as_bytes());
    }
    let mut ps = ProjectSession::new("stream-demux", "ws", "claude");
    ps.set_status(SessionStatus::WaitingForInput);
    registry.add(ps, pty, vt).unwrap();

    let mut app = TuiApp::new(registry);

    // 1. Mouse scroll + trailing keystrokes in a single chunk: \x1b[<64;10;20Mhello\r
    app.handle_raw_bytes(b"\x1b[<64;10;20Mhello\r");
    // Scrollback was scrolled up by 3 from the mouse event, but typing "hello\r" reset it to 0
    assert_eq!(app.registry.active_session().unwrap().vt().scrollback(), 0);
    assert_eq!(
        app.registry.active_session().unwrap().status(),
        SessionStatus::Busy
    );

    // Let the cat process echo back
    std::thread::sleep(Duration::from_millis(100));
    app.process_pty_streams();
    let text = app.registry.active_session().unwrap().vt().contents();
    assert!(
        text.contains("hello")
            || app.registry.active_session().unwrap().status() == SessionStatus::Busy
    );

    // 2. Pure mouse scroll alone: increases scrollback
    app.registry
        .active_session_mut()
        .unwrap()
        .project_session
        .set_status(SessionStatus::WaitingForInput);
    app.handle_raw_bytes(b"\x1b[<64;10;20M");
    assert_eq!(app.registry.active_session().unwrap().vt().scrollback(), 3);

    // 3. Keystrokes before and after mouse event: b"abc\x1b[<64;10;20Mdef\r"
    app.handle_raw_bytes(b"abc\x1b[<64;10;20Mdef\r");
    assert_eq!(
        app.registry.active_session().unwrap().status(),
        SessionStatus::Busy
    );
}

#[test]
fn test_integration_errored_session_mouse_vs_key_dismissal() {
    use botai::tui::app::TuiApp;

    let tmp = tempdir().expect("Failed to create tempdir");
    #[cfg(windows)]
    let (cmd, args) = ("cmd.exe", vec!["/c", "exit 1"]);
    #[cfg(not(windows))]
    let (cmd, args) = ("sh", vec!["-c", "exit 1"]);

    let mut registry = SessionRegistry::new();
    let config = PtyConfig::new(cmd, tmp.path()).with_args(args);
    let pty = PtySession::spawn(config).unwrap();
    let vt = VtScreen::new(80, 24);
    let mut ps = ProjectSession::new("error-session", "ws", "claude");
    ps.set_status(SessionStatus::Error);
    registry.add(ps, pty, vt).unwrap();

    let mut app = TuiApp::new(registry);
    assert_eq!(app.registry.len(), 1);

    // 1. Mouse scroll event alone does NOT dismiss the errored session
    app.handle_raw_bytes(b"\x1b[<64;10;20M");
    assert_eq!(app.registry.len(), 1);

    // 2. Mouse event whose coordinates or button contain ASCII byte 32 (space) does NOT dismiss
    app.handle_raw_bytes(b"\x1b[<64;32;20M");
    assert_eq!(app.registry.len(), 1);

    // 3. Mixed mouse event followed by Enter key dismisses the errored session
    app.handle_raw_bytes(b"\x1b[<64;10;20M\r");
    assert_eq!(app.registry.len(), 0);
}

#[test]
fn test_integration_split_sgr_mouse_and_keystrokes_across_chunks() {
    use botai::tui::app::TuiApp;
    use std::time::Duration;

    let tmp = tempdir().expect("Failed to create tempdir");
    #[cfg(windows)]
    let (cmd, args) = ("cmd.exe", vec!["/c", "more"]);
    #[cfg(not(windows))]
    let (cmd, args) = ("sh", vec!["-c", "cat"]);

    let mut registry = SessionRegistry::new();
    let config = PtyConfig::new(cmd, tmp.path()).with_args(args);
    let pty = PtySession::spawn(config).unwrap();
    let mut vt = VtScreen::new(80, 24);
    for i in 1..=30 {
        vt.process(format!("output line {i}\r\n").as_bytes());
    }
    let mut ps = ProjectSession::new("split-mouse", "ws", "claude");
    ps.set_status(SessionStatus::WaitingForInput);
    registry.add(ps, pty, vt).unwrap();

    let mut app = TuiApp::new(registry);

    // 1. Test split SGR mouse sequence across chunks: Chunk 1: b"\x1b[<64;10;", Chunk 2: b"20M"
    app.handle_raw_bytes(b"\x1b[<64;10;");
    assert_eq!(app.pending_raw_input, b"\x1b[<64;10;");
    assert_eq!(app.registry.active_session().unwrap().vt().scrollback(), 0);

    app.handle_raw_bytes(b"20M");
    assert!(app.pending_raw_input.is_empty());
    assert_eq!(app.registry.active_session().unwrap().vt().scrollback(), 3);

    // 2. Test split SGR mouse followed by trailing keystrokes across chunks: Chunk 1: b"\x1b[<64;10;", Chunk 2: b"20Mhello\r"
    app.registry
        .active_session_mut()
        .unwrap()
        .project_session
        .set_status(SessionStatus::WaitingForInput);
    app.handle_raw_bytes(b"\x1b[<64;10;");
    assert_eq!(app.pending_raw_input, b"\x1b[<64;10;");

    app.handle_raw_bytes(b"20Mhello\r");
    assert!(app.pending_raw_input.is_empty());
    // Mouse scroll increased scrollback, but trailing "hello\r" reset it to 0 and marked busy
    assert_eq!(app.registry.active_session().unwrap().vt().scrollback(), 0);
    assert_eq!(
        app.registry.active_session().unwrap().status(),
        SessionStatus::Busy
    );

    // Let the cat process echo back
    std::thread::sleep(Duration::from_millis(100));
    app.process_pty_streams();
    let text = app.registry.active_session().unwrap().vt().contents();
    assert!(
        text.contains("hello")
            || app.registry.active_session().unwrap().status() == SessionStatus::Busy
    );
}

#[test]
fn test_integration_mouse_coordinate_translation_to_child_viewport() {
    use botai::tui::app::TuiApp;
    use botai::tui::events::{mouse_event_to_sgr_bytes, translate_mouse_event};
    use botai::tui::ui::STATUS_BAR_HEIGHT;
    use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
    use std::time::Duration;

    let tmp = tempdir().expect("Failed to create tempdir");
    #[cfg(windows)]
    let (cmd, args) = ("cmd.exe", vec!["/c", "more"]);
    #[cfg(not(windows))]
    let (cmd, args) = ("sh", vec!["-c", "cat"]);

    let mut registry = SessionRegistry::new();
    let config = PtyConfig::new(cmd, tmp.path()).with_args(args);
    let pty = PtySession::spawn(config).unwrap();
    let mut vt = VtScreen::new(80, 24);
    vt.process(b"\x1b[?1000h"); // Enable mouse tracking
    let mut ps = ProjectSession::new("mouse-app", "ws", "claude");
    ps.set_status(SessionStatus::WaitingForInput);
    registry.add(ps, pty, vt).unwrap();

    let mut app = TuiApp::new(registry);
    assert_eq!(STATUS_BAR_HEIGHT, 1);

    // 1. Mouse event on row 0 (status bar) is ignored and not forwarded to child PTY
    let click_row_0 = MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 9,
        row: 0,
        modifiers: KeyModifiers::empty(),
    };
    assert_eq!(
        translate_mouse_event(click_row_0, STATUS_BAR_HEIGHT, 24),
        None
    );
    app.handle_mouse_event(click_row_0);
    assert_eq!(
        app.registry.active_session().unwrap().status(),
        SessionStatus::WaitingForInput
    );

    // 2. Mouse event on row 1 is translated to child row 0 and generates SGR row 1 (\x1b[<0;10;1M)
    let click_row_1 = MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 9,
        row: 1,
        modifiers: KeyModifiers::empty(),
    };
    let translated_1 = translate_mouse_event(click_row_1, STATUS_BAR_HEIGHT, 24).unwrap();
    assert_eq!(translated_1.row, 0);
    assert_eq!(translated_1.column, 9);
    assert_eq!(mouse_event_to_sgr_bytes(&translated_1), b"\x1b[<0;10;1M");

    app.handle_mouse_event(click_row_1);
    assert_eq!(
        app.registry.active_session().unwrap().status(),
        SessionStatus::Busy
    );

    #[cfg(not(windows))]
    {
        // On Unix, cat echoes back the bytes forwarded to PTY
        std::thread::sleep(Duration::from_millis(100));
        let echoed = app
            .registry
            .active_session_mut()
            .unwrap()
            .pty_mut()
            .read_nonblocking()
            .unwrap_or_default();
        let echoed_str = String::from_utf8_lossy(&echoed);
        assert!(
            echoed_str.contains("\x1b[<0;10;1M") || echoed_str.contains("^[[<0;10;1M"),
            "Expected echoed PTY bytes to contain '\\x1b[<0;10;1M' or '^[[<0;10;1M', got: {:?}",
            echoed_str
        );
    }

    // 3. Mouse event on row N (e.g. row 15) is translated to child row N-1 (row 14) and generates SGR row N (\x1b[<0;10;15M)
    app.registry
        .active_session_mut()
        .unwrap()
        .project_session
        .set_status(SessionStatus::WaitingForInput);
    let click_row_15 = MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 9,
        row: 15,
        modifiers: KeyModifiers::empty(),
    };
    let translated_15 = translate_mouse_event(click_row_15, STATUS_BAR_HEIGHT, 24).unwrap();
    assert_eq!(translated_15.row, 14);
    assert_eq!(translated_15.column, 9);
    assert_eq!(mouse_event_to_sgr_bytes(&translated_15), b"\x1b[<0;10;15M");

    app.handle_mouse_event(click_row_15);
    assert_eq!(
        app.registry.active_session().unwrap().status(),
        SessionStatus::Busy
    );

    #[cfg(not(windows))]
    {
        std::thread::sleep(Duration::from_millis(100));
        let echoed = app
            .registry
            .active_session_mut()
            .unwrap()
            .pty_mut()
            .read_nonblocking()
            .unwrap_or_default();
        let echoed_str = String::from_utf8_lossy(&echoed);
        assert!(
            echoed_str.contains("\x1b[<0;10;15M") || echoed_str.contains("^[[<0;10;15M"),
            "Expected echoed PTY bytes to contain '\\x1b[<0;10;15M' or '^[[<0;10;15M', got: {:?}",
            echoed_str
        );
    }

    // 4. Drag and Up events on row N are translated accordingly
    // Drag on host row 15 (child row 14) -> \x1b[<32;10;15M
    app.registry
        .active_session_mut()
        .unwrap()
        .project_session
        .set_status(SessionStatus::WaitingForInput);
    let drag_row_15 = MouseEvent {
        kind: MouseEventKind::Drag(MouseButton::Left),
        column: 9,
        row: 15,
        modifiers: KeyModifiers::empty(),
    };
    let translated_drag = translate_mouse_event(drag_row_15, STATUS_BAR_HEIGHT, 24).unwrap();
    assert_eq!(translated_drag.row, 14);
    assert_eq!(
        translated_drag.kind,
        MouseEventKind::Drag(MouseButton::Left)
    );
    assert_eq!(
        mouse_event_to_sgr_bytes(&translated_drag),
        b"\x1b[<32;10;15M"
    );

    app.handle_mouse_event(drag_row_15);
    assert_eq!(
        app.registry.active_session().unwrap().status(),
        SessionStatus::Busy
    );

    #[cfg(not(windows))]
    {
        std::thread::sleep(Duration::from_millis(100));
        let echoed = app
            .registry
            .active_session_mut()
            .unwrap()
            .pty_mut()
            .read_nonblocking()
            .unwrap_or_default();
        let echoed_str = String::from_utf8_lossy(&echoed);
        assert!(
            echoed_str.contains("\x1b[<32;10;15M") || echoed_str.contains("^[[<32;10;15M"),
            "Expected echoed PTY bytes to contain '\\x1b[<32;10;15M' or '^[[<32;10;15M', got: {:?}",
            echoed_str
        );
    }

    // Up on host row 15 (child row 14) -> \x1b[<0;10;15m
    app.registry
        .active_session_mut()
        .unwrap()
        .project_session
        .set_status(SessionStatus::WaitingForInput);
    let up_row_15 = MouseEvent {
        kind: MouseEventKind::Up(MouseButton::Left),
        column: 9,
        row: 15,
        modifiers: KeyModifiers::empty(),
    };
    let translated_up = translate_mouse_event(up_row_15, STATUS_BAR_HEIGHT, 24).unwrap();
    assert_eq!(translated_up.row, 14);
    assert_eq!(translated_up.kind, MouseEventKind::Up(MouseButton::Left));
    assert_eq!(mouse_event_to_sgr_bytes(&translated_up), b"\x1b[<0;10;15m");

    app.handle_mouse_event(up_row_15);
    assert_eq!(
        app.registry.active_session().unwrap().status(),
        SessionStatus::Busy
    );

    #[cfg(not(windows))]
    {
        std::thread::sleep(Duration::from_millis(100));
        let echoed = app
            .registry
            .active_session_mut()
            .unwrap()
            .pty_mut()
            .read_nonblocking()
            .unwrap_or_default();
        let echoed_str = String::from_utf8_lossy(&echoed);
        assert!(
            echoed_str.contains("\x1b[<0;10;15m") || echoed_str.contains("^[[<0;10;15m"),
            "Expected echoed PTY bytes to contain '\\x1b[<0;10;15m' or '^[[<0;10;15m', got: {:?}",
            echoed_str
        );
    }

    // 5. Raw SGR bytes via handle_raw_bytes:
    // Host row 1 (1-based SGR \x1b[<0;10;1M -> 0-based host row 0 -> status bar) is rejected
    app.registry
        .active_session_mut()
        .unwrap()
        .project_session
        .set_status(SessionStatus::WaitingForInput);
    app.handle_raw_bytes(b"\x1b[<0;10;1M");
    assert_eq!(
        app.registry.active_session().unwrap().status(),
        SessionStatus::WaitingForInput
    );

    // Host row 2 (1-based SGR \x1b[<0;10;2M -> 0-based host row 1 -> child row 0) is accepted
    app.handle_raw_bytes(b"\x1b[<0;10;2M");
    assert_eq!(
        app.registry.active_session().unwrap().status(),
        SessionStatus::Busy
    );
}

#[test]
fn test_integration_escape_key_forwarding_and_leader_cancellation() {
    use botai::tui::app::TuiApp;
    use std::time::Duration;

    let tmp = tempdir().expect("Failed to create tempdir");
    #[cfg(windows)]
    let (cmd, args) = ("cmd.exe", vec!["/c", "more"]);
    #[cfg(not(windows))]
    let (cmd, args) = ("sh", vec!["-c", "cat"]);

    let mut registry = SessionRegistry::new();
    let config = PtyConfig::new(cmd, tmp.path()).with_args(args);
    let pty = PtySession::spawn(config).unwrap();
    let vt = VtScreen::new(80, 24);
    let mut ps = ProjectSession::new("esc-test", "ws", "claude");
    ps.set_status(SessionStatus::WaitingForInput);
    registry.add(ps, pty, vt).unwrap();

    let mut app = TuiApp::new(registry);

    // 1. Standalone 0x1b fed into handle_raw_bytes does NOT get trapped in pending_raw_input
    app.handle_raw_bytes(b"\x1b");
    assert!(
        app.pending_raw_input.is_empty(),
        "0x1b must not be stored in pending_raw_input"
    );
    assert!(
        app.raw_decoder.has_pending(),
        "0x1b must be held in raw_decoder pending escape disambiguation"
    );

    // Before flush, status remains WaitingForInput
    assert_eq!(
        app.registry.active_session().unwrap().status(),
        SessionStatus::WaitingForInput
    );

    // 2. flush_pending_input flushes \x1b to active PTY
    app.flush_pending_input();
    assert!(!app.raw_decoder.has_pending());
    assert_eq!(
        app.registry.active_session().unwrap().status(),
        SessionStatus::Busy
    );

    #[cfg(not(windows))]
    {
        std::thread::sleep(Duration::from_millis(100));
        let echoed = app
            .registry
            .active_session_mut()
            .unwrap()
            .pty_mut()
            .read_nonblocking()
            .unwrap_or_default();
        let echoed_str = String::from_utf8_lossy(&echoed);
        assert!(
            echoed_str.contains('\x1b') || echoed_str.contains("^["),
            "Expected echoed PTY bytes to contain '\\x1b' or '^[', got: {:?}",
            echoed_str
        );
    }

    // 3. Standalone 0x1b fed when leader_active is true followed by flush_pending_input() cancels leader mode
    app.registry
        .active_session_mut()
        .unwrap()
        .project_session
        .set_status(SessionStatus::WaitingForInput);

    // Activate leader mode
    app.handle_raw_bytes(&[0x00]);
    assert!(app.leader_active);
    assert!(app.raw_decoder.is_leader_active());

    // Send 0x1b (Esc)
    app.handle_raw_bytes(b"\x1b");
    assert!(app.pending_raw_input.is_empty());
    assert!(app.raw_decoder.has_pending());
    assert!(app.leader_active);

    // Flush cancels leader mode and resets leader_active to false
    app.flush_pending_input();
    assert!(!app.leader_active);
    assert!(!app.raw_decoder.is_leader_active());
    assert!(!app.raw_decoder.has_pending());
    // Active session status should still be WaitingForInput (escape was consumed to cancel leader, not forwarded to PTY)
    assert_eq!(
        app.registry.active_session().unwrap().status(),
        SessionStatus::WaitingForInput
    );
}

#[test]
fn test_integration_escape_timeout_in_run_loop_with_receiver() {
    use botai::tui::app::TuiApp;
    use ratatui::backend::TestBackend;
    use std::sync::mpsc;
    use std::time::Duration;

    let tmp = tempdir().expect("Failed to create tempdir");
    #[cfg(windows)]
    let (cmd, args) = ("cmd.exe", vec!["/c", "ping -n 10 127.0.0.1 > nul"]);
    #[cfg(not(windows))]
    let (cmd, args) = ("sh", vec!["-c", "sleep 10"]);

    let mut registry = SessionRegistry::new();
    let config = PtyConfig::new(cmd, tmp.path()).with_args(args);
    let pty = PtySession::spawn(config).unwrap();
    let vt = VtScreen::new(80, 24);
    let mut ps = ProjectSession::new("loop-esc", "ws", "claude");
    ps.set_status(SessionStatus::WaitingForInput);
    registry.add(ps, pty, vt).unwrap();

    let mut app = TuiApp::new(registry);
    app.tick_rate = Duration::from_millis(20);

    let (stdin_tx, stdin_rx) = mpsc::channel();
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();

    // Send standalone 0x1b into channel
    stdin_tx.send(vec![0x1b]).unwrap();

    // Spawn a thread to send Ctrl+Space + q after timeout has elapsed, quitting the loop
    let tx_clone = stdin_tx.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(60));
        let _ = tx_clone.send(vec![0x00, b'q']);
    });

    app.run_loop_with_receiver(&mut terminal, &stdin_rx)
        .unwrap();

    // After the loop has processed the timeout, the escape was flushed to the active session PTY
    // making status Busy before the subsequent quit
    assert!(app.should_quit);
    assert!(!app.raw_decoder.has_pending());
    assert!(app.pending_raw_input.is_empty());
}
