use std::time::{Duration, Instant};
use tempfile::tempdir;

use botai::notify::{
    ActivityTracker, NotificationEvent, NotificationService, StreamControlEvent, StreamScanner,
};
use botai::session::{
    ProjectSession, PtyConfig, PtySession, SessionRegistry, SessionStatus, VtScreen,
};
use botai::tui::app::TuiApp;
use botai::tui::quick_launcher::QuickLauncherState;

fn create_mock_session(
    project_name: &str,
    template_name: &str,
    agent_name: &str,
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
    let ps = ProjectSession::new(project_name, template_name, agent_name);

    (ps, pty, vt)
}

#[test]
fn test_stream_scanner_all_control_sequences_and_chunks() {
    let mut scanner = StreamScanner::new();

    // 1. Standalone BEL
    let events = scanner.scan(b"Processing task...\x07");
    assert_eq!(events, vec![StreamControlEvent::Bell]);

    // 2. OSC 9 with BEL
    let events = scanner.scan(b"\x1b]9;Attention required\x07");
    assert_eq!(
        events,
        vec![StreamControlEvent::Osc9("Attention required".to_string())]
    );

    // 3. OSC 9 with ST (\x1b\)
    let events = scanner.scan(b"\x1b]9;Attention with ST\x1b\\");
    assert_eq!(
        events,
        vec![StreamControlEvent::Osc9("Attention with ST".to_string())]
    );

    // 4. OSC 777 notify
    let events = scanner.scan(b"\x1b]777;notify;botai;Build passed\x07");
    assert_eq!(
        events,
        vec![StreamControlEvent::Osc777 {
            title: "botai".to_string(),
            message: "Build passed".to_string(),
        }]
    );

    // 5. OSC 133 semantic prompt integration
    let events = scanner.scan(b"\x1b]133;A\x07");
    assert_eq!(events, vec![StreamControlEvent::PromptStart]);

    let events = scanner.scan(b"\x1b]133;B\x07");
    assert_eq!(events, vec![StreamControlEvent::CommandStart]);

    let events = scanner.scan(b"\x1b]133;C\x07");
    assert_eq!(events, vec![StreamControlEvent::CommandOutput]);

    let events = scanner.scan(b"\x1b]133;D;0\x07");
    assert_eq!(
        events,
        vec![StreamControlEvent::CommandFinished { exit_code: 0 }]
    );

    let events = scanner.scan(b"\x1b]133;D;1\x07");
    assert_eq!(
        events,
        vec![StreamControlEvent::CommandFinished { exit_code: 1 }]
    );

    // 6. Split chunks across reads
    let ev1 = scanner.scan(b"Prefix \x1b]777;not");
    assert!(ev1.is_empty());
    let ev2 = scanner.scan(b"ify;Test Title;Split Message\x1b");
    assert!(ev2.is_empty());
    let ev3 = scanner.scan(b"\\ Suffix");
    assert_eq!(
        ev3,
        vec![StreamControlEvent::Osc777 {
            title: "Test Title".to_string(),
            message: "Split Message".to_string(),
        }]
    );
}

#[test]
fn test_per_session_activity_tracker_isolation() {
    let mut registry = SessionRegistry::new();
    let (ps1, pty1, vt1) = create_mock_session("proj-alpha", "tpl", "claude");
    let (ps2, pty2, vt2) = create_mock_session("proj-beta", "tpl", "agy");

    let id1 = registry.add(ps1, pty1, vt1).unwrap();
    let id2 = registry.add(ps2, pty2, vt2).unwrap();

    // Session 1 receives stream with BEL -> transitions to WaitingForInput
    let s1 = registry.get_mut(&id1).unwrap();
    let notifs1 = s1
        .tracker
        .process_bytes(b"Claude is waiting for confirmation\x07");
    assert_eq!(s1.tracker.status(), SessionStatus::WaitingForInput);
    assert_eq!(notifs1.len(), 1);

    // Session 2 receives normal stream bytes -> transitions to Busy
    let s2 = registry.get_mut(&id2).unwrap();
    let notifs2 = s2.tracker.process_bytes(b"Agy is compiling code...\n");
    assert_eq!(s2.tracker.status(), SessionStatus::Busy);
    assert!(notifs2.is_empty());

    // Session 1 remains WaitingForInput and Session 2 remains Busy
    assert_eq!(
        registry.get(&id1).unwrap().tracker.status(),
        SessionStatus::WaitingForInput
    );
    assert_eq!(
        registry.get(&id2).unwrap().tracker.status(),
        SessionStatus::Busy
    );
}

#[test]
fn test_stream_bytes_non_destructive_passthrough_to_vt() {
    let (_ps, _pty, mut vt) = create_mock_session("proj-vt", "tpl", "claude");
    let mut tracker = ActivityTracker::default();

    let raw_stream = b"Line 1 text\r\n\x1b]133;A\x07Line 2 prompt > ";
    let notifs = tracker.process_bytes(raw_stream);
    vt.process(raw_stream);

    assert_eq!(tracker.status(), SessionStatus::WaitingForInput);
    assert_eq!(notifs.len(), 1);

    let screen_text = vt.contents();
    assert!(screen_text.contains("Line 1 text"));
    assert!(screen_text.contains("Line 2 prompt > "));
}

