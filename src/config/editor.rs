use std::path::{Path, PathBuf};

use crate::config::ConfigError;

/// Target platform for editor fallback resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetPlatform {
    Unix,
    Windows,
}

/// Splits a shell command string into executable binary and argument tokens,
/// respecting single quotes, double quotes, and backslash escaping.
pub fn split_command_words(s: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut in_single_quotes = false;
    let mut in_double_quotes = false;
    let mut had_char = false;

    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let ch = chars[i];

        if in_single_quotes {
            if ch == '\'' {
                in_single_quotes = false;
            } else {
                current.push(ch);
            }
            had_char = true;
            i += 1;
            continue;
        }

        if in_double_quotes {
            if ch == '"' {
                in_double_quotes = false;
                had_char = true;
                i += 1;
                continue;
            }

            if ch == '\\' {
                if i + 1 < chars.len() {
                    let next_ch = chars[i + 1];
                    if next_ch == '"' || next_ch == '\\' || next_ch == '$' || next_ch == '`' {
                        current.push(next_ch);
                        had_char = true;
                        i += 2;
                        continue;
                    }
                }
                current.push('\\');
                had_char = true;
                i += 1;
                continue;
            }

            current.push(ch);
            had_char = true;
            i += 1;
            continue;
        }

        // Outside quotes
        if ch == '\'' {
            in_single_quotes = true;
            had_char = true;
            i += 1;
            continue;
        }

        if ch == '"' {
            in_double_quotes = true;
            had_char = true;
            i += 1;
            continue;
        }

        if ch == '\\' {
            if i + 1 < chars.len() {
                let next_ch = chars[i + 1];
                current.push(next_ch);
                had_char = true;
                i += 2;
                continue;
            } else {
                current.push('\\');
                had_char = true;
                i += 1;
                continue;
            }
        }

        if ch.is_whitespace() {
            if had_char || !current.is_empty() {
                words.push(current);
                current = String::new();
                had_char = false;
            }
            i += 1;
            continue;
        }

        current.push(ch);
        had_char = true;
        i += 1;
    }

    if had_char || !current.is_empty() {
        words.push(current);
    }

    words
}

/// Finds the absolute or relative path to an executable if it exists and is executable in `$PATH` or directly.
pub fn find_executable_in_path(cmd: &str) -> Option<PathBuf> {
    let cmd_trimmed = cmd.trim();
    if cmd_trimmed.is_empty() {
        return None;
    }

    let cmd_path = Path::new(cmd_trimmed);
    // If cmd contains path separators (e.g. /usr/bin/nano or ./my-editor), check directly
    if cmd_path.components().count() > 1 {
        if is_file_or_executable(cmd_path) {
            return Some(cmd_path.to_path_buf());
        }
        #[cfg(windows)]
        {
            for ext in &[".exe", ".cmd", ".bat"] {
                let candidate = format!("{}{}", cmd_trimmed, ext);
                let p = Path::new(&candidate);
                if is_file_or_executable(p) {
                    return Some(p.to_path_buf());
                }
            }
        }
        return None;
    }

    if let Some(paths) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&paths) {
            let candidate = dir.join(cmd_trimmed);
            if is_file_or_executable(&candidate) {
                return Some(candidate);
            }
            #[cfg(windows)]
            {
                for ext in &[".exe", ".cmd", ".bat"] {
                    let candidate_ext = dir.join(format!("{}{}", cmd_trimmed, ext));
                    if is_file_or_executable(&candidate_ext) {
                        return Some(candidate_ext);
                    }
                }
            }
        }
    }

    None
}

/// Returns true if the given command or binary is found in `$PATH` or executable directly.
pub fn is_executable_in_path(cmd: &str) -> bool {
    find_executable_in_path(cmd).is_some()
}

fn is_file_or_executable(path: &Path) -> bool {
    if let Ok(metadata) = path.metadata() {
        if metadata.is_file() {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                return metadata.permissions().mode() & 0o111 != 0;
            }
            #[cfg(not(unix))]
            {
                return true;
            }
        }
    }
    false
}

