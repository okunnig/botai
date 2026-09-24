# Requirements for botai (Multi-Repo Workspace Orchestrator & Session Multiplexer)

`botai` is a cross-platform multi-repo workspace orchestrator and session multiplexer for AI agent workflows (e.g., Claude Code, agy, GitHub Copilot CLI, Aider) written in idiomatic Rust.

---

## 1. Core Architecture & Platform Support
- **Cross-Platform**: Compiles natively to Linux, macOS, and Windows (using Windows ConPTY).
- **Strict CLI / TUI Application**: Single binary with zero external runtime dependencies (no system `tmux` or `zellij` requirement).
- **Embedded Terminal Multiplexer**: Built-in PTY management and TUI multiplexer using `portable-pty`, `ratatui`, and `crossterm`.

---

## 2. Configuration & Template Management
- **Declarative TOML Configuration**: Project templates and agent tool definitions are configured via a user configuration file:
  - Linux / macOS: `~/.config/botai/config.toml` (or `$XDG_CONFIG_HOME/botai/config.toml`)
  - Windows: `%APPDATA%\botai\config.toml`
- **Central Repositories Directory (`repos_dir`)**:
  - Required global configuration field `repos_dir` specifying where remote repositories are cloned and cached.
  - Does not assume any implicit default directory; must be explicitly configured in `config.toml` (e.g. `repos_dir = "~/.local/share/botai/repos"` or `repos_dir = "~/code/repos"`).
  - Can be overridden at runtime via the `BOTAI_REPOS_DIR` environment variable.
- **Central Repositories Registry (`[repos]`) & Reusable Definitions**:
  - Optional top-level `[repos.<name>]` configuration table allowing repositories and their canonical build/setup pipelines to be defined once and reused across multiple templates.
  - Fields for `[repos.<name>]`:
    - `url`: (Optional) Remote Git clone URL or local directory path.
    - `setup`: (Optional) List of setup commands to execute in the repository's worktree upon creation (e.g. `setup = [ ["npm", "install"], ["npm", "run", "codegen"] ]` or `setup = ["npm install", "cargo check"]`).
    - `env`: (Optional) Per-repository environment variables applied during setup hook execution.
- **Hybrid Repository Declarations & Selective Inclusion**:
  - Repositories in `templates.<name>.repos` support:
    - **Registered repo keys** (e.g., `"web-frontend"`): Resolves to the matching `[repos.web-frontend]` entry, inheriting its clone URL, setup hooks, and environment.
    - **Remote Git URLs** (`https://...`, `git@...`, `ssh://...`): Automatically cloned into `<repos_dir>/<repo_name>`.
    - **Relative repo names** (e.g., `"ecommerce-api"`): Resolved relative to `repos_dir` (`<repos_dir>/ecommerce-api`).
    - **Absolute local paths** (e.g., `/home/user/code/my-repo`): Direct reference to existing local repository.
  - **Repository Entry Formats & Default Inclusion (`default`)**:
    - Repository entries in a template's `repos` list support two syntactic forms:
      - **Simple Strings**: e.g., `"ecommerce-api"` or `"https://github.com/example/api.git"`. Treated as enabled by default (`default = true`).
      - **Inline Tables**: e.g., `{ repo = "docs", default = false }` or `{ name = "docs", default = false }`. Supports either `repo` or `name` field alias for the repository identifier.
    - **`default` (Optional Boolean)**: Controls whether the repository is included by default during project creation. Defaults to `true` when omitted.
      - When `default = true` (or when specified as a simple string), the repository is pre-selected by default in the TUI creation modal checklist and included automatically by CLI `botai create`.
      - When `default = false`, the repository is marked optional (opt-in) and is unselected by default in the TUI creation modal checklist and omitted by CLI `botai create` unless explicitly requested via `--include <repo>` or `--all`.
  - Sanitized flat naming inside `repos_dir` based on the repository basename.
  - Conflict detection: Errors if two different remote URLs resolve to the same folder name.
- **Templates / Project Templates**:
  - Define named sets of local or remote repositories. Each template can specify:
    - `description`: Optional summary of the template.
    - `default_agent`: Optional agent override specific to this template.
    - `agents_md_append`: Optional custom markdown instructions appended to `AGENTS.md`.
    - `repos`: List of repository entries (simple strings or inline tables with `default = true|false`).
    - `env`: Environment variables injected into the agent session and workspace hooks.
    - `post_create`: (Optional) List of commands executed at the project root after worktrees and setup hooks finish.
    - `pre_destroy`: (Optional) List of commands executed at the project root before destroying worktrees.
- **Agent Definitions**: Configurable agent launch commands, arguments, and environment variables.
- **Dynamic Configuration & Template Reloading**:
  - `Config` is designed as a pure, lightweight data structure containing parsed configuration without filesystem path tracking or self-reloading responsibilities.
  - Runtime application state containers (`TuiApp` and `DashboardState`) track the configuration file path (`config_path`), last known filesystem modification time (`config_last_modified` / `mtime`), and file size (`config_file_len` / `len`).
  - When initiating project creation (opening the project creation modal via `n`/`c` on the dashboard, via the in-session launcher shortcut, or submitting creation), `botai` checks both filesystem modification time (`mtime`) and file size (`len`) against `config_last_modified` and `config_file_len`. If both `mtime` and `len` are unchanged, disk reloading is skipped (returning `Ok(false)`), avoiding redundant I/O and deserialization. When either timestamp or file size changes, configuration is reloaded from disk (returning `Ok(true)`), immediately updating active templates and settings. Checking both attributes ensures robust cross-platform change detection even within coarse timestamp intervals (e.g. Windows NTFS ~15ms timestamp granularity).
  - Enables users to add, update, or remove templates and agent tools in `config.toml` (e.g. via `botai edit` in another terminal or an external editor) and immediately select and instantiate them without restarting `botai`.
  - **Graceful Error Handling**: If the configuration file on disk contains invalid TOML syntax or cannot be read while being edited, `botai` gracefully retains the last valid active configuration without crashing or interrupting running sessions.
- **Configuration Template Generation (`botai init`)**:
  - Built-in generator for a fully documented, self-describing configuration file with extensive explanatory comments.
  - Serves as an interactive starting template and inline documentation for all supported agents, project templates, repos, and environment variable configurations.
  - Automatically targets the standard platform configuration location with safe directory creation and overwrite guards.

### Annotated `config.toml` Template
```toml
# ==============================================================================
# botai - Multi-Repo Workspace Orchestrator Configuration File
# ==============================================================================
# Default locations:
#   - Linux/macOS: ~/.config/botai/config.toml (or $XDG_CONFIG_HOME/botai/config.toml)
#   - Windows:     %APPDATA%\botai\config.toml
# ==============================================================================

# Global default AI agent tool to use when a template does not specify an override.
default_agent = "claude"

# Color theme for the TUI interface ("tokyo-night", "catppuccin", "cyberpunk", "gruvbox", "monochrome", "ansi").
theme = "tokyo-night"

# Central directory where remote repositories are cloned and cached (required).
# Can also be overridden at runtime by setting the BOTAI_REPOS_DIR environment variable.
repos_dir = "~/.local/share/botai/repos"

# ------------------------------------------------------------------------------
# Agent Definitions
# ------------------------------------------------------------------------------
# Configure the AI agent CLI commands, CLI arguments, and environment variables.

[agents.claude]
command = "claude"
args = []
# env = { ANTHROPIC_API_KEY = "sk-ant-..." }

[agents.agy]
command = "agy"
args = []

[agents.copilot]
command = "gh"
args = ["copilot", "suggest"]

[agents.aider]
command = "aider"
args = ["--no-auto-commits"]

# ------------------------------------------------------------------------------
# Reusable Repository Definitions (Optional)
# ------------------------------------------------------------------------------
# Define canonical repository URLs, multi-command setup pipelines, and environment
# variables once for reuse across multiple project templates.
# Each repository entry supports:
# - url: (Optional) Remote Git clone URL or local directory path.
# - setup: (Optional) Lifecycle commands executed in the worktree after checkout.
# - env: (Optional) Per-repository environment variables applied during setup hooks.

[repos.web-frontend]
url = "https://github.com/example/ecommerce-web.git"
setup = [
  ["npm", "install"],
  ["npm", "run", "codegen"]
]
env = { NODE_ENV = "development" }

[repos.ecommerce-api]
url = "https://github.com/example/ecommerce-api.git"
setup = [
  ["cargo", "check"]
]

[repos.ecommerce-docs]
url = "https://github.com/example/ecommerce-docs.git"
setup = [
  ["npm", "run", "docs:build"]
]
env = { DOCS_ENV = "local" }

# ------------------------------------------------------------------------------
# Project Templates
# ------------------------------------------------------------------------------
# Define named multi-repo project templates. Each template specifies:
# - description: Optional summary of the template.
# - default_agent: Optional agent override specific to this template.
# - agents_md_append: (Optional) Custom markdown instructions appended to AGENTS.md.
# - repos: List of repository entries (simple strings or inline tables with `default = true|false`).
# - env: Environment variables injected into the agent session.
# - post_create: (Optional) Commands executed at project root after setup.
# - pre_destroy: (Optional) Commands executed at project root before cleanup.

[templates.ecommerce]
description = "E-commerce platform services"
default_agent = "agy"
agents_md_append = """
- Run integration tests with `cargo test --workspace`.
- Follow coding standards documented in the template.
"""
repos = [
  "ecommerce-api",                               # Simple registered repo key (included by default)
  { repo = "web-frontend", default = true },     # Inline table explicitly included by default
  { name = "ecommerce-docs", default = false },  # Opt-in / optional repo (unselected by default in TUI/CLI)
  "/home/user/code/ecommerce-shared"             # Local filesystem repository path
]
post_create = [
  ["docker", "compose", "up", "-d"]
]
pre_destroy = [
  ["docker", "compose", "down"]
]
env = { PORT = "8080", NODE_ENV = "development" }
```

