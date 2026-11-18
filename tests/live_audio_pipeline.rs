use std::collections::VecDeque;
use std::f32::consts::PI;
use voice_runtime::app::RuntimeApp;
use voice_runtime::config::{
    load_selected_audio_device, load_selected_model, save_selected_audio_device,
    save_selected_model,
};
use voice_runtime::mode::RuntimeMode;
use voice_runtime::speech::{
    discover_models_in_roots, downmix_to_mono, frame_rms, load_wav_16khz_mono,
    normalize_command_transcript, rank_transcript_candidates, resample_linear_16khz,
    resolve_input_device_selector, AudioDeviceInfo, SpeechCandidate, SpeechEngine, SpeechError,
    UtteranceSegmenter, VadConfig, TARGET_SAMPLE_RATE,
};

struct MockSpeechEngine {
    responses: VecDeque<Vec<SpeechCandidate>>,
    last_phrases: Vec<String>,
}

impl MockSpeechEngine {
    fn new(responses: impl IntoIterator<Item = Vec<SpeechCandidate>>) -> Self {
        Self {
            responses: responses.into_iter().collect(),
            last_phrases: Vec::new(),
        }
    }
}

impl SpeechEngine for MockSpeechEngine {
    fn name(&self) -> &'static str {
        "mock-speech"
    }

    fn recognize_command(
        &mut self,
        phrases: &[String],
    ) -> Result<Vec<SpeechCandidate>, SpeechError> {
        self.last_phrases = phrases.to_vec();
        Ok(self.responses.pop_front().unwrap_or_default())
    }
}

#[test]
fn downmix_and_resample_48khz_stereo_to_16khz_mono() {
    // 480 stereo frames (10 ms at 48 kHz) -> 160 mono samples at 16 kHz.
    let mut stereo = Vec::with_capacity(480 * 2);
    for _ in 0..480 {
        stereo.push(0.4);
        stereo.push(0.2);
    }
    let mono = downmix_to_mono(&stereo, 2);
    assert_eq!(mono.len(), 480);
    assert!((mono[0] - 0.3).abs() < 1e-5);

    let resampled = resample_linear_16khz(&mono, 48_000);
    assert_eq!(resampled.len(), 160);
    assert!((resampled[0] - 0.3).abs() < 1e-5);
}

#[test]
fn vad_utterance_segmenter_captures_preroll_speech_and_trailing_silence() {
    let mut segmenter = UtteranceSegmenter::new(VadConfig {
        speech_rms_threshold: 0.02,
        silence_rms_threshold: 0.01,
        min_speech_ms: 128,
        min_silence_ms: 192,
        max_speech_ms: 4_000,
        pre_roll_ms: 64,
    });

    // Leading silence (10 frames = 320 ms)
    let silence = vec![0.0f32; 512 * 10];
    assert!(segmenter.push_samples(&silence).is_none());

    // Speech sine wave (8 frames = 256 ms, 440 Hz, amp 0.2 -> RMS ~0.141)
    let speech_len = 512 * 8;
    let speech: Vec<f32> = (0..speech_len)
        .map(|i| 0.2 * (2.0 * PI * 440.0 * (i as f32) / (TARGET_SAMPLE_RATE as f32)).sin())
        .collect();
    assert!(frame_rms(&speech) > 0.1);
    assert!(segmenter.push_samples(&speech).is_none());

    // Trailing silence (8 frames = 256 ms > min_silence_ms 192 ms) triggers emission.
    let trailing = vec![0.0f32; 512 * 8];
    let utt = segmenter
        .push_samples(&trailing)
        .expect("utterance should be segmented after trailing silence");

    // Pre-roll (~64 ms = 1024 samples) + speech (4096) + trailing silence (~192 ms = 3072)
    assert!(utt.len() >= speech_len + 1024);
}

#[test]
fn wav_loader_reads_and_converts_48khz_stereo_int16_wav() {
    let wav_path = std::env::temp_dir().join(format!(
        "voice_runtime_audio_test_{}.wav",
        std::process::id()
    ));

    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: 48_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    {
        let mut writer = hound::WavWriter::create(&wav_path, spec).unwrap();
        // 0.25 seconds at 48 kHz = 12_000 frames
        for i in 0..12_000 {
            let s = (0.5 * (2.0 * PI * 220.0 * (i as f32) / 48_000.0).sin() * 32767.0) as i16;
            writer.write_sample(s).unwrap();
            writer.write_sample(s).unwrap();
        }
        writer.finalize().unwrap();
    }

    let pcm = load_wav_16khz_mono(&wav_path).unwrap();
    let _ = std::fs::remove_file(&wav_path);

    // 0.25 seconds at 16 kHz = 4_000 mono samples
    assert_eq!(pcm.len(), 4_000);
    assert!(frame_rms(&pcm) > 0.25);
}

#[test]
fn transcript_normalization_and_fuzzy_candidate_ranking() {
    assert_eq!(
        normalize_command_transcript("  Open Hint, Air! "),
        "open hint air"
    );

    let phrases = vec![
        "take line".to_string(),
        "open hint air".to_string(),
        "status check".to_string(),
    ];
    let cands = rank_transcript_candidates("Take line.", &phrases);
    assert_eq!(cands[0].transcript, "take line");
    assert!(cands[0].confidence > 0.9);

    let fuzzy_cands = rank_transcript_candidates("Take lines.", &phrases);
    assert!(fuzzy_cands.iter().any(|c| c.transcript == "take line"));
}

