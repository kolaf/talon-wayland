use std::collections::HashMap;
use voice_runtime::app::RuntimeApp;
use voice_runtime::runtime::errors::ExecutionError;
use voice_runtime::runtime::talon_ast::{TalonBodyStmt, TalonCommand};
use voice_runtime::speech::MatchedCommand;

fn test_matched_command(spoken: &str, action_name: &str) -> MatchedCommand {
    MatchedCommand::new(
        TalonCommand {
            spoken_form: spoken.to_string(),
            body: vec![TalonBodyStmt::ActionInvoke {
                name: action_name.to_string(),
                args: vec![],
            }],
            line: 1,
            ..Default::default()
        },
        HashMap::new(),
    )
}

#[test]
fn unsupported_action_returns_explicit_error() {
    let mut app = RuntimeApp::new_for_test();
    let command = test_matched_command("unsupported thing", "user.unsupported_action");
    let err = app.execute_command(&command).unwrap_err();
    assert!(matches!(
        err,
        ExecutionError::ActionNotFound(_) | ExecutionError::UnsupportedFeature(_)
    ));
    assert_eq!(app.last_error(), Some(&err));
}

#[test]
fn missing_action_returns_action_not_found() {
    let mut app = RuntimeApp::new_for_test();
    let command = test_matched_command("mystery", "user.does_not_exist");
    let err = app.execute_command(&command).unwrap_err();
    assert!(matches!(err, ExecutionError::ActionNotFound(_)));
}
