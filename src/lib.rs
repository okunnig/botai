#![forbid(unsafe_code)]

pub mod cli;
pub mod config;
pub mod context;
pub mod git;
pub mod notify;
pub mod project;
pub mod session;
pub mod tui;

pub use git::{remove_worktree_auto, resolve_main_repo};
pub use project::{ProjectBuilder, ProjectError, ProjectSetupEvent, ProjectValidation};
pub use tui::Theme;
