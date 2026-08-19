# Requirements for botai (Agent Orchestrator)

`botai` is an opinionated, cross-platform terminal session orchestrator and multiplexer for AI agent CLI tools (e.g., Claude Code, agy, GitHub Copilot CLI, Aider) written in idiomatic Rust.

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
- **Hybrid Repository Declarations**:
  - Repositories in `templates.<name>.repos` support:
    - **Remote Git URLs** (`https://...`, `git@...`, `ssh://...`): Automatically cloned into `<repos_dir>/<repo_name>`.
    - **Relative repo names** (e.g., `"ecommerce-api"`): Resolved relative to `repos_dir` (`<repos_dir>/ecommerce-api`).
    - **Absolute local paths** (e.g., `/home/user/code/my-repo`): Direct reference to existing local repository.
  - Sanitized flat naming inside `repos_dir` based on the repository basename.
  - Conflict detection: Errors if two different remote URLs resolve to the same folder name.
- **Templates / Project Templates**: Define named sets of local or remote repositories. Each template can specify `description`, `default_agent`, `agents_md_append` (custom template instructions appended to `AGENTS.md`), `repos`, and `env`.
- **Agent Definitions**: Configurable agent launch commands, arguments, and environment variables.
- **Configuration Template Generation (`botai init`)**:
  - Built-in generator for a fully documented, self-describing configuration file with extensive explanatory comments.
  - Serves as an interactive starting template and inline documentation for all supported agents, project templates, repos, and environment variable configurations.
  - Automatically targets the standard platform configuration location with safe directory creation and overwrite guards.

### Annotated `config.toml` Template
```toml
# ==============================================================================
# botai - Configuration File
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
# Project Templates
# ------------------------------------------------------------------------------
# Define named multi-repo project templates. Each template specifies:
# - description: Optional summary of the template.
# - default_agent: Optional agent override specific to this template.
# - agents_md_append: (Optional) Custom markdown instructions appended to AGENTS.md.
# - repos: List of Git URLs, relative repo names in repos_dir, or absolute local paths.
# - env: Environment variables injected into the agent session.

[templates.ecommerce]
description = "E-commerce platform services"
default_agent = "agy"
agents_md_append = """
- Run integration tests with `cargo test --workspace`.
- Follow coding standards documented in the template.
"""
repos = [
  "https://github.com/example/ecommerce-api.git",
  "https://github.com/example/ecommerce-web.git",
  "/home/user/code/ecommerce-shared"
]
env = { PORT = "8080", NODE_ENV = "development" }
```

---

## 3. Project Creation & Git Worktree Lifecycle
- **Project Structure**: When starting a project for a template, `botai` creates a project root directory in the current working directory (`CWD/<project_name>`).
- **Automatic On-Demand Cloning**:
  - Missing repositories in `repos_dir` are automatically cloned on-demand before worktree creation.
  - If a clone fails (e.g. invalid URL, authentication failure), project initialization cleanly rolls back partially created directories.
- **Best-Effort Upstream Fetching**:
  - Performs a best-effort `git fetch origin` for cached repositories to discover upstream default branch changes.
  - Branches worktrees off `origin/<default_branch>` when available, or falls back to local default branch if offline.
- **Worktrees**: For every repository defined in the template:
  - Creates a git worktree inside `<project_name>/<repo_dir_name>`.
  - Checks out a new branch named `<project_name>`.
  - **Strict Branching Strategy**: Branches strictly off the latest default branch (`origin/main`, `origin/master`, or local `main` / `master`) of each repository.
  - **Collision Prevention**: Fails with an explicit error if a branch or worktree named `<project_name>` already exists in any of the template repositories or if `<project_name>` directory exists.
- **Agent Execution**: Spawns a single agent CLI instance at the project root (`CWD/<project_name>`), providing access to all worktree subdirectories.

---

## 4. Multi-Repo Context Synthesis & Seeding
- **Auto-Generated Context Files (`AGENTS.md` / `WORKSPACE.md`)**:
  - Automatically synthesizes a root orientation file inside `CWD/<project_name>` detailing:
    - The multi-repo structure and mapping of child worktree directories.
    - Each repository's purpose and inter-dependencies.
    - Guidance for the AI agent on navigating, building, and modifying multiple repos simultaneously.
- **Custom Template Instructions (`agents_md_append`)**:
  - Optional custom markdown instructions configured per template in `config.toml` via `agents_md_append`.
  - When configured and non-empty after trimming, automatically appended to `AGENTS.md` under a dedicated `## Template Instructions` section.
- **Initial Task / Prompt Seeding**:
  - `botai start <template> <project-name> --prompt "..."` (or `-m "..."`):
  - Injects or passes an initial task description directly to the AI agent on launch.

