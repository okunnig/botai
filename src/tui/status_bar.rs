use crate::session::{SessionEntry, SessionRegistry};
use crate::tui::theme::Theme;
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Widget,
};

/// Status bar widget displaying active project tabs, agent status badges, and multiplexer hotkeys.
pub struct StatusBar<'a> {
    registry: &'a SessionRegistry,
    leader_active: bool,
    theme: Option<&'a Theme>,
}

impl<'a> StatusBar<'a> {
    /// Creates a new `StatusBar` widget backed by the given `SessionRegistry`.
    pub fn new(registry: &'a SessionRegistry) -> Self {
        Self {
            registry,
            leader_active: false,
            theme: None,
        }
    }

    /// Configures whether leader mode is actively displayed on the status bar.
    pub fn with_leader(mut self, leader_active: bool) -> Self {
        self.leader_active = leader_active;
        self
    }

    /// Configures the theme used by the status bar.
    pub fn with_theme(mut self, theme: &'a Theme) -> Self {
        self.theme = Some(theme);
        self
    }

    /// Formats a single tab title for a session entry with leading status dot indicator and hotkey number.
    pub fn format_tab(index: usize, session: &SessionEntry, _is_active: bool) -> String {
        let hotkey = if index < 9 {
            format!("{}", index + 1)
        } else {
            "-".to_string()
        };

        let proj = session.project_name();

        format!("  ● {hotkey}: {proj}  ")
    }

