#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutionError {
    UnsupportedFeature(String),
    BackendUnavailable(String),
    ActionNotFound(String),
    ContextMismatch(String),
}

impl std::fmt::Display for ExecutionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExecutionError::UnsupportedFeature(msg) => write!(f, "unsupported feature: {msg}"),
            ExecutionError::BackendUnavailable(msg) => write!(f, "backend unavailable: {msg}"),
            ExecutionError::ActionNotFound(msg) => write!(f, "action not found: {msg}"),
            ExecutionError::ContextMismatch(msg) => write!(f, "context mismatch: {msg}"),
        }
    }
}

impl std::error::Error for ExecutionError {}
