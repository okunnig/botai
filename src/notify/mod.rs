use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::session::SessionStatus;
use thiserror::Error;

/// Notification system error variants.
#[derive(Debug, Error)]
pub enum NotificationError {
    #[error("Failed to deliver notification: {0}")]
    DeliveryFailed(String),
    #[error("Notification service unavailable")]
    ServiceUnavailable,
}

/// Notification trigger event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotificationEvent {
    /// Agent finished executing its prompt/command.
    Completed {
        project_name: String,
        exit_code: Option<i32>,
    },
    /// Agent is idle and waiting for user input or approval.
    WaitingForInput { project_name: String },
    /// Agent encountered an error or crashed.
    Error {
        project_name: String,
        message: String,
    },
}

impl NotificationEvent {
    /// Generates the title for the notification.
    pub fn title(&self) -> String {
        match self {
            Self::Completed { project_name, .. } => format!("botai: [{project_name}] Completed"),
            Self::WaitingForInput { project_name } => {
                format!("botai: [{project_name}] Awaiting Input")
            }
            Self::Error { project_name, .. } => format!("botai: [{project_name}] Error"),
        }
    }

    /// Generates the body message for the notification.
    pub fn body(&self) -> String {
        match self {
            Self::Completed { exit_code, .. } => match exit_code {
                Some(code) => format!("Task finished with exit code {code}."),
                None => "Task finished successfully.".to_string(),
            },
            Self::WaitingForInput { .. } => {
                "AI agent is waiting for your response or confirmation.".to_string()
            }
            Self::Error { message, .. } => message.clone(),
        }
    }

    /// Returns the associated project name.
    pub fn project_name(&self) -> &str {
        match self {
            Self::Completed { project_name, .. }
            | Self::WaitingForInput { project_name }
            | Self::Error { project_name, .. } => project_name,
        }
    }
}

/// Activity detector tracking stream activity and computing session status transitions.
#[derive(Debug, Clone)]
pub struct ActivityTracker {
    last_activity: Instant,
    last_status: SessionStatus,
    idle_threshold: Duration,
}

impl ActivityTracker {
    /// Creates a new `ActivityTracker` with default or custom idle threshold.
    pub fn new(idle_threshold: Duration) -> Self {
        Self {
            last_activity: Instant::now(),
            last_status: SessionStatus::Initializing,
            idle_threshold,
        }
    }

    /// Records that bytes were received from the PTY stream.
    pub fn record_activity(&mut self) -> SessionStatus {
        self.last_activity = Instant::now();
        self.last_status = SessionStatus::Busy;
        SessionStatus::Busy
    }

    /// Evaluates current status given elapsed time since last activity and child exit status.
    pub fn evaluate(
        &mut self,
        is_alive: bool,
        exit_code: Option<i32>,
        now: Instant,
    ) -> (SessionStatus, Option<NotificationEvent>, &str) {
        let (status, notif) = if !is_alive {
            match exit_code {
                Some(0) | None => (
                    SessionStatus::Done,
                    Some(NotificationEvent::Completed {
                        project_name: String::new(),
                        exit_code,
                    }),
                ),
                Some(code) => (
                    SessionStatus::Error,
                    Some(NotificationEvent::Error {
                        project_name: String::new(),
                        message: format!("Process exited with status code {code}"),
                    }),
                ),
            }
        } else if self.last_status == SessionStatus::Busy
            && now.saturating_duration_since(self.last_activity) >= self.idle_threshold
        {
            (
                SessionStatus::WaitingForInput,
                Some(NotificationEvent::WaitingForInput {
                    project_name: String::new(),
                }),
            )
        } else {
            (self.last_status, None)
        };

        self.last_status = status;
        (status, notif, "")
    }

    /// Returns the current known status.
    pub fn status(&self) -> SessionStatus {
        self.last_status
    }
}

/// Notification dispatcher with debouncing per project and event type.
pub struct NotificationService {
    enabled: bool,
    cooldown: Duration,
    last_notified: HashMap<String, Instant>,
}

impl NotificationService {
    /// Creates a new `NotificationService`.
    pub fn new(enabled: bool, cooldown: Duration) -> Self {
        Self {
            enabled,
            cooldown,
            last_notified: HashMap::new(),
        }
    }

