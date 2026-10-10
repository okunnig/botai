use crate::session::{SessionEntry, SessionRegistry};
use crate::tui::theme::Theme;
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Paragraph, Widget},
};

/// A reusable, declarative top bar / header widget for the TUI.
///
/// Handles background styling via `Block`, horizontal layout splitting via `Layout::horizontal`,
/// and right-aligned action / navigation hints via `Paragraph::new(...).alignment(Alignment::Right)`.
#[derive(Debug, Clone)]
pub struct TopBar<'a> {
    left: Line<'a>,
    right: Option<Line<'a>>,
    right_len: u16,
    right_padding: bool,
    theme: Option<&'a Theme>,
}

impl<'a> TopBar<'a> {
    /// Creates a new `TopBar` widget with the given left-aligned line.
    pub fn new(left: impl Into<Line<'a>>) -> Self {
        Self {
            left: left.into(),
            right: None,
            right_len: 0,
            right_padding: true,
            theme: None,
        }
    }

    /// Configures right-aligned content with default trailing 1-character padding.
    pub fn with_right(mut self, right: impl Into<Line<'a>>, len: u16) -> Self {
        self.right = Some(right.into());
        self.right_len = len;
        self.right_padding = true;
        self
    }

    /// Configures right-aligned content and whether to include trailing 1-character padding.
    pub fn with_right_and_padding(
        mut self,
        right: impl Into<Line<'a>>,
        len: u16,
        padding: bool,
    ) -> Self {
        self.right = Some(right.into());
        self.right_len = len;
        self.right_padding = padding;
        self
    }

    /// Configures the theme used by the top bar.
    pub fn with_theme(mut self, theme: &'a Theme) -> Self {
        self.theme = Some(theme);
        self
    }

    /// Renders the top bar into the given buffer area.
    pub fn render_to_buffer(&self, area: Rect, buf: &mut Buffer) {
        if area.height == 0 || area.width == 0 {
            return;
        }

        let default_theme = Theme::default();
        let theme = self.theme.unwrap_or(&default_theme);

        // 1. Block-based background styling
        let bg_style = Style::default()
            .bg(theme.status_bar_bg)
            .fg(theme.status_bar_fg);
        Block::default().style(bg_style).render(area, buf);

        // 2. Declarative horizontal layout & right-aligned paragraph
        if let Some(ref right_line) = self.right {
            let pad = if self.right_padding { 1 } else { 0 };
            let needed_right_width = self.right_len + pad;

            if area.width >= needed_right_width {
                let chunks = Layout::horizontal([
                    Constraint::Fill(1),
                    Constraint::Length(needed_right_width),
                ])
                .split(area);

                // Render left content
                Paragraph::new(self.left.clone()).render(chunks[0], buf);

                // Render right content with right alignment
                let right_to_render = if self.right_padding {
                    let mut spans = right_line.spans.clone();
                    spans.push(Span::styled(" ", Style::default().bg(theme.status_bar_bg)));
                    Line::from(spans)
                } else {
                    right_line.clone()
                };

                Paragraph::new(right_to_render)
                    .alignment(ratatui::layout::Alignment::Right)
                    .render(chunks[1], buf);
                return;
            }
        }

        // Single full-width left paragraph if right content is absent or cannot fit
        Paragraph::new(self.left.clone()).render(area, buf);
    }
}

impl<'a> Widget for TopBar<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        self.render_to_buffer(area, buf);
    }
}

impl<'a> Widget for &TopBar<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        self.render_to_buffer(area, buf);
    }
}