---

## 5. Agent Monitoring & Notifications
- **Desktop Notifications**:
  - Emits native OS desktop notifications (via `notify-rust` / terminal notifications) when:
    - An agent CLI process completes a task or exits.
    - An agent transitions to an idle state awaiting user input / approval.
- **Real-Time Session Indicators in TUI**:
  - Live status indicators formatted as a leading solid round dot symbol (`●` U+25CF) inside project tabs in the status bar (e.g., `  ● 1: my-project  ` with active/inactive styling):
    - ● **Busy / Thinking**: Themed Busy color (`theme.status_busy`, e.g., Cyan / Electric Blue / Amber) when agent is actively executing.
    - ● **Waiting for Input**: Themed Waiting color (`theme.status_waiting`, e.g., Peach / Amber / Sky Blue) when agent is awaiting user response or confirmation.
    - ● **Done / Idle**: Themed Done color (`theme.status_done`, e.g., Green) when agent finished its assigned prompt or task.
    - ● **Error / Interrupted**: Themed Error color (`theme.status_error`, e.g., Red) on non-zero exit code or process failure.
    - ● **Initializing / Terminated**: Themed status bar foreground color (`theme.status_bar_fg`).

---

## 6. Session Management & Navigation (TUI Multiplexer)
- **Multi-Project Concurrency**: Maintains concurrent active project sessions in the background.
- **Dynamic Viewport & Responsive Resizing**:
  - Automatically synchronizes PTY dimensions and VT virtual screen buffers to the host terminal viewport on initial startup, on window resize events (`Event::Resize`), and when creating or switching active project sessions.
  - Propagates window size changes (`SIGWINCH` / `ResizePseudoConsole`) so full-screen interactive agent CLIs (`agy`, `claude`, `aider`, etc.) dynamically re-render to the exact available terminal dimensions.
- **Hardware Cursor Positioning & Visibility**:
  - Accurately tracks cursor coordinates and visibility state (`\x1b[?25h` / `\x1b[?25l`) from the active session's VT parser.
  - Positions the terminal hardware cursor directly at the active input position during interactive sessions, rendering the native block cursor where user input is focused.
  - Automatically positions cursor in active text fields within dialogs/modals and hides the cursor when appropriate.
- **Session Termination & Exit Transitions**:
  - **Multi-Session Scenario (Normal Exit)**: When an agent process exits cleanly (exit code 0) and other session tabs remain active, the finished tab is automatically closed and focus transitions immediately to the adjacent active session tab.
  - **Last / Only Session Scenario (Context-Aware Exit)**:
    - If `botai` was launched from the dashboard, terminating the last session automatically transitions back to the interactive project dashboard (with refreshed project directories).
    - If `botai` was launched directly via CLI (`botai start` or `botai attach`), terminating the last session exits `botai` cleanly back to the host shell.
  - **Error & Crash Retention**: If an agent process exits with an error or crash (non-zero exit code or abnormal termination), the session remains visible in the registry with the Error status (`●` in `status_error` color) and an inline prompt banner (e.g. `[Process exited with code X - Press Enter / Ctrl+Space w to dismiss]`), ensuring error outputs and logs are not lost until explicitly dismissed.
- **Session Detachment & Background Persistence**:
  - Detaching from an active session (`Ctrl + Space` then `d`):
    - When launched from the dashboard, returns to the interactive project dashboard with refreshed project listings while leaving all background sessions and agent processes running. Users can re-attach to existing running sessions, switch between active sessions, or create/destroy other projects.
    - When launched directly from CLI (`botai start` or `botai attach` without dashboard), detaching exits `botai` cleanly back to the host shell while leaving background sessions intact.
- **Status Bar & Project Indicators**: An unobtrusive top/bottom status bar displaying active sessions, status badges, active project name, template, key hints, and a visible `[LEADER]` mode indicator when active.
- **Leader Key Architecture (`Ctrl + Space`)**:
  - Global multiplexer actions are invoked via the leader prefix `Ctrl + Space`, followed by a command key:
    - `Ctrl + Space` then `1` .. `9`: Directly switch to project session 1 to 9.
    - `Ctrl + Space` then `c` (or `n`): Open the Quick Project Launcher & Creator modal.
    - `Ctrl + Space` then `n` / `p`: Switch to Next / Previous active project session.
    - `Ctrl + Space` then `w` (or `x`): Close / terminate active project session.
    - `Ctrl + Space` then `d`: Detach from active session (returns to dashboard when dashboard is present, or exits to host shell when running in standalone CLI mode).
    - `Ctrl + Space` then `q`: Quit `botai`.
    - `Ctrl + Space` then `Ctrl + Space` (or `Space`): Forward a literal `Ctrl + Space` (`0x00`) to the active PTY.
    - `Ctrl + Space` then `Esc` (or any unrecognized key): Cancel leader mode without executing an action.