---

## 3. Project Creation & Git Worktree Lifecycle
- **Project Structure**: When starting a project for a template, `botai` creates a project root directory in the current working directory (`CWD/<project_name>`).
- **Selective Repository Orchestration**:
  - **Selective Provisioning**: When creating a project workspace, only the actively selected/included repositories are cloned, fetched, checked out into worktrees, and have setup hooks executed.
    - Repositories omitted or unselected (e.g. optional repositories with `default = false` not explicitly included, or repositories excluded via CLI `--exclude` or unchecked in the TUI modal) are skipped entirely: no directory is created in the project workspace, no worktree is checked out, no upstream git fetch is performed, and no per-repo setup hooks are run.
    - Synthesized context (`AGENTS.md`) only documents and references the repositories that were actively selected and provisioned into the project.
  - **Zero-Repository Validation Rule**:
    - At least one repository must be selected when creating a project.
    - Attempting to initialize a project with zero selected repositories fails with an immediate validation error (`"At least one repository must be selected for project creation"`) and cleanly aborts initialization before creating any project directories, worktrees, or branches.
- **Automatic On-Demand Cloning**:
  - Missing repositories in `repos_dir` are automatically cloned on-demand before worktree creation (only for selected repositories).
  - If a clone fails (e.g. invalid URL, authentication failure), project initialization cleanly rolls back partially created directories.
- **Best-Effort Upstream Fetching**:
  - Performs a best-effort `git fetch origin` for cached selected repositories to discover upstream default branch changes.
  - Branches worktrees off `origin/<default_branch>` when available, or falls back to local default branch if offline.
- **Worktrees & Metadata Lifecycle**: For every selected repository in the project:
  - Executes `git worktree prune` prior to creating new worktrees to automatically clear stale worktree metadata from manually deleted directories.
  - Creates a git worktree inside `<project_name>/<repo_dir_name>`.
  - Checks out a new branch named `<project_name>`.
- **Lifecycle Command Hooks Execution Model**:
  - **Per-Repository Setup Hooks (`setup`)**:
    - Runs in the checked-out worktree directory (`CWD/<project_name>/<repo_name>`) immediately after worktree creation.
    - Repositories execute their setup command pipelines concurrently in parallel; commands within a single repository run sequentially in order.
    - Direct execution without shell wrappers (arguments preserved safely).
    - **Warn and Continue Policy**: Non-zero exit codes or execution failures are logged with warning events but do NOT trigger destructive rollback of the workspace, ensuring developers and AI agents can inspect and fix setup issues.
  - **Workspace Post-Creation Hooks (`post_create`)**:
    - Runs sequentially at the project root directory (`CWD/<project_name>`) after all worktrees and repo setup hooks complete and `AGENTS.md` is generated.
    - Inherits template environment variables.
    - Failure policy: Warn and Continue.
  - **Workspace Pre-Destruction Hooks (`pre_destroy`)**:
    - Runs at the project root directory during project destruction before removing worktree directories and branches.
  - **Windows Executable & PATHEXT Probing**:
    - On Windows, hook execution probes `%PATH%` and `%PATHEXT%` (`.COM`, `.EXE`, `.BAT`, `.CMD`) to automatically resolve scripts like `npm`, `yarn`, and `pnpm`.
    - Automatically resolves bare commands and path-prefixed scripts into their `.cmd` or `.bat` executables so Rust's `std::process::Command` invokes them with Windows `cmd.exe /c` batch wrapping without requiring explicit file extensions or shell wrappers.
  - **Strict Branching Strategy**: Branches strictly off the latest default branch (`origin/main`, `origin/master`, or local `main` / `master`) of each repository.
  - **Collision Prevention**: Fails with an explicit error if a branch or worktree named `<project_name>` already exists in any of the template repositories or if `<project_name>` directory exists.
  - **Backing Repository Auto-Discovery (`resolve_main_repo`)**: Resolves the main cached repository from any worktree directory using `git rev-parse --git-common-dir` and `.git` file (`gitdir: <path>`) fallback traversal.
  - **Clean Destruction & Metadata Pruning (`remove_worktree_auto`)**: Validates uncommitted changes (guarded unless forced), removes worktrees directly from backing central repositories, and executes `git worktree prune` so stale `.git/worktrees/<name>` metadata entries are eliminated while preserving git branch history.
- **Agent Execution**: Spawns a single agent CLI instance at the project root (`CWD/<project_name>`), providing access to all worktree subdirectories.

---

## 4. Multi-Repo Context Synthesis & Seeding
- **Auto-Generated Context Files (`AGENTS.md` / `WORKSPACE.md` / `CLAUDE.md`)**:
  - Automatically synthesizes a root orientation file inside `CWD/<project_name>` detailing:
    - The multi-repo structure and mapping of child worktree directories.
    - Each repository's purpose and inter-dependencies.
    - Guidance for the AI agent on navigating, building, and modifying multiple repos simultaneously.
    - Explicit repository guidelines instructing AI agents to inspect and adhere to local instruction files (`AGENTS.md`, `CLAUDE.md`, `GEMINI.md`) in child worktrees prior to planning, reading, or modifying code in those subdirectories.
  - **`CLAUDE.md` Context Import (`@AGENTS.md`)**:
    - When `AGENTS.md` is created at the project root, automatically creates a `CLAUDE.md` file at the project root containing `@AGENTS.md` so that Claude Code imports and evaluates the synthesized multi-repo orientation context directly via its native `@include` directive without duplicating file content or relying on fragile OS filesystem symlinks.
- **Custom Template Instructions (`agents_md_append`)**:
  - Optional custom markdown instructions configured per template in `config.toml` via `agents_md_append`.
  - When configured and non-empty after trimming, automatically appended to `AGENTS.md` under a dedicated `## Template Instructions` section.
- **Initial Task / Prompt Seeding**:
  - `botai create <template> <project-name> --attach --prompt "..."` (or `-a -p "..."`):
  - Injects or passes an initial task description directly to the AI agent on launch (requires `--attach` / `-a`) and through TUI creation modals.
  - **Context Synthesis**: In all cases, the initial prompt is written into `AGENTS.md` under a dedicated `## Task Description` section.
  - **Agent CLI Prompt Adapters**: When an initial prompt is present, `botai` automatically adapts and appends the prompt to the spawned agent CLI arguments so it immediately starts working upon session launch:
    - **`agy`**: Appends `["-i", "<prompt>"]` (or `--prompt-interactive`).
    - **`aider`**: Appends `["--message", "<prompt>"]`.
    - **`claude`**: Appends positional argument `["<prompt>"]`.
    - **`copilot`**: Appends positional argument `["<prompt>"]`.
    - **Unknown / Custom Agents**: Does not append prompt arguments to the CLI command; the agent relies exclusively on `AGENTS.md`.
  - **Safe Argv Passing**: The prompt string is passed directly as an element in `PtyConfig.args` without shell escaping or string concatenation, preserving whitespace and newlines.

---

## 5. Agent Monitoring & Notifications
- **Multi-Tiered Agent Status & Attention Detection**:
  - **PTY Stream Protocol & Control Sequence Scanner**:
    - **ASCII Bell (`\x07` / `BEL`) Interception**: Emitting an ASCII bell character triggers an immediate transition to `SessionStatus::WaitingForInput` and emits a notification event without waiting for silence timeouts.
    - **OSC 9 / OSC 777 Notification Decoding**: Decodes terminal notification escape sequences (`\x1b]9;<message>\x07` and `\x1b]777;notify;<title>;<message>\x07`) to extract custom notification messages and titles dispatched directly by AI agents or CLI tools.
    - **OSC 133 Semantic Shell Integration Prompts**: Recognizes standard semantic prompt markers:
      - `\x1b]133;A\x07` (Prompt start / ready for user input): Immediately transitions session to `SessionStatus::WaitingForInput`.
      - `\x1b]133;B\x07` (Command start / user execution): Transitions session to `SessionStatus::Busy`.
      - `\x1b]133;C\x07` (Command output stream): Records active output stream processing.
      - `\x1b]133;D[;<exit_code>]\x07` (Command finished): Evaluates exit status to transition to `SessionStatus::Done` or `SessionStatus::Error`.
    - **Transparent Passthrough**: Control sequence decoding operates non-destructively; raw bytes are passed intact to the virtual terminal (`vt100`) parser.
    - **Silence / Inactivity Timeout Fallback**: Employs an idle threshold (default 1,500 ms) as a robust baseline fallback for legacy tools or raw command output not emitting explicit control sequences.
  - **Per-Session Activity Tracking**:
    - Each project session maintains an isolated, independent `ActivityTracker` instance with its own activity clock and state machine, preventing cross-session timer crosstalk.
