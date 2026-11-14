## Description

<!-- Provide a concise description of the changes made and the motivation behind them. -->

## Related Issues

<!-- Reference related issue(s) using keywords (e.g., Fixes #123, Closes #456) -->
Closes #

## Type of Change

- [ ] `feat`: New user-facing feature or CLI subcommand/flag
- [ ] `fix`: Bug fix
- [ ] `docs`: Documentation updates or additions
- [ ] `refactor`: Internal code reorganization without changing behavior
- [ ] `perf`: Performance optimization
- [ ] `test`: New or updated tests
- [ ] `chore`: Tooling, packaging, dependencies, or repository maintenance

## Checklist

- [ ] Code follows the project's formatting conventions (`cargo fmt --all -- --check`).
- [ ] No clippy warnings or errors (`cargo clippy --all-targets --all-features -- -D warnings`).
- [ ] All unit and integration tests pass (`cargo test --all-targets --all-features`).
- [ ] Doc tests pass (`cargo test --doc`).
- [ ] **Strict Zero-Unsafe Policy**: Verified no `unsafe` blocks exist (`#![forbid(unsafe_code)]` preserved).
- [ ] If applicable, updated `REQS.md` and documentation (`README.md`).
- [ ] Commit messages follow the [Conventional Commits](https://www.conventionalcommits.org/) convention.
