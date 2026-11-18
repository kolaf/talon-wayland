use voice_runtime::runtime::talon_parser::parse_talon_file;

#[test]
fn parses_context_header_and_simple_command() {
    let src = "app.name: Code\ntag: user.tabs\n-\nfocus left: key(ctrl-left)\n";
    let file = parse_talon_file(src).unwrap();
    assert_eq!(file.context.app_name.as_deref(), Some("Code"));
    assert_eq!(file.context.tags, vec!["user.tabs"]);
    assert_eq!(file.commands[0].spoken_form, "focus left");
}
