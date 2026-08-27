use clap::{Parser, Subcommand};
use std::path::PathBuf;

/// botai - Multi-repo workspace orchestrator and session multiplexer for AI agent workflows.
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
    /// Create a new multi-repo workspace project and optionally launch an AI agent session.
    Create {
        /// Name of the project template defined in config.toml.
        template: String,

        /// Name of the project / feature to create.
        project: String,

        /// Override the AI agent CLI tool (e.g. claude, agy, copilot).
        #[arg(short = 'A', long)]
        agent: Option<String>,

        /// Initial task description or prompt to pass to the agent (requires --attach).
        #[arg(short, long, requires = "attach")]
        prompt: Option<String>,

        /// Launch interactive TUI multiplexer immediately after project creation.
        #[arg(short, long)]
        attach: bool,
    },

    /// Synchronize template repositories by cloning missing repos and fetching latest updates.
    Sync {
        /// Optional template name to sync. If omitted, synchronizes all configured templates.
        template: Option<String>,
    },

    /// Attach to an ongoing project session in the TUI multiplexer.
    Attach {
        /// Project name to attach to.
        project: String,
    },

    /// List all configured templates, repositories, and active projects.
    List,

    /// Initialize a fully commented configuration file template.
    Init {
        /// Overwrite destination configuration file if it already exists.
        #[arg(short, long)]
        force: bool,

        /// Output template configuration to stdout instead of writing to disk.
        #[arg(short, long)]
        stdout: bool,
    },

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
    fn test_cli_create_subcommand_defaults() {
        let cli = Cli::try_parse_from(["botai", "create", "ecommerce", "feature-cart"])
            .expect("Failed to parse create with defaults");

        match cli.command {
            Some(Commands::Create {
                template,
                project,
                agent,
                prompt,
                attach,
            }) => {
                assert_eq!(template, "ecommerce");
                assert_eq!(project, "feature-cart");
                assert_eq!(agent, None);
                assert_eq!(prompt, None);
                assert!(!attach);
            }
            _ => panic!("Expected Create command"),
        }
    }

    #[test]
    fn test_cli_create_subcommand_with_attach_long_and_short() {
        let cli_long =
            Cli::try_parse_from(["botai", "create", "ecommerce", "feature-cart", "--attach"])
                .expect("Failed to parse create with --attach");
        match cli_long.command {
            Some(Commands::Create { attach, .. }) => assert!(attach),
            _ => panic!("Expected Create command"),
        }

        let cli_short = Cli::try_parse_from(["botai", "create", "ecommerce", "feature-cart", "-a"])
            .expect("Failed to parse create with -a");
        match cli_short.command {
            Some(Commands::Create { attach, .. }) => assert!(attach),
            _ => panic!("Expected Create command"),
        }
    }

    #[test]
    fn test_cli_create_subcommand_with_agent_flags() {
        let cli_long = Cli::try_parse_from([
            "botai",
            "create",
            "ecommerce",
            "feature-cart",
            "--agent",
            "agy",
        ])
        .expect("Failed to parse create with --agent");
        match cli_long.command {
            Some(Commands::Create { agent, .. }) => {
                assert_eq!(agent, Some("agy".to_string()));
            }
            _ => panic!("Expected Create command"),
        }

        let cli_short = Cli::try_parse_from([
            "botai",
            "create",
            "ecommerce",
            "feature-cart",
            "-A",
            "claude",
        ])
        .expect("Failed to parse create with -A");
        match cli_short.command {
            Some(Commands::Create { agent, .. }) => {
                assert_eq!(agent, Some("claude".to_string()));
            }
            _ => panic!("Expected Create command"),
        }
    }

    #[test]
    fn test_cli_create_subcommand_with_prompt_requires_attach() {
        let cli_full = Cli::try_parse_from([
            "botai",
            "create",
            "ecommerce",
            "feature-cart",
            "--agent",
            "agy",
            "--attach",
            "--prompt",
            "Implement cart discount logic",
        ])
        .expect("Failed to parse create with prompt and attach");

        match cli_full.command {
            Some(Commands::Create {
                template,
                project,
                agent,
                prompt,
                attach,
            }) => {
                assert_eq!(template, "ecommerce");
                assert_eq!(project, "feature-cart");
                assert_eq!(agent, Some("agy".to_string()));
                assert_eq!(prompt, Some("Implement cart discount logic".to_string()));
                assert!(attach);
            }
            _ => panic!("Expected Create command"),
        }

        let cli_short = Cli::try_parse_from([
            "botai",
            "create",
            "ecommerce",
            "feature-cart",
            "-a",
            "-p",
            "Quick task",
        ])
        .expect("Failed to parse create with short prompt and attach flags");

        match cli_short.command {
            Some(Commands::Create { prompt, attach, .. }) => {
                assert_eq!(prompt, Some("Quick task".to_string()));
                assert!(attach);
            }
            _ => panic!("Expected Create command"),
        }

        // Fails when --prompt is specified without --attach
        let err_long = Cli::try_parse_from([
            "botai",
            "create",
            "ecommerce",
            "feature-cart",
            "--prompt",
            "Missing attach flag",
        ]);
        assert!(
            err_long.is_err(),
            "--prompt without --attach must fail validation"
        );

        let err_short = Cli::try_parse_from([
            "botai",
            "create",
            "ecommerce",
            "feature-cart",
            "-p",
            "Missing attach flag",
        ]);
        assert!(
            err_short.is_err(),
            "-p without -a/--attach must fail validation"
        );
    }

    #[test]
    fn test_cli_start_subcommand_is_completely_removed() {
        let err = Cli::try_parse_from(["botai", "start", "ecommerce", "feature-cart"]);
        assert!(
            err.is_err(),
            "botai start must not be recognized as a valid subcommand"
        );
    }

    #[test]
    fn test_cli_sync_subcommand() {
        let cli_all = Cli::try_parse_from(["botai", "sync"]).expect("Failed to parse sync all");
        match cli_all.command {
            Some(Commands::Sync { template }) => {
                assert_eq!(template, None);
            }
            _ => panic!("Expected Sync command"),
        }

        let cli_tpl = Cli::try_parse_from(["botai", "sync", "ecommerce"])
            .expect("Failed to parse sync template");
        match cli_tpl.command {
            Some(Commands::Sync { template }) => {
                assert_eq!(template, Some("ecommerce".to_string()));
            }
            _ => panic!("Expected Sync command with template"),
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
    fn test_cli_init_subcommand_defaults() {
        let cli = Cli::try_parse_from(["botai", "init"]).expect("Failed to parse init");
        match cli.command {
            Some(Commands::Init { force, stdout }) => {
                assert!(!force);
                assert!(!stdout);
            }
            _ => panic!("Expected Init command"),
        }
    }

    #[test]
    fn test_cli_init_subcommand_flags() {
        let cli = Cli::try_parse_from(["botai", "init", "--force", "--stdout"])
            .expect("Failed to parse init with long flags");
        match cli.command {
            Some(Commands::Init { force, stdout }) => {
                assert!(force);
                assert!(stdout);
            }
            _ => panic!("Expected Init command"),
        }

        let cli_short = Cli::try_parse_from(["botai", "init", "-f", "-s"])
            .expect("Failed to parse init with short flags");
        match cli_short.command {
            Some(Commands::Init { force, stdout }) => {
                assert!(force);
                assert!(stdout);
            }
            _ => panic!("Expected Init command"),
        }
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
