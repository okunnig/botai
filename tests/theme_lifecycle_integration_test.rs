use std::fs;
use tempfile::tempdir;

use botai::config::Config;
use botai::session::{ProjectSession, PtyConfig, PtySession, SessionRegistry, SessionStatus, VtScreen};
use botai::tui::app::render_error_banner;
use botai::tui::dashboard::{render_dashboard, DashboardState};
use botai::tui::quick_launcher::{render_quick_launcher, QuickLauncherState};
use botai::tui::status_bar::StatusBar;
use botai::tui::ui::compute_layout;
use botai::tui::TuiApp;
use botai::Theme;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::Terminal;

fn create_test_session(
    name: &str,
    ws: &str,
    agent: &str,
    status: SessionStatus,
) -> (ProjectSession, PtySession, VtScreen) {
    let tmp = tempdir().expect("tempdir");
    #[cfg(windows)]
    let (cmd, args) = ("cmd.exe", vec!["/c", "exit 0"]);
    #[cfg(not(windows))]
    let (cmd, args) = ("sh", vec!["-c", "true"]);

    let pty_config = PtyConfig::new(cmd, tmp.path())
        .with_args(args)
        .with_dimensions(80, 24);
    let pty = PtySession::spawn(pty_config).expect("spawn test pty");
    let vt = VtScreen::new(80, 24);
    let mut ps = ProjectSession::new(name, ws, agent);
    ps.set_status(status);

    (ps, pty, vt)
}

#[test]
fn test_integration_theme_lifecycle_from_toml_to_tui_rendering() {
    let dir = tempdir().expect("tempdir");
    let config_path = dir.path().join("config.toml");

    let toml = r#"
default_agent = "claude"
theme = "cyberpunk"
repos_dir = "/tmp/repos"

[agents.claude]
command = "claude"

[workspaces.ecommerce]
repos = ["/tmp/repos/shop"]
"#;
    fs::write(&config_path, toml).expect("write config");

    // 1. Load config and verify theme resolution
    let config = Config::load_from_file(&config_path).expect("load config");
    assert_eq!(config.theme, "cyberpunk");
    let theme = config.theme();
    assert_eq!(theme, Theme::cyberpunk());

    // 2. Test Dashboard rendering with Cyberpunk theme
    let dashboard_state = DashboardState::new(
        vec!["ecommerce".to_string()],
        vec!["shop-service".to_string()],
    );
    let area = Rect::new(0, 0, 80, 20);
    let mut buf = Buffer::empty(area);
    render_dashboard(&dashboard_state, area, &mut buf, &theme);

    // Verify left panel focused border matches cyberpunk border_focused
    let border_cell = buf.cell((0, 0)).unwrap();
    assert_eq!(border_cell.fg, theme.border_focused);

    // Verify footer bar background matches cyberpunk status_bar_bg
    let footer_cell = buf.cell((0, 19)).unwrap();
    assert_eq!(footer_cell.bg, theme.status_bar_bg);

    // 3. Test Quick Launcher rendering with Cyberpunk theme
    let launcher_state = QuickLauncherState::new(
        vec!["ecommerce".to_string()],
        vec!["shop-service".to_string()],
        Some("ecommerce"),
    );
    let mut launcher_buf = Buffer::empty(area);
    render_quick_launcher(&launcher_state, area, &mut launcher_buf, &theme);

    // Verify launcher container border matches cyberpunk border_focused
    let l_border_cell = launcher_buf.cell((4, 2)).unwrap();
    assert_eq!(l_border_cell.fg, theme.border_focused);

    // 4. Test Status Bar rendering with Cyberpunk theme
    let mut registry = SessionRegistry::new();
    let (ps, pty, vt) = create_test_session("shop-service", "ecommerce", "claude", SessionStatus::Busy);
    registry.add(ps, pty, vt).unwrap();

    let mut status_buf = Buffer::empty(Rect::new(0, 0, 80, 1));
    StatusBar::render_to_buffer_with_theme(&registry, false, &theme, Rect::new(0, 0, 80, 1), &mut status_buf);

    // Active tab cell background matches cyberpunk tab_active_bg (neon cyan)
    let active_tab_cell = status_buf.cell((2, 0)).unwrap();
    assert_eq!(active_tab_cell.bg, theme.tab_active_bg);
    assert_eq!(active_tab_cell.fg, theme.tab_active_fg);

    // Empty status bar cell background matches cyberpunk status_bar_bg
    let empty_cell = status_buf.cell((79, 0)).unwrap();
    assert_eq!(empty_cell.bg, theme.status_bar_bg);

    // 5. Test Error Banner rendering with Cyberpunk theme
    let mut error_buf = Buffer::empty(Rect::new(0, 0, 80, 5));
    render_error_banner(Some(1), Rect::new(0, 0, 80, 5), &mut error_buf, &theme);
    let banner_cell = error_buf.cell((0, 4)).unwrap();
    assert_eq!(banner_cell.bg, theme.error_banner_bg);
    assert_eq!(banner_cell.fg, theme.error_banner_fg);
}

#[test]
fn test_integration_theme_palette_switching_and_fallbacks() {
    let dir = tempdir().expect("tempdir");

    let test_cases = [
        ("tokyo-night", Theme::tokyo_night()),
        ("catppuccin-mocha", Theme::catppuccin()),
        ("gruvbox-dark", Theme::gruvbox()),
        ("zinc", Theme::monochrome()),
        ("classic", Theme::ansi()),
        ("non_existent_palette", Theme::tokyo_night()), // fallback
    ];

    for (name, expected_theme) in test_cases {
        let config_path = dir.path().join(format!("config_{}.toml", name));
        let toml = format!(
            r#"
default_agent = "claude"
theme = "{}"
repos_dir = "/tmp/repos"
"#,
            name
        );
        fs::write(&config_path, toml).expect("write config");

        let config = Config::load_from_file(&config_path).expect("load config");
        assert_eq!(config.theme(), expected_theme);

        // Verify TuiApp integration with this config
        let app = TuiApp::new(SessionRegistry::new()).with_config(config);
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|f| {
                let (status_area, _) = compute_layout(f.area());
                let app_theme = app.config.as_ref().map(|c| c.theme()).unwrap_or_default();
                f.render_widget(
                    StatusBar::new(&app.registry).with_theme(&app_theme),
                    status_area,
                );
            })
            .unwrap();

        let cell = terminal.backend().buffer().cell((0, 0)).unwrap();
        assert_eq!(cell.bg, expected_theme.status_bar_bg);
    }
}
