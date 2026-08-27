use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

#[test]
fn test_cli_edit_auto_initializes_missing_config_and_opens_editor() {
    let temp_dir = TempDir::new().expect("create tempdir");
    let config_path = temp_dir.path().join("nested").join("config.toml");

    assert!(!config_path.exists());

    let mut cmd = Command::cargo_bin("botai").expect("binary exists");

    #[cfg(unix)]
    let editor_arg = "sh -c 'echo \"# edited\" >> \"$1\"' --";
    #[cfg(windows)]
    let editor_arg = "cmd.exe /C echo # edited >>";

    let assert = cmd
        .args([
            "--config",
            config_path.to_str().unwrap(),
            "edit",
            "--editor",
            editor_arg,
        ])
        .assert();

    assert.success();

    assert!(config_path.exists(), "Configuration file must be created");
    let content = fs::read_to_string(&config_path).expect("read config file");
    assert!(
        content.contains("botai - Multi-Repo Workspace Orchestrator"),
        "Should contain default template header"
    );
    assert!(
        content.contains("default_agent = \"claude\""),
        "Should contain default agent setting"
    );
    #[cfg(unix)]
    assert!(
        content.contains("# edited"),
        "Should contain edits appended by mock editor"
    );
}

#[test]
fn test_cli_edit_with_short_flag_multi_word_editor() {
    let temp_dir = TempDir::new().expect("create tempdir");
    let config_path = temp_dir.path().join("config.toml");
    fs::write(
        &config_path,
        "default_agent = \"claude\"\nrepos_dir = \"/repos\"\n",
    )
    .expect("write initial config");

    let mut cmd = Command::cargo_bin("botai").expect("binary exists");

    #[cfg(unix)]
    let editor_arg = "sh -c 'cat \"$1\" > /dev/null' --";
    #[cfg(windows)]
    let editor_arg = "cmd.exe /C type";

    let assert = cmd
        .args([
            "--config",
            config_path.to_str().unwrap(),
            "edit",
            "-e",
            editor_arg,
        ])
        .assert();

    assert.success();
}

#[test]
fn test_cli_edit_respects_visual_environment_variable() {
    let temp_dir = TempDir::new().expect("create tempdir");
    let config_path = temp_dir.path().join("config.toml");
    fs::write(
        &config_path,
        "default_agent = \"agy\"\nrepos_dir = \"/repos\"\n",
    )
    .expect("write initial config");

    #[cfg(unix)]
    let visual_cmd = "sh -c 'echo \"# from-visual\" >> \"$1\"' --";
    #[cfg(windows)]
    let visual_cmd = "cmd.exe /C echo # from-visual >>";

    let mut cmd = Command::cargo_bin("botai").expect("binary exists");
    let assert = cmd
        .env("VISUAL", visual_cmd)
        .env("EDITOR", "invalid-editor-should-not-run")
        .args(["--config", config_path.to_str().unwrap(), "edit"])
        .assert();

    assert.success();

    #[cfg(unix)]
    {
        let content = fs::read_to_string(&config_path).expect("read config");
        assert!(content.contains("# from-visual"));
    }
}

#[test]
fn test_cli_edit_respects_editor_environment_variable_when_visual_empty() {
    let temp_dir = TempDir::new().expect("create tempdir");
    let config_path = temp_dir.path().join("config.toml");
    fs::write(
        &config_path,
        "default_agent = \"claude\"\nrepos_dir = \"/repos\"\n",
    )
    .expect("write initial config");

    #[cfg(unix)]
    let editor_cmd = "sh -c 'echo \"# from-editor\" >> \"$1\"' --";
    #[cfg(windows)]
    let editor_cmd = "cmd.exe /C echo # from-editor >>";

    let mut cmd = Command::cargo_bin("botai").expect("binary exists");
    let assert = cmd
        .env("VISUAL", "   ")
        .env("EDITOR", editor_cmd)
        .args(["--config", config_path.to_str().unwrap(), "edit"])
        .assert();

    assert.success();

    #[cfg(unix)]
    {
        let content = fs::read_to_string(&config_path).expect("read config");
        assert!(content.contains("# from-editor"));
    }
}

#[test]
fn test_cli_edit_passes_through_editor_exit_code() {
    #[cfg(unix)]
    {
        let temp_dir = TempDir::new().expect("create tempdir");
        let config_path = temp_dir.path().join("config.toml");

        let mut cmd = Command::cargo_bin("botai").expect("binary exists");
        let assert = cmd
            .args([
                "--config",
                config_path.to_str().unwrap(),
                "edit",
                "--editor",
                "sh -c 'exit 42' --",
            ])
            .assert();

        assert.failure().code(42);
    }
}

#[test]
fn test_cli_edit_fails_cleanly_when_editor_not_found() {
    let temp_dir = TempDir::new().expect("create tempdir");
    let config_path = temp_dir.path().join("config.toml");

    let mut cmd = Command::cargo_bin("botai").expect("binary exists");
    let assert = cmd
        .args([
            "--config",
            config_path.to_str().unwrap(),
            "edit",
            "--editor",
            "__non_existent_binary_for_test_12345__",
        ])
        .assert();

    assert
        .failure()
        .stderr(predicate::str::contains("Failed to execute editor"));
}
