use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// High-level actions triggered by user keyboard events or internal TUI timers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TuiAction {
    /// Enters leader mode (`Ctrl + Space`).
    EnterLeader,
    /// Cancels leader mode (`Esc` or unrecognized key while leader is active).
    CancelLeader,
    /// Switches to the session at the specified 0-based index (`1` .. `9`).
    SwitchSession(usize),
    /// Cycles to the next active session (`n` / `N`).
    NextSession,
    /// Cycles to the previous active session (`p` / `P`).
    PrevSession,
    /// Detaches from the active session (`d` / `D`).
    Detach,
    /// Closes / terminates the active session (`w` / `W` or `x` / `X`).
    CloseSession,
    /// Opens the in-session Quick Project Launcher modal (`c` / `C`).
    OpenLauncher,
    /// Quits the entire botai dashboard (`q` / `Q`).
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
    /// Maps a crossterm `KeyEvent` and leader state into a `TuiAction`.
    pub fn map_key_event(event: KeyEvent, leader_active: bool) -> TuiAction {
        map_key_event(event, leader_active)
    }

    /// Checks if a crossterm `KeyEvent` represents `Ctrl+Space`.
    pub fn is_ctrl_space(event: &KeyEvent) -> bool {
        is_ctrl_space(event)
    }

    /// Converts a crossterm `KeyEvent` into raw terminal input bytes suitable for writing to a PTY.
    pub fn key_to_bytes(event: &KeyEvent) -> Vec<u8> {
        key_to_bytes(event)
    }
}

/// Checks if a crossterm `KeyEvent` represents `Ctrl+Space` across various terminal encodings.
pub fn is_ctrl_space(event: &KeyEvent) -> bool {
    let has_ctrl = event.modifiers.contains(KeyModifiers::CONTROL);
    match event.code {
        KeyCode::Char(' ') if has_ctrl => true,
        KeyCode::Char('@') if has_ctrl => true,
        KeyCode::Char('\0') => true,
        KeyCode::Null => true,
        _ => false,
    }
}

