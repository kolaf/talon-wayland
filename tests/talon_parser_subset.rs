use voice_runtime::mode::RuntimeMode;
use voice_runtime::runtime::talon_ast::{SpokenToken, TalonBodyStmt};
use voice_runtime::runtime::talon_parser::{
    parse_context_matches, parse_spoken_rule, parse_talon_file,
};

#[test]
fn parses_context_header_and_simple_command() {
    let src = "app.name: Code\ntag: user.tabs\n-\nfocus left: key(ctrl-left)\n";
    let file = parse_talon_file(src).unwrap();
    assert_eq!(file.context.app_name.as_deref(), Some("Code"));
    assert_eq!(file.context.tags, vec!["user.tabs"]);
    assert_eq!(file.commands[0].spoken_form, "focus left");
}

#[test]
fn parses_all_supported_context_headers() {
    let src = "app: vscode\napp.name: Code\nos: linux\nmode: code\ntag: user.tabs\ntag: user.git\ncode.language: rust\n-\nfocus left: key(ctrl-left)\n";
    let file = parse_talon_file(src).unwrap();
    assert_eq!(file.context.app.as_deref(), Some("vscode"));
    assert_eq!(file.context.app_name.as_deref(), Some("Code"));
    assert_eq!(file.context.os.as_deref(), Some("linux"));
    assert_eq!(file.context.mode, Some(RuntimeMode::Code));
    assert_eq!(file.context.tags, vec!["user.tabs", "user.git"]);
    assert_eq!(file.context.code_language.as_deref(), Some("rust"));
}

