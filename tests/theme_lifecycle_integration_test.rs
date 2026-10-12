use std::fs;
use tempfile::tempdir;

use botai::config::Config;
use botai::session::{
    ProjectSession, PtyConfig, PtySession, SessionRegistry, SessionStatus, VtScreen,
};
use botai::tui::app::render_error_banner;
use botai::tui::dashboard::{render_dashboard, DashboardState};
use botai::tui::status_bar::StatusBar;
use botai::tui::ui::compute_layout;
use botai::tui::{ProjectCreationModal, TuiApp};
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

[templates.ecommerce]
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

    // Verify top header status bar background matches cyberpunk status_bar_bg
    let header_cell = buf.cell((0, 0)).unwrap();
    assert_eq!(header_cell.bg, theme.status_bar_bg);

    // Verify left panel focused border matches cyberpunk border_focused (at y = 1)
    let border_cell = buf.cell((0, 1)).unwrap();
    assert_eq!(border_cell.fg, theme.border_focused);

    // 3. Test Creation Modal rendering with Cyberpunk theme
    let modal = ProjectCreationModal::new(&config, dir.path(), Some("ecommerce"));
    let mut modal_buf = Buffer::empty(area);
    modal.render_modal(area, &mut modal_buf, &theme);

    // Verify modal container border matches cyberpunk modal_title
    let m_border_cell = modal_buf.cell((2, 1)).unwrap();
    assert_eq!(m_border_cell.fg, theme.modal_title);

    // 4. Test Status Bar rendering with Cyberpunk theme
    let mut registry = SessionRegistry::new();
    let (ps, pty, vt) =
        create_test_session("shop-service", "ecommerce", "claude", SessionStatus::Busy);
    registry.add(ps, pty, vt).unwrap();

    let mut status_buf = Buffer::empty(Rect::new(0, 0, 80, 1));
    StatusBar::render_to_buffer_with_theme(
        &registry,
        false,
        &theme,
        Rect::new(0, 0, 80, 1),
        &mut status_buf,
    );

    // Active tab leading space cell at (1, 0) matches cyberpunk tab_active_bg and tab_active_fg
    let active_tab_cell = status_buf.cell((1, 0)).unwrap();
    assert_eq!(active_tab_cell.symbol(), " ");
    assert_eq!(active_tab_cell.bg, theme.tab_active_bg);
    assert_eq!(active_tab_cell.fg, theme.tab_active_fg);

    // Active tab dot cell at (2, 0) matches cyberpunk status_busy and tab_active_bg
    let active_tab_dot = status_buf.cell((2, 0)).unwrap();
    assert_eq!(active_tab_dot.symbol(), "●");
    assert_eq!(active_tab_dot.bg, theme.tab_active_bg);
    assert_eq!(active_tab_dot.fg, theme.status_busy);

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
        let config_path = dir.path().join(format!("config_{name}.toml"));
        let toml = format!(
            r#"
default_agent = "claude"
theme = "{name}"
repos_dir = "/tmp/repos"
"#
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
                let app_theme = app.config.theme();
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
