use voice_runtime::mode::RuntimeMode;
use voice_runtime::runtime::command_inventory::CommandInventory;
use voice_runtime::runtime::talon_ast::{TalonCommand, TalonContextHeader};

fn test_command(mode: RuntimeMode, spoken: &str) -> TalonCommand {
    TalonCommand {
        spoken_form: spoken.to_string(),
        body: vec![],
        context: TalonContextHeader {
            app: None,
            app_name: None,
            os: None,
            mode: Some(mode),
            tags: vec![],
            code_language: None,
        },
        source_path: None,
        line: 1,
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
