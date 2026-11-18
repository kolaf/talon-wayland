#[test]
fn starts_with_no_backends_connected() {
    let app = voice_runtime::app::RuntimeApp::new_for_test();
    assert_eq!(app.active_mode().to_string(), "global");
    assert!(app.backends().is_empty());
}
