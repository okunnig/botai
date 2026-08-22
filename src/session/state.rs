use ratatui::style::Color;
use serde::{Deserialize, Serialize};
use std::time::SystemTime;

/// Real-time lifecycle status of an AI agent CLI process session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionStatus {
    /// The session is being set up (worktree creation, process spawning).
    Initializing,
    /// The agent is actively executing, thinking, or streaming tool output (●).
    Busy,
    /// The agent is idle and waiting for user response, permission, or prompt input (●).
    WaitingForInput,
    /// The agent has successfully finished its task or initial prompt (●).
    Done,
    /// The agent encountered a non-zero exit code or execution error (●).
    Error,
    /// The session was manually closed, detached, or killed.
    Terminated,
}

impl SessionStatus {
    /// Returns the solid round dot status symbol indicator (●).
    pub fn status_symbol(&self) -> &'static str {
        "●"
    }

    /// Returns the badge icon representing this status in the TUI status bar.
    pub fn badge_icon(&self) -> &'static str {
        self.status_symbol()
    }

    /// Returns the contextual theme color associated with this status.
    pub fn status_color(&self, theme: &crate::tui::Theme) -> Color {
        match self {
            SessionStatus::Busy => theme.status_busy,
            SessionStatus::WaitingForInput => theme.status_waiting,
            SessionStatus::Done => theme.status_done,
            SessionStatus::Error => theme.status_error,
            SessionStatus::Initializing | SessionStatus::Terminated => theme.status_bar_fg,
        }
    }

    /// Returns a human-readable badge text / label.
    pub fn badge_text(&self) -> &'static str {
        match self {
            SessionStatus::Initializing => "Initializing",
            SessionStatus::Busy => "Thinking",
            SessionStatus::WaitingForInput => "Waiting for Input",
            SessionStatus::Done => "Done",
            SessionStatus::Error => "Error",
            SessionStatus::Terminated => "Terminated",
        }
    }

    /// Returns true if the session is currently active and can receive input/output.
    pub fn is_active(&self) -> bool {
        matches!(
            self,
            SessionStatus::Initializing | SessionStatus::Busy | SessionStatus::WaitingForInput
        )
    }

    /// Returns true if the session has concluded.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            SessionStatus::Done | SessionStatus::Error | SessionStatus::Terminated
        )
    }
}

impl std::fmt::Display for SessionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.badge_text())
    }
}

/// Metadata and state tracking for a project session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectSession {
    /// Unique identifier for this session.
    pub id: String,
    /// Name of the project (e.g. branch name and working directory).
    pub project_name: String,
    /// Name of the template this project belongs to.
    pub template_name: String,
    /// Name of the agent CLI tool being used (e.g., "claude", "agy").
    pub agent_name: String,
    /// Current lifecycle status.
    pub status: SessionStatus,
    /// Timestamp when the session was created.
    pub created_at: SystemTime,
    /// Timestamp of the last status change or activity.
    pub updated_at: SystemTime,
    /// Optional initial prompt supplied to the agent.
    pub prompt: Option<String>,
    /// Exit code if the process has completed.
    pub exit_code: Option<u32>,
}

/// Type alias for `ProjectSession` for compatibility.
pub type SessionState = ProjectSession;

impl ProjectSession {
    /// Creates a new `ProjectSession` in the `Initializing` state.
    pub fn new(
        project_name: impl Into<String>,
        template_name: impl Into<String>,
        agent_name: impl Into<String>,
    ) -> Self {
        let p_name = project_name.into();
        let t_name = template_name.into();
        let a_name = agent_name.into();
        let now = SystemTime::now();
        let id = format!("{t_name}:{p_name}:{a_name}");

        Self {
            id,
            project_name: p_name,
            template_name: t_name,
            agent_name: a_name,
            status: SessionStatus::Initializing,
            created_at: now,
            updated_at: now,
            prompt: None,
            exit_code: None,
        }
    }

    /// Backward compatibility helper for workspace_name.
    #[inline]
    pub fn workspace_name(&self) -> &str {
        &self.template_name
    }

