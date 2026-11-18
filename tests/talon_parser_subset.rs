use voice_runtime::runtime::talon_parser::parse_talon_file;

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
    assert_eq!(file.context.mode, Some(voice_runtime::mode::RuntimeMode::Code));
    assert_eq!(file.context.tags, vec!["user.tabs", "user.git"]);
    assert_eq!(file.context.code_language.as_deref(), Some("rust"));
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
    let stmt = &cmd.body[0];
    let voice_runtime::runtime::talon_ast::TalonBodyStmt::ActionInvoke { name, args } = stmt else {
        panic!("expected ActionInvoke");
    };
    assert_eq!(name, "user.rango_command_without_target");
    assert_eq!(args, &vec!["\"scrollDownPage\"".to_string(), "0.5".to_string()]);
}

#[test]
fn errors_on_unsupported_context_header() {
    let src = "title: /foo/\n-\nhello: insert(hi)\n";
    let err = parse_talon_file(src).unwrap_err();
    assert!(err.message.contains("unsupported context header"));
}

#[test]
fn errors_on_settings_block() {
    let src = "app.name: Code\n-\nsettings():\n    key_wait = 1.5\n";
    let err = parse_talon_file(src).unwrap_err();
    assert!(err.message.to_lowercase().contains("unsupported construct"));
}

#[test]
fn errors_on_indented_statement_without_command() {
    let src = "app.name: Code\n-\n    key(enter)\n";
    let err = parse_talon_file(src).unwrap_err();
    assert!(err.message.contains("indented"));
}
