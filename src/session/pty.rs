use super::SessionError;
use portable_pty::{
    native_pty_system, Child, CommandBuilder, MasterPty, PtySize as PortablePtySize,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver};
use std::sync::{Arc, Mutex};
use std::thread;

/// Dimensions and resolution of a pseudo-terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PtySize {
    /// Number of columns (horizontal character cells).
    pub cols: u16,
    /// Number of rows (vertical character lines).
    pub rows: u16,
    /// Optional pixel width (defaults to 0 if unused).
    pub pixel_width: u16,
    /// Optional pixel height (defaults to 0 if unused).
    pub pixel_height: u16,
}

impl PtySize {
    /// Creates a new `PtySize` with the given columns and rows.
    pub fn new(cols: u16, rows: u16) -> Self {
        Self {
            cols,
            rows,
            pixel_width: 0,
            pixel_height: 0,
        }
    }

    /// Creates a new `PtySize` with columns, rows, and pixel dimensions.
    pub fn with_pixels(cols: u16, rows: u16, pixel_width: u16, pixel_height: u16) -> Self {
        Self {
            cols,
            rows,
            pixel_width,
            pixel_height,
        }
    }
}

impl Default for PtySize {
    fn default() -> Self {
        Self {
            cols: 80,
            rows: 24,
            pixel_width: 0,
            pixel_height: 0,
        }
    }
}

impl From<(u16, u16)> for PtySize {
    fn from((cols, rows): (u16, u16)) -> Self {
        Self::new(cols, rows)
    }
}

impl From<portable_pty::PtySize> for PtySize {
    fn from(size: portable_pty::PtySize) -> Self {
        Self {
            cols: size.cols,
            rows: size.rows,
            pixel_width: size.pixel_width,
            pixel_height: size.pixel_height,
        }
    }
}

impl From<PtySize> for portable_pty::PtySize {
    fn from(size: PtySize) -> Self {
        portable_pty::PtySize {
            cols: size.cols,
            rows: size.rows,
            pixel_width: size.pixel_width,
            pixel_height: size.pixel_height,
        }
    }
}

/// Represents the exit status of a child process executed in a PTY.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExitStatus {
    /// The numeric exit code returned by the process, if available.
    pub code: Option<u32>,
    /// Whether the process exited successfully (exit code 0).
    pub success: bool,
}

impl ExitStatus {
    /// Creates an `ExitStatus` from a numeric exit code.
    pub fn with_exit_code(code: u32) -> Self {
        Self {
            code: Some(code),
            success: code == 0,
        }
    }

    /// Creates an `ExitStatus` indicating a successful exit (0).
    pub fn success_status() -> Self {
        Self {
            code: Some(0),
            success: true,
        }
    }

    /// Creates an `ExitStatus` for a terminated/killed process with no explicit exit code.
    pub fn terminated() -> Self {
        Self {
            code: None,
            success: false,
        }
    }

    /// Returns whether the exit status represents a successful completion.
    pub fn success(&self) -> bool {
        self.success
    }

    /// Returns the exit code if available.
    pub fn exit_code(&self) -> Option<u32> {
        self.code
    }
}

impl From<portable_pty::ExitStatus> for ExitStatus {
    fn from(status: portable_pty::ExitStatus) -> Self {
        Self {
            code: Some(status.exit_code()),
            success: status.success(),
        }
    }
}

/// Configuration parameters for spawning a child process within a pseudo-terminal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PtyConfig {
    /// Command / executable to run (e.g., "claude", "agy", "sh", "cmd.exe").
    pub command: String,
    /// Command line arguments.
    pub args: Vec<String>,
    /// Initial working directory for the process.
    pub working_dir: PathBuf,
    /// Environment variables injected into the process environment.
    pub env: HashMap<String, String>,
    /// Initial terminal size (columns and rows).
    pub size: PtySize,
}

impl PtyConfig {
    /// Creates a new `PtyConfig` with the given command and working directory.
    pub fn new(command: impl Into<String>, working_dir: impl Into<PathBuf>) -> Self {
        Self {
            command: command.into(),
            args: Vec::new(),
            working_dir: working_dir.into(),
            env: HashMap::new(),
            size: PtySize::default(),
        }
    }

