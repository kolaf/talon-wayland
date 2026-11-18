use crate::backend::AppBackend;
use crate::mode::RuntimeMode;

/// The main application object.
///
/// Task 1: keep this shape stable so later tasks can wire mode selection and backend
/// registration without reshaping the crate.
pub struct RuntimeApp {
    active_mode: RuntimeMode,
    backends: Vec<Box<dyn AppBackend>>,
}

impl RuntimeApp {
    /// Create a minimal app instance suitable for unit tests.
    pub fn new_for_test() -> Self {
        Self {
            active_mode: RuntimeMode::Global,
            backends: Vec::new(),
        }
    }

    pub fn active_mode(&self) -> RuntimeMode {
        self.active_mode
    }

    pub fn backends(&self) -> &[Box<dyn AppBackend>] {
        &self.backends
    }
}
