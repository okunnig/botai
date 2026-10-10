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

/// Truncates a label string to at most `max_chars` characters, appending an ellipsis `…` if truncated.
pub fn truncate_label(s: &str, max_chars: usize) -> String {
    let char_count = s.chars().count();
    if char_count <= max_chars {
        return s.to_string();
    }
    if max_chars == 0 {
        return String::new();
    }
    let take_count = max_chars.saturating_sub(1);
    let mut truncated: String = s.chars().take(take_count).collect();
    truncated.push('…');
    truncated
}

/// Maximum width in columns for an individual session tab in the status bar.
pub const MAX_TAB_WIDTH: usize = 32;

/// Dynamically and gracefully truncates template and project names to fit within `max_tab_width`.
///
/// Prioritizes keeping the project name clear for agent session identification while shrinking
/// or omitting template names when horizontal space is constrained.
pub fn smart_truncate_tab_title(
    template: &str,
    project: &str,
    max_tab_width: usize,
) -> (String, String) {
    let tpl_trimmed = template.trim();
    if tpl_trimmed.is_empty() {
        let overhead = 8; // "  ● 1 proj  " (sym, hotkey, spaces)
        let max_proj = max_tab_width.saturating_sub(overhead);
        return (String::new(), truncate_label(project, max_proj));
    }

    let overhead = 11; // "  ● 1 [tpl] proj  "
    let tpl_len = tpl_trimmed.chars().count();
    let proj_len = project.chars().count();

    if tpl_len + proj_len + overhead <= max_tab_width {
        return (tpl_trimmed.to_string(), project.to_string());
    }

    let budget = max_tab_width.saturating_sub(overhead);
    // If budget is very tight, omit template to preserve maximum project clarity
    if budget < 8 {
        let max_proj = max_tab_width.saturating_sub(8);
        return (String::new(), truncate_label(project, max_proj));
    }

    // Attempt to keep project intact and truncate template down to at least 4 chars
    if budget >= proj_len.saturating_add(4) {
        let tpl_alloc = budget.saturating_sub(proj_len);
        (truncate_label(tpl_trimmed, tpl_alloc), project.to_string())
    } else {
        // If both need truncation, allocate a compact 4..6 chars to template and the rest to project
        let tpl_alloc = 5.min(budget / 3).max(3);
        let proj_alloc = budget.saturating_sub(tpl_alloc);
        (
            truncate_label(tpl_trimmed, tpl_alloc),
            truncate_label(project, proj_alloc),
        )
    }
}

