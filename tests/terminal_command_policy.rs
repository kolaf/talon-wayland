use voice_runtime::backend::terminal::{map_terminal_action, TerminalDispatch};
use voice_runtime::runtime::errors::ExecutionError;
use voice_runtime::runtime::executor::RuntimeValue;

#[test]
fn terminal_insert_prefers_text_dispatch() {
    let dispatch =
        map_terminal_action("insert", &[RuntimeValue::String("git status".into())]).unwrap();
    assert!(matches!(dispatch, TerminalDispatch::InsertText(_)));
}

#[test]
fn destructive_terminal_key_action_requires_explicit_supported_mapping() {
    let err = map_terminal_action("user.unsupported_terminal_combo", &[]).unwrap_err();
    assert!(matches!(err, ExecutionError::UnsupportedFeature(_)));
}
