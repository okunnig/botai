pub mod scanner;

pub use scanner::{StreamControlEvent, StreamScanner};

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
    /// Custom notification from control sequences (OSC 9 / OSC 777).
    Custom {
        project_name: String,
        title: Option<String>,
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
            Self::Custom {
                project_name,
                title,
                ..
            } => match title {
                Some(t) if !t.trim().is_empty() => format!("botai: [{project_name}] {t}"),
                _ => format!("botai: [{project_name}] Notification"),
            },
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
            Self::Custom { message, .. } => message.clone(),
        }
    }

    /// Returns the associated project name.
    pub fn project_name(&self) -> &str {
        match self {
            Self::Completed { project_name, .. }
            | Self::WaitingForInput { project_name }
            | Self::Error { project_name, .. }
            | Self::Custom { project_name, .. } => project_name,
        }
    }

    /// Sets or updates the associated project name.
    pub fn set_project_name(&mut self, name: impl Into<String>) {
        let name = name.into();
        match self {
            Self::Completed { project_name, .. }
            | Self::WaitingForInput { project_name }
            | Self::Error { project_name, .. }
            | Self::Custom { project_name, .. } => *project_name = name,
        }
    }

    /// Returns true if this event indicates the session is waiting for user input or approval.
    pub fn is_waiting(&self) -> bool {
        matches!(self, Self::WaitingForInput { .. })
    }
}

/// Activity detector tracking stream activity, control sequences, and computing session status transitions.
#[derive(Debug, Clone)]
pub struct ActivityTracker {
    last_activity: Instant,
    last_status: SessionStatus,
    idle_threshold: Duration,
    scanner: StreamScanner,
}

impl Default for ActivityTracker {
    fn default() -> Self {
        Self::new(Duration::from_millis(1500))
    }
}

impl ActivityTracker {
    /// Creates a new `ActivityTracker` with the specified idle threshold.
    pub fn new(idle_threshold: Duration) -> Self {
        Self {
            last_activity: Instant::now(),
            last_status: SessionStatus::Initializing,
            idle_threshold,
            scanner: StreamScanner::new(),
        }
    }

    /// Records that bytes were received or sent, marking the session as busy.
    pub fn record_activity(&mut self) -> SessionStatus {
        self.last_activity = Instant::now();
        self.last_status = SessionStatus::Busy;
        SessionStatus::Busy
    }