#[test]
fn app_recognize_and_execute_supports_voice_mode_switch_and_command_execution() {
    let mut app = RuntimeApp::new_for_end_to_end_test(RuntimeMode::Code);
    let mut engine = MockSpeechEngine::new([
        vec![SpeechCandidate {
            transcript: "browser mode".into(),
            confidence: 0.95,
        }],
        vec![SpeechCandidate {
            transcript: "open hint air".into(),
            confidence: 0.95,
        }],
    ]);

    let heard1 = app.recognize_and_execute_once(&mut engine).unwrap();
    assert_eq!(heard1.as_deref(), Some("browser mode"));
    assert_eq!(app.active_mode(), RuntimeMode::Browser);

    let heard2 = app.recognize_and_execute_once(&mut engine).unwrap();
    assert_eq!(heard2.as_deref(), Some("open hint air"));
    assert_eq!(app.last_backend_invocation_for_test(), "browser");
    assert!(engine.last_phrases.iter().any(|p| p == "open hint air"));
}

#[test]
fn resolve_input_device_selector_matches_index_exact_and_substring() {
    let devices = vec![
        AudioDeviceInfo {
            index: 1,
            name: "Built-in Analog Stereo".into(),
            is_default: true,
            sample_rate: Some(48_000),
            channels: Some(2),
        },
        AudioDeviceInfo {
            index: 2,
            name: "Blue Yeti USB Microphone".into(),
            is_default: false,
            sample_rate: Some(48_000),
            channels: Some(1),
        },
    ];

    assert_eq!(
        resolve_input_device_selector("2", &devices).unwrap().name,
        "Blue Yeti USB Microphone"
    );
    assert_eq!(
        resolve_input_device_selector("built-in analog stereo", &devices)
            .unwrap()
            .index,
        1
    );
    assert_eq!(
        resolve_input_device_selector("yeti", &devices)
            .unwrap()
            .index,
        2
    );

    assert!(resolve_input_device_selector("3", &devices).is_err());
    assert!(resolve_input_device_selector("nonexistent", &devices).is_err());
}

#[test]
fn runtime_app_selects_clears_and_exposes_active_audio_device_in_status() {
    let devices = vec![
        AudioDeviceInfo {
            index: 1,
            name: "Default Mic".into(),
            is_default: true,
            sample_rate: Some(48_000),
            channels: Some(1),
        },
        AudioDeviceInfo {
            index: 2,
            name: "Headset Mic".into(),
            is_default: false,
            sample_rate: Some(48_000),
            channels: Some(1),
        },
    ];

    let mut app = RuntimeApp::new_for_test();
    assert_eq!(app.active_audio_device(), None);
    assert_eq!(app.status().audio_device, None);

    let chosen = app.select_audio_device("headset", &devices).unwrap();
    assert_eq!(chosen.name, "Headset Mic");
    assert_eq!(app.active_audio_device(), Some("Headset Mic"));
    assert_eq!(app.status().audio_device.as_deref(), Some("Headset Mic"));

    app.clear_audio_device();
    assert_eq!(app.active_audio_device(), None);
    assert_eq!(app.status().audio_device, None);
}

#[test]
fn audio_device_selection_persists_and_clears_in_config_dir() {
    let dir = std::env::temp_dir().join(format!(
        "voice_runtime_audio_cfg_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);

    assert_eq!(load_selected_audio_device(Some(&dir)), None);

    save_selected_audio_device(Some("Blue Yeti USB Microphone"), Some(&dir)).unwrap();
    assert_eq!(
        load_selected_audio_device(Some(&dir)).as_deref(),
        Some("Blue Yeti USB Microphone")
    );

    save_selected_audio_device(None, Some(&dir)).unwrap();
    assert_eq!(load_selected_audio_device(Some(&dir)), None);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn discovers_huggingface_hub_conformer_models_and_excludes_vad() {
    let root = std::env::temp_dir().join(format!(
        "voice_runtime_hf_models_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);

    let hf_parakeet = root.join(
        "models--handy-computer--parakeet-tdt-0.6b-v2-gguf/snapshots/abc123",
    );
    let hf_blobs = root.join("models--handy-computer--parakeet-tdt-0.6b-v2-gguf/blobs");
    let whisper_dir = root.join("whisper");
    let vad_dir = root.join("vad");

    std::fs::create_dir_all(&hf_parakeet).unwrap();
    std::fs::create_dir_all(&hf_blobs).unwrap();
    std::fs::create_dir_all(&whisper_dir).unwrap();
    std::fs::create_dir_all(&vad_dir).unwrap();

    let parakeet_file = hf_parakeet.join("parakeet-tdt-0.6b-v2-Q8_0.gguf");
    let whisper_file = whisper_dir.join("ggml-base.en.bin");
    let vad_file = vad_dir.join("silero-vad-v6.2.gguf");
    let blob_file = hf_blobs.join("deadbeef0000");

    std::fs::write(&parakeet_file, b"gguf").unwrap();
    std::fs::write(&whisper_file, b"ggml").unwrap();
    std::fs::write(&vad_file, b"vad").unwrap();
    std::fs::write(&blob_file, b"blob").unwrap();

    let discovered = discover_models_in_roots(std::slice::from_ref(&root));
    assert_eq!(discovered, vec![parakeet_file, whisper_file]);

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn speech_model_selection_persists_and_clears_in_config_dir() {
    let dir = std::env::temp_dir().join(format!(
        "voice_runtime_model_cfg_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);

    assert_eq!(load_selected_model(Some(&dir)), None);

    let model = std::path::PathBuf::from("/tmp/parakeet-tdt-0.6b-v2-Q8_0.gguf");
    save_selected_model(Some(&model), Some(&dir)).unwrap();
    assert_eq!(load_selected_model(Some(&dir)), Some(model));

    save_selected_model(None, Some(&dir)).unwrap();
    assert_eq!(load_selected_model(Some(&dir)), None);

    let _ = std::fs::remove_dir_all(&dir);
}
