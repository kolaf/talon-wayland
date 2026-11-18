/// Command-oriented speech recognizer interface.
///
/// Task 1: boundary only; implementations come in later tasks.
pub trait SpeechEngine {
    fn name(&self) -> &'static str;
}