/// Resolves the editor command with configurable environment lookup, path lookup, and target platform.
pub fn resolve_editor_with_platform<E, P>(
    cli_editor: Option<&str>,
    env_lookup: E,
    is_in_path: P,
    platform: TargetPlatform,
) -> Result<Vec<String>, ConfigError>
where
    E: Fn(&str) -> Option<String>,
    P: Fn(&str) -> bool,
{
    // 1. CLI --editor <cmd> / -e <cmd>
    if let Some(cmd) = cli_editor {
        let words = split_command_words(cmd);
        if !words.is_empty() {
            return Ok(words);
        }
    }

    // 2. $VISUAL environment variable
    if let Some(val) = env_lookup("VISUAL") {
        if !val.trim().is_empty() {
            let words = split_command_words(&val);
            if !words.is_empty() {
                return Ok(words);
            }
        }
    }

    // 3. $EDITOR environment variable
    if let Some(val) = env_lookup("EDITOR") {
        if !val.trim().is_empty() {
            let words = split_command_words(&val);
            if !words.is_empty() {
                return Ok(words);
            }
        }
    }

    // 4. Platform fallbacks
    match platform {
        TargetPlatform::Unix => {
            for candidate in ["nano", "vim", "vi"] {
                if is_in_path(candidate) {
                    return Ok(vec![candidate.to_string()]);
                }
            }
            // 5. If none resolved or found in $PATH
            Err(ConfigError::NoEditorFound)
        }
        TargetPlatform::Windows => {
            // Windows fallback: notepad.exe
            Ok(vec!["notepad.exe".to_string()])
        }
    }
}

/// Resolves the editor command to use, based on CLI override, $VISUAL, $EDITOR, or platform fallbacks.
pub fn resolve_editor(cli_editor: Option<&str>) -> Result<Vec<String>, ConfigError> {
    let platform = if cfg!(windows) {
        TargetPlatform::Windows
    } else {
        TargetPlatform::Unix
    };

    resolve_editor_with_platform(
        cli_editor,
        |k| std::env::var(k).ok(),
        is_executable_in_path,
        platform,
    )
}

