use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    buffer::Buffer,
    layout::{Rect, Size},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Widget},
};

use crate::tui::modal_frame::{centered_modal, render_modal_frame};
use crate::tui::theme::Theme;

/// Outcome of a key event handled by [`QuitModalState`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuitModalAction {
    Confirm,
    Cancel,
}

/// State for the quit confirmation modal shown when active sessions are running.
///
/// Snapshots the active session count at the moment the modal opens and waits for explicit user
/// input without auto-closing or mutating the count if background sessions exit while open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuitModalState {
    pub running_sessions: usize,
}

impl QuitModalState {
    /// Creates a new `QuitModalState` with the snapshotted count of running sessions.
    pub fn new(running_sessions: usize) -> Self {
        Self { running_sessions }
    }

    /// Handles keyboard events within the quit confirmation modal.
    ///
    /// - `y` / `Y` / `Enter` (without Ctrl or Alt modifiers) -> `Some(QuitModalAction::Confirm)`
    /// - `n` / `N` / `Esc` (without Ctrl or Alt modifiers) -> `Some(QuitModalAction::Cancel)`
    /// - All other keys -> `None`
    pub fn handle_key(&mut self, key: KeyEvent) -> Option<QuitModalAction> {
        if key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            return None;
        }
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                Some(QuitModalAction::Confirm)
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => Some(QuitModalAction::Cancel),
            _ => None,
        }
    }
}

