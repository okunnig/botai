use super::{ProjectSession, PtySession, SessionError, SessionStatus, VtScreen};
use crate::notify::ActivityTracker;

/// A single active session entry in the session registry, holding state, PTY handle, virtual terminal, and activity tracker.
pub struct SessionEntry {
    /// Metadata and lifecycle state for the project session.
    pub project_session: ProjectSession,
    /// Handle to the underlying PTY process.
    pub pty: PtySession,
    /// In-memory virtual terminal screen parser.
    pub vt: VtScreen,
    /// Isolated activity tracker and stream scanner for this session.
    pub tracker: ActivityTracker,
}

impl SessionEntry {
    /// Creates a new `SessionEntry`.
    pub fn new(project_session: ProjectSession, pty: PtySession, vt: VtScreen) -> Self {
        Self {
            project_session,
            pty,
            vt,
            tracker: ActivityTracker::default(),
        }
    }

    /// Sets a custom activity tracker on this session entry.
    pub fn with_tracker(mut self, tracker: ActivityTracker) -> Self {
        self.tracker = tracker;
        self
    }

    /// Returns the session ID.
    pub fn id(&self) -> &str {
        &self.project_session.id
    }

    /// Returns the project name.
    pub fn project_name(&self) -> &str {
        &self.project_session.project_name
    }

    /// Returns the template name.
    pub fn template_name(&self) -> &str {
        &self.project_session.template_name
    }

    /// Backward compatibility alias for template_name.
    pub fn workspace_name(&self) -> &str {
        self.template_name()
    }

    /// Returns the agent CLI tool name.
    pub fn agent_name(&self) -> &str {
        &self.project_session.agent_name
    }

    /// Returns the current lifecycle status.
    pub fn status(&self) -> SessionStatus {
        self.project_session.status
    }

    /// Returns a reference to the virtual terminal screen.
    pub fn vt(&self) -> &VtScreen {
        &self.vt
    }

    /// Returns a mutable reference to the virtual terminal screen.
    pub fn vt_mut(&mut self) -> &mut VtScreen {
        &mut self.vt
    }

    /// Returns a reference to the PTY session.
    pub fn pty(&self) -> &PtySession {
        &self.pty
    }

    /// Returns a mutable reference to the PTY session.
    pub fn pty_mut(&mut self) -> &mut PtySession {
        &mut self.pty
    }

    /// Returns a reference to the session's activity tracker.
    pub fn tracker(&self) -> &ActivityTracker {
        &self.tracker
    }

    /// Returns a mutable reference to the session's activity tracker.
    pub fn tracker_mut(&mut self) -> &mut ActivityTracker {
        &mut self.tracker
    }
}

/// Registry managing concurrent AI agent project sessions, active focus, and navigation.
pub struct SessionRegistry {
    sessions: Vec<SessionEntry>,
    active_index: Option<usize>,
}

impl Default for SessionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionRegistry {
    /// Creates a new empty `SessionRegistry`.
    pub fn new() -> Self {
        Self {
            sessions: Vec::new(),
            active_index: None,
        }
    }

    /// Adds a new session to the registry.
    ///
    /// If no active session was set, sets the newly added session as active.
    /// Returns an error if a session with the same ID already exists.
    pub fn add(
        &mut self,
        project_session: ProjectSession,
        pty: PtySession,
        vt: VtScreen,
    ) -> Result<String, SessionError> {
        if self
            .sessions
            .iter()
            .any(|s| s.project_session.id == project_session.id)
        {
            return Err(SessionError::Other(format!(
                "Session with ID '{}' already exists in registry",
                project_session.id
            )));
        }

        let id = project_session.id.clone();
        let entry = SessionEntry::new(project_session, pty, vt);
        self.sessions.push(entry);

        if self.active_index.is_none() {
            self.active_index = Some(0);
        }

        Ok(id)
    }

    /// Looks up a session by unique ID.
    pub fn get(&self, id: &str) -> Option<&SessionEntry> {
        self.sessions.iter().find(|s| s.project_session.id == id)
    }

    /// Looks up a mutable session by unique ID.
    pub fn get_mut(&mut self, id: &str) -> Option<&mut SessionEntry> {
        self.sessions
            .iter_mut()
            .find(|s| s.project_session.id == id)
    }

    /// Looks up a session by project name.
    pub fn get_by_project(&self, name: &str) -> Option<&SessionEntry> {
        self.sessions
            .iter()
            .find(|s| s.project_session.project_name == name)
    }

