use voice_runtime::mode::RuntimeMode;
use voice_runtime::runtime::command_inventory::CommandInventory;
use voice_runtime::runtime::python_host::PythonCompatibilityHost;
use voice_runtime::runtime::talon_ast::{TalonCommand, TalonContextHeader};
use voice_runtime::runtime::talon_parser::parse_talon_file;
use voice_runtime::speech::{
    best_command_match, best_command_match_with_host, SpeechCandidate,
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
fn best_match_prefers_exact_mode_specific_command() {
    let inventory = test_inventory([
        (RuntimeMode::Code, "take line"),
        (RuntimeMode::Global, "take"),
    ]);
    let candidates = vec![SpeechCandidate {
        transcript: "take line".into(),
        confidence: 0.81,
    }];
    assert_eq!(
        best_command_match(&candidates, &inventory, RuntimeMode::Code)
            .unwrap()
            .spoken_form(),
        "take line"
    );
}

#[test]
fn matches_rule_with_python_list_and_capture() {
    let file = parse_talon_file(
        "\
tag: browser
-
open hint <user.letters>: user.rango_open_hint(letters)
",
    )
    .unwrap();
    let inventory = CommandInventory::from_files(vec![file]);

    let mut host = PythonCompatibilityHost::default();
    host.load_python_source(
        r#"
from talon import Module, Context

mod = Module()
mod.list("letter")

ctx = Context()
ctx.lists["user.letter"] = {"air": "a", "bat": "b"}

@mod.capture(rule="{user.letter}+")
def letters(m) -> str:
    return "".join(m.letter_list)
"#,
        "letters.py",
    )
    .unwrap();

    let candidates = vec![SpeechCandidate {
        transcript: "open hint air bat".into(),
        confidence: 0.95,
    }];
    let matched = best_command_match_with_host(
        &candidates,
        &inventory,
        Some(&host),
        RuntimeMode::Browser,
    )
    .unwrap();

    assert_eq!(matched.spoken_form(), "open hint <user.letters>");
    assert_eq!(
        matched.captures().get("letters").map(String::as_str),
        Some("ab")
    );
    assert_eq!(
        matched.captures().get("user.letters").map(String::as_str),
        Some("ab")
    );
}

#[test]
fn unrecognized_transcript_returns_none_without_dictation_fallback() {
    let inventory = test_inventory([(RuntimeMode::Code, "take line")]);
    let candidates = vec![SpeechCandidate {
        transcript: "send an email to bob".into(),
        confidence: 0.99,
    }];
    assert!(best_command_match(&candidates, &inventory, RuntimeMode::Code).is_none());
}