- **Background-Aware Desktop Notifications**:
  - Emits native OS desktop notifications (via `notify-rust` / terminal notifications) with per-project debouncing and cooldown:
    - An agent CLI process completes a task or exits.
    - An agent transitions to an idle state awaiting user input / approval, or emits a `BEL` / OSC notification.
  - **Focus-Aware Notification Filtering**: Desktop notifications for waiting/attention events are suppressed if the user is actively viewing and interacting with that specific session, triggering only when the session is running in the background (different project tab, modal dialog, or dashboard open).
- **Real-Time Session Indicators in TUI**:
  - Live status indicators formatted as a leading solid round dot symbol (`●` U+25CF) inside project tabs in the status bar (e.g., `  ● 1 [ecommerce] payment-api  `, falling back to `  ● 1 payment-api  ` if template name is empty, with active/inactive styling):
    - ● **Busy / Thinking**: Themed Busy color (`theme.status_busy`, e.g., Cyan / Electric Blue / Amber) when agent is actively executing.
    - ● **Waiting for Input**: Themed Waiting color (`theme.status_waiting`, e.g., Peach / Amber / Sky Blue) when agent is awaiting user response, confirmation, or attention.
    - ● **Done / Idle**: Themed Done color (`theme.status_done`, e.g., Green) when agent finished its assigned prompt or task.
    - ● **Error / Interrupted**: Themed Error color (`theme.status_error`, e.g., Red) on non-zero exit code or process failure.
    - ● **Initializing / Terminated**: Themed status bar foreground color (`theme.status_bar_fg`).

---

## 6. Session Management & Navigation (TUI Multiplexer)
- **Multi-Project Concurrency**: Maintains concurrent active project sessions in the background.
- **Zero-Allocation VT Cell Rendering & High-Throughput Performance**:
  - `TerminalView::render_to_buffer` renders virtual terminal cells directly from `vt100::Screen` borrowing `&str` symbols and mapping colors without per-cell heap allocations, ensuring high-throughput 60FPS+ TUI rendering.
- **Dynamic Viewport & Responsive Resizing**:
  - Automatically synchronizes PTY dimensions and VT virtual screen buffers to the host terminal viewport on initial startup, on window resize events (`Event::Resize`), and when creating or switching active project sessions.
  - Propagates window size changes (`SIGWINCH` / `ResizePseudoConsole`) so full-screen interactive agent CLIs (`agy`, `claude`, `aider`, etc.) dynamically re-render to the exact available terminal dimensions.
- **Hardware Cursor Positioning & Visibility**:
  - Accurately tracks cursor coordinates and visibility state (`\x1b[?25h` / `\x1b[?25l`) from the active session's VT parser.
  - Positions the terminal hardware cursor directly at the active input position during interactive sessions, rendering the native block cursor where user input is focused when at live scrollback position.
  - Automatically positions cursor in active text fields within dialogs/modals and hides the cursor when viewing scrollback history or when appropriate.
