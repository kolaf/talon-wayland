use std::collections::VecDeque;
use voice_runtime::app::RuntimeApp;
use voice_runtime::frontend::ipc::{send_ipc_command, IpcCommand, IpcControlServer};
use voice_runtime::mode::RuntimeMode;
use voice_runtime::speech::{SpeechCandidate, SpeechEngine, SpeechError};

struct QueueSpeechEngine {
    batches: VecDeque<Vec<SpeechCandidate>>,
    calls: usize,
}

impl QueueSpeechEngine {
    fn new(batches: impl IntoIterator<Item = Vec<SpeechCandidate>>) -> Self {
        Self {
            batches: batches.into_iter().collect(),
            calls: 0,
        }
    }
}

impl SpeechEngine for QueueSpeechEngine {
    fn recognize_command(
        &mut self,
        _phrases: &[String],
    ) -> Result<Vec<SpeechCandidate>, SpeechError> {
        self.calls += 1;
        Ok(self.batches.pop_front().unwrap_or_default())
    }
}

#[test]
fn toggling_listening_pauses_and_resumes_speech_recognition() {
    let mut app = RuntimeApp::new_for_end_to_end_test(RuntimeMode::Browser);
    assert!(app.is_listening());
    assert!(app.status().listening);

    let mut engine = QueueSpeechEngine::new([vec![SpeechCandidate {
        transcript: "open hint air".into(),
        confidence: 0.95,
    }]]);

    assert!(!app.toggle_listening());
    assert!(!app.status().listening);
    let paused_res = app.recognize_and_execute_once(&mut engine).unwrap();
    assert_eq!(paused_res, None);
    assert_eq!(engine.calls, 0);

    assert!(app.toggle_listening());
    assert!(app.status().listening);
    let resumed_res = app.recognize_and_execute_once(&mut engine).unwrap();
    assert_eq!(resumed_res.as_deref(), Some("open hint air"));
    assert_eq!(engine.calls, 1);
    assert_eq!(app.last_backend_invocation_for_test(), "browser");
}

#[test]
fn wayland_ipc_control_server_handles_toggle_mode_device_and_status() {
    let sock_path = std::env::temp_dir().join(format!(
        "voice_runtime_ipc_test_{}.sock",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&sock_path);

    let server = IpcControlServer::bind(&sock_path).unwrap();
    let mut app = RuntimeApp::new_for_end_to_end_test(RuntimeMode::Global);

    let client = std::thread::spawn({
        let sock_path = sock_path.clone();
        move || {
            let r1 = send_ipc_command(&sock_path, &IpcCommand::ToggleListening).unwrap();
            let r2 = send_ipc_command(&sock_path, &IpcCommand::SetMode(RuntimeMode::Code)).unwrap();
            let r3 = send_ipc_command(&sock_path, &IpcCommand::CycleMode).unwrap();
            let r4 =
                send_ipc_command(&sock_path, &IpcCommand::SelectDevice("Headset".into())).unwrap();
            let r5 = send_ipc_command(&sock_path, &IpcCommand::Status).unwrap();
            (r1, r2, r3, r4, r5)
        }
    });

    let mut total_handled = 0;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while total_handled < 5 && std::time::Instant::now() < deadline {
        let handled = server.poll_once(&mut app);
        total_handled += handled.len();
        if total_handled < 5 {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    let (r1, r2, r3, r4, r5) = client.join().unwrap();
    assert!(r1.contains("listening=false"));
    assert!(r2.contains("mode=code"));
    assert!(r3.contains("mode=browser"));
    assert!(r4.contains("device=Headset"));
    assert_eq!(r5, r4);

    assert!(!app.is_listening());
    assert_eq!(app.active_mode(), RuntimeMode::Browser);
    assert_eq!(app.active_audio_device(), Some("Headset"));
}
