use crate::speech::candidate::SpeechCandidate;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpeechError {
    Unavailable(String),
    RecognitionFailed(String),
}

impl std::fmt::Display for SpeechError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SpeechError::Unavailable(msg) => write!(f, "speech engine unavailable: {msg}"),
            SpeechError::RecognitionFailed(msg) => write!(f, "speech recognition failed: {msg}"),
        }
    }
}

impl std::error::Error for SpeechError {}

/// Command-oriented speech recognizer interface.
pub trait SpeechEngine {
    fn name(&self) -> &'static str {
        "speech-engine"
    }

    fn recognize_command(&mut self, phrases: &[String])
        -> Result<Vec<SpeechCandidate>, SpeechError>;
}
