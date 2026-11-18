use crate::backend::AppBackend;
use crate::mode::{ModeSource, RuntimeMode};

/// The main application object.
///
/// Task 1: keep this shape stable so later tasks can wire mode selection and backend
/// registration without reshaping the crate.
pub struct RuntimeApp {
    active_mode: RuntimeMode,
    mode_source: ModeSource,
    backends: Vec<Box<dyn AppBackend>>,
}

impl RuntimeApp {
    /// Create a minimal app instance suitable for unit tests.
    pub fn new_for_test() -> Self {
        Self {
            active_mode: RuntimeMode::Global,
            mode_source: ModeSource::Default,
            backends: Vec::new(),
        }
    }

    pub fn active_mode(&self) -> RuntimeMode {
        self.active_mode
    }

    pub fn mode_source(&self) -> ModeSource {
        self.mode_source
    }

    /// Cycle through the supported modes in a fixed, user-visible order.
    pub fn cycle_mode(&mut self) {
        self.active_mode = self.active_mode.next_in_cycle();
        self.mode_source = ModeSource::ManualOverride;
    }

    /// Manually set the active mode. This should beat any automatic selection.
    pub fn set_manual_mode(&mut self, mode: RuntimeMode) {
        self.active_mode = mode;
        self.mode_source = ModeSource::ManualOverride;
    }

    /// Clear a previous manual mode override.
    pub fn clear_manual_mode(&mut self) {
        if self.mode_source.is_manual_override() {
            self.active_mode = RuntimeMode::Global;
            self.mode_source = ModeSource::Default;
        }
    }

    pub fn backends(&self) -> &[Box<dyn AppBackend>] {
        &self.backends
    }
}
