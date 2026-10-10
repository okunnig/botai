use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Reusable single-line text input component supporting in-line cursor navigation,
/// standard readline editing keybindings, and paste sanitization.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TextInput {
    pub value: String,
    pub cursor: usize,
    pub placeholder: String,
}

impl TextInput {
    /// Creates an empty `TextInput`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the placeholder text.
    pub fn with_placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    /// Sets the initial value and places the cursor at the end.
    pub fn with_value(mut self, value: impl Into<String>) -> Self {
        self.value = value.into();
        self.cursor = self.value.chars().count();
        self
    }

    /// Returns the current string slice of the text input.
    pub fn value(&self) -> &str {
        &self.value
    }

    /// Returns the current character cursor offset.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Sets the value and clamps the cursor to the new length.
    pub fn set_value(&mut self, value: impl Into<String>) {
        self.value = value.into();
        self.cursor = self.cursor.min(self.value.chars().count());
    }

    /// Sets the cursor offset, clamped to the length of the value.
    pub fn set_cursor(&mut self, cursor: usize) {
        self.cursor = cursor.min(self.value.chars().count());
    }

    /// Returns the byte index corresponding to `char_pos` and the clamped character offset.
    /// If `char_pos` exceeds the total number of characters, returns `(self.value.len(), total_chars)`.
    fn char_to_byte_idx(&self, char_pos: usize) -> (usize, usize) {
        let byte_idx = self.value.len();
        let mut clamped_char = 0;
        for (i, (b, _)) in self.value.char_indices().enumerate() {
            if i == char_pos {
                return (b, i);
            }
            clamped_char = i + 1;
        }
        (byte_idx, clamped_char)
    }

    /// Scans backward from `cursor_byte` across one word boundary (skipping non-alphanumeric, then alphanumeric).
    /// Returns `(start_byte, skipped_chars)`.
    fn scan_word_backward_boundary(&self, cursor_byte: usize) -> (usize, usize) {
        let prefix = &self.value[..cursor_byte];
        let mut iter = prefix.char_indices().rev().peekable();
        let mut skipped_chars = 0;
        let mut start_byte = cursor_byte;

        // Phase 1: skip non-alphanumeric
        while let Some(&(_, c)) = iter.peek() {
            if !c.is_alphanumeric() {
                let (b, _) = iter.next().unwrap();
                start_byte = b;
                skipped_chars += 1;
            } else {
                break;
            }
        }

        // Phase 2: skip alphanumeric
        while let Some(&(_, c)) = iter.peek() {
            if c.is_alphanumeric() {
                let (b, _) = iter.next().unwrap();
                start_byte = b;
                skipped_chars += 1;
            } else {
                break;
            }
        }

        (start_byte, skipped_chars)
    }

    /// Scans forward from `cursor_byte` across one word boundary (skipping non-alphanumeric, then alphanumeric).
    /// Returns the number of characters advanced.
    fn scan_word_forward_chars(&self, cursor_byte: usize) -> usize {
        let suffix = &self.value[cursor_byte..];
        let mut iter = suffix.chars().peekable();
        let mut advanced_chars = 0;

        // Phase 1: skip non-alphanumeric
        while let Some(&c) = iter.peek() {
            if !c.is_alphanumeric() {
                iter.next();
                advanced_chars += 1;
            } else {
                break;
            }
        }

        // Phase 2: skip alphanumeric
        while let Some(&c) = iter.peek() {
            if c.is_alphanumeric() {
                iter.next();
                advanced_chars += 1;
            } else {
                break;
            }
        }

        advanced_chars
    }

    /// Inserts a single character at the current cursor offset.
    /// Newlines (`\r`, `\n`) and control characters are ignored.
    pub fn insert_char(&mut self, c: char) {
        if c == '\n' || c == '\r' || c.is_control() {
            return;
        }
        let (byte_idx, clamped_cursor) = self.char_to_byte_idx(self.cursor);
        self.value.insert(byte_idx, c);
        self.cursor = clamped_cursor + 1;
    }

