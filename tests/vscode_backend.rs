use voice_runtime::backend::vscode::{VsCodeBackend, VsCodeCommand};
use voice_runtime::runtime::errors::ExecutionError;

fn test_vscode_command() -> VsCodeCommand {
    VsCodeCommand {
        command_name: "cursorless.takeLine".into(),
        args: vec![],
    }
}

#[test]
fn vscode_backend_rejects_send_when_disconnected() {
    let backend = VsCodeBackend::new_for_test_disconnected();
    let err = backend.send_for_test(test_vscode_command()).unwrap_err();
    assert!(matches!(err, ExecutionError::BackendUnavailable(_)));
}

#[test]
fn stale_vscode_connection_reports_backend_unavailable_after_disconnect() {
    let mut backend = VsCodeBackend::new_for_test_connected();
    backend.send(test_vscode_command()).unwrap();
    backend.disconnect();
    let err = backend.send(test_vscode_command()).unwrap_err();
    assert!(matches!(err, ExecutionError::BackendUnavailable(_)));
}
