use botai::config::{Config, ConfigError};
use std::fs;
use tempfile::tempdir;

#[test]
fn test_integration_full_config_lifecycle() {
    let dir = tempdir().expect("Failed to create tempdir");
    let config_file_path = dir.path().join("config.toml");

    let toml_content = r#"
default_agent = "claude"

[agents.claude]
command = "claude"
args = ["--dangerously-skip-permissions"]
env = { ANTHROPIC_API_KEY = "sk-test-123" }

[agents.agy]
command = "agy"
args = ["--model", "gemini-pro"]

[workspaces.backend]
description = "Microservices backend"
default_agent = "agy"
repos = [
    "/var/repos/auth-service",
    "/var/repos/payment-service"
]
env = { DB_HOST = "localhost", DB_PORT = "5432" }

[workspaces.frontend]
description = "Web and Mobile Frontends"
repos = [
    "/var/repos/web-client",
    "/var/repos/mobile-app"
]
env = { REACT_APP_API_URL = "http://localhost:8080" }
"#;

    fs::write(&config_file_path, toml_content).expect("Failed to write test config file");

    // 1. Load config from file
    let config =
        Config::load_from_file(&config_file_path).expect("Should load valid config file from disk");

    // 2. Validate parsed structures
    assert_eq!(config.default_agent, "claude");
    assert_eq!(config.agents.len(), 2);
    assert_eq!(config.workspaces.len(), 2);

    // 3. Test workspace queries
    let backend_ws = config
        .get_workspace("backend")
        .expect("backend workspace must exist");
    assert_eq!(
        backend_ws.description.as_deref(),
        Some("Microservices backend")
    );
    assert_eq!(backend_ws.repos.len(), 2);

    let frontend_ws = config
        .get_workspace("frontend")
        .expect("frontend workspace must exist");
    assert_eq!(frontend_ws.default_agent, None);

    // 4. Test agent resolution (override vs fallback)
    let backend_agent = config
        .resolve_agent_for_workspace("backend")
        .expect("backend should resolve to agy");
    assert_eq!(backend_agent.command, "agy");
    assert_eq!(
        backend_agent.args,
        vec!["--model".to_string(), "gemini-pro".to_string()]
    );

    let frontend_agent = config
        .resolve_agent_for_workspace("frontend")
        .expect("frontend should resolve to global default claude");
    assert_eq!(frontend_agent.command, "claude");
    assert_eq!(
        frontend_agent.args,
        vec!["--dangerously-skip-permissions".to_string()]
    );
    assert_eq!(
        frontend_agent.env.get("ANTHROPIC_API_KEY"),
        Some(&"sk-test-123".to_string())
    );

    // 5. Test non-existent queries
    assert!(config.get_workspace("non_existent").is_none());
    assert!(config.get_agent("non_existent").is_none());

    let resolve_missing_ws = config.resolve_agent_for_workspace("non_existent");
    match resolve_missing_ws {
        Err(ConfigError::WorkspaceNotFound(name)) => assert_eq!(name, "non_existent"),
        other => panic!("Expected WorkspaceNotFound, got {:?}", other),
    }
}

#[test]
fn test_integration_corrupted_config_file() {
    let dir = tempdir().expect("Failed to create tempdir");
    let config_file_path = dir.path().join("broken_config.toml");

    fs::write(&config_file_path, "default_agent = [unclosed syntax")
        .expect("Failed to write broken config");

    let res = Config::load_from_file(&config_file_path);
    assert!(res.is_err(), "Loading corrupted config should return error");

    match res.unwrap_err() {
        ConfigError::Parse(_) => (),
        other => panic!("Expected ConfigError::Parse, got {:?}", other),
    }
}

#[test]
fn test_integration_init_template_lifecycle() {
    let dir = tempdir().expect("Failed to create tempdir");
    let config_path = dir.path().join("subfolder").join("config.toml");

    // 1. Initialize fresh template
    let generated = Config::init_file(&config_path, false)
        .expect("Initializing config in non-existent path should succeed");
    assert_eq!(generated, config_path);
    assert!(config_path.exists());

    // 2. Parse generated config and verify all agents/workspaces
    let config = Config::load_from_file(&config_path)
        .expect("Generated template must be immediately loadable");
    assert_eq!(config.default_agent, "claude");
    assert!(config.get_agent("claude").is_some());
    assert!(config.get_agent("agy").is_some());
    assert!(config.get_agent("copilot").is_some());
    assert!(config.get_agent("aider").is_some());
    assert!(config.get_workspace("ecommerce").is_some());

    // 3. Re-init without force fails with AlreadyExists
    let err = Config::init_file(&config_path, false)
        .expect_err("Re-init without force must return ConfigError::AlreadyExists");
    match err {
        ConfigError::AlreadyExists(p) => assert_eq!(p, config_path),
        other => panic!("Expected ConfigError::AlreadyExists, got {:?}", other),
    }

    // 4. Overwrite dummy content with force=true
    fs::write(&config_path, "corrupted = true").expect("Failed to write dummy content");
    Config::init_file(&config_path, true).expect("Re-init with force=true must succeed");
    let reloaded = Config::load_from_file(&config_path).expect("Reloading overwritten config");
    assert_eq!(reloaded.default_agent, "claude");
}
