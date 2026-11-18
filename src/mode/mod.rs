use core::fmt;

pub use crate::frontend::focus::{infer_mode, ForegroundAppInfo, ForegroundDetector};

/// High-level runtime mode.
///
/// Note: dictation is out of scope for this project; modes are command-centric.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum RuntimeMode {
    Global,
    Code,
    Browser,
    Terminal,
}

impl fmt::Display for RuntimeMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            RuntimeMode::Global => "global",
            RuntimeMode::Code => "code",
            RuntimeMode::Browser => "browser",
            RuntimeMode::Terminal => "terminal",
        };
        f.write_str(s)
    }
}

/// Where the currently-active mode came from.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum ModeSource {
    /// Default mode (boot / fallback).
    Default,
    /// Mode selected based on detected foreground application.
    Foreground,
    /// User manually set the mode; should beat foreground detection.
    ManualOverride,
}

impl ModeSource {
    pub fn is_manual_override(&self) -> bool {
        matches!(self, ModeSource::ManualOverride)
    }
}

impl RuntimeMode {
    /// Next mode in the cycle order used by the manual cycle shortcut.
    ///
    /// Order: Global → Code → Browser → Terminal → Global
    pub fn next_in_cycle(self) -> Self {
        match self {
            RuntimeMode::Global => RuntimeMode::Code,
            RuntimeMode::Code => RuntimeMode::Browser,
            RuntimeMode::Browser => RuntimeMode::Terminal,
            RuntimeMode::Terminal => RuntimeMode::Global,
        }
    }
}
