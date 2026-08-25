use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers, MouseButton, MouseEvent,
    MouseEventKind,
};

/// Raw leader actions decoded from raw stdin byte streams.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RawAction {
    /// Enters leader mode (`Ctrl + Space` / `\x1b[32;5u` / `0x00`).
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
}

/// Alias for multiplexer actions.
pub type TuiAction = RawAction;

/// State-machine decoder for raw stdin byte streams intercepting Ctrl+Space (`0x00` and `\x1b[32;5u`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RawLeaderDecoder {
    leader_active: bool,
    pending_escape: Vec<u8>,
}

impl RawLeaderDecoder {
    /// Kitty keyboard protocol escape sequence for Ctrl+Space (`\x1b[32;5u`).
    pub const KITTY_CTRL_SPACE: &'static [u8] = b"\x1b[32;5u";

    /// Kitty keyboard protocol escape sequence for Shift+Enter (`\x1b[13;2u`).
    pub const KITTY_SHIFT_ENTER: &'static [u8] = b"\x1b[13;2u";

    /// Kitty keyboard protocol alternate key sequence for Shift+Enter (`\x1b[13:10;2u`).
    pub const KITTY_SHIFT_ENTER_ALT: &'static [u8] = b"\x1b[13:10;2u";

    /// Xterm modifyOtherKeys escape sequence for Shift+Enter (`\x1b[27;2;13~`).
    pub const XTERM_SHIFT_ENTER: &'static [u8] = b"\x1b[27;2;13~";

    /// Creates a new `RawLeaderDecoder` with leader mode inactive.
    pub fn new() -> Self {
        Self {
            leader_active: false,
            pending_escape: Vec::new(),
        }
    }

    /// Returns whether leader mode is currently active.
    pub fn is_leader_active(&self) -> bool {
        self.leader_active
    }

    /// Explicitly sets the leader mode active state.
    pub fn set_leader_active(&mut self, active: bool) {
        self.leader_active = active;
        self.pending_escape.clear();
    }

    /// Decodes a chunk of raw input bytes into a sequence of `RawAction`s.
    pub fn decode(&mut self, input: &[u8]) -> Vec<RawAction> {
        let mut actions = Vec::new();
        let mut passthrough_buf = Vec::new();

        let mut combined = Vec::with_capacity(self.pending_escape.len() + input.len());
        combined.extend_from_slice(&self.pending_escape);
        combined.extend_from_slice(input);
        self.pending_escape.clear();

        let mut i = 0;
        while i < combined.len() {
            if self.leader_active {
                // In leader mode: check if it's Kitty Ctrl+Space (which forwards literal Ctrl+Space)
                if combined[i] == 0x1b {
                    let rest = &combined[i..];
                    if rest.starts_with(Self::KITTY_CTRL_SPACE) {
                        self.flush_passthrough(&mut actions, &mut passthrough_buf);
                        self.leader_active = false;
                        actions.push(RawAction::Passthrough(vec![0x00]));
                        i += Self::KITTY_CTRL_SPACE.len();
                        continue;
                    } else if rest.starts_with(b"\x1b[") && Self::KITTY_CTRL_SPACE.starts_with(rest)
                    {
                        // Incomplete prefix of Kitty Ctrl+Space at the end of input
                        self.flush_passthrough(&mut actions, &mut passthrough_buf);
                        self.pending_escape.extend_from_slice(rest);
                        break;
                    } else if rest.starts_with(b"\x1b[") {
                        // Consume the entire CSI sequence to cancel leader
                        let csi_len = rest[2..]
                            .iter()
                            .position(|&b| (0x40..=0x7e).contains(&b))
                            .map(|pos| 2 + pos + 1)
                            .unwrap_or(rest.len());
                        self.flush_passthrough(&mut actions, &mut passthrough_buf);
                        self.leader_active = false;
                        actions.push(RawAction::CancelLeader);
                        i += csi_len;
                        continue;
                    } else if rest.starts_with(b"\x1bO") && rest.len() >= 3 {
                        self.flush_passthrough(&mut actions, &mut passthrough_buf);
                        self.leader_active = false;
                        actions.push(RawAction::CancelLeader);
                        i += 3;
                        continue;
                    }
                }

                self.flush_passthrough(&mut actions, &mut passthrough_buf);
                self.leader_active = false;
                let b = combined[i];
                match b {
                    b'1'..=b'9' => {
                        let idx = (b - b'1') as usize;
                        actions.push(RawAction::SwitchSession(idx));
                    }
                    b'c' | b'C' => {
                        actions.push(RawAction::OpenLauncher);
                    }
                    b'n' | b'N' => {
                        actions.push(RawAction::NextSession);
                    }
                    b'p' | b'P' => {
                        actions.push(RawAction::PrevSession);
                    }
                    b'w' | b'W' | b'x' | b'X' => {
                        actions.push(RawAction::CloseSession);
                    }
                    b'd' | b'D' => {
                        actions.push(RawAction::Detach);
                    }
                    b'q' | b'Q' => {
                        actions.push(RawAction::Quit);
                    }
                    0x00 | b' ' => {
                        actions.push(RawAction::Passthrough(vec![0x00]));
                    }
                    _ => {
                        actions.push(RawAction::CancelLeader);
                    }
                }
                i += 1;
            } else {
                // Not in leader mode
                if combined[i] == 0x00 {
                    self.flush_passthrough(&mut actions, &mut passthrough_buf);
                    self.leader_active = true;
                    actions.push(RawAction::EnterLeader);
                    i += 1;
                } else if combined[i] == 0x1b {
                    let rest = &combined[i..];
                    if rest.starts_with(Self::KITTY_CTRL_SPACE) {
                        self.flush_passthrough(&mut actions, &mut passthrough_buf);
                        self.leader_active = true;
                        actions.push(RawAction::EnterLeader);
                        i += Self::KITTY_CTRL_SPACE.len();
                    } else if rest.starts_with(b"\x1b[") && Self::KITTY_CTRL_SPACE.starts_with(rest)
                    {
                        // Could be start of Kitty Ctrl+Space
                        self.flush_passthrough(&mut actions, &mut passthrough_buf);
                        self.pending_escape.extend_from_slice(rest);
                        break;
                    } else {
                        passthrough_buf.push(combined[i]);
                        i += 1;
                    }
                } else {
                    passthrough_buf.push(combined[i]);
                    i += 1;
                }
            }
        }

        self.flush_passthrough(&mut actions, &mut passthrough_buf);
        actions
    }

