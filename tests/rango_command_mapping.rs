use voice_runtime::backend::browser::map_rango_action;
use voice_runtime::runtime::errors::ExecutionError;
use voice_runtime::runtime::executor::RuntimeValue;

#[test]
fn rango_open_hint_action_maps_to_browser_command() {
    let cmd = map_rango_action("user.rango_open_hint", &[]).unwrap();
    assert_eq!(cmd.command_name, "rango.openHint");
}

#[test]
fn rango_command_without_target_maps_action_and_numeric_arg() {
    let cmd = map_rango_action(
        "user.rango_command_without_target",
        &[
            RuntimeValue::String("\"scrollDownPage\"".into()),
            RuntimeValue::Number("0.5".into()),
        ],
    )
    .unwrap();
    assert_eq!(cmd.command_name, "rango.scrollDownPage");
    assert_eq!(cmd.args, vec!["0.5".to_string()]);
}

#[test]
fn non_rango_action_returns_unsupported_feature() {
    let err = map_rango_action("user.cursorless_take_line", &[]).unwrap_err();
    assert!(matches!(err, ExecutionError::UnsupportedFeature(_)));
}
