use voice_runtime::mode::RuntimeMode;
use voice_runtime::runtime::command_inventory::CommandInventory;
use voice_runtime::runtime::python_host::PythonCompatibilityHost;
use voice_runtime::runtime::talon_ast::{TalonCommand, TalonContextHeader};
use voice_runtime::runtime::talon_parser::parse_talon_file;
use voice_runtime::speech::{
    phrase_inventory_for_mode, phrase_inventory_for_mode_with_host,
};

fn test_inventory(items: impl IntoIterator<Item = (RuntimeMode, &'static str)>) -> CommandInventory {
    let commands = items
        .into_iter()
        .map(|(mode, spoken)| TalonCommand {
            spoken_form: spoken.to_string(),
            context: TalonContextHeader {
                mode: Some(mode),
                ..Default::default()
            },
            line: 1,
            ..Default::default()
        })
        .collect();
    CommandInventory::from_test_commands(commands)
}

#[test]
fn code_mode_phrase_inventory_excludes_browser_only_commands() {
    let inventory = test_inventory([
        (RuntimeMode::Code, "take line"),
        (RuntimeMode::Browser, "open hint"),
    ]);
    assert_eq!(
        phrase_inventory_for_mode(&inventory, RuntimeMode::Code),
        vec!["take line"]
    );
}

#[test]
fn expands_list_rules_in_phrase_inventory_using_python_host() {
    let file = parse_talon_file(
        "\
tag: browser
-
hint {user.letter}: user.rango_open_hint(letter)
",
    )
    .unwrap();
    let inventory = CommandInventory::from_files(vec![file]);

    let mut host = PythonCompatibilityHost::default();
    host.register_list("user.letter", vec!["air".into(), "bat".into()]);

    assert_eq!(
        phrase_inventory_for_mode_with_host(&inventory, &host, RuntimeMode::Browser),
        vec!["hint air", "hint bat"]
    );
}