- **Keyboard Enhancement Protocol Negotiation**:
  - Pushes the progressive keyboard enhancement flag (`KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES`, sequence `\x1b[>1u`) via `PushKeyboardEnhancementFlags` on startup. Returning ONLY `DISAMBIGUATE_ESCAPE_CODES` enables unambiguous distinction between modified keys (e.g. distinguishing Shift+Enter from plain Enter, modified arrows, Ctrl+Shift combinations) without requesting alternate key reporting (`REPORT_ALTERNATE_KEYS`). (Note: Including `REPORT_ALTERNATE_KEYS` causes Alacritty and Kitty-protocol terminals to send alternate key codepoints `\x1b[13:10;2u` which crossterm's parser uses to strip the Shift modifier, converting Shift+Enter into a plain unshifted Enter/Newline; `DISAMBIGUATE_ESCAPE_CODES` is the exact flag used by `agy` and Kitty-protocol TUIs to ensure Shift+Enter creates newlines seamlessly.)
  - Accepts both `KeyEventKind::Press` and `KeyEventKind::Repeat` so held keystrokes (such as repeated backspace, arrow navigation, or enter) are processed smoothly across all interactive modals and PTY sessions.
  - Dynamically negotiates host terminal environment for xterm `modifyOtherKeys`:
    - Mode 2 (`\x1b[>4;2m`): Enabled when running inside `tmux` (detected via `TMUX` environment variable) to guarantee modifier reporting through tmux's internal terminal multiplexing layer.
    - None (no escape sequence emitted): For Alacritty 0.13+, WezTerm, Kitty, Ghostty, and standard terminals, relying on the native Kitty Keyboard Protocol (`\x1b[>1u`) rather than sending `modifyOtherKeys` Mode 1 (`\x1b[>4;1m`) which would override Alacritty/WezTerm's Kitty mode and suppress Shift+Enter escape sequences.
  - Ensures `PopKeyboardEnhancementFlags` and disabling `modifyOtherKeys` (`\x1b[>4;0m`) are executed during teardown and in `TerminalCleanupGuard::drop`, guaranteeing the host terminal's original keyboard state is fully restored even on unexpected exit or error.
- **Terminal State Restoration & Panic Hook**:
  - `install_panic_hook` ensures terminal restoration (raw mode disabled, alternate screen exited, mouse capture disabled, keyboard enhancement flags popped, cursor shown) before diagnostics are printed on any unexpected panic.
  - Installed via `std::panic::set_hook` at program startup (`src/main.rs`), wrapping the previous panic hook.
  - Intercepts panics occurring in any thread, immediately invoking `crossterm::terminal::disable_raw_mode()`, resetting tmux `modifyOtherKeys` (`\x1b[>4;0m` if inside a tmux session), executing `PopKeyboardEnhancementFlags`, `DisableMouseCapture`, `LeaveAlternateScreen`, and `crossterm::cursor::Show` before handing control back to the previous hook to format backtraces and panic reports onto a clean host terminal.
- **Full CSI-u / ANSI Modifier Key Passthrough & Stream Demuxing**:
  - Encodes and decodes modified key events using standard Xterm / CSI-u modifier parameter values:
    `modifier_code = 1 + (Shift ? 1 : 0) + (Alt ? 2 : 0) + (Ctrl ? 4 : 0) + (Super/Meta ? 8 : 0)`.
    - Handles progressive Kitty sub-parameters (`\x1b[<codepoint>:<shifted>;<modifiers>:<event_type>u`) and masks out lock keys (Caps Lock `64`, Num Lock `128`) to accurately detect modifiers under all keyboard states.
    - Buffers split escape sequences across chunk boundaries (including single `\x1b` byte splits) in `RawLeaderDecoder` symmetrically across inactive and active leader modes to prevent raw escape leakages.
    - Stream-demuxes mixed input buffers containing interleaved SGR mouse sequences (`\x1b[<...M/m`) and keystrokes, buffering incomplete cross-chunk SGR mouse prefixes without dropping trailing keys or leaking coordinates.
    - **SGR Mouse Prefix Disambiguation**:
      - Incomplete SGR mouse sequence detection (`is_incomplete_sgr_mouse`) strictly requires a `\x1b[<` prefix. Standalone `\x1b` or `\x1b[` bytes are never classified as incomplete SGR mouse sequences and are never trapped in `pending_raw_input`.
    - **Standalone Escape Key Handling & Timeout Flushing (`flush_pending_input`)**:
      - When a standalone `\x1b` byte arrives in raw stdin mode (e.g. from pressing `Esc` in terminals like Terminal.app on macOS), `RawLeaderDecoder` temporarily buffers it in `pending_escape` to disambiguate whether it begins a Kitty protocol `Ctrl+Space` sequence (`\x1b[32;5u`).
      - On stdin timeout in `TuiApp::run_loop_with_receiver` (`stdin_rx.recv_timeout` returns `Err(_)`), `TuiApp::flush_pending_input()` is invoked:
        - Flushes and drains any lingering `pending_raw_input` via `process_non_mouse_bytes`.
        - Invokes `RawLeaderDecoder::flush()`:
          - If leader mode is inactive: Dispatches `RawAction::Passthrough(vec![0x1b])`, writing the Escape byte directly to the active session PTY to provide immediate and reliable `Esc` key response in Vim, nano, shells, and CLI tools.
          - If leader mode is active: Dispatches `RawAction::CancelLeader`, cleanly exiting leader mode and resetting `leader_active` to `false`.
  - **Modified Enter, Shift+Enter, and Newlines**:
    - Unmodified Enter (`KeyCode::Enter` with no modifiers): `\r` (`0x0D`, CR).
    - Unmodified Newline (`KeyCode::Char('\n')` with no modifiers): `\n` (`0x0A`, LF / Ctrl+J).
    - Unmodified Carriage Return (`KeyCode::Char('\r')` with no modifiers): `\r` (`0x0D`, CR).
    - Ctrl+J (`KeyCode::Char('j')` with Ctrl, or unmodified `\n`): `0x0A` (LF).
    - Modified Enter / Newlines (Shift, Alt, Ctrl, Super, etc. for `KeyCode::Enter`, `KeyCode::Char('\n')`, and `KeyCode::Char('\r')`): `\x1b[13;<mod>u` (e.g. `Alt+Enter` -> `\x1b[13;3u`; `Ctrl+Enter` -> `\x1b[13;5u`; `Ctrl+Shift+Enter` -> `\x1b[13;6u`; `Super+Enter` -> `\x1b[13;9u`).
    - **Seamless Agent-Aware Shift+Enter Multiline Routing**:
      - Inspects the active session (`active.project_session.agent`) when receiving raw Shift+Enter sequences (Kitty CSI-u `\x1b[13;<mod>u` with Shift bit, sub-parameters `\x1b[13;2:1u`, alternate codepoints `\x1b[13:10;2u`, or xterm modifyOtherKeys `\x1b[27;<mod>;13~`):
        - For `claude` (or non-Kitty agents): Sends `b"\n"` (`0x0A` / Line Feed) so Claude Code inserts a clean multiline newline instead of submitting.
        - For `agy`: Forwards `b"\x1b[13;2u"` (native Kitty protocol multiline newline sequence).
        - Default/other agents: Sends `b"\n"`.
      - Unmodified Enter (`\r`) is forwarded unchanged as `\r` (`0x0D`, CR) across all sessions.
  - **Modified Tab & BackTab**:
    - Unmodified Tab: `\t` (`0x09`).
    - Shift+Tab / BackTab: `\x1b[Z`.
    - Other modified Tab (Ctrl+Tab, Alt+Tab, etc.): `\x1b[9;<mod>u`.
  - **Modified Backspace**:
    - Unmodified Backspace: `\x7f`.
    - Modified Backspace: `\x1b[127;<mod>u`.
  - **Modified Esc**:
    - Unmodified Esc: `\x1b`.
    - Modified Esc: `\x1b[27;<mod>u`.
  - **Arrow Keys (`Up`, `Down`, `Right`, `Left`)**:
    - Unmodified: `\x1b[A`, `\x1b[B`, `\x1b[C`, `\x1b[D`.
    - Modified: `\x1b[1;<mod>A`, `\x1b[1;<mod>B`, `\x1b[1;<mod>C`, `\x1b[1;<mod>D`.
  - **Navigation Keys (`Home`, `End`)**:
    - Unmodified: `\x1b[H`, `\x1b[F`.
    - Modified: `\x1b[1;<mod>H`, `\x1b[1;<mod>F`.
  - **Paging & Edit Keys (`PageUp`, `PageDown`, `Delete`, `Insert`)**:
    - Unmodified: `\x1b[5~`, `\x1b[6~`, `\x1b[3~`, `\x1b[2~`.
    - Modified: `\x1b[5;<mod>~`, `\x1b[6;<mod>~`, `\x1b[3;<mod>~`, `\x1b[2;<mod>~`.
  - **Function Keys (`F1`..`F12`)**:
    - F1..F4: Unmodified `\x1bOP`..`\x1bOS`; modified `\x1b[1;<mod>P`..`\x1b[1;<mod>S`.
    - F5..F12: Unmodified `\x1b[<num>~`; modified `\x1b[<num>;<mod>~`.
  - **Character Keys (`KeyCode::Char(c)`)**:
    - Single Ctrl: Standard C0 ASCII control byte (`Ctrl+A` -> `0x01` .. `Ctrl+Z` -> `0x1A`, `Ctrl+Space` -> `0x00`).
    - Single Alt: Prepend ESC (`\x1b` + char UTF-8 bytes).
    - Multi-modifier (Ctrl+Shift, Ctrl+Alt, Super): CSI-u format `\x1b[<codepoint>;<mod>u`.
- **Bracketed Paste Event Passthrough**:
  - Formats pasted clipboard text as standard bracketed paste escape sequence (`\x1b[200~<text>\x1b[201~`) via `paste_to_bytes`.
  - When an active session is focused and leader mode is inactive, forwarded directly to the active session's PTY master/slave writer, updating activity tracking and resetting scrollback to live output.
  - When dialogs or modals are active (Project Creation Modal, Dashboard), forwarded to the active text input field or search filter, preserving whitespace and newlines for multiline prompts.
- **Mouse Event Capture & Dual-Mode Scroll Handling**:
  - Enables terminal mouse capture (`crossterm::event::EnableMouseCapture`) on application startup and cleanly releases it (`crossterm::event::DisableMouseCapture`) on termination / cleanup.
  - **Child Process Mouse Protocol (SGR 1006) & Viewport Coordinate Translation**:
    - When an active session process explicitly enables mouse reporting (e.g. `\x1b[?1000h` / `\x1b[?1006h` in `htop`, `vim`, `nano`, `mc`, `claude`), mouse clicks, drags, releases, and scroll events are formatted as standard SGR escape sequences (`\x1b[<{code};{col};{row}{M/m}`) and forwarded to the child PTY.
    - **Status Bar Click Rejection**: Host mouse events targeting the top status bar (`mouse.row < STATUS_BAR_HEIGHT`, where `STATUS_BAR_HEIGHT = 1`) are rejected and dropped, preventing unintentional clicks from reaching the child process.
    - **Viewport Coordinate Translation**: Host terminal mouse row coordinates `mouse.row >= STATUS_BAR_HEIGHT` are translated into child viewport space via `mouse.row - STATUS_BAR_HEIGHT` (e.g., host row 1 translates to child row 0, generating SGR row 1 `\x1b[<...;...;1M`, and host row N translates to child row N-1, generating SGR row N). This eliminates off-by-one row selection errors when selecting text or clicking UI elements in child processes.
    - **Viewport Bounds Enforcement**: Events exceeding the active child VT height (`child_row >= vt.rows()`) are bounded and dropped.
  - **Native Virtual Terminal Scrollback**: When mouse reporting is not enabled by the child process (such as in standard CLI prompts like `agy`, `claude`, `aider`, and shells), mouse scroll wheel events (`ScrollUp`, `ScrollDown`) navigate the `VtScreen` scrollback buffer directly (3 lines per scroll notch), allowing users to view older output history without erroneously transmitting Up/Down arrow history navigation keys.
  - **Automatic Live Snapping**: Any keyboard input or paste event automatically resets scrollback offset to `0`, instantly returning the viewport to live output.
  - **Modal & Dashboard Mouse Scrolling**: When floating modals (such as the Asynchronous Setup & Log Modal) or the interactive Dashboard are active, mouse wheel scroll events scroll the active log view or list view appropriately without leaking scroll bytes to background sessions.
- **Session Termination & Exit Transitions**:
  - **Multi-Session Scenario (Normal Exit)**: When an agent process exits cleanly (exit code 0) and other session tabs remain active, the finished tab is automatically closed and focus transitions immediately to the adjacent active session tab.
  - **Last / Only Session Scenario (Context-Aware Exit)**:
    - If `botai` was launched from the dashboard, terminating the last session automatically transitions back to the interactive project dashboard (with refreshed project directories).
    - If `botai` was launched directly via CLI (`botai create --attach` or `botai attach`), terminating the last session exits `botai` cleanly back to the host shell.
  - **Error & Crash Retention**: If an agent process exits with an error or crash (non-zero exit code or abnormal termination), the session remains visible in the registry with the Error status (`●` in `status_error` color) and an inline prompt banner (e.g. `[Process exited with code X - Press Enter / Ctrl+Space w to dismiss]`), ensuring error outputs and logs are not lost until explicitly dismissed.
- **Child Process Lifecycle Management & Process Reaping (`PtySession`)**:
  - `PtySession` implements `Drop` to kill and reap child processes automatically when dropped (or when an active session, `SessionEntry`, or `SessionRegistry` is removed or dropped), preventing orphaned or zombie processes in the operating system process table.
  - `PtySession::kill()` explicitly terminates running processes and invokes `child.wait()` to reap process entries from the OS process table.
  - `PtySession::process_id()` exposes the underlying OS child process identifier (`Option<u32>`) when available.
- **Session Detachment & Background Persistence**:
  - Detaching from an active session (`Ctrl + Space` then `x`):
    - When launched from the dashboard, returns to the interactive project dashboard with refreshed project listings while leaving all background sessions and agent processes running. Users can re-attach to existing running sessions, switch between active sessions, or create/destroy other projects.
    - When launched directly from CLI (`botai create --attach` or `botai attach` without dashboard), detaching exits `botai` cleanly back to the host shell while leaving background sessions intact.
- **Status Bar & Project Indicators**: An unobtrusive top/bottom status bar displaying active sessions, status badges, and streamlined key hints. Each session tab displays its index hotkey, template name in brackets, and project name: `● <Hotkey> [<Template>] <Project>` (or `● <Hotkey> <Project>` if template name is empty). In normal state, it displays a concise right-aligned `'Ctrl+Space: Actions'` hint (omitted gracefully on narrow viewports). When the leader key is triggered (`leader_active`), it displays the focused actionable commands menu `'[1..9 Switch | c New | n/p Nav | w Close | x Detach | d Destroy | q Quit]'` in high-contrast leader styling. When no active sessions exist, it displays `'[No Active Sessions] - Press Ctrl+Space then q to quit'`, transitioning to the active leader menu when `Ctrl+Space` is pressed.
- **Leader Key Architecture (`Ctrl + Space`)**:
  - Global multiplexer actions are invoked via the leader prefix `Ctrl + Space`, followed by a command key:
    - `Ctrl + Space` then `1` .. `9`: Directly switch to project session 1 to 9.
    - `Ctrl + Space` then `c`: Open the unified project creation modal (`ProjectCreationModal`).
    - `Ctrl + Space` then `n` / `p`: Switch to Next / Previous active project session.
    - `Ctrl + Space` then `w`: Close / terminate active project session.
    - `Ctrl + Space` then `x`: Detach from active session (returns to dashboard when dashboard is present, or exits to host shell when running in standalone CLI mode).
    - `Ctrl + Space` then `d`: Destroy active project session workspace (prompts confirmation dialog and triggers clean worktree destruction and directory removal).
    - `Ctrl + Space` then `q`: Quit `botai`.
    - `Ctrl + Space` then `Ctrl + Space` (or `Space`): Forward a literal `Ctrl + Space` (`0x00`) to the active PTY.
    - `Ctrl + Space` then `Esc` (or any unrecognized key): Cancel leader mode without executing an action.

---

## 7. CLI Commands & Lifecycle Operations
- **Default (No Arguments)**: Launching `botai` without arguments opens the interactive TUI project switcher / dashboard.
- **`botai create <template> <project> [--agent <agent> / -A <agent>] [--prompt <prompt> / -p <prompt>] [--include <repos>] [--exclude <repos>] [--all] [--attach / -a]`**:
  - Creates a new multi-repo workspace project under `CWD/<project>` from the specified template.
  - **Arguments & Flags**:
    - `template`: (Positional required String) Name of the project template defined in `config.toml`.
    - `project`: (Positional required String) Name of the project directory and git branch to create.
    - `--agent` / `-A`: (Optional String) Override the AI agent CLI tool (e.g. `claude`, `agy`, `copilot`, `aider`).
    - `--prompt` / `-p`: (Optional String) Initial task description or prompt to pass to the agent (requires `--attach` / `-a`).
    - `--include <repos>`: (Optional String, comma-separated or repeatable) Explicitly select repositories to include in the project, opting into repositories even if declared with `default = false` in the template configuration.
    - `--exclude <repos>`: (Optional String, comma-separated or repeatable) Explicitly deselect repositories to exclude from the project, opting out of repositories even if declared with `default = true` in the template configuration.
    - `--all`: (Optional Flag) Include all repositories configured in the template, overriding any `default = false` settings.
    - `--attach` / `-a`: (Flag) Launch interactive TUI multiplexer immediately after project creation.
  - **Repository Selection Precedence & Conflict Validation (`resolve_cli_selected_repos`)**:
    - Evaluated upfront via `resolve_cli_selected_repos(&all_template_repos, include, exclude, all)` before initializing `ProjectBuilder` or launching TUI sessions.
    - **Conflict Validation**: Specifying the same repository in both `--include` and `--exclude` is rejected as an invalid conflicting argument (`Cannot both include and exclude repository '<repo>'`).
    - **Repository Existence Validation**: Any repository specified in `--include` or `--exclude` must match a configured repository in the template (by directory name or entry string); unrecognized names fail immediately with an error listing available template repositories (`Repository '<repo>' not found in template. Available repositories: ...`).
    - **Evaluation Precedence**:
      1. Baseline: If `--all` is specified, all template repositories are initially included. Otherwise, only repositories marked with `default = true` (or simple strings) are initially included.
      2. Inclusions: Any repositories specified via `--include` are added to the active selection (deduplicating if already included).
      3. Exclusions: Any repositories specified via `--exclude` are removed from the active selection.
    - **Zero-Repository Guard**: If applying exclusions results in zero selected repositories, creation fails with an immediate error: at least one repository must be selected (`Cannot create project: at least one repository must be selected.`).
  - **Non-Attached Mode (Default, without `--attach`)**:
    - Executes workspace initialization steps 1-4 via `ProjectBuilder::create_workspace`.
    - Validates inputs, auto-clones missing repositories to `repos_dir`, fetches upstream branches, provisions git worktrees on branch `<project>`, and synthesizes `AGENTS.md`.
    - Does NOT spawn any agent PTY process or enter raw terminal mode.
    - Streams progress logs to stdout in the host shell and outputs:
      `✓ Project '<project>' created successfully at <path>!`
      `Hint: Run 'botai attach <project>' to launch the agent session or run 'botai' to open the interactive dashboard.`
    - Exits cleanly with code 0 (or non-zero on error with atomic rollback).
  - **Attached Mode (with `--attach` / `-a`)**:
    - Executes workspace initialization steps 1-4.
    - Spawns the AI agent CLI inside a pseudo-terminal (`PtySession`) in step 5.
    - Switches host terminal to raw mode and launches the interactive TUI multiplexer attached directly to the newly created project session.
- **`botai sync [template]`**:
  - Pre-clones missing repositories and updates existing cached repositories across all or a specific template.
  - Clones missing repositories to `repos_dir` if needed.
- **`botai attach <project-name>`**: Re-attaches to an ongoing project session.
- **`botai list`**: Lists configured templates, repositories with clone/cached status, active projects, worktrees, and running sessions.
- **`botai init [--force] [--stdout]`**:
  - Generates an annotated configuration template serving as both documentation and an editable starter configuration.
  - Defaults to writing to standard platform configuration path (`~/.config/botai/config.toml` or `%APPDATA%\botai\config.toml`), or the custom file passed via global `--config <FILE>`.
  - Automatically creates parent directories if they do not exist.
  - Refuses to overwrite an existing configuration file unless `--force` (`-f`) is passed.
  - `--stdout` (`-s`): Outputs the configuration template directly to `stdout` without creating or modifying files on disk.
- **`botai edit [--editor <cmd> / -e <cmd>]`**:
  - Opens the `botai` configuration file in the user's text editor of choice.
  - **Configuration Path Resolution**:
    - Uses the path specified via global `--config <FILE>` if provided, or the standard platform default (`~/.config/botai/config.toml` or `%APPDATA%\botai\config.toml`).
    - If the configuration file does not exist at the target path, automatically initializes the file with the full documented default template (creating parent directories if necessary) before opening the editor.
  - **Editor Resolution Order & Precedence**:
    1. CLI `--editor <cmd>` / `-e <cmd>` argument.
    2. `$VISUAL` environment variable if set and non-empty.
    3. `$EDITOR` environment variable if set and non-empty.
    4. Platform fallbacks:
       - Unix / macOS: Checks `$PATH` sequentially for `nano`, `vim`, then `vi`.
       - Windows: `notepad.exe`.
    5. If no editor can be resolved or found in `$PATH`, terminates with an informative error message and non-zero exit code.
  - **Command & Argument Parsing**:
    - Multi-word editor variables (e.g. `EDITOR="code --wait"` or `VISUAL="subl -w"`) are split into executable binary and arguments according to shell word splitting rules, with the configuration file path appended as the final argument.
  - **Execution & Lifecycle**:
    - Spawns the editor process with inherited standard I/O (stdin, stdout, stderr) and waits synchronously for the process to exit.
    - Exits `botai` with the editor process's exit status code (silent pass-through without blocking on syntax validation).
- **`botai destroy <project-name> [--force]`**:
  - Validates all child worktrees for uncommitted changes upfront and refuses deletion unless `--force` is specified.
  - Executes configured template `pre_destroy` lifecycle hooks sequentially at the project root directory before unlinking worktrees, capturing command output and logging warnings on failures.
  - Automatically resolves the underlying source repository in `repos_dir` (or local path) for every child worktree via `resolve_main_repo`.
  - Executes `git worktree remove` and `git worktree prune` directly against the backing central cached repositories via `remove_worktree_auto`, thoroughly clearing worktree metadata, `.git/worktrees/<name>` directories, and internal git registrations.
  - Removes the project directory and its worktrees from disk.
  - Preserves git branches (leaving code intact in git history) and enables immediate re-creation without stale metadata collisions.
  - Explicitly terminates running sessions for the project and reaps child processes before worktree removal and directory cleanup, avoiding process and file lock contention across Unix and Windows platforms.
  - Streams real-time structured destruction events (`ProjectDestroyEvent`: Step, Log, Success, Error) to CLI stdout or the TUI destruction modal, ensuring progress visibility without blocking.

---

## 8. Unified Project Creation Engine & Interactive TUI Creation Modal
- **Unified Backend Engine (`ProjectBuilder`)**:
  - Single, centralized project initialization pipeline in `src/project/` used across CLI `botai create` (both non-attached and attached modes), the TUI Dashboard, and In-Session project creation.
  - **Sequential Pipeline Steps**:
    1. **Pre-Validation**: Multi-tier validation with `validate_fast` (immediate 0ms local validation of project name, template existence, empty selected repos, and CWD directory collision) and `check_branch_collisions` / `validate_detailed` (effective repository resolution and git branch collision verification strictly across effective repositories).
    2. **Repository Resolution & Cloning**: Resolves effective repositories (`resolve_effective_repos`), clones missing remote Git repositories into `repos_dir` and performs best-effort upstream fetch only for effective repositories.
    3. **Worktree Checkout**: Creates a git worktree checked out to branch `<project_name>` at `<project_dir>/<repo_name>` strictly for effective repositories.
    4. **Per-Repository Setup Hooks Execution**: Concurrently executes configured repository `setup` commands in parallel across active effective worktrees (sequential command execution per worktree), capturing logs and proceeding under the Warn and Continue policy upon failures.
    5. **Context Synthesis**: Synthesizes multi-repo context and template instructions into `<project_dir>/AGENTS.md` documenting only effective repositories in the Repository Layout table.
    6. **Workspace Post-Creation Hooks Execution**: Runs configured template `post_create` commands sequentially at the project root directory.
    7. **PTY Process Spawning**: Spawns the agent CLI process inside a master/slave pseudo-terminal attached to a virtual terminal screen buffer.
  - **Modular Execution**:
    - `resolve_cli_selected_repos`: Standalone helper resolving and validating `--include`, `--exclude`, and `--all` CLI flags against template repositories.
    - `ProjectBuilder::with_selected_repos`: Builder method configuring explicit repository selection subsets.
    - `ProjectBuilder::resolve_effective_repos`: Resolves effective repositories based on explicit selection or `default_include` flags, returning `ProjectError::NoRepositoriesSelected` if empty or `ProjectError::RepositoryNotInTemplate` if a selected repository is not declared in the template.
    - `ProjectBuilder::create_workspace`: Executes steps 1-6 without spawning PTY, returning the canonical project path and repository mappings for effective repositories.
    - `ProjectBuilder::build`: Executes steps 1-6 via `create_workspace` and proceeds with step 7 PTY spawning for attached session workflows.
  - **Event & Progress Streaming**: Emits structured `ProjectSetupEvent` (Steps, Logs, Success, Error) consumed by CLI stdout or the TUI setup modal.
  - **Atomic Rollback**: On any core initialization error (e.g. branch collision, clone failure, worktree creation failure, agent spawn failure), cleanly removes any partially created worktrees using `remove_worktree_auto` in backing central repositories and deletes the project directory. (Note: Non-zero exit codes from user setup/post-create hooks log warnings and do not trigger destructive rollback).
- **Unified Interactive Project Creation Modal (Two-Step Flow)**:
  - Single shared modal component (`ProjectCreationModal`) utilized in both Dashboard and In-Session creation.
  - **Dynamic Configuration & Template Reloading on Modal Activation**:
    - Opening the creation modal (via `n`/`c` on the dashboard or `RawAction::OpenLauncher` within an active session) triggers configuration reloading from disk using filesystem modification time (`mtime`) and file size (`len`) caching.
    - While `Config` remains a pure data structure, `TuiApp` and `DashboardState` track `config_path`, `config_last_modified` (`mtime`), and `config_file_len` (`len`), checking both filesystem modification time and file size to ensure robust cross-platform change detection even within coarse timestamp intervals (e.g. Windows NTFS ~15ms granularity). Unnecessary reloads are skipped when both attributes remain unchanged, and newly configured templates or modified repository references are immediately reflected in Step 1 template selection when the file has been modified.
  - **Modal Geometry & Responsive Viewport Sizing**:
    - Standard modal dimensions defined by constants `MODAL_WIDTH = 76` and `MODAL_HEIGHT = 24`.
    - Computed via helper `modal_area(area: Rect) -> Option<Rect>`:
      - Width: `MODAL_WIDTH.min(area.width.saturating_sub(4))`
      - Height: `MODAL_HEIGHT.min(area.height.saturating_sub(2))`
      - Graceful minimum dimensions guard: Returns `None` if `width < 25` or `height < 10`.
      - Horizontally and vertically centered in the parent terminal viewport.
  - **Unified 5-Part Layout Architecture**:
    - Both Step 1 and Step 2 adhere to an identical 5-part vertical layout structure (`[Rect; 5]`):
      1. **Top Area (`Constraint::Length(1)` or `Constraint::Length(top_len)`)**:
         - *Step 1 (`step1_layout`)*: Search filter input row (`Filter: <query>`).
         - *Step 2 (`step2_layout`)*: Template summary row (`Template: <name> | Repos (<count>): <repos>`) and optional description row (`top_len = 2` if description is present and `inner.height >= 16`, otherwise `1`).
      2. **Top Divider (`Constraint::Length(1)`)**: Full-width horizontal separator line (`─`) styled in `theme.border_unfocused`.
      3. **Body Area (`Constraint::Min(4)`)**:
         - *Step 1*: Master-detail horizontal split (left matching templates list, right template details preview).
         - *Step 2*: Form area rendering Project Name, Agent CLI selector, Repositories checklist, and Initial Prompt box, cleanly ending after the prompt input.
      4. **Bottom Divider (`Constraint::Length(1)`)**:
         - *Step 1*: Full-width horizontal separator line (`─`) styled in `theme.border_unfocused`.
         - *Step 2*: Horizontal divider embedding the live pre-validation status message (`── ✗ <message> ────`, `── ℹ <message> ────`, or `── ✓ Ready to create project ────`), keeping the form body clean and anchoring validation feedback immediately above the bottom shortcuts row.
      5. **Footer Shortcuts Hint Bar (`Constraint::Length(1)`)**: Bottom-anchored key navigation hints (`theme.key_hint`, bold) consistently anchored to the base of the modal across both steps.
  - **Asynchronous Non-Blocking Pre-Validation**:
    - Keystrokes run `validate_fast()` immediately (0ms) so basic errors and warnings show without UI latency.
    - Git branch collisions are checked asynchronously in background worker threads with generation counters to prevent stale results and UI freezing.
    - Non-blocking `poll_validation()` drains validation worker responses on TUI loop ticks.
  - **Two-Step Dialog Progression**:
    1. **Step 1: Searchable Template Selection (`CreationStep::SelectTemplate`)**:
       - **Top Filter Search Bar**: Real-time search filter powered by the reusable `TextInput` component (`Filter: <query>`), supporting in-line character cursor positioning, full readline editing keybindings (`Home`/`Ctrl+A`, `End`/`Ctrl+E`, `Left`/`Right`, `Ctrl+Left`/`Alt+B`, `Ctrl+Right`/`Alt+F`, `Backspace`, `Delete`, `Ctrl+U`, `Ctrl+K`, `Ctrl+W`), paste sanitization, and hardware terminal cursor synchronization.
       - **Matching Criteria**: Strictly case-insensitive substring matching against template names only (descriptions and constituent repository names/sources are not matched).
       - **Selection Synchronization Rules**:
         - *Query Changes (typing, backspacing, pasting)*: If the currently selected template is still among the matching results, preserve its selection; otherwise reset selection to index 0 (top hit).
         - *On Enter in Step 1*: Transitions to Step 2 (`ConfigureDetails`) cleanly with the chosen template without double-indexing into matching indices.
         - *On Esc in Step 2*: Backtracks to Step 1 preserving the filter query and highlighting the active template in the matching list.
       - **Master-Detail Split Layout**:
         - *Left Panel*: Filtered template list with active selection indicator (` > `) and matching template count.
          - *Right Preview Pane*: Live preview of the highlighted template:
            - Description, repository count, default agent CLI, and custom instructions.
            - **Default vs Optional Repositories Preview**: Lists constituent repositories indicating inclusion status: default repositories are rendered normally (e.g. `• <name>`), while optional opt-in repositories (`default = false`) are explicitly badged with `[optional]`.
        - **Controls**: `Up`/`Down` and `Ctrl+P`/`Ctrl+N` (navigate template list), text typing and readline shortcuts (edit search query), `Enter` selects template and advances to Step 2, `Esc` cancels/closes modal.
        - **Consistent Template Selection & Preview (No Single-Template Bypass)**: Regardless of template count (even when only one template is configured), the creation modal always begins in Step 1 (`CreationStep::SelectTemplate`) to ensure uniform visibility of template descriptions, repository sources, and configuration. When only a single template exists, it is automatically highlighted and pre-selected so the user can immediately review its preview and press `Enter` to advance.
        - **Empty-State Presentation**: If zero templates are configured in `config.toml`, Step 1 renders a helpful empty-state preview card: `"No templates found in config.toml. Run 'botai edit' to configure templates."` with `Esc` to cancel.
    2. **Step 2: Project Details & Configuration (`CreationStep::ConfigureDetails`)**:
       - **Modal Header**: Displays step progress and selected template name (`Create New Project (Template: <name>)`).
        - **Form Fields**:
          - `Project Name:` Editable one-line text input powered by the reusable `TextInput` widget component with:
            - **In-Line Cursor Navigation**: `Left` / `Right` arrows exclusively move the cursor character-by-character within the text without transitioning field focus.
            - **Standard Readline Keybindings**:
              - `Ctrl+A` / `Home`: Move cursor to the beginning of the project name.
              - `Ctrl+E` / `End`: Move cursor to the end of the project name.
              - `Backspace`: Delete character before the cursor.
              - `Delete`: Delete character under the cursor.
              - `Ctrl+U`: Delete from cursor to the beginning of the line.
              - `Ctrl+K`: Delete from cursor to the end of the line.
              - `Ctrl+W`: Delete the word immediately preceding the cursor.
              - `Ctrl+Left` / `Alt+B`: Move cursor backward one word.
              - `Ctrl+Right` / `Alt+F`: Move cursor forward one word.
            - **Field Navigation Isolation**: Focus switching is handled exclusively by `Tab` / `Shift+Tab` or `Up` / `Down` arrows.
            - **Paste Handling**: Sanitizes pasted text by stripping newlines and inserting content at the current cursor position.
            - **Accurate Terminal Hardware Cursor**: Positions the terminal cursor at the exact column offset matching the current cursor position.
            - **Real-Time Pre-Validation Badges**: Dynamically displays `[✓ Ready]`, `[✗ Collision]`, or `[Required]`.
          - `Agent CLI:` Interactive tool selector (`claude`, `agy`, `copilot`, `aider`, etc. cycled via `Left`/`Right`).
          - `Repositories:` Interactive repository selection checklist:
            - **Header & Counter**: Displays `Repositories (<selected_count>/<total_count> selected):` with dynamic scroll indicator badges (`▲` if scrolled down, `▼` if more items exist below).
            - **Item Presentation**: Each repository item displays an active pointer indicator (`▶ ` for the highlighted item, `  ` otherwise), checkbox state (`[x]` for included/selected, `[ ]` for excluded/unselected), repository name (bold when selected, dimmed when unselected), and an ` (optional)` tag for repositories configured with `default = false`.
            - **Initial State**: Automatically pre-populated when selecting or changing templates based on template repository configurations: repositories with `default = true` (or string syntax) are pre-checked `[x]`, and repositories with `default = false` are pre-unchecked `[ ]`.
            - **Navigation & Focus Cycle**: Integrated into the Step 2 field focus cycle (`Project Name` -> `Agent CLI` -> `Repositories` -> `Initial Prompt`), navigated using `Tab` / `Shift+Tab`. When `Repositories` is focused:
              - `Up` / `Down` arrows or `k` / `j` move the highlighted item up and down through the checklist.
              - Navigating `Up` / `k` past the top repository transitions focus to `Agent CLI`.
              - Navigating `Down` / `j` past the bottom repository transitions focus to `Initial Prompt`.
              - Mouse wheel scrolling up and down inside the repository list scrolls the checklist item selection.
            - **Selection Controls**:
              - `Space`: Toggles the inclusion state of the currently highlighted repository.
              - `a` / `A`: Bulk toggles all repositories (deselects all if all are currently selected; otherwise selects all).
            - **Scrollable Viewport**: Constrained to a visible height of up to 3 items (`REPOS_VISIBLE_ROWS = 3`). When repository count exceeds 3, the viewport scrolls smoothly to keep the active item visible, accompanied by `▲` / `▼` indicators.
            - **Real-Time Pre-Validation**: Continuously validates repository selection:
              - Fast synchronous validation: If zero repositories are selected, immediately marks the modal invalid with `"At least one repository must be selected"` and blocks form submission (`Enter`).
              - Asynchronous branch collision validation: Passes only currently selected repositories (`with_selected_repos(selected_repos)`) to `ProjectBuilder::pre_validate()`, ensuring that branch collisions in unselected/deselected repositories do not block project creation.
          - `Initial Prompt:` Optional task instruction prompt rendered in a dedicated multi-line box (3–4 visible lines) with:
            - **Word Wrapping & Sizing**: Automatic line wrapping according to available modal width, preserving line breaks.
            - **Internal Vertical Scrolling**: Automatic vertical scrolling keeping the active line and cursor in view when text exceeds visible lines.
            - **In-Line Cursor Navigation**: Full cursor positioning (`Left`/`Right` character movement, `Home`/`End` to line/prompt boundaries, `Backspace`/`Delete` at cursor position).
            - **Vertical Navigation**: `Up`/`Down` arrows move the cursor vertically across lines within the prompt (with top-line `Up` / bottom-line `Down` transitioning focus between form fields).
            - **Newline Insertion**: `Shift+Enter` or `Alt+Enter` inserts a newline (`\n`) at the cursor position without triggering submission.
            - **Bracketed / Multi-line Paste**: Pasting multi-line content preserves newlines cleanly without premature form submission.
            - **Accurate Terminal Cursor**: Positions the hardware terminal cursor at the corresponding wrapped line and column offset.
          - **Embedded Validation Status in Bottom Divider**: Validation feedback is embedded directly into the bottom horizontal divider line (`── ✗ <message> ────`, `── ℹ <message> ────`, or `── ✓ Ready to create project ────`) immediately above the footer hints row. The message is truncated with an ellipsis (`...`) if the available divider width is constrained, keeping the form body uncluttered and cleanly separated from the footer shortcuts.
        - **Back-Navigation**: Pressing `Esc` in Step 2 returns to Step 1 without losing configuration. Pressing `Backspace` inside editable text fields strictly performs character deletion (no-op when empty or at cursor offset 0) and never navigates away from the step.
        - **Submission**: Pressing `Enter` validates inputs and transitions into the **Asynchronous Live Setup Progress & Log Modal**, spawning a background worker driving `ProjectBuilder` configured with the selected template, project name, agent tool override, explicit list of selected repositories (`with_selected_repos(selected_repos)`), and optional initial prompt. The selected agent override and repository list are plumbed through `DashboardAction::CreateProject` and `CreationModalAction::Submit` directly into `start_project_creation` and `run_setup_worker` to initialize the project with exactly the chosen repositories.

---

## 9. Interactive TUI Project Picker & Launcher Dashboard
- **Top Header Status Bar**:
  - Full-width top header bar (`Constraint::Length(1)`) matching application header styling.
  - Filled with `theme.status_bar_bg` and `theme.status_bar_fg`.
  - Displays brand title ` botai ` on the left styled with `theme.modal_title` and bold formatting.
  - Displays right-aligned key navigation hints styled with `theme.key_hint`:
    - Full hints: `"↑/↓: Navigate | Enter: Attach | n/c: New Project | d: Destroy | q: Quit"`
    - Compact fallback: `"Enter: Attach | n: New | d: Destroy | q: Quit"`
- **Full-Width Local Projects Dashboard**:
  - Clean, full-width single-panel interface listing all local project directories found in CWD (directories containing `AGENTS.md`).
  - **Live Session Indicators**: Running project sessions are distinguished with a leading colored status dot indicator (`●` U+25CF) reflecting live states (`Busy`, `Waiting`, `Done`, `Error`), while inactive/stopped projects display subtle spacing or inactive badges (`○` / `[Inactive]`).
  - **Friendly Empty State**: When no local projects exist in CWD, renders a clean, centered empty-state card prompt: `"No local projects found in CWD. Press [n] or [c] to create a new project."`
- **Creation Shortcut (`n` / `c`)**:
  - Pressing `n` or `c` from anywhere on the dashboard opens the unified `ProjectCreationModal` where templates are selected, previewed, and configured.
- **Project Selection & Actions**:
  - `Up` / `Down` / `j` / `k` (or mouse wheel scroll): Navigate between local projects in the list.
  - `Enter`: Attach/resume focus on the selected project session.
  - `d`: Open the confirmation modal (` Confirm Destroy Project `) for clean worktree removal and directory deletion.
- **Asynchronous Live Project Destruction Progress & Log Modal (`DestroyProgressModalState`)**:
  - **Non-Blocking Background Worker**: Upon confirming destruction with `y` or `Enter`, `botai` does NOT block or freeze the TUI event loop; it launches the destruction pipeline inside a dedicated background worker thread (`std::thread::spawn`).
  - **Pre-Destruction Session Termination**: Before launching the background destruction worker thread, any active or background session for the project is removed from the `SessionRegistry` and its child process is explicitly terminated and reaped (`entry.pty.kill()`), preventing orphaned processes and file lock contention during worktree removal and directory deletion.
  - **Structured Progress Events**: Emits sequential `ProjectDestroyEvent` events (`Step`, `Log`, `Success`, `Error`) over an `mpsc::channel` to drive the modal UI.
  - **Destruction Lifecycle Steps**:
    1. `Step [1/4]`: Executing template `pre_destroy` lifecycle hooks (streaming hook command output).
    2. `Step [2/4]`: Validating worktrees and checking uncommitted changes (enforcing safety unless forced).
    3. `Step [3/4]`: Removing git worktrees and pruning metadata across backing central repositories via `remove_worktree_auto`.
    4. `Step [4/4]`: Recursively deleting the project workspace directory.
  - **Rich Modal Presentation**:
    - Centered floating dialog styled with active theme colors.
    - Title bar: ` Destroying Project: <name> ` (with dynamic status colors: busy/yellow while in progress, green on success, red on error).
    - Header: Displays active step counter and status message with an animated spinner (`⠋`, `⠙`, `⠹`, etc.).
    - Scrollable Log Box (` Destruction Logs `): Retains all timestamped/streamed log lines with full mouse wheel and keyboard scrolling (`Up`, `Down`, `PageUp`, `PageDown`).
    - Footer Hints: Informs user of running status, or prompts `[Enter] / [Esc] Return to dashboard` upon completion.
  - **Completion & Dismissal**:
    - Stays open upon completion so the user can inspect final status and logs at their own pace.
    - Dismissing via `Enter`, `Esc`, `q`, or `Space` closes the modal and automatically refreshes the dashboard's project list.
  - `q` / `Esc`: Quit `botai`.

---

## 10. In-Session Project Creation & Navigation
- **In-Session Project Creation (`Ctrl + Space` then `c`)**:
  - Can be invoked at any time while working inside an active terminal session.
  - Opens the unified two-step `ProjectCreationModal` floating dialog directly (identical workflow and presentation as the overview dashboard):
    - **Step 1 (`CreationStep::SelectTemplate`)**: Searchable template list with real-time text input filter, matching count, and full template preview pane. Defaults to the global default template from `config.toml`. `Enter` advances to Step 2, `Esc` dismisses the modal and returns focus to the active session.
    - **Step 2 (`CreationStep::ConfigureDetails`)**: Editable project name with readline editing and real-time pre-validation badges, interactive Agent CLI selector, and multiline word-wrapped initial prompt box. `Enter` validates inputs, dismisses the modal, and transitions directly into the Asynchronous Live Setup Modal. `Esc` returns to Step 1.
- **In-Session Project Destruction (`Ctrl + Space` then `d`)**:
  - Can be invoked while inside an active terminal session to destroy the current project immediately without first having to detach or close it.
  - Prompts the user with a confirmation dialog: `Confirm Destroy Project: <name>` (`[y] / [Enter] Confirm | [n] / [Esc] Cancel`).
  - Upon confirmation (`y` / `Enter`), immediately terminates and reaps the active session's child PTY process, cleans up worktrees across backing central repositories using `remove_worktree_auto`, deletes the project workspace directory, and streams progress in `DestroyProgressModalState`.
  - When destruction completes, dismissing the modal smoothly shifts focus to an adjacent active project session if other sessions exist, or transitions back to the dashboard if no active sessions remain.
- **In-Session Project Navigation & Detachment**:
  - Direct session switching: `Ctrl + Space` followed by `1`..`9` jumps directly to session index 0..8.
  - Relative session switching: `Ctrl + Space` followed by `n` (next session) or `p` (previous session) cycles through active sessions.
  - Overview detachment: `Ctrl + Space` followed by `x` detaches from the active session back to the interactive project dashboard (overview), allowing full-screen browsing and attaching of local existing projects.


---

## 11. UI Theming & Color Schemes
- **Declarative Theme Configuration**:
  - Configured globally in `config.toml` via the `theme` setting (e.g. `theme = "tokyo-night"`).
  - Defaults to `"tokyo-night"` if omitted or unrecognized.
  - Theme names are normalized case-insensitively and support aliases (e.g., `"catppuccin"` / `"catppuccin-mocha"`, `"gruvbox"` / `"gruvbox-dark"`, `"monochrome"` / `"slate"` / `"minimal"`, `"ansi"` / `"default"` / `"classic"`).
- **Supported Built-in Palettes**:
  - **`tokyo-night`** (Default): Modern deep slate/navy chrome (`#1a1b26`), electric cyan & blue tab highlights (`#7aa2f7`), vibrant purple borders (`#bb9af7`), and warm amber gold leader mode (`#e0af68`).
  - **`catppuccin` / `catppuccin-mocha`**: Soothing dark pastel palette (`#1e1e2e`), mauve borders (`#cba6f7`), sky blue selections (`#89dceb`), peach input highlights (`#fab387`), and crust backgrounds.
  - **`cyberpunk` / `synthwave`**: High-contrast dark cyber aesthetic (`#0a0a0f`), hot neon magenta (`#ff007f`), electric neon cyan (`#00f0ff`), and vivid acid yellow leader indicator (`#ffe600`).
  - **`gruvbox` / `gruvbox-dark`**: Warm retro earthy palette (`#282828`), rust orange active tabs (`#fe8019`), warm cream text (`#ebdbb2`), forest green selections (`#b8bb26`), and bright yellow leader mode (`#fabd2f`).
  - **`monochrome` / `slate`**: Minimalist zinc/slate dark tones (`#18181b` / `#27272a`), ice blue accents, crisp white active text and borders (`#f4f4f5`).
  - **`ansi` / `default`**: Standard 16-color ANSI terminal fallback palette using native terminal cyan, yellow, green, red, dark gray, and white without 24-bit TrueColor RGB definitions.
- **Complete TUI Element Coverage**:
  - **Status Bar**: Background, default text, active session tab, inactive tabs, tab index numbers (1..9), leader mode indicator badge and hints.
  - **Agent Status Indicators**: Distinct contextual theme colors applied to the leading round status dot (`●`) for Thinking / Busy (`status_busy`), Waiting for Input (`status_waiting`), Done / Idle (`status_done`), and Error / Interrupted (`status_error`), with Initializing and Terminated states rendered in `status_bar_fg`.
  - **Dashboard**: Panel borders (focused vs unfocused), list item selections, selection pointers (`>`), panel titles, and top header status bar key hints.
  - **Modals (Creation Modal, Setup Progress Modal & Destroy Modal)**: Dialog borders, focused input borders/prompts, placeholder text, field labels, and button/key hints.
  - **Exit & Crash Banners**: High-visibility error notification banners rendered across the bottom line upon non-zero process exits.

---

## 12. Native Compilation & Local Installation
- **Host Platform Support**:
  - **Linux**: `linux-x86_64`, `linux-aarch64`
  - **Windows**: `windows-x86_64`, `windows-aarch64`
  - **macOS**: `macos-x86_64` (Intel), `macos-aarch64` (Apple Silicon)
- **Native Host Compilation & Local Installation**:
  - Native compilation using standard Cargo (`cargo build --release`, `cargo install --path .`) without requiring containers or external C toolchains.
  - Zero external runtime dependencies; single binary installed to `$CARGO_HOME/bin/botai` (`~/.cargo/bin/botai` or `%USERPROFILE%\.cargo\bin\botai.exe`) for direct shell execution.
  - Support for locked builds (`cargo install --path . --locked`) and development installations (`cargo install --path . --force`).

---

## 13. CI/CD Automation & PR Quality Validation Gates
- **Continuous Integration Pipeline (`.github/workflows/ci.yml`)**:
  - **Triggers**:
    - Exclusively runs on Pull Requests targeting the `main` branch.
  - **Concurrency Control**: Automatically cancels in-progress workflow runs for the same branch or PR ref (`cancel-in-progress: true`), preventing redundant build queue congestion.
  - **Security & Least Privilege**: Top-level permissions explicitly constrained to `contents: read`.
- **Parallel Quality Gates**:
  - **1. Rustfmt Gate (`fmt`)**:
    - Runs on `ubuntu-latest`.
    - Validates uniform codebase formatting across all source and test files (`cargo fmt --all -- --check`).
    - Uses stable toolchain with the official `rustfmt` component.
  - **2. Clippy Linter Gate (`clippy`)**:
    - Matrix across both Linux (`ubuntu-latest`) and Windows (`windows-latest`) runners with `fail-fast: false` (Job Name: `Clippy (${{ matrix.os }})`).
    - Enforces zero compiler and linter warnings (`cargo clippy --all-targets --all-features -- -D warnings`).
    - Employs GitHub Actions Rust caching (`Swatinem/rust-cache@v2`) for accelerated feedback cycles.
  - **3. Test Suite Gate (`test`)**:
    - Matrix across both Linux (`ubuntu-latest`) and Windows (`windows-latest`) runners with `fail-fast: false` (Job Name: `Test Suite (${{ matrix.os }})`).
    - Executes the entire test suite across all crates, targets, unit tests, and integration tests (`cargo test --all-targets --all-features`).
    - Executes documentation test suite (`cargo test --doc`).
    - Leverages shared Rust dependency and compilation caching (`Swatinem/rust-cache@v2`).
  - **4. Unsafe Code Audit Gate (`unsafe-audit`)**:
    - Runs on `ubuntu-latest`.
    - **Strict `#![forbid(unsafe_code)]` Enforcement**: Verifies that crate root headers (`src/lib.rs` and `src/main.rs`) explicitly mandate `#![forbid(unsafe_code)]`.
    - **Zero Unsafe Usage Policy**: Scans all source files under `src/` to guarantee zero `unsafe` keyword blocks or invocations exist in the production codebase.
    - **Compilation Verification**: Confirms clean `cargo check --all-targets --all-features` pass under the strict forbid lint constraint.

---

## 14. GitHub Release Workflow Automation (`.github/workflows/release.yml`)
- **Workflow Trigger (`workflow_dispatch`)**:
  - Exclusively triggered on demand via GitHub Actions manual workflow dispatch (`workflow_dispatch`).
  - **Inputs**:
    - `tag`: (Required string) Git tag to release (e.g., `v0.1.0`).
    - `prerelease`: (Optional boolean, default `false`) Manual override flag to mark the release as a pre-release.
  - **Tag Verification**:
    - Verifies that the specified git tag already exists in the repository prior to building.
    - Aborts workflow execution with an explicit error if the tag is not found.
  - **Pre-Release Evaluation**:
    - Automatically classified as a pre-release if `prerelease` input is `true` OR if the tag name contains a hyphen `-` (e.g., `v0.1.0-rc.1`, `v1.0.0-beta.2`).
- **Security & Permissions**:
  - Top-level or release job permissions: `contents: write` to publish GitHub releases and upload release assets.
- **Cross-Platform Runner Matrix**:
  - Compiles release binaries across the 3 native GitHub Actions runner operating systems:
    - **Linux (`x86_64`)**: Runner `ubuntu-latest`, archive format `.tar.gz`, platform stem `linux-x86_64`.
    - **macOS Apple Silicon (`aarch64`)**: Runner `macos-latest`, archive format `.tar.gz`, platform stem `macos-aarch64`.
    - **Windows (`x86_64`)**: Runner `windows-latest`, archive format `.zip`, platform stem `windows-x86_64`.
- **Packaging & Distributable Assets**:
  - Release archives contain:
    - Release binary (`botai` or `botai.exe`)
    - `README.md`
    - `LICENSE`
  - Archive naming format: `botai-<tag>-<platform>.<tar.gz|zip>` (e.g. `botai-v0.1.0-linux-x86_64.tar.gz`).
  - Generates SHA-256 checksums per platform archive during build jobs.
- **Release Publication & Checksum Aggregation**:
  - Dedicated publication job running after all matrix builds finish successfully.
  - Aggregates individual platform archive hashes into a single root `SHA256SUMS` manifest file.
  - Publishes GitHub Release targeting the provided `tag`:
    - Release title: `Release <tag>`
    - Release notes: GitHub auto-generated release notes (`generate_release_notes: true`).
    - Status: Published immediately (`draft: false`).
    - Pre-release status: Evaluated from hybrid detection (checkbox or tag hyphen).
    - Attached assets: Linux `.tar.gz`, macOS `.tar.gz`, Windows `.zip`, and combined `SHA256SUMS`.