/// Maps a crossterm `KeyEvent` and leader state into a high-level `TuiAction`.
pub fn map_key_event(event: KeyEvent, leader_active: bool) -> TuiAction {
    if !leader_active {
        if is_ctrl_space(&event) {
            return TuiAction::EnterLeader;
        }
        return TuiAction::Passthrough(key_to_bytes(&event));
    }

    // Leader mode is active
    if is_ctrl_space(&event) {
        return TuiAction::Passthrough(vec![0x00]);
    }

    match event.code {
        KeyCode::Char(c) if ('1'..='9').contains(&c) => {
            let index = (c.to_digit(10).unwrap() - 1) as usize;
            TuiAction::SwitchSession(index)
        }
        KeyCode::Char('c') | KeyCode::Char('C') => TuiAction::OpenLauncher,
        KeyCode::Char('n') | KeyCode::Char('N') => TuiAction::NextSession,
        KeyCode::Char('p') | KeyCode::Char('P') => TuiAction::PrevSession,
        KeyCode::Char('w') | KeyCode::Char('W') | KeyCode::Char('x') | KeyCode::Char('X') => {
            TuiAction::CloseSession
        }
        KeyCode::Char('d') | KeyCode::Char('D') => TuiAction::Detach,
        KeyCode::Char('q') | KeyCode::Char('Q') => TuiAction::Quit,
        KeyCode::Char(' ') => TuiAction::Passthrough(vec![0x00]),
        _ => TuiAction::CancelLeader,
    }
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
                '@' | ' ' | '\0' => vec![0x00],
                '[' => vec![0x1b],
                '\\' => vec![0x1c],
                ']' => vec![0x1d],
                '^' => vec![0x1e],
                '_' | '/' => vec![0x1f],
                _ => c.to_string().into_bytes(),
            };
        }
    }

    if event.code == KeyCode::Null || event.code == KeyCode::Char('\0') {
        return vec![0x00];
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
    fn test_is_ctrl_space_recognition() {
        // Ctrl+Space
        assert!(is_ctrl_space(&make_key(KeyCode::Char(' '), KeyModifiers::CONTROL)));
        // Ctrl+@
        assert!(is_ctrl_space(&make_key(KeyCode::Char('@'), KeyModifiers::CONTROL)));
        // Char('\0')
        assert!(is_ctrl_space(&make_key(KeyCode::Char('\0'), KeyModifiers::NONE)));
        assert!(is_ctrl_space(&make_key(KeyCode::Char('\0'), KeyModifiers::CONTROL)));
        // KeyCode::Null
        assert!(is_ctrl_space(&make_key(KeyCode::Null, KeyModifiers::NONE)));

        // Non-Ctrl+Space
        assert!(!is_ctrl_space(&make_key(KeyCode::Char(' '), KeyModifiers::NONE)));
        assert!(!is_ctrl_space(&make_key(KeyCode::Char('a'), KeyModifiers::CONTROL)));
        assert!(!is_ctrl_space(&make_key(KeyCode::Esc, KeyModifiers::NONE)));
    }

    #[test]
    fn test_map_key_event_when_leader_inactive() {
        // Ctrl+Space enters leader mode
        assert_eq!(
            map_key_event(make_key(KeyCode::Char(' '), KeyModifiers::CONTROL), false),
            TuiAction::EnterLeader
        );
        assert_eq!(
            map_key_event(make_key(KeyCode::Char('@'), KeyModifiers::CONTROL), false),
            TuiAction::EnterLeader
        );
        assert_eq!(
            map_key_event(make_key(KeyCode::Char('\0'), KeyModifiers::NONE), false),
            TuiAction::EnterLeader
        );

        // All other keys pass through
        assert_eq!(
            map_key_event(make_key(KeyCode::Char('1'), KeyModifiers::NONE), false),
            TuiAction::Passthrough(b"1".to_vec())
        );
        assert_eq!(
            map_key_event(make_key(KeyCode::Char('c'), KeyModifiers::NONE), false),
            TuiAction::Passthrough(b"c".to_vec())
        );
        assert_eq!(
            map_key_event(make_key(KeyCode::Enter, KeyModifiers::NONE), false),
            TuiAction::Passthrough(b"\r".to_vec())
        );
    }

    #[test]
    fn test_map_key_event_when_leader_active_commands() {
        // 1..9 switch session
        assert_eq!(
            map_key_event(make_key(KeyCode::Char('1'), KeyModifiers::NONE), true),
            TuiAction::SwitchSession(0)
        );
        assert_eq!(
            map_key_event(make_key(KeyCode::Char('5'), KeyModifiers::NONE), true),
            TuiAction::SwitchSession(4)
        );
        assert_eq!(
            map_key_event(make_key(KeyCode::Char('9'), KeyModifiers::NONE), true),
            TuiAction::SwitchSession(8)
        );

        // c / C opens quick launcher
        assert_eq!(
            map_key_event(make_key(KeyCode::Char('c'), KeyModifiers::NONE), true),
            TuiAction::OpenLauncher
        );
        assert_eq!(
            map_key_event(make_key(KeyCode::Char('C'), KeyModifiers::SHIFT), true),
            TuiAction::OpenLauncher
        );

        // n / N next session
        assert_eq!(
            map_key_event(make_key(KeyCode::Char('n'), KeyModifiers::NONE), true),
            TuiAction::NextSession
        );
        assert_eq!(
            map_key_event(make_key(KeyCode::Char('N'), KeyModifiers::SHIFT), true),
            TuiAction::NextSession
        );

        // p / P prev session
        assert_eq!(
            map_key_event(make_key(KeyCode::Char('p'), KeyModifiers::NONE), true),
            TuiAction::PrevSession
        );
        assert_eq!(
            map_key_event(make_key(KeyCode::Char('P'), KeyModifiers::SHIFT), true),
            TuiAction::PrevSession
        );

        // w / W and x / X close session
        assert_eq!(
            map_key_event(make_key(KeyCode::Char('w'), KeyModifiers::NONE), true),
            TuiAction::CloseSession
        );
        assert_eq!(
            map_key_event(make_key(KeyCode::Char('W'), KeyModifiers::SHIFT), true),
            TuiAction::CloseSession
        );
        assert_eq!(
            map_key_event(make_key(KeyCode::Char('x'), KeyModifiers::NONE), true),
            TuiAction::CloseSession
        );
        assert_eq!(
            map_key_event(make_key(KeyCode::Char('X'), KeyModifiers::SHIFT), true),
            TuiAction::CloseSession
        );

        // d / D detach
        assert_eq!(
            map_key_event(make_key(KeyCode::Char('d'), KeyModifiers::NONE), true),
            TuiAction::Detach
        );
        assert_eq!(
            map_key_event(make_key(KeyCode::Char('D'), KeyModifiers::SHIFT), true),
            TuiAction::Detach
        );

        // q / Q quit
        assert_eq!(
            map_key_event(make_key(KeyCode::Char('q'), KeyModifiers::NONE), true),
            TuiAction::Quit
        );
        assert_eq!(
            map_key_event(make_key(KeyCode::Char('Q'), KeyModifiers::SHIFT), true),
            TuiAction::Quit
        );

        // Space / Ctrl+Space forwards literal 0x00
        assert_eq!(
            map_key_event(make_key(KeyCode::Char(' '), KeyModifiers::NONE), true),
            TuiAction::Passthrough(vec![0x00])
        );
        assert_eq!(
            map_key_event(make_key(KeyCode::Char(' '), KeyModifiers::CONTROL), true),
            TuiAction::Passthrough(vec![0x00])
        );

        // Esc or any unrecognized key cancels leader mode
        assert_eq!(
            map_key_event(make_key(KeyCode::Esc, KeyModifiers::NONE), true),
            TuiAction::CancelLeader
        );
        assert_eq!(
            map_key_event(make_key(KeyCode::Char('z'), KeyModifiers::NONE), true),
            TuiAction::CancelLeader
        );
        assert_eq!(
            map_key_event(make_key(KeyCode::Up, KeyModifiers::NONE), true),
            TuiAction::CancelLeader
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
        let action = KeyHandler::map_key_event(key, false);
        assert_eq!(action, TuiAction::Passthrough(b"k".to_vec()));
    }
}
