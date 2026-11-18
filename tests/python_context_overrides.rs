use voice_runtime::runtime::python_host::PythonCompatibilityHost;

#[test]
fn context_override_wins_over_default_action() {
    let host = PythonCompatibilityHost::from_test_override(
        "user.open_target",
        "browser",
        "browser_impl",
        "default_impl",
    );
    assert_eq!(
        host.resolve_for_test("user.open_target", "browser"),
        "browser_impl"
    );
}