/// Computes the sliding window `(start, end)` of visible tab indices centered around `active_idx`
/// such that the total length of visible tabs fits within `max_width`.
pub fn compute_visible_tab_range(
    tab_lens: &[usize],
    active_idx: usize,
    max_width: u16,
) -> (usize, usize) {
    if tab_lens.is_empty() || max_width == 0 {
        return (0, 0);
    }

    let total_len: usize = tab_lens.iter().sum();
    if total_len as u16 <= max_width {
        return (0, tab_lens.len());
    }

    let act = active_idx.min(tab_lens.len().saturating_sub(1));
    let mut start = act;
    let mut end = act + 1;
    let mut used_width = tab_lens[act] as u16;

    // Expand rightwards as much as possible
    while end < tab_lens.len() {
        let next_len = tab_lens[end] as u16;
        if used_width + next_len <= max_width {
            used_width += next_len;
            end += 1;
        } else {
            break;
        }
    }

    // Expand leftwards as much as possible
    while start > 0 {
        let prev_len = tab_lens[start - 1] as u16;
        if used_width + prev_len <= max_width {
            used_width += prev_len;
            start -= 1;
        } else {
            break;
        }
    }

    (start, end)
}

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
        let tpl = session.template_name().trim();

        if tpl.is_empty() {
            format!("  ● {hotkey} {proj}  ")
        } else {
            format!("  ● {hotkey} [{tpl}] {proj}  ")
        }
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
        let tpl = session.template_name().trim();
        let label = if tpl.is_empty() {
            format!(" {hotkey} {proj}  ")
        } else {
            format!(" {hotkey} [{tpl}] {proj}  ")
        };

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
                Span::styled(label, active_style),
            ]
        } else {
            let inactive_style = Style::default()
                .fg(theme.tab_inactive_fg)
                .bg(theme.tab_inactive_bg);
            let dot_style_inactive = Style::default().fg(dot_color).bg(theme.tab_inactive_bg);

            vec![
                Span::styled("  ", inactive_style),
                Span::styled("●", dot_style_inactive),
                Span::styled(label, inactive_style),
            ]
        }
    }

    /// Builds the styled `Line` for all visible session tabs within the given available width.
    pub fn build_tabs_line<'b>(
        sessions: &[SessionEntry],
        active_idx: usize,
        max_width: u16,
        theme: &'b Theme,
    ) -> Line<'b> {
        if sessions.is_empty() {
            return Line::default();
        }

        let tab_spans_list: Vec<Vec<Span<'b>>> = sessions
            .iter()
            .enumerate()
            .map(|(i, session)| {
                let is_active = i == active_idx;
                Self::format_tab_spans(i, session, is_active, theme)
            })
            .collect();

        let tab_lens: Vec<usize> = tab_spans_list
            .iter()
            .map(|spans| {
                spans
                    .iter()
                    .map(|s| s.content.chars().count())
                    .sum::<usize>()
                    + 1
            })
            .collect();

        let (visible_start, visible_end) =
            compute_visible_tab_range(&tab_lens, active_idx, max_width);

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

        Line::from(spans)
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

        if registry.is_empty() {
            if leader_active {
                let leader_msg =
                    "[1..9 Switch | c New | n/p Nav | w Close | x Detach | d Destroy | q Quit]";
                let line = Line::from(Span::styled(
                    leader_msg,
                    Style::default()
                        .fg(theme.leader_fg)
                        .bg(theme.leader_bg)
                        .add_modifier(Modifier::BOLD),
                ));
                TopBar::new(line)
                    .with_theme(theme)
                    .render_to_buffer(area, buf);
            } else {
                let msg = " [No Active Sessions] - Press Ctrl+Space then q to quit ";
                let line = Line::from(Span::styled(
                    msg,
                    Style::default()
                        .fg(theme.modal_title)
                        .bg(theme.status_bar_bg)
                        .add_modifier(Modifier::BOLD),
                ));
                TopBar::new(line)
                    .with_theme(theme)
                    .render_to_buffer(area, buf);
            }
            return;
        }

        let active_idx = registry.active_index().unwrap_or(0);
        let sessions = registry.list();

        let tabs_line = Self::build_tabs_line(sessions, active_idx, area.width, theme);
        let tabs_width = tabs_line.width() as u16;

        let mut top_bar = TopBar::new(tabs_line).with_theme(theme);

        if leader_active {
            let leader_msg =
                "[1..9 Switch | c New | n/p Nav | w Close | x Detach | d Destroy | q Quit]";
            let leader_len = leader_msg.chars().count() as u16;
            let leader_style = Style::default()
                .fg(theme.leader_fg)
                .bg(theme.leader_bg)
                .add_modifier(Modifier::BOLD);

            if area.width >= tabs_width + leader_len + 2 {
                let leader_line = Line::from(Span::styled(leader_msg, leader_style));
                top_bar = top_bar.with_right_and_padding(leader_line, leader_len, true);
            } else if area.width >= leader_len {
                let leader_line = Line::from(Span::styled(leader_msg, leader_style));
                top_bar = top_bar.with_right_and_padding(leader_line, leader_len, false);
            }
        } else {
            let hint = "Ctrl+Space: Actions";
            let hint_len = hint.chars().count() as u16;
            if area.width >= tabs_width + hint_len + 2 {
                let hint_style = Style::default().fg(theme.key_hint).bg(theme.status_bar_bg);
                let hint_line = Line::from(Span::styled(hint, hint_style));
                top_bar = top_bar.with_right_and_padding(hint_line, hint_len, true);
            }
        }

        top_bar.render_to_buffer(area, buf);
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
        assert_eq!(active_tab, "  ● 1 [ws] my-feature  ");

        let inactive_tab = StatusBar::format_tab(1, &entry, false);
        assert_eq!(inactive_tab, "  ● 2 [ws] my-feature  ");

        let overflow_active = StatusBar::format_tab(9, &entry, true);
        assert_eq!(overflow_active, "  ● - [ws] my-feature  ");

        let overflow_inactive = StatusBar::format_tab(9, &entry, false);
        assert_eq!(overflow_inactive, "  ● - [ws] my-feature  ");
    }

    #[test]
    fn test_status_bar_tab_formatting_empty_template_fallback() {
        let (ps, pty, vt) =
            create_test_session_with_status("my-feature", "", "claude", SessionStatus::Busy);
        let entry = SessionEntry::new(ps, pty, vt);
        let theme = Theme::tokyo_night();

        let active_tab = StatusBar::format_tab(0, &entry, true);
        assert_eq!(active_tab, "  ● 1 my-feature  ");

        let inactive_tab = StatusBar::format_tab(1, &entry, false);
        assert_eq!(inactive_tab, "  ● 2 my-feature  ");

        let spans = StatusBar::format_tab_spans(0, &entry, true, &theme);
        assert_eq!(spans[2].content, " 1 my-feature  ");

        // Also test whitespace-only template fallback
        let (ps_blank, pty_blank, vt_blank) =
            create_test_session_with_status("my-feature", "   ", "claude", SessionStatus::Busy);
        let entry_blank = SessionEntry::new(ps_blank, pty_blank, vt_blank);
        assert_eq!(
            StatusBar::format_tab(0, &entry_blank, true),
            "  ● 1 my-feature  "
        );
        let spans_blank = StatusBar::format_tab_spans(0, &entry_blank, true, &theme);
        assert_eq!(spans_blank[2].content, " 1 my-feature  ");
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

            assert_eq!(active_spans[2].content, " 1 [ws] test-proj  ");
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

            assert_eq!(inactive_spans[2].content, " 2 [ws] test-proj  ");
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
            .contains("[1..9 Switch | c New | n/p Nav | w Close | x Detach | d Destroy | q Quit]"));
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

        assert!(rendered.contains("● 1 [ws] proj-backend"));
        assert!(rendered.contains("● 2 [ws] proj-frontend"));
        assert!(rendered.contains("Ctrl+Space: Actions"));

        // Verify status dot colors in buffer cells
        // Session 1 (active, Busy): dot at x = 2
        let dot_cell_1 = buffer.cell((2, 0)).unwrap();
        assert_eq!(dot_cell_1.symbol(), "●");
        assert_eq!(dot_cell_1.fg, theme.status_busy);
        assert_eq!(dot_cell_1.bg, theme.tab_active_bg);

        // Session 2 (inactive, WaitingForInput):
        // Tab 1 length: "  ● 1 [ws] proj-backend  ".chars().count() = 25, + 1 separator space = 26
        // Tab 2 starts at x = 26: "  " (2 chars) -> dot at x = 28
        let dot_cell_2 = buffer.cell((28, 0)).unwrap();
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

        assert!(rendered.contains("● 1 [ws] proj-backend"));
        assert!(rendered
            .contains("[1..9 Switch | c New | n/p Nav | w Close | x Detach | d Destroy | q Quit]"));
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
        assert!(rendered_wide.contains("● 1 [ws] p1"));
        assert!(rendered_wide.contains("● 2 [ws] p2"));
        assert!(rendered_wide.contains("Ctrl+Space: Actions"));

        // 2. Tight buffer (35 cols) -> Tabs visible, hint cleanly omitted
        let mut buf_tight = Buffer::empty(Rect::new(0, 0, 35, 1));
        StatusBar::render_to_buffer(&registry, false, Rect::new(0, 0, 35, 1), &mut buf_tight);
        let rendered_tight = buf_tight
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect::<String>();
        assert!(rendered_tight.contains("● 1 [ws] p1"));
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
            rendered60.contains("● 8 [ws] service-long-name-8"),
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
        assert!(rendered.contains("● 1 [ws] themed-proj"));

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

    #[test]
    fn test_compute_visible_tab_range() {
        // 1. Empty and zero width cases
        assert_eq!(compute_visible_tab_range(&[], 0, 100), (0, 0));
        assert_eq!(compute_visible_tab_range(&[10, 20], 0, 0), (0, 0));

        // 2. All tabs fit
        let lens = vec![15, 15, 15];
        assert_eq!(compute_visible_tab_range(&lens, 0, 50), (0, 3));
        assert_eq!(compute_visible_tab_range(&lens, 1, 45), (0, 3));

        // 3. Active index 0, expands rightwards
        let ten_tabs = vec![10; 10]; // 100 total
        assert_eq!(compute_visible_tab_range(&ten_tabs, 0, 35), (0, 3));

        // 4. Active index 9 (last), expands leftwards
        assert_eq!(compute_visible_tab_range(&ten_tabs, 9, 35), (7, 10));

        // 5. Active index 5 (middle), expands right then left
        // Start at 5 (len 10), expand right: 6 (20), 7 (30), 8 (40), stop right. Left 4 cannot fit (50 > 45) -> (5, 9)
        assert_eq!(compute_visible_tab_range(&ten_tabs, 5, 45), (5, 9));

        // 6. Active index out of bounds clamped safely
        assert_eq!(compute_visible_tab_range(&ten_tabs, 99, 30), (7, 10));
    }

    #[test]
    fn test_top_bar_widget_rendering() {
        let theme = Theme::tokyo_night();
        let area = Rect::new(0, 0, 80, 1);
        let mut buf = Buffer::empty(area);

        let left_line = Line::from("Left Content");
        let right_line = Line::from("Right Hint");

        let top_bar = TopBar::new(left_line)
            .with_right(right_line, 10)
            .with_theme(&theme);

        top_bar.render_to_buffer(area, &mut buf);

        let rendered = buf
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect::<String>();

        assert!(rendered.contains("Left Content"));
        assert!(rendered.contains("Right Hint"));

        // Background of empty cell in the middle matches theme
        let mid_cell = buf.cell((40, 0)).unwrap();
        assert_eq!(mid_cell.bg, theme.status_bar_bg);

        // Trailing cell at x = 79 has padding space with status_bar_bg
        let last_cell = buf.cell((79, 0)).unwrap();
        assert_eq!(last_cell.symbol(), " ");
        assert_eq!(last_cell.bg, theme.status_bar_bg);
    }

    #[test]
    fn test_top_bar_tight_width_omits_right_content() {
        let area = Rect::new(0, 0, 20, 1);
        let mut buf = Buffer::empty(area);

        let top_bar = TopBar::new(Line::from("Long Left Content"))
            .with_right(Line::from("Very Long Right Action Hint"), 27);

        top_bar.render_to_buffer(area, &mut buf);

        let rendered = buf
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect::<String>();

        assert!(rendered.contains("Long Left"));
        assert!(!rendered.contains("Very Long Right Action Hint"));
    }

    #[test]
    fn test_top_bar_widget_trait_implementations() {
        let area = Rect::new(0, 0, 50, 1);
        let mut buf1 = Buffer::empty(area);
        let mut buf2 = Buffer::empty(area);

        let top_bar = TopBar::new(Line::from("Test Bar"));
        // Value render
        top_bar.clone().render(area, &mut buf1);
        // Ref render
        (&top_bar).render(area, &mut buf2);

        let str1 = buf1
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect::<String>();
        let str2 = buf2
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect::<String>();

        assert_eq!(str1, str2);
        assert!(str1.contains("Test Bar"));
    }

    #[test]
    fn test_status_bar_build_tabs_line_slice_input() {
        let theme = Theme::tokyo_night();
        let mut registry = SessionRegistry::new();
        let (ps1, pty1, vt1) =
            create_test_session_with_status("slice-tab1", "ws", "claude", SessionStatus::Busy);
        let (ps2, pty2, vt2) = create_test_session_with_status(
            "slice-tab2",
            "ws",
            "agy",
            SessionStatus::WaitingForInput,
        );

        registry.add(ps1, pty1, vt1).unwrap();
        registry.add(ps2, pty2, vt2).unwrap();

        // Pass registry.list() directly as &[SessionEntry] slice
        let sessions_slice: &[SessionEntry] = registry.list();
        let tabs_line = StatusBar::build_tabs_line(sessions_slice, 0, 100, &theme);

        let rendered = tabs_line
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect::<String>();

        assert!(rendered.contains("● 1 [ws] slice-tab1"));
        assert!(rendered.contains("● 2 [ws] slice-tab2"));
    }
}