    fn flush_passthrough(&self, actions: &mut Vec<RawAction>, buf: &mut Vec<u8>) {
        if !buf.is_empty() {
            actions.push(RawAction::Passthrough(std::mem::take(buf)));
        }
    }

    /// Flushes any pending incomplete escape bytes as passthrough or cancels leader.
    pub fn flush(&mut self) -> Vec<RawAction> {
        if self.pending_escape.is_empty() {
            return Vec::new();
        }
        let pending = std::mem::take(&mut self.pending_escape);
        if self.leader_active {
            self.leader_active = false;
            vec![RawAction::CancelLeader]
        } else {
            vec![RawAction::Passthrough(pending)]
        }
    }
}

/// Returns whether the given raw byte sequence represents a Shift+Enter key event.
pub fn is_shift_enter(bytes: &[u8]) -> bool {
    bytes == RawLeaderDecoder::KITTY_SHIFT_ENTER
        || bytes == RawLeaderDecoder::KITTY_SHIFT_ENTER_ALT
        || bytes == RawLeaderDecoder::XTERM_SHIFT_ENTER
}

fn make_key_event(code: KeyCode, modifiers: KeyModifiers) -> Event {
    Event::Key(KeyEvent {
        code,
        modifiers,
        kind: KeyEventKind::Press,
        state: KeyEventState::empty(),
    })
}

fn decode_csi_modifiers(m: u8) -> KeyModifiers {
    let mut modifiers = KeyModifiers::empty();
    if m > 1 {
        let mod_bits = m - 1;
        if (mod_bits & 1) != 0 {
            modifiers |= KeyModifiers::SHIFT;
        }
        if (mod_bits & 2) != 0 {
            modifiers |= KeyModifiers::ALT;
        }
        if (mod_bits & 4) != 0 {
            modifiers |= KeyModifiers::CONTROL;
        }
        if (mod_bits & 8) != 0 {
            modifiers |= KeyModifiers::SUPER;
        }
    }
    modifiers
}