    /// Looks up a mutable session by project name.
    pub fn get_by_project_mut(&mut self, name: &str) -> Option<&mut SessionEntry> {
        self.sessions
            .iter_mut()
            .find(|s| s.project_session.project_name == name)
    }

    /// Returns a list of references to all sessions in insertion order.
    pub fn list(&self) -> Vec<&SessionEntry> {
        self.sessions.iter().collect()
    }

    /// Returns a mutable list of references to all sessions.
    pub fn list_mut(&mut self) -> Vec<&mut SessionEntry> {
        self.sessions.iter_mut().collect()
    }

    /// Removes a session by ID and adjusts the active index accordingly.
    pub fn remove(&mut self, id: &str) -> Result<Option<SessionEntry>, SessionError> {
        if let Some(pos) = self
            .sessions
            .iter()
            .position(|s| s.project_session.id == id)
        {
            let removed = self.sessions.remove(pos);
            if self.sessions.is_empty() {
                self.active_index = None;
            } else if let Some(current) = self.active_index {
                if current >= self.sessions.len() {
                    self.active_index = Some(self.sessions.len() - 1);
                } else if pos < current {
                    self.active_index = Some(current - 1);
                }
            }
            Ok(Some(removed))
        } else {
            Ok(None)
        }
    }

    /// Returns the 0-based index of the currently active session, if any.
    pub fn active_index(&self) -> Option<usize> {
        self.active_index
    }

    /// Returns a reference to the active session entry.
    pub fn active_session(&self) -> Option<&SessionEntry> {
        let idx = self.active_index?;
        self.sessions.get(idx)
    }

    /// Returns a mutable reference to the active session entry.
    pub fn active_session_mut(&mut self) -> Option<&mut SessionEntry> {
        let idx = self.active_index?;
        self.sessions.get_mut(idx)
    }

    /// Sets the active session index (0-based).
    pub fn set_active_index(&mut self, index: usize) -> Result<(), SessionError> {
        if self.sessions.is_empty() {
            return Err(SessionError::InvalidSessionIndex(index));
        }
        if index < self.sessions.len() {
            self.active_index = Some(index);
            Ok(())
        } else {
            Err(SessionError::InvalidSessionIndex(index))
        }
    }

    /// Cycles focus to the next session (wrapping around to the first).
    pub fn next_session(&mut self) {
        if self.sessions.is_empty() {
            self.active_index = None;
            return;
        }
        let current = self.active_index.unwrap_or(0);
        self.active_index = Some((current + 1) % self.sessions.len());
    }

    /// Cycles focus to the previous session (wrapping around to the last).
    pub fn prev_session(&mut self) {
        if self.sessions.is_empty() {
            self.active_index = None;
            return;
        }
        let current = self.active_index.unwrap_or(0);
        self.active_index = Some((current + self.sessions.len() - 1) % self.sessions.len());
    }

    /// Switches focus by 1-indexed session number (`1..=9`).
    pub fn switch_to_session_number(&mut self, num: usize) {
        if num >= 1 && num <= self.sessions.len() {
            self.active_index = Some(num - 1);
        }
    }

    /// Returns the number of registered sessions.
    pub fn len(&self) -> usize {
        self.sessions.len()
    }

