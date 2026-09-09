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

    /// Inserts a single character at the current cursor offset.
    /// Newlines (`\r`, `\n`) and control characters are ignored.
    pub fn insert_char(&mut self, c: char) {
        if c == '\n' || c == '\r' || c.is_control() {
            return;
        }
        let mut chars: Vec<char> = self.value.chars().collect();
        let idx = self.cursor.min(chars.len());
        chars.insert(idx, c);
        self.value = chars.into_iter().collect();
        self.cursor = idx + 1;
    }

    /// Inserts a string at the current cursor offset, stripping newlines and control characters.
    pub fn insert_str(&mut self, text: &str) -> bool {
        let mut cleaned = Vec::new();
        for c in text.chars() {
            if c != '\n' && c != '\r' && !c.is_control() {
                cleaned.push(c);
            }
        }
        if cleaned.is_empty() {
            return false;
        }
        let mut chars: Vec<char> = self.value.chars().collect();
        let idx = self.cursor.min(chars.len());
        let count = cleaned.len();
        chars.splice(idx..idx, cleaned);
        self.value = chars.into_iter().collect();
        self.cursor = idx + count;
        true
    }

    /// Deletes the character before the cursor (Backspace).
    /// Returns `true` if a character was removed, or `false` if cursor is at position 0.
    pub fn delete_char_backward(&mut self) -> bool {
        if self.cursor == 0 {
            return false;
        }
        let mut chars: Vec<char> = self.value.chars().collect();
        let idx = self.cursor.min(chars.len());
        if idx > 0 {
            chars.remove(idx - 1);
            self.value = chars.into_iter().collect();
            self.cursor = idx - 1;
            true
        } else {
            false
        }
    }

    /// Deletes the character under the cursor (Delete).
    /// Returns `true` if a character was removed, or `false` if cursor is at the end.
    pub fn delete_char_forward(&mut self) -> bool {
        let mut chars: Vec<char> = self.value.chars().collect();
        let idx = self.cursor;
        if idx < chars.len() {
            chars.remove(idx);
            self.value = chars.into_iter().collect();
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
        let chars: Vec<char> = self.value.chars().collect();
        let idx = self.cursor.min(chars.len());
        if idx > 0 {
            let remaining: String = chars[idx..].iter().collect();
            self.value = remaining;
            self.cursor = 0;
            true
        } else {
            false
        }
    }

    /// Deletes text from the cursor to the end of the line (Ctrl+K).
    /// Returns `true` if any characters were deleted.
    pub fn delete_to_end(&mut self) -> bool {
        let chars: Vec<char> = self.value.chars().collect();
        let idx = self.cursor.min(chars.len());
        if idx < chars.len() {
            let remaining: String = chars[..idx].iter().collect();
            self.value = remaining;
            true
        } else {
            false
        }
    }

    /// Deletes the word immediately preceding the cursor (Ctrl+W).
    /// Returns `true` if any characters were deleted.
    pub fn delete_word_backward(&mut self) -> bool {
        let chars: Vec<char> = self.value.chars().collect();
        let idx = self.cursor.min(chars.len());
        if idx == 0 {
            return false;
        }
        let mut pos = idx;
        while pos > 0 && !chars[pos - 1].is_alphanumeric() {
            pos -= 1;
        }
        while pos > 0 && chars[pos - 1].is_alphanumeric() {
            pos -= 1;
        }
        if pos == idx {
            return false;
        }
        let mut new_chars = Vec::new();
        new_chars.extend_from_slice(&chars[..pos]);
        new_chars.extend_from_slice(&chars[idx..]);
        self.value = new_chars.into_iter().collect();
        self.cursor = pos;
        true
    }

    /// Moves the cursor backward one word (Ctrl+Left / Alt+B).
    pub fn move_word_backward(&mut self) {
        let chars: Vec<char> = self.value.chars().collect();
        let mut pos = self.cursor.min(chars.len());
        if pos == 0 {
            return;
        }
        while pos > 0 && !chars[pos - 1].is_alphanumeric() {
            pos -= 1;
        }
        while pos > 0 && chars[pos - 1].is_alphanumeric() {
            pos -= 1;
        }
        self.cursor = pos;
    }

    /// Moves the cursor forward one word (Ctrl+Right / Alt+F).
    pub fn move_word_forward(&mut self) {
        let chars: Vec<char> = self.value.chars().collect();
        let len = chars.len();
        let mut pos = self.cursor.min(len);
        if pos >= len {
            return;
        }
        while pos < len && !chars[pos].is_alphanumeric() {
            pos += 1;
        }
        while pos < len && chars[pos].is_alphanumeric() {
            pos += 1;
        }
        self.cursor = pos;
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
}