    /// Processes incoming stream byte chunks, scanning for control sequences and updating status.
    ///
    /// Returns any notification events triggered by control sequences (BEL, OSC 9, OSC 777, OSC 133).
    pub fn process_bytes(&mut self, bytes: &[u8]) -> Vec<NotificationEvent> {
        if bytes.is_empty() {
            return Vec::new();
        }

        self.last_activity = Instant::now();
        let stream_events = self.scanner.scan(bytes);

        if stream_events.is_empty() {
            if self.last_status != SessionStatus::Done && self.last_status != SessionStatus::Error {
                self.last_status = SessionStatus::Busy;
            }
            return Vec::new();
        }

        let mut notifs = Vec::new();

        for ev in stream_events {
            match ev {
                StreamControlEvent::Bell => {
                    self.last_status = SessionStatus::WaitingForInput;
                    notifs.push(NotificationEvent::WaitingForInput {
                        project_name: String::new(),
                    });
                }
                StreamControlEvent::Osc9(msg) => {
                    notifs.push(NotificationEvent::Custom {
                        project_name: String::new(),
                        title: None,
                        message: msg,
                    });
                }
                StreamControlEvent::Osc777 { title, message } => {
                    notifs.push(NotificationEvent::Custom {
                        project_name: String::new(),
                        title: Some(title),
                        message,
                    });
                }
                StreamControlEvent::PromptStart => {
                    self.last_status = SessionStatus::WaitingForInput;
                    notifs.push(NotificationEvent::WaitingForInput {
                        project_name: String::new(),
                    });
                }
                StreamControlEvent::CommandStart | StreamControlEvent::CommandOutput => {
                    self.last_status = SessionStatus::Busy;
                }
                StreamControlEvent::CommandFinished { exit_code } => {
                    if exit_code == 0 {
                        self.last_status = SessionStatus::Done;
                        notifs.push(NotificationEvent::Completed {
                            project_name: String::new(),
                            exit_code: Some(0),
                        });
                    } else {
                        self.last_status = SessionStatus::Error;
                        notifs.push(NotificationEvent::Error {
                            project_name: String::new(),
                            message: format!("Process exited with status code {exit_code}"),
                        });
                    }
                }
            }
        }

        notifs
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
                Some(0) => {
                    let notif = if self.last_status != SessionStatus::Done {
                        Some(NotificationEvent::Completed {
                            project_name: String::new(),
                            exit_code: Some(0),
                        })
                    } else {
                        None
                    };
                    (SessionStatus::Done, notif)
                }
                Some(code) => {
                    let notif = if self.last_status != SessionStatus::Error {
                        Some(NotificationEvent::Error {
                            project_name: String::new(),
                            message: format!("Process exited with status code {code}"),
                        })
                    } else {
                        None
                    };
                    (SessionStatus::Error, notif)
                }
                None => {
                    let notif = if self.last_status != SessionStatus::Error {
                        Some(NotificationEvent::Error {
                            project_name: String::new(),
                            message: "Process terminated unexpectedly".to_string(),
                        })
                    } else {
                        None
                    };
                    (SessionStatus::Error, notif)
                }
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

    /// Sets the status directly.
    pub fn set_status(&mut self, status: SessionStatus) {
        self.last_status = status;
    }

    /// Returns the timestamp of the last recorded activity.
    pub fn last_activity(&self) -> Instant {
        self.last_activity
    }

    /// Sets the last activity timestamp (useful for testing silence thresholds).
    pub fn set_last_activity(&mut self, time: Instant) {
        self.last_activity = time;
    }

    /// Returns the configured idle threshold.
    pub fn idle_threshold(&self) -> Duration {
        self.idle_threshold
    }

    /// Sets the idle threshold.
    pub fn set_idle_threshold(&mut self, threshold: Duration) {
        self.idle_threshold = threshold;
    }

    /// Returns a reference to the stream scanner.
    pub fn scanner(&self) -> &StreamScanner {
        &self.scanner
    }

    /// Returns a mutable reference to the stream scanner.
    pub fn scanner_mut(&mut self) -> &mut StreamScanner {
        &mut self.scanner
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
        assert!(!completed.is_waiting());

        let waiting = NotificationEvent::WaitingForInput {
            project_name: "frontend-ui".to_string(),
        };
        assert_eq!(waiting.title(), "botai: [frontend-ui] Awaiting Input");
        assert!(waiting.body().contains("AI agent is waiting"));
        assert!(waiting.is_waiting());

        let err = NotificationEvent::Error {
            project_name: "db-migration".to_string(),
            message: "Fatal: connection refused".to_string(),
        };
        assert_eq!(err.title(), "botai: [db-migration] Error");
        assert_eq!(err.body(), "Fatal: connection refused");
        assert!(!err.is_waiting());

        let custom = NotificationEvent::Custom {
            project_name: "api-service".to_string(),
            title: Some("Deploy Ready".to_string()),
            message: "v1.2.3 deployed".to_string(),
        };
        assert_eq!(custom.title(), "botai: [api-service] Deploy Ready");
        assert_eq!(custom.body(), "v1.2.3 deployed");
        assert_eq!(custom.project_name(), "api-service");
        assert!(!custom.is_waiting());

        let custom_no_title = NotificationEvent::Custom {
            project_name: "api-service".to_string(),
            title: None,
            message: "General update".to_string(),
        };
        assert_eq!(custom_no_title.title(), "botai: [api-service] Notification");
        assert_eq!(custom_no_title.body(), "General update");
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
    fn test_activity_tracker_process_bytes_bell() {
        let mut tracker = ActivityTracker::new(Duration::from_millis(1000));
        let notifs = tracker.process_bytes(b"Analyzing repository...\x07");

        assert_eq!(tracker.status(), SessionStatus::WaitingForInput);
        assert_eq!(
            notifs,
            vec![NotificationEvent::WaitingForInput {
                project_name: String::new()
            }]
        );
    }

    #[test]
    fn test_activity_tracker_process_bytes_osc_9() {
        let mut tracker = ActivityTracker::new(Duration::from_millis(1000));
        let notifs = tracker.process_bytes(b"\x1b]9;Prompt review required\x07");

        assert_eq!(
            notifs,
            vec![NotificationEvent::Custom {
                project_name: String::new(),
                title: None,
                message: "Prompt review required".to_string(),
            }]
        );
    }

    #[test]
    fn test_activity_tracker_process_bytes_osc_777() {
        let mut tracker = ActivityTracker::new(Duration::from_millis(1000));
        let notifs = tracker.process_bytes(b"\x1b]777;notify;Agent Done;All tasks executed\x1b\\");

        assert_eq!(
            notifs,
            vec![NotificationEvent::Custom {
                project_name: String::new(),
                title: Some("Agent Done".to_string()),
                message: "All tasks executed".to_string(),
            }]
        );
    }

    #[test]
    fn test_activity_tracker_process_bytes_osc_133_lifecycle() {
        let mut tracker = ActivityTracker::new(Duration::from_millis(1000));

        // Command start
        let notifs = tracker.process_bytes(b"\x1b]133;B\x07");
        assert_eq!(tracker.status(), SessionStatus::Busy);
        assert!(notifs.is_empty());

        // Command output
        let notifs = tracker.process_bytes(b"\x1b]133;C\x07Running compilation...\n");
        assert_eq!(tracker.status(), SessionStatus::Busy);
        assert!(notifs.is_empty());

        // Command finish exit 0
        let notifs = tracker.process_bytes(b"\x1b]133;D;0\x07");
        assert_eq!(tracker.status(), SessionStatus::Done);
        assert_eq!(
            notifs,
            vec![NotificationEvent::Completed {
                project_name: String::new(),
                exit_code: Some(0),
            }]
        );

        // Prompt start (ready for input)
        let notifs = tracker.process_bytes(b"\x1b]133;A\x07");
        assert_eq!(tracker.status(), SessionStatus::WaitingForInput);
        assert_eq!(
            notifs,
            vec![NotificationEvent::WaitingForInput {
                project_name: String::new()
            }]
        );
    }

    #[test]
    fn test_activity_tracker_isolation_between_sessions() {
        let mut tracker1 = ActivityTracker::new(Duration::from_millis(100));
        let mut tracker2 = ActivityTracker::new(Duration::from_millis(100));

        // Session 1 receives activity and becomes Busy
        tracker1.record_activity();
        assert_eq!(tracker1.status(), SessionStatus::Busy);
        assert_eq!(tracker2.status(), SessionStatus::Initializing);

        // Session 2 receives BEL and becomes WaitingForInput
        let notifs2 = tracker2.process_bytes(b"\x07");
        assert_eq!(tracker2.status(), SessionStatus::WaitingForInput);
        assert_eq!(notifs2.len(), 1);
        assert_eq!(tracker1.status(), SessionStatus::Busy);
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