    /// Inserts a string at the current cursor offset, stripping newlines and control characters.
    pub fn insert_str(&mut self, text: &str) -> bool {
        let (byte_idx, clamped_cursor) = self.char_to_byte_idx(self.cursor);
        let has_control = text
            .chars()
            .any(|c| c == '\n' || c == '\r' || c.is_control());
        if !has_control {
            if text.is_empty() {
                return false;
            }
            let count = text.chars().count();
            self.value.insert_str(byte_idx, text);
            self.cursor = clamped_cursor + count;
            true
        } else {
            let mut cleaned = String::with_capacity(text.len());
            let mut count = 0;
            for c in text.chars() {
                if c != '\n' && c != '\r' && !c.is_control() {
                    cleaned.push(c);
                    count += 1;
                }
            }
            if cleaned.is_empty() {
                return false;
            }
            self.value.insert_str(byte_idx, &cleaned);
            self.cursor = clamped_cursor + count;
            true
        }
    }

    /// Deletes the character before the cursor (Backspace).
    /// Returns `true` if a character was removed, or `false` if cursor is at position 0.
    pub fn delete_char_backward(&mut self) -> bool {
        if self.cursor == 0 || self.value.is_empty() {
            return false;
        }
        let target_idx = self.cursor - 1;
        if let Some((byte_idx, _)) = self.value.char_indices().nth(target_idx) {
            self.value.remove(byte_idx);
            self.cursor = target_idx;
            true
        } else {
            // Cursor was out of bounds (> char count). Remove the last character.
            if let Some((last_byte, _)) = self.value.char_indices().last() {
                self.value.remove(last_byte);
                self.cursor = self.value.chars().count();
                true
            } else {
                self.cursor = 0;
                false
            }
        }
    }

    /// Deletes the character under the cursor (Delete).
    /// Returns `true` if a character was removed, or `false` if cursor is at the end.
    pub fn delete_char_forward(&mut self) -> bool {
        if let Some((byte_idx, _)) = self.value.char_indices().nth(self.cursor) {
            self.value.remove(byte_idx);
            true
        } else {
            false
        }
    }

    /// Moves the cursor left by one character.
    pub fn move_cursor_left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    /// Moves the cursor right by one character.
    pub fn move_cursor_right(&mut self) {
        let total = self.value.chars().count();
        if self.cursor < total {
            self.cursor += 1;
        }
    }

    /// Moves the cursor to the beginning of the text (Home / Ctrl+A).
    pub fn move_cursor_home(&mut self) {
        self.cursor = 0;
    }

    /// Moves the cursor to the end of the text (End / Ctrl+E).
    pub fn move_cursor_end(&mut self) {
        self.cursor = self.value.chars().count();
    }

    /// Deletes text from the cursor to the beginning of the line (Ctrl+U).
    /// Returns `true` if any characters were deleted.
    pub fn delete_to_start(&mut self) -> bool {
        if self.cursor == 0 || self.value.is_empty() {
            self.cursor = 0;
            return false;
        }
        let (byte_idx, _) = self.char_to_byte_idx(self.cursor);
        if byte_idx > 0 {
            self.value.replace_range(..byte_idx, "");
            self.cursor = 0;
            true
        } else {
            self.cursor = 0;
            false
        }
    }

    /// Deletes text from the cursor to the end of the line (Ctrl+K).
    /// Returns `true` if any characters were deleted.
    pub fn delete_to_end(&mut self) -> bool {
        if let Some((byte_idx, _)) = self.value.char_indices().nth(self.cursor) {
            self.value.truncate(byte_idx);
            true
        } else {
            false
        }
    }

    /// Deletes the word immediately preceding the cursor (Ctrl+W).
    /// Returns `true` if any characters were deleted.
    pub fn delete_word_backward(&mut self) -> bool {
        if self.cursor == 0 || self.value.is_empty() {
            return false;
        }
        let (cursor_byte, clamped_cursor) = self.char_to_byte_idx(self.cursor);
        if cursor_byte == 0 {
            self.cursor = 0;
            return false;
        }
        let (start_byte, skipped_chars) = self.scan_word_backward_boundary(cursor_byte);
        if skipped_chars == 0 {
            return false;
        }
        self.value.replace_range(start_byte..cursor_byte, "");
        self.cursor = clamped_cursor - skipped_chars;
        true
    }

    /// Moves the cursor backward one word (Ctrl+Left / Alt+B).
    pub fn move_word_backward(&mut self) {
        if self.cursor == 0 || self.value.is_empty() {
            self.cursor = 0;
            return;
        }
        let (cursor_byte, clamped_cursor) = self.char_to_byte_idx(self.cursor);
        let (_, skipped_chars) = self.scan_word_backward_boundary(cursor_byte);
        self.cursor = clamped_cursor - skipped_chars;
    }