#[test]
fn parses_extended_context_headers_and_talon_modes() {
    let src = "\
app.exe: /^code$/i
title: /Visual Studio Code/
mode: command
not mode: dictation
and tag: user.cursorless
not tag: user.disabled
-
take line: user.cursorless_command(\"setSelection\")
";
    let file = parse_talon_file(src).unwrap();
    assert_eq!(file.context.app_exe.as_deref(), Some("/^code$/i"));
    assert_eq!(file.context.title.as_deref(), Some("/Visual Studio Code/"));
    assert_eq!(file.context.mode, None);
    assert_eq!(file.context.talon_modes, vec!["command"]);
    assert_eq!(file.context.not_modes, vec!["dictation"]);
    assert_eq!(file.context.tags, vec!["user.cursorless"]);
    assert_eq!(file.context.not_tags, vec!["user.disabled"]);

    let py_ctx = parse_context_matches("os: linux\nand app: vscode\n").unwrap();
    assert_eq!(py_ctx.os.as_deref(), Some("linux"));
    assert_eq!(py_ctx.app.as_deref(), Some("vscode"));
}

#[test]
fn parses_headerless_talon_file() {
    let src = "# global commands without header separator\nhello world: insert(\"hello\")\nbye: 'goodbye'\n";
    let file = parse_talon_file(src).unwrap();
    assert!(file.context.is_global_only());
    assert_eq!(file.commands.len(), 2);
    assert_eq!(file.commands[0].spoken_form, "hello world");
    assert_eq!(file.commands[1].spoken_form, "bye");
    assert_eq!(
        file.commands[1].body[0],
        TalonBodyStmt::ActionInvoke {
            name: "insert".into(),
            args: vec!["'goodbye'".into()],
        }
    );
}

#[test]
fn parses_top_level_settings_and_tag_activations() {
    let src = "\
app: vscode
-
tag(): user.tabs
settings():
    user.scroll_speed = 1.5
    user.cursorless_show_hat = true
focus left: key(ctrl-left)
";
    let file = parse_talon_file(src).unwrap();
    assert_eq!(file.activated_tags, vec!["user.tabs"]);
    assert_eq!(
        file.settings,
        vec![
            ("user.scroll_speed".into(), "1.5".into()),
            ("user.cursorless_show_hat".into(), "true".into()),
        ]
    );
    assert_eq!(file.commands.len(), 1);
    assert_eq!(file.commands[0].spoken_form, "focus left");
}

#[test]
fn skips_hardware_and_noise_triggers_with_diagnostics() {
    let src = "\
os: linux
-
parrot(pop): user.mouse_click()
gamepad(dpad_up):
    key(up)
hello: insert(\"hi\")
";
    let file = parse_talon_file(src).unwrap();
    assert_eq!(file.diagnostics.len(), 2);
    assert_eq!(file.commands.len(), 1);
    assert_eq!(file.commands[0].spoken_form, "hello");
}

#[test]
fn parses_multiline_command_body_with_multiple_actions() {
    let src = "app.name: Code\n-\ngo to google:\n    key(ctrl-t)\n    insert(google.com)\n    key(enter)\n";
    let file = parse_talon_file(src).unwrap();
    assert_eq!(file.commands.len(), 1);
    assert_eq!(file.commands[0].spoken_form, "go to google");
    assert_eq!(file.commands[0].body.len(), 3);
}

#[test]
fn parses_action_with_string_and_number_args() {
    let src = "tag: browser\n-\nhalf down: user.rango_command_without_target(\"scrollDownPage\", 0.5)\n";
    let file = parse_talon_file(src).unwrap();
    let cmd = &file.commands[0];
    assert_eq!(cmd.body.len(), 1);
    let TalonBodyStmt::ActionInvoke { name, args } = &cmd.body[0] else {
        panic!("expected ActionInvoke");
    };
    assert_eq!(name, "user.rango_command_without_target");
    assert_eq!(args, &vec!["\"scrollDownPage\"".to_string(), "0.5".to_string()]);
}

#[test]
fn parses_commas_in_quoted_args_and_nested_calls() {
    let src = "\
greet: insert(\"hello, world\")
snake <user.text>: insert(user.formatted_text(text, \"SNAKE_CASE\"))
";
    let file = parse_talon_file(src).unwrap();
    let TalonBodyStmt::ActionInvoke { args: greet_args, .. } = &file.commands[0].body[0] else {
        panic!("expected ActionInvoke");
    };
    assert_eq!(greet_args, &vec!["\"hello, world\"".to_string()]);

    let TalonBodyStmt::ActionInvoke { args: snake_args, .. } = &file.commands[1].body[0] else {
        panic!("expected ActionInvoke");
    };
    assert_eq!(
        snake_args,
        &vec!["user.formatted_text(text, \"SNAKE_CASE\")".to_string()]
    );
}

#[test]
fn parses_local_variable_assignment_in_command_body() {
    let src = "\
format <user.text>:
    formatted = user.formatted_text(text, \"CAMEL_CASE\")
    insert(formatted)
";
    let file = parse_talon_file(src).unwrap();
    assert_eq!(
        file.commands[0].body[0],
        TalonBodyStmt::Assign {
            var: "formatted".into(),
            expr: "user.formatted_text(text, \"CAMEL_CASE\")".into(),
        }
    );
    assert_eq!(
        file.commands[0].body[1],
        TalonBodyStmt::ActionInvoke {
            name: "insert".into(),
            args: vec!["formatted".into()],
        }
    );
}

#[test]
fn parses_spoken_rule_ast_constructs() {
    let rule = parse_spoken_rule("^take <user.cursorless_target> [please]$").unwrap();
    assert!(rule.anchored_start);
    assert!(rule.anchored_end);
    assert_eq!(
        rule.tokens,
        vec![
            SpokenToken::Word("take".into()),
            SpokenToken::CaptureRef("user.cursorless_target".into()),
            SpokenToken::Optional(vec![SpokenToken::Word("please".into())]),
        ]
    );

    let alt_rep = parse_spoken_rule("(go | move) {user.arrow_key}+").unwrap();
    assert!(!alt_rep.anchored_start);
    assert!(!alt_rep.anchored_end);
    assert_eq!(
        alt_rep.tokens,
        vec![
            SpokenToken::Alternation(vec![
                vec![SpokenToken::Word("go".into())],
                vec![SpokenToken::Word("move".into())],
            ]),
            SpokenToken::Repeat {
                token: Box::new(SpokenToken::ListRef("user.arrow_key".into())),
                one_or_more: true,
            },
        ]
    );
}

#[test]
fn errors_on_unsupported_context_header() {
    let src = "unknown.header: /foo/\n-\nhello: insert(hi)\n";
    let err = parse_talon_file(src).unwrap_err();
    assert!(err.message.contains("unsupported context header"));
}

#[test]
fn errors_on_settings_block_inside_command() {
    let src = "app.name: Code\n-\nmy command:\n    settings():\n";
    let err = parse_talon_file(src).unwrap_err();
    assert!(err.message.to_lowercase().contains("unsupported construct"));
}

#[test]
fn errors_on_indented_statement_without_command() {
    let src = "app.name: Code\n-\n    key(enter)\n";
    let err = parse_talon_file(src).unwrap_err();
    assert!(err.message.contains("indented"));
}