---

## 7. CLI Commands & Lifecycle Operations
- **Default (No Arguments)**: Launching `botai` without arguments opens the interactive TUI project switcher / dashboard.
- **`botai start <template> <project-name> [--agent <agent>] [--prompt <prompt>]`**:
  - Validates repository states and branch names.
  - Clones missing remote repositories to `repos_dir` and performs best-effort upstream fetch.
  - Creates project directory and git worktrees.
  - Generates multi-repo `AGENTS.md` context file.
  - Launches the agent CLI (with initial prompt if provided) and drops user into the interactive TUI session.
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
- **`botai destroy <project-name> [--force]`**:
  - Refuses deletion if any worktree contains uncommitted git changes unless `--force` is specified.
  - Cleans up git worktrees and removes the project directory.
  - Preserves git branches (leaving code intact in git history).

---

## 8. Interactive TUI Project Picker & Launcher Dashboard
- **Top Header Status Bar**:
  - Full-width top header bar (`Constraint::Length(1)`) matching the rest of the application's header styling.
  - Filled with `theme.status_bar_bg` and `theme.status_bar_fg`.
  - Displays a clean brand title ` botai ` on the left styled with `theme.modal_title` and bold formatting.
  - Displays right-aligned key navigation hints styled with `theme.key_hint`:
    - Full hints: `"Tab: Switch | ↑/↓: Navigate | Enter: Select | n: New | d: Destroy | q: Quit"`
    - Responsive compact fallback for narrower screens: `"Tab: Switch | Enter: Select | n: New | d: Destroy | q: Quit"`
    - Minimal fallback: `"q: Quit"`
- **Unified Two-Panel Dashboard**:
  - Displayed when launching `botai` with no active sessions, positioned below the top header status bar.
  - **Left Panel (Existing Projects)**: Lists existing local project directories found in the current working directory. Running project sessions are distinguished with a leading colored status dot indicator (`●` U+25CF) reflecting the session's live state (e.g. Busy, Waiting, Done, Error) while inactive projects display blank spacing (`  `). Press `Enter` to attach or switch focus to the selected project.
  - **Right Panel (Project Templates)**: Lists all available project templates from `config.toml`. Press `n` or `Enter` to create and launch a new project.
- **Interactive Project Creation Modal**:
  - Floating centered modal triggered by `n` or template selection.
  - Input fields for `Project Name:` and optional `Initial Prompt:`.
  - On submission (`Enter`), transitions smoothly into the **Asynchronous Live Setup Progress & Log Modal**.
- **Asynchronous Live Setup Progress & Log Modal**:
  - Executes git repository cloning, fetching, worktree initialization, context generation, and PTY process spawning on a background worker thread without blocking the TUI event loop or multiplexer.
  - Displays real-time step indicators (`Step [1/4]..[4/4]`), status messages, an animated activity spinner, and a scrollable tailing log window streaming stdout/stderr messages.
  - **Success Transition**: Automatically adds the newly created session to `SessionRegistry`, switches focus to the new project tab, and dismisses the setup modal.
  - **Error Retention & Rollback**: On failure (e.g. branch collision, network timeout, git clone failure), cleanly rolls back partially created directories and worktrees, keeps the setup modal open displaying the error description and full logs, and prompts `[Esc] / [Enter] Close` so diagnostic details are never lost.
- **Destroy Project Confirmation Modal**:
  - Pressing `d` on the Left Panel (Existing Projects) opens a dedicated centered confirmation modal (` Confirm Destroy Project `) styled with alert theme colors.
  - Prompts: `Are you sure you want to destroy project '<project_name>'? This will permanently remove the project directory and its git worktrees.`
  - **Confirmation**: Pressing `y`, `Y`, or `Enter` closes the modal and executes clean project directory and worktree removal.
  - **Cancellation**: Pressing `n`, `N`, or `Esc` closes the modal without performing any destructive action.
  - Hardware cursor is automatically hidden while the confirmation modal is open.
- **Dashboard Keybindings**:
  - `Tab` / `Left` / `Right`: Switch active panel focus.
  - `Up` / `Down`: Navigate list items within the active panel.
  - `Enter`: Attach/switch to selected project (Left Panel) or open creation modal (Right Panel).
  - `n`: Trigger new project modal for the selected template.
  - `d`: Open destroy confirmation modal for the selected project.
  - `q` / `Esc`: Quit `botai`.

---

