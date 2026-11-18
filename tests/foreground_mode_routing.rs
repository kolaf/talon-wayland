use voice_runtime::app::RuntimeApp;
use voice_runtime::mode::{infer_mode, ForegroundAppInfo, ModeSource, RuntimeMode};

#[test]
fn foreground_mode_vscode_maps_to_code_mode() {
    let app = ForegroundAppInfo {
        app_id: "code".into(),
        title: "main.rs - Visual Studio Code".into(),
    };
    assert_eq!(infer_mode(&app), Some(RuntimeMode::Code));
}

#[test]
fn foreground_mode_vivaldi_maps_to_browser_mode() {
    let app = ForegroundAppInfo {
        app_id: "vivaldi-stable".into(),
        title: "Docs - Vivaldi".into(),
    };
    assert_eq!(infer_mode(&app), Some(RuntimeMode::Browser));
}

#[test]
fn foreground_mode_manual_override_is_not_replaced_by_foreground_refresh() {
    let mut app = RuntimeApp::new_for_test();
    app.set_manual_mode(RuntimeMode::Terminal);
    app.refresh_mode_from_foreground();
    assert_eq!(app.active_mode(), RuntimeMode::Terminal);
    assert_eq!(app.mode_source(), ModeSource::ManualOverride);
}
