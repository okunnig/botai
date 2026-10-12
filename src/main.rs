#![forbid(unsafe_code)]

use anyhow::{bail, Context, Result};
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use botai::cli::{resolve_cli_selected_repos, Cli, Commands};
use botai::config::Config;
use botai::project::{ProjectBuilder, ProjectSetupEvent};
use botai::session::{ProjectSession, PtyConfig, PtySession, SessionRegistry, VtScreen};
use botai::tui::{DashboardState, TuiApp};

fn main() -> Result<()> {
    botai::tui::install_panic_hook();
    let cli = Cli::parse_args();

    if let Some(Commands::Init { force, stdout }) = cli.command {
        return cmd_init(cli.config.as_deref(), force, stdout);
    }

    if let Some(Commands::Edit { editor }) = cli.command {
        return cmd_edit(cli.config.as_deref(), editor.as_deref());
    }

    let config_path = match cli.config {
        Some(path) => path,
        None => Config::default_path().context(
            "Could not determine default configuration directory. Please specify --config <FILE>.",
        )?,
    };

    let config = if config_path.exists() {
        Config::load_from_file(&config_path)
            .with_context(|| format!("Failed to load configuration from {config_path:?}"))?
    } else {
        Config::fallback()
    };

    match cli.command {
        None => run_default_dashboard(&config, &config_path),
        Some(Commands::Init { .. }) | Some(Commands::Edit { .. }) => unreachable!(),
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
            let all_repos = config.resolve_template_repos(&template)?;
            let selected_repos = resolve_cli_selected_repos(
                &all_repos,
                include.as_deref(),
                exclude.as_deref(),
                all,
            )?;
            cmd_create(
                &config,
                &template,
                &project,
                agent.as_deref(),
                prompt.as_deref(),
                attach,
                selected_repos,
            )
        }
        Some(Commands::Sync { template }) => cmd_sync(&config, template.as_deref()),
        Some(Commands::Attach { project }) => cmd_attach(&config, &project),
        Some(Commands::List) => cmd_list(&config),
        Some(Commands::Destroy { project, force }) => cmd_destroy(Some(&config), &project, force),
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
        .with_context(|| format!("Failed to initialize configuration at {target_path:?}"))?;

    println!("Initialized botai configuration at: {target_path:?}");
    Ok(())
}

/// `botai edit [--editor <cmd>]`
fn cmd_edit(config_override: Option<&Path>, editor_override: Option<&str>) -> Result<()> {
    let target_path = match config_override {
        Some(path) => path.to_path_buf(),
        None => Config::default_path().context(
            "Could not determine default configuration directory. Please specify --config <FILE>.",
        )?,
    };

    if !target_path.exists() {
        Config::init_file(&target_path, false)
            .with_context(|| format!("Failed to initialize configuration at {target_path:?}"))?;
    }

    let editor_cmd = botai::config::resolve_editor(editor_override)?;
    let status = botai::config::run_editor(&editor_cmd, &target_path)?;

    let exit_code = status
        .code()
        .unwrap_or(if status.success() { 0 } else { 1 });
    if exit_code != 0 {
        std::process::exit(exit_code);
    }

    Ok(())
}