    /// Sets a custom session identifier.
    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.id = id.into();
        self
    }

    /// Sets the initial prompt text.
    pub fn with_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.prompt = Some(prompt.into());
        self
    }

    /// Updates the session status and refreshes `updated_at`.
    pub fn set_status(&mut self, status: SessionStatus) {
        self.status = status;
        self.updated_at = SystemTime::now();
    }

    /// Marks the session as busy / thinking.
    pub fn mark_busy(&mut self) {
        self.set_status(SessionStatus::Busy);
    }

    /// Marks the session as waiting for user input.
    pub fn mark_waiting_for_input(&mut self) {
        self.set_status(SessionStatus::WaitingForInput);
    }

    /// Marks the session as successfully completed (Done).
    pub fn mark_done(&mut self, exit_code: Option<u32>) {
        self.exit_code = exit_code;
        self.set_status(SessionStatus::Done);
    }

    /// Marks the session as failed or in error.
    pub fn mark_error(&mut self, exit_code: Option<u32>) {
        self.exit_code = exit_code;
        self.set_status(SessionStatus::Error);
    }

    /// Marks the session as terminated / killed.
    pub fn mark_terminated(&mut self) {
        self.set_status(SessionStatus::Terminated);
    }

    /// Returns whether the session is currently active.
    pub fn is_active(&self) -> bool {
        self.status.is_active()
    }

    /// Returns whether the session has concluded.
    pub fn is_terminal(&self) -> bool {
        self.status.is_terminal()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_status_properties_and_badges() {
        assert_eq!(SessionStatus::Initializing.status_symbol(), "●");
        assert_eq!(SessionStatus::Initializing.badge_icon(), "●");
        assert_eq!(SessionStatus::Initializing.badge_text(), "Initializing");
        assert!(SessionStatus::Initializing.is_active());
        assert!(!SessionStatus::Initializing.is_terminal());

        assert_eq!(SessionStatus::Busy.status_symbol(), "●");
        assert_eq!(SessionStatus::Busy.badge_icon(), "●");
        assert_eq!(SessionStatus::Busy.badge_text(), "Thinking");
        assert!(SessionStatus::Busy.is_active());
        assert!(!SessionStatus::Busy.is_terminal());

        assert_eq!(SessionStatus::WaitingForInput.status_symbol(), "●");
        assert_eq!(SessionStatus::WaitingForInput.badge_icon(), "●");
        assert_eq!(
            SessionStatus::WaitingForInput.badge_text(),
            "Waiting for Input"
        );
        assert!(SessionStatus::WaitingForInput.is_active());
        assert!(!SessionStatus::WaitingForInput.is_terminal());

        assert_eq!(SessionStatus::Done.status_symbol(), "●");
        assert_eq!(SessionStatus::Done.badge_icon(), "●");
        assert_eq!(SessionStatus::Done.badge_text(), "Done");
        assert!(!SessionStatus::Done.is_active());
        assert!(SessionStatus::Done.is_terminal());

        assert_eq!(SessionStatus::Error.status_symbol(), "●");
        assert_eq!(SessionStatus::Error.badge_icon(), "●");
        assert_eq!(SessionStatus::Error.badge_text(), "Error");
        assert!(!SessionStatus::Error.is_active());
        assert!(SessionStatus::Error.is_terminal());

        assert_eq!(SessionStatus::Terminated.status_symbol(), "●");
        assert_eq!(SessionStatus::Terminated.badge_icon(), "●");
        assert_eq!(SessionStatus::Terminated.badge_text(), "Terminated");
        assert!(!SessionStatus::Terminated.is_active());
        assert!(SessionStatus::Terminated.is_terminal());

        // Test Display implementation
        assert_eq!(SessionStatus::Initializing.to_string(), "Initializing");
        assert_eq!(SessionStatus::Busy.to_string(), "Thinking");
        assert_eq!(
            SessionStatus::WaitingForInput.to_string(),
            "Waiting for Input"
        );
        assert_eq!(SessionStatus::Done.to_string(), "Done");
        assert_eq!(SessionStatus::Error.to_string(), "Error");
        assert_eq!(SessionStatus::Terminated.to_string(), "Terminated");
    }

    #[test]
    fn test_session_status_colors_across_themes() {
        use crate::tui::Theme;

        let themes = [
            Theme::tokyo_night(),
            Theme::catppuccin(),
            Theme::cyberpunk(),
            Theme::gruvbox(),
            Theme::monochrome(),
            Theme::ansi(),
        ];

        for theme in &themes {
            assert_eq!(
                SessionStatus::Busy.status_color(theme),
                theme.status_busy,
                "Busy status color mismatch for theme {}",
                theme.name
            );
            assert_eq!(
                SessionStatus::WaitingForInput.status_color(theme),
                theme.status_waiting,
                "Waiting status color mismatch for theme {}",
                theme.name
            );
            assert_eq!(
                SessionStatus::Done.status_color(theme),
                theme.status_done,
                "Done status color mismatch for theme {}",
                theme.name
            );
            assert_eq!(
                SessionStatus::Error.status_color(theme),
                theme.status_error,
                "Error status color mismatch for theme {}",
                theme.name
            );
            assert_eq!(
                SessionStatus::Initializing.status_color(theme),
                theme.status_bar_fg,
                "Initializing status color mismatch for theme {}",
                theme.name
            );
            assert_eq!(
                SessionStatus::Terminated.status_color(theme),
                theme.status_bar_fg,
                "Terminated status color mismatch for theme {}",
                theme.name
            );
        }
    }

    #[test]
    fn test_project_session_lifecycle_transitions() {
        let mut session = ProjectSession::new("feat-checkout", "ecommerce", "claude")
            .with_prompt("Fix checkout button bug");

        assert_eq!(session.project_name, "feat-checkout");
        assert_eq!(session.template_name, "ecommerce");
        assert_eq!(session.agent_name, "claude");
        assert_eq!(session.status, SessionStatus::Initializing);
        assert_eq!(session.prompt.as_deref(), Some("Fix checkout button bug"));
        assert!(session.is_active());
        assert!(!session.is_terminal());
        assert_eq!(session.exit_code, None);

        // Transition to busy / thinking
        session.mark_busy();
        assert_eq!(session.status, SessionStatus::Busy);
        assert!(session.is_active());

        // Transition to waiting for input
        session.mark_waiting_for_input();
        assert_eq!(session.status, SessionStatus::WaitingForInput);
        assert!(session.is_active());

        // Transition to busy again
        session.mark_busy();
        assert_eq!(session.status, SessionStatus::Busy);

        // Transition to Done with exit code 0
        session.mark_done(Some(0));
        assert_eq!(session.status, SessionStatus::Done);
        assert_eq!(session.exit_code, Some(0));
        assert!(!session.is_active());
        assert!(session.is_terminal());
    }

    #[test]
    fn test_project_session_error_and_terminated_transitions() {
        let mut session = ProjectSession::new("bugfix-api", "backend", "agy");
        session.mark_busy();
        session.mark_error(Some(1));

        assert_eq!(session.status, SessionStatus::Error);
        assert_eq!(session.exit_code, Some(1));
        assert!(session.is_terminal());

        let mut killed_session = ProjectSession::new("test-proj", "ws", "copilot");
        killed_session.mark_terminated();
        assert_eq!(killed_session.status, SessionStatus::Terminated);
        assert!(killed_session.is_terminal());
    }

    #[test]
    fn test_project_session_custom_id_and_special_characters() {
        let session = ProjectSession::new("feature/auth-2.0 🚀", "ws-cloud", "claude")
            .with_id("custom-uuid-12345");

        assert_eq!(session.id, "custom-uuid-12345");
        assert_eq!(session.project_name, "feature/auth-2.0 🚀");
        assert_eq!(session.template_name, "ws-cloud");
    }
}
