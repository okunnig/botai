# TODO

## TUI Top Bar & Header Refactoring

Refactor the tmux-like top status bar and dashboard header into a clean, declarative Ratatui component.

### Tasks
- [ ] **Unified `TopBar` / `HeaderBar` Widget**:
  - Consolidate the duplicate header rendering logic in `src/tui/status_bar.rs` and `src/tui/dashboard.rs` into a single reusable widget.
  - Support both tabbed session views (project tabs, status badges, leader mode) and dashboard views (brand title, navigation hints).

- [ ] **Declarative Horizontal Layout (`Layout::horizontal`)**:
  - Replace manual X-coordinate offset math (`start_x = area.right().saturating_sub(...)`) with `Layout::horizontal([Constraint::Fill(1), Constraint::Length(...)])` to split tabs and right-aligned key hints/badges.

- [ ] **Native Right-Aligned Paragraphs**:
  - Render right-aligned hotkey hints and `[LEADER]` indicators using `Paragraph::new(...).right_aligned()` instead of manual line placement via `buf.set_line`.

- [ ] **Block-Based Background Styling**:
  - Eliminate nested cell-by-cell background clearing loops (`buf.cell_mut((x, y))`) by wrapping header areas in styled `Block` widgets or background style containers.

- [ ] **Streamlined Tab Windowing & Overflow**:
  - Simplify the sliding-window tab centering / clipping algorithm when tabs exceed viewport width.