/// Computes the sliding window `(start, end)` of visible tab indices centered around `active_idx`
/// such that the total length of visible tabs fits within `max_width`.
///
/// When tabs overflow `max_width`, 4 columns are reserved to accommodate overflow indicator arrows
/// (`"◀ "` on the left and `" ▶"` on the right).
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

    // Reserve 4 columns for overflow arrows ("◀ " and " ▶")
    let budget = max_width.saturating_sub(4);
    let act = active_idx.min(tab_lens.len().saturating_sub(1));
    let mut start = act;
    let mut end = act + 1;
    let mut used_width = tab_lens[act] as u16;

    // Expand rightwards as much as possible
    while end < tab_lens.len() {
        let next_len = tab_lens[end] as u16;
        if used_width.saturating_add(next_len) <= budget {
            used_width += next_len;
            end += 1;
        } else {
            break;
        }
    }

    // Expand leftwards as much as possible
    while start > 0 {
        let prev_len = tab_lens[start - 1] as u16;
        if used_width.saturating_add(prev_len) <= budget {
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

        let sym = session.status().status_symbol();
        let (tpl, proj) = smart_truncate_tab_title(
            session.template_name(),
            session.project_name(),
            MAX_TAB_WIDTH,
        );

        if tpl.is_empty() {
            format!("  {sym} {hotkey} {proj}  ")
        } else {
            format!("  {sym} {hotkey} [{tpl}] {proj}  ")
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

        let mut dot_color = session.status().status_color(theme);
        // Contrast fallback guard on active tabs: if dot color collides with active tab bg, use active tab fg
        if is_active && dot_color == theme.tab_active_bg {
            dot_color = theme.tab_active_fg;
        }

        let sym = session.status().status_symbol();
        let (tpl, proj) = smart_truncate_tab_title(
            session.template_name(),
            session.project_name(),
            MAX_TAB_WIDTH,
        );
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
                Span::styled(sym, dot_style_active),
                Span::styled(label, active_style),
            ]
        } else {
            let inactive_style = Style::default()
                .fg(theme.tab_inactive_fg)
                .bg(theme.tab_inactive_bg);
            let dot_style_inactive = Style::default().fg(dot_color).bg(theme.tab_inactive_bg);

            vec![
                Span::styled("  ", inactive_style),
                Span::styled(sym, dot_style_inactive),
                Span::styled(label, inactive_style),
            ]
        }
    }

    /// Builds the styled `Line` for all visible session tabs within the given available width.
    pub fn build_tabs_line<'b>(
        sessions: &[&SessionEntry],
        active_idx: usize,
        max_width: u16,
        theme: &'b Theme,
    ) -> Line<'b> {
        if sessions.is_empty() || max_width == 0 {
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
                " ▶",
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

        if leader_active {
            let leader_msg =
                "[1..9 Switch | c New | n/p Nav | w Close | x Detach | d Destroy | q Quit]";
            let leader_len = leader_msg.chars().count() as u16;
            let leader_style = Style::default()
                .fg(theme.leader_fg)
                .bg(theme.leader_bg)
                .add_modifier(Modifier::BOLD);

            let needed_leader_width = leader_len + 1;
            let tabs_max_width = area.width.saturating_sub(needed_leader_width);

            let tabs_line = Self::build_tabs_line(&sessions, active_idx, tabs_max_width, theme);
            let mut top_bar = TopBar::new(tabs_line).with_theme(theme);

            if area.width >= needed_leader_width {
                let leader_line = Line::from(Span::styled(leader_msg, leader_style));
                top_bar = top_bar.with_right_and_padding(leader_line, leader_len, true);
            } else if area.width >= leader_len {
                let leader_line = Line::from(Span::styled(leader_msg, leader_style));
                top_bar = top_bar.with_right_and_padding(leader_line, leader_len, false);
            }

            top_bar.render_to_buffer(area, buf);
        } else {
            let hint = "Ctrl+Space: Actions";
            let hint_len = hint.chars().count() as u16;
            let needed_hint_width = hint_len + 1;

            let full_tabs_line = Self::build_tabs_line(&sessions, active_idx, area.width, theme);
            let full_tabs_width = full_tabs_line.width() as u16;

            if area.width > full_tabs_width + needed_hint_width {
                let hint_style = Style::default().fg(theme.key_hint).bg(theme.status_bar_bg);
                let hint_line = Line::from(Span::styled(hint, hint_style));
                let top_bar = TopBar::new(full_tabs_line)
                    .with_theme(theme)
                    .with_right_and_padding(hint_line, hint_len, true);
                top_bar.render_to_buffer(area, buf);
            } else {
                let top_bar = TopBar::new(full_tabs_line).with_theme(theme);
                top_bar.render_to_buffer(area, buf);
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

            assert_eq!(active_spans[1].content, status.status_symbol());
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

            assert_eq!(inactive_spans[1].content, status.status_symbol());
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
        assert!(rendered.contains("? 2 [ws] proj-frontend"));
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
        assert_eq!(dot_cell_2.symbol(), "?");
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
        assert!(rendered_wide.contains("? 2 [ws] p2"));
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

        // Active tab 8 MUST be rendered with Done checkmark symbol '✓'
        assert!(
            rendered60.contains("✓ 8 [ws] service-long-name-8"),
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
                assert_eq!(dot_cell.symbol(), status.status_symbol());
                let mut expected_fg = status.status_color(theme);
                if expected_fg == theme.tab_active_bg {
                    expected_fg = theme.tab_active_fg;
                }
                assert_eq!(
                    dot_cell.fg, expected_fg,
                    "Status color mismatch for {:?} under theme {}",
                    status, theme.name
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

        // 6. Active index out of bounds clamped safely (budget = 30 - 4 = 26, fits 2 tabs: 8 and 9)
        assert_eq!(compute_visible_tab_range(&ten_tabs, 99, 30), (8, 10));
    }

    #[test]
    fn test_status_bar_contrast_fallback_guard_on_active_tab() {
        // Create a custom theme where status_busy collides with tab_active_bg
        let mut colliding_theme = Theme::tokyo_night();
        colliding_theme.status_busy = colliding_theme.tab_active_bg;

        let (ps, pty, vt) =
            create_test_session_with_status("colliding-tab", "ws", "claude", SessionStatus::Busy);
        let entry = SessionEntry::new(ps, pty, vt);

        // Active tab format spans must fall back to tab_active_fg
        let active_spans = StatusBar::format_tab_spans(0, &entry, true, &colliding_theme);
        let dot_span = &active_spans[1];
        assert_eq!(dot_span.content, "●");
        assert_eq!(dot_span.style.fg, Some(colliding_theme.tab_active_fg));
        assert_eq!(dot_span.style.bg, Some(colliding_theme.tab_active_bg));

        // Inactive tab keeps original status color against tab_inactive_bg
        let inactive_spans = StatusBar::format_tab_spans(1, &entry, false, &colliding_theme);
        assert_eq!(
            inactive_spans[1].style.fg,
            Some(colliding_theme.tab_active_bg)
        );
    }

    #[test]
    fn test_smart_truncate_tab_title_and_label() {
        // truncate_label tests
        assert_eq!(truncate_label("short", 10), "short");
        assert_eq!(truncate_label("exact", 5), "exact");
        assert_eq!(truncate_label("toolongstring", 8), "toolong…");
        assert_eq!(truncate_label("abc", 0), "");
        assert_eq!(truncate_label("abc", 1), "…");

        // smart_truncate_tab_title tests
        // Fits comfortably
        let (tpl, proj) = smart_truncate_tab_title("ecommerce", "payment-api", 35);
        assert_eq!(tpl, "ecommerce");
        assert_eq!(proj, "payment-api");

        // Moderate constraint: template truncated, project preserved intact
        let (tpl_trunc, proj_intact) =
            smart_truncate_tab_title("microservices-platform", "auth-gateway", 27);
        assert_eq!(proj_intact, "auth-gateway");
        assert!(tpl_trunc.ends_with('…'));

        // Very tight constraint: template omitted or compact, project shortened
        let (tpl_omitted, proj_trunc) =
            smart_truncate_tab_title("backend", "feature-long-name", 15);
        assert!(proj_trunc.ends_with('…'));
        assert!(tpl_omitted.is_empty() || tpl_omitted.ends_with('…'));

        // Empty template fallback
        let (tpl_empty, proj_short) = smart_truncate_tab_title("", "my-feature-branch", 18);
        assert!(tpl_empty.is_empty());
        assert!(proj_short.ends_with('…'));
    }

    #[test]
    fn test_status_bar_leader_mode_constrained_tabs_width() {
        let mut registry = SessionRegistry::new();
        for i in 1..=5 {
            let name = format!("service-{}", i);
            let (ps, pty, vt) =
                create_test_session_with_status(&name, "template", "claude", SessionStatus::Busy);
            registry.add(ps, pty, vt).unwrap();
        }
        registry.set_active_index(2).unwrap(); // Active is service-3

        let theme = Theme::tokyo_night();
        // Buffer width 110: leader menu is 73 (+ 1 = 74 cols). Available for tabs = 110 - 74 = 36 cols.
        let mut buffer = Buffer::empty(Rect::new(0, 0, 110, 1));
        StatusBar::render_to_buffer_with_theme(
            &registry,
            true,
            &theme,
            Rect::new(0, 0, 110, 1),
            &mut buffer,
        );

        let rendered = buffer
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect::<String>();

        // Must render leader command menu on the right
        assert!(rendered
            .contains("[1..9 Switch | c New | n/p Nav | w Close | x Detach | d Destroy | q Quit]"));
        // Active tab service-3 MUST be visible on the left
        assert!(rendered.contains("3 [template] service-3"));
    }

    #[test]
    fn test_status_bar_overflow_arrow_reservation_and_formatting() {
        let mut registry = SessionRegistry::new();
        for i in 1..=8 {
            let (ps, pty, vt) = create_test_session_with_status(
                &format!("proj-{}", i),
                "ws",
                "claude",
                SessionStatus::Busy,
            );
            registry.add(ps, pty, vt).unwrap();
        }
        registry.set_active_index(3).unwrap(); // 4th tab is active (middle)

        let theme = Theme::tokyo_night();
        // Constrain width so overflow arrows appear on both sides
        let mut buffer = Buffer::empty(Rect::new(0, 0, 50, 1));
        StatusBar::render_to_buffer_with_theme(
            &registry,
            false,
            &theme,
            Rect::new(0, 0, 50, 1),
            &mut buffer,
        );

        let rendered = buffer
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect::<String>();

        // Left overflow arrow formatted as "◀ "
        assert!(rendered.contains("◀ "));
        // Right overflow arrow symmetrically formatted as " ▶"
        assert!(rendered.contains(" ▶"));
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
    fn test_status_bar_tab_truncation_with_max_tab_width() {
        let theme = Theme::tokyo_night();

        // 1. Normal / short names remain untouched
        let (ps_short, pty_short, vt_short) =
            create_test_session_with_status("my-api", "backend", "claude", SessionStatus::Busy);
        let entry_short = SessionEntry::new(ps_short, pty_short, vt_short);

        let short_tab = StatusBar::format_tab(0, &entry_short, true);
        assert_eq!(short_tab, "  ● 1 [backend] my-api  ");
        assert!(short_tab.chars().count() <= MAX_TAB_WIDTH);

        let short_spans = StatusBar::format_tab_spans(0, &entry_short, true, &theme);
        assert_eq!(short_spans[2].content, " 1 [backend] my-api  ");

        // 2. Long project name (50+ chars) is cleanly truncated to fit within MAX_TAB_WIDTH
        let long_proj =
            "very-long-project-name-exceeding-fifty-characters-in-total-length-for-testing";
        assert!(long_proj.chars().count() > 50);
        let (ps_long_proj, pty_long_proj, vt_long_proj) =
            create_test_session_with_status(long_proj, "tpl", "claude", SessionStatus::Busy);
        let entry_long_proj = SessionEntry::new(ps_long_proj, pty_long_proj, vt_long_proj);

        let long_proj_tab = StatusBar::format_tab(0, &entry_long_proj, true);
        assert!(
            long_proj_tab.chars().count() <= MAX_TAB_WIDTH,
            "Formatted tab width {} exceeds MAX_TAB_WIDTH {}",
            long_proj_tab.chars().count(),
            MAX_TAB_WIDTH
        );
        assert!(long_proj_tab.contains('…'));
        assert!(long_proj_tab.contains("[tpl]"));

        let long_proj_spans = StatusBar::format_tab_spans(0, &entry_long_proj, true, &theme);
        let long_proj_spans_len: usize = long_proj_spans
            .iter()
            .map(|s| s.content.chars().count())
            .sum();
        assert!(long_proj_spans_len <= MAX_TAB_WIDTH);

        // 3. Long template name (50+ chars) is cleanly truncated to fit within MAX_TAB_WIDTH
        let long_tpl = "extra-long-template-name-exceeding-fifty-characters-in-length-easily";
        assert!(long_tpl.chars().count() > 50);
        let (ps_long_tpl, pty_long_tpl, vt_long_tpl) =
            create_test_session_with_status("service", long_tpl, "claude", SessionStatus::Busy);
        let entry_long_tpl = SessionEntry::new(ps_long_tpl, pty_long_tpl, vt_long_tpl);

        let long_tpl_tab = StatusBar::format_tab(0, &entry_long_tpl, true);
        assert!(
            long_tpl_tab.chars().count() <= MAX_TAB_WIDTH,
            "Formatted tab width {} exceeds MAX_TAB_WIDTH {}",
            long_tpl_tab.chars().count(),
            MAX_TAB_WIDTH
        );
        assert!(long_tpl_tab.contains('…'));
        assert!(long_tpl_tab.contains("service"));

        // 4. Both template and project 50+ chars
        let (ps_both_long, pty_both_long, vt_both_long) =
            create_test_session_with_status(long_proj, long_tpl, "claude", SessionStatus::Busy);
        let entry_both_long = SessionEntry::new(ps_both_long, pty_both_long, vt_both_long);

        let both_long_tab = StatusBar::format_tab(0, &entry_both_long, true);
        assert!(both_long_tab.chars().count() <= MAX_TAB_WIDTH);
        assert!(both_long_tab.contains('…'));

        // 5. Long project with empty template omits brackets and fits MAX_TAB_WIDTH
        let (ps_no_tpl, pty_no_tpl, vt_no_tpl) =
            create_test_session_with_status(long_proj, "", "claude", SessionStatus::Busy);
        let entry_no_tpl = SessionEntry::new(ps_no_tpl, pty_no_tpl, vt_no_tpl);

        let no_tpl_tab = StatusBar::format_tab(0, &entry_no_tpl, true);
        assert!(no_tpl_tab.chars().count() <= MAX_TAB_WIDTH);
        assert!(!no_tpl_tab.contains('[') && !no_tpl_tab.contains(']'));
        assert!(no_tpl_tab.contains('…'));
    }
}
