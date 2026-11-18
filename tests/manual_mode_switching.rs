use voice_runtime::app::RuntimeApp;
use voice_runtime::mode::{ModeSource, RuntimeMode};

#[test]
fn manual_mode_override_beats_default_mode() {
    let mut app = RuntimeApp::new_for_test();
    app.set_manual_mode(RuntimeMode::Browser);
    assert_eq!(app.active_mode(), RuntimeMode::Browser);
    assert_eq!(app.mode_source(), ModeSource::ManualOverride);
}

#[test]
fn cycle_mode_visits_code_browser_terminal_and_global() {
    let mut app = RuntimeApp::new_for_test();
    app.cycle_mode();
    assert_eq!(app.active_mode(), RuntimeMode::Code);
    app.cycle_mode();
    assert_eq!(app.active_mode(), RuntimeMode::Browser);
    app.cycle_mode();
    assert_eq!(app.active_mode(), RuntimeMode::Terminal);
    app.cycle_mode();
    assert_eq!(app.active_mode(), RuntimeMode::Global);
}
