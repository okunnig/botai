use std::collections::HashMap;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Errors that can occur during context synthesis and AGENTS.md generation.
#[derive(Debug, Error)]
pub enum ContextError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Invalid context configuration: {0}")]
    InvalidContext(String),
}

/// Metadata mapping a child worktree to its original repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoMapping {
    /// Directory name inside the project root (e.g., "frontend", "backend").
    pub name: String,
    /// Absolute path to the original git repository.
    pub original_path: PathBuf,
    /// Absolute or relative path to the worktree directory inside project root.
    pub worktree_path: PathBuf,
}

impl RepoMapping {
    pub fn new(
        name: impl Into<String>,
        original_path: impl Into<PathBuf>,
        worktree_path: impl Into<PathBuf>,
    ) -> Self {
        Self {
            name: name.into(),
            original_path: original_path.into(),
            worktree_path: worktree_path.into(),
        }
    }
}

/// Information needed to synthesize project context for AI agents working in a multi-repo project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectContext {
    /// Name of the active project (and git branch).
    pub project_name: String,
    /// Name of the template configuration.
    pub template_name: String,
    /// Optional template description.
    pub description: Option<String>,
    /// Optional custom template instructions appended to AGENTS.md.
    pub agents_md_append: Option<String>,
    /// Mappings of all repositories included in this template/project.
    pub repos: Vec<RepoMapping>,
    /// Environment variables injected for the template.
    pub env: HashMap<String, String>,
    /// Optional initial prompt/task seeded by the user.
    pub initial_prompt: Option<String>,
}

impl ProjectContext {
    pub fn new(project_name: impl Into<String>, template_name: impl Into<String>) -> Self {
        Self {
            project_name: project_name.into(),
            template_name: template_name.into(),
            description: None,
            agents_md_append: None,
            repos: Vec::new(),
            env: HashMap::new(),
            initial_prompt: None,
        }
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn with_agents_md_append(mut self, text: impl Into<String>) -> Self {
        self.agents_md_append = Some(text.into());
        self
    }

    pub fn with_repos(mut self, repos: Vec<RepoMapping>) -> Self {
        self.repos = repos;
        self
    }

    pub fn with_env(mut self, env: HashMap<String, String>) -> Self {
        self.env = env;
        self
    }

    pub fn with_initial_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.initial_prompt = Some(prompt.into());
        self
    }
}

/// Context synthesizer for generating AI agent orientation markdown (`AGENTS.md`).
pub struct ContextSynthesizer;

impl ContextSynthesizer {
    /// Synthesizes the markdown content for `AGENTS.md` in memory.
    pub fn synthesize(context: &ProjectContext) -> String {
        let mut md = String::new();

        md.push_str(&format!("# Project: {}\n\n", context.project_name));
        md.push_str(&format!("Template: **{}**\n\n", context.template_name));

        if let Some(desc) = &context.description {
            md.push_str(&format!("{desc}\n\n"));
        }

        if let Some(prompt) = &context.initial_prompt {
            md.push_str("## Task Description\n\n");
            md.push_str(&format!("{prompt}\n\n"));
        }

        md.push_str("## Repository Layout\n\n");
        if context.repos.is_empty() {
            md.push_str("No repositories configured.\n\n");
        } else {
            md.push_str("| Repository | Subdirectory | Original Source Path |\n");
            md.push_str("|---|---|---|\n");
            for repo in &context.repos {
                md.push_str(&format!(
                    "| {} | `{}` | `{}` |\n",
                    repo.name,
                    repo.worktree_path.display(),
                    repo.original_path.display()
                ));
            }
            md.push('\n');
        }

        if !context.env.is_empty() {
            md.push_str("## Environment Variables\n\n");
            md.push_str("| Variable | Value |\n");
            md.push_str("|---|---|\n");
            let mut keys: Vec<_> = context.env.keys().collect();
            keys.sort();
            for key in keys {
                let val = &context.env[key];
                md.push_str(&format!("| `{key}` | `{val}` |\n"));
            }
            md.push('\n');
        }

        md.push_str("## Multi-Repository Guidelines\n\n");
        md.push_str(&format!(
            "- **Active Branch**: All worktrees are checked out to branch `{}`.\n",
            context.project_name
        ));
        md.push_str("- **Multi-Repo Project**: Each subfolder corresponds to an independent Git worktree repository.\n");
        md.push_str("- **Atomic Commits**: Commit changes within each individual repository worktree as appropriate.\n");
        md.push_str("- **Execution Context**: Run build/test commands within the respective repository subdirectories.\n");

        if let Some(text) = &context.agents_md_append {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                md.push('\n');
                md.push_str("## Template Instructions\n\n");
                md.push_str(trimmed);
                md.push('\n');
            }
        }

