use voice_runtime::backend::browser::{BrowserBackend, BrowserCommand};
use voice_runtime::runtime::errors::ExecutionError;

fn test_browser_command() -> BrowserCommand {
    BrowserCommand {
        command_name: "rango.openHint".into(),
        args: vec!["a".into()],
    }
}

#[test]
fn browser_backend_reports_missing_extension() {
    let backend = BrowserBackend::new_for_test_disconnected();
    let err = backend.send_for_test(test_browser_command()).unwrap_err();
    assert!(matches!(err, ExecutionError::BackendUnavailable(_)));
}

#[test]
fn stale_browser_extension_disconnect_reports_backend_unavailable() {
    let mut backend = BrowserBackend::new_for_test_connected();
    backend.send(test_browser_command()).unwrap();
    backend.disconnect();
    let err = backend.send(test_browser_command()).unwrap_err();
    assert!(matches!(err, ExecutionError::BackendUnavailable(_)));
}
