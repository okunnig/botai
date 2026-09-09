#![forbid(unsafe_code)]

pub mod cli;
pub mod config;
pub mod context;
pub mod git;
pub mod notify;
pub mod project;
pub mod session;
pub mod tui;

pub use config::{resolve_editor, run_editor, split_command_words, Config, ConfigError};
pub use git::{remove_worktree_auto, resolve_main_repo};
pub use project::{
    adapt_agent_args, destroy_project_workspace, destroy_project_workspace_with_progress,
    extract_template_name_from_agents_md, ProjectBuilder, ProjectDestroyEvent, ProjectError,
    ProjectSetupEvent, ProjectValidation,
};
pub use tui::Theme;
