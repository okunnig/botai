use crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, KeyboardEnhancementFlags, MouseEvent, MouseEventKind,
};

/// modifyOtherKeys mode configuration for host terminal compatibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModifyOtherKeysMode {
    /// Mode 1 (`\x1b[>4;1m`): reports modified keys for WezTerm / Alacritty.
    Mode1,
    /// Mode 2 (`\x1b[>4;2m`): reports all modified keys including within tmux.
    Mode2,
}

impl ModifyOtherKeysMode {
    /// Returns the ANSI escape sequence to enable this `modifyOtherKeys` mode.
    pub fn enable_sequence(&self) -> &'static [u8] {
        match self {
            ModifyOtherKeysMode::Mode1 => b"\x1b[>4;1m",
            ModifyOtherKeysMode::Mode2 => b"\x1b[>4;2m",
        }
    }

    /// Returns the ANSI escape sequence to reset / disable `modifyOtherKeys` mode.
    pub fn reset_sequence() -> &'static [u8] {
        b"\x1b[>4;0m"
    }
}

/// Detects the recommended `modifyOtherKeys` mode based on explicit environment variables.
///
/// On Unix, `modifyOtherKeys` is not enabled for Alacritty or standard terminals because
/// Alacritty 0.13+ and modern terminals natively support the Kitty Keyboard Protocol
/// (`KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES`). Sending `\x1b[>4;1m` (`modifyOtherKeys` Mode 1)
/// overrides Alacritty's Kitty mode and suppresses Shift+Enter escape sequences.
/// `ModifyOtherKeysMode::Mode2` is only enabled when running inside `tmux` (`in_tmux == true`).
pub fn host_modify_other_keys_mode_for_env(
    in_tmux: bool,
    _term_program: Option<&str>,
    _wezterm_pane: bool,
    _alacritty_window_id: bool,
) -> Option<ModifyOtherKeysMode> {
    if in_tmux {
        return Some(ModifyOtherKeysMode::Mode2);
    }
    None
}

/// Detects the recommended `modifyOtherKeys` mode for the current process environment.
pub fn host_modify_other_keys_mode() -> Option<ModifyOtherKeysMode> {
    let in_tmux = std::env::var("TMUX").is_ok();
    let term_program = std::env::var("TERM_PROGRAM").ok();
    let wezterm_pane = std::env::var("WEZTERM_PANE").is_ok();
    let alacritty_window_id = std::env::var("ALACRITTY_WINDOW_ID").is_ok();

    host_modify_other_keys_mode_for_env(
        in_tmux,
        term_program.as_deref(),
        wezterm_pane,
        alacritty_window_id,
    )
}

/// Returns the keyboard enhancement flags (`KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES`).
///
/// Returning ONLY `DISAMBIGUATE_ESCAPE_CODES` (`\x1b[>1u`) ensures terminals like Alacritty and agy
/// disambiguate modified keys (e.g. Shift+Enter -> `\x1b[13;2u`) without sending alternate key
/// codepoints (`REPORT_ALTERNATE_KEYS` -> `\x1b[13:10;2u`) which crossterm strips the Shift modifier from.
pub fn keyboard_enhancement_flags() -> KeyboardEnhancementFlags {
    KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
}

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

    /// Formats a text slice as a standard bracketed paste escape sequence (`\x1b[200~<text>\x1b[201~`).
    pub fn paste_to_bytes(text: &str) -> Vec<u8> {
        paste_to_bytes(text)
    }

    /// Computes the CSI-u / Xterm modifier parameter value (1..16).
    pub fn modifier_code(modifiers: KeyModifiers) -> u8 {
        modifier_code(modifiers)
    }

    /// Converts a crossterm `MouseEventKind` into standard ANSI / VT escape sequences for scrolling.
    pub fn mouse_scroll_to_bytes(kind: MouseEventKind) -> Vec<u8> {
        mouse_scroll_to_bytes(kind)
    }

    /// Converts a crossterm `MouseEvent` into standard ANSI / VT escape sequences for scrolling.
    pub fn mouse_event_to_bytes(event: &MouseEvent) -> Vec<u8> {
        mouse_event_to_bytes(event)
    }

    /// Converts a crossterm `MouseEvent` into standard SGR (1006) ANSI escape sequences for child processes with mouse reporting enabled.
    pub fn mouse_event_to_sgr_bytes(event: &MouseEvent) -> Vec<u8> {
        mouse_event_to_sgr_bytes(event)
    }

    /// Returns the keyboard enhancement flags (`KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES`).
    pub fn keyboard_enhancement_flags() -> KeyboardEnhancementFlags {
        keyboard_enhancement_flags()
    }

    /// Detects the recommended `modifyOtherKeys` mode for the current process environment.
    pub fn host_modify_other_keys_mode() -> Option<ModifyOtherKeysMode> {
        host_modify_other_keys_mode()
    }

    /// Detects the recommended `modifyOtherKeys` mode based on explicit environment variables.
    pub fn host_modify_other_keys_mode_for_env(
        in_tmux: bool,
        term_program: Option<&str>,
        wezterm_pane: bool,
        alacritty_window_id: bool,
    ) -> Option<ModifyOtherKeysMode> {
        host_modify_other_keys_mode_for_env(
            in_tmux,
            term_program,
            wezterm_pane,
            alacritty_window_id,
        )
    }
}

