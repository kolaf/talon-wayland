use voice_runtime::backend::vscode::map_cursorless_action;
use voice_runtime::runtime::errors::ExecutionError;
use voice_runtime::runtime::executor::RuntimeValue;

#[test]
fn cursorless_style_action_maps_to_vscode_command() {
    let cmd = map_cursorless_action("user.cursorless_take_line", &[]).unwrap();
    assert_eq!(cmd.command_name, "cursorless.takeLine");
}

#[test]
fn generic_vscode_action_maps_command_id_and_args() {
    let cmd = map_cursorless_action(
        "user.vscode_with_plugin",
        &[
            RuntimeValue::String("\"workbench.action.quickOpen\"".into()),
            RuntimeValue::String("main.rs".into()),
        ],
    )
    .unwrap();
    assert_eq!(cmd.command_name, "workbench.action.quickOpen");
    assert_eq!(cmd.args, vec!["main.rs".to_string()]);
}

#[test]
fn non_vscode_action_returns_unsupported_feature() {
    let err = map_cursorless_action("user.rango_open_hint", &[]).unwrap_err();
    assert!(matches!(err, ExecutionError::UnsupportedFeature(_)));
}