        md
    }

    /// Generates `AGENTS.md` in the specified project root directory.
    pub fn generate_agents_md(
        project_root: &Path,
        context: &ProjectContext,
    ) -> Result<PathBuf, ContextError> {
        if !project_root.exists() {
            std::fs::create_dir_all(project_root)?;
        }
        let target_path = project_root.join("AGENTS.md");
        let content = Self::synthesize(context);
        std::fs::write(&target_path, content)?;
        Ok(target_path)
    }
}

/// Convenience function for generating `AGENTS.md`.
pub fn generate_agents_md(
    project_root: &Path,
    context: &ProjectContext,
) -> Result<PathBuf, ContextError> {
    ContextSynthesizer::generate_agents_md(project_root, context)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_synthesize_full_context() {
        let mut env = HashMap::new();
        env.insert("PORT".to_string(), "8080".to_string());
        env.insert("NODE_ENV".to_string(), "development".to_string());

        let repos = vec![
            RepoMapping::new(
                "ecommerce-api",
                PathBuf::from("/home/user/code/ecommerce-api"),
                PathBuf::from("ecommerce-api"),
            ),
            RepoMapping::new(
                "ecommerce-web",
                PathBuf::from("/home/user/code/ecommerce-web"),
                PathBuf::from("ecommerce-web"),
            ),
        ];

        let context = ProjectContext::new("checkout-flow", "ecommerce")
            .with_description("E-commerce core services and web app")
            .with_repos(repos)
            .with_env(env)
            .with_initial_prompt("Implement Stripe checkout endpoint and frontend integration.");

        let markdown = ContextSynthesizer::synthesize(&context);

        // Header & metadata
        assert!(markdown.contains("# Project: checkout-flow"));
        assert!(markdown.contains("Template: **ecommerce**"));
        assert!(markdown.contains("E-commerce core services and web app"));

        // Multi-repo table
        assert!(markdown.contains("ecommerce-api"));
        assert!(markdown.contains("/home/user/code/ecommerce-api"));
        assert!(markdown.contains("ecommerce-web"));
        assert!(markdown.contains("/home/user/code/ecommerce-web"));

        // Environment variables
        assert!(markdown.contains("PORT"));
        assert!(markdown.contains("8080"));
        assert!(markdown.contains("NODE_ENV"));
        assert!(markdown.contains("development"));

        // Task prompt
        assert!(markdown.contains("Implement Stripe checkout endpoint and frontend integration."));

        // Agent orientation guidance
        assert!(markdown.contains("Multi-Repository Guidelines"));
        assert!(markdown.contains("checkout-flow"));
    }

    #[test]
    fn test_synthesize_minimal_context() {
        let repos = vec![RepoMapping::new(
            "single-service",
            PathBuf::from("/var/repos/single-service"),
            PathBuf::from("single-service"),
        )];

        let context = ProjectContext::new("quick-fix", "infra").with_repos(repos);

        let markdown = ContextSynthesizer::synthesize(&context);

        assert!(markdown.contains("# Project: quick-fix"));
        assert!(markdown.contains("Template: **infra**"));
        assert!(markdown.contains("single-service"));
        assert!(markdown.contains("/var/repos/single-service"));
    }

    #[test]
    fn test_generate_agents_md_file_created() {
        let temp_dir = TempDir::new().expect("create tempdir");
        let project_root = temp_dir.path().join("my-project");

        let context =
            ProjectContext::new("my-project", "my-template").with_repos(vec![RepoMapping::new(
                "backend",
                PathBuf::from("/repos/backend"),
                PathBuf::from("backend"),
            )]);

        let agents_md_path =
            generate_agents_md(&project_root, &context).expect("generate_agents_md should succeed");

        assert_eq!(agents_md_path, project_root.join("AGENTS.md"));
        assert!(agents_md_path.exists());

        let content = fs::read_to_string(&agents_md_path).expect("read AGENTS.md");
        assert!(content.contains("my-project"));
        assert!(content.contains("my-template"));
        assert!(content.contains("backend"));
    }

    #[test]
    fn test_generate_agents_md_overwrites_existing() {
        let temp_dir = TempDir::new().expect("create tempdir");
        let project_root = temp_dir.path().join("existing-project");
        fs::create_dir_all(&project_root).expect("create project dir");

        let agents_md = project_root.join("AGENTS.md");
        fs::write(&agents_md, "Old content").expect("write old file");

        let context = ProjectContext::new("existing-project", "template-v2")
            .with_description("Updated template");

        generate_agents_md(&project_root, &context).expect("generate agents md");

        let content = fs::read_to_string(&agents_md).expect("read updated file");
        assert!(!content.contains("Old content"));
        assert!(content.contains("template-v2"));
        assert!(content.contains("Updated template"));
    }

    #[test]
    fn test_special_characters_escaping_and_markdown_safety() {
        let mut env = HashMap::new();
        env.insert("CONFIG_PIPE".to_string(), "foo|bar|baz".to_string());
        env.insert(
            "URL".to_string(),
            "https://example.com/?a=1&b=2".to_string(),
        );

        let repos = vec![RepoMapping::new(
            "repo | name with pipes",
            PathBuf::from("/path/to/repo|pipe"),
            PathBuf::from("repo | name with pipes"),
        )];

        let context = ProjectContext::new("fix/weird-chars_<123>", "ws & co")
            .with_description("Handling <div> tags & pipes | nicely")
            .with_repos(repos)
            .with_env(env)
            .with_initial_prompt("Prompt with `code` and *markdown* syntax.");

        let markdown = ContextSynthesizer::synthesize(&context);

        assert!(markdown.contains("fix/weird-chars_<123>"));
        assert!(markdown.contains("ws & co"));
        assert!(markdown.contains("Handling <div> tags & pipes | nicely"));
        assert!(markdown.contains("CONFIG_PIPE"));
        assert!(markdown.contains("foo|bar|baz"));
        assert!(markdown.contains("Prompt with `code` and *markdown* syntax."));
    }

    #[test]
    fn test_synthesize_with_agents_md_append() {
        let context = ProjectContext::new("checkout-fix", "payments")
            .with_description("Payments microservices")
            .with_agents_md_append(
                "### Guidelines\n- Always run `cargo test --all`\n- Follow conventional commits",
            );

        let markdown = ContextSynthesizer::synthesize(&context);

        assert!(markdown.contains("## Multi-Repository Guidelines"));
        assert!(markdown.contains("## Template Instructions\n\n### Guidelines\n- Always run `cargo test --all`\n- Follow conventional commits\n"));
    }

    #[test]
    fn test_synthesize_without_agents_md_append() {
        let context = ProjectContext::new("checkout-fix", "payments")
            .with_description("Payments microservices");

        let markdown = ContextSynthesizer::synthesize(&context);

        assert!(markdown.contains("## Multi-Repository Guidelines"));
        assert!(!markdown.contains("## Template Instructions"));
    }

    #[test]
    fn test_synthesize_with_empty_or_whitespace_agents_md_append() {
        let context_empty = ProjectContext::new("p1", "ws1").with_agents_md_append("");
        let md_empty = ContextSynthesizer::synthesize(&context_empty);
        assert!(!md_empty.contains("## Template Instructions"));

        let context_whitespace =
            ProjectContext::new("p2", "ws2").with_agents_md_append("   \n\t  \n  ");
        let md_whitespace = ContextSynthesizer::synthesize(&context_whitespace);
        assert!(!md_whitespace.contains("## Template Instructions"));
    }
}
