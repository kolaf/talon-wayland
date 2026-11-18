/// Backend integration point for controlling external tools (e.g. editor, browser, terminal).
///
/// Task 1: boundary only.
pub trait AppBackend {
    fn name(&self) -> &'static str;
}
