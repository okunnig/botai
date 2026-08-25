pub mod app;
pub mod creation_modal;
pub mod dashboard;
pub mod events;
pub mod quick_launcher;
pub mod setup_modal;
pub mod status_bar;
pub mod theme;
pub mod ui;

pub use app::TuiApp;
pub use creation_modal::{
    CreationFocus, CreationModalAction, CreationStep, ProjectCreationModal,
};
pub use dashboard::{
    render_dashboard, CreationModalState, DashboardAction, DashboardState,
    DestroyModalState,
};
pub use events::{
    mouse_event_to_bytes, mouse_event_to_sgr_bytes, mouse_scroll_to_bytes, parse_raw_events,
    paste_to_bytes, RawAction, RawLeaderDecoder, TuiAction,
};
pub use quick_launcher::{
    render_quick_launcher, LauncherFocus, LauncherTab, QuickLauncherAction, QuickLauncherState,
};
pub use setup_modal::{
    render_setup_modal, run_setup_worker, SetupModalState, SetupProgressEvent,
};
pub use status_bar::StatusBar;
pub use theme::Theme;
pub use ui::{compute_layout, vt_color_to_ratatui, TerminalView};