    /// Sets the command arguments.
    pub fn with_args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args = args.into_iter().map(Into::into).collect();
        self
    }

    /// Adds a single argument.
    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    /// Sets a single environment variable.
    pub fn with_env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.insert(key.into(), value.into());
        self
    }

    /// Sets all environment variables.
    pub fn with_envs(mut self, env: HashMap<String, String>) -> Self {
        self.env = env;
        self
    }

    /// Sets the initial PTY size.
    pub fn with_size(mut self, size: PtySize) -> Self {
        self.size = size;
        self
    }

    /// Sets the initial PTY size by columns and rows.
    pub fn with_dimensions(mut self, cols: u16, rows: u16) -> Self {
        self.size = PtySize::new(cols, rows);
        self
    }
}

/// A handle to a running pseudo-terminal session and child process.
pub struct PtySession {
    pub(crate) config: PtyConfig,
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Arc<Mutex<Box<dyn Child + Send + Sync>>>,
    rx: Receiver<Vec<u8>>,
    cached_exit_status: Arc<Mutex<Option<ExitStatus>>>,
}

impl PtySession {
    /// Spawns a new process in a pseudo-terminal according to the provided `PtyConfig`.
    pub fn spawn(config: PtyConfig) -> Result<Self, SessionError> {
        if !config.working_dir.exists() {
            return Err(SessionError::InvalidWorkingDirectory(
                config.working_dir.clone(),
            ));
        }
        if config.size.cols == 0 || config.size.rows == 0 {
            return Err(SessionError::InvalidSize {
                cols: config.size.cols,
                rows: config.size.rows,
            });
        }

        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(config.size.into())
            .map_err(|e| SessionError::Other(format!("Failed to open PTY pair: {e}")))?;

        let mut cmd = CommandBuilder::new(&config.command);
        cmd.args(&config.args);
        cmd.cwd(&config.working_dir);
        for (k, v) in &config.env {
            cmd.env(k, v);
        }

        let child = pair.slave.spawn_command(cmd).map_err(|e| {
            let err_str = e.to_string();
            if err_str.contains("not found")
                || err_str.contains("No such file or directory")
                || err_str.contains("cannot find the file specified")
            {
                SessionError::ProcessNotFound(config.command.clone())
            } else {
                SessionError::SpawnFailed {
                    command: config.command.clone(),
                    reason: err_str,
                }
            }
        })?;

        // Drop the slave handle in parent process so EOF works properly when child exits
        drop(pair.slave);

        let writer = pair
            .master
            .take_writer()
            .map_err(|e| SessionError::Other(format!("Failed to acquire PTY writer: {e}")))?;
        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| SessionError::Other(format!("Failed to clone PTY reader: {e}")))?;

        let (tx, rx) = channel();
        let _ = thread::Builder::new()
            .name("pty-reader".to_string())
            .spawn(move || {
                let mut buf = [0u8; 4096];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            if tx.send(buf[..n].to_vec()).is_err() {
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }
            });

