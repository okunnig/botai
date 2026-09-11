# botai - Multi-Repo Workspace Orchestrator

A cross-platform multi-repo workspace orchestrator and session multiplexer for AI agent workflows (Claude Code, agy, Copilot CLI, etc.) written in idiomatic Rust.

## AI Agent usage

- main agent focuses on determining what to do, how to do it and user communication
- main agent delegates implementation and TDD workflow to subagents

## Key Concepts
- **Template** / **Project Template**: A named collection of git repositories defined in a declarative TOML configuration (`~/.config/botai/config.toml` or `%APPDATA%\botai\config.toml`).
- **Project**: An isolated, multi-repo work environment created inside `CWD/<project_name>`. Each repository gets a git worktree checked out to a new branch `<project_name>`.
- **Context Synthesis**: Auto-generated multi-repo orientation context (`AGENTS.md`) at the project root for seamless agent multi-directory comprehension.
- **Agent Instance & Monitoring**: Single AI agent CLI process per project root with desktop notifications and real-time status badges in the TUI (Thinking, Waiting for Input, Done).
- **TUI Multiplexer**: Native Rust-powered terminal multiplexing (`portable-pty`, `ratatui`, `crossterm`) with `Ctrl+Space` leader key architecture (`Ctrl+Space` -> `1..9`, `c`, `n`/`p`, `w`, `x`, `d`, `q`).

## Architecture & Requirements
See [REQS.md](REQS.md) for full technical requirements, configuration schemas, CLI specifications, and git worktree lifecycle details.

## Implementation
- use of `unsafe` is strictly forbidden
- apply engineering best practices
- create commits after each completed implementation
- Requirements must be written to REQS.md, the document must describe the full feature set
