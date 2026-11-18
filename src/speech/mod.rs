//! Command-oriented speech engine interface, audio capture, and rule matching.

pub mod audio;
pub mod candidate;
pub mod engine;
pub mod matcher;
pub mod transcribe_engine;

pub use audio::{
    downmix_to_mono, enumerate_input_devices, frame_rms, list_input_devices, load_wav_16khz_mono,
    resample_linear_16khz, resolve_input_device_selector, AudioDeviceInfo, MicrophoneCapture,
    UtteranceSegmenter, VadConfig, TARGET_SAMPLE_RATE,
};
pub use candidate::{MatchedCommand, SpeechCandidate};
pub use engine::{SpeechEngine, SpeechError};
pub use matcher::{
    best_command_match, best_command_match_with_host, phrase_inventory_for_mode,
    phrase_inventory_for_mode_with_host,
};
pub use transcribe_engine::{
    discover_available_models, discover_default_model_path, discover_models_in_roots,
    normalize_command_transcript, rank_transcript_candidates, TranscribeCppEngine,
};
