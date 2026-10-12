pub mod pty;
pub mod registry;
pub mod state;
pub mod vt;

pub use pty::{ExitStatus, PtyConfig, PtySession, PtySize, PtyWaker};
pub use registry::{SessionEntry, SessionRegistry};
pub use state::{ProjectSession, SessionState, SessionStatus};
pub use vt::{DsrResponder, VtCell, VtScreen};

use std::path::PathBuf;
use thiserror::Error;

/// Errors that can occur during PTY session management, virtual terminal operations,
/// and process lifecycle management.
#[derive(Debug, Error)]
pub enum SessionError {
    #[error("Failed to spawn process '{command}': {reason}")]
    SpawnFailed { command: String, reason: String },

    #[error("I/O error during PTY operation: {0}")]
    Io(#[from] std::io::Error),

    #[error("Command or executable not found: {0}")]
    ProcessNotFound(String),

    #[error("Process has already terminated")]
    AlreadyTerminated,

    #[error("Invalid terminal dimensions: cols={cols}, rows={rows}")]
    InvalidSize { cols: u16, rows: u16 },

    #[error("Working directory does not exist or is invalid: {0}")]
    InvalidWorkingDirectory(PathBuf),

    #[error("Invalid session index: {0}")]
    InvalidSessionIndex(usize),

    #[error("Session not found: {0}")]
    SessionNotFound(String),

    #[error("PTY session error: {0}")]
    Other(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_error_display_formatting() {
        let spawn_err = SessionError::SpawnFailed {
            command: "nonexistent_cli".to_string(),
            reason: "No such file or directory".to_string(),
        };
        assert_eq!(
            spawn_err.to_string(),
            "Failed to spawn process 'nonexistent_cli': No such file or directory"
        );

        let not_found = SessionError::ProcessNotFound("claude".to_string());
        assert_eq!(
            not_found.to_string(),
            "Command or executable not found: claude"
        );

        let already_term = SessionError::AlreadyTerminated;
        assert_eq!(already_term.to_string(), "Process has already terminated");

        let invalid_size = SessionError::InvalidSize { cols: 0, rows: 0 };
        assert_eq!(
            invalid_size.to_string(),
            "Invalid terminal dimensions: cols=0, rows=0"
        );

        let invalid_dir = SessionError::InvalidWorkingDirectory(PathBuf::from("/nonexistent/path"));
        assert_eq!(
            invalid_dir.to_string(),
            "Working directory does not exist or is invalid: /nonexistent/path"
        );

        let other = SessionError::Other("custom pty failure".to_string());
        assert_eq!(other.to_string(), "PTY session error: custom pty failure");
    }

    #[test]
    fn test_io_error_conversion_into_session_error() {
        let io_err = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "permission denied");
        let session_err: SessionError = io_err.into();
        match session_err {
            SessionError::Io(e) => {
                assert_eq!(e.kind(), std::io::ErrorKind::PermissionDenied);
            }
            _ => panic!("Expected SessionError::Io variant"),
        }
    }
}