    /// Formats a single tab into styled Spans with a themed leading colored status dot indicator.
    pub fn format_tab_spans<'b>(
        index: usize,
        session: &SessionEntry,
        is_active: bool,
        theme: &'b Theme,
    ) -> Vec<Span<'b>> {
        let hotkey = if index < 9 {
            format!("{}", index + 1)
        } else {
            "-".to_string()
        };

        let proj = session.project_name();
        let dot_color = session.status().status_color(theme);

        if is_active {
            let active_style = Style::default()
                .fg(theme.tab_active_fg)
                .bg(theme.tab_active_bg)
                .add_modifier(Modifier::BOLD);
            let dot_style_active = Style::default()
                .fg(dot_color)
                .bg(theme.tab_active_bg)
                .add_modifier(Modifier::BOLD);

            vec![
                Span::styled("  ", active_style),
                Span::styled("●", dot_style_active),
                Span::styled(format!(" {hotkey}: {proj}  "), active_style),
            ]
        } else {
            let inactive_style = Style::default()
                .fg(theme.tab_inactive_fg)
                .bg(theme.tab_inactive_bg);
            let dot_style_inactive = Style::default().fg(dot_color).bg(theme.tab_inactive_bg);

            vec![
                Span::styled("  ", inactive_style),
                Span::styled("●", dot_style_inactive),
                Span::styled(format!(" {hotkey}: {proj}  "), inactive_style),
            ]
        }
    }

    /// Renders the status bar directly into a Ratatui `Buffer` using the default theme.
    pub fn render_to_buffer(
        registry: &SessionRegistry,
        leader_active: bool,
        area: Rect,
        buf: &mut Buffer,
    ) {
        let default_theme = Theme::default();
        Self::render_to_buffer_with_theme(registry, leader_active, &default_theme, area, buf);
    }

    /// Renders the status bar directly into a Ratatui `Buffer` using the specified `Theme`.
    pub fn render_to_buffer_with_theme(
        registry: &SessionRegistry,
        leader_active: bool,
        theme: &Theme,
        area: Rect,
        buf: &mut Buffer,
    ) {
        if area.height == 0 || area.width == 0 {
            return;
        }

        // Fill entire background cleanly
        let bg_style = Style::default()
            .bg(theme.status_bar_bg)
            .fg(theme.status_bar_fg);
        for x in area.left()..area.right() {
            for y in area.top()..area.bottom() {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_style(bg_style).set_char(' ');
                }
            }
        }

        if registry.is_empty() {
            if leader_active {
                let leader_msg = "[1..9 Switch | c New | n/p Nav | w Close | d Detach | q Quit]";
                let line = Line::from(Span::styled(
                    leader_msg,
                    Style::default()
                        .fg(theme.leader_fg)
                        .bg(theme.leader_bg)
                        .add_modifier(Modifier::BOLD),
                ));
                buf.set_line(area.x, area.y, &line, area.width);
            } else {
                let msg = " [No Active Sessions] - Press Ctrl+Space then q to quit ";
                let line = Line::from(Span::styled(
                    msg,
                    Style::default()
                        .fg(theme.modal_title)
                        .bg(theme.status_bar_bg)
                        .add_modifier(Modifier::BOLD),
                ));
                buf.set_line(area.x, area.y, &line, area.width);
            }
            return;
        }

        let active_idx = registry.active_index().unwrap_or(0);
        let sessions = registry.list();

        // Format all tabs using themed spans
        let tab_spans_list: Vec<Vec<Span>> = sessions
            .iter()
            .enumerate()
            .map(|(i, session)| {
                let is_active = i == active_idx;
                Self::format_tab_spans(i, session, is_active, theme)
            })
            .collect();

        // Tab character length calculations
        let tab_lens: Vec<usize> = sessions
            .iter()
            .enumerate()
            .map(|(i, session)| {
                let is_active = i == active_idx;
                Self::format_tab(i, session, is_active).chars().count() + 1
            })
            .collect();

        let total_tabs_len: usize = tab_lens.iter().sum();

        let (visible_start, visible_end) = if total_tabs_len as u16 <= area.width {
            (0, tab_spans_list.len())
        } else {
            // Find window of tabs centered around active_idx that fits in area.width
            let act = active_idx.min(tab_spans_list.len().saturating_sub(1));
            let mut start = act;
            let mut end = act + 1;
            let mut used_width = tab_lens[act] as u16;

            // Expand right as much as possible
            while end < tab_spans_list.len() {
                let next_len = tab_lens[end] as u16;
                if used_width + next_len <= area.width {
                    used_width += next_len;
                    end += 1;
                } else {
                    break;
                }
            }

            // Expand left as much as possible
            while start > 0 {
                let prev_len = tab_lens[start - 1] as u16;
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
                Style::default()
                    .fg(theme.leader_bg)
                    .bg(theme.status_bar_bg)
                    .add_modifier(Modifier::BOLD),
            ));
        }

        for tab_spans in &tab_spans_list[visible_start..visible_end] {
            spans.extend(tab_spans.clone());
            spans.push(Span::raw(" "));
        }

        if visible_end < tab_spans_list.len() {
            spans.push(Span::styled(
                "▶",
                Style::default()
                    .fg(theme.leader_bg)
                    .bg(theme.status_bar_bg)
                    .add_modifier(Modifier::BOLD),
            ));
        }

        let line = Line::from(spans);
        let tabs_width = line.width() as u16;
        buf.set_line(area.x, area.y, &line, area.width);

        if leader_active {
            let leader_msg = "[1..9 Switch | c New | n/p Nav | w Close | d Detach | q Quit]";
            let leader_len = leader_msg.chars().count() as u16;
            if area.width >= tabs_width + leader_len + 2 {
                let start_x = area.right().saturating_sub(leader_len + 1);
                let leader_style = Style::default()
                    .fg(theme.leader_fg)
                    .bg(theme.leader_bg)
                    .add_modifier(Modifier::BOLD);
                let leader_line = Line::from(Span::styled(leader_msg, leader_style));
                buf.set_line(start_x, area.y, &leader_line, leader_len);
            } else if area.width >= leader_len {
                let start_x = area.right().saturating_sub(leader_len);
                let leader_style = Style::default()
                    .fg(theme.leader_fg)
                    .bg(theme.leader_bg)
                    .add_modifier(Modifier::BOLD);
                let leader_line = Line::from(Span::styled(leader_msg, leader_style));
                buf.set_line(start_x, area.y, &leader_line, leader_len);
            }
        } else {
            let hint = "Ctrl+Space: Actions";
            let hint_len = hint.chars().count() as u16;
            if area.width >= tabs_width + hint_len + 2 {
                let start_x = area.right().saturating_sub(hint_len + 1);
                let hint_style = Style::default().fg(theme.key_hint).bg(theme.status_bar_bg);
                let hint_line = Line::from(Span::styled(hint, hint_style));
                buf.set_line(start_x, area.y, &hint_line, hint_len);
            }
        }
    }
}

