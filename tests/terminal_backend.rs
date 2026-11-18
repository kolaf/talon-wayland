use voice_runtime::backend::terminal::{map_terminal_action, TerminalBackend, TerminalDispatch};
use voice_runtime::backend::AppBackend;
use voice_runtime::runtime::errors::ExecutionError;
use voice_runtime::runtime::executor::RuntimeValue;

#[test]
fn terminal_backend_allows_safe_control_keys_and_rejects_unallowlisted_keys() {
    let dispatch =
        map_terminal_action("key", &[RuntimeValue::String("ctrl-c".into())]).unwrap();
    assert!(matches!(dispatch, TerminalDispatch::SendKeys(_)));

    let err = map_terminal_action("key", &[RuntimeValue::String("alt-f4".into())]).unwrap_err();
    assert!(matches!(err, ExecutionError::UnsupportedFeature(_)));
}

#[test]
fn disconnected_terminal_backend_returns_backend_unavailable() {
    let mut backend = TerminalBackend::new_for_test_disconnected();
    let err = backend
        .execute_action("insert", &[RuntimeValue::String("ls".into())])
        .unwrap_err();
    assert!(matches!(err, ExecutionError::BackendUnavailable(_)));
}
