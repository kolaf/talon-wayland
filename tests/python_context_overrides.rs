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

#[test]
fn register_and_resolve_list_and_capture() {
    let mut host = PythonCompatibilityHost::default();
    host.register_list(
        "user.letters",
        vec!["a".to_string(), "b".to_string(), "c".to_string()],
    );
    host.register_capture("user.word", "(\\w+)");

    assert_eq!(
        host.list_values("user.letters").unwrap(),
        ["a", "b", "c"]
    );
    assert_eq!(host.capture_signature("user.word").unwrap(), "(\\w+)");
}
