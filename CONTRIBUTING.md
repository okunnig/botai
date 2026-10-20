# Contributing to botai

Thank you for your interest in contributing to `botai`! We welcome contributions, bug reports, feature suggestions, and documentation improvements.

---

## Code of Conduct

All contributors and participants are expected to adhere to our [Code of Conduct](CODE_OF_CONDUCT.md). Please read it before participating.

---

## Development Prerequisites

- **Rust**: Version 1.74 or later (stable channel). You can install or update Rust using [rustup](https://rustup.rs/):
  ```bash
  rustup update stable
  ```
- **Git**: Modern Git installation (version 2.20+ recommended for full worktree support).
- **Cargo components**: `rustfmt` and `clippy` (included with standard Rust toolchains):
  ```bash
  rustup component add rustfmt clippy
  ```

---

## Architecture & Zero-Unsafe Policy

`botai` enforces a **strict zero-unsafe policy**:
- Both `src/lib.rs` and `src/main.rs` are annotated with `#![forbid(unsafe_code)]`.
- Use of the `unsafe` keyword anywhere in the codebase is strictly prohibited and enforced via automated CI checks.
- When platform-specific system APIs are necessary (such as Windows console virtual terminal mode), use safe crate abstractions (e.g., `crossterm_winapi`).

---

## Local Development Workflow

### 1. Clone Repository & Setup

```bash
git clone https://github.com/okunnig/botai.git
cd botai
```

### 2. Build the Project

```bash
cargo build
```

### 3. Verify Code Quality

Before opening a pull request or submitting changes, ensure your code passes all validation checks:

- **Formatting Check**:
  ```bash
  cargo fmt --all -- --check
  ```
  *(To automatically apply code formatting, run `cargo fmt --all`)*

- **Clippy Linter**:
  ```bash
  cargo clippy --all-targets --all-features -- -D warnings
  ```

- **Test Suite**:
  ```bash
  cargo test --all-targets --all-features
  ```

- **Documentation Tests**:
  ```bash
  cargo test --doc
  ```

---

## Commit Message Conventions

`botai` follows the [Conventional Commits](https://www.conventionalcommits.org/) specification:

```
<type>(<scope>): <short description>
```

Common types:
- `feat`: A new user-facing feature or CLI capability
- `fix`: A bug fix
- `docs`: Documentation updates or additions
- `refactor`: Code reorganization or cleanup without functional behavior changes
- `test`: Adding or updating test suites
- `chore`: Packaging, tooling, dependencies, or maintenance changes
- `perf`: Performance improvements

Examples:
- `feat(tui): add hotkey for quick template switcher`
- `fix(git): sanitize worktree path for windows filesystems`
- `docs(readme): add troubleshooting section for pty allocation`
- `chore(oss): prepare repository for open source release`

---

## Pull Request Guidelines

1. **Create a Topic Branch**: Create a descriptive branch from `main` (e.g., `git checkout -b feat/custom-agent-flags`).
2. **Atomic & Focused Changes**: Keep pull requests focused on a single logical change or bug fix.
3. **Add Tests**: Accompany new features or bug fixes with appropriate unit or integration tests under `tests/`.
4. **Preserve Documentation**: Keep documentation, docstrings, and `REQS.md` synchronized with any architectural or behavioral changes.
5. **Run Pre-Flight Checks**: Run formatting, clippy, and the test suite locally before pushing.
6. **Submit PR**: Open a pull request against the `main` branch. Complete the pull request template checklist to help reviewers understand your changes.
