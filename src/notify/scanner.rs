/// Control and semantic notification events detected within PTY byte streams.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamControlEvent {
    /// Standalone ASCII Bell character (`\x07` / BEL)
    Bell,
    /// OSC 9 desktop notification message: `\x1b]9;<message>\x07` or `\x1b]9;<message>\x1b\\`
    Osc9(String),
    /// OSC 777 notification: `\x1b]777;notify;<title>;<message>\x07` or `\x1b]777;notify;<title>;<message>\x1b\\`
    Osc777 { title: String, message: String },
    /// OSC 133;A (Semantic prompt start / ready for user input)
    PromptStart,
    /// OSC 133;B (Command start / user execution)
    CommandStart,
    /// OSC 133;C (Command output execution stream)
    CommandOutput,
    /// OSC 133;D[;<exit_code>] (Command finished with exit code)
    CommandFinished { exit_code: i32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScannerState {
    Normal,
    SawEsc,
    InOsc,
    OscSawEsc,
}

/// State-machine scanner detecting control sequences (BEL, OSC 9, OSC 777, OSC 133) in byte streams.
///
/// Designed to handle streaming byte chunks across arbitrary chunk boundaries non-destructively.
#[derive(Debug, Clone)]
pub struct StreamScanner {
    state: ScannerState,
    osc_buf: Vec<u8>,
    max_osc_len: usize,
}

impl Default for StreamScanner {
    fn default() -> Self {
        Self::new()
    }
}

impl StreamScanner {
    /// Creates a new `StreamScanner` with default buffer limits.
    pub fn new() -> Self {
        Self {
            state: ScannerState::Normal,
            osc_buf: Vec::with_capacity(256),
            max_osc_len: 4096,
        }
    }

    /// Feeds a chunk of bytes into the scanner and returns any control events detected.
    pub fn scan(&mut self, bytes: &[u8]) -> Vec<StreamControlEvent> {
        let mut events = Vec::new();

        for &b in bytes {
            match self.state {
                ScannerState::Normal => {
                    if b == 0x07 {
                        events.push(StreamControlEvent::Bell);
                    } else if b == 0x1B {
                        self.state = ScannerState::SawEsc;
                    }
                }
                ScannerState::SawEsc => {
                    if b == b']' {
                        self.osc_buf.clear();
                        self.state = ScannerState::InOsc;
                    } else if b == 0x1B {
                        self.state = ScannerState::SawEsc;
                    } else {
                        self.state = ScannerState::Normal;
                        if b == 0x07 {
                            events.push(StreamControlEvent::Bell);
                        }
                    }
                }
                ScannerState::InOsc => {
                    if b == 0x07 {
                        if let Some(ev) = self.parse_osc_payload() {
                            events.push(ev);
                        }
                        self.osc_buf.clear();
                        self.state = ScannerState::Normal;
                    } else if b == 0x1B {
                        self.state = ScannerState::OscSawEsc;
                    } else if self.osc_buf.len() < self.max_osc_len {
                        self.osc_buf.push(b);
                    } else {
                        self.osc_buf.clear();
                        self.state = ScannerState::Normal;
                    }
                }
                ScannerState::OscSawEsc => {
                    if b == b'\\' || b == 0x07 {
                        if let Some(ev) = self.parse_osc_payload() {
                            events.push(ev);
                        }
                        self.osc_buf.clear();
                        self.state = ScannerState::Normal;
                    } else if b == b']' {
                        self.osc_buf.clear();
                        self.state = ScannerState::InOsc;
                    } else if b == 0x1B {
                        self.state = ScannerState::OscSawEsc;
                    } else if self.osc_buf.len() + 2 <= self.max_osc_len {
                        self.osc_buf.push(0x1B);
                        self.osc_buf.push(b);
                        self.state = ScannerState::InOsc;
                    } else {
                        self.osc_buf.clear();
                        self.state = ScannerState::Normal;
                    }
                }
            }
        }

        events
    }

    /// Parses the accumulated OSC buffer into a `StreamControlEvent`.
    fn parse_osc_payload(&self) -> Option<StreamControlEvent> {
        let s = std::str::from_utf8(&self.osc_buf).ok()?;

        // 1. OSC 9: "9;<msg>"
        if let Some(msg) = s.strip_prefix("9;") {
            return Some(StreamControlEvent::Osc9(msg.to_string()));
        }
        if s == "9" {
            return Some(StreamControlEvent::Osc9(String::new()));
        }

        // 2. OSC 777: "777;notify;<title>;<msg>" or "777;<title>;<msg>"
        if let Some(rest) = s.strip_prefix("777;") {
            if let Some(notify_rest) = rest.strip_prefix("notify;") {
                if let Some((title, msg)) = notify_rest.split_once(';') {
                    return Some(StreamControlEvent::Osc777 {
                        title: title.to_string(),
                        message: msg.to_string(),
                    });
                } else {
                    return Some(StreamControlEvent::Osc777 {
                        title: notify_rest.to_string(),
                        message: String::new(),
                    });
                }
            } else if let Some((title, msg)) = rest.split_once(';') {
                return Some(StreamControlEvent::Osc777 {
                    title: title.to_string(),
                    message: msg.to_string(),
                });
            } else {
                return Some(StreamControlEvent::Osc777 {
                    title: rest.to_string(),
                    message: String::new(),
                });
            }
        }

        // 3. OSC 133 Semantic Shell Integration Prompts
        if let Some(rest) = s.strip_prefix("133;") {
            let code = rest.chars().next()?;
            let remainder = &rest[code.len_utf8()..];
            match code {
                'A' => return Some(StreamControlEvent::PromptStart),
                'B' => return Some(StreamControlEvent::CommandStart),
                'C' => return Some(StreamControlEvent::CommandOutput),
                'D' => {
                    let exit_code = if let Some(args) = remainder.strip_prefix(';') {
                        let code_str = args.split(';').next().unwrap_or("0");
                        code_str.trim().parse::<i32>().unwrap_or(0)
                    } else {
                        0
                    };
                    return Some(StreamControlEvent::CommandFinished { exit_code });
                }
                _ => return None,
            }
        }

        None
    }

    /// Resets the internal state and clears buffers.
    pub fn reset(&mut self) {
        self.state = ScannerState::Normal;
        self.osc_buf.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bell_detection() {
        let mut scanner = StreamScanner::new();

        // Standalone BEL
        let events = scanner.scan(b"Hello\x07World");
        assert_eq!(events, vec![StreamControlEvent::Bell]);

        // Multiple BELs
        let events = scanner.scan(b"\x07\x07");
        assert_eq!(
            events,
            vec![StreamControlEvent::Bell, StreamControlEvent::Bell]
        );
    }

    #[test]
    fn test_osc_9_parsing_bel_and_st() {
        let mut scanner = StreamScanner::new();

        // OSC 9 with BEL terminator
        let events = scanner.scan(b"\x1b]9;Task completed successfully\x07");
        assert_eq!(
            events,
            vec![StreamControlEvent::Osc9(
                "Task completed successfully".to_string()
            )]
        );

        // OSC 9 with ST (\x1b\) terminator
        let events = scanner.scan(b"\x1b]9;Build ready\x1b\\");
        assert_eq!(
            events,
            vec![StreamControlEvent::Osc9("Build ready".to_string())]
        );
    }

    #[test]
    fn test_osc_777_parsing() {
        let mut scanner = StreamScanner::new();

        // OSC 777 notify with title and message
        let events = scanner.scan(b"\x1b]777;notify;botai;Build finished in 2.4s\x07");
        assert_eq!(
            events,
            vec![StreamControlEvent::Osc777 {
                title: "botai".to_string(),
                message: "Build finished in 2.4s".to_string(),
            }]
        );

        // OSC 777 with ST terminator
        let events = scanner.scan(b"\x1b]777;notify;Deploy;Success on prod\x1b\\");
        assert_eq!(
            events,
            vec![StreamControlEvent::Osc777 {
                title: "Deploy".to_string(),
                message: "Success on prod".to_string(),
            }]
        );

        // OSC 777 with semicolon in message
        let events = scanner.scan(b"\x1b]777;notify;Test;Error in module: a; b; c\x07");
        assert_eq!(
            events,
            vec![StreamControlEvent::Osc777 {
                title: "Test".to_string(),
                message: "Error in module: a; b; c".to_string(),
            }]
        );
    }

    #[test]
    fn test_osc_133_semantic_prompts() {
        let mut scanner = StreamScanner::new();

        // 133;A (Prompt start)
        let events = scanner.scan(b"\x1b]133;A\x07");
        assert_eq!(events, vec![StreamControlEvent::PromptStart]);

        // 133;B (Command start)
        let events = scanner.scan(b"\x1b]133;B\x07");
        assert_eq!(events, vec![StreamControlEvent::CommandStart]);

        // 133;C (Command output)
        let events = scanner.scan(b"\x1b]133;C\x07");
        assert_eq!(events, vec![StreamControlEvent::CommandOutput]);

        // 133;D (Command finished - default 0)
        let events = scanner.scan(b"\x1b]133;D\x07");
        assert_eq!(
            events,
            vec![StreamControlEvent::CommandFinished { exit_code: 0 }]
        );

        // 133;D;0 (Command finished - exit 0)
        let events = scanner.scan(b"\x1b]133;D;0\x07");
        assert_eq!(
            events,
            vec![StreamControlEvent::CommandFinished { exit_code: 0 }]
        );

        // 133;D;1 (Command finished - exit 1)
        let events = scanner.scan(b"\x1b]133;D;1\x07");
        assert_eq!(
            events,
            vec![StreamControlEvent::CommandFinished { exit_code: 1 }]
        );

        // 133;D;127;aid=42 (Command finished with metadata)
        let events = scanner.scan(b"\x1b]133;D;127;aid=42\x1b\\");
        assert_eq!(
            events,
            vec![StreamControlEvent::CommandFinished { exit_code: 127 }]
        );
    }

    #[test]
    fn test_chunked_streaming_across_reads() {
        let mut scanner = StreamScanner::new();

        // Split across ESC and OSC
        let ev1 = scanner.scan(b"Some text\x1b");
        assert!(ev1.is_empty());
        let ev2 = scanner.scan(b"]133;");
        assert!(ev2.is_empty());
        let ev3 = scanner.scan(b"A\x07More text");
        assert_eq!(ev3, vec![StreamControlEvent::PromptStart]);

        // Split across ST terminator (\x1b and \)
        let ev1 = scanner.scan(b"\x1b]9;Progress 50%\x1b");
        assert!(ev1.is_empty());
        let ev2 = scanner.scan(b"\\");
        assert_eq!(
            ev2,
            vec![StreamControlEvent::Osc9("Progress 50%".to_string())]
        );
    }

    #[test]
    fn test_multiple_sequences_and_interleaved_text() {
        let mut scanner = StreamScanner::new();
        let payload = b"\x1b]133;B\x07cargo test\n\x1b]133;C\x07running tests...\n\x1b]133;D;0\x07\x1b]133;A\x07";
        let events = scanner.scan(payload);
        assert_eq!(
            events,
            vec![
                StreamControlEvent::CommandStart,
                StreamControlEvent::CommandOutput,
                StreamControlEvent::CommandFinished { exit_code: 0 },
                StreamControlEvent::PromptStart,
            ]
        );
    }

    #[test]
    fn test_unrecognized_osc_and_ansi_passthrough() {
        let mut scanner = StreamScanner::new();

        // Window title OSC 0 and color CSI sequences should not produce events
        let events = scanner.scan(b"\x1b]0;my-window-title\x07\x1b[32;1mGreen Text\x1b[0m");
        assert!(events.is_empty());
    }
}
