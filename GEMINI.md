# botai - Agent Orchestrator

A cross-platform terminal session orchestrator and multiplexer for AI agent CLI tools (Claude Code, agy, Copilot CLI, etc.) written in idiomatic Rust.

## Key Concepts
- **Workspace**: A named collection of local git repositories defined in a declarative TOML configuration (`~/.config/botai/config.toml` or `%APPDATA%\botai\config.toml`).
- **Project**: An isolated, multi-repo work environment created inside `CWD/<project_name>`. Each repository gets a git worktree checked out to a new branch `<project_name>`.
- **Context Synthesis**: Auto-generated multi-repo orientation context (`AGENTS.md`) at the project root for seamless agent multi-directory comprehension.
- **Agent Instance & Monitoring**: Single AI agent CLI process per project root with desktop notifications and real-time status badges in the TUI (Thinking, Waiting for Input, Done).
- **TUI Multiplexer**: Native Rust-powered terminal multiplexing (`portable-pty`, `ratatui`, `crossterm`) with leader-free keybindings (`Alt+1..9`, `Alt+N/P`, `Alt+D`).

## Build Configuration
- All build artifacts and temporary files must be directed to `/tmp/botai-build` (configured in `.cargo/config.toml` via `build.target-dir = "/tmp/botai-build"`).

## Architecture & Requirements
See [REQS.md](file:///home/okunnig/programming/botai/REQS.md) for full technical requirements, configuration schemas, CLI specifications, and git worktree lifecycle details.

## Implementation
- use of `unsafe` is strictly forbidden
- apply engineering best practices
- create commits after each completed implementation
- Requirements must be written to REQS.md, the document must describe the full feature set
