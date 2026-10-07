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
    /// Detaches from the active session (`x` / `X`).
    Detach,
    /// Destroys the active project session workspace (`d` / `D`).
    DestroyProject,
    /// Closes / terminates the active session (`w` / `W`).
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

    /// Returns whether there are pending incomplete escape bytes stored.
    pub fn has_pending(&self) -> bool {
        !self.pending_escape.is_empty()
    }

    /// Explicitly sets the leader mode active state.
    pub fn set_leader_active(&mut self, active: bool) {
        if self.leader_active != active {
            self.leader_active = active;
            self.pending_escape.clear();
        }
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
                    } else if rest.starts_with(b"\x1b") && Self::KITTY_CTRL_SPACE.starts_with(rest)
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
                    b'w' | b'W' => {
                        actions.push(RawAction::CloseSession);
                    }
                    b'x' | b'X' => {
                        actions.push(RawAction::Detach);
                    }
                    b'd' | b'D' => {
                        actions.push(RawAction::DestroyProject);
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
                    } else if rest.starts_with(b"\x1b") && Self::KITTY_CTRL_SPACE.starts_with(rest)
                    {
                        // Incomplete prefix of Kitty Ctrl+Space at the end of input (including single 0x1b)
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
    // 1. Exact slice equality fallbacks
    if bytes == RawLeaderDecoder::KITTY_SHIFT_ENTER
        || bytes == RawLeaderDecoder::KITTY_SHIFT_ENTER_ALT
        || bytes == RawLeaderDecoder::XTERM_SHIFT_ENTER
    {
        return true;
    }

    // 2. Kitty keyboard protocol: \x1b[...u
    if bytes.starts_with(b"\x1b[") && bytes.ends_with(b"u") && bytes.len() >= 4 {
        if let Ok(param_str) = std::str::from_utf8(&bytes[2..bytes.len() - 1]) {
            let parts: Vec<&str> = param_str.split(';').collect();
            let key_code = parts
                .first()
                .and_then(|s| s.split(':').next())
                .unwrap_or("");
            if key_code == "13" || key_code == "10" {
                if let Some(mod_part) = parts.get(1) {
                    if let Some(mod_str) = mod_part.split(':').next() {
                        if let Ok(m) = mod_str.parse::<u32>() {
                            let mod_bits = m.saturating_sub(1);
                            if (mod_bits & 1) != 0 {
                                return true;
                            }
                        }
                    }
                }
            }
        }
    }

    // 3. Xterm modifyOtherKeys format: \x1b[27;<mod>;13~ or \x1b[27;<mod>;10~
    if bytes.starts_with(b"\x1b[") && bytes.ends_with(b"~") && bytes.len() >= 6 {
        if let Ok(param_str) = std::str::from_utf8(&bytes[2..bytes.len() - 1]) {
            let parts: Vec<&str> = param_str.split(';').collect();
            if parts.len() == 3 && parts.first().and_then(|s| s.split(':').next()) == Some("27") {
                let key_code = parts.get(2).and_then(|s| s.split(':').next()).unwrap_or("");
                if key_code == "13" || key_code == "10" {
                    if let Some(mod_part) = parts.get(1) {
                        if let Some(mod_str) = mod_part.split(':').next() {
                            if let Ok(m) = mod_str.parse::<u32>() {
                                let mod_bits = m.saturating_sub(1);
                                if (mod_bits & 1) != 0 {
                                    return true;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    false
}

/// Returns whether the given raw byte sequence represents an unmodified Enter CSI-u escape sequence.
///
/// Terminals supporting Kitty keyboard protocol with `DISAMBIGUATE_ESCAPE_CODES` (such as
/// iTerm2 3.7.2, Ghostty) send `\x1b[13u` or `\x1b[13;1u` for unmodified Enter instead of raw CR (`\r`).
pub fn is_unmodified_enter(bytes: &[u8]) -> bool {
    // Must be a CSI-u sequence: \x1b[...u
    if !bytes.starts_with(b"\x1b[") || !bytes.ends_with(b"u") || bytes.len() < 4 {
        return false;
    }

    if let Ok(param_str) = std::str::from_utf8(&bytes[2..bytes.len() - 1]) {
        let parts: Vec<&str> = param_str.split(';').collect();
        let key_code = parts
            .first()
            .and_then(|s| s.split(':').next())
            .unwrap_or("");

        if key_code == "13" || key_code == "10" {
            // Case 1: No modifiers specified (\x1b[13u, \x1b[13:10u, \x1b[10u)
            if parts.len() == 1 {
                return true;
            }

            // Case 2: Modifiers specified (\x1b[13;1u, \x1b[13;1:1u, etc.)
            if let Some(mod_part) = parts.get(1) {
                let mut mod_subparts = mod_part.split(':');
                let mod_str = mod_subparts.next().unwrap_or("");
                let event_type = mod_subparts.next();

                // If event type is present, reject release events (event_type 3)
                if event_type == Some("3") {
                    return false;
                }

                // Unmodified modifier is 1 (or omitted/empty)
                if mod_str.is_empty() || mod_str == "1" {
                    return true;
                }

                // If numeric, check whether any actual modifier key (Shift=1, Alt=2, Ctrl=4, Super=8) is pressed
                if let Ok(m) = mod_str.parse::<u32>() {
                    let mod_bits = m.saturating_sub(1);
                    if (mod_bits & (1 | 2 | 4 | 8)) == 0 {
                        return true;
                    }
                }
            }
        }
    }

    false
}

fn make_key_event(code: KeyCode, modifiers: KeyModifiers) -> Event {
    Event::Key(KeyEvent {
        code,
        modifiers,
        kind: KeyEventKind::Press,
        state: KeyEventState::empty(),
    })
}

fn decode_csi_modifiers(m: u32) -> KeyModifiers {
    let mut modifiers = KeyModifiers::empty();
    let mod_bits = m.saturating_sub(1);
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
    modifiers
}

/// Attempts to parse an SGR mouse sequence at the beginning of `bytes`.
/// Returns `Some((consumed_len, MouseEvent))` on success.
pub fn parse_sgr_mouse(bytes: &[u8]) -> Option<(usize, MouseEvent)> {
    if !bytes.starts_with(b"\x1b[<") {
        return None;
    }
    let rest = &bytes[3..];
    let end_pos = rest.iter().position(|&b| b == b'M' || b == b'm')?;
    let term = rest[end_pos];
    let is_release = term == b'm';
    let param_str = std::str::from_utf8(&rest[..end_pos]).ok()?;
    let parts: Vec<&str> = param_str.split(';').collect();
    if parts.len() != 3 {
        return None;
    }
    let btn = parts[0].parse::<u16>().ok()?;
    let col = parts[1].parse::<u16>().ok()?;
    let row = parts[2].parse::<u16>().ok()?;

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
    let mouse = MouseEvent {
        kind,
        column: col.saturating_sub(1),
        row: row.saturating_sub(1),
        modifiers,
    };
    let total_len = 3 + end_pos + 1;
    Some((total_len, mouse))
}

/// Returns whether `bytes` is a valid incomplete prefix of an SGR mouse sequence.
/// A valid incomplete prefix must strictly start with `\x1b[<`; a single `\x1b` or `\x1b[`
/// is not treated as an SGR mouse sequence.
pub fn is_incomplete_sgr_mouse(bytes: &[u8]) -> bool {
    if bytes.len() < 3 || bytes.len() > 32 || !bytes.starts_with(b"\x1b[<") {
        return false;
    }
    if bytes.len() == 3 {
        return true;
    }
    let rest = &bytes[3..];
    if rest.contains(&b'M') || rest.contains(&b'm') {
        return false;
    }
    let semicolon_count = rest.iter().filter(|&&b| b == b';').count();
    if semicolon_count > 2 {
        return false;
    }
    rest.iter().all(|&b| b.is_ascii_digit() || b == b';')
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
        if let Some((consumed, mouse)) = parse_sgr_mouse(&bytes[i..]) {
            events.push(Event::Mouse(mouse));
            i += consumed;
            continue;
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
                    if parts.len() == 3
                        && parts.first().and_then(|s| s.split(':').next()) == Some("27")
                    {
                        let mod_num = parts
                            .get(1)
                            .and_then(|s| s.split(':').next())
                            .and_then(|s| s.parse::<u32>().ok())
                            .unwrap_or(1);
                        let modifiers = decode_csi_modifiers(mod_num);
                        let cp_str = parts.get(2).and_then(|s| s.split(':').next()).unwrap_or("");
                        if let Ok(cp) = cp_str.parse::<u32>() {
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
                        let key_num = parts
                            .first()
                            .and_then(|s| s.split(':').next())
                            .and_then(|s| s.parse::<u32>().ok());
                        let mod_num = parts
                            .get(1)
                            .and_then(|s| s.split(':').next())
                            .and_then(|s| s.parse::<u32>().ok())
                            .unwrap_or(1);
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
                    let cp_str = parts
                        .first()
                        .and_then(|s| s.split(':').next())
                        .unwrap_or("");
                    let codepoint = cp_str.parse::<u32>().ok();
                    let mod_num = parts
                        .get(1)
                        .and_then(|s| s.split(':').next())
                        .and_then(|s| s.parse::<u32>().ok())
                        .unwrap_or(1);
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
                    let mod_str = std::str::from_utf8(&after[..end_idx]).unwrap_or("");
                    let mod_num = mod_str
                        .split(':')
                        .next()
                        .and_then(|s| s.parse::<u32>().ok())
                        .unwrap_or(1);
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

/// Translates host terminal mouse coordinates into child terminal viewport coordinates.
///
/// If `mouse.row < status_bar_height`, the mouse event occurred within the status bar area
/// and is rejected (`None`).
/// If the translated row exceeds or equals `vt_height`, the event is out of bounds and rejected (`None`).
/// Otherwise, returns `Some(MouseEvent)` with `row` translated to child viewport space (`row - status_bar_height`).
pub fn translate_mouse_event(
    mouse: crossterm::event::MouseEvent,
    status_bar_height: u16,
    vt_height: u16,
) -> Option<crossterm::event::MouseEvent> {
    if mouse.row < status_bar_height {
        return None;
    }
    let child_row = mouse.row - status_bar_height;
    if child_row >= vt_height {
        return None;
    }
    Some(crossterm::event::MouseEvent {
        kind: mouse.kind,
        column: mouse.column,
        row: child_row,
        modifiers: mouse.modifiers,
    })
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

        // w / W -> CloseSession
        for b in b"wW" {
            decoder.set_leader_active(true);
            assert_eq!(decoder.decode(&[*b]), vec![RawAction::CloseSession]);
            assert!(!decoder.is_leader_active());
        }

        // x / X -> Detach
        for b in b"xX" {
            decoder.set_leader_active(true);
            assert_eq!(decoder.decode(&[*b]), vec![RawAction::Detach]);
            assert!(!decoder.is_leader_active());
        }

        // d / D -> DestroyProject
        for b in b"dD" {
            decoder.set_leader_active(true);
            assert_eq!(decoder.decode(&[*b]), vec![RawAction::DestroyProject]);
            assert!(!decoder.is_leader_active());
        }

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
        assert_eq!(decoder.decode(b"z"), vec![RawAction::CancelLeader]);
        assert!(!decoder.is_leader_active());

        // Single Esc in leader mode buffers pending escape and flushes as CancelLeader
        decoder.set_leader_active(true);
        assert_eq!(decoder.decode(&[0x1b]), vec![]);
        assert_eq!(decoder.flush(), vec![RawAction::CancelLeader]);
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

    #[test]
    fn test_raw_leader_decoder_single_byte_escape_split() {
        let mut decoder = RawLeaderDecoder::new();

        // 1. Chunk 1: b"\x1b", Chunk 2: b"[32;5u" -> should buffer and enter leader mode
        let actions1 = decoder.decode(b"\x1b");
        assert_eq!(actions1, vec![]);
        assert!(!decoder.is_leader_active());

        let actions2 = decoder.decode(b"[32;5u");
        assert_eq!(actions2, vec![RawAction::EnterLeader]);
        assert!(decoder.is_leader_active());

        // 2. In leader mode: Chunk 1: b"\x1b[32", Chunk 2: b";5u" -> should emit Passthrough(vec![0x00])
        decoder.set_leader_active(true);
        let actions3 = decoder.decode(b"\x1b[32");
        assert_eq!(actions3, vec![]);
        assert!(decoder.is_leader_active());

        let actions4 = decoder.decode(b";5u");
        assert_eq!(actions4, vec![RawAction::Passthrough(vec![0x00])]);
        assert!(!decoder.is_leader_active());

        // 3. Chunk 1: b"\x1b", Chunk 2: b"a" -> should passthrough \x1ba
        let actions5 = decoder.decode(b"\x1b");
        assert_eq!(actions5, vec![]);
        let actions6 = decoder.decode(b"a");
        assert_eq!(actions6, vec![RawAction::Passthrough(b"\x1ba".to_vec())]);

        // 4. Inactive leader mode: Chunk 1: b"\x1b[32", Chunk 2: b";5u"
        let actions7 = decoder.decode(b"\x1b[32");
        assert_eq!(actions7, vec![]);
        let actions8 = decoder.decode(b";5u");
        assert_eq!(actions8, vec![RawAction::EnterLeader]);
        assert!(decoder.is_leader_active());

        // 5. Inactive leader mode: Chunk 1: b"\x1b", flush()
        decoder.set_leader_active(false);
        let _ = decoder.decode(b"\x1b");
        let flushed = decoder.flush();
        assert_eq!(flushed, vec![RawAction::Passthrough(vec![0x1b])]);

        // 6. Active leader mode: single \x1b followed by flush cancels leader
        decoder.set_leader_active(true);
        let actions_esc = decoder.decode(b"\x1b");
        assert_eq!(actions_esc, vec![]);
        assert!(decoder.is_leader_active());
        let flushed = decoder.flush();
        assert_eq!(flushed, vec![RawAction::CancelLeader]);
        assert!(!decoder.is_leader_active());

        // 7. Active leader mode: split on \x1b followed by [32;5u -> emits Passthrough(0x00)
        decoder.set_leader_active(true);
        let actions_split1 = decoder.decode(b"\x1b");
        assert_eq!(actions_split1, vec![]);
        assert!(decoder.is_leader_active());
        let actions_split2 = decoder.decode(b"[32;5u");
        assert_eq!(actions_split2, vec![RawAction::Passthrough(vec![0x00])]);
        assert!(!decoder.is_leader_active());
    }

    #[test]
    fn test_raw_leader_decoder_has_pending() {
        let mut decoder = RawLeaderDecoder::new();
        assert!(!decoder.has_pending());

        // Prefix of Kitty Ctrl+Space (\x1b[32;5u) leaves pending escape
        let actions = decoder.decode(b"\x1b");
        assert_eq!(actions, vec![]);
        assert!(decoder.has_pending());

        // Flush clears pending escape
        let flushed = decoder.flush();
        assert_eq!(flushed, vec![RawAction::Passthrough(vec![0x1b])]);
        assert!(!decoder.has_pending());

        // Feeding complete non-escape key does not leave pending
        let actions = decoder.decode(b"hello");
        assert_eq!(actions, vec![RawAction::Passthrough(b"hello".to_vec())]);
        assert!(!decoder.has_pending());
    }

    #[test]
    fn test_is_incomplete_sgr_mouse() {
        // Valid incomplete prefixes (must start with \x1b[<)
        assert!(is_incomplete_sgr_mouse(b"\x1b[<"));
        assert!(is_incomplete_sgr_mouse(b"\x1b[<64"));
        assert!(is_incomplete_sgr_mouse(b"\x1b[<64;"));
        assert!(is_incomplete_sgr_mouse(b"\x1b[<64;10"));
        assert!(is_incomplete_sgr_mouse(b"\x1b[<64;10;"));
        assert!(is_incomplete_sgr_mouse(b"\x1b[<64;10;20"));

        // Standalone escape and CSI prefixes are NOT incomplete SGR mouse sequences
        assert!(!is_incomplete_sgr_mouse(b"\x1b"));
        assert!(!is_incomplete_sgr_mouse(b"\x1b["));

        // Complete SGR mouse sequences (not incomplete)
        assert!(!is_incomplete_sgr_mouse(b"\x1b[<64;10;20M"));
        assert!(!is_incomplete_sgr_mouse(b"\x1b[<64;10;20m"));
        assert!(!is_incomplete_sgr_mouse(b"\x1b[<0;1;1M"));
        assert!(!is_incomplete_sgr_mouse(b"\x1b[<0;1;1m"));

        // Invalid prefixes
        assert!(!is_incomplete_sgr_mouse(b""));
        assert!(!is_incomplete_sgr_mouse(b"a"));
        assert!(!is_incomplete_sgr_mouse(b"hello"));
        assert!(!is_incomplete_sgr_mouse(b"\x1b[A"));
        assert!(!is_incomplete_sgr_mouse(b"\x1b[13;2u"));
        assert!(!is_incomplete_sgr_mouse(b"\x1b[27;2;13~"));
        assert!(!is_incomplete_sgr_mouse(b"\x1b[<64;10;20;30"));
        assert!(!is_incomplete_sgr_mouse(b"\x1b[<invalid"));
        assert!(!is_incomplete_sgr_mouse(&[b'x'; 33]));
    }

    #[test]
    fn test_parse_raw_events_csi_u_with_event_types_and_locks() {
        // \x1b[13;2:1u -> Enter with Shift (event type 1: press)
        assert_eq!(
            parse_raw_events(b"\x1b[13;2:1u"),
            vec![make_key_event(KeyCode::Enter, KeyModifiers::SHIFT)]
        );

        // \x1b[13;66u -> Enter with Shift and CapsLock (mod 66 = 1 + 1 (Shift) + 64 (CapsLock))
        assert_eq!(
            parse_raw_events(b"\x1b[13;66u"),
            vec![make_key_event(KeyCode::Enter, KeyModifiers::SHIFT)]
        );

        // \x1b[32;5:1u -> Space with Control (event type 1: press)
        assert_eq!(
            parse_raw_events(b"\x1b[32;5:1u"),
            vec![make_key_event(KeyCode::Char(' '), KeyModifiers::CONTROL)]
        );

        // \x1b[13:10;2:1u -> Enter with Shift (Kitty alternate codepoints + event type)
        assert_eq!(
            parse_raw_events(b"\x1b[13:10;2:1u"),
            vec![make_key_event(KeyCode::Enter, KeyModifiers::SHIFT)]
        );

        // \x1b[9;131u -> Tab with Alt and NumLock (mod 131 = 1 + 2 (Alt) + 128 (NumLock))
        assert_eq!(
            parse_raw_events(b"\x1b[9;131u"),
            vec![make_key_event(KeyCode::Tab, KeyModifiers::ALT)]
        );

        // \x1b[9;66:1u -> BackTab with Shift and CapsLock (mod 66 = 1 + 1 (Shift) + 64 (CapsLock))
        assert_eq!(
            parse_raw_events(b"\x1b[9;66:1u"),
            vec![make_key_event(KeyCode::BackTab, KeyModifiers::SHIFT)]
        );
    }

    #[test]
    fn test_is_shift_enter_with_event_types_and_locks() {
        // Kitty format with Shift and sub-parameters
        assert!(is_shift_enter(b"\x1b[13;2:1u"));
        assert!(is_shift_enter(b"\x1b[13;66u"));
        assert!(is_shift_enter(b"\x1b[13:10;2:1u"));
        assert!(is_shift_enter(b"\x1b[13:text;2:1u"));
        assert!(is_shift_enter(b"\x1b[10;2:1u"));
        assert!(is_shift_enter(b"\x1b[10;66u"));

        // Exact slice fallbacks
        assert!(is_shift_enter(b"\x1b[13;2u"));
        assert!(is_shift_enter(b"\x1b[13:10;2u"));
        assert!(is_shift_enter(b"\x1b[27;2;13~"));

        // Xterm format with Shift and lock modifiers
        assert!(is_shift_enter(b"\x1b[27;66;13~"));
        assert!(is_shift_enter(b"\x1b[27;2;10~"));

        // Negative cases
        assert!(!is_shift_enter(b"\x1b[13;5u")); // Ctrl+Enter
        assert!(!is_shift_enter(b"\x1b[13;1u")); // Unmodified Enter
        assert!(!is_shift_enter(b"\r"));
        assert!(!is_shift_enter(b"\n"));
        assert!(!is_shift_enter(b"\x1b[32;5u")); // Ctrl+Space
    }

    #[test]
    fn test_is_unmodified_enter() {
        // Positive cases: Kitty keyboard protocol unmodified enter variants
        assert!(is_unmodified_enter(b"\x1b[13u"));
        assert!(is_unmodified_enter(b"\x1b[13;1u"));
        assert!(is_unmodified_enter(b"\x1b[13;1:1u"));
        assert!(is_unmodified_enter(b"\x1b[13:10u"));
        assert!(is_unmodified_enter(b"\x1b[13:10;1u"));
        assert!(is_unmodified_enter(b"\x1b[13:10;1:1u"));
        assert!(is_unmodified_enter(b"\x1b[10u"));
        assert!(is_unmodified_enter(b"\x1b[10;1u"));
        assert!(is_unmodified_enter(b"\x1b[10;1:1u"));
        assert!(is_unmodified_enter(b"\x1b[13;1:2u")); // Repeat event

        // Positive cases: lock keys active with no modifier keys pressed
        assert!(is_unmodified_enter(b"\x1b[13;65u")); // CapsLock (64 + 1)
        assert!(is_unmodified_enter(b"\x1b[13;129u")); // NumLock (128 + 1)

        // Negative cases: Modified Enter
        assert!(!is_unmodified_enter(b"\x1b[13;2u")); // Shift+Enter
        assert!(!is_unmodified_enter(b"\x1b[13;2:1u")); // Shift+Enter with event type
        assert!(!is_unmodified_enter(b"\x1b[13;3u")); // Alt+Enter
        assert!(!is_unmodified_enter(b"\x1b[13;5u")); // Ctrl+Enter
        assert!(!is_unmodified_enter(b"\x1b[13;9u")); // Super+Enter

        // Negative cases: Key release event (event type 3)
        assert!(!is_unmodified_enter(b"\x1b[13;1:3u"));

        // Negative cases: Raw non-escape characters and other keys
        assert!(!is_unmodified_enter(b"\r"));
        assert!(!is_unmodified_enter(b"\n"));
        assert!(!is_unmodified_enter(b"\x1b[32;1u")); // Space
        assert!(!is_unmodified_enter(b"\x1b[13~"));
        assert!(!is_unmodified_enter(b""));
        assert!(!is_unmodified_enter(b"hello"));
    }

    #[test]
    fn test_translate_mouse_event_status_bar_row_0_ignored() {
        let mouse = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 10,
            row: 0,
            modifiers: KeyModifiers::empty(),
        };
        // Status bar height = 1, active VT height = 24
        assert_eq!(translate_mouse_event(mouse, 1, 24), None);
    }

    #[test]
    fn test_translate_mouse_event_row_1_to_row_0_generates_sgr_row_1() {
        let mouse = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 5,
            row: 1,
            modifiers: KeyModifiers::empty(),
        };
        // Row 1 on host maps to child row 0
        let translated = translate_mouse_event(mouse, 1, 24).expect("translated");
        assert_eq!(translated.column, 5);
        assert_eq!(translated.row, 0);
        assert_eq!(translated.kind, MouseEventKind::Down(MouseButton::Left));

        // When formatted as SGR bytes, 0-based child row 0 turns into 1-based row 1: \x1b[<0;6;1M
        let sgr = mouse_event_to_sgr_bytes(&translated);
        assert_eq!(sgr, b"\x1b[<0;6;1M");
    }

    #[test]
    fn test_translate_mouse_event_row_n_to_n_minus_1_generates_sgr_row_n() {
        let mouse = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 15,
            row: 10,
            modifiers: KeyModifiers::empty(),
        };
        // Row 10 on host maps to child row 9
        let translated = translate_mouse_event(mouse, 1, 24).expect("translated");
        assert_eq!(translated.column, 15);
        assert_eq!(translated.row, 9);

        // When formatted as SGR bytes, 0-based child row 9 turns into 1-based row 10: \x1b[<0;16;10M
        let sgr = mouse_event_to_sgr_bytes(&translated);
        assert_eq!(sgr, b"\x1b[<0;16;10M");

        // Test with row 23 (host row 24):
        let mouse_bottom = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 0,
            row: 24,
            modifiers: KeyModifiers::empty(),
        };
        let translated_bottom =
            translate_mouse_event(mouse_bottom, 1, 24).expect("translated bottom");
        assert_eq!(translated_bottom.row, 23);
        let sgr_bottom = mouse_event_to_sgr_bytes(&translated_bottom);
        assert_eq!(sgr_bottom, b"\x1b[<0;1;24M");
    }

    #[test]
    fn test_translate_mouse_event_drag_and_up_events() {
        // Drag event: Left drag on host row 8 with Shift modifier
        let drag_mouse = MouseEvent {
            kind: MouseEventKind::Drag(MouseButton::Left),
            column: 12,
            row: 8,
            modifiers: KeyModifiers::SHIFT,
        };
        let translated_drag = translate_mouse_event(drag_mouse, 1, 24).expect("translated drag");
        assert_eq!(
            translated_drag.kind,
            MouseEventKind::Drag(MouseButton::Left)
        );
        assert_eq!(translated_drag.modifiers, KeyModifiers::SHIFT);
        assert_eq!(translated_drag.column, 12);
        assert_eq!(translated_drag.row, 7);

        // SGR code: 32 (drag) + 4 (shift) = 36, col: 13, row: 8, suffix 'M'
        let sgr_drag = mouse_event_to_sgr_bytes(&translated_drag);
        assert_eq!(sgr_drag, b"\x1b[<36;13;8M");

        // Up event: Left up on host row 8 with Control modifier
        let up_mouse = MouseEvent {
            kind: MouseEventKind::Up(MouseButton::Left),
            column: 12,
            row: 8,
            modifiers: KeyModifiers::CONTROL,
        };
        let translated_up = translate_mouse_event(up_mouse, 1, 24).expect("translated up");
        assert_eq!(translated_up.kind, MouseEventKind::Up(MouseButton::Left));
        assert_eq!(translated_up.modifiers, KeyModifiers::CONTROL);
        assert_eq!(translated_up.column, 12);
        assert_eq!(translated_up.row, 7);

        // SGR code: 0 (left) + 16 (control) = 16, col: 13, row: 8, suffix 'm' (release)
        let sgr_up = mouse_event_to_sgr_bytes(&translated_up);
        assert_eq!(sgr_up, b"\x1b[<16;13;8m");
    }

    #[test]
    fn test_translate_mouse_event_out_of_bounds_ignored() {
        // Active VT height = 24, status bar height = 1. Valid host rows are 1..=24 (child rows 0..=23).
        // Row 25 -> child row 24 >= 24 (out of bounds)
        let mouse_out = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 10,
            row: 25,
            modifiers: KeyModifiers::empty(),
        };
        assert_eq!(translate_mouse_event(mouse_out, 1, 24), None);

        // Row 100 -> out of bounds
        let mouse_far = MouseEvent {
            kind: MouseEventKind::Moved,
            column: 10,
            row: 100,
            modifiers: KeyModifiers::empty(),
        };
        assert_eq!(translate_mouse_event(mouse_far, 1, 24), None);
    }
}