/// Computes the CSI-u / Xterm modifier parameter value:
/// `1 + (Shift ? 1 : 0) + (Alt ? 2 : 0) + (Ctrl ? 4 : 0) + (Super/Meta ? 8 : 0)`.
pub fn modifier_code(modifiers: KeyModifiers) -> u8 {
    let mut val = 1u8;
    if modifiers.contains(KeyModifiers::SHIFT) {
        val += 1;
    }
    if modifiers.contains(KeyModifiers::ALT) {
        val += 2;
    }
    if modifiers.contains(KeyModifiers::CONTROL) {
        val += 4;
    }
    if modifiers.contains(KeyModifiers::SUPER)
        || modifiers.contains(KeyModifiers::META)
        || modifiers.contains(KeyModifiers::HYPER)
    {
        val += 8;
    }
    val
}

/// Formats a text slice as a standard bracketed paste escape sequence (`\x1b[200~<text>\x1b[201~`).
pub fn paste_to_bytes(text: &str) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(text.len() + 12);
    bytes.extend_from_slice(b"\x1b[200~");
    bytes.extend_from_slice(text.as_bytes());
    bytes.extend_from_slice(b"\x1b[201~");
    bytes
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

/// Converts a crossterm `KeyEvent` into standard ANSI / VT / CSI-u byte sequences for PTY passthrough.
pub fn key_to_bytes(event: &KeyEvent) -> Vec<u8> {
    if event.code == KeyCode::Null || event.code == KeyCode::Char('\0') {
        return vec![0x00];
    }

    let m = modifier_code(event.modifiers);

    // Handle character keys
    if let KeyCode::Char(c) = event.code {
        if c == '\n' || c == '\r' {
            if m == 1 {
                return if c == '\n' {
                    b"\n".to_vec()
                } else {
                    b"\r".to_vec()
                };
            } else {
                return format!("\x1b[13;{m}u").into_bytes();
            }
        }

        if event.modifiers.is_empty() || event.modifiers == KeyModifiers::SHIFT {
            return c.to_string().into_bytes();
        }

        if event.modifiers == KeyModifiers::CONTROL {
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

        if event.modifiers == KeyModifiers::ALT {
            let mut alt_seq = vec![0x1b];
            alt_seq.extend(c.to_string().into_bytes());
            return alt_seq;
        }

        // Multi-modifier combinations (e.g. Ctrl+Shift, Ctrl+Alt, Super) -> CSI-u format
        return format!("\x1b[{};{m}u", c as u32).into_bytes();
    }

    // Handle non-character keys
    match event.code {
        KeyCode::Enter => {
            if m == 1 {
                b"\r".to_vec()
            } else {
                format!("\x1b[13;{m}u").into_bytes()
            }
        }
        KeyCode::Tab => {
            if m == 1 {
                b"\t".to_vec()
            } else if m == 2 {
                // Shift only -> BackTab
                b"\x1b[Z".to_vec()
            } else {
                format!("\x1b[9;{m}u").into_bytes()
            }
        }
        KeyCode::BackTab => b"\x1b[Z".to_vec(),
        KeyCode::Backspace => {
            if m == 1 {
                vec![0x7f]
            } else {
                format!("\x1b[127;{m}u").into_bytes()
            }
        }
        KeyCode::Esc => {
            if m == 1 {
                vec![0x1b]
            } else {
                format!("\x1b[27;{m}u").into_bytes()
            }
        }
        KeyCode::Up => {
            if m == 1 {
                b"\x1b[A".to_vec()
            } else {
                format!("\x1b[1;{m}A").into_bytes()
            }
        }
        KeyCode::Down => {
            if m == 1 {
                b"\x1b[B".to_vec()
            } else {
                format!("\x1b[1;{m}B").into_bytes()
            }
        }
        KeyCode::Right => {
            if m == 1 {
                b"\x1b[C".to_vec()
            } else {
                format!("\x1b[1;{m}C").into_bytes()
            }
        }
        KeyCode::Left => {
            if m == 1 {
                b"\x1b[D".to_vec()
            } else {
                format!("\x1b[1;{m}D").into_bytes()
            }
        }
        KeyCode::Home => {
            if m == 1 {
                b"\x1b[H".to_vec()
            } else {
                format!("\x1b[1;{m}H").into_bytes()
            }
        }
        KeyCode::End => {
            if m == 1 {
                b"\x1b[F".to_vec()
            } else {
                format!("\x1b[1;{m}F").into_bytes()
            }
        }
        KeyCode::PageUp => {
            if m == 1 {
                b"\x1b[5~".to_vec()
            } else {
                format!("\x1b[5;{m}~").into_bytes()
            }
        }
        KeyCode::PageDown => {
            if m == 1 {
                b"\x1b[6~".to_vec()
            } else {
                format!("\x1b[6;{m}~").into_bytes()
            }
        }
        KeyCode::Delete => {
            if m == 1 {
                b"\x1b[3~".to_vec()
            } else {
                format!("\x1b[3;{m}~").into_bytes()
            }
        }
        KeyCode::Insert => {
            if m == 1 {
                b"\x1b[2~".to_vec()
            } else {
                format!("\x1b[2;{m}~").into_bytes()
            }
        }
        KeyCode::F(n) => match n {
            1 => {
                if m == 1 {
                    b"\x1bOP".to_vec()
                } else {
                    format!("\x1b[1;{m}P").into_bytes()
                }
            }
            2 => {
                if m == 1 {
                    b"\x1bOQ".to_vec()
                } else {
                    format!("\x1b[1;{m}Q").into_bytes()
                }
            }
            3 => {
                if m == 1 {
                    b"\x1bOR".to_vec()
                } else {
                    format!("\x1b[1;{m}R").into_bytes()
                }
            }
            4 => {
                if m == 1 {
                    b"\x1bOS".to_vec()
                } else {
                    format!("\x1b[1;{m}S").into_bytes()
                }
            }
            5 => {
                if m == 1 {
                    b"\x1b[15~".to_vec()
                } else {
                    format!("\x1b[15;{m}~").into_bytes()
                }
            }
            6 => {
                if m == 1 {
                    b"\x1b[17~".to_vec()
                } else {
                    format!("\x1b[17;{m}~").into_bytes()
                }
            }
            7 => {
                if m == 1 {
                    b"\x1b[18~".to_vec()
                } else {
                    format!("\x1b[18;{m}~").into_bytes()
                }
            }
            8 => {
                if m == 1 {
                    b"\x1b[19~".to_vec()
                } else {
                    format!("\x1b[19;{m}~").into_bytes()
                }
            }
            9 => {
                if m == 1 {
                    b"\x1b[20~".to_vec()
                } else {
                    format!("\x1b[20;{m}~").into_bytes()
                }
            }
            10 => {
                if m == 1 {
                    b"\x1b[21~".to_vec()
                } else {
                    format!("\x1b[21;{m}~").into_bytes()
                }
            }
            11 => {
                if m == 1 {
                    b"\x1b[23~".to_vec()
                } else {
                    format!("\x1b[23;{m}~").into_bytes()
                }
            }
            12 => {
                if m == 1 {
                    b"\x1b[24~".to_vec()
                } else {
                    format!("\x1b[24;{m}~").into_bytes()
                }
            }
            _ => Vec::new(),
        },
        _ => Vec::new(),
    }
}

/// Converts a crossterm `MouseEventKind` into standard ANSI / VT escape sequences for scrolling.
pub fn mouse_scroll_to_bytes(kind: MouseEventKind) -> Vec<u8> {
    match kind {
        MouseEventKind::ScrollUp => b"\x1b[A\x1b[A\x1b[A".to_vec(),
        MouseEventKind::ScrollDown => b"\x1b[B\x1b[B\x1b[B".to_vec(),
        MouseEventKind::ScrollLeft => b"\x1b[D".to_vec(),
        MouseEventKind::ScrollRight => b"\x1b[C".to_vec(),
        _ => Vec::new(),
    }
}

/// Converts a crossterm `MouseEvent` into standard ANSI / VT escape sequences for scrolling.
pub fn mouse_event_to_bytes(event: &MouseEvent) -> Vec<u8> {
    mouse_scroll_to_bytes(event.kind)
}

/// Converts a crossterm `MouseEvent` into standard SGR (1006) ANSI escape sequences for child processes with mouse reporting enabled.
pub fn mouse_event_to_sgr_bytes(event: &MouseEvent) -> Vec<u8> {
    let mut modifier_flag = 0u8;
    if event.modifiers.contains(KeyModifiers::SHIFT) {
        modifier_flag += 4;
    }
    if event.modifiers.contains(KeyModifiers::ALT) {
        modifier_flag += 8;
    }
    if event.modifiers.contains(KeyModifiers::CONTROL) {
        modifier_flag += 16;
    }

    let (btn_code, release) = match event.kind {
        MouseEventKind::Down(crossterm::event::MouseButton::Left) => (modifier_flag, false),
        MouseEventKind::Down(crossterm::event::MouseButton::Middle) => (1 + modifier_flag, false),
        MouseEventKind::Down(crossterm::event::MouseButton::Right) => (2 + modifier_flag, false),
        MouseEventKind::Up(crossterm::event::MouseButton::Left) => (modifier_flag, true),
        MouseEventKind::Up(crossterm::event::MouseButton::Middle) => (1 + modifier_flag, true),
        MouseEventKind::Up(crossterm::event::MouseButton::Right) => (2 + modifier_flag, true),
        MouseEventKind::Drag(crossterm::event::MouseButton::Left) => (32 + modifier_flag, false),
        MouseEventKind::Drag(crossterm::event::MouseButton::Middle) => (33 + modifier_flag, false),
        MouseEventKind::Drag(crossterm::event::MouseButton::Right) => (34 + modifier_flag, false),
        MouseEventKind::Moved => (35 + modifier_flag, false),
        MouseEventKind::ScrollUp => (64 + modifier_flag, false),
        MouseEventKind::ScrollDown => (65 + modifier_flag, false),
        MouseEventKind::ScrollLeft => (66 + modifier_flag, false),
        MouseEventKind::ScrollRight => (67 + modifier_flag, false),
    };

    let suffix = if release { 'm' } else { 'M' };
    format!(
        "\x1b[<{};{};{}{}",
        btn_code,
        event.column.saturating_add(1),
        event.row.saturating_add(1),
        suffix
    )
    .into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEventKind, KeyEventState, MouseButton};

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
    fn test_modifier_code_calculation() {
        assert_eq!(modifier_code(KeyModifiers::empty()), 1);
        assert_eq!(modifier_code(KeyModifiers::SHIFT), 2);
        assert_eq!(modifier_code(KeyModifiers::ALT), 3);
        assert_eq!(modifier_code(KeyModifiers::SHIFT | KeyModifiers::ALT), 4);
        assert_eq!(modifier_code(KeyModifiers::CONTROL), 5);
        assert_eq!(modifier_code(KeyModifiers::CONTROL | KeyModifiers::SHIFT), 6);
        assert_eq!(modifier_code(KeyModifiers::CONTROL | KeyModifiers::ALT), 7);
        assert_eq!(
            modifier_code(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SHIFT),
            8
        );
        assert_eq!(modifier_code(KeyModifiers::SUPER), 9);
        assert_eq!(modifier_code(KeyModifiers::META), 9);
        assert_eq!(modifier_code(KeyModifiers::CONTROL | KeyModifiers::SUPER), 13);
        assert_eq!(
            KeyHandler::modifier_code(KeyModifiers::CONTROL | KeyModifiers::SHIFT),
            6
        );
    }

    #[test]
    fn test_key_to_bytes_modified_enter_csi_u() {
        // Shift+Enter -> \x1b[13;2u (crucial for multiline in agy / Bubbletea)
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::Enter, KeyModifiers::SHIFT)),
            b"\x1b[13;2u".to_vec()
        );
        // Alt+Enter -> \x1b[13;3u
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::Enter, KeyModifiers::ALT)),
            b"\x1b[13;3u".to_vec()
        );
        // Ctrl+Enter -> \x1b[13;5u
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::Enter, KeyModifiers::CONTROL)),
            b"\x1b[13;5u".to_vec()
        );
        // Ctrl+Shift+Enter -> \x1b[13;6u
        assert_eq!(
            key_to_bytes(&make_key(
                KeyCode::Enter,
                KeyModifiers::CONTROL | KeyModifiers::SHIFT
            )),
            b"\x1b[13;6u".to_vec()
        );
        // Super+Enter -> \x1b[13;9u
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::Enter, KeyModifiers::SUPER)),
            b"\x1b[13;9u".to_vec()
        );

        // KeyCode::Char('\n') and KeyCode::Char('\r') handling
        // Without modifiers -> LF (0x0A) or CR (0x0D)
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::Char('\n'), KeyModifiers::empty())),
            b"\n".to_vec()
        );
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::Char('\r'), KeyModifiers::empty())),
            b"\r".to_vec()
        );

        // With Shift -> \x1b[13;2u
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::Char('\n'), KeyModifiers::SHIFT)),
            b"\x1b[13;2u".to_vec()
        );
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::Char('\r'), KeyModifiers::SHIFT)),
            b"\x1b[13;2u".to_vec()
        );

        // With Alt -> \x1b[13;3u
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::Char('\n'), KeyModifiers::ALT)),
            b"\x1b[13;3u".to_vec()
        );
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::Char('\r'), KeyModifiers::ALT)),
            b"\x1b[13;3u".to_vec()
        );

        // With Ctrl and Ctrl+Shift
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::Char('\n'), KeyModifiers::CONTROL)),
            b"\x1b[13;5u".to_vec()
        );
        assert_eq!(
            key_to_bytes(&make_key(
                KeyCode::Char('\n'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT
            )),
            b"\x1b[13;6u".to_vec()
        );
    }

    #[test]
    fn test_key_to_bytes_modified_tab_and_backtab() {
        // Tab no modifiers -> \t
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::Tab, KeyModifiers::empty())),
            b"\t".to_vec()
        );
        // Shift+Tab -> \x1b[Z (BackTab)
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::Tab, KeyModifiers::SHIFT)),
            b"\x1b[Z".to_vec()
        );
        // BackTab -> \x1b[Z
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::BackTab, KeyModifiers::SHIFT)),
            b"\x1b[Z".to_vec()
        );
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::BackTab, KeyModifiers::empty())),
            b"\x1b[Z".to_vec()
        );
        // Alt+Tab -> \x1b[9;3u
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::Tab, KeyModifiers::ALT)),
            b"\x1b[9;3u".to_vec()
        );
        // Ctrl+Tab -> \x1b[9;5u
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::Tab, KeyModifiers::CONTROL)),
            b"\x1b[9;5u".to_vec()
        );
        // Ctrl+Shift+Tab -> \x1b[9;6u
        assert_eq!(
            key_to_bytes(&make_key(
                KeyCode::Tab,
                KeyModifiers::CONTROL | KeyModifiers::SHIFT
            )),
            b"\x1b[9;6u".to_vec()
        );
    }

    #[test]
    fn test_key_to_bytes_modified_backspace_and_esc() {
        // Backspace
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::Backspace, KeyModifiers::empty())),
            vec![0x7f]
        );
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::Backspace, KeyModifiers::SHIFT)),
            b"\x1b[127;2u".to_vec()
        );
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::Backspace, KeyModifiers::ALT)),
            b"\x1b[127;3u".to_vec()
        );
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::Backspace, KeyModifiers::CONTROL)),
            b"\x1b[127;5u".to_vec()
        );
        assert_eq!(
            key_to_bytes(&make_key(
                KeyCode::Backspace,
                KeyModifiers::CONTROL | KeyModifiers::SHIFT
            )),
            b"\x1b[127;6u".to_vec()
        );

        // Esc
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::Esc, KeyModifiers::empty())),
            vec![0x1b]
        );
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::Esc, KeyModifiers::SHIFT)),
            b"\x1b[27;2u".to_vec()
        );
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::Esc, KeyModifiers::ALT)),
            b"\x1b[27;3u".to_vec()
        );
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::Esc, KeyModifiers::CONTROL)),
            b"\x1b[27;5u".to_vec()
        );
    }

    #[test]
    fn test_key_to_bytes_modified_arrows() {
        // Up
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::Up, KeyModifiers::empty())),
            b"\x1b[A".to_vec()
        );
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::Up, KeyModifiers::SHIFT)),
            b"\x1b[1;2A".to_vec()
        );
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::Up, KeyModifiers::ALT)),
            b"\x1b[1;3A".to_vec()
        );
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::Up, KeyModifiers::CONTROL)),
            b"\x1b[1;5A".to_vec()
        );
        assert_eq!(
            key_to_bytes(&make_key(
                KeyCode::Up,
                KeyModifiers::CONTROL | KeyModifiers::SHIFT
            )),
            b"\x1b[1;6A".to_vec()
        );

        // Down
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::Down, KeyModifiers::SHIFT)),
            b"\x1b[1;2B".to_vec()
        );
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::Down, KeyModifiers::CONTROL)),
            b"\x1b[1;5B".to_vec()
        );

        // Right
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::Right, KeyModifiers::SHIFT)),
            b"\x1b[1;2C".to_vec()
        );
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::Right, KeyModifiers::CONTROL)),
            b"\x1b[1;5C".to_vec()
        );

        // Left
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::Left, KeyModifiers::SHIFT)),
            b"\x1b[1;2D".to_vec()
        );
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::Left, KeyModifiers::CONTROL)),
            b"\x1b[1;5D".to_vec()
        );
    }

    #[test]
    fn test_key_to_bytes_modified_navigation_paging_edit() {
        // Home & End
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::Home, KeyModifiers::CONTROL)),
            b"\x1b[1;5H".to_vec()
        );
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::Home, KeyModifiers::SHIFT)),
            b"\x1b[1;2H".to_vec()
        );
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::End, KeyModifiers::CONTROL)),
            b"\x1b[1;5F".to_vec()
        );
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::End, KeyModifiers::SHIFT)),
            b"\x1b[1;2F".to_vec()
        );

        // PageUp & PageDown
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::PageUp, KeyModifiers::SHIFT)),
            b"\x1b[5;2~".to_vec()
        );
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::PageUp, KeyModifiers::CONTROL)),
            b"\x1b[5;5~".to_vec()
        );
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::PageDown, KeyModifiers::SHIFT)),
            b"\x1b[6;2~".to_vec()
        );
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::PageDown, KeyModifiers::CONTROL)),
            b"\x1b[6;5~".to_vec()
        );

        // Delete & Insert
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::Delete, KeyModifiers::SHIFT)),
            b"\x1b[3;2~".to_vec()
        );
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::Delete, KeyModifiers::CONTROL)),
            b"\x1b[3;5~".to_vec()
        );
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::Insert, KeyModifiers::SHIFT)),
            b"\x1b[2;2~".to_vec()
        );
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::Insert, KeyModifiers::CONTROL)),
            b"\x1b[2;5~".to_vec()
        );
    }

    #[test]
    fn test_key_to_bytes_modified_function_keys() {
        // F1..F4
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::F(1), KeyModifiers::SHIFT)),
            b"\x1b[1;2P".to_vec()
        );
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::F(1), KeyModifiers::CONTROL)),
            b"\x1b[1;5P".to_vec()
        );
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::F(2), KeyModifiers::ALT)),
            b"\x1b[1;3Q".to_vec()
        );
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::F(3), KeyModifiers::SHIFT)),
            b"\x1b[1;2R".to_vec()
        );
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::F(4), KeyModifiers::CONTROL)),
            b"\x1b[1;5S".to_vec()
        );

        // F5..F12
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::F(5), KeyModifiers::SHIFT)),
            b"\x1b[15;2~".to_vec()
        );
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::F(6), KeyModifiers::CONTROL)),
            b"\x1b[17;5~".to_vec()
        );
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::F(7), KeyModifiers::ALT)),
            b"\x1b[18;3~".to_vec()
        );
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::F(8), KeyModifiers::SHIFT)),
            b"\x1b[19;2~".to_vec()
        );
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::F(9), KeyModifiers::CONTROL)),
            b"\x1b[20;5~".to_vec()
        );
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::F(10), KeyModifiers::SHIFT)),
            b"\x1b[21;2~".to_vec()
        );
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::F(11), KeyModifiers::CONTROL)),
            b"\x1b[23;5~".to_vec()
        );
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::F(12), KeyModifiers::ALT)),
            b"\x1b[24;3~".to_vec()
        );
    }

    #[test]
    fn test_key_to_bytes_modified_chars_and_combos() {
        // Alt + char -> ESC + char
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::Char('x'), KeyModifiers::ALT)),
            b"\x1bx".to_vec()
        );
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::Char('A'), KeyModifiers::ALT)),
            b"\x1bA".to_vec()
        );

        // Ctrl + Shift + char -> CSI-u format
        assert_eq!(
            key_to_bytes(&make_key(
                KeyCode::Char('c'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT
            )),
            format!("\x1b[{};6u", 'c' as u32).into_bytes()
        );
        assert_eq!(
            key_to_bytes(&make_key(
                KeyCode::Char('k'),
                KeyModifiers::CONTROL | KeyModifiers::ALT
            )),
            format!("\x1b[{};7u", 'k' as u32).into_bytes()
        );
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::Char('z'), KeyModifiers::SUPER)),
            format!("\x1b[{};9u", 'z' as u32).into_bytes()
        );
    }

    #[test]
    fn test_bracketed_paste_formatting() {
        assert_eq!(
            paste_to_bytes("echo 'hello world'"),
            b"\x1b[200~echo 'hello world'\x1b[201~".to_vec()
        );
        assert_eq!(
            paste_to_bytes("multiline\nline 2\r\n"),
            b"\x1b[200~multiline\nline 2\r\n\x1b[201~".to_vec()
        );
        assert_eq!(
            KeyHandler::paste_to_bytes("botai paste"),
            b"\x1b[200~botai paste\x1b[201~".to_vec()
        );
    }

    #[test]
    fn test_key_handler_passthrough_action() {
        let key = make_key(KeyCode::Char('k'), KeyModifiers::empty());
        let action = KeyHandler::map_key_event(key, false);
        assert_eq!(action, TuiAction::Passthrough(b"k".to_vec()));
    }

    #[test]
    fn test_mouse_scroll_to_bytes_scroll_up() {
        assert_eq!(
            mouse_scroll_to_bytes(MouseEventKind::ScrollUp),
            b"\x1b[A\x1b[A\x1b[A".to_vec()
        );
        let event = MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 10,
            row: 15,
            modifiers: KeyModifiers::empty(),
        };
        assert_eq!(mouse_event_to_bytes(&event), b"\x1b[A\x1b[A\x1b[A".to_vec());
        assert_eq!(
            KeyHandler::mouse_scroll_to_bytes(MouseEventKind::ScrollUp),
            b"\x1b[A\x1b[A\x1b[A".to_vec()
        );
        assert_eq!(
            KeyHandler::mouse_event_to_bytes(&event),
            b"\x1b[A\x1b[A\x1b[A".to_vec()
        );
    }

    #[test]
    fn test_mouse_scroll_to_bytes_scroll_down() {
        assert_eq!(
            mouse_scroll_to_bytes(MouseEventKind::ScrollDown),
            b"\x1b[B\x1b[B\x1b[B".to_vec()
        );
        let event = MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 5,
            row: 20,
            modifiers: KeyModifiers::empty(),
        };
        assert_eq!(mouse_event_to_bytes(&event), b"\x1b[B\x1b[B\x1b[B".to_vec());
        assert_eq!(
            KeyHandler::mouse_scroll_to_bytes(MouseEventKind::ScrollDown),
            b"\x1b[B\x1b[B\x1b[B".to_vec()
        );
        assert_eq!(
            KeyHandler::mouse_event_to_bytes(&event),
            b"\x1b[B\x1b[B\x1b[B".to_vec()
        );
    }

    #[test]
    fn test_mouse_scroll_to_bytes_scroll_left_right() {
        assert_eq!(
            mouse_scroll_to_bytes(MouseEventKind::ScrollLeft),
            b"\x1b[D".to_vec()
        );
        assert_eq!(
            mouse_scroll_to_bytes(MouseEventKind::ScrollRight),
            b"\x1b[C".to_vec()
        );

        let left_event = MouseEvent {
            kind: MouseEventKind::ScrollLeft,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::empty(),
        };
        let right_event = MouseEvent {
            kind: MouseEventKind::ScrollRight,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::empty(),
        };
        assert_eq!(mouse_event_to_bytes(&left_event), b"\x1b[D".to_vec());
        assert_eq!(mouse_event_to_bytes(&right_event), b"\x1b[C".to_vec());
    }

    #[test]
    fn test_mouse_scroll_to_bytes_non_scroll_events() {
        let non_scroll_kinds = vec![
            MouseEventKind::Down(MouseButton::Left),
            MouseEventKind::Down(MouseButton::Right),
            MouseEventKind::Down(MouseButton::Middle),
            MouseEventKind::Up(MouseButton::Left),
            MouseEventKind::Up(MouseButton::Right),
            MouseEventKind::Up(MouseButton::Middle),
            MouseEventKind::Drag(MouseButton::Left),
            MouseEventKind::Drag(MouseButton::Right),
            MouseEventKind::Drag(MouseButton::Middle),
            MouseEventKind::Moved,
        ];

        for kind in non_scroll_kinds {
            assert!(
                mouse_scroll_to_bytes(kind).is_empty(),
                "Expected empty byte sequence for non-scroll kind {kind:?}"
            );
            let event = MouseEvent {
                kind,
                column: 12,
                row: 8,
                modifiers: KeyModifiers::empty(),
            };
            assert!(
                mouse_event_to_bytes(&event).is_empty(),
                "Expected empty byte sequence for non-scroll event {event:?}"
            );
            assert!(
                KeyHandler::mouse_scroll_to_bytes(kind).is_empty(),
                "Expected empty byte sequence from KeyHandler for non-scroll kind {kind:?}"
            );
            assert!(
                KeyHandler::mouse_event_to_bytes(&event).is_empty(),
                "Expected empty byte sequence from KeyHandler for non-scroll event {event:?}"
            );
        }
    }

    #[test]
    fn test_mouse_event_to_sgr_bytes() {
        // ScrollUp at col 10, row 5 (0-indexed -> 11, 6 1-indexed)
        let scroll_up = MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 10,
            row: 5,
            modifiers: KeyModifiers::empty(),
        };
        assert_eq!(
            mouse_event_to_sgr_bytes(&scroll_up),
            b"\x1b[<64;11;6M".to_vec()
        );
        assert_eq!(
            KeyHandler::mouse_event_to_sgr_bytes(&scroll_up),
            b"\x1b[<64;11;6M".to_vec()
        );

        // ScrollDown at col 0, row 0
        let scroll_down = MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::empty(),
        };
        assert_eq!(
            mouse_event_to_sgr_bytes(&scroll_down),
            b"\x1b[<65;1;1M".to_vec()
        );

        // Left button down and up (release)
        let left_down = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 20,
            row: 10,
            modifiers: KeyModifiers::empty(),
        };
        assert_eq!(
            mouse_event_to_sgr_bytes(&left_down),
            b"\x1b[<0;21;11M".to_vec()
        );

        let left_up = MouseEvent {
            kind: MouseEventKind::Up(MouseButton::Left),
            column: 20,
            row: 10,
            modifiers: KeyModifiers::empty(),
        };
        assert_eq!(
            mouse_event_to_sgr_bytes(&left_up),
            b"\x1b[<0;21;11m".to_vec()
        );

        // With Shift modifier (+4)
        let shift_scroll_up = MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 5,
            row: 5,
            modifiers: KeyModifiers::SHIFT,
        };
        assert_eq!(
            mouse_event_to_sgr_bytes(&shift_scroll_up),
            b"\x1b[<68;6;6M".to_vec()
        );
    }

    #[test]
    fn test_modify_other_keys_mode_sequences() {
        assert_eq!(ModifyOtherKeysMode::Mode1.enable_sequence(), b"\x1b[>4;1m");
        assert_eq!(ModifyOtherKeysMode::Mode2.enable_sequence(), b"\x1b[>4;2m");
        assert_eq!(ModifyOtherKeysMode::reset_sequence(), b"\x1b[>4;0m");
    }

    #[test]
    fn test_host_modify_other_keys_mode_for_env() {
        // tmux always yields Mode2, regardless of other flags
        assert_eq!(
            host_modify_other_keys_mode_for_env(true, None, false, false),
            Some(ModifyOtherKeysMode::Mode2)
        );
        assert_eq!(
            host_modify_other_keys_mode_for_env(true, Some("WezTerm"), true, true),
            Some(ModifyOtherKeysMode::Mode2)
        );
        assert_eq!(
            host_modify_other_keys_mode_for_env(true, Some("alacritty"), false, true),
            Some(ModifyOtherKeysMode::Mode2)
        );

        // Alacritty, WezTerm, and other modern/standard terminals yield None on Unix
        // to preserve native Kitty Keyboard Protocol and Shift+Enter handling
        assert_eq!(
            host_modify_other_keys_mode_for_env(false, Some("WezTerm"), false, false),
            None
        );
        assert_eq!(
            host_modify_other_keys_mode_for_env(false, Some("wezterm"), false, false),
            None
        );
        assert_eq!(
            host_modify_other_keys_mode_for_env(false, None, true, false),
            None
        );

        // Alacritty yields None
        assert_eq!(
            host_modify_other_keys_mode_for_env(false, Some("Alacritty"), false, false),
            None
        );
        assert_eq!(
            host_modify_other_keys_mode_for_env(false, Some("alacritty"), false, false),
            None
        );
        assert_eq!(
            host_modify_other_keys_mode_for_env(false, None, false, true),
            None
        );

        // Other terminals yield None
        assert_eq!(
            host_modify_other_keys_mode_for_env(false, Some("xterm-256color"), false, false),
            None
        );
        assert_eq!(
            host_modify_other_keys_mode_for_env(false, Some("Apple_Terminal"), false, false),
            None
        );
        assert_eq!(
            host_modify_other_keys_mode_for_env(false, Some("iTerm.app"), false, false),
            None
        );
        assert_eq!(
            host_modify_other_keys_mode_for_env(false, None, false, false),
            None
        );
    }

    #[test]
    fn test_keyboard_enhancement_flags() {
        let flags = keyboard_enhancement_flags();
        assert_eq!(flags, KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES);
        assert!(flags.contains(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES));
        assert!(!flags.contains(KeyboardEnhancementFlags::REPORT_EVENT_TYPES));
        assert!(!flags.contains(KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS));
        assert!(!flags.contains(KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES));
    }

    #[test]
    fn test_key_to_bytes_enter_newlines_and_ctrl_j() {
        // Plain unmodified Enter -> CR (\r, 0x0D)
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::Enter, KeyModifiers::empty())),
            b"\r".to_vec()
        );
        // Plain unmodified LF -> \n (0x0A)
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::Char('\n'), KeyModifiers::empty())),
            b"\n".to_vec()
        );
        // Plain unmodified CR -> \r (0x0D)
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::Char('\r'), KeyModifiers::empty())),
            b"\r".to_vec()
        );

        // Ctrl+J -> ASCII 0x0A (LF)
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::Char('j'), KeyModifiers::CONTROL)),
            vec![0x0a]
        );
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::Char('J'), KeyModifiers::CONTROL)),
            vec![0x0a]
        );

        // Shift+Enter (CSI-u modifier 2)
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::Enter, KeyModifiers::SHIFT)),
            b"\x1b[13;2u".to_vec()
        );
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::Char('\n'), KeyModifiers::SHIFT)),
            b"\x1b[13;2u".to_vec()
        );
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::Char('\r'), KeyModifiers::SHIFT)),
            b"\x1b[13;2u".to_vec()
        );

        // Alt+Enter (CSI-u modifier 3)
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::Enter, KeyModifiers::ALT)),
            b"\x1b[13;3u".to_vec()
        );
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::Char('\n'), KeyModifiers::ALT)),
            b"\x1b[13;3u".to_vec()
        );
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::Char('\r'), KeyModifiers::ALT)),
            b"\x1b[13;3u".to_vec()
        );

        // Ctrl+Enter (CSI-u modifier 5)
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::Enter, KeyModifiers::CONTROL)),
            b"\x1b[13;5u".to_vec()
        );
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::Char('\n'), KeyModifiers::CONTROL)),
            b"\x1b[13;5u".to_vec()
        );
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::Char('\r'), KeyModifiers::CONTROL)),
            b"\x1b[13;5u".to_vec()
        );

        // Ctrl+Shift+Enter (CSI-u modifier 6)
        assert_eq!(
            key_to_bytes(&make_key(
                KeyCode::Enter,
                KeyModifiers::CONTROL | KeyModifiers::SHIFT
            )),
            b"\x1b[13;6u".to_vec()
        );
        assert_eq!(
            key_to_bytes(&make_key(
                KeyCode::Char('\n'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT
            )),
            b"\x1b[13;6u".to_vec()
        );
        assert_eq!(
            key_to_bytes(&make_key(
                KeyCode::Char('\r'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT
            )),
            b"\x1b[13;6u".to_vec()
        );

        // Super+Enter (CSI-u modifier 9)
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::Enter, KeyModifiers::SUPER)),
            b"\x1b[13;9u".to_vec()
        );
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::Char('\n'), KeyModifiers::SUPER)),
            b"\x1b[13;9u".to_vec()
        );
        assert_eq!(
            key_to_bytes(&make_key(KeyCode::Char('\r'), KeyModifiers::SUPER)),
            b"\x1b[13;9u".to_vec()
        );
    }

    #[test]
    fn test_key_handler_keyboard_and_modify_other_keys_delegation() {
        assert_eq!(
            KeyHandler::keyboard_enhancement_flags(),
            keyboard_enhancement_flags()
        );
        assert_eq!(
            KeyHandler::host_modify_other_keys_mode_for_env(true, None, false, false),
            Some(ModifyOtherKeysMode::Mode2)
        );
        assert_eq!(
            KeyHandler::host_modify_other_keys_mode_for_env(false, Some("wezterm"), false, false),
            None
        );
        assert_eq!(
            KeyHandler::host_modify_other_keys_mode_for_env(false, None, false, false),
            None
        );
    }
}