    /// Moves the cursor forward one word (Ctrl+Right / Alt+F).
    pub fn move_word_forward(&mut self) {
        let (cursor_byte, clamped_cursor) = self.char_to_byte_idx(self.cursor);
        if cursor_byte >= self.value.len() {
            self.cursor = clamped_cursor;
            return;
        }
        let advanced = self.scan_word_forward_chars(cursor_byte);
        self.cursor = clamped_cursor + advanced;
    }

    /// Handles a keyboard event. Returns `true` if the input text was modified.
    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        let has_ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let has_alt = key.modifiers.contains(KeyModifiers::ALT);

        if (key.code == KeyCode::Left && (has_ctrl || has_alt))
            || ((key.code == KeyCode::Char('b') || key.code == KeyCode::Char('B')) && has_alt)
        {
            self.move_word_backward();
            return false;
        }

        if (key.code == KeyCode::Right && (has_ctrl || has_alt))
            || ((key.code == KeyCode::Char('f') || key.code == KeyCode::Char('F')) && has_alt)
        {
            self.move_word_forward();
            return false;
        }

        match key.code {
            KeyCode::Left => {
                self.move_cursor_left();
                false
            }
            KeyCode::Right => {
                self.move_cursor_right();
                false
            }
            KeyCode::Home => {
                self.move_cursor_home();
                false
            }
            KeyCode::End => {
                self.move_cursor_end();
                false
            }
            KeyCode::Backspace => self.delete_char_backward(),
            KeyCode::Delete => self.delete_char_forward(),
            KeyCode::Char(c) if has_ctrl => match c {
                'a' | 'A' => {
                    self.move_cursor_home();
                    false
                }
                'e' | 'E' => {
                    self.move_cursor_end();
                    false
                }
                'u' | 'U' => self.delete_to_start(),
                'k' | 'K' => self.delete_to_end(),
                'w' | 'W' => self.delete_word_backward(),
                _ => false,
            },
            KeyCode::Char(c) if !has_ctrl && !has_alt => {
                self.insert_char(c);
                true
            }
            _ => false,
        }
    }

    /// Handles pasted text by stripping newlines and control characters, then inserting at cursor.
    /// Returns `true` if characters were inserted.
    pub fn handle_paste(&mut self, text: &str) -> bool {
        self.insert_str(text)
    }
}

impl From<String> for TextInput {
    fn from(s: String) -> Self {
        Self::new().with_value(s)
    }
}

impl From<&str> for TextInput {
    fn from(s: &str) -> Self {
        Self::new().with_value(s)
    }
}

impl From<&TextInput> for String {
    fn from(ti: &TextInput) -> Self {
        ti.value.clone()
    }
}

impl std::ops::Deref for TextInput {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

impl AsRef<str> for TextInput {
    fn as_ref(&self) -> &str {
        &self.value
    }
}

impl std::fmt::Display for TextInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.value)
    }
}

impl PartialEq<&str> for TextInput {
    fn eq(&self, other: &&str) -> bool {
        self.value == *other
    }
}

impl PartialEq<str> for TextInput {
    fn eq(&self, other: &str) -> bool {
        self.value == other
    }
}

impl PartialEq<String> for TextInput {
    fn eq(&self, other: &String) -> bool {
        self.value == *other
    }
}

impl PartialEq<TextInput> for &str {
    fn eq(&self, other: &TextInput) -> bool {
        *self == other.value
    }
}

