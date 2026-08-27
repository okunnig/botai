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
    assert!(text2.contains("Ctrl+Space:Leader"));
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
fn test_integration_tui_multiplexer_csi_u_modifiers_and_paste() {
    use botai::tui::events::{
        host_modify_other_keys_mode_for_env, key_to_bytes, keyboard_enhancement_flags,
        paste_to_bytes, KeyHandler, ModifyOtherKeysMode,
    };
    use crossterm::event::KeyboardEnhancementFlags;

    // 1. Verify keyboard enhancement flags
    let flags = keyboard_enhancement_flags();
    assert_eq!(flags, KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES);
    assert!(flags.contains(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES));
    assert!(!flags.contains(KeyboardEnhancementFlags::REPORT_EVENT_TYPES));
    assert!(!flags.contains(KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS));
    assert!(!flags.contains(KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES));
    assert_eq!(KeyHandler::keyboard_enhancement_flags(), flags);

    // 2. Verify modifyOtherKeys host terminal environment detection
    assert_eq!(
        host_modify_other_keys_mode_for_env(true, None, false, false),
        Some(ModifyOtherKeysMode::Mode2)
    );
    assert_eq!(
        host_modify_other_keys_mode_for_env(false, Some("WezTerm"), false, false),
        None
    );
    assert_eq!(
        host_modify_other_keys_mode_for_env(false, None, true, false),
        None
    );
    assert_eq!(
        host_modify_other_keys_mode_for_env(false, Some("Alacritty"), false, false),
        None
    );
    assert_eq!(
        host_modify_other_keys_mode_for_env(false, None, false, true),
        None
    );
    assert_eq!(
        host_modify_other_keys_mode_for_env(false, Some("xterm"), false, false),
        None
    );

    assert_eq!(ModifyOtherKeysMode::Mode1.enable_sequence(), b"\x1b[>4;1m");
    assert_eq!(ModifyOtherKeysMode::Mode2.enable_sequence(), b"\x1b[>4;2m");
    assert_eq!(ModifyOtherKeysMode::reset_sequence(), b"\x1b[>4;0m");

    // 3. Shift+Enter multiline input format for agy / modern TUIs
    let shift_enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT);
    assert_eq!(key_to_bytes(&shift_enter), b"\x1b[13;2u".to_vec());

    let shift_char_lf = KeyEvent::new(KeyCode::Char('\n'), KeyModifiers::SHIFT);
    assert_eq!(key_to_bytes(&shift_char_lf), b"\x1b[13;2u".to_vec());

    let shift_char_cr = KeyEvent::new(KeyCode::Char('\r'), KeyModifiers::SHIFT);
    assert_eq!(key_to_bytes(&shift_char_cr), b"\x1b[13;2u".to_vec());

    // 4. Ctrl+Enter, Alt+Enter, Ctrl+Shift+Enter, Super+Enter, Ctrl+J
    let ctrl_enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL);
    assert_eq!(key_to_bytes(&ctrl_enter), b"\x1b[13;5u".to_vec());

    let alt_enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT);
    assert_eq!(key_to_bytes(&alt_enter), b"\x1b[13;3u".to_vec());

    let alt_char_lf = KeyEvent::new(KeyCode::Char('\n'), KeyModifiers::ALT);
    assert_eq!(key_to_bytes(&alt_char_lf), b"\x1b[13;3u".to_vec());

    let alt_char_cr = KeyEvent::new(KeyCode::Char('\r'), KeyModifiers::ALT);
    assert_eq!(key_to_bytes(&alt_char_cr), b"\x1b[13;3u".to_vec());

    let ctrl_shift_enter =
        KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL | KeyModifiers::SHIFT);
    assert_eq!(key_to_bytes(&ctrl_shift_enter), b"\x1b[13;6u".to_vec());

    let super_enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::SUPER);
    assert_eq!(key_to_bytes(&super_enter), b"\x1b[13;9u".to_vec());

    let ctrl_j = KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL);
    assert_eq!(key_to_bytes(&ctrl_j), vec![0x0a]);

    // Plain LF / CR / Enter
    let plain_enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(key_to_bytes(&plain_enter), b"\r".to_vec());

    let plain_char_lf = KeyEvent::new(KeyCode::Char('\n'), KeyModifiers::NONE);
    assert_eq!(key_to_bytes(&plain_char_lf), b"\n".to_vec());

    let plain_char_cr = KeyEvent::new(KeyCode::Char('\r'), KeyModifiers::NONE);
    assert_eq!(key_to_bytes(&plain_char_cr), b"\r".to_vec());

    // 5. Modified arrows and navigation
    let ctrl_up = KeyEvent::new(KeyCode::Up, KeyModifiers::CONTROL);
    assert_eq!(key_to_bytes(&ctrl_up), b"\x1b[1;5A".to_vec());

    let shift_down = KeyEvent::new(KeyCode::Down, KeyModifiers::SHIFT);
    assert_eq!(key_to_bytes(&shift_down), b"\x1b[1;2B".to_vec());

    let ctrl_home = KeyEvent::new(KeyCode::Home, KeyModifiers::CONTROL);
    assert_eq!(key_to_bytes(&ctrl_home), b"\x1b[1;5H".to_vec());

    // 6. Bracketed paste encoding
    let pasted = paste_to_bytes("def test():\n    return 42\n");
    assert_eq!(
        pasted,
        b"\x1b[200~def test():\n    return 42\n\x1b[201~".to_vec()
    );
}
