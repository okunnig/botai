use crate::session::{SessionEntry, SessionRegistry};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Widget,
};

/// Status bar widget displaying active project tabs, agent status badges, and multiplexer hotkeys.
pub struct StatusBar<'a> {
    registry: &'a SessionRegistry,
}

impl<'a> StatusBar<'a> {
    /// Creates a new `StatusBar` widget backed by the given `SessionRegistry`.
    pub fn new(registry: &'a SessionRegistry) -> Self {
        Self { registry }
    }

    /// Formats a single tab title for a session entry with hotkey number and status badge.
    pub fn format_tab(index: usize, session: &SessionEntry, is_active: bool) -> String {
        let hotkey = if index < 9 {
            format!("{}", index + 1)
        } else {
            "-".to_string()
        };

        let badge = session.status().badge_icon();
        let proj = session.project_name();

        if is_active {
            format!(" [{}: {} {}] ", hotkey, proj, badge)
        } else {
            format!("  {}: {} {}  ", hotkey, proj, badge)
        }
    }

    /// Renders the status bar directly into a Ratatui `Buffer`.
    pub fn render_to_buffer(registry: &SessionRegistry, area: Rect, buf: &mut Buffer) {
        if area.height == 0 || area.width == 0 {
            return;
        }

        // Fill background
        let bg_style = Style::default().bg(Color::DarkGray).fg(Color::White);
        for x in area.left()..area.right() {
            for y in area.top()..area.bottom() {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_style(bg_style).set_char(' ');
                }
            }
        }

        let mut spans = Vec::new();

        if registry.is_empty() {
            spans.push(Span::styled(
                " [No Active Sessions] - Press Alt+Q to quit ",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ));
        } else {
            let active_idx = registry.active_index().unwrap_or(0);
            let sessions = registry.list();

            for (i, session) in sessions.iter().enumerate() {
                let is_active = i == active_idx;
                let text = Self::format_tab(i, session, is_active);

                let style = if is_active {
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::White).bg(Color::DarkGray)
                };

                spans.push(Span::styled(text, style));
                spans.push(Span::raw(" "));
            }
        }

        let line = Line::from(spans);
        let tabs_width = line.width() as u16;
        buf.set_line(area.x, area.y, &line, area.width);

        // Keybinding hints on the right (only rendered if there is space without overlapping tabs)
        let hints = "Alt+C:New/Open | Alt+1..9:Switch | Alt+N/P:Next/Prev | Alt+D:Detach | Alt+X:Close | Alt+Q:Quit";
        let hints_len = hints.chars().count() as u16;
        if area.width >= tabs_width + hints_len + 2 {
            let start_x = area.right().saturating_sub(hints_len + 1);
            let hint_style = Style::default().fg(Color::LightCyan).bg(Color::DarkGray);
            let hint_line = Line::from(Span::styled(hints, hint_style));
            buf.set_line(start_x, area.y, &hint_line, hints_len);
        } else {
            // Shortened hints fallback if space permits
            let short_hints = "Alt+C:New | Alt+1..9 | Alt+N/P | Alt+D | Alt+Q";
            let short_len = short_hints.chars().count() as u16;
            if area.width >= tabs_width + short_len + 2 {
                let start_x = area.right().saturating_sub(short_len + 1);
                let hint_style = Style::default().fg(Color::LightCyan).bg(Color::DarkGray);
                let hint_line = Line::from(Span::styled(short_hints, hint_style));
                buf.set_line(start_x, area.y, &hint_line, short_len);
            }
        }
    }
}

impl<'a> Widget for StatusBar<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        Self::render_to_buffer(self.registry, area, buf);
    }
}

impl<'a> Widget for &StatusBar<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        StatusBar::render_to_buffer(self.registry, area, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{ProjectSession, PtyConfig, PtySession, SessionStatus, VtScreen};
    use tempfile::tempdir;

    fn create_test_session_with_status(
        project_name: &str,
        workspace: &str,
        agent: &str,
        status: SessionStatus,
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
        let mut ps = ProjectSession::new(project_name, workspace, agent);
        ps.set_status(status);

        (ps, pty, vt)
    }

    #[test]
    fn test_status_bar_tab_formatting() {
        let (ps, pty, vt) =
            create_test_session_with_status("my-feature", "ws", "claude", SessionStatus::Busy);
        let entry = SessionEntry::new(ps, pty, vt);

        let active_tab = StatusBar::format_tab(0, &entry, true);
        assert_eq!(active_tab, " [1: my-feature ⏳] ");

        let inactive_tab = StatusBar::format_tab(1, &entry, false);
        assert_eq!(inactive_tab, "  2: my-feature ⏳  ");
    }

    #[test]
    fn test_status_bar_badges_display() {
        let statuses = [
            (SessionStatus::Initializing, "🔄"),
            (SessionStatus::Busy, "⏳"),
            (SessionStatus::WaitingForInput, "💬"),
            (SessionStatus::Done, "✅"),
            (SessionStatus::Error, "⚠️"),
            (SessionStatus::Terminated, "⏹️"),
        ];

        for (status, badge) in statuses {
            let (ps, pty, vt) =
                create_test_session_with_status("test-badge", "ws", "claude", status);
            let entry = SessionEntry::new(ps, pty, vt);
            let tab_str = StatusBar::format_tab(0, &entry, true);
            assert!(
                tab_str.contains(badge),
                "Expected badge {badge} in tab string '{tab_str}' for status {status:?}"
            );
        }
    }

    #[test]
    fn test_status_bar_empty_registry_rendering() {
        let registry = SessionRegistry::new();
        let mut buffer = Buffer::empty(Rect::new(0, 0, 100, 1));
        StatusBar::render_to_buffer(&registry, Rect::new(0, 0, 100, 1), &mut buffer);

        let rendered = buffer
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(rendered.contains("No Active Sessions"));
    }

    #[test]
    fn test_status_bar_multi_session_rendering() {
        let mut registry = SessionRegistry::new();
        let (ps1, pty1, vt1) =
            create_test_session_with_status("proj-backend", "ws", "claude", SessionStatus::Busy);
        let (ps2, pty2, vt2) = create_test_session_with_status(
            "proj-frontend",
            "ws",
            "agy",
            SessionStatus::WaitingForInput,
        );

        registry.add(ps1, pty1, vt1).unwrap();
        registry.add(ps2, pty2, vt2).unwrap();

        let mut buffer = Buffer::empty(Rect::new(0, 0, 160, 1));
        StatusBar::render_to_buffer(&registry, Rect::new(0, 0, 160, 1), &mut buffer);

        let rendered = buffer
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();

        assert!(rendered.contains("1: proj-backend ⏳"));
        assert!(rendered.contains("2: proj-frontend 💬"));
        assert!(rendered.contains("Alt+C:New/Open"));
        assert!(rendered.contains("Alt+1..9:Switch"));
    }
}
