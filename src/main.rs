use anyhow::{bail, Context, Result};
use std::env;
use std::fs;
use std::io;
use std::path::Path;

use botai::cli::{Cli, Commands};
use botai::config::Config;
use botai::context::{generate_agents_md, ProjectContext, RepoMapping};
use botai::git::{create_worktree, has_uncommitted_changes, remove_worktree};
use botai::session::{ProjectSession, PtyConfig, PtySession, SessionRegistry, VtScreen};
use botai::tui::{DashboardState, TuiApp};

fn main() -> Result<()> {
    let cli = Cli::parse_args();

    if let Some(Commands::Init { force, stdout }) = cli.command {
        return cmd_init(cli.config.as_deref(), force, stdout);
    }

    let config_path = match cli.config {
        Some(path) => path,
        None => Config::default_path().context(
            "Could not determine default configuration directory. Please specify --config <FILE>.",
        )?,
    };

    let config = if config_path.exists() {
        Config::load_from_file(&config_path)
            .with_context(|| format!("Failed to load configuration from {:?}", config_path))?
    } else {
        Config::load_from_str("default_agent = \"claude\"\nrepos_dir = \".botai_repos\"\n")
            .context("Failed to load fallback configuration")?
    };

    match cli.command {
        None => run_default_dashboard(&config),
        Some(Commands::Init { .. }) => unreachable!(),
        Some(Commands::Start {
            workspace,
            project,
            agent,
            prompt,
        }) => cmd_start(
            &config,
            &workspace,
            &project,
            agent.as_deref(),
            prompt.as_deref(),
        ),
        Some(Commands::Sync { workspace }) => cmd_sync(&config, workspace.as_deref()),
        Some(Commands::Attach { project }) => cmd_attach(&config, &project),
        Some(Commands::List) => cmd_list(&config),
        Some(Commands::Destroy { project, force }) => cmd_destroy(&project, force),
    }
}

/// `botai init [--force] [--stdout]`
fn cmd_init(config_override: Option<&Path>, force: bool, stdout: bool) -> Result<()> {
    if stdout {
        print!("{}", Config::default_template());
        return Ok(());
    }

    let target_path = match config_override {
        Some(path) => path.to_path_buf(),
        None => Config::default_path().context(
            "Could not determine default configuration directory. Please specify --config <FILE>.",
        )?,
    };

    Config::init_file(&target_path, force)
        .with_context(|| format!("Failed to initialize configuration at {:?}", target_path))?;

    println!("Initialized botai configuration at: {:?}", target_path);
    Ok(())
}

/// `botai sync [workspace]`
fn cmd_sync(config: &Config, workspace_filter: Option<&str>) -> Result<()> {
    let repos_dir = config.resolved_repos_dir();
    println!("=== botai Repository Sync ===");
    println!("Central Repos Directory: {:?}", repos_dir);

    let workspaces_to_sync: Vec<&str> = match workspace_filter {
        Some(ws_name) => {
            if !config.workspaces.contains_key(ws_name) {
                bail!("Workspace '{}' not found in configuration", ws_name);
            }
            vec![ws_name]
        }
        None => {
            let mut keys: Vec<&str> = config.workspaces.keys().map(|s| s.as_str()).collect();
            keys.sort();
            keys
        }
    };

    if workspaces_to_sync.is_empty() {
        println!("No workspaces configured to sync.");
        return Ok(());
    }

    for ws_name in workspaces_to_sync {
        println!("\n--- Workspace: {} ---", ws_name);
        let resolved = config.resolve_workspace_repos(ws_name)?;
        for repo in resolved {
            if !repo.exists_locally() {
                if let Some(url) = &repo.clone_url {
                    println!("  [clone] Cloning '{}' from {}...", repo.name, url);
                    botai::git::clone_repo(url, &repo.local_path).with_context(|| {
                        format!("Failed to clone repository '{}' from '{}'", repo.name, url)
                    })?;
                    println!("  ✓ Cloned {} into {:?}", repo.name, repo.local_path);
                } else {
                    eprintln!(
                        "  ✗ Local repository not found at {:?} (no remote clone URL configured)",
                        repo.local_path
                    );
                }
            } else {
                print!("  [fetch] Fetching updates for '{}'...", repo.name);
                if let Err(e) = botai::git::fetch_repo(&repo.local_path) {
                    println!(" (fetch failed: {}, using cached branch)", e);
                } else {
                    println!(" ✓ Updated");
                }
            }
        }
    }

    println!("\nSynchronization completed successfully.");
    Ok(())
}

