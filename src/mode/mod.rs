use core::fmt;

/// High-level runtime mode.
///
/// Note: dictation is out of scope for this project; modes are command-centric.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
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