impl<'a> Widget for StatusBar<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let default_theme = Theme::default();
        let theme = self.theme.unwrap_or(&default_theme);
        Self::render_to_buffer_with_theme(self.registry, self.leader_active, theme, area, buf);
    }
}

impl<'a> Widget for &StatusBar<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let default_theme = Theme::default();
        let theme = self.theme.unwrap_or(&default_theme);
        StatusBar::render_to_buffer_with_theme(self.registry, self.leader_active, theme, area, buf);
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
        assert_eq!(active_tab, "  ● 1: my-feature  ");

        let inactive_tab = StatusBar::format_tab(1, &entry, false);
        assert_eq!(inactive_tab, "  ● 2: my-feature  ");

        let overflow_active = StatusBar::format_tab(9, &entry, true);
        assert_eq!(overflow_active, "  ● -: my-feature  ");

        let overflow_inactive = StatusBar::format_tab(9, &entry, false);
        assert_eq!(overflow_inactive, "  ● -: my-feature  ");
    }

    #[test]
    fn test_status_bar_tab_spans_formatting_and_colors() {
        let theme = Theme::tokyo_night();
        let statuses = [
            (SessionStatus::Initializing, theme.status_bar_fg),
            (SessionStatus::Busy, theme.status_busy),
            (SessionStatus::WaitingForInput, theme.status_waiting),
            (SessionStatus::Done, theme.status_done),
            (SessionStatus::Error, theme.status_error),
            (SessionStatus::Terminated, theme.status_bar_fg),
        ];

        for (status, expected_color) in statuses {
            let (ps, pty, vt) =
                create_test_session_with_status("test-proj", "ws", "claude", status);
            let entry = SessionEntry::new(ps, pty, vt);

            // Active tab spans
            let active_spans = StatusBar::format_tab_spans(0, &entry, true, &theme);
            assert_eq!(active_spans.len(), 3);
            assert_eq!(active_spans[0].content, "  ");
            assert_eq!(active_spans[0].style.fg, Some(theme.tab_active_fg));
            assert_eq!(active_spans[0].style.bg, Some(theme.tab_active_bg));

            assert_eq!(active_spans[1].content, "●");
            assert_eq!(active_spans[1].style.fg, Some(expected_color));
            assert_eq!(active_spans[1].style.bg, Some(theme.tab_active_bg));

            assert_eq!(active_spans[2].content, " 1: test-proj  ");
            assert_eq!(active_spans[2].style.fg, Some(theme.tab_active_fg));
            assert_eq!(active_spans[2].style.bg, Some(theme.tab_active_bg));

            // Inactive tab spans
            let inactive_spans = StatusBar::format_tab_spans(1, &entry, false, &theme);
            assert_eq!(inactive_spans.len(), 3);
            assert_eq!(inactive_spans[0].content, "  ");
            assert_eq!(inactive_spans[0].style.fg, Some(theme.tab_inactive_fg));
            assert_eq!(inactive_spans[0].style.bg, Some(theme.tab_inactive_bg));

            assert_eq!(inactive_spans[1].content, "●");
            assert_eq!(inactive_spans[1].style.fg, Some(expected_color));
            assert_eq!(inactive_spans[1].style.bg, Some(theme.tab_inactive_bg));

            assert_eq!(inactive_spans[2].content, " 2: test-proj  ");
            assert_eq!(inactive_spans[2].style.fg, Some(theme.tab_inactive_fg));
            assert_eq!(inactive_spans[2].style.bg, Some(theme.tab_inactive_bg));
        }
    }

    #[test]
    fn test_status_bar_empty_registry_rendering() {
        let registry = SessionRegistry::new();
        // Inactive leader
        let mut buffer = Buffer::empty(Rect::new(0, 0, 100, 1));
        StatusBar::render_to_buffer(&registry, false, Rect::new(0, 0, 100, 1), &mut buffer);

        let rendered = buffer
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect::<String>();
        assert!(rendered.contains("No Active Sessions"));
        assert!(rendered.contains("Press Ctrl+Space then q to quit"));

        // Active leader
        let mut buffer_leader = Buffer::empty(Rect::new(0, 0, 100, 1));
        StatusBar::render_to_buffer(&registry, true, Rect::new(0, 0, 100, 1), &mut buffer_leader);

        let rendered_leader = buffer_leader
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect::<String>();
        assert!(rendered_leader
            .contains("[1..9 Switch | c New | n/p Nav | w Close | d Detach | q Quit]"));
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

        let theme = Theme::tokyo_night();
        let mut buffer = Buffer::empty(Rect::new(0, 0, 160, 1));
        StatusBar::render_to_buffer_with_theme(
            &registry,
            false,
            &theme,
            Rect::new(0, 0, 160, 1),
            &mut buffer,
        );

        let rendered = buffer
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect::<String>();

        assert!(rendered.contains("● 1: proj-backend"));
        assert!(rendered.contains("● 2: proj-frontend"));
        assert!(rendered.contains("Ctrl+Space: Actions"));

        // Verify status dot colors in buffer cells
        // Session 1 (active, Busy): dot at x = 2
        let dot_cell_1 = buffer.cell((2, 0)).unwrap();
        assert_eq!(dot_cell_1.symbol(), "●");
        assert_eq!(dot_cell_1.fg, theme.status_busy);
        assert_eq!(dot_cell_1.bg, theme.tab_active_bg);

        // Session 2 (inactive, WaitingForInput):
        // Tab 1 length: "  ● 1: proj-backend  ".chars().count() = 21, + 1 separator space = 22
        // Tab 2 starts at x = 22: "  " (2 chars) -> dot at x = 24
        let dot_cell_2 = buffer.cell((24, 0)).unwrap();
        assert_eq!(dot_cell_2.symbol(), "●");
        assert_eq!(dot_cell_2.fg, theme.status_waiting);
        assert_eq!(dot_cell_2.bg, theme.tab_inactive_bg);
    }

    #[test]
    fn test_status_bar_leader_active_rendering() {
        let mut registry = SessionRegistry::new();
        let (ps1, pty1, vt1) =
            create_test_session_with_status("proj-backend", "ws", "claude", SessionStatus::Busy);
        registry.add(ps1, pty1, vt1).unwrap();

        let mut buffer = Buffer::empty(Rect::new(0, 0, 160, 1));
        StatusBar::render_to_buffer(&registry, true, Rect::new(0, 0, 160, 1), &mut buffer);

        let rendered = buffer
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect::<String>();

        assert!(rendered.contains("● 1: proj-backend"));
        assert!(rendered.contains("[1..9 Switch | c New | n/p Nav | w Close | d Detach | q Quit]"));
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

        // 1. Wide buffer (100 cols) -> Displays "Ctrl+Space: Actions"
        let mut buf_wide = Buffer::empty(Rect::new(0, 0, 100, 1));
        StatusBar::render_to_buffer(&registry, false, Rect::new(0, 0, 100, 1), &mut buf_wide);
        let rendered_wide = buf_wide
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect::<String>();
        assert!(rendered_wide.contains("● 1: p1"));
        assert!(rendered_wide.contains("● 2: p2"));
        assert!(rendered_wide.contains("Ctrl+Space: Actions"));

        // 2. Tight buffer (35 cols) -> Tabs visible, hint cleanly omitted
        let mut buf_tight = Buffer::empty(Rect::new(0, 0, 35, 1));
        StatusBar::render_to_buffer(&registry, false, Rect::new(0, 0, 35, 1), &mut buf_tight);
        let rendered_tight = buf_tight
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect::<String>();
        assert!(rendered_tight.contains("● 1: p1"));
        assert!(!rendered_tight.contains("Ctrl+Space: Actions"));
    }

    #[test]
    fn test_status_bar_tab_clipping_preserves_active_tab() {
        let mut registry = SessionRegistry::new();
        for i in 1..=10 {
            let name = format!("service-long-name-{i}");
            let (ps, pty, vt) =
                create_test_session_with_status(&name, "ws", "claude", SessionStatus::Done);
            registry.add(ps, pty, vt).unwrap();
        }

        // Set active session to the 8th one (1-indexed 8 -> index 7)
        registry.set_active_index(7).unwrap();

        // Render in a 60-col width buffer (cannot fit all 10 long tabs)
        let mut buf60 = Buffer::empty(Rect::new(0, 0, 60, 1));
        StatusBar::render_to_buffer(&registry, false, Rect::new(0, 0, 60, 1), &mut buf60);
        let rendered60 = buf60
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect::<String>();

        // Active tab 8 MUST be rendered
        assert!(
            rendered60.contains("● 8: service-long-name-8"),
            "Rendered output must contain active tab 8, got: {rendered60}"
        );
    }

    #[test]
    fn test_status_bar_themed_rendering() {
        let mut registry = SessionRegistry::new();
        let (ps1, pty1, vt1) =
            create_test_session_with_status("themed-proj", "ws", "claude", SessionStatus::Busy);
        registry.add(ps1, pty1, vt1).unwrap();

        let theme = Theme::cyberpunk();
        let mut buffer = Buffer::empty(Rect::new(0, 0, 80, 1));
        StatusBar::render_to_buffer_with_theme(
            &registry,
            false,
            &theme,
            Rect::new(0, 0, 80, 1),
            &mut buffer,
        );

        // Check active tab text and colors
        let rendered = buffer
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect::<String>();
        assert!(rendered.contains("● 1: themed-proj"));

        // Tab leading space cell at (1, 0) matches tab_active_fg and tab_active_bg
        let space_cell = buffer.cell((1, 0)).unwrap();
        assert_eq!(space_cell.symbol(), " ");
        assert_eq!(space_cell.bg, theme.tab_active_bg);
        assert_eq!(space_cell.fg, theme.tab_active_fg);

        // Status dot cell at (2, 0) matches status_busy and tab_active_bg
        let dot_cell = buffer.cell((2, 0)).unwrap();
        assert_eq!(dot_cell.symbol(), "●");
        assert_eq!(dot_cell.bg, theme.tab_active_bg);
        assert_eq!(dot_cell.fg, theme.status_busy);

        // Hotkey cell at (4, 0) matches tab_active_fg and tab_active_bg
        let hotkey_cell = buffer.cell((4, 0)).unwrap();
        assert_eq!(hotkey_cell.symbol(), "1");
        assert_eq!(hotkey_cell.bg, theme.tab_active_bg);
        assert_eq!(hotkey_cell.fg, theme.tab_active_fg);

        // Background trailing cells should match theme.status_bar_bg
        let empty_cell = buffer.cell((79, 0)).unwrap();
        assert_eq!(empty_cell.bg, theme.status_bar_bg);
    }

    #[test]
    fn test_status_bar_status_dot_colors_in_buffer() {
        let themes = [
            Theme::tokyo_night(),
            Theme::catppuccin(),
            Theme::cyberpunk(),
            Theme::gruvbox(),
            Theme::monochrome(),
            Theme::ansi(),
        ];

        let statuses = [
            SessionStatus::Initializing,
            SessionStatus::Busy,
            SessionStatus::WaitingForInput,
            SessionStatus::Done,
            SessionStatus::Error,
            SessionStatus::Terminated,
        ];

        for theme in &themes {
            for status in statuses {
                let mut registry = SessionRegistry::new();
                let (ps, pty, vt) =
                    create_test_session_with_status("status-test", "ws", "claude", status);
                registry.add(ps, pty, vt).unwrap();

                let mut buffer = Buffer::empty(Rect::new(0, 0, 60, 1));
                StatusBar::render_to_buffer_with_theme(
                    &registry,
                    false,
                    theme,
                    Rect::new(0, 0, 60, 1),
                    &mut buffer,
                );

                // Dot cell is at (2, 0)
                let dot_cell = buffer.cell((2, 0)).unwrap();
                assert_eq!(dot_cell.symbol(), "●");
                assert_eq!(
                    dot_cell.fg,
                    status.status_color(theme),
                    "Status color mismatch for {:?} under theme {}",
                    status,
                    theme.name
                );
                assert_eq!(dot_cell.bg, theme.tab_active_bg);
            }
        }
    }
}