/// Default action: launches interactive TUI dashboard / multiplexer.
fn run_default_dashboard(config: &Config) -> Result<()> {
    let cwd = env::current_dir().context("Failed to get current working directory")?;
    let dashboard = DashboardState::from_env(config, &cwd);
    let registry = SessionRegistry::new();
    let mut app = TuiApp::new(registry)
        .with_dashboard(dashboard)
        .with_config(config.clone())
        .with_cwd(cwd);
    let mut stdout = io::stdout();
    app.run(&mut stdout)?;
    Ok(())
}

/// `botai start <workspace> <project> [--agent <agent>] [--prompt <prompt>]`
fn cmd_start(
    config: &Config,
    workspace_name: &str,
    project_name: &str,
    agent_override: Option<&str>,
    prompt: Option<&str>,
) -> Result<()> {
    let ws = config
        .get_workspace(workspace_name)
        .with_context(|| format!("Workspace '{workspace_name}' not found in configuration"))?;

    let agent_name = match agent_override {
        Some(name) => name,
        None => ws.default_agent.as_deref().unwrap_or(&config.default_agent),
    };

    let agent = config
        .get_agent(agent_name)
        .with_context(|| format!("Agent '{agent_name}' not found in configuration"))?;

    let resolved_repos = config
        .resolve_workspace_repos(workspace_name)
        .with_context(|| format!("Failed to resolve repositories for workspace '{workspace_name}'"))?;

    // Auto-clone any missing repositories and fetch updates best-effort
    for repo in &resolved_repos {
        if !repo.exists_locally() {
            if let Some(url) = &repo.clone_url {
                println!(
                    "Repository '{}' is not cached locally. Auto-cloning from {}...",
                    repo.name, url
                );
                botai::git::clone_repo(url, &repo.local_path).with_context(|| {
                    format!("Failed to clone repository '{}' from '{}'", repo.name, url)
                })?;
                println!("✓ Cloned into {:?}", repo.local_path);
            } else {
                bail!(
                    "Local repository '{}' not found at {:?}. Please verify your configuration.",
                    repo.name,
                    repo.local_path
                );
            }
        } else {
            // Best-effort fetch
            let _ = botai::git::fetch_repo(&repo.local_path);
        }
    }

    let cwd = env::current_dir().context("Failed to get current working directory")?;
    let project_dir = cwd.join(project_name);

    if project_dir.exists() {
        bail!(
            "Project directory '{:?}' already exists. Please choose a different project name or remove it.",
            project_dir
        );
    }

    fs::create_dir_all(&project_dir)
        .with_context(|| format!("Failed to create project directory {:?}", project_dir))?;

    let project_dir = project_dir
        .canonicalize()
        .unwrap_or(project_dir);

    let mut repo_mappings = Vec::new();

    // Create worktree for each resolved repo in the workspace
    for repo in &resolved_repos {
        let worktree_path = project_dir.join(&repo.name);

        println!(
            "Creating worktree for repo '{}' at {:?}",
            repo.name, worktree_path
        );

        if let Err(e) = create_worktree(&repo.local_path, &worktree_path, project_name) {
            // Rollback on failure
            let _ = fs::remove_dir_all(&project_dir);
            return Err(e).with_context(|| {
                format!(
                    "Failed to create git worktree for {:?} checked out to branch '{}'",
                    repo.local_path, project_name
                )
            });
        }

        repo_mappings.push(RepoMapping::new(
            repo.name.clone(),
            repo.local_path.clone(),
            worktree_path,
        ));
    }

    // Synthesize and write AGENTS.md
    let project_context = ProjectContext {
        project_name: project_name.to_string(),
        workspace_name: workspace_name.to_string(),
        description: ws.description.clone(),
        repos: repo_mappings,
        env: ws.env.clone(),
        initial_prompt: prompt.map(|s| s.to_string()),
    };

    let agents_md_path = generate_agents_md(&project_dir, &project_context)
        .context("Failed to generate AGENTS.md context file")?;
    println!("Generated context orientation file at {:?}", agents_md_path);

    // Build PTY configuration and spawn agent process
    let mut pty_config = PtyConfig::new(&agent.command, &project_dir)
        .with_args(agent.args.clone())
        .with_dimensions(100, 30);

    for (k, v) in &agent.env {
        pty_config = pty_config.with_env(k, v);
    }
    for (k, v) in &ws.env {
        pty_config = pty_config.with_env(k, v);
    }

    let pty = PtySession::spawn(pty_config)
        .with_context(|| format!("Failed to spawn agent command '{}'", agent.command))?;
    let vt = VtScreen::new(100, 30);
    let mut ps = ProjectSession::new(project_name, workspace_name, agent_name);
    if let Some(p) = prompt {
        ps = ps.with_prompt(p);
    }

    let mut registry = SessionRegistry::new();
    registry.add(ps, pty, vt)?;

    let mut app = TuiApp::new(registry);
    let mut stdout = io::stdout();
    app.run(&mut stdout)?;

    Ok(())
}