/// Decodes raw input bytes into a sequence of crossterm `Event`s (Key, Paste, Mouse)
/// for consumption by modal dialogues and the dashboard.
pub fn parse_raw_events(bytes: &[u8]) -> Vec<Event> {
    let mut events = Vec::new();
    let mut i = 0;

    while i < bytes.len() {
        // 1. Bracketed paste: \x1b[200~<content>\x1b[201~
        if bytes[i..].starts_with(b"\x1b[200~") {
            let start = i + 6;
            if let Some(pos) = bytes[start..].windows(6).position(|w| w == b"\x1b[201~") {
                let paste_text = String::from_utf8_lossy(&bytes[start..start + pos]).into_owned();
                events.push(Event::Paste(paste_text));
                i = start + pos + 6;
            } else {
                let paste_text = String::from_utf8_lossy(&bytes[start..]).into_owned();
                events.push(Event::Paste(paste_text));
                i = bytes.len();
            }
            continue;
        }

        // 2. SGR Mouse reporting: \x1b[<Btn;Col;Row[Mm]
        if bytes[i..].starts_with(b"\x1b[<") {
            let rest = &bytes[i + 3..];
            if let Some(end_pos) = rest.iter().position(|&b| b == b'M' || b == b'm') {
                let term = rest[end_pos];
                let is_release = term == b'm';
                if let Ok(param_str) = std::str::from_utf8(&rest[..end_pos]) {
                    let parts: Vec<&str> = param_str.split(';').collect();
                    if parts.len() == 3 {
                        let btn_res = parts[0].parse::<u16>();
                        let col_res = parts[1].parse::<u16>();
                        let row_res = parts[2].parse::<u16>();
                        if let (Ok(btn), Ok(col), Ok(row)) = (btn_res, col_res, row_res) {
                            let mut modifiers = KeyModifiers::empty();
                            if (btn & 4) != 0 {
                                modifiers |= KeyModifiers::SHIFT;
                            }
                            if (btn & 8) != 0 {
                                modifiers |= KeyModifiers::ALT;
                            }
                            if (btn & 16) != 0 {
                                modifiers |= KeyModifiers::CONTROL;
                            }
                            let btn_base = btn & !(4 | 8 | 16);
                            let kind = if is_release {
                                match btn_base {
                                    0 => MouseEventKind::Up(MouseButton::Left),
                                    1 => MouseEventKind::Up(MouseButton::Middle),
                                    2 => MouseEventKind::Up(MouseButton::Right),
                                    _ => MouseEventKind::Up(MouseButton::Left),
                                }
                            } else {
                                match btn_base {
                                    0 => MouseEventKind::Down(MouseButton::Left),
                                    1 => MouseEventKind::Down(MouseButton::Middle),
                                    2 => MouseEventKind::Down(MouseButton::Right),
                                    32 => MouseEventKind::Drag(MouseButton::Left),
                                    33 => MouseEventKind::Drag(MouseButton::Middle),
                                    34 => MouseEventKind::Drag(MouseButton::Right),
                                    35 => MouseEventKind::Moved,
                                    64 => MouseEventKind::ScrollUp,
                                    65 => MouseEventKind::ScrollDown,
                                    66 => MouseEventKind::ScrollLeft,
                                    67 => MouseEventKind::ScrollRight,
                                    _ => MouseEventKind::Moved,
                                }
                            };
                            events.push(Event::Mouse(MouseEvent {
                                kind,
                                column: col.saturating_sub(1),
                                row: row.saturating_sub(1),
                                modifiers,
                            }));
                            i += 3 + end_pos + 1;
                            continue;
                        }
                    }
                }
            }
        }

        // 3. CSI sequences starting with \x1b[
        if bytes[i..].starts_with(b"\x1b[") {
            let rest = &bytes[i + 2..];

            // BackTab: \x1b[Z
            if rest.starts_with(b"Z") {
                events.push(make_key_event(KeyCode::BackTab, KeyModifiers::SHIFT));
                i += 3;
                continue;
            }

            // Arrow keys: Up (A), Down (B), Right (C), Left (D)
            if rest.starts_with(b"A") {
                events.push(make_key_event(KeyCode::Up, KeyModifiers::empty()));
                i += 3;
                continue;
            }
            if rest.starts_with(b"B") {
                events.push(make_key_event(KeyCode::Down, KeyModifiers::empty()));
                i += 3;
                continue;
            }
            if rest.starts_with(b"C") {
                events.push(make_key_event(KeyCode::Right, KeyModifiers::empty()));
                i += 3;
                continue;
            }
            if rest.starts_with(b"D") {
                events.push(make_key_event(KeyCode::Left, KeyModifiers::empty()));
                i += 3;
                continue;
            }
            if rest.starts_with(b"H") {
                events.push(make_key_event(KeyCode::Home, KeyModifiers::empty()));
                i += 3;
                continue;
            }
            if rest.starts_with(b"F") {
                events.push(make_key_event(KeyCode::End, KeyModifiers::empty()));
                i += 3;
                continue;
            }

            // CSI ~ sequences: \x1b[1~ (Home), \x1b[2~ (Insert), \x1b[3~ (Delete), \x1b[4~ (End),
            // \x1b[5~ (PageUp), \x1b[6~ (PageDown), \x1b[7~ (Home), \x1b[8~ (End),
            // \x1b[15~ (F5), \x1b[17~ (F6), \x1b[18~ (F7), \x1b[19~ (F8), \x1b[20~ (F9), \x1b[21~ (F10), \x1b[23~ (F11), \x1b[24~ (F12)
            // modifyOtherKeys format: \x1b[27;<mod>;<codepoint>~
            if let Some(tilde_pos) = rest.iter().position(|&b| b == b'~') {
                if let Ok(param_str) = std::str::from_utf8(&rest[..tilde_pos]) {
                    let parts: Vec<&str> = param_str.split(';').collect();
                    if parts.len() == 3 && parts[0] == "27" {
                        let mod_num = parts[1].parse::<u8>().unwrap_or(1);
                        let modifiers = decode_csi_modifiers(mod_num);
                        if let Ok(cp) = parts[2].parse::<u32>() {
                            let code = match cp {
                                13 => KeyCode::Enter,
                                9 => {
                                    if modifiers.contains(KeyModifiers::SHIFT) {
                                        KeyCode::BackTab
                                    } else {
                                        KeyCode::Tab
                                    }
                                }
                                27 => KeyCode::Esc,
                                127 => KeyCode::Backspace,
                                ch => char::from_u32(ch)
                                    .map(KeyCode::Char)
                                    .unwrap_or(KeyCode::Null),
                            };
                            events.push(make_key_event(code, modifiers));
                            i += 2 + tilde_pos + 1;
                            continue;
                        }
                    } else {
                        let key_num = parts[0].parse::<u32>().ok();
                        let mod_num = parts.get(1).and_then(|s| s.parse::<u8>().ok()).unwrap_or(1);
                        let modifiers = decode_csi_modifiers(mod_num);

                        if let Some(num) = key_num {
                            let code = match num {
                                1 | 7 => Some(KeyCode::Home),
                                2 => Some(KeyCode::Insert),
                                3 => Some(KeyCode::Delete),
                                4 | 8 => Some(KeyCode::End),
                                5 => Some(KeyCode::PageUp),
                                6 => Some(KeyCode::PageDown),
                                11..=14 => Some(KeyCode::F((num - 10) as u8)),
                                15 => Some(KeyCode::F(5)),
                                17 => Some(KeyCode::F(6)),
                                18 => Some(KeyCode::F(7)),
                                19 => Some(KeyCode::F(8)),
                                20 => Some(KeyCode::F(9)),
                                21 => Some(KeyCode::F(10)),
                                23 => Some(KeyCode::F(11)),
                                24 => Some(KeyCode::F(12)),
                                _ => None,
                            };
                            if let Some(c) = code {
                                events.push(make_key_event(c, modifiers));
                                i += 2 + tilde_pos + 1;
                                continue;
                            }
                        }
                    }
                }
            }

            // CSI-u modified key format: \x1b[<codepoint>;<mod>u or \x1b[<codepoint>u
            if let Some(u_pos) = rest.iter().position(|&b| b == b'u') {
                if let Ok(param_str) = std::str::from_utf8(&rest[..u_pos]) {
                    let parts: Vec<&str> = param_str.split(';').collect();
                    let cp_str = parts[0].split(':').next().unwrap_or(parts[0]);
                    let codepoint = cp_str.parse::<u32>().ok();
                    let mod_num = parts.get(1).and_then(|s| s.parse::<u8>().ok()).unwrap_or(1);
                    let modifiers = decode_csi_modifiers(mod_num);

                    if let Some(cp) = codepoint {
                        let code = match cp {
                            13 => KeyCode::Enter,
                            9 => {
                                if modifiers.contains(KeyModifiers::SHIFT) {
                                    KeyCode::BackTab
                                } else {
                                    KeyCode::Tab
                                }
                            }
                            27 => KeyCode::Esc,
                            127 => KeyCode::Backspace,
                            ch => char::from_u32(ch)
                                .map(KeyCode::Char)
                                .unwrap_or(KeyCode::Null),
                        };
                        events.push(make_key_event(code, modifiers));
                        i += 2 + u_pos + 1;
                        continue;
                    }
                }
            }

            // Modified arrow / nav keys: \x1b[1;<mod>A, \x1b[1;<mod>B, \x1b[1;<mod>C, \x1b[1;<mod>D, etc.
            if rest.starts_with(b"1;") {
                let after = &rest[2..];
                if let Some(end_idx) = after.iter().position(|&b| b.is_ascii_alphabetic()) {
                    let term = after[end_idx];
                    if let Ok(mod_num) = std::str::from_utf8(&after[..end_idx])
                        .unwrap_or("")
                        .parse::<u8>()
                    {
                        let modifiers = decode_csi_modifiers(mod_num);
                        let code = match term {
                            b'A' => Some(KeyCode::Up),
                            b'B' => Some(KeyCode::Down),
                            b'C' => Some(KeyCode::Right),
                            b'D' => Some(KeyCode::Left),
                            b'H' => Some(KeyCode::Home),
                            b'F' => Some(KeyCode::End),
                            b'P' => Some(KeyCode::F(1)),
                            b'Q' => Some(KeyCode::F(2)),
                            b'R' => Some(KeyCode::F(3)),
                            b'S' => Some(KeyCode::F(4)),
                            _ => None,
                        };
                        if let Some(c) = code {
                            events.push(make_key_event(c, modifiers));
                            i += 2 + 2 + end_idx + 1;
                            continue;
                        }
                    }
                }
            }
        }

        // 4. SS3 sequences starting with \x1bO
        if bytes[i..].starts_with(b"\x1bO") {
            let rest = &bytes[i + 2..];
            if !rest.is_empty() {
                let code = match rest[0] {
                    b'A' => Some(KeyCode::Up),
                    b'B' => Some(KeyCode::Down),
                    b'C' => Some(KeyCode::Right),
                    b'D' => Some(KeyCode::Left),
                    b'H' => Some(KeyCode::Home),
                    b'F' => Some(KeyCode::End),
                    b'P' => Some(KeyCode::F(1)),
                    b'Q' => Some(KeyCode::F(2)),
                    b'R' => Some(KeyCode::F(3)),
                    b'S' => Some(KeyCode::F(4)),
                    _ => None,
                };
                if let Some(c) = code {
                    events.push(make_key_event(c, KeyModifiers::empty()));
                    i += 3;
                    continue;
                }
            }
        }

        // 5. Alt+char: \x1b<char>
        if bytes[i] == 0x1b && i + 1 < bytes.len() && bytes[i + 1] != b'[' && bytes[i + 1] != b'O' {
            let next_byte = bytes[i + 1];
            if next_byte == b'\r' || next_byte == b'\n' {
                events.push(make_key_event(KeyCode::Enter, KeyModifiers::ALT));
                i += 2;
                continue;
            } else if next_byte == b'\t' {
                events.push(make_key_event(KeyCode::Tab, KeyModifiers::ALT));
                i += 2;
                continue;
            } else if next_byte == 0x7f || next_byte == 0x08 {
                events.push(make_key_event(KeyCode::Backspace, KeyModifiers::ALT));
                i += 2;
                continue;
            } else if let Ok(s) = std::str::from_utf8(&bytes[i + 1..]) {
                if let Some(c) = s.chars().next() {
                    events.push(make_key_event(KeyCode::Char(c), KeyModifiers::ALT));
                    i += 1 + c.len_utf8();
                    continue;
                }
            }
        }

        // 6. Standalone Esc: 0x1b
        if bytes[i] == 0x1b {
            events.push(make_key_event(KeyCode::Esc, KeyModifiers::empty()));
            i += 1;
            continue;
        }

        // 7. Single byte / ASCII control / Enter / Tab / Backspace / UTF-8
        let b = bytes[i];
        match b {
            0x00 => {
                events.push(make_key_event(KeyCode::Char(' '), KeyModifiers::CONTROL));
                i += 1;
            }
            0x09 => {
                // Tab
                events.push(make_key_event(KeyCode::Tab, KeyModifiers::empty()));
                i += 1;
            }
            0x0a | 0x0d => {
                // Enter (\n, \r)
                events.push(make_key_event(KeyCode::Enter, KeyModifiers::empty()));
                i += 1;
            }
            0x7f | 0x08 => {
                // Backspace
                events.push(make_key_event(KeyCode::Backspace, KeyModifiers::empty()));
                i += 1;
            }
            0x01..=0x1a => {
                // Ctrl+A..Ctrl+Z (e.g. 0x03 -> 'c', 0x17 -> 'w', 0x15 -> 'u', 0x0b -> 'k')
                let letter = (b'a' + b - 1) as char;
                events.push(make_key_event(KeyCode::Char(letter), KeyModifiers::CONTROL));
                i += 1;
            }
            0x1c..=0x1f => {
                let code = match b {
                    0x1c => KeyCode::Char('\\'),
                    0x1d => KeyCode::Char(']'),
                    0x1e => KeyCode::Char('^'),
                    0x1f => KeyCode::Char('/'),
                    _ => KeyCode::Null,
                };
                events.push(make_key_event(code, KeyModifiers::CONTROL));
                i += 1;
            }
            _ => {
                // UTF-8 character decode
                if let Ok(s) = std::str::from_utf8(&bytes[i..]) {
                    if let Some(ch) = s.chars().next() {
                        let modifiers = if ch.is_uppercase() {
                            KeyModifiers::SHIFT
                        } else {
                            KeyModifiers::empty()
                        };
                        events.push(make_key_event(KeyCode::Char(ch), modifiers));
                        i += ch.len_utf8();
                        continue;
                    }
                }
                // Fallback for invalid byte
                i += 1;
            }
        }
    }

    events
}

