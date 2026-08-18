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

        // Fill entire background cleanly
        let bg_style = Style::default().bg(Color::DarkGray).fg(Color::White);
        for x in area.left()..area.right() {
            for y in area.top()..area.bottom() {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_style(bg_style).set_char(' ');
                }
            }
        }

        if registry.is_empty() {
            let msg = " [No Active Sessions] - Press Alt+Q to quit ";
            let line = Line::from(Span::styled(
                msg,
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ));
            buf.set_line(area.x, area.y, &line, area.width);
            return;
        }

        let active_idx = registry.active_index().unwrap_or(0);
        let sessions = registry.list();

        // Format all tabs
        let tab_items: Vec<(String, Style)> = sessions
            .iter()
            .enumerate()
            .map(|(i, session)| {
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
                (text, style)
            })
            .collect();

        // Check if all tabs fit
        let total_tabs_len: usize = tab_items.iter().map(|(t, _)| t.chars().count() + 1).sum();

        let (visible_start, visible_end) = if total_tabs_len as u16 <= area.width {
            (0, tab_items.len())
        } else {
            // Find window of tabs centered around active_idx that fits in area.width
            let act = active_idx.min(tab_items.len().saturating_sub(1));
            let mut start = act;
            let mut end = act + 1;
            let mut used_width = tab_items[act].0.chars().count() as u16 + 1;

            // Expand right as much as possible
            while end < tab_items.len() {
                let next_len = tab_items[end].0.chars().count() as u16 + 1;
                if used_width + next_len <= area.width {
                    used_width += next_len;
                    end += 1;
                } else {
                    break;
                }
            }

            // Expand left as much as possible
            while start > 0 {
                let prev_len = tab_items[start - 1].0.chars().count() as u16 + 1;
                if used_width + prev_len <= area.width {
                    used_width += prev_len;
                    start -= 1;
                } else {
                    break;
                }
            }

            (start, end)
        };

        let mut spans = Vec::new();
        if visible_start > 0 {
            spans.push(Span::styled(
                "◀ ",
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            ));
        }

        for (text, style) in &tab_items[visible_start..visible_end] {
            spans.push(Span::styled(text.clone(), *style));
            spans.push(Span::raw(" "));
        }

        if visible_end < tab_items.len() {
            spans.push(Span::styled(
                "▶",
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            ));
        }

        let line = Line::from(spans);
        let tabs_width = line.width() as u16;
        buf.set_line(area.x, area.y, &line, area.width);

        // Multi-tier responsive hints based on remaining space
        let full_hints = "Alt+C:New/Open | Alt+1..9:Switch | Alt+N/P:Next/Prev | Alt+D:Detach | Alt+X:Close | Alt+Q:Quit";
        let compact_hints = "Alt+C:New | Alt+1..9 | Alt+N/P | Alt+D | Alt+Q";
        let minimal_hints = "Alt+C | Alt+1..9 | Alt+D";

        let full_len = full_hints.chars().count() as u16;
        let compact_len = compact_hints.chars().count() as u16;
        let minimal_len = minimal_hints.chars().count() as u16;

        let hint_to_render = if area.width >= tabs_width + full_len + 2 {
            Some((full_hints, full_len))
        } else if area.width >= tabs_width + compact_len + 2 {
            Some((compact_hints, compact_len))
        } else if area.width >= tabs_width + minimal_len + 2 {
            Some((minimal_hints, minimal_len))
        } else {
            None
        };

        if let Some((hint_str, hint_len)) = hint_to_render {
            let start_x = area.right().saturating_sub(hint_len + 1);
            let hint_style = Style::default().fg(Color::LightCyan).bg(Color::DarkGray);
            let hint_line = Line::from(Span::styled(hint_str, hint_style));
            buf.set_line(start_x, area.y, &hint_line, hint_len);
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

    #[test]
    fn test_status_bar_responsive_hints_at_various_widths() {
        let mut registry = SessionRegistry::new();
        let (ps1, pty1, vt1) =
            create_test_session_with_status("p1", "ws", "claude", SessionStatus::Busy);
        let (ps2, pty2, vt2) =
            create_test_session_with_status("p2", "ws", "agy", SessionStatus::WaitingForInput);

        registry.add(ps1, pty1, vt1).unwrap();
        registry.add(ps2, pty2, vt2).unwrap();

        // 1. 200 cols -> Full hints
        let mut buf200 = Buffer::empty(Rect::new(0, 0, 200, 1));
        StatusBar::render_to_buffer(&registry, Rect::new(0, 0, 200, 1), &mut buf200);
        let rendered200 = buf200.content().iter().map(|c| c.symbol()).collect::<String>();
        assert!(rendered200.contains("Alt+C:New/Open | Alt+1..9:Switch"));

        // 2. 120 cols -> Full or compact hints
        let mut buf120 = Buffer::empty(Rect::new(0, 0, 120, 1));
        StatusBar::render_to_buffer(&registry, Rect::new(0, 0, 120, 1), &mut buf120);
        let rendered120 = buf120.content().iter().map(|c| c.symbol()).collect::<String>();
        assert!(rendered120.contains("Alt+C:New"));

        // 3. 80 cols -> Compact or minimal hints
        let mut buf80 = Buffer::empty(Rect::new(0, 0, 80, 1));
        StatusBar::render_to_buffer(&registry, Rect::new(0, 0, 80, 1), &mut buf80);
        let rendered80 = buf80.content().iter().map(|c| c.symbol()).collect::<String>();
        assert!(rendered80.contains("Alt+C"));

        // 4. 40 cols -> Tight space, hints gracefully hidden, active tab still visible
        let mut buf40 = Buffer::empty(Rect::new(0, 0, 40, 1));
        StatusBar::render_to_buffer(&registry, Rect::new(0, 0, 40, 1), &mut buf40);
        let rendered40 = buf40.content().iter().map(|c| c.symbol()).collect::<String>();
        assert!(rendered40.contains("1: p1"));
        assert!(!rendered40.contains("Alt+C:New/Open"));
    }

    #[test]
    fn test_status_bar_tab_clipping_preserves_active_tab() {
        let mut registry = SessionRegistry::new();
        for i in 1..=10 {
            let name = format!("service-long-name-{}", i);
            let (ps, pty, vt) =
                create_test_session_with_status(&name, "ws", "claude", SessionStatus::Done);
            registry.add(ps, pty, vt).unwrap();
        }

        // Set active session to the 8th one (1-indexed 8 -> index 7)
        registry.set_active_index(7).unwrap();

        // Render in a 60-col width buffer (cannot fit all 10 long tabs)
        let mut buf60 = Buffer::empty(Rect::new(0, 0, 60, 1));
        StatusBar::render_to_buffer(&registry, Rect::new(0, 0, 60, 1), &mut buf60);
        let rendered60 = buf60.content().iter().map(|c| c.symbol()).collect::<String>();

        // Active tab 8 MUST be rendered
        assert!(
            rendered60.contains("8: service-long-name-8"),
            "Rendered output must contain active tab 8, got: {}",
            rendered60
        );
    }
}