#[test]
fn test_background_aware_notification_filtering_focused_vs_background() {
    let mut registry = SessionRegistry::new();
    let (ps1, pty1, vt1) = create_mock_session("focused-proj", "tpl", "claude");
    let (ps2, pty2, vt2) = create_mock_session("background-proj", "tpl", "agy");

    let id1 = registry.add(ps1, pty1, vt1).unwrap();
    let id2 = registry.add(ps2, pty2, vt2).unwrap();

    // Set session 1 as active/focused
    registry.set_active_index(0).unwrap();

    let mut app = TuiApp::new(registry);
    app.notifier = NotificationService::new(true, Duration::from_millis(0));

    // 1. Focused session transitions to WaitingForInput via BEL
    // Notifications for focused session waiting events should be SUPPRESSED.
    let (notifs1, focused_name, focused_id) = {
        let focused_session = app.registry.get_mut(&id1).unwrap();
        let notifs = focused_session.tracker.process_bytes(b"\x07");
        focused_session
            .project_session
            .set_status(focused_session.tracker.status());
        (
            notifs,
            focused_session.project_name().to_string(),
            focused_session.id().to_string(),
        )
    };

    let is_focused_mode =
        app.quick_launcher.is_none() && app.setup_modal.is_none() && !app.is_showing_dashboard();
    let active_id = app.registry.active_session().map(|s| s.id().to_string());

    for mut event in notifs1 {
        event.set_project_name(&focused_name);
        let is_active_focused = is_focused_mode && active_id.as_deref() == Some(&focused_id);
        if event.is_waiting() && is_active_focused {
            // Suppressed!
        } else {
            let sent = app.notifier.notify(&event).unwrap();
            assert!(!sent, "Focused session waiting event should not be sent");
        }
    }

    // 2. Background session transitions to WaitingForInput via OSC 133;A
    // Notifications for background session waiting events should FIRE.
    let (notifs2, bg_name, bg_id) = {
        let bg_session = app.registry.get_mut(&id2).unwrap();
        let notifs = bg_session.tracker.process_bytes(b"\x1b]133;A\x07");
        bg_session
            .project_session
            .set_status(bg_session.tracker.status());
        (
            notifs,
            bg_session.project_name().to_string(),
            bg_session.id().to_string(),
        )
    };

    let mut bg_notified = false;
    for mut event in notifs2 {
        event.set_project_name(&bg_name);
        let is_active_focused = is_focused_mode && active_id.as_deref() == Some(&bg_id);
        if event.is_waiting() && is_active_focused {
            // Suppressed
        } else {
            let sent = app.notifier.notify(&event).unwrap();
            if sent {
                bg_notified = true;
            }
        }
    }
    assert!(
        bg_notified,
        "Background session waiting event must trigger desktop notification"
    );
}

#[test]
fn test_background_aware_filtering_when_modal_or_launcher_active() {
    let mut registry = SessionRegistry::new();
    let (ps1, pty1, vt1) = create_mock_session("my-proj", "tpl", "claude");
    let id1 = registry.add(ps1, pty1, vt1).unwrap();
    registry.set_active_index(0).unwrap();

    let mut app = TuiApp::new(registry);
    app.notifier = NotificationService::new(true, Duration::from_millis(0));

    // Open Quick Launcher modal (so active session is obstructed / modal active)
    app.quick_launcher = Some(QuickLauncherState::new(vec!["tpl".into()], vec![], None));

    let (notifs, name, id) = {
        let session = app.registry.get_mut(&id1).unwrap();
        let notifs = session.tracker.process_bytes(b"\x07");
        session.project_session.set_status(session.tracker.status());
        (
            notifs,
            session.project_name().to_string(),
            session.id().to_string(),
        )
    };

    let is_focused_mode =
        app.quick_launcher.is_none() && app.setup_modal.is_none() && !app.is_showing_dashboard();
    let active_id = app.registry.active_session().map(|s| s.id().to_string());

    let mut notified = false;
    for mut event in notifs {
        event.set_project_name(&name);
        let is_active_focused = is_focused_mode && active_id.as_deref() == Some(&id);
        if event.is_waiting() && is_active_focused {
            // Suppressed
        } else {
            let sent = app.notifier.notify(&event).unwrap();
            if sent {
                notified = true;
            }
        }
    }
    assert!(
        notified,
        "Session obstructed by Quick Launcher modal should fire desktop notification"
    );
}

#[test]
fn test_silence_timeout_fallback_to_waiting_for_input() {
    let mut tracker = ActivityTracker::new(Duration::from_millis(1500));

    // 1. Initial output marks busy
    let notifs = tracker.process_bytes(b"Starting task...\n");
    assert_eq!(tracker.status(), SessionStatus::Busy);
    assert!(notifs.is_empty());

    // 2. Before 1.5s timeout: remains Busy, no notification
    let now = Instant::now();
    tracker.set_last_activity(now);
    let (status, notif, _) = tracker.evaluate(true, None, now + Duration::from_millis(1000));
    assert_eq!(status, SessionStatus::Busy);
    assert!(notif.is_none());

    // 3. After 1.5s silence timeout: transitions to WaitingForInput with event
    let (status, notif, _) = tracker.evaluate(true, None, now + Duration::from_millis(1600));
    assert_eq!(status, SessionStatus::WaitingForInput);
    assert!(matches!(
        notif,
        Some(NotificationEvent::WaitingForInput { .. })
    ));

    // 4. Subsequent evaluate ticks don't re-emit WaitingForInput notification
    let (status, notif_subsequent, _) =
        tracker.evaluate(true, None, now + Duration::from_millis(2000));
    assert_eq!(status, SessionStatus::WaitingForInput);
    assert!(notif_subsequent.is_none());
}
