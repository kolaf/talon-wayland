use std::collections::HashMap;
use voice_runtime::app::RuntimeApp;
use voice_runtime::mode::RuntimeMode;
use voice_runtime::runtime::errors::ExecutionError;
use voice_runtime::runtime::talon_ast::{TalonBodyStmt, TalonCommand};
use voice_runtime::runtime::talon_parser::parse_talon_file;
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
fn successful_execution_clears_previous_error() {
    let mut app = RuntimeApp::new_for_test();
    app.set_last_error_for_test(ExecutionError::BackendUnavailable("browser".into()));
    let command = test_matched_command("noop", "user.test_noop");
    app.execute_command(&command).unwrap();
    assert!(app.last_error().is_none());
}

#[test]
fn executes_python_action_with_local_assignment_and_interpolation() {
    let mut app = RuntimeApp::new_for_test();
    app.set_manual_mode(RuntimeMode::Global);
    app.python_host_mut()
        .load_python_source(
            r#"
from talon import Module

mod = Module()

@mod.action_class
class Actions:
    def upper_wrap(word: str) -> str:
        return word.upper()
"#,
            "pipeline_test.py",
        )
        .unwrap();

    let file = parse_talon_file(
        "\
shout <user.word>:
    upper = user.upper_wrap(word)
    insert(\"say: {upper}!\")
",
    )
    .unwrap();

    let mut captures = HashMap::new();
    captures.insert("word".into(), "hello".into());
    let matched = MatchedCommand::new(file.commands[0].clone(), captures);

    app.execute_command(&matched).unwrap();
    assert!(app.last_error().is_none());
}