    /// Returns true if the registry contains no sessions.
    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }

    /// Resizes both the PTY and virtual terminal grid for all registered sessions.
    pub fn resize_all(&mut self, cols: u16, rows: u16) {
        for session in &mut self.sessions {
            let _ = session.pty_mut().resize(cols, rows);
            session.vt_mut().resize(cols, rows);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::PtyConfig;
    use tempfile::tempdir;

    fn create_test_session(
        project_name: &str,
        workspace: &str,
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
        let ps = ProjectSession::new(project_name, workspace, agent);

        (ps, pty, vt)
    }

    #[test]
    fn test_session_registry_new_empty() {
        let registry = SessionRegistry::new();
        assert_eq!(registry.len(), 0);
        assert!(registry.is_empty());
        assert_eq!(registry.active_index(), None);
        assert!(registry.active_session().is_none());
        assert!(registry.list().is_empty());
    }

    #[test]
    fn test_session_registry_add_and_get() {
        let mut registry = SessionRegistry::new();
        let (ps, pty, vt) = create_test_session("frontend-ui", "web-app", "claude");
        let expected_id = ps.id.clone();

        let id = registry.add(ps, pty, vt).expect("Failed to add session");
        assert_eq!(id, expected_id);
        assert_eq!(registry.len(), 1);
        assert!(!registry.is_empty());
        assert_eq!(registry.active_index(), Some(0));

        let session = registry.get(&id).expect("Session not found by ID");
        assert_eq!(session.project_name(), "frontend-ui");
        assert_eq!(session.template_name(), "web-app");
        assert_eq!(session.agent_name(), "claude");
        assert_eq!(session.id(), id);

        assert!(registry.get("nonexistent-id").is_none());
    }

    #[test]
    fn test_session_registry_get_by_project() {
        let mut registry = SessionRegistry::new();
        let (ps1, pty1, vt1) = create_test_session("proj-alpha", "ws1", "claude");
        let (ps2, pty2, vt2) = create_test_session("proj-beta", "ws1", "agy");

        registry.add(ps1, pty1, vt1).unwrap();
        registry.add(ps2, pty2, vt2).unwrap();

        assert_eq!(registry.len(), 2);

        let alpha = registry
            .get_by_project("proj-alpha")
            .expect("proj-alpha not found");
        assert_eq!(alpha.project_name(), "proj-alpha");
        assert_eq!(alpha.agent_name(), "claude");

        let beta = registry
            .get_by_project("proj-beta")
            .expect("proj-beta not found");
        assert_eq!(beta.project_name(), "proj-beta");
        assert_eq!(beta.agent_name(), "agy");

        assert!(registry.get_by_project("proj-gamma").is_none());
    }

    #[test]
    fn test_session_registry_duplicate_id_rejected() {
        let mut registry = SessionRegistry::new();
        let (ps1, pty1, vt1) = create_test_session("duplicate-p", "ws", "claude");
        let (ps2, pty2, vt2) = create_test_session("duplicate-p", "ws", "claude");

        let res1 = registry.add(ps1, pty1, vt1);
        assert!(res1.is_ok());

        let res2 = registry.add(ps2, pty2, vt2);
        assert!(res2.is_err());
    }

    #[test]
    fn test_session_registry_cycling_next_and_prev() {
        let mut registry = SessionRegistry::new();
        let (ps1, pty1, vt1) = create_test_session("p1", "ws", "claude");
        let (ps2, pty2, vt2) = create_test_session("p2", "ws", "agy");
        let (ps3, pty3, vt3) = create_test_session("p3", "ws", "copilot");

        registry.add(ps1, pty1, vt1).unwrap();
        registry.add(ps2, pty2, vt2).unwrap();
        registry.add(ps3, pty3, vt3).unwrap();

        assert_eq!(registry.active_index(), Some(0));
        assert_eq!(registry.active_session().unwrap().project_name(), "p1");

        // Next cycles 0 -> 1 -> 2 -> 0
        registry.next_session();
        assert_eq!(registry.active_index(), Some(1));
        assert_eq!(registry.active_session().unwrap().project_name(), "p2");

        registry.next_session();
        assert_eq!(registry.active_index(), Some(2));
        assert_eq!(registry.active_session().unwrap().project_name(), "p3");

        registry.next_session();
        assert_eq!(registry.active_index(), Some(0));
        assert_eq!(registry.active_session().unwrap().project_name(), "p1");

        // Prev cycles 0 -> 2 -> 1 -> 0
        registry.prev_session();
        assert_eq!(registry.active_index(), Some(2));
        assert_eq!(registry.active_session().unwrap().project_name(), "p3");

        registry.prev_session();
        assert_eq!(registry.active_index(), Some(1));
        assert_eq!(registry.active_session().unwrap().project_name(), "p2");

        registry.prev_session();
        assert_eq!(registry.active_index(), Some(0));
        assert_eq!(registry.active_session().unwrap().project_name(), "p1");
    }

    #[test]
    fn test_session_registry_switch_to_session_number_1_indexed() {
        let mut registry = SessionRegistry::new();
        let (ps1, pty1, vt1) = create_test_session("p1", "ws", "claude");
        let (ps2, pty2, vt2) = create_test_session("p2", "ws", "agy");
        let (ps3, pty3, vt3) = create_test_session("p3", "ws", "copilot");

        registry.add(ps1, pty1, vt1).unwrap();
        registry.add(ps2, pty2, vt2).unwrap();
        registry.add(ps3, pty3, vt3).unwrap();

        // Switch to session 3 (index 2)
        registry.switch_to_session_number(3);
        assert_eq!(registry.active_index(), Some(2));
        assert_eq!(registry.active_session().unwrap().project_name(), "p3");

        // Switch to session 1 (index 0)
        registry.switch_to_session_number(1);
        assert_eq!(registry.active_index(), Some(0));
        assert_eq!(registry.active_session().unwrap().project_name(), "p1");

        // Switch to session 2 (index 1)
        registry.switch_to_session_number(2);
        assert_eq!(registry.active_index(), Some(1));
        assert_eq!(registry.active_session().unwrap().project_name(), "p2");

        // Out of bounds: 0 or > 3 should be ignored (no change)
        registry.switch_to_session_number(0);
        assert_eq!(registry.active_index(), Some(1));

        registry.switch_to_session_number(4);
        assert_eq!(registry.active_index(), Some(1));
    }

    #[test]
    fn test_session_registry_set_active_index_boundaries() {
        let mut registry = SessionRegistry::new();
        assert!(registry.set_active_index(0).is_err());

        let (ps1, pty1, vt1) = create_test_session("p1", "ws", "claude");
        registry.add(ps1, pty1, vt1).unwrap();

        assert!(registry.set_active_index(0).is_ok());
        assert_eq!(registry.active_index(), Some(0));

        assert!(registry.set_active_index(1).is_err());
        assert_eq!(registry.active_index(), Some(0));
    }

    #[test]
    fn test_session_registry_remove_and_reindex() {
        let mut registry = SessionRegistry::new();
        let (ps1, pty1, vt1) = create_test_session("p1", "ws", "claude");
        let (ps2, pty2, vt2) = create_test_session("p2", "ws", "agy");
        let (ps3, pty3, vt3) = create_test_session("p3", "ws", "copilot");

        let id1 = registry.add(ps1, pty1, vt1).unwrap();
        let id2 = registry.add(ps2, pty2, vt2).unwrap();
        let id3 = registry.add(ps3, pty3, vt3).unwrap();

        // Active is 0 (p1). Set active to 2 (p3).
        registry.set_active_index(2).unwrap();
        assert_eq!(registry.active_index(), Some(2));

        // Remove p1 (at pos 0, before active 2). Active index should shift down to 1.
        let removed = registry.remove(&id1).unwrap();
        assert!(removed.is_some());
        assert_eq!(removed.unwrap().project_name(), "p1");
        assert_eq!(registry.len(), 2);
        assert_eq!(registry.active_index(), Some(1));
        assert_eq!(registry.active_session().unwrap().project_name(), "p3");

        // Remove active session (p3 at index 1). Active index should clamp to 0 (p2).
        let removed3 = registry.remove(&id3).unwrap();
        assert!(removed3.is_some());
        assert_eq!(registry.len(), 1);
        assert_eq!(registry.active_index(), Some(0));
        assert_eq!(registry.active_session().unwrap().project_name(), "p2");

        // Remove remaining session (p2). Active index becomes None.
        let removed2 = registry.remove(&id2).unwrap();
        assert!(removed2.is_some());
        assert_eq!(registry.len(), 0);
        assert_eq!(registry.active_index(), None);
        assert!(registry.active_session().is_none());

        // Removing non-existent ID returns Ok(None)
        let removed_none = registry.remove("unknown-id").unwrap();
        assert!(removed_none.is_none());
    }

    #[test]
    fn test_session_registry_get_mut_and_active_mut() {
        let mut registry = SessionRegistry::new();
        let (ps, pty, vt) = create_test_session("p1", "ws", "claude");
        let id = registry.add(ps, pty, vt).unwrap();

        if let Some(entry) = registry.get_mut(&id) {
            entry.project_session.mark_busy();
        }
        assert_eq!(registry.get(&id).unwrap().status(), SessionStatus::Busy);

        if let Some(active) = registry.active_session_mut() {
            active.project_session.mark_done(Some(0));
        }
        assert_eq!(registry.get(&id).unwrap().status(), SessionStatus::Done);
    }

    #[test]
    fn test_session_registry_resize_all() {
        let mut registry = SessionRegistry::new();
        let (ps1, pty1, vt1) = create_test_session("p1", "ws", "claude");
        let (ps2, pty2, vt2) = create_test_session("p2", "ws", "agy");

        registry.add(ps1, pty1, vt1).unwrap();
        registry.add(ps2, pty2, vt2).unwrap();

        assert_eq!(registry.sessions[0].vt().size(), (80, 24));
        assert_eq!(registry.sessions[1].vt().size(), (80, 24));

        registry.resize_all(140, 50);

        assert_eq!(registry.sessions[0].vt().size(), (140, 50));
        assert_eq!(registry.sessions[1].vt().size(), (140, 50));
    }
}