impl PartialEq<TextInput> for String {
    fn eq(&self, other: &TextInput) -> bool {
        *self == other.value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl_key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    fn alt_key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::ALT)
    }

    #[test]
    fn test_text_input_new_and_defaults() {
        let input = TextInput::new();
        assert_eq!(input.value(), "");
        assert_eq!(input.cursor(), 0);
        assert_eq!(input.placeholder, "");

        let input = TextInput::new()
            .with_placeholder("Enter project name")
            .with_value("initial");
        assert_eq!(input.value(), "initial");
        assert_eq!(input.cursor(), 7);
        assert_eq!(input.placeholder, "Enter project name");
    }

    #[test]
    fn test_text_input_insert_and_set_value() {
        let mut input = TextInput::new();
        input.insert_char('a');
        input.insert_char('b');
        input.insert_char('c');
        assert_eq!(input.value(), "abc");
        assert_eq!(input.cursor(), 3);

        input.set_cursor(1);
        assert_eq!(input.cursor(), 1);
        input.insert_char('x');
        assert_eq!(input.value(), "axbc");
        assert_eq!(input.cursor(), 2);

        // Control characters and newlines are ignored
        input.insert_char('\n');
        input.insert_char('\r');
        input.insert_char('\x00');
        assert_eq!(input.value(), "axbc");
        assert_eq!(input.cursor(), 2);

        input.set_value("short");
        assert_eq!(input.value(), "short");
        assert_eq!(input.cursor(), 2);

        input.set_cursor(100);
        assert_eq!(input.cursor(), 5);
    }

    #[test]
    fn test_text_input_cursor_movements() {
        let mut input = TextInput::new().with_value("hello");
        assert_eq!(input.cursor(), 5);

        input.move_cursor_left();
        assert_eq!(input.cursor(), 4);

        input.move_cursor_left();
        input.move_cursor_left();
        assert_eq!(input.cursor(), 2);

        input.move_cursor_home();
        assert_eq!(input.cursor(), 0);

        // Does not underflow
        input.move_cursor_left();
        assert_eq!(input.cursor(), 0);

        input.move_cursor_right();
        assert_eq!(input.cursor(), 1);

        input.move_cursor_end();
        assert_eq!(input.cursor(), 5);

        // Does not overflow
        input.move_cursor_right();
        assert_eq!(input.cursor(), 5);
    }

    #[test]
    fn test_text_input_word_movements() {
        let mut input = TextInput::new().with_value("foo-bar-baz");
        assert_eq!(input.cursor(), 11);

        // Backward word jumps
        input.move_word_backward();
        assert_eq!(input.cursor(), 8); // At 'b' in "baz"

        input.move_word_backward();
        assert_eq!(input.cursor(), 4); // At 'b' in "bar"

        input.move_word_backward();
        assert_eq!(input.cursor(), 0); // At 'f' in "foo"

        input.move_word_backward();
        assert_eq!(input.cursor(), 0);

        // Forward word jumps
        input.move_word_forward();
        assert_eq!(input.cursor(), 3); // After "foo"

        input.move_word_forward();
        assert_eq!(input.cursor(), 7); // After "bar"

        input.move_word_forward();
        assert_eq!(input.cursor(), 11); // After "baz"

        input.move_word_forward();
        assert_eq!(input.cursor(), 11);
    }

    #[test]
    fn test_text_input_deletions() {
        let mut input = TextInput::new().with_value("abcd");
        input.set_cursor(2); // Between 'b' and 'c'

        // Backspace removes 'b'
        assert!(input.delete_char_backward());
        assert_eq!(input.value(), "acd");
        assert_eq!(input.cursor(), 1);

        // Delete removes 'c'
        assert!(input.delete_char_forward());
        assert_eq!(input.value(), "ad");
        assert_eq!(input.cursor(), 1);

        // Delete removes 'd'
        assert!(input.delete_char_forward());
        assert_eq!(input.value(), "a");
        assert_eq!(input.cursor(), 1);

        // Delete at end returns false
        assert!(!input.delete_char_forward());

        // Backspace removes 'a'
        assert!(input.delete_char_backward());
        assert_eq!(input.value(), "");
        assert_eq!(input.cursor(), 0);

        // Backspace when empty returns false
        assert!(!input.delete_char_backward());
    }

    #[test]
    fn test_text_input_readline_kills() {
        // Ctrl+U: delete to start
        let mut input = TextInput::new().with_value("first-second-third");
        input.set_cursor(13); // At start of "third"
        assert!(input.delete_to_start());
        assert_eq!(input.value(), "third");
        assert_eq!(input.cursor(), 0);
        assert!(!input.delete_to_start()); // No-op at 0

        // Ctrl+K: delete to end
        let mut input = TextInput::new().with_value("first-second-third");
        input.set_cursor(5); // At '-' before second
        assert!(input.delete_to_end());
        assert_eq!(input.value(), "first");
        assert_eq!(input.cursor(), 5);
        assert!(!input.delete_to_end()); // No-op at end

        // Ctrl+W: delete word backward
        let mut input = TextInput::new().with_value("alpha-beta-gamma");
        assert!(input.delete_word_backward());
        assert_eq!(input.value(), "alpha-beta-");
        assert_eq!(input.cursor(), 11);

        assert!(input.delete_word_backward());
        assert_eq!(input.value(), "alpha-");
        assert_eq!(input.cursor(), 6);

        assert!(input.delete_word_backward());
        assert_eq!(input.value(), "");
        assert_eq!(input.cursor(), 0);
        assert!(!input.delete_word_backward());
    }

    #[test]
    fn test_text_input_paste_sanitization() {
        let mut input = TextInput::new().with_value("start-end");
        input.set_cursor(6); // Between '-' and "end"

        assert!(input.handle_paste("mid\r\ndle-"));
        assert_eq!(input.value(), "start-middle-end");
        assert_eq!(input.cursor(), 13);

        // Pasting only newlines/control chars returns false
        assert!(!input.handle_paste("\r\n\x01\x02"));
        assert_eq!(input.value(), "start-middle-end");
    }

    #[test]
    fn test_text_input_handle_key_dispatch() {
        let mut input = TextInput::new();

        // Char insert returns true
        assert!(input.handle_key(key(KeyCode::Char('x'))));
        assert!(input.handle_key(key(KeyCode::Char('y'))));
        assert_eq!(input.value(), "xy");

        // Cursor movement returns false
        assert!(!input.handle_key(key(KeyCode::Left)));
        assert_eq!(input.cursor(), 1);

        // Home returns false
        assert!(!input.handle_key(ctrl_key('a')));
        assert_eq!(input.cursor(), 0);

        // End returns false
        assert!(!input.handle_key(ctrl_key('e')));
        assert_eq!(input.cursor(), 2);

        // Word back returns false
        assert!(!input.handle_key(alt_key('b')));
        assert_eq!(input.cursor(), 0);

        // Word forward returns false
        assert!(!input.handle_key(alt_key('f')));
        assert_eq!(input.cursor(), 2);

        // Ctrl+Left / Ctrl+Right
        let ctrl_left = KeyEvent::new(KeyCode::Left, KeyModifiers::CONTROL);
        assert!(!input.handle_key(ctrl_left));
        assert_eq!(input.cursor(), 0);

        let ctrl_right = KeyEvent::new(KeyCode::Right, KeyModifiers::CONTROL);
        assert!(!input.handle_key(ctrl_right));
        assert_eq!(input.cursor(), 2);

        // Backspace returns true
        assert!(input.handle_key(key(KeyCode::Backspace)));
        assert_eq!(input.value(), "x");

        // Ctrl+W returns true
        assert!(input.handle_key(ctrl_key('w')));
        assert_eq!(input.value(), "");

        // Empty backspace returns false
        assert!(!input.handle_key(key(KeyCode::Backspace)));

        // Non-input key returns false
        assert!(!input.handle_key(key(KeyCode::Tab)));
        assert!(!input.handle_key(key(KeyCode::Enter)));
        assert!(!input.handle_key(key(KeyCode::Esc)));
    }

    #[test]
    fn test_text_input_traits_deref_and_equality() {
        let input: TextInput = "hello-world".into();
        assert_eq!(input, "hello-world");
        assert_eq!("hello-world", input);
        assert_eq!(input, "hello-world".to_string());
        assert_eq!("hello-world".to_string(), input);
        assert_eq!(input.len(), 11);
        assert!(!input.is_empty());
        assert_eq!(format!("{input}"), "hello-world");
    }

    #[test]
    fn test_text_input_multibyte_swedish() {
        let mut input = TextInput::new();
        input.insert_char('å');
        input.insert_char('ä');
        input.insert_char('ö');
        assert_eq!(input.value(), "åäö");
        assert_eq!(input.cursor(), 3);
        assert_eq!(input.value().len(), 6); // 2 bytes per char

        // Navigate left and insert in middle
        input.move_cursor_left();
        assert_eq!(input.cursor(), 2);
        input.insert_char('k');
        assert_eq!(input.value(), "åäkö");
        assert_eq!(input.cursor(), 3);

        // Delete backward (removes 'k')
        assert!(input.delete_char_backward());
        assert_eq!(input.value(), "åäö");
        assert_eq!(input.cursor(), 2);

        // Delete forward (removes 'ö')
        assert!(input.delete_char_forward());
        assert_eq!(input.value(), "åä");
        assert_eq!(input.cursor(), 2);

        // Move to start and delete forward (removes 'å')
        input.move_cursor_home();
        assert_eq!(input.cursor(), 0);
        assert!(input.delete_char_forward());
        assert_eq!(input.value(), "ä");
        assert_eq!(input.cursor(), 0);

        // Insert at start
        input.insert_char('ö');
        assert_eq!(input.value(), "öä");
        assert_eq!(input.cursor(), 1);

        // Delete backward (removes 'ö')
        assert!(input.delete_char_backward());
        assert_eq!(input.value(), "ä");
        assert_eq!(input.cursor(), 0);

        // Delete backward at start returns false
        assert!(!input.delete_char_backward());
    }

    #[test]
    fn test_text_input_multibyte_cyrillic_and_emojis() {
        let mut input = TextInput::new();
        // Cyrillic: each letter is 2 bytes
        assert!(input.insert_str("Привет, мир!"));
        assert_eq!(input.value(), "Привет, мир!");
        assert_eq!(input.cursor(), 12);

        // Move left 5 characters to cursor 7 (before space)
        for _ in 0..5 {
            input.move_cursor_left();
        }
        assert_eq!(input.cursor(), 7);

        // Insert a 4-byte emoji
        input.insert_char('🦀');
        assert_eq!(input.value(), "Привет,🦀 мир!");
        assert_eq!(input.cursor(), 8);

        // Delete backward removes the entire 4-byte emoji
        assert!(input.delete_char_backward());
        assert_eq!(input.value(), "Привет, мир!");
        assert_eq!(input.cursor(), 7);

        // Delete forward removes the space
        assert!(input.delete_char_forward());
        assert_eq!(input.value(), "Привет,мир!");
        assert_eq!(input.cursor(), 7);

        // Insert multi-byte emoji string
        assert!(input.insert_str("🚀🎉"));
        assert_eq!(input.value(), "Привет,🚀🎉мир!");
        assert_eq!(input.cursor(), 9);

        // Delete backward twice removes both emojis
        assert!(input.delete_char_backward());
        assert_eq!(input.value(), "Привет,🚀мир!");
        assert_eq!(input.cursor(), 8);
        assert!(input.delete_char_backward());
        assert_eq!(input.value(), "Привет,мир!");
        assert_eq!(input.cursor(), 7);
    }

    #[test]
    fn test_text_input_edge_cases_empty_and_boundaries() {
        let mut input = TextInput::new();

        // On empty input:
        assert!(!input.delete_char_backward());
        assert!(!input.delete_char_forward());
        assert!(!input.delete_to_start());
        assert!(!input.delete_to_end());
        assert!(!input.delete_word_backward());
        input.move_word_backward();
        assert_eq!(input.cursor(), 0);
        input.move_word_forward();
        assert_eq!(input.cursor(), 0);
        input.move_cursor_left();
        assert_eq!(input.cursor(), 0);
        input.move_cursor_right();
        assert_eq!(input.cursor(), 0);
        assert_eq!(input.value(), "");

        // Single character boundaries
        input.insert_char('x');
        assert_eq!(input.cursor(), 1);
        // At end: delete forward returns false
        assert!(!input.delete_char_forward());
        assert_eq!(input.value(), "x");
        // Move to start
        input.move_cursor_left();
        assert_eq!(input.cursor(), 0);
        // At start: delete backward returns false
        assert!(!input.delete_char_backward());
        assert_eq!(input.value(), "x");
        // Delete forward deletes 'x'
        assert!(input.delete_char_forward());
        assert_eq!(input.value(), "");
        assert_eq!(input.cursor(), 0);

        // Out-of-bounds cursor robustness:
        input.set_value("alpha");
        input.cursor = 100; // Directly set out-of-bounds cursor
        input.insert_char('!');
        assert_eq!(input.value(), "alpha!");
        assert_eq!(input.cursor(), 6);

        input.cursor = 200;
        assert!(input.insert_str(" beta"));
        assert_eq!(input.value(), "alpha! beta");
        assert_eq!(input.cursor(), 11);

        input.cursor = 500;
        assert!(input.delete_char_backward());
        assert_eq!(input.value(), "alpha! bet");
        assert_eq!(input.cursor(), 10);

        input.cursor = 500;
        assert!(!input.delete_char_forward());
        assert_eq!(input.value(), "alpha! bet");

        input.cursor = 500;
        assert!(!input.delete_to_end());
        assert_eq!(input.value(), "alpha! bet");

        input.cursor = 500;
        assert!(input.delete_to_start());
        assert_eq!(input.value(), "");
        assert_eq!(input.cursor(), 0);
    }

    #[test]
    fn test_text_input_multibyte_word_navigation_and_deletions() {
        let mut input = TextInput::new().with_value("hallå-där_vännen! 123");
        assert_eq!(input.cursor(), 21);

        // Word backward jumps across Swedish and punctuation
        input.move_word_backward();
        assert_eq!(input.cursor(), 18); // at '1' in "123"

        input.move_word_backward();
        assert_eq!(input.cursor(), 10); // at 'v' in "vännen"

        input.move_word_backward();
        assert_eq!(input.cursor(), 6); // at 'd' in "där"

        input.move_word_backward();
        assert_eq!(input.cursor(), 0); // at 'h' in "hallå"

        input.move_word_backward();
        assert_eq!(input.cursor(), 0);

        // Word forward jumps across Swedish and punctuation
        input.move_word_forward();
        assert_eq!(input.cursor(), 5); // after "hallå"

        input.move_word_forward();
        assert_eq!(input.cursor(), 9); // after "där"

        input.move_word_forward();
        assert_eq!(input.cursor(), 16); // after "vännen"

        input.move_word_forward();
        assert_eq!(input.cursor(), 21); // after "123"

        input.move_word_forward();
        assert_eq!(input.cursor(), 21);

        // Deleting words backward with mixed Swedish characters
        assert!(input.delete_word_backward());
        assert_eq!(input.value(), "hallå-där_vännen! ");
        assert_eq!(input.cursor(), 18);

        assert!(input.delete_word_backward());
        assert_eq!(input.value(), "hallå-där_");
        assert_eq!(input.cursor(), 10);

        assert!(input.delete_word_backward());
        assert_eq!(input.value(), "hallå-");
        assert_eq!(input.cursor(), 6);

        assert!(input.delete_word_backward());
        assert_eq!(input.value(), "");
        assert_eq!(input.cursor(), 0);

        assert!(!input.delete_word_backward());
    }

    #[test]
    fn test_text_input_multibyte_paste_and_readline_kills() {
        let mut input = TextInput::new();
        // Paste with Swedish, Cyrillic, emojis and stripped newlines
        assert!(input.handle_paste("smörgås\r\nПривет\x00🦀"));
        assert_eq!(input.value(), "smörgåsПривет🦀");
        assert_eq!(input.cursor(), 14);

        // Ctrl+K from middle:
        // Set cursor to 7 (after "smörgås")
        input.set_cursor(7);
        assert!(input.delete_to_end());
        assert_eq!(input.value(), "smörgås");
        assert_eq!(input.cursor(), 7);

        // Ctrl+U from cursor 4 (before 'g')
        input.set_cursor(4);
        assert!(input.delete_to_start());
        assert_eq!(input.value(), "gås");
        assert_eq!(input.cursor(), 0);
    }

    #[test]
    fn test_text_input_rapid_insertions_and_deletions() {
        let mut input = TextInput::new();
        let samples = ['a', 'å', 'б', '🎉', 'z', 'ö', 'д', '🚀'];

        // Rapidly insert characters
        for &c in &samples {
            input.insert_char(c);
        }
        assert_eq!(input.cursor(), 8);
        assert_eq!(input.value().chars().count(), 8);

        // Move to middle and insert more
        input.set_cursor(4);
        for &c in &samples {
            input.insert_char(c);
        }
        assert_eq!(input.cursor(), 12);
        assert_eq!(input.value().chars().count(), 16);

        // Rapid deletions from middle
        for _ in 0..6 {
            assert!(input.delete_char_backward());
        }
        assert_eq!(input.cursor(), 6);
        assert_eq!(input.value().chars().count(), 10);

        for _ in 0..4 {
            assert!(input.delete_char_forward());
        }
        assert_eq!(input.cursor(), 6);
        assert_eq!(input.value().chars().count(), 6);
    }
}