/// `botai attach <project>`
fn cmd_attach(config: &Config, project_name: &str) -> Result<()> {
    let cwd = env::current_dir().context("Failed to get current working directory")?;
    let project_dir = cwd.join(project_name);

    if !project_dir.exists() {
        bail!("Project directory '{:?}' does not exist.", project_dir);
    }

    let project_dir = project_dir
        .canonicalize()
        .unwrap_or(project_dir);

    let agent = config
        .get_agent(&config.default_agent)
        .with_context(|| format!("Default agent '{}' not found", config.default_agent))?;

    let pty_config = PtyConfig::new(&agent.command, &project_dir)
        .with_args(agent.args.clone())
        .with_dimensions(100, 30);

    let pty = PtySession::spawn(pty_config)?;
    let vt = VtScreen::new(100, 30);
    let ps = ProjectSession::new(project_name, "attached", &config.default_agent);

    let mut registry = SessionRegistry::new();
    registry.add(ps, pty, vt)?;

    let mut app = TuiApp::new(registry);
    let mut stdout = io::stdout();
    app.run(&mut stdout)?;

    Ok(())
}

/// `botai list`
fn cmd_list(config: &Config) -> Result<()> {
    println!("=== botai Configuration ===");
    println!("Default Agent: {}", config.default_agent);
    println!("Central Repos Directory: {:?}", config.resolved_repos_dir());

    println!("\n=== Configured Workspaces ===");
    if config.workspaces.is_empty() {
        println!("  (None configured)");
    } else {
        for (name, ws) in &config.workspaces {
            println!("- Workspace: {}", name);
            if let Some(desc) = &ws.description {
                println!("  Description: {}", desc);
            }
            if let Some(agent) = &ws.default_agent {
                println!("  Default Agent: {}", agent);
            }
            println!("  Repositories:");
            match config.resolve_workspace_repos(name) {
                Ok(resolved) => {
                    for repo in resolved {
                        let status = if repo.exists_locally() {
                            if repo.clone_url.is_some() {
                                "[cached in repos_dir]"
                            } else {
                                "[local path]"
                            }
                        } else if repo.clone_url.is_some() {
                            "[missing - will auto-clone]"
                        } else {
                            "[missing local path]"
                        };
                        println!("    • {} -> {:?} {}", repo.name, repo.local_path, status);
                    }
                }
                Err(e) => {
                    println!("    (Configuration error: {})", e);
                }
            }
        }
    }

    println!("\n=== Configured Agents ===");
    println!("Default Agent: {}", config.default_agent);
    for (name, agent) in &config.agents {
        println!("- {}: {} {:?}", name, agent.command, agent.args);
    }

    println!("\n=== Local Projects in CWD ===");
    let cwd = env::current_dir()?;
    let mut found_projects = 0;
    if let Ok(entries) = fs::read_dir(&cwd) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && path.join("AGENTS.md").exists() {
                found_projects += 1;
                println!("- Project: {:?}", path.file_name().unwrap());
            }
        }
    }
    if found_projects == 0 {
        println!("  (No active project directories found in current directory)");
    }

    Ok(())
}

/// `botai destroy <project> [--force]`
fn cmd_destroy(project_name: &str, force: bool) -> Result<()> {
    let cwd = env::current_dir()?;
    let project_dir = cwd.join(project_name);

    if !project_dir.exists() {
        bail!("Project directory {:?} does not exist.", project_dir);
    }

    // Inspect child directories for git worktrees
    if let Ok(entries) = fs::read_dir(&project_dir) {
        for entry in entries.flatten() {
            let child_path = entry.path();
            if child_path.is_dir() && child_path.join(".git").exists() {
                if !force && has_uncommitted_changes(&child_path)? {
                    bail!(
                        "Worktree {:?} has uncommitted changes. Use --force to destroy anyway.",
                        child_path
                    );
                }
                // Attempt removal
                let _ = remove_worktree(Path::new("."), &child_path, force);
            }
        }
    }

    fs::remove_dir_all(&project_dir)
        .with_context(|| format!("Failed to remove project directory {:?}", project_dir))?;

    println!("Successfully destroyed project '{project_name}' and cleaned up worktrees.");
    Ok(())
}
