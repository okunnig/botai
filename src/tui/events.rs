use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// High-level actions triggered by user keyboard events or internal TUI timers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TuiAction {
    /// Switches to the session at the specified 0-based index (`Alt + 1` .. `Alt + 9`).
    SwitchSession(usize),
    /// Cycles to the next active session (`Alt + n` / `Alt + N`).
    NextSession,
    /// Cycles to the previous active session (`Alt + p` / `Alt + P`).
    PrevSession,
    /// Detaches from the active session (`Alt + d` / `Alt + D`).
    Detach,
    /// Closes / terminates the active session (`Alt + x` / `Alt + X`).
    CloseSession,
    /// Opens the in-session Quick Project Launcher modal (`Alt + c` / `Alt + C`).
    OpenLauncher,
    /// Quits the entire botai dashboard (`Alt + q` / `Alt + Q`).
    Quit,
    /// Raw byte sequence forwarded directly to the active session's PTY.
    Passthrough(Vec<u8>),
    /// Window or terminal resize event with new columns and rows.
    Resize(u16, u16),
    /// Periodic tick for UI animations, badges, and polling.
    Tick,
}

/// Handler for converting crossterm input events into multiplexer actions or raw PTY bytes.
pub struct KeyHandler;

impl KeyHandler {
    /// Maps a crossterm `KeyEvent` into a `TuiAction`.
    pub fn map_key_event(event: KeyEvent) -> TuiAction {
        map_key_event(event)
    }

    /// Converts a crossterm `KeyEvent` into raw terminal input bytes suitable for writing to a PTY.
    pub fn key_to_bytes(event: &KeyEvent) -> Vec<u8> {
        key_to_bytes(event)
    }
}

/// Maps a crossterm `KeyEvent` into a high-level `TuiAction`.
pub fn map_key_event(event: KeyEvent) -> TuiAction {
    let has_alt = event.modifiers.contains(KeyModifiers::ALT);
    let has_ctrl = event.modifiers.contains(KeyModifiers::CONTROL);

    if has_alt && !has_ctrl {
        match event.code {
            KeyCode::Char(c) if c.is_ascii_digit() && c != '0' => {
                let index = (c.to_digit(10).unwrap() - 1) as usize;
                return TuiAction::SwitchSession(index);
            }
            KeyCode::Char('n') | KeyCode::Char('N') => return TuiAction::NextSession,
            KeyCode::Char('p') | KeyCode::Char('P') => return TuiAction::PrevSession,
            KeyCode::Char('c') | KeyCode::Char('C') => return TuiAction::OpenLauncher,
            KeyCode::Char('d') | KeyCode::Char('D') => return TuiAction::Detach,
            KeyCode::Char('x') | KeyCode::Char('X') => return TuiAction::CloseSession,
            KeyCode::Char('q') | KeyCode::Char('Q') => return TuiAction::Quit,
            _ => {}
        }
    }

    TuiAction::Passthrough(key_to_bytes(&event))
}

