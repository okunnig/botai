use clap::{Parser, Subcommand};
use std::path::PathBuf;

/// botai - Terminal session orchestrator and multiplexer for AI agent CLI tools.
#[derive(Parser, Debug, Clone, PartialEq, Eq)]
#[command(name = "botai", author, version, about, long_about = None)]
pub struct Cli {
    /// Custom configuration file path.
    #[arg(short, long, global = true, value_name = "FILE")]
    pub config: Option<PathBuf>,

    /// Subcommand to execute. If omitted, launches the interactive TUI multiplexer.
    #[command(subcommand)]
    pub command: Option<Commands>,
}

/// Available subcommands for botai.
#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum Commands {
    /// Start a new project with git worktrees and launch an AI agent CLI.
    Start {
        /// Name of the workspace defined in config.toml.
        workspace: String,

        /// Name of the project / feature to create.
        project: String,

        /// Override the AI agent CLI tool (e.g. claude, agy, copilot).
        #[arg(short, long)]
        agent: Option<String>,

        /// Initial task description or prompt to pass to the agent.
        #[arg(short, long)]
        prompt: Option<String>,
    },

    /// Attach to an ongoing project session in the TUI multiplexer.
    Attach {
        /// Project name to attach to.
        project: String,
    },

    /// List all configured workspaces, repositories, and active projects.
    List,

    /// Safely teardown a project by removing its git worktrees and directory.
    Destroy {
        /// Name of the project directory to destroy.
        project: String,

        /// Force deletion even if there are uncommitted git changes.
        #[arg(short, long)]
        force: bool,
    },
}

impl Cli {
    /// Parses CLI arguments from the environment.
    pub fn parse_args() -> Self {
        Self::parse()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cli_default_no_subcommand() {
        let cli = Cli::try_parse_from(["botai"]).expect("Failed to parse default");
        assert!(cli.command.is_none());
        assert!(cli.config.is_none());
    }

    #[test]
    fn test_cli_custom_config_path() {
        let cli = Cli::try_parse_from(["botai", "--config", "/custom/path/config.toml"])
            .expect("Failed to parse custom config");
        assert_eq!(cli.config, Some(PathBuf::from("/custom/path/config.toml")));
        assert!(cli.command.is_none());
    }

    #[test]
    fn test_cli_start_subcommand() {
        let cli = Cli::try_parse_from([
            "botai",
            "start",
            "ecommerce",
            "feature-cart",
            "--agent",
            "agy",
            "--prompt",
            "Implement cart discount logic",
        ])
        .expect("Failed to parse start");

        match cli.command {
            Some(Commands::Start {
                workspace,
                project,
                agent,
                prompt,
            }) => {
                assert_eq!(workspace, "ecommerce");
                assert_eq!(project, "feature-cart");
                assert_eq!(agent, Some("agy".to_string()));
                assert_eq!(prompt, Some("Implement cart discount logic".to_string()));
            }
            _ => panic!("Expected Start command"),
        }
    }

    #[test]
    fn test_cli_attach_subcommand() {
        let cli = Cli::try_parse_from(["botai", "attach", "feature-cart"])
            .expect("Failed to parse attach");

        match cli.command {
            Some(Commands::Attach { project }) => {
                assert_eq!(project, "feature-cart");
            }
            _ => panic!("Expected Attach command"),
        }
    }

    #[test]
    fn test_cli_list_subcommand() {
        let cli = Cli::try_parse_from(["botai", "list"]).expect("Failed to parse list");
        assert_eq!(cli.command, Some(Commands::List));
    }

    #[test]
    fn test_cli_destroy_subcommand() {
        let cli = Cli::try_parse_from(["botai", "destroy", "feature-cart", "--force"])
            .expect("Failed to parse destroy");

        match cli.command {
            Some(Commands::Destroy { project, force }) => {
                assert_eq!(project, "feature-cart");
                assert!(force);
            }
            _ => panic!("Expected Destroy command"),
        }
    }
}
