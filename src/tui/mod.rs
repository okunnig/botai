pub mod app;
pub mod dashboard;
pub mod events;
pub mod status_bar;
pub mod ui;

pub use app::TuiApp;
pub use dashboard::{
    render_dashboard, ActivePanel, CreationModalState, DashboardAction, DashboardState,
};
pub use events::{key_to_bytes, map_key_event, KeyHandler, TuiAction};
pub use status_bar::StatusBar;
pub use ui::{compute_layout, vt_color_to_ratatui, TerminalView};