/// Converts a crossterm `KeyEvent` into standard ANSI / VT byte sequences for PTY passthrough.
pub fn key_to_bytes(event: &KeyEvent) -> Vec<u8> {
    let has_ctrl = event.modifiers.contains(KeyModifiers::CONTROL);
    let has_alt = event.modifiers.contains(KeyModifiers::ALT);

    if has_ctrl {
        if let KeyCode::Char(c) = event.code {
            let lower = c.to_ascii_lowercase();
            if lower.is_ascii_alphabetic() {
                return vec![lower as u8 - b'a' + 1];
            }
            return match c {
                '@' | ' ' => vec![0x00],
                '[' => vec![0x1b],
                '\\' => vec![0x1c],
                ']' => vec![0x1d],
                '^' => vec![0x1e],
                '_' | '/' => vec![0x1f],
                _ => c.to_string().into_bytes(),
            };
        }
    }

    let bytes = match event.code {
        KeyCode::Char(c) => c.to_string().into_bytes(),
        KeyCode::Enter => b"\r".to_vec(),
        KeyCode::Backspace => vec![0x7f],
        KeyCode::Tab => b"\t".to_vec(),
        KeyCode::BackTab => b"\x1b[Z".to_vec(),
        KeyCode::Esc => vec![0x1b],
        KeyCode::Up => b"\x1b[A".to_vec(),
        KeyCode::Down => b"\x1b[B".to_vec(),
        KeyCode::Right => b"\x1b[C".to_vec(),
        KeyCode::Left => b"\x1b[D".to_vec(),
        KeyCode::Home => b"\x1b[H".to_vec(),
        KeyCode::End => b"\x1b[F".to_vec(),
        KeyCode::PageUp => b"\x1b[5~".to_vec(),
        KeyCode::PageDown => b"\x1b[6~".to_vec(),
        KeyCode::Delete => b"\x1b[3~".to_vec(),
        KeyCode::Insert => b"\x1b[2~".to_vec(),
        KeyCode::F(n) => match n {
            1 => b"\x1bOP".to_vec(),
            2 => b"\x1bOQ".to_vec(),
            3 => b"\x1bOR".to_vec(),
            4 => b"\x1bOS".to_vec(),
            5 => b"\x1b[15~".to_vec(),
            6 => b"\x1b[17~".to_vec(),
            7 => b"\x1b[18~".to_vec(),
            8 => b"\x1b[19~".to_vec(),
            9 => b"\x1b[20~".to_vec(),
            10 => b"\x1b[21~".to_vec(),
            11 => b"\x1b[23~".to_vec(),
            12 => b"\x1b[24~".to_vec(),
            _ => Vec::new(),
        },
        _ => Vec::new(),
    };

    // If Alt modifier is active on a regular key, prepend ESC (0x1b)
    if has_alt && !has_ctrl && !bytes.is_empty() {
        let mut alt_seq = vec![0x1b];
        alt_seq.extend(bytes);
        return alt_seq;
    }

    bytes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEventKind, KeyEventState};

    fn make_key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            state: KeyEventState::empty(),
        }
    }

    #[test]
    fn test_map_key_event_alt_hotkeys_switch_session() {
        assert_eq!(
            map_key_event(make_key(KeyCode::Char('1'), KeyModifiers::ALT)),
            TuiAction::SwitchSession(0)
        );
        assert_eq!(
            map_key_event(make_key(KeyCode::Char('2'), KeyModifiers::ALT)),
            TuiAction::SwitchSession(1)
        );
        assert_eq!(
            map_key_event(make_key(KeyCode::Char('9'), KeyModifiers::ALT)),
            TuiAction::SwitchSession(8)
        );
    }

    #[test]
    fn test_map_key_event_alt_navigation_next_prev() {
        assert_eq!(
            map_key_event(make_key(KeyCode::Char('n'), KeyModifiers::ALT)),
            TuiAction::NextSession
        );
        assert_eq!(
            map_key_event(make_key(KeyCode::Char('N'), KeyModifiers::ALT)),
            TuiAction::NextSession
        );
        assert_eq!(
            map_key_event(make_key(KeyCode::Char('p'), KeyModifiers::ALT)),
            TuiAction::PrevSession
        );
        assert_eq!(
            map_key_event(make_key(KeyCode::Char('P'), KeyModifiers::ALT)),
            TuiAction::PrevSession
        );
    }

    #[test]
    fn test_map_key_event_alt_lifecycle_actions() {
        assert_eq!(
            map_key_event(make_key(KeyCode::Char('c'), KeyModifiers::ALT)),
            TuiAction::OpenLauncher
        );
        assert_eq!(
            map_key_event(make_key(KeyCode::Char('C'), KeyModifiers::ALT)),
            TuiAction::OpenLauncher
        );
        assert_eq!(
            map_key_event(make_key(KeyCode::Char('d'), KeyModifiers::ALT)),
            TuiAction::Detach
        );
        assert_eq!(
            map_key_event(make_key(KeyCode::Char('D'), KeyModifiers::ALT)),
            TuiAction::Detach
        );
        assert_eq!(
            map_key_event(make_key(KeyCode::Char('x'), KeyModifiers::ALT)),
            TuiAction::CloseSession
        );
        assert_eq!(
            map_key_event(make_key(KeyCode::Char('X'), KeyModifiers::ALT)),
            TuiAction::CloseSession
        );
        assert_eq!(
            map_key_event(make_key(KeyCode::Char('q'), KeyModifiers::ALT)),
            TuiAction::Quit
        );
        assert_eq!(
            map_key_event(make_key(KeyCode::Char('Q'), KeyModifiers::ALT)),
            TuiAction::Quit
        );
    }

    #[test]
    fn test_key_to_bytes_control_characters() {
        // Ctrl+C -> 0x03 (ETX)
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            vec![0x03]
        );
        // Ctrl+D -> 0x04 (EOT)
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::Char('d'), KeyModifiers::CONTROL)),
            vec![0x04]
        );
        // Ctrl+A -> 0x01
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::Char('a'), KeyModifiers::CONTROL)),
            vec![0x01]
        );
        // Ctrl+Z -> 0x1a
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::Char('z'), KeyModifiers::CONTROL)),
            vec![0x1a]
        );
        // Ctrl+Space -> 0x00
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::Char(' '), KeyModifiers::CONTROL)),
            vec![0x00]
        );
        // Ctrl+[ -> 0x1b (ESC)
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::Char('['), KeyModifiers::CONTROL)),
            vec![0x1b]
        );
    }

    #[test]
    fn test_key_to_bytes_printable_and_utf8() {
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::Char('h'), KeyModifiers::empty())),
            b"h".to_vec()
        );
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::Char('A'), KeyModifiers::SHIFT)),
            b"A".to_vec()
        );
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::Char('🚀'), KeyModifiers::empty())),
            "🚀".as_bytes().to_vec()
        );
    }

    #[test]
    fn test_key_to_bytes_standard_keys() {
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::Enter, KeyModifiers::empty())),
            b"\r".to_vec()
        );
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::Backspace, KeyModifiers::empty())),
            vec![0x7f]
        );
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::Tab, KeyModifiers::empty())),
            b"\t".to_vec()
        );
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::BackTab, KeyModifiers::SHIFT)),
            b"\x1b[Z".to_vec()
        );
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::Esc, KeyModifiers::empty())),
            vec![0x1b]
        );
    }

    #[test]
    fn test_key_to_bytes_arrows_and_navigation() {
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::Up, KeyModifiers::empty())),
            b"\x1b[A".to_vec()
        );
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::Down, KeyModifiers::empty())),
            b"\x1b[B".to_vec()
        );
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::Right, KeyModifiers::empty())),
            b"\x1b[C".to_vec()
        );
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::Left, KeyModifiers::empty())),
            b"\x1b[D".to_vec()
        );
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::Home, KeyModifiers::empty())),
            b"\x1b[H".to_vec()
        );
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::End, KeyModifiers::empty())),
            b"\x1b[F".to_vec()
        );
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::PageUp, KeyModifiers::empty())),
            b"\x1b[5~".to_vec()
        );
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::PageDown, KeyModifiers::empty())),
            b"\x1b[6~".to_vec()
        );
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::Delete, KeyModifiers::empty())),
            b"\x1b[3~".to_vec()
        );
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::Insert, KeyModifiers::empty())),
            b"\x1b[2~".to_vec()
        );
    }

    #[test]
    fn test_key_to_bytes_function_keys() {
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::F(1), KeyModifiers::empty())),
            b"\x1bOP".to_vec()
        );
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::F(4), KeyModifiers::empty())),
            b"\x1bOS".to_vec()
        );
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::F(5), KeyModifiers::empty())),
            b"\x1b[15~".to_vec()
        );
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::F(12), KeyModifiers::empty())),
            b"\x1b[24~".to_vec()
        );
    }

    #[test]
    fn test_key_handler_passthrough_action() {
        let key = make_key(KeyCode::Char('k'), KeyModifiers::empty());
        let action = KeyHandler::map_key_event(key);
        assert_eq!(action, TuiAction::Passthrough(b"k".to_vec()));
    }
}
