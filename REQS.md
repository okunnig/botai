# Requirements for botai (Agent Orchestrator)

`botai` is an opinionated, cross-platform terminal session orchestrator and multiplexer for AI agent CLI tools (e.g., Claude Code, agy, GitHub Copilot CLI, Aider) written in idiomatic Rust.

---

## 1. Core Architecture & Platform Support
- **Cross-Platform**: Compiles natively to Linux, macOS, and Windows (using Windows ConPTY).
- **Strict CLI / TUI Application**: Single binary with zero external runtime dependencies (no system `tmux` or `zellij` requirement).
- **Embedded Terminal Multiplexer**: Built-in PTY management and TUI multiplexer using `portable-pty`, `ratatui`, and `crossterm`.

---

## 2. Configuration & Workspace Management
- **Declarative TOML Configuration**: Workspaces and agent tool definitions are configured via a user configuration file:
  - Linux / macOS: `~/.config/botai/config.toml` (or `$XDG_CONFIG_HOME/botai/config.toml`)
  - Windows: `%APPDATA%\botai\config.toml`
- **Central Repositories Directory (`repos_dir`)**:
  - Required global configuration field `repos_dir` specifying where remote repositories are cloned and cached.
  - Does not assume any implicit default directory; must be explicitly configured in `config.toml` (e.g. `repos_dir = "~/.local/share/botai/repos"` or `repos_dir = "~/code/repos"`).
  - Can be overridden at runtime via the `BOTAI_REPOS_DIR` environment variable.
- **Hybrid Repository Declarations**:
  - Repositories in `workspaces.<name>.repos` support:
    - **Remote Git URLs** (`https://...`, `git@...`, `ssh://...`): Automatically cloned into `<repos_dir>/<repo_name>`.
    - **Relative repo names** (e.g., `"ecommerce-api"`): Resolved relative to `repos_dir` (`<repos_dir>/ecommerce-api`).
    - **Absolute local paths** (e.g., `/home/user/code/my-repo`): Direct reference to existing local repository.
  - Sanitized flat naming inside `repos_dir` based on the repository basename.
  - Conflict detection: Errors if two different remote URLs resolve to the same folder name.
- **Workspaces**: Define named sets of local or remote repositories.
- **Agent Definitions**: Configurable agent launch commands, arguments, and environment variables.
- **Configuration Template Generation (`botai init`)**:
  - Built-in generator for a fully documented, self-describing configuration file with extensive explanatory comments.
  - Serves as an interactive starting template and inline documentation for all supported agents, workspaces, repos, and environment variable configurations.
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

# Global default AI agent tool to use when a workspace does not specify an override.
default_agent = "claude"

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
# Workspaces
# ------------------------------------------------------------------------------
# Define named multi-repo workspaces. Each workspace specifies:
# - description: Optional summary of the workspace.
# - default_agent: Optional agent override specific to this workspace.
# - repos: List of Git URLs, relative repo names in repos_dir, or absolute local paths.
# - env: Environment variables injected into the agent session.

