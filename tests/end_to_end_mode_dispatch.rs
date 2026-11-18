use voice_runtime::app::RuntimeApp;
use voice_runtime::mode::RuntimeMode;

#[test]
fn recognized_phrase_routes_to_backend_for_active_mode() {
    let mut app = RuntimeApp::new_for_end_to_end_test(RuntimeMode::Browser);
    app.handle_recognized_phrase("open hint air").unwrap();
    assert_eq!(app.last_backend_invocation_for_test(), "browser");

    app.set_manual_mode(RuntimeMode::Code);
    app.handle_recognized_phrase("take line").unwrap();
    assert_eq!(app.last_backend_invocation_for_test(), "vscode");

    app.set_manual_mode(RuntimeMode::Terminal);
    app.handle_recognized_phrase("status check").unwrap();
    assert_eq!(app.last_backend_invocation_for_test(), "terminal");
}
