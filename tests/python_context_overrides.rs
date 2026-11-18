use voice_runtime::mode::RuntimeMode;
use voice_runtime::runtime::python_host::{
    load_python_module_to_host, PythonCompatibilityHost, PythonHostError, RecordedActionCall,
};

#[test]
fn context_override_wins_over_default_action() {
    let host = PythonCompatibilityHost::from_test_override(
        "user.open_target",
        "browser",
        "browser_impl",
        "default_impl",
    );
    assert_eq!(
        host.resolve_for_test("user.open_target", "browser"),
        "browser_impl"
    );
}

#[test]
fn register_and_resolve_list_and_capture() {
    let mut host = PythonCompatibilityHost::default();
    host.register_list(
        "user.letters",
        vec!["a".to_string(), "b".to_string(), "c".to_string()],
    );
    host.register_capture("user.word", "(\\w+)");

    assert_eq!(
        host.list_values("user.letters").unwrap(),
        ["a", "b", "c"]
    );
    assert_eq!(host.capture_signature("user.word").unwrap(), "(\\w+)");
}

#[test]
fn python_module_and_context_action_override_executes_for_matching_mode() {
    let py_src = r#"
from talon import Module, Context, actions

mod = Module()

@mod.action_class
class Actions:
    def open_target(target: str) -> str:
        actions.key("ctrl-l")
        actions.insert(target)
        return "default_impl"

ctx = Context()
ctx.matches = """
os: linux
app: vscode
"""

@ctx.action_class("user")
class CodeActions:
    def open_target(target: str) -> str:
        actions.user.vscode("workbench.action.quickOpen", target)
        return "vscode_impl"
"#;

    let mut host = PythonCompatibilityHost::default();
    host.load_python_source(py_src, "open_target.py").unwrap();

    assert_eq!(
        host.resolve_action_impl("user.open_target", RuntimeMode::Code)
            .as_deref(),
        Some("CodeActions.open_target")
    );
    assert_eq!(
        host.resolve_action_impl("user.open_target", RuntimeMode::Global)
            .as_deref(),
        Some("Actions.open_target")
    );

    let (code_ret, code_trace) = host
        .invoke_action_with_trace("user.open_target", RuntimeMode::Code, &["main.rs".into()])
        .unwrap();
    assert_eq!(code_ret.as_deref(), Some("vscode_impl"));
    assert_eq!(
        code_trace,
        vec![RecordedActionCall {
            name: "user.vscode".into(),
            args: vec!["workbench.action.quickOpen".into(), "main.rs".into()],
        }]
    );

    let (global_ret, global_trace) = host
        .invoke_action_with_trace(
            "user.open_target",
            RuntimeMode::Global,
            &["https://example.com".into()],
        )
        .unwrap();
    assert_eq!(global_ret.as_deref(), Some("default_impl"));
    assert_eq!(
        global_trace,
        vec![
            RecordedActionCall {
                name: "key".into(),
                args: vec!["ctrl-l".into()],
            },
            RecordedActionCall {
                name: "insert".into(),
                args: vec!["https://example.com".into()],
            },
        ]
    );
}

#[test]
fn python_ctx_lists_and_capture_transform_spoken_tokens() {
    let py_src = r#"
from talon import Module, Context

mod = Module()
mod.list("letter", desc="Phonetic alphabet")

ctx = Context()
ctx.lists["user.letter"] = {
    "air": "a",
    "bat": "b",
    "cap": "c",
}

@mod.capture(rule="{user.letter}+")
def letters(m) -> str:
    return "".join(m.letter_list)
"#;

    let mut host = PythonCompatibilityHost::default();
    host.load_python_source(py_src, "letters.py").unwrap();

    assert_eq!(
        host.list_values("user.letter").unwrap(),
        &["air".to_string(), "bat".to_string(), "cap".to_string()]
    );
    assert_eq!(
        host.list_mapping("user.letter").unwrap().get("air").map(String::as_str),
        Some("a")
    );
    assert_eq!(
        host.capture_signature("user.letters"),
        Some("{user.letter}+")
    );

    let captured = host
        .invoke_capture(
            "user.letters",
            RuntimeMode::Global,
            &["air".into(), "bat".into()],
        )
        .unwrap();
    assert_eq!(captured, "ab");
}

#[test]
fn python_actions_next_and_host_isolation() {
    let py_a = r#"
from talon import Module, Context, actions

mod = Module()

@mod.action_class
class Actions:
    def greet(name: str) -> str:
        actions.insert(f"hello {name}")
        return "base"

ctx = Context()
ctx.matches = "tag: browser"

@ctx.action_class("user")
class BrowserActions:
    def greet(name: str) -> str:
        actions.key("esc")
        return actions.next(name)
"#;

    let py_b = r#"
from talon import Module
mod = Module()
@mod.action_class
class Other:
    def ping() -> str:
        return "pong"
"#;

    let mut host_a = PythonCompatibilityHost::default();
    let mut host_b = PythonCompatibilityHost::default();
    host_a.load_python_source(py_a, "a.py").unwrap();
    host_b.load_python_source(py_b, "b.py").unwrap();

    // Host A and Host B remain isolated.
    assert!(host_a.resolve_action_impl("user.ping", RuntimeMode::Global).is_none());
    assert!(host_b.resolve_action_impl("user.greet", RuntimeMode::Global).is_none());

    let (ret, trace) = host_a
        .invoke_action_with_trace("user.greet", RuntimeMode::Browser, &["world".into()])
        .unwrap();
    assert_eq!(ret.as_deref(), Some("base"));
    assert_eq!(
        trace,
        vec![
            RecordedActionCall {
                name: "key".into(),
                args: vec!["esc".into()],
            },
            RecordedActionCall {
                name: "insert".into(),
                args: vec!["hello world".into()],
            },
        ]
    );
}

#[test]
fn python_syntax_error_returns_parse_error() {
    let mut host = PythonCompatibilityHost::default();
    let err = host
        .load_python_source("def broken(:\n    pass\n", "broken.py")
        .unwrap_err();
    assert!(matches!(err, PythonHostError::Parse { .. }));
}

#[test]
fn load_python_file_supports_relative_csv_loading() {
    let dir = std::env::temp_dir().join(format!(
        "voice_runtime_py_test_{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let csv_path = dir.join("symbols.csv");
    let py_path = dir.join("symbols.py");

    std::fs::write(&csv_path, "spoken,value\nplus,+\nminus,-\n").unwrap();
    std::fs::write(
        &py_path,
        r#"
import csv
from pathlib import Path
from talon import Module, Context

mod = Module()
mod.list("symbol_key", desc="Symbols")
ctx = Context()

with open(Path(__file__).parent / "symbols.csv", newline="") as f:
    rows = list(csv.DictReader(f))
ctx.lists["user.symbol_key"] = {r["spoken"]: r["value"] for r in rows}
"#,
    )
    .unwrap();

    let host = load_python_module_to_host(&py_path).unwrap();
    assert_eq!(
        host.list_mapping("user.symbol_key")
            .unwrap()
            .get("plus")
            .map(String::as_str),
        Some("+")
    );
    assert_eq!(
        host.list_mapping("user.symbol_key")
            .unwrap()
            .get("minus")
            .map(String::as_str),
        Some("-")
    );

    let _ = std::fs::remove_dir_all(&dir);
}
