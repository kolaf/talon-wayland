use crate::mode::{ModeSource, RuntimeMode};

/// A small, stable status projection for UI/CLI surfaces.
///
/// This keeps later frontend code from reaching into internal app state.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct RuntimeStatus {
    pub mode: RuntimeMode,
    pub mode_source: ModeSource,
}