## 9. In-Session Quick Launcher & Project Creator Modal
- **In-Session Trigger (`Ctrl + Space` then `c` / `n`)**:
  - Can be invoked at any time while working inside an active terminal session.
  - Renders a floating modal centered over the active terminal view without interrupting background PTY reading or activity tracking.
- **Tabbed Modal Views**:
  - **Create New Project Tab**:
    - **Template Selector**: Defaults to the active project's template (or the first configured template); cycled using `Left` / `Right`.
    - **Project Name Input**: Text entry field with interactive editing.
    - **Initial Prompt Input**: Optional text entry field for initial task instruction.
    - **Submission**: On `Enter`, closes the quick launcher and transitions immediately into the **Asynchronous Live Setup Progress & Log Modal**, spawning a background worker to execute repository cloning, worktree creation, `AGENTS.md` context synthesis, and agent PTY launching without freezing the active session.
    - **Inline & Modal Error Feedback**: If creation fails, full diagnostic logs and error banners are retained until dismissed via `[Esc]` or `[Enter]`.
  - **Attach Existing Project Tab**:
    - Lists local project directories in CWD (directories with `AGENTS.md`).
    - Running sessions are visually indicated with a leading colored status dot indicator (`●` U+25CF) reflecting their real-time state (`Busy`, `Waiting`, `Done`, `Error`).
    - Navigated via `Up` / `Down` / `j` / `k`.
    - **Submission**: If the project already has an active session tab, switches focus to it; otherwise spawns and attaches the project in a new session tab, adds it to the registry, closes the modal, and switches to it.
- **Navigation & Controls**:
  - `Tab` / `BackTab` (`Shift+Tab`): Move focus between modal tabs and form fields.
  - `Left` / `Right`: Switch between 'Create New' and 'Attach Existing' tabs when tab bar is focused, or cycle templates when template field is focused.
  - `Enter`: Submit create or attach action.
  - `Esc`: Dismiss modal and return focus to active terminal PTY.


---

## 10. UI Theming & Color Schemes
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
  - **Modals (Creation Modal & Quick Launcher)**: Dialog borders, active/inactive tab bars, focused input borders/prompts, placeholder text, field labels, and button/key hints.
  - **Exit & Crash Banners**: High-visibility error notification banners rendered across the bottom line upon non-zero process exits.

---

## 11. Cross-Platform Compilation & Distributable Artifacts
- **Target Matrix & Architectural Support**:
  - **Linux (glibc)**:
    - `x86_64-unknown-linux-gnu` (standard glibc Linux x86_64)
    - `aarch64-unknown-linux-gnu` (glibc Linux ARM64 / aarch64)
  - **Windows (x86_64)**:
    - `x86_64-pc-windows-gnu` (Windows 64-bit executable `botai.exe`)
  - **macOS (Intel & Apple Silicon)**:
    - `x86_64-apple-darwin` (Intel 64-bit macOS)
    - `aarch64-apple-darwin` (Apple Silicon M1/M2/M3 macOS)
- **Containerized Cross-Compilation (`cross`)**:
  - Strictly uses `cross` containerized builds for multi-platform compilation without polluting the host environment.
  - Verifies `cross` is installed in `PATH` before executing builds.
  - **`Cross.toml`**: Configures target architectures with container image definitions and environment passthroughs.
- **Unified Build Script (`scripts/build.sh`)**:
  - Clean, concise (~80 lines), fully self-contained bash script.
  - **Usage Modes**:
    - `./scripts/build.sh` or `./scripts/build.sh all`: Builds all supported targets (Linux, Windows, macOS).
    - `./scripts/build.sh linux`: Builds Linux targets (`x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`).
    - `./scripts/build.sh windows`: Builds Windows target (`x86_64-pc-windows-gnu`).
    - `./scripts/build.sh macos`: Builds macOS targets (`x86_64-apple-darwin`, `aarch64-apple-darwin`).
    - `./scripts/build.sh <target-triple>`: Builds a specific target triple (e.g., `./scripts/build.sh x86_64-unknown-linux-gnu`).
- **Packaging Format & Distributable Artifacts**:
  - Linux / macOS: `.tar.gz` archive containing binary (`botai`), `README.md`, and `REQS.md`.
  - Windows: `.zip` archive containing `botai.exe`, `README.md`, and `REQS.md`.
  - `dist/botai-v<version>-linux-x86_64.tar.gz`
  - `dist/botai-v<version>-linux-aarch64.tar.gz`
  - `dist/botai-v<version>-windows-x86_64.zip`
  - `dist/botai-v<version>-macos-x86_64.tar.gz`
  - `dist/botai-v<version>-macos-aarch64.tar.gz`
  - `dist/SHA256SUMS`: Cryptographic SHA-256 verification manifest automatically generated in `dist/`.



