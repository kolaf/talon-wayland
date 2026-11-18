use voice_runtime::mode::RuntimeMode;
use voice_runtime::runtime::command_inventory::CommandInventory;
use voice_runtime::runtime::talon_ast::{TalonCommand, TalonContextHeader};
use voice_runtime::runtime::talon_parser::parse_talon_file;

fn test_command(mode: RuntimeMode, spoken: &str) -> TalonCommand {
    TalonCommand {
        spoken_form: spoken.to_string(),
        context: TalonContextHeader {
            mode: Some(mode),
            ..Default::default()
        },
        line: 1,
        ..Default::default()
    }
}

#[test]
fn inventory_filters_code_commands_by_mode() {
    let inventory = CommandInventory::from_test_commands(vec![
        test_command(RuntimeMode::Code, "take line"),
        test_command(RuntimeMode::Browser, "open hint"),
    ]);
    let spoken = inventory
        .commands_for_mode(RuntimeMode::Code)
        .into_iter()
        .map(|cmd| cmd.spoken_form.as_str())
        .collect::<Vec<_>>();
    assert_eq!(spoken, vec!["take line"]);
}

#[test]
fn transitive_tag_activation_routes_commands_by_mode() {
    let vscode_file = parse_talon_file(
        "\
app: vscode
-
tag(): user.tabs
",
    )
    .unwrap();

    let tabs_file = parse_talon_file(
        "\
tag: user.tabs
-
tab next: key(ctrl-pagedown)
",
    )
    .unwrap();

    let browser_file = parse_talon_file(
        "\
tag: browser
-
open hint: user.rango_command_without_target(\"clickElement\")
",
    )
    .unwrap();

    let inventory = CommandInventory::from_files(vec![vscode_file, tabs_file, browser_file]);

    let code_cmds: Vec<&str> = inventory
        .commands_for_mode(RuntimeMode::Code)
        .into_iter()
        .map(|c| c.spoken_form.as_str())
        .collect();
    assert_eq!(code_cmds, vec!["tab next"]);

    let browser_cmds: Vec<&str> = inventory
        .commands_for_mode(RuntimeMode::Browser)
        .into_iter()
        .map(|c| c.spoken_form.as_str())
        .collect();
    assert_eq!(browser_cmds, vec!["open hint"]);

    assert!(inventory.commands_for_mode(RuntimeMode::Terminal).is_empty());
}

#[test]
fn excludes_non_linux_and_dictation_commands() {
    let win_file = parse_talon_file("os: windows\napp: vscode\n-\nwin cmd: key(win-r)\n").unwrap();
    let dictation_file =
        parse_talon_file("mode: dictation\n-\nscratch that: key(ctrl-z)\n").unwrap();
    let linux_global =
        parse_talon_file("os: linux\nmode: command\n-\ncopy that: edit.copy()\n").unwrap();
    let headerless = parse_talon_file("slap: key(enter)\n").unwrap();

    let inventory =
        CommandInventory::from_files(vec![win_file, dictation_file, linux_global, headerless]);

    let global_cmds: Vec<&str> = inventory
        .commands_for_mode(RuntimeMode::Global)
        .into_iter()
        .map(|c| c.spoken_form.as_str())
        .collect();
    assert_eq!(global_cmds, vec!["copy that", "slap"]);

    let effective_code: Vec<&str> = inventory
        .effective_commands_for_mode(RuntimeMode::Code)
        .into_iter()
        .map(|c| c.spoken_form.as_str())
        .collect();
    assert_eq!(effective_code, vec!["copy that", "slap"]);
}