[workspaces.ecommerce]
description = "E-commerce platform services"
default_agent = "agy"
repos = [
  "https://github.com/example/ecommerce-api.git",
  "https://github.com/example/ecommerce-web.git",
  "/home/user/code/ecommerce-shared"
]
env = { PORT = "8080", NODE_ENV = "development" }
```

---

## 3. Project Creation & Git Worktree Lifecycle
- **Project Structure**: When starting a project for a workspace, `botai` creates a project root directory in the current working directory (`CWD/<project_name>`).
- **Automatic On-Demand Cloning**:
  - Missing repositories in `repos_dir` are automatically cloned on-demand before worktree creation.
  - If a clone fails (e.g. invalid URL, authentication failure), project initialization cleanly rolls back partially created directories.
- **Best-Effort Upstream Fetching**:
  - Performs a best-effort `git fetch origin` for cached repositories to discover upstream default branch changes.
  - Branches worktrees off `origin/<default_branch>` when available, or falls back to local default branch if offline.
- **Worktrees**: For every repository defined in the workspace:
  - Creates a git worktree inside `<project_name>/<repo_dir_name>`.
  - Checks out a new branch named `<project_name>`.
  - **Strict Branching Strategy**: Branches strictly off the latest default branch (`origin/main`, `origin/master`, or local `main` / `master`) of each repository.
  - **Collision Prevention**: Fails with an explicit error if a branch or worktree named `<project_name>` already exists in any of the workspace repositories or if `<project_name>` directory exists.
- **Agent Execution**: Spawns a single agent CLI instance at the project root (`CWD/<project_name>`), providing access to all worktree subdirectories.

---

## 4. Multi-Repo Context Synthesis & Seeding
- **Auto-Generated Context Files (`AGENTS.md` / `WORKSPACE.md`)**:
  - Automatically synthesizes a root orientation file inside `CWD/<project_name>` detailing:
    - The multi-repo structure and mapping of child worktree directories.
    - Each repository's purpose and inter-dependencies.
    - Guidance for the AI agent on navigating, building, and modifying multiple repos simultaneously.
- **Initial Task / Prompt Seeding**:
  - `botai start <workspace> <project-name> --prompt "..."` (or `-m "..."`):
  - Injects or passes an initial task description directly to the AI agent on launch.

---

## 5. Agent Monitoring & Notifications
- **Desktop Notifications**:
  - Emits native OS desktop notifications (via `notify-rust` / terminal notifications) when:
    - An agent CLI process completes a task or exits.
    - An agent transitions to an idle state awaiting user input / approval.
- **Real-Time Session Badges in TUI**:
  - Live status indicators next to project tabs in the status bar:
    - ⏳ **Busy / Thinking**: Agent process is actively executing / outputting.
    - 💬 **Waiting for Input**: Agent process is idle, awaiting user response or confirmation.
    - ✅ **Done / Idle**: Agent finished its assigned prompt or task.
    - ⚠️ **Error / Interrupted**: Non-zero exit code or process interruption.

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
  - **Error & Crash Retention**: If an agent process exits with an error or crash (non-zero exit code or abnormal termination), the session remains visible in the registry with the `Error ⚠️` badge and an inline prompt banner (e.g. `[Process exited with code X - Press Enter / Ctrl+Space w to dismiss]`), ensuring error outputs and logs are not lost until explicitly dismissed.
- **Status Bar & Project Indicators**: An unobtrusive top/bottom status bar displaying active sessions, status badges, active project name, workspace, key hints, and a visible `[LEADER]` mode indicator when active.
- **Leader Key Architecture (`Ctrl + Space`)**:
  - Global multiplexer actions are invoked via the leader prefix `Ctrl + Space`, followed by a command key:
    - `Ctrl + Space` then `1` .. `9`: Directly switch to project session 1 to 9.
    - `Ctrl + Space` then `c` (or `n`): Open the Quick Project Launcher & Creator modal.
    - `Ctrl + Space` then `n` / `p`: Switch to Next / Previous active project session.
    - `Ctrl + Space` then `w` (or `x`): Close / terminate active project session.
    - `Ctrl + Space` then `d`: Detach from active session (return to CLI / dashboard).
    - `Ctrl + Space` then `q`: Quit `botai`.
    - `Ctrl + Space` then `Ctrl + Space` (or `Space`): Forward a literal `Ctrl + Space` (`0x00`) to the active PTY.
    - `Ctrl + Space` then `Esc` (or any unrecognized key): Cancel leader mode without executing an action.

---

## 7. CLI Commands & Lifecycle Operations
- **Default (No Arguments)**: Launching `botai` without arguments opens the interactive TUI project switcher / dashboard.
- **`botai start <workspace> <project-name> [--agent <agent>] [--prompt <prompt>]`**:
  - Validates repository states and branch names.
  - Clones missing remote repositories to `repos_dir` and performs best-effort upstream fetch.
  - Creates project directory and git worktrees.
  - Generates multi-repo `AGENTS.md` context file.
  - Launches the agent CLI (with initial prompt if provided) and drops user into the interactive TUI session.
- **`botai sync [workspace]`**:
  - Pre-clones missing repositories and updates existing cached repositories across all or a specific workspace.
  - Clones missing repositories to `repos_dir` if needed.
- **`botai attach <project-name>`**: Re-attaches to an ongoing project session.
- **`botai list`**: Lists configured workspaces, repositories with clone/cached status, active projects, worktrees, and running sessions.
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
- **Unified Two-Panel Dashboard**:
  - Displayed when launching `botai` with no active sessions.
  - **Left Panel (Existing Projects)**: Lists existing local project directories found in the current working directory. Press `Enter` to attach to the selected project.
  - **Right Panel (Configured Workspaces)**: Lists all available workspaces from `config.toml`. Press `n` or `Enter` to create and launch a new project.
- **Interactive Project Creation Modal**:
  - Floating centered modal triggered by `n` or workspace selection.
  - Input fields for `Project Name:` and optional `Initial Prompt:`.
  - Automatically initializes worktrees, generates `AGENTS.md`, spawns the configured AI agent in a PTY, and switches to the live multiplexer session.
- **Dashboard Keybindings**:
  - `Tab` / `Left` / `Right`: Switch active panel focus.
  - `Up` / `Down`: Navigate list items within the active panel.
  - `Enter`: Attach to selected project (Left Panel) or open creation modal (Right Panel).
  - `n`: Trigger new project modal for the selected workspace.
  - `d`: Destroy selected project directory (with confirmation check).
  - `q` / `Esc`: Quit `botai`.

---

## 9. In-Session Quick Launcher & Project Creator Modal
- **In-Session Trigger (`Ctrl + Space` then `c` / `n`)**:
  - Can be invoked at any time while working inside an active terminal session.
  - Renders a floating modal centered over the active terminal view without interrupting background PTY reading or activity tracking.
- **Tabbed Modal Views**:
  - **Create New Project Tab**:
    - **Workspace Selector**: Defaults to the active project's workspace (or the first configured workspace); cycled using `Left` / `Right`.
    - **Project Name Input**: Text entry field with interactive editing.
    - **Initial Prompt Input**: Optional text entry field for initial task instruction.
    - **Inline Error Feedback**: If creation fails (e.g. branch or worktree collision, directory exists, clone failure), the modal remains open displaying an inline error banner so input is not lost.
    - **Submission**: On `Enter`, creates worktrees, generates `AGENTS.md`, spawns the agent in a new PTY session, registers it in `SessionRegistry`, closes the modal, and switches immediately to the new session tab.
  - **Attach Existing Project Tab**:
    - Lists local project directories in CWD (directories with `AGENTS.md`).
    - Navigated via `Up` / `Down` / `j` / `k`.
    - **Submission**: If the project already has an active session tab, switches focus to it; otherwise spawns and attaches the project in a new session tab, adds it to the registry, closes the modal, and switches to it.
- **Navigation & Controls**:
  - `Tab` / `BackTab` (`Shift+Tab`): Move focus between modal tabs and form fields.
  - `Left` / `Right`: Switch between 'Create New' and 'Attach Existing' tabs when tab bar is focused, or cycle workspaces when workspace field is focused.
  - `Enter`: Submit create or attach action.
  - `Esc`: Dismiss modal and return focus to active terminal PTY.