/// Formats a text slice as a standard bracketed paste escape sequence (`\x1b[200~<text>\x1b[201~`).
pub fn paste_to_bytes(text: &str) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(text.len() + 12);
    bytes.extend_from_slice(b"\x1b[200~");
    bytes.extend_from_slice(text.as_bytes());
    bytes.extend_from_slice(b"\x1b[201~");
    bytes
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

/// Converts a crossterm `MouseEvent` into standard SGR (1006) ANSI escape sequences.
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

    #[test]
    fn test_raw_leader_decoder_passthrough_normal_typing() {
        let mut decoder = RawLeaderDecoder::new();
        assert!(!decoder.is_leader_active());

        // Plain text
        let actions = decoder.decode(b"cargo build\r");
        assert_eq!(
            actions,
            vec![RawAction::Passthrough(b"cargo build\r".to_vec())]
        );
        assert!(!decoder.is_leader_active());

        // ANSI escape sequences
        let actions = decoder.decode(b"\x1b[A\x1b[B\x1b[13;2u");
        assert_eq!(
            actions,
            vec![RawAction::Passthrough(b"\x1b[A\x1b[B\x1b[13;2u".to_vec())]
        );
        assert!(!decoder.is_leader_active());
    }

    #[test]
    fn test_raw_leader_decoder_ctrl_space_0x00() {
        let mut decoder = RawLeaderDecoder::new();

        // 0x00 enters leader mode
        let actions = decoder.decode(&[0x00]);
        assert_eq!(actions, vec![RawAction::EnterLeader]);
        assert!(decoder.is_leader_active());

        // Embedded 0x00 splits passthrough before and enters leader
        decoder.set_leader_active(false);
        let actions = decoder.decode(b"abc\x00");
        assert_eq!(
            actions,
            vec![
                RawAction::Passthrough(b"abc".to_vec()),
                RawAction::EnterLeader
            ]
        );
        assert!(decoder.is_leader_active());
    }

    #[test]
    fn test_raw_leader_decoder_kitty_ctrl_space() {
        let mut decoder = RawLeaderDecoder::new();

        // Complete Kitty Ctrl+Space sequence \x1b[32;5u
        let actions = decoder.decode(b"\x1b[32;5u");
        assert_eq!(actions, vec![RawAction::EnterLeader]);
        assert!(decoder.is_leader_active());

        // Embedded Kitty Ctrl+Space in input buffer
        decoder.set_leader_active(false);
        let actions = decoder.decode(b"test\x1b[32;5u");
        assert_eq!(
            actions,
            vec![
                RawAction::Passthrough(b"test".to_vec()),
                RawAction::EnterLeader
            ]
        );
        assert!(decoder.is_leader_active());
    }

    #[test]
    fn test_raw_leader_decoder_kitty_split_chunks() {
        let mut decoder = RawLeaderDecoder::new();

        // Chunk 1: partial prefix \x1b[32;
        let actions1 = decoder.decode(b"\x1b[32;");
        assert!(actions1.is_empty());
        assert!(!decoder.is_leader_active());

        // Chunk 2: remainder 5u
        let actions2 = decoder.decode(b"5u");
        assert_eq!(actions2, vec![RawAction::EnterLeader]);
        assert!(decoder.is_leader_active());
    }

    #[test]
    fn test_raw_leader_decoder_kitty_mismatch_flushes_passthrough() {
        let mut decoder = RawLeaderDecoder::new();

        // Chunk 1: \x1b[32; (partial prefix of Kitty Ctrl+Space)
        let actions1 = decoder.decode(b"\x1b[32;");
        assert!(actions1.is_empty());

        // Chunk 2: 9u (not 5u -> not Kitty Ctrl+Space) -> flushes complete \x1b[32;9u as passthrough
        let actions2 = decoder.decode(b"9u");
        assert_eq!(
            actions2,
            vec![RawAction::Passthrough(b"\x1b[32;9u".to_vec())]
        );
        assert!(!decoder.is_leader_active());
    }

    #[test]
    fn test_raw_leader_decoder_leader_hotkeys() {
        let mut decoder = RawLeaderDecoder::new();

        // 1..9 switch session (0-based 0..8)
        for i in 1..=9 {
            decoder.set_leader_active(true);
            let byte = b'0' + i;
            let actions = decoder.decode(&[byte]);
            assert_eq!(actions, vec![RawAction::SwitchSession((i - 1) as usize)]);
            assert!(!decoder.is_leader_active());
        }

        // c / C -> OpenLauncher
        decoder.set_leader_active(true);
        assert_eq!(decoder.decode(b"c"), vec![RawAction::OpenLauncher]);
        assert!(!decoder.is_leader_active());

        decoder.set_leader_active(true);
        assert_eq!(decoder.decode(b"C"), vec![RawAction::OpenLauncher]);
        assert!(!decoder.is_leader_active());

        // n / N -> NextSession
        decoder.set_leader_active(true);
        assert_eq!(decoder.decode(b"n"), vec![RawAction::NextSession]);
        assert!(!decoder.is_leader_active());

        decoder.set_leader_active(true);
        assert_eq!(decoder.decode(b"N"), vec![RawAction::NextSession]);
        assert!(!decoder.is_leader_active());

        // p / P -> PrevSession
        decoder.set_leader_active(true);
        assert_eq!(decoder.decode(b"p"), vec![RawAction::PrevSession]);
        assert!(!decoder.is_leader_active());

        decoder.set_leader_active(true);
        assert_eq!(decoder.decode(b"P"), vec![RawAction::PrevSession]);
        assert!(!decoder.is_leader_active());

        // w / W / x / X -> CloseSession
        for b in b"wWxX" {
            decoder.set_leader_active(true);
            assert_eq!(decoder.decode(&[*b]), vec![RawAction::CloseSession]);
            assert!(!decoder.is_leader_active());
        }

        // d / D -> Detach
        decoder.set_leader_active(true);
        assert_eq!(decoder.decode(b"d"), vec![RawAction::Detach]);
        assert!(!decoder.is_leader_active());

        decoder.set_leader_active(true);
        assert_eq!(decoder.decode(b"D"), vec![RawAction::Detach]);
        assert!(!decoder.is_leader_active());

        // q / Q -> Quit
        decoder.set_leader_active(true);
        assert_eq!(decoder.decode(b"q"), vec![RawAction::Quit]);
        assert!(!decoder.is_leader_active());

        decoder.set_leader_active(true);
        assert_eq!(decoder.decode(b"Q"), vec![RawAction::Quit]);
        assert!(!decoder.is_leader_active());

        // 0x00 / Space -> Passthrough 0x00 (literal Ctrl+Space forwarding)
        decoder.set_leader_active(true);
        assert_eq!(
            decoder.decode(&[0x00]),
            vec![RawAction::Passthrough(vec![0x00])]
        );
        assert!(!decoder.is_leader_active());

        decoder.set_leader_active(true);
        assert_eq!(
            decoder.decode(b" "),
            vec![RawAction::Passthrough(vec![0x00])]
        );
        assert!(!decoder.is_leader_active());

        // Kitty Ctrl+Space in leader mode -> Passthrough 0x00
        decoder.set_leader_active(true);
        assert_eq!(
            decoder.decode(b"\x1b[32;5u"),
            vec![RawAction::Passthrough(vec![0x00])]
        );
        assert!(!decoder.is_leader_active());

        // Esc or any unrecognized byte -> CancelLeader
        decoder.set_leader_active(true);
        assert_eq!(decoder.decode(&[0x1b]), vec![RawAction::CancelLeader]);
        assert!(!decoder.is_leader_active());

        decoder.set_leader_active(true);
        assert_eq!(decoder.decode(b"z"), vec![RawAction::CancelLeader]);
        assert!(!decoder.is_leader_active());
    }

    #[test]
    fn test_raw_leader_decoder_flush() {
        let mut decoder = RawLeaderDecoder::new();

        // Inactive: pending escape flushed as Passthrough
        let _ = decoder.decode(b"\x1b[3");
        let flushed = decoder.flush();
        assert_eq!(flushed, vec![RawAction::Passthrough(b"\x1b[3".to_vec())]);

        // Active: pending escape cancels leader
        decoder.set_leader_active(true);
        let _ = decoder.decode(b"\x1b[32;");
        let flushed = decoder.flush();
        assert_eq!(flushed, vec![RawAction::CancelLeader]);
        assert!(!decoder.is_leader_active());
    }

    #[test]
    fn test_parse_raw_events_chars_and_utf8() {
        // Plain ascii
        let events = parse_raw_events(b"hello");
        assert_eq!(events.len(), 5);
        assert_eq!(
            events[0],
            make_key_event(KeyCode::Char('h'), KeyModifiers::empty())
        );
        assert_eq!(
            events[4],
            make_key_event(KeyCode::Char('o'), KeyModifiers::empty())
        );

        // Uppercase with Shift
        let events = parse_raw_events(b"Hi");
        assert_eq!(
            events[0],
            make_key_event(KeyCode::Char('H'), KeyModifiers::SHIFT)
        );
        assert_eq!(
            events[1],
            make_key_event(KeyCode::Char('i'), KeyModifiers::empty())
        );

        // Multi-byte UTF-8 emoji and umlaut
        let events = parse_raw_events("🚀äöü".as_bytes());
        assert_eq!(events.len(), 4);
        assert_eq!(
            events[0],
            make_key_event(KeyCode::Char('🚀'), KeyModifiers::empty())
        );
        assert_eq!(
            events[1],
            make_key_event(KeyCode::Char('ä'), KeyModifiers::empty())
        );
        assert_eq!(
            events[2],
            make_key_event(KeyCode::Char('ö'), KeyModifiers::empty())
        );
        assert_eq!(
            events[3],
            make_key_event(KeyCode::Char('ü'), KeyModifiers::empty())
        );
    }

    #[test]
    fn test_is_shift_enter_helper() {
        assert!(is_shift_enter(b"\x1b[13;2u"));
        assert!(is_shift_enter(b"\x1b[13:10;2u"));
        assert!(is_shift_enter(b"\x1b[27;2;13~"));

        assert!(!is_shift_enter(b"\r"));
        assert!(!is_shift_enter(b"\n"));
        assert!(!is_shift_enter(b"\x1b[13;5u"));
        assert!(!is_shift_enter(b"\x1b[32;5u"));
        assert!(!is_shift_enter(b"hello"));
    }

    #[test]
    fn test_raw_leader_decoder_shift_enter() {
        let mut decoder = RawLeaderDecoder::new();

        // 1. Direct Shift+Enter sequences in passthrough
        assert_eq!(
            decoder.decode(b"\x1b[13;2u"),
            vec![RawAction::Passthrough(b"\x1b[13;2u".to_vec())]
        );
        assert_eq!(
            decoder.decode(b"\x1b[13:10;2u"),
            vec![RawAction::Passthrough(b"\x1b[13:10;2u".to_vec())]
        );
        assert_eq!(
            decoder.decode(b"\x1b[27;2;13~"),
            vec![RawAction::Passthrough(b"\x1b[27;2;13~".to_vec())]
        );

        // 2. Shift+Enter cancels active leader mode
        decoder.set_leader_active(true);
        assert_eq!(decoder.decode(b"\x1b[13;2u"), vec![RawAction::CancelLeader]);
        assert!(!decoder.is_leader_active());

        decoder.set_leader_active(true);
        assert_eq!(
            decoder.decode(b"\x1b[27;2;13~"),
            vec![RawAction::CancelLeader]
        );
        assert!(!decoder.is_leader_active());
    }

    #[test]
    fn test_parse_raw_events_enter_variations() {
        // Plain CR and LF
        assert_eq!(
            parse_raw_events(b"\r"),
            vec![make_key_event(KeyCode::Enter, KeyModifiers::empty())]
        );
        assert_eq!(
            parse_raw_events(b"\n"),
            vec![make_key_event(KeyCode::Enter, KeyModifiers::empty())]
        );

        // Shift+Enter CSI-u: \x1b[13;2u
        assert_eq!(
            parse_raw_events(b"\x1b[13;2u"),
            vec![make_key_event(KeyCode::Enter, KeyModifiers::SHIFT)]
        );

        // Shift+Enter Kitty alternate: \x1b[13:10;2u
        assert_eq!(
            parse_raw_events(b"\x1b[13:10;2u"),
            vec![make_key_event(KeyCode::Enter, KeyModifiers::SHIFT)]
        );

        // Shift+Enter Xterm modifyOtherKeys: \x1b[27;2;13~
        assert_eq!(
            parse_raw_events(b"\x1b[27;2;13~"),
            vec![make_key_event(KeyCode::Enter, KeyModifiers::SHIFT)]
        );

        // Ctrl+Enter Xterm modifyOtherKeys: \x1b[27;5;13~
        assert_eq!(
            parse_raw_events(b"\x1b[27;5;13~"),
            vec![make_key_event(KeyCode::Enter, KeyModifiers::CONTROL)]
        );

        // Alt+Enter CSI-u and escape
        assert_eq!(
            parse_raw_events(b"\x1b[13;3u"),
            vec![make_key_event(KeyCode::Enter, KeyModifiers::ALT)]
        );
        assert_eq!(
            parse_raw_events(b"\x1b\r"),
            vec![make_key_event(KeyCode::Enter, KeyModifiers::ALT)]
        );

        // Ctrl+Enter and Ctrl+Shift+Enter
        assert_eq!(
            parse_raw_events(b"\x1b[13;5u"),
            vec![make_key_event(KeyCode::Enter, KeyModifiers::CONTROL)]
        );
        assert_eq!(
            parse_raw_events(b"\x1b[13;6u"),
            vec![make_key_event(
                KeyCode::Enter,
                KeyModifiers::CONTROL | KeyModifiers::SHIFT
            )]
        );
    }

    #[test]
    fn test_parse_raw_events_tab_backtab() {
        // Tab: \t
        assert_eq!(
            parse_raw_events(b"\t"),
            vec![make_key_event(KeyCode::Tab, KeyModifiers::empty())]
        );

        // BackTab: \x1b[Z
        assert_eq!(
            parse_raw_events(b"\x1b[Z"),
            vec![make_key_event(KeyCode::BackTab, KeyModifiers::SHIFT)]
        );

        // CSI-u Tab and Shift+Tab
        assert_eq!(
            parse_raw_events(b"\x1b[9;2u"),
            vec![make_key_event(KeyCode::BackTab, KeyModifiers::SHIFT)]
        );
        assert_eq!(
            parse_raw_events(b"\x1b[9;5u"),
            vec![make_key_event(KeyCode::Tab, KeyModifiers::CONTROL)]
        );
    }

    #[test]
    fn test_parse_raw_events_backspace_esc_delete() {
        // Backspace: 0x7f, 0x08
        assert_eq!(
            parse_raw_events(&[0x7f]),
            vec![make_key_event(KeyCode::Backspace, KeyModifiers::empty())]
        );
        assert_eq!(
            parse_raw_events(&[0x08]),
            vec![make_key_event(KeyCode::Backspace, KeyModifiers::empty())]
        );
        assert_eq!(
            parse_raw_events(b"\x1b[127;2u"),
            vec![make_key_event(KeyCode::Backspace, KeyModifiers::SHIFT)]
        );

        // Esc: 0x1b
        assert_eq!(
            parse_raw_events(b"\x1b"),
            vec![make_key_event(KeyCode::Esc, KeyModifiers::empty())]
        );
        assert_eq!(
            parse_raw_events(b"\x1b[27;2u"),
            vec![make_key_event(KeyCode::Esc, KeyModifiers::SHIFT)]
        );

        // Delete: \x1b[3~
        assert_eq!(
            parse_raw_events(b"\x1b[3~"),
            vec![make_key_event(KeyCode::Delete, KeyModifiers::empty())]
        );
        assert_eq!(
            parse_raw_events(b"\x1b[3;2~"),
            vec![make_key_event(KeyCode::Delete, KeyModifiers::SHIFT)]
        );
    }

    #[test]
    fn test_parse_raw_events_arrow_keys_and_navigation() {
        // Plain arrows
        assert_eq!(
            parse_raw_events(b"\x1b[A"),
            vec![make_key_event(KeyCode::Up, KeyModifiers::empty())]
        );
        assert_eq!(
            parse_raw_events(b"\x1b[B"),
            vec![make_key_event(KeyCode::Down, KeyModifiers::empty())]
        );
        assert_eq!(
            parse_raw_events(b"\x1b[C"),
            vec![make_key_event(KeyCode::Right, KeyModifiers::empty())]
        );
        assert_eq!(
            parse_raw_events(b"\x1b[D"),
            vec![make_key_event(KeyCode::Left, KeyModifiers::empty())]
        );

        // Modified arrows
        assert_eq!(
            parse_raw_events(b"\x1b[1;2A"),
            vec![make_key_event(KeyCode::Up, KeyModifiers::SHIFT)]
        );
        assert_eq!(
            parse_raw_events(b"\x1b[1;5B"),
            vec![make_key_event(KeyCode::Down, KeyModifiers::CONTROL)]
        );

        // Home, End, PageUp, PageDown, Insert
        assert_eq!(
            parse_raw_events(b"\x1b[H"),
            vec![make_key_event(KeyCode::Home, KeyModifiers::empty())]
        );
        assert_eq!(
            parse_raw_events(b"\x1b[F"),
            vec![make_key_event(KeyCode::End, KeyModifiers::empty())]
        );
        assert_eq!(
            parse_raw_events(b"\x1b[1~"),
            vec![make_key_event(KeyCode::Home, KeyModifiers::empty())]
        );
        assert_eq!(
            parse_raw_events(b"\x1b[4~"),
            vec![make_key_event(KeyCode::End, KeyModifiers::empty())]
        );
        assert_eq!(
            parse_raw_events(b"\x1b[5~"),
            vec![make_key_event(KeyCode::PageUp, KeyModifiers::empty())]
        );
        assert_eq!(
            parse_raw_events(b"\x1b[6~"),
            vec![make_key_event(KeyCode::PageDown, KeyModifiers::empty())]
        );
        assert_eq!(
            parse_raw_events(b"\x1b[2~"),
            vec![make_key_event(KeyCode::Insert, KeyModifiers::empty())]
        );
    }

    #[test]
    fn test_parse_raw_events_control_characters() {
        assert_eq!(
            parse_raw_events(&[0x03]),
            vec![make_key_event(KeyCode::Char('c'), KeyModifiers::CONTROL)]
        );
        assert_eq!(
            parse_raw_events(&[0x04]),
            vec![make_key_event(KeyCode::Char('d'), KeyModifiers::CONTROL)]
        );
        assert_eq!(
            parse_raw_events(&[0x01]),
            vec![make_key_event(KeyCode::Char('a'), KeyModifiers::CONTROL)]
        );
        assert_eq!(
            parse_raw_events(&[0x17]),
            vec![make_key_event(KeyCode::Char('w'), KeyModifiers::CONTROL)]
        );
        assert_eq!(
            parse_raw_events(&[0x15]),
            vec![make_key_event(KeyCode::Char('u'), KeyModifiers::CONTROL)]
        );
        assert_eq!(
            parse_raw_events(&[0x0b]),
            vec![make_key_event(KeyCode::Char('k'), KeyModifiers::CONTROL)]
        );
    }

    #[test]
    fn test_parse_raw_events_bracketed_paste() {
        let events = parse_raw_events(b"\x1b[200~echo 'hello world'\x1b[201~");
        assert_eq!(events, vec![Event::Paste("echo 'hello world'".to_string())]);

        let events_multiline = parse_raw_events(b"\x1b[200~line 1\nline 2\r\n\x1b[201~");
        assert_eq!(
            events_multiline,
            vec![Event::Paste("line 1\nline 2\r\n".to_string())]
        );
    }

    #[test]
    fn test_parse_raw_events_sgr_mouse() {
        // ScrollUp at col 10, row 5 (0-indexed -> 9, 4)
        let events = parse_raw_events(b"\x1b[<64;10;5M");
        assert_eq!(
            events,
            vec![Event::Mouse(MouseEvent {
                kind: MouseEventKind::ScrollUp,
                column: 9,
                row: 4,
                modifiers: KeyModifiers::empty(),
            })]
        );

        // ScrollDown at col 1, row 1 (0-indexed -> 0, 0)
        let events = parse_raw_events(b"\x1b[<65;1;1M");
        assert_eq!(
            events,
            vec![Event::Mouse(MouseEvent {
                kind: MouseEventKind::ScrollDown,
                column: 0,
                row: 0,
                modifiers: KeyModifiers::empty(),
            })]
        );

        // Left Down at col 21, row 11 (0-indexed -> 20, 10)
        let events = parse_raw_events(b"\x1b[<0;21;11M");
        assert_eq!(
            events,
            vec![Event::Mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: 20,
                row: 10,
                modifiers: KeyModifiers::empty(),
            })]
        );

        // Left Up at col 21, row 11
        let events = parse_raw_events(b"\x1b[<0;21;11m");
        assert_eq!(
            events,
            vec![Event::Mouse(MouseEvent {
                kind: MouseEventKind::Up(MouseButton::Left),
                column: 20,
                row: 10,
                modifiers: KeyModifiers::empty(),
            })]
        );
    }

    #[test]
    fn test_mouse_event_to_sgr_bytes() {
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
    }
}
