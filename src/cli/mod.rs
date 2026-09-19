use clap::{Parser, Subcommand};
use std::path::PathBuf;

use crate::config::ResolvedRepo;

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

        /// Explicitly include repositories (comma-separated or repeatable).
        #[arg(long, value_delimiter = ',')]
        include: Option<Vec<String>>,

        /// Explicitly exclude repositories (comma-separated or repeatable).
        #[arg(long, value_delimiter = ',')]
        exclude: Option<Vec<String>>,

        /// Include all repositories defined in the template regardless of default settings.
        #[arg(long)]
        all: bool,

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

    /// Open the configuration file in the user's editor of choice.
    Edit {
        /// Override the editor executable or command (e.g. 'nano', 'vim', 'code --wait').
        #[arg(short, long)]
        editor: Option<String>,
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

/// Resolves the effective repository names selected via CLI flags (`--include`, `--exclude`, `--all`)
/// against the repositories declared in a template.
///
/// # Validation & Precedence Rules
/// 1. A repository cannot appear in both `include` and `exclude`.
/// 2. Every repository specified in `include` or `exclude` must exist in `all_template_repos`.
/// 3. If `all` is true, begins with all repositories defined in the template. Otherwise, begins with
///    repositories where `default_include` is true.
/// 4. Repositories in `include` are added (deduplicated).
/// 5. Repositories in `exclude` are removed.
/// 6. At least one repository must remain selected; otherwise an error is returned.
pub fn resolve_cli_selected_repos(
    all_template_repos: &[ResolvedRepo],
    include: Option<&[String]>,
    exclude: Option<&[String]>,
    all: bool,
) -> anyhow::Result<Vec<String>> {
    // 1. Conflict check: cannot both include and exclude the same repo
    if let (Some(inc), Some(exc)) = (include, exclude) {
        for repo in inc {
            if exc.iter().any(|e| e == repo) {
                anyhow::bail!("Cannot both include and exclude repository '{repo}'");
            }
        }
    }

    // 2. Existence check: all repos in include/exclude must exist in template
    let available_names = all_template_repos
        .iter()
        .map(|r| r.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");

    if let Some(inc) = include {
        for repo in inc {
            if !all_template_repos.iter().any(|r| &r.name == repo) {
                anyhow::bail!(
                    "Repository '{repo}' not found in template. Available repositories: {available_names}"
                );
            }
        }
    }

    if let Some(exc) = exclude {
        for repo in exc {
            if !all_template_repos.iter().any(|r| &r.name == repo) {
                anyhow::bail!(
                    "Repository '{repo}' not found in template. Available repositories: {available_names}"
                );
            }
        }
    }

    // 3. Selection calculation
    let mut selected_names: Vec<String> = if all {
        all_template_repos.iter().map(|r| r.name.clone()).collect()
    } else {
        all_template_repos
            .iter()
            .filter(|r| r.default_include)
            .map(|r| r.name.clone())
            .collect()
    };

    if let Some(inc) = include {
        for repo in inc {
            if !selected_names.iter().any(|s| s == repo) {
                selected_names.push(repo.clone());
            }
        }
    }

    if let Some(exc) = exclude {
        selected_names.retain(|s| !exc.iter().any(|e| e == s));
    }

    // 4. Non-empty check
    if selected_names.is_empty() {
        anyhow::bail!("Cannot create project: at least one repository must be selected.");
    }

    Ok(selected_names)
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
                include,
                exclude,
                all,
            }) => {
                assert_eq!(template, "ecommerce");
                assert_eq!(project, "feature-cart");
                assert_eq!(agent, None);
                assert_eq!(prompt, None);
                assert!(!attach);
                assert_eq!(include, None);
                assert_eq!(exclude, None);
                assert!(!all);
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
                ..
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

    #[test]
    fn test_cli_edit_subcommand_defaults() {
        let cli = Cli::try_parse_from(["botai", "edit"]).expect("Failed to parse edit defaults");
        match cli.command {
            Some(Commands::Edit { editor }) => {
                assert_eq!(editor, None);
            }
            _ => panic!("Expected Edit command"),
        }
    }

    #[test]
    fn test_cli_edit_subcommand_with_long_editor_flag() {
        let cli = Cli::try_parse_from(["botai", "edit", "--editor", "nvim"])
            .expect("Failed to parse edit with --editor");
        match cli.command {
            Some(Commands::Edit { editor }) => {
                assert_eq!(editor, Some("nvim".to_string()));
            }
            _ => panic!("Expected Edit command with nvim"),
        }
    }

    #[test]
    fn test_cli_edit_subcommand_with_short_editor_flag() {
        let cli = Cli::try_parse_from(["botai", "edit", "-e", "code --wait"])
            .expect("Failed to parse edit with -e");
        match cli.command {
            Some(Commands::Edit { editor }) => {
                assert_eq!(editor, Some("code --wait".to_string()));
            }
            _ => panic!("Expected Edit command with code --wait"),
        }
    }

    #[test]
    fn test_cli_edit_subcommand_with_global_config_flag() {
        let cli = Cli::try_parse_from([
            "botai",
            "--config",
            "/custom/path/config.toml",
            "edit",
            "-e",
            "vim",
        ])
        .expect("Failed to parse global config with edit");

        assert_eq!(cli.config, Some(PathBuf::from("/custom/path/config.toml")));
        match cli.command {
            Some(Commands::Edit { editor }) => {
                assert_eq!(editor, Some("vim".to_string()));
            }
            _ => panic!("Expected Edit command with vim"),
        }
    }

    fn mock_resolved_repo(name: &str, default_include: bool) -> ResolvedRepo {
        ResolvedRepo {
            name: name.to_string(),
            local_path: PathBuf::from(format!("/mock/{name}")),
            clone_url: None,
            original_entry: name.to_string(),
            setup_hooks: Vec::new(),
            env: std::collections::HashMap::new(),
            default_include,
        }
    }

    #[test]
    fn test_cli_create_subcommand_with_include_exclude_all_flags() {
        // Comma-separated values and --all flag
        let cli_comma = Cli::try_parse_from([
            "botai",
            "create",
            "ecommerce",
            "feat-cart",
            "--include",
            "repo1,repo2",
            "--exclude",
            "repo3,repo4",
            "--all",
        ])
        .expect("Failed to parse comma-separated include and exclude");

        match cli_comma.command {
            Some(Commands::Create {
                include,
                exclude,
                all,
                ..
            }) => {
                assert_eq!(
                    include,
                    Some(vec!["repo1".to_string(), "repo2".to_string()])
                );
                assert_eq!(
                    exclude,
                    Some(vec!["repo3".to_string(), "repo4".to_string()])
                );
                assert!(all);
            }
            _ => panic!("Expected Create command"),
        }

        // Multiple repeatable flags
        let cli_repeat = Cli::try_parse_from([
            "botai",
            "create",
            "ecommerce",
            "feat-cart",
            "--include",
            "repo1",
            "--include",
            "repo2",
            "--exclude",
            "repo3",
            "--exclude",
            "repo4",
        ])
        .expect("Failed to parse repeatable include and exclude");

        match cli_repeat.command {
            Some(Commands::Create {
                include,
                exclude,
                all,
                ..
            }) => {
                assert_eq!(
                    include,
                    Some(vec!["repo1".to_string(), "repo2".to_string()])
                );
                assert_eq!(
                    exclude,
                    Some(vec!["repo3".to_string(), "repo4".to_string()])
                );
                assert!(!all);
            }
            _ => panic!("Expected Create command"),
        }

        // Mixed comma-separated and repeatable flags
        let cli_mixed = Cli::try_parse_from([
            "botai",
            "create",
            "ecommerce",
            "feat-cart",
            "--include",
            "repo1,repo2",
            "--include",
            "repo3",
            "--exclude",
            "repo4,repo5",
            "--exclude",
            "repo6",
        ])
        .expect("Failed to parse mixed include and exclude");

        match cli_mixed.command {
            Some(Commands::Create {
                include, exclude, ..
            }) => {
                assert_eq!(
                    include,
                    Some(vec![
                        "repo1".to_string(),
                        "repo2".to_string(),
                        "repo3".to_string()
                    ])
                );
                assert_eq!(
                    exclude,
                    Some(vec![
                        "repo4".to_string(),
                        "repo5".to_string(),
                        "repo6".to_string()
                    ])
                );
            }
            _ => panic!("Expected Create command"),
        }
    }

    #[test]
    fn test_resolve_cli_selected_repos_defaults() {
        let repos = vec![
            mock_resolved_repo("repo1", true),
            mock_resolved_repo("repo2", true),
            mock_resolved_repo("repo3", false),
        ];

        let selected = resolve_cli_selected_repos(&repos, None, None, false)
            .expect("default selection should succeed");
        assert_eq!(selected, vec!["repo1", "repo2"]);
    }

    #[test]
    fn test_resolve_cli_selected_repos_all() {
        let repos = vec![
            mock_resolved_repo("repo1", true),
            mock_resolved_repo("repo2", true),
            mock_resolved_repo("repo3", false),
        ];

        let selected = resolve_cli_selected_repos(&repos, None, None, true)
            .expect("--all selection should succeed");
        assert_eq!(selected, vec!["repo1", "repo2", "repo3"]);

        // --all combined with --exclude
        let selected_with_exclude =
            resolve_cli_selected_repos(&repos, None, Some(&["repo2".to_string()]), true)
                .expect("--all with --exclude should succeed");
        assert_eq!(selected_with_exclude, vec!["repo1", "repo3"]);
    }

    #[test]
    fn test_resolve_cli_selected_repos_include_adding_excluded() {
        let repos = vec![
            mock_resolved_repo("repo1", true),
            mock_resolved_repo("repo2", true),
            mock_resolved_repo("repo3", false),
        ];

        let selected =
            resolve_cli_selected_repos(&repos, Some(&["repo3".to_string()]), None, false)
                .expect("including default-false repo should succeed");
        assert_eq!(selected, vec!["repo1", "repo2", "repo3"]);

        // Deduplication when including an already default-included repo
        let selected_dedup = resolve_cli_selected_repos(
            &repos,
            Some(&["repo1".to_string(), "repo3".to_string()]),
            None,
            false,
        )
        .expect("deduplicated include should succeed");
        assert_eq!(selected_dedup, vec!["repo1", "repo2", "repo3"]);
    }

    #[test]
    fn test_resolve_cli_selected_repos_exclude_removing_default() {
        let repos = vec![
            mock_resolved_repo("repo1", true),
            mock_resolved_repo("repo2", true),
            mock_resolved_repo("repo3", false),
        ];

        let selected =
            resolve_cli_selected_repos(&repos, None, Some(&["repo1".to_string()]), false)
                .expect("excluding default repo should succeed");
        assert_eq!(selected, vec!["repo2"]);
    }

    #[test]
    fn test_resolve_cli_selected_repos_conflict_between_include_and_exclude() {
        let repos = vec![
            mock_resolved_repo("repo1", true),
            mock_resolved_repo("repo2", true),
            mock_resolved_repo("repo3", false),
        ];

        let err = resolve_cli_selected_repos(
            &repos,
            Some(&["repo1".to_string()]),
            Some(&["repo1".to_string()]),
            false,
        )
        .unwrap_err();

        assert_eq!(
            err.to_string(),
            "Cannot both include and exclude repository 'repo1'"
        );
    }

    #[test]
    fn test_resolve_cli_selected_repos_unknown_repo() {
        let repos = vec![
            mock_resolved_repo("repo1", true),
            mock_resolved_repo("repo2", true),
            mock_resolved_repo("repo3", false),
        ];

        let err_inc = resolve_cli_selected_repos(&repos, Some(&["ghost".to_string()]), None, false)
            .unwrap_err();
        assert_eq!(
            err_inc.to_string(),
            "Repository 'ghost' not found in template. Available repositories: repo1, repo2, repo3"
        );

        let err_exc =
            resolve_cli_selected_repos(&repos, None, Some(&["phantom".to_string()]), false)
                .unwrap_err();
        assert_eq!(
            err_exc.to_string(),
            "Repository 'phantom' not found in template. Available repositories: repo1, repo2, repo3"
        );
    }

    #[test]
    fn test_resolve_cli_selected_repos_empty_selection_error() {
        let repos = vec![
            mock_resolved_repo("repo1", true),
            mock_resolved_repo("repo2", true),
            mock_resolved_repo("repo3", false),
        ];

        // Excluding all default repositories results in empty selection
        let err = resolve_cli_selected_repos(
            &repos,
            None,
            Some(&["repo1".to_string(), "repo2".to_string()]),
            false,
        )
        .unwrap_err();
        assert_eq!(
            err.to_string(),
            "Cannot create project: at least one repository must be selected."
        );

        // Template containing only default_include=false repositories without --include or --all
        let repos_only_false = vec![mock_resolved_repo("repo3", false)];
        let err_no_defaults =
            resolve_cli_selected_repos(&repos_only_false, None, None, false).unwrap_err();
        assert_eq!(
            err_no_defaults.to_string(),
            "Cannot create project: at least one repository must be selected."
        );
    }
}