    /// Attempts to dispatch a desktop notification if not within cooldown.
    pub fn notify(&mut self, event: &NotificationEvent) -> Result<bool, NotificationError> {
        if !self.enabled {
            return Ok(false);
        }

        let key = format!(
            "{}:{:?}",
            event.project_name(),
            std::mem::discriminant(event)
        );
        let now = Instant::now();

        if let Some(last_time) = self.last_notified.get(&key) {
            if now.duration_since(*last_time) < self.cooldown {
                return Ok(false); // Debounced
            }
        }

        self.last_notified.insert(key, now);

        // Attempt delivery via notify-rust (gracefully handle headless/mock environments)
        #[cfg(not(test))]
        {
            let _ = notify_rust::Notification::new()
                .summary(&event.title())
                .body(&event.body())
                .appname("botai")
                .timeout(notify_rust::Timeout::Milliseconds(5000))
                .show();
        }

        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_notification_event_formatting() {
        let completed = NotificationEvent::Completed {
            project_name: "auth-service".to_string(),
            exit_code: Some(0),
        };
        assert_eq!(completed.title(), "botai: [auth-service] Completed");
        assert_eq!(completed.body(), "Task finished with exit code 0.");
        assert_eq!(completed.project_name(), "auth-service");

        let waiting = NotificationEvent::WaitingForInput {
            project_name: "frontend-ui".to_string(),
        };
        assert_eq!(waiting.title(), "botai: [frontend-ui] Awaiting Input");
        assert!(waiting.body().contains("AI agent is waiting"));

        let err = NotificationEvent::Error {
            project_name: "db-migration".to_string(),
            message: "Fatal: connection refused".to_string(),
        };
        assert_eq!(err.title(), "botai: [db-migration] Error");
        assert_eq!(err.body(), "Fatal: connection refused");
    }

    #[test]
    fn test_activity_tracker_state_transitions() {
        let mut tracker = ActivityTracker::new(Duration::from_millis(100));
        assert_eq!(tracker.status(), SessionStatus::Initializing);

        // Activity received -> Busy
        let status = tracker.record_activity();
        assert_eq!(status, SessionStatus::Busy);
        assert_eq!(tracker.status(), SessionStatus::Busy);

        // Within idle threshold -> Still Busy
        let start = Instant::now();
        let (status, notif, _) = tracker.evaluate(true, None, start);
        assert_eq!(status, SessionStatus::Busy);
        assert!(notif.is_none());

        // Past idle threshold -> WaitingForInput
        let past_threshold = start + Duration::from_millis(150);
        let (status, notif, _) = tracker.evaluate(true, None, past_threshold);
        assert_eq!(status, SessionStatus::WaitingForInput);
        assert!(matches!(
            notif,
            Some(NotificationEvent::WaitingForInput { .. })
        ));

        // Process terminated cleanly -> Done
        let (status, notif, _) = tracker.evaluate(false, Some(0), past_threshold);
        assert_eq!(status, SessionStatus::Done);
        assert!(matches!(
            notif,
            Some(NotificationEvent::Completed {
                exit_code: Some(0),
                ..
            })
        ));

        // Process crashed -> Error
        let mut crash_tracker = ActivityTracker::new(Duration::from_millis(100));
        crash_tracker.record_activity();
        let (status, notif, _) = crash_tracker.evaluate(false, Some(1), past_threshold);
        assert_eq!(status, SessionStatus::Error);
        assert!(matches!(notif, Some(NotificationEvent::Error { .. })));
    }

    #[test]
    fn test_notification_service_debouncing() {
        let mut service = NotificationService::new(true, Duration::from_millis(200));

        let event = NotificationEvent::WaitingForInput {
            project_name: "test-proj".to_string(),
        };

        // First notification sends
        let sent = service.notify(&event).unwrap();
        assert!(sent);

        // Immediate subsequent notification is debounced
        let debounced = service.notify(&event).unwrap();
        assert!(!debounced);

        // Disabled notification service sends nothing
        let mut disabled_service = NotificationService::new(false, Duration::from_millis(200));
        let not_sent = disabled_service.notify(&event).unwrap();
        assert!(!not_sent);
    }
}