        Ok(Self {
            config,
            master: pair.master,
            writer,
            child: Arc::new(Mutex::new(child)),
            rx,
            cached_exit_status: Arc::new(Mutex::new(None)),
        })
    }

    /// Writes raw bytes to the child process's standard input via the PTY master.
    pub fn write_all(&mut self, data: &[u8]) -> Result<(), SessionError> {
        self.writer.write_all(data).map_err(SessionError::Io)?;
        self.writer.flush().map_err(SessionError::Io)?;
        Ok(())
    }

    /// Reads available non-blocking output bytes from the PTY master.
    pub fn read_nonblocking(&mut self) -> Result<Vec<u8>, SessionError> {
        let mut data = Vec::new();
        while let Ok(chunk) = self.rx.try_recv() {
            data.extend_from_slice(&chunk);
        }
        Ok(data)
    }

    /// Resizes the pseudo-terminal window to the specified columns and rows.
    pub fn resize(&mut self, cols: u16, rows: u16) -> Result<(), SessionError> {
        if cols == 0 || rows == 0 {
            return Err(SessionError::InvalidSize { cols, rows });
        }
        let pty_size = PortablePtySize {
            cols,
            rows,
            pixel_width: self.config.size.pixel_width,
            pixel_height: self.config.size.pixel_height,
        };
        self.master
            .resize(pty_size)
            .map_err(|e| SessionError::Other(e.to_string()))?;
        self.config.size.cols = cols;
        self.config.size.rows = rows;
        Ok(())
    }

    /// Returns whether the child process is still alive and running.
    pub fn is_alive(&self) -> bool {
        if let Ok(cached) = self.cached_exit_status.lock() {
            if cached.is_some() {
                return false;
            }
        }
        if let Ok(mut child) = self.child.lock() {
            match child.try_wait() {
                Ok(Some(status)) => {
                    if let Ok(mut cached) = self.cached_exit_status.lock() {
                        *cached = Some(ExitStatus::from(status));
                    }
                    false
                }
                Ok(None) => true,
                Err(_) => false,
            }
        } else {
            false
        }
    }

    /// Non-blockingly checks if the child process has exited and returns its exit status.
    pub fn try_wait(&mut self) -> Result<Option<ExitStatus>, SessionError> {
        let mut cached = self
            .cached_exit_status
            .lock()
            .map_err(|_| SessionError::Other("Mutex poisoned".to_string()))?;
        if let Some(status) = *cached {
            return Ok(Some(status));
        }

        let mut child = self
            .child
            .lock()
            .map_err(|_| SessionError::Other("Mutex poisoned".to_string()))?;
        match child.try_wait() {
            Ok(Some(status)) => {
                let exit_status = ExitStatus::from(status);
                *cached = Some(exit_status);
                Ok(Some(exit_status))
            }
            Ok(None) => Ok(None),
            Err(e) => Err(SessionError::Other(e.to_string())),
        }
    }

    /// Terminate / kill the child process immediately.
    pub fn kill(&mut self) -> Result<(), SessionError> {
        let mut child = self
            .child
            .lock()
            .map_err(|_| SessionError::Other("Mutex poisoned".to_string()))?;
        match child.kill() {
            Ok(()) => {
                if let Ok(mut cached) = self.cached_exit_status.lock() {
                    if cached.is_none() {
                        *cached = Some(ExitStatus::terminated());
                    }
                }
                Ok(())
            }
            Err(e) => {
                // Check if process has already terminated
                if let Ok(Some(status)) = child.try_wait() {
                    if let Ok(mut cached) = self.cached_exit_status.lock() {
                        *cached = Some(ExitStatus::from(status));
                    }
                    return Ok(());
                }
                Err(SessionError::Other(e.to_string()))
            }
        }
    }

    /// Returns a reference to the configuration used to spawn this session.
    pub fn config(&self) -> &PtyConfig {
        &self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tempfile::tempdir;

    #[test]
    fn test_pty_size_defaults_and_constructors() {
        let default_size = PtySize::default();
        assert_eq!(default_size.cols, 80);
        assert_eq!(default_size.rows, 24);
        assert_eq!(default_size.pixel_width, 0);
        assert_eq!(default_size.pixel_height, 0);

        let custom_size = PtySize::new(120, 40);
        assert_eq!(custom_size.cols, 120);
        assert_eq!(custom_size.rows, 40);

        let pixel_size = PtySize::with_pixels(100, 30, 800, 600);
        assert_eq!(pixel_size.cols, 100);
        assert_eq!(pixel_size.rows, 30);
        assert_eq!(pixel_size.pixel_width, 800);
        assert_eq!(pixel_size.pixel_height, 600);

        let from_tuple: PtySize = (160, 50).into();
        assert_eq!(from_tuple.cols, 160);
        assert_eq!(from_tuple.rows, 50);
    }

    #[test]
    fn test_pty_size_portable_pty_conversions() {
        let pty_size = PtySize::with_pixels(132, 43, 1024, 768);
        let portable: portable_pty::PtySize = pty_size.into();
        assert_eq!(portable.cols, 132);
        assert_eq!(portable.rows, 43);
        assert_eq!(portable.pixel_width, 1024);
        assert_eq!(portable.pixel_height, 768);

        let converted_back: PtySize = portable.into();
        assert_eq!(converted_back, pty_size);
    }

    #[test]
    fn test_exit_status_helpers() {
        let success = ExitStatus::with_exit_code(0);
        assert!(success.success());
        assert_eq!(success.exit_code(), Some(0));

        let failure = ExitStatus::with_exit_code(127);
        assert!(!failure.success());
        assert_eq!(failure.exit_code(), Some(127));

        let explicit_success = ExitStatus::success_status();
        assert!(explicit_success.success());
        assert_eq!(explicit_success.exit_code(), Some(0));

        let terminated = ExitStatus::terminated();
        assert!(!terminated.success());
        assert_eq!(terminated.exit_code(), None);
    }

    #[test]
    fn test_pty_config_builder_and_defaults() {
        let working_dir = PathBuf::from("/workspace/project");
        let config = PtyConfig::new("claude", &working_dir)
            .arg("--dangerously-skip-permissions")
            .arg("--verbose")
            .with_env("ANTHROPIC_API_KEY", "sk-test-secret")
            .with_env("NODE_ENV", "test")
            .with_dimensions(100, 35);

        assert_eq!(config.command, "claude");
        assert_eq!(
            config.args,
            vec![
                "--dangerously-skip-permissions".to_string(),
                "--verbose".to_string()
            ]
        );
        assert_eq!(config.working_dir, PathBuf::from("/workspace/project"));
        assert_eq!(
            config.env.get("ANTHROPIC_API_KEY"),
            Some(&"sk-test-secret".to_string())
        );
        assert_eq!(config.env.get("NODE_ENV"), Some(&"test".to_string()));
        assert_eq!(config.size.cols, 100);
        assert_eq!(config.size.rows, 35);
    }

    #[test]
    fn test_pty_config_cross_platform_paths() {
        let win_path = PathBuf::from("C:\\Users\\Developer\\Projects\\botai_test");
        let win_config = PtyConfig::new("cmd.exe", &win_path)
            .with_args(vec!["/c", "dir"])
            .with_size(PtySize::new(80, 24));
        assert_eq!(win_config.working_dir, win_path);
        assert_eq!(win_config.command, "cmd.exe");
        assert_eq!(win_config.args, vec!["/c", "dir"]);

        let unix_path = PathBuf::from("/var/workspace/my_project_1");
        let unix_config = PtyConfig::new("sh", &unix_path)
            .with_args(vec!["-c", "ls -la"])
            .with_size(PtySize::new(120, 40));
        assert_eq!(unix_config.working_dir, unix_path);
        assert_eq!(unix_config.command, "sh");
    }

    #[test]
    fn test_pty_config_batch_envs_and_args() {
        let mut envs = HashMap::new();
        envs.insert("KEY1".to_string(), "VAL1".to_string());
        envs.insert("KEY2".to_string(), "VAL2".to_string());

        let config = PtyConfig::new("agy", PathBuf::from("/tmp"))
            .with_args(vec!["--model", "gemini-pro", "--stream"])
            .with_envs(envs.clone());

        assert_eq!(config.args.len(), 3);
        assert_eq!(config.env, envs);
    }

    #[test]
    fn test_pty_session_invalid_working_directory() {
        let invalid_dir = PathBuf::from("/path/that/definitely/does/not/exist/botai_12345");
        let config = PtyConfig::new("echo", invalid_dir.clone());
        let result = PtySession::spawn(config);
        assert!(matches!(
            result,
            Err(SessionError::InvalidWorkingDirectory(_))
        ));
    }

    #[test]
    fn test_pty_session_invalid_dimensions() {
        let tmp = tempdir().expect("Failed to create tempdir");
        let config = PtyConfig::new("echo", tmp.path()).with_dimensions(0, 0);
        let result = PtySession::spawn(config);
        assert!(matches!(result, Err(SessionError::InvalidSize { .. })));
    }

    #[test]
    fn test_pty_session_spawn_and_read_output() {
        let tmp = tempdir().expect("Failed to create tempdir");
        #[cfg(windows)]
        let (cmd, args) = ("cmd.exe", vec!["/c", "echo hello_from_pty_unit_test"]);
        #[cfg(not(windows))]
        let (cmd, args) = ("sh", vec!["-c", "echo hello_from_pty_unit_test"]);

        let config = PtyConfig::new(cmd, tmp.path()).with_args(args);
        let mut session = PtySession::spawn(config).expect("Failed to spawn PTY session");

        // Wait a brief moment for child to execute and output
        thread::sleep(Duration::from_millis(150));

        let mut output = String::new();
        for _ in 0..10 {
            if let Ok(bytes) = session.read_nonblocking() {
                if !bytes.is_empty() {
                    output.push_str(&String::from_utf8_lossy(&bytes));
                }
            }
            if output.contains("hello_from_pty_unit_test") {
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }

        assert!(
            output.contains("hello_from_pty_unit_test"),
            "Expected output to contain 'hello_from_pty_unit_test', got: {output}"
        );
    }

    #[test]
    fn test_pty_session_resize() {
        let tmp = tempdir().expect("Failed to create tempdir");
        #[cfg(windows)]
        let (cmd, args) = ("cmd.exe", vec!["/c", "echo resized"]);
        #[cfg(not(windows))]
        let (cmd, args) = ("sh", vec!["-c", "echo resized"]);

        let config = PtyConfig::new(cmd, tmp.path()).with_args(args);
        let mut session = PtySession::spawn(config).expect("Failed to spawn PTY session");

        assert!(session.resize(100, 30).is_ok());
        assert_eq!(session.config().size.cols, 100);
        assert_eq!(session.config().size.rows, 30);

        assert!(matches!(
            session.resize(0, 0),
            Err(SessionError::InvalidSize { cols: 0, rows: 0 })
        ));
    }
}