/// `botai sync [template]`
fn cmd_sync(config: &Config, template_filter: Option<&str>) -> Result<()> {
    let repos_dir = config.resolved_repos_dir();
    println!("=== botai Repository Sync ===");
    println!("Central Repos Directory: {repos_dir:?}");

    let templates_to_sync: Vec<&str> = match template_filter {
        Some(tpl_name) => {
            if !config.templates.contains_key(tpl_name) {
                bail!("Template '{tpl_name}' not found in configuration");
            }
            vec![tpl_name]
        }
        None => {
            let mut keys: Vec<&str> = config.templates.keys().map(|s| s.as_str()).collect();
            keys.sort();
            keys
        }
    };

    if templates_to_sync.is_empty() {
        println!("No templates configured to sync.");
        return Ok(());
    }

    for tpl_name in templates_to_sync {
        println!("\n--- Template: {tpl_name} ---");
        let resolved = config.resolve_template_repos(tpl_name)?;
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
                    println!(" (fetch failed: {e}, using cached branch)");
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
fn run_default_dashboard(config: &Config, config_path: &Path) -> Result<()> {
    let cwd = env::current_dir().context("Failed to get current working directory")?;
    let dashboard = DashboardState::from_env(config, &cwd).with_config_path(config_path);
    let registry = SessionRegistry::new();
    let mut app = TuiApp::new(registry)
        .with_dashboard(dashboard)
        .with_config(config.clone())
        .with_config_path(config_path)
        .with_cwd(cwd);
    let mut stdout = io::stdout();
    app.run(&mut stdout)?;
    Ok(())
}

/// `botai create <template> <project> [--agent <agent>] [--prompt <prompt>] [--include <repos>] [--exclude <repos>] [--all] [--attach]`
fn cmd_create(
    config: &Config,
    template_name: &str,
    project_name: &str,
    agent_override: Option<&str>,
    prompt: Option<&str>,
    attach: bool,
    selected_repos: Vec<String>,
) -> Result<()> {
    let cwd = env::current_dir().context("Failed to get current working directory")?;
    let (term_cols, term_rows) = crossterm::terminal::size()
        .map(|(c, r)| (c, r.saturating_sub(1).max(1)))
        .unwrap_or((100, 30));

    let mut builder = ProjectBuilder::new(config.clone(), template_name, project_name)
        .with_cwd(cwd.clone())
        .with_viewport(term_cols, term_rows)
        .with_selected_repos(selected_repos);

    if let Some(agent) = agent_override {
        builder = builder.with_agent(agent);
    }
    if let Some(p) = prompt {
        builder = builder.with_prompt(p);
    }

    if !attach {
        let progress_sink = |event: ProjectSetupEvent| match event {
            ProjectSetupEvent::Log(msg) => {
                println!("{msg}");
            }
            ProjectSetupEvent::Error(err) => {
                eprintln!("✗ Error: {err}");
            }
            _ => {}
        };

        let (project_dir, _) = builder
            .create_workspace(progress_sink)
            .with_context(|| format!("Failed to create project '{project_name}'"))?;

        println!(
            "✓ Project '{project_name}' created successfully at {}!",
            project_dir.display()
        );
        println!(
            "Hint: Run 'botai attach {project_name}' to launch the agent session or run 'botai' to open the interactive dashboard."
        );
        return Ok(());
    }

    // Stream step and log progress directly to stdout
    let progress_sink = |event: ProjectSetupEvent| match event {
        ProjectSetupEvent::Step {
            current,
            total,
            message,
        } => {
            println!("[{current}/{total}] {message}");
        }
        ProjectSetupEvent::Log(msg) => {
            println!("{msg}");
        }
        ProjectSetupEvent::Success => {
            println!("✓ Project setup completed successfully!");
        }
        ProjectSetupEvent::Error(err) => {
            eprintln!("✗ Error: {err}");
        }
    };

    let (ps, pty, vt) = builder
        .build(progress_sink)
        .with_context(|| format!("Failed to create project '{project_name}'"))?;

    run_tui(config, cwd, ps, pty, vt)
}

/// `botai attach <project>`
fn cmd_attach(config: &Config, project_name: &str) -> Result<()> {
    let cwd = env::current_dir().context("Failed to get current working directory")?;
    let project_dir = cwd.join(project_name);

    if !project_dir.exists() {
        bail!("Project directory '{project_dir:?}' does not exist.");
    }

    let project_dir = dunce::canonicalize(&project_dir).unwrap_or(project_dir);

    let template_name = botai::project::extract_template_name_from_agents_md(&project_dir);
    let agent_name = template_name
        .as_deref()
        .and_then(|t| config.get_template(t))
        .and_then(|t| t.default_agent.as_deref())
        .unwrap_or(&config.default_agent);

    let agent = config
        .get_agent(agent_name)
        .with_context(|| format!("Agent '{agent_name}' not found"))?;

    let (term_cols, term_rows) = crossterm::terminal::size()
        .map(|(c, r)| (c, r.saturating_sub(1).max(1)))
        .unwrap_or((100, 30));

    let mut pty_config = PtyConfig::new(&agent.command, &project_dir)
        .with_args(agent.args.clone())
        .with_dimensions(term_cols, term_rows);

    for (k, v) in &agent.env {
        pty_config = pty_config.with_env(k, v);
    }
    if let Some(tpl) = template_name
        .as_deref()
        .and_then(|t| config.get_template(t))
    {
        for (k, v) in &tpl.env {
            pty_config = pty_config.with_env(k, v);
        }
    }

    let pty = PtySession::spawn(pty_config)?;
    let vt = VtScreen::new(term_cols, term_rows);
    let ps = ProjectSession::new(
        project_name,
        template_name.as_deref().unwrap_or("attached"),
        agent_name,
    );

    run_tui(config, cwd, ps, pty, vt)
}

/// Registers a single session and runs the TUI multiplexer until exit.
fn run_tui(
    config: &Config,
    cwd: PathBuf,
    ps: ProjectSession,
    pty: PtySession,
    vt: VtScreen,
) -> Result<()> {
    let mut registry = SessionRegistry::new();
    registry.add(ps, pty, vt)?;

    let mut app = TuiApp::new(registry)
        .with_config(config.clone())
        .with_cwd(cwd);
    let mut stdout = io::stdout();
    app.run(&mut stdout)?;

    Ok(())
}

/// `botai list`
fn cmd_list(config: &Config) -> Result<()> {
    println!("=== botai Configuration ===");
    println!("Default Agent: {}", config.default_agent);
    println!("Central Repos Directory: {:?}", config.resolved_repos_dir());

    println!("\n=== Configured Templates ===");
    if config.templates.is_empty() {
        println!("  (None configured)");
    } else {
        for (name, tpl) in &config.templates {
            println!("- Template: {name}");
            if let Some(desc) = &tpl.description {
                println!("  Description: {desc}");
            }
            if let Some(agent) = &tpl.default_agent {
                println!("  Default Agent: {agent}");
            }
            println!("  Repositories:");
            match config.resolve_template_repos(name) {
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
                    println!("    (Configuration error: {e})");
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
fn cmd_destroy(config: Option<&Config>, project_name: &str, force: bool) -> Result<()> {
    let cwd = env::current_dir().context("Failed to get current working directory")?;
    let progress_sink = |event: botai::project::ProjectDestroyEvent| match event {
        botai::project::ProjectDestroyEvent::Step {
            current,
            total,
            message,
        } => {
            println!("[{current}/{total}] {message}");
        }
        botai::project::ProjectDestroyEvent::Log(msg) => {
            println!("{msg}");
        }
        botai::project::ProjectDestroyEvent::Success => {
            println!("✓ Successfully destroyed project '{project_name}' and cleaned up worktrees.");
        }
        botai::project::ProjectDestroyEvent::Error(err) => {
            eprintln!("✗ Error: {err}");
        }
    };
    botai::project::destroy_project_workspace_with_progress(
        config,
        &cwd,
        project_name,
        force,
        progress_sink,
    )?;
    Ok(())
}
