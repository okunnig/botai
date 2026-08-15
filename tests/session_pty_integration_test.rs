use botai::session::{
    ProjectSession, PtyConfig, PtySession, PtySize, SessionError, SessionStatus, VtScreen,
};
use std::collections::HashMap;
use std::thread;
use std::time::Duration;
use tempfile::tempdir;

#[test]
fn test_pty_config_and_session_state_creation() {
    let tmp = tempdir().expect("Failed to create tempdir");
    let working_dir = tmp.path().to_path_buf();

    let mut env = HashMap::new();
    env.insert("BOTAI_TEST_VAR".to_string(), "active".to_string());

    let config = PtyConfig::new("echo", &working_dir)
        .arg("hello botai")
        .with_envs(env.clone())
        .with_size(PtySize::new(100, 30));

    assert_eq!(config.command, "echo");
    assert_eq!(config.args, vec!["hello botai"]);
    assert_eq!(config.working_dir, working_dir);
    assert_eq!(
        config.env.get("BOTAI_TEST_VAR"),
        Some(&"active".to_string())
    );
    assert_eq!(config.size.cols, 100);
    assert_eq!(config.size.rows, 30);

    let mut session = ProjectSession::new("test-project", "test-workspace", "claude")
        .with_prompt("Execute test prompt");

    assert_eq!(session.project_name, "test-project");
    assert_eq!(session.workspace_name, "test-workspace");
    assert_eq!(session.agent_name, "claude");
    assert_eq!(session.status, SessionStatus::Initializing);
    assert_eq!(session.prompt.as_deref(), Some("Execute test prompt"));

    session.mark_busy();
    assert_eq!(session.status, SessionStatus::Busy);
    assert!(session.is_active());

    session.mark_waiting_for_input();
    assert_eq!(session.status, SessionStatus::WaitingForInput);

    session.mark_done(Some(0));
    assert_eq!(session.status, SessionStatus::Done);
    assert_eq!(session.exit_code, Some(0));
    assert!(session.is_terminal());
}

#[test]
fn test_integration_spawn_pty_session_lifecycle() {
    let tmp = tempdir().expect("Failed to create tempdir");

    #[cfg(windows)]
    let (cmd, args) = ("cmd.exe", vec!["/c", "echo hello botai from pty"]);
    #[cfg(not(windows))]
    let (cmd, args) = ("sh", vec!["-c", "echo hello botai from pty"]);

    let config = PtyConfig::new(cmd, tmp.path()).with_args(args);
    let mut session = PtySession::spawn(config).expect("Failed to spawn PTY session");

    let mut output = String::new();
    for _ in 0..20 {
        if let Ok(bytes) = session.read_nonblocking() {
            if !bytes.is_empty() {
                output.push_str(&String::from_utf8_lossy(&bytes));
            }
        }
        if output.contains("hello botai from pty") {
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }

    assert!(
        output.contains("hello botai from pty"),
        "PTY output missing expected text, got: {output}"
    );

    // Wait for process to exit and verify exit status
    let mut exited = false;
    for _ in 0..20 {
        if let Ok(Some(status)) = session.try_wait() {
            assert!(status.success());
            assert_eq!(status.exit_code(), Some(0));
            exited = true;
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }
    assert!(exited, "Process should have exited with code 0");
    assert!(!session.is_alive());
}

#[test]
fn test_integration_vt_screen_processing() {
    let mut vt = VtScreen::new(80, 24);
    vt.process(b"hello botai\r\nline two");
    let contents = vt.contents();
    assert!(contents.contains("hello botai"));
    assert!(contents.contains("line two"));

    // Test ANSI color codes
    vt.process(b"\r\n\x1b[32mgreen text\x1b[0m");
    let formatted = vt.contents_formatted();
    assert!(!formatted.is_empty());

    // Test resize
    vt.resize(100, 30);
    assert_eq!(vt.size(), (100, 30));
    assert_eq!(vt.cols(), 100);
    assert_eq!(vt.rows(), 30);
}

#[test]
fn test_integration_pty_and_vt_screen_end_to_end() {
    let tmp = tempdir().expect("Failed to create tempdir");

    #[cfg(windows)]
    let (cmd, args) = ("cmd.exe", vec!["/c", "echo [TEST_E2E_OUTPUT]"]);
    #[cfg(not(windows))]
    let (cmd, args) = ("sh", vec!["-c", "echo [TEST_E2E_OUTPUT]"]);

    let config = PtyConfig::new(cmd, tmp.path())
        .with_args(args)
        .with_dimensions(80, 24);

    let mut session = PtySession::spawn(config).expect("Failed to spawn PTY session");
    let mut vt = VtScreen::new(80, 24);

    for _ in 0..20 {
        if let Ok(bytes) = session.read_nonblocking() {
            if !bytes.is_empty() {
                vt.process(&bytes);
            }
        }
        if vt.contents().contains("[TEST_E2E_OUTPUT]") {
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }

    assert!(
        vt.contents().contains("[TEST_E2E_OUTPUT]"),
        "VtScreen did not capture PTY output. Contents:\n{}",
        vt.contents()
    );
}

#[test]
fn test_integration_pty_kill_and_termination() {
    let tmp = tempdir().expect("Failed to create tempdir");

    #[cfg(windows)]
    let (cmd, args) = ("cmd.exe", vec!["/c", "timeout /t 30 >nul"]);
    #[cfg(not(windows))]
    let (cmd, args) = ("sh", vec!["-c", "sleep 30"]);

    let config = PtyConfig::new(cmd, tmp.path()).with_args(args);
    let mut session = PtySession::spawn(config).expect("Failed to spawn long-running PTY process");

    assert!(session.is_alive());
    assert!(session.kill().is_ok());

    thread::sleep(Duration::from_millis(100));
    assert!(!session.is_alive());
}

#[test]
fn test_integration_pty_spawn_invalid_command() {
    let tmp = tempdir().expect("Failed to create tempdir");
    let config = PtyConfig::new("nonexistent_command_xyz_123", tmp.path());
    let result = PtySession::spawn(config);
    assert!(
        matches!(
            result,
            Err(SessionError::ProcessNotFound(_)) | Err(SessionError::SpawnFailed { .. })
        ),
        "Expected ProcessNotFound or SpawnFailed error, got: {:?}",
        result.err()
    );
}

#[test]
fn test_cross_platform_command_builder_helper() {
    let tmp = tempdir().expect("Failed to create tempdir");
    let base_dir = tmp.path().to_path_buf();

    #[cfg(windows)]
    let config = PtyConfig::new("cmd.exe", &base_dir)
        .with_args(vec!["/c", "echo cross-platform test"])
        .with_size(PtySize::new(80, 24));

    #[cfg(not(windows))]
    let config = PtyConfig::new("sh", &base_dir)
        .with_args(vec!["-c", "echo cross-platform test"])
        .with_size(PtySize::new(80, 24));

    assert_eq!(config.working_dir, base_dir);
    assert_eq!(config.size.cols, 80);
    assert_eq!(config.size.rows, 24);
    assert!(!config.args.is_empty());
}