/// Renders the centered floating quit confirmation dialog into `buf`.
pub fn render_quit_modal(modal: &QuitModalState, area: Rect, buf: &mut Buffer, theme: &Theme) {
    let Some(modal_area) = centered_modal(area, Size::new(76, 8), Size::new(20, 5)) else {
        return;
    };
    let inner = render_modal_frame(modal_area, buf, " Confirm Quit ", theme.status_waiting);

    let count_str = modal.running_sessions.to_string();
    let noun = if modal.running_sessions == 1 {
        "session"
    } else {
        "sessions"
    };

    let lines = vec![
        Line::from(vec![
            Span::raw("You have "),
            Span::styled(
                count_str,
                Style::default()
                    .fg(theme.modal_title)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!(" running {noun}. Quit botai?")),
        ]),
        Line::from(Span::styled(
            "Quitting will terminate active sessions (project files stay on disk).",
            Style::default().fg(theme.status_bar_fg),
        )),
        Line::from(""),
        Line::from(Span::styled(
            " [y] / [Enter] Confirm  |  [n] / [Esc] Cancel ",
            Style::default()
                .fg(theme.key_hint)
                .add_modifier(Modifier::BOLD),
        )),
    ];

    Paragraph::new(lines).render(inner, buf);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn key_mod(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    #[test]
    fn test_quit_modal_handle_key_confirm_cancel_and_ignored() {
        let mut modal = QuitModalState::new(2);

        // Confirm keys
        assert_eq!(
            modal.handle_key(key(KeyCode::Char('y'))),
            Some(QuitModalAction::Confirm)
        );
        assert_eq!(
            modal.handle_key(key(KeyCode::Char('Y'))),
            Some(QuitModalAction::Confirm)
        );
        assert_eq!(
            modal.handle_key(key_mod(KeyCode::Char('Y'), KeyModifiers::SHIFT)),
            Some(QuitModalAction::Confirm)
        );
        assert_eq!(
            modal.handle_key(key(KeyCode::Enter)),
            Some(QuitModalAction::Confirm)
        );

        // Cancel keys
        assert_eq!(
            modal.handle_key(key(KeyCode::Char('n'))),
            Some(QuitModalAction::Cancel)
        );
        assert_eq!(
            modal.handle_key(key(KeyCode::Char('N'))),
            Some(QuitModalAction::Cancel)
        );
        assert_eq!(
            modal.handle_key(key_mod(KeyCode::Char('N'), KeyModifiers::SHIFT)),
            Some(QuitModalAction::Cancel)
        );
        assert_eq!(
            modal.handle_key(key(KeyCode::Esc)),
            Some(QuitModalAction::Cancel)
        );

        // Ignored keys
        assert_eq!(modal.handle_key(key(KeyCode::Char('q'))), None);
        assert_eq!(modal.handle_key(key(KeyCode::Char(' '))), None);
        assert_eq!(modal.handle_key(key(KeyCode::Tab)), None);
        assert_eq!(modal.handle_key(key(KeyCode::Up)), None);
        assert_eq!(modal.handle_key(key(KeyCode::Backspace)), None);

        // Ctrl and Alt modifier guards reject confirm/cancel keys
        assert_eq!(
            modal.handle_key(key_mod(KeyCode::Char('y'), KeyModifiers::CONTROL)),
            None
        );
        assert_eq!(
            modal.handle_key(key_mod(KeyCode::Char('Y'), KeyModifiers::ALT)),
            None
        );
        assert_eq!(
            modal.handle_key(key_mod(KeyCode::Enter, KeyModifiers::CONTROL)),
            None
        );
        assert_eq!(
            modal.handle_key(key_mod(KeyCode::Enter, KeyModifiers::ALT)),
            None
        );
        assert_eq!(
            modal.handle_key(key_mod(KeyCode::Char('n'), KeyModifiers::CONTROL)),
            None
        );
        assert_eq!(
            modal.handle_key(key_mod(KeyCode::Char('n'), KeyModifiers::ALT)),
            None
        );
        assert_eq!(
            modal.handle_key(key_mod(KeyCode::Esc, KeyModifiers::CONTROL)),
            None
        );
        assert_eq!(
            modal.handle_key(key_mod(KeyCode::Esc, KeyModifiers::ALT)),
            None
        );
    }

    #[test]
    fn test_render_quit_modal_singular_and_border_color() {
        let modal = QuitModalState::new(1);
        let area = Rect::new(0, 0, 100, 30);
        let mut buf = Buffer::empty(area);
        let theme = Theme::tokyo_night();

        render_quit_modal(&modal, area, &mut buf, &theme);

        let rendered: String = buf
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect();

        assert!(rendered.contains("Confirm Quit"));
        assert!(rendered.contains("You have 1 running session. Quit botai?"));
        assert!(!rendered.contains("1 running sessions"));
        assert!(rendered.contains("Quitting will terminate active sessions"));
        assert!(rendered.contains("[y] / [Enter] Confirm  |  [n] / [Esc] Cancel"));

        // Verify centered modal border uses theme.status_waiting
        let modal_area = centered_modal(area, Size::new(76, 8), Size::new(20, 5)).unwrap();
        let top_left = buf.cell((modal_area.x, modal_area.y)).unwrap();
        assert_eq!(top_left.fg, theme.status_waiting);
    }

    #[test]
    fn test_render_quit_modal_plural() {
        let modal = QuitModalState::new(3);
        let area = Rect::new(0, 0, 100, 30);
        let mut buf = Buffer::empty(area);
        let theme = Theme::gruvbox();

        render_quit_modal(&modal, area, &mut buf, &theme);

        let rendered: String = buf
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect();

        assert!(rendered.contains("Confirm Quit"));
        assert!(rendered.contains("You have 3 running sessions. Quit botai?"));
        assert!(rendered.contains("[y] / [Enter] Confirm  |  [n] / [Esc] Cancel"));

        let modal_area = centered_modal(area, Size::new(76, 8), Size::new(20, 5)).unwrap();
        let top_left = buf.cell((modal_area.x, modal_area.y)).unwrap();
        assert_eq!(top_left.fg, theme.status_waiting);
    }

    #[test]
    fn test_render_quit_modal_tiny_viewport_guard() {
        let modal = QuitModalState::new(2);
        let theme = Theme::default();

        // Width too narrow (< 20 after 4-col margin => area.width < 24)
        let narrow = Rect::new(0, 0, 20, 10);
        let mut buf_narrow = Buffer::empty(narrow);
        render_quit_modal(&modal, narrow, &mut buf_narrow, &theme);
        let rendered_narrow: String = buf_narrow
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect();
        assert!(!rendered_narrow.contains("Confirm Quit"));

        // Height too short (< 5 after 2-row margin => area.height < 7)
        let short = Rect::new(0, 0, 80, 6);
        let mut buf_short = Buffer::empty(short);
        render_quit_modal(&modal, short, &mut buf_short, &theme);
        let rendered_short: String = buf_short
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect();
        assert!(!rendered_short.contains("Confirm Quit"));
    }
}
