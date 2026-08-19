# botai - Agent Orchestrator

A cross-platform terminal session orchestrator and multiplexer for AI agent CLI tools (Claude Code, agy, GitHub Copilot CLI, Aider, etc.) written in idiomatic Rust.

## Features

- **Multi-Repo Workspace Isolation**: Declarative project templates manage collections of Git repositories with isolated Git worktrees.
- **Context Synthesis**: Automatically generates `AGENTS.md` orientation context detailing the multi-repo structure and inter-dependencies.
- **PTY Session Multiplexer**: Embedded terminal multiplexer powered by `portable-pty`, `ratatui`, and `crossterm` with `Ctrl+Space` leader key navigation.
- **Real-Time Agent Monitoring**: Visual status badges (Thinking, Waiting for Input, Done, Error) and native desktop notifications.
- **Zero Runtime Dependencies**: Standalone compiled binary with no system tmux or zellij requirement.

## Quick Start

### Interactive Project Dashboard
Run `botai` without arguments to open the interactive TUI dashboard:
```bash
botai
```

### CLI Commands
```bash
# Start a new project from a template
botai start ecommerce my-feature-branch --prompt "Implement payment webhook handler"

# List configured templates and active projects
botai list

# Sync / pre-clone template repositories
botai sync ecommerce

# Attach to an ongoing project session
botai attach my-feature-branch

# Initialize default configuration
botai init

# Clean up and destroy a project worktree
botai destroy my-feature-branch
```

## Release Build & Packaging

Native release compilation and packaging is handled via `scripts/build.sh`:

```bash
# Build and package release binary for the current host machine
./scripts/build.sh
```

The script automatically detects the host operating system and architecture (`linux-x86_64`, `linux-aarch64`, `macos-x86_64`, `macos-aarch64`, `windows-x86_64`), invokes `cargo build --release`, and packages the binary with `README.md` and `REQS.md` into `dist/botai-v<version>-<platform>.<tar.gz|zip>`. It also generates and updates SHA-256 checksums in `dist/SHA256SUMS`.

## Documentation & Requirements

For complete technical specifications, configuration schema, and architectural requirements, see [REQS.md](REQS.md).