/// Spawns the editor command with the given configuration file path as argument,
/// inheriting standard I/O (stdin, stdout, stderr), and waits for completion.
pub fn run_editor(
    editor_cmd: &[String],
    file_path: &Path,
) -> Result<std::process::ExitStatus, ConfigError> {
    if editor_cmd.is_empty() {
        return Err(ConfigError::EmptyEditorCommand);
    }

    let binary = &editor_cmd[0];
    let args = &editor_cmd[1..];

    let mut command = std::process::Command::new(binary);
    command.args(args);
    command.arg(file_path);
    command.stdin(std::process::Stdio::inherit());
    command.stdout(std::process::Stdio::inherit());
    command.stderr(std::process::Stdio::inherit());

    let mut child = command
        .spawn()
        .map_err(|e| ConfigError::EditorExecutionFailed {
            command: binary.clone(),
            source: e,
        })?;

    let status = child
        .wait()
        .map_err(|e| ConfigError::EditorExecutionFailed {
            command: binary.clone(),
            source: e,
        })?;

    Ok(status)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_command_words_basic() {
        assert_eq!(split_command_words("nano"), vec!["nano"]);
        assert_eq!(split_command_words("code --wait"), vec!["code", "--wait"]);
        assert_eq!(
            split_command_words("code --wait --new-window"),
            vec!["code", "--wait", "--new-window"]
        );
        assert_eq!(split_command_words("  subl   -w  "), vec!["subl", "-w"]);
    }

    #[test]
    fn test_split_command_words_quotes_and_escapes() {
        assert_eq!(split_command_words("'code' -w"), vec!["code", "-w"]);
        assert_eq!(
            split_command_words("\"/usr/bin/my editor\" -a -b"),
            vec!["/usr/bin/my editor", "-a", "-b"]
        );
        assert_eq!(split_command_words("code\\ --wait"), vec!["code --wait"]);
        assert_eq!(
            split_command_words("\"C:\\Program Files\\Editor\\editor.exe\" --wait"),
            vec!["C:\\Program Files\\Editor\\editor.exe", "--wait"]
        );
        assert_eq!(
            split_command_words("editor \"\" arg"),
            vec!["editor", "", "arg"]
        );
    }

    #[test]
    fn test_split_command_words_empty() {
        assert!(split_command_words("").is_empty());
        assert!(split_command_words("   ").is_empty());
        assert!(split_command_words("\t\n  ").is_empty());
    }

    #[test]
    fn test_resolve_editor_cli_precedence() {
        let env_map = |k: &str| match k {
            "VISUAL" => Some("subl -w".to_string()),
            "EDITOR" => Some("nano".to_string()),
            _ => None,
        };
        let path_lookup = |_cmd: &str| true;

        let res = resolve_editor_with_platform(
            Some("nvim -u NONE"),
            env_map,
            path_lookup,
            TargetPlatform::Unix,
        )
        .expect("CLI override should succeed");

        assert_eq!(res, vec!["nvim", "-u", "NONE"]);
    }

    #[test]
    fn test_resolve_editor_visual_precedence_over_editor() {
        let env_map = |k: &str| match k {
            "VISUAL" => Some("code --wait".to_string()),
            "EDITOR" => Some("nano".to_string()),
            _ => None,
        };
        let path_lookup = |_cmd: &str| true;

        let res = resolve_editor_with_platform(None, env_map, path_lookup, TargetPlatform::Unix)
            .expect("VISUAL should resolve");

        assert_eq!(res, vec!["code", "--wait"]);
    }

    #[test]
    fn test_resolve_editor_editor_used_when_visual_empty() {
        let env_map = |k: &str| match k {
            "VISUAL" => Some("   ".to_string()),
            "EDITOR" => Some("vim".to_string()),
            _ => None,
        };
        let path_lookup = |_cmd: &str| true;

        let res = resolve_editor_with_platform(None, env_map, path_lookup, TargetPlatform::Unix)
            .expect("EDITOR should resolve when VISUAL is whitespace");

        assert_eq!(res, vec!["vim"]);
    }

    #[test]
    fn test_resolve_editor_unix_fallbacks_nano() {
        let env_map = |_k: &str| None;
        let path_lookup = |cmd: &str| cmd == "nano" || cmd == "vim" || cmd == "vi";

        let res = resolve_editor_with_platform(None, env_map, path_lookup, TargetPlatform::Unix)
            .expect("nano fallback should resolve");

        assert_eq!(res, vec!["nano"]);
    }

    #[test]
    fn test_resolve_editor_unix_fallbacks_vim_when_nano_missing() {
        let env_map = |_k: &str| None;
        let path_lookup = |cmd: &str| cmd == "vim" || cmd == "vi";

        let res = resolve_editor_with_platform(None, env_map, path_lookup, TargetPlatform::Unix)
            .expect("vim fallback should resolve");

        assert_eq!(res, vec!["vim"]);
    }

    #[test]
    fn test_resolve_editor_unix_fallbacks_vi_when_nano_and_vim_missing() {
        let env_map = |_k: &str| None;
        let path_lookup = |cmd: &str| cmd == "vi";

        let res = resolve_editor_with_platform(None, env_map, path_lookup, TargetPlatform::Unix)
            .expect("vi fallback should resolve");

        assert_eq!(res, vec!["vi"]);
    }

    #[test]
    fn test_resolve_editor_unix_error_when_none_found() {
        let env_map = |_k: &str| None;
        let path_lookup = |_cmd: &str| false;

        let res = resolve_editor_with_platform(None, env_map, path_lookup, TargetPlatform::Unix);

        match res {
            Err(ConfigError::NoEditorFound) => {}
            other => panic!("Expected ConfigError::NoEditorFound, got {other:?}"),
        }
    }

    #[test]
    fn test_resolve_editor_windows_fallback() {
        let env_map = |_k: &str| None;
        let path_lookup = |_cmd: &str| false;

        let res = resolve_editor_with_platform(None, env_map, path_lookup, TargetPlatform::Windows)
            .expect("Windows fallback should resolve");

        assert_eq!(res, vec!["notepad.exe"]);
    }

    #[test]
    fn test_run_editor_empty_command_error() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let temp_file = temp_dir.path().join("config.toml");
        let res = run_editor(&[], &temp_file);
        match res {
            Err(ConfigError::EmptyEditorCommand) => {}
            other => panic!("Expected ConfigError::EmptyEditorCommand, got {other:?}"),
        }
    }

    #[test]
    fn test_run_editor_invalid_binary_error() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let temp_file = temp_dir.path().join("config.toml");
        let res = run_editor(
            &["__non_existent_binary_xyz_12345__".to_string()],
            &temp_file,
        );
        match res {
            Err(ConfigError::EditorExecutionFailed { command, .. }) => {
                assert_eq!(command, "__non_existent_binary_xyz_12345__");
            }
            other => panic!("Expected ConfigError::EditorExecutionFailed, got {other:?}"),
        }
    }

    #[test]
    fn test_run_editor_successful_execution() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let temp_file = temp_dir.path().join("config.toml");
        std::fs::write(&temp_file, "default_agent = 'claude'").expect("write temp");

        #[cfg(unix)]
        let cmd = vec!["true".to_string()];
        #[cfg(windows)]
        let cmd = vec![
            "cmd.exe".to_string(),
            "/C".to_string(),
            "exit".to_string(),
            "0".to_string(),
        ];

        let status = run_editor(&cmd, &temp_file).expect("run_editor true/cmd should succeed");
        assert!(status.success());
    }
}
