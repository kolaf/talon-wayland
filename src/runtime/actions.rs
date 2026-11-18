use crate::mode::RuntimeMode;
use std::collections::HashMap;

/// Minimal resolved context used for action resolution.
///
/// This is intentionally a narrow compatibility type. It can be expanded later without
/// changing the action registry's external API surface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedContext {
    pub mode: RuntimeMode,
}

impl ResolvedContext {
    pub fn new(mode: RuntimeMode) -> Self {
        Self { mode }
    }
}

/// Handle returned by the registry.
///
/// For now this is just a stable identifier for tests and for wiring the executor later.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionHandle {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionError {
    NotFound { name: String },
}

impl std::fmt::Display for ActionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ActionError::NotFound { name } => write!(f, "action not found: {name}"),
        }
    }
}

impl std::error::Error for ActionError {}

/// Runtime action boundary.
///
/// v1 actions are name-addressable and optionally allow a context-specific override.
pub trait RuntimeAction: Send + Sync {
    fn name(&self) -> &str;
}

#[derive(Debug, Default)]
pub struct ActionRegistry {
    builtins: HashMap<String, ActionHandle>,
}

impl ActionRegistry {
    pub fn register_builtin(&mut self, name: impl Into<String>) {
        let name = name.into();
        self.builtins
            .insert(name.clone(), ActionHandle { name });
    }

    pub fn resolve(&self, name: &str) -> Result<ActionHandle, ActionError> {
        self.builtins
            .get(name)
            .cloned()
            .ok_or_else(|| ActionError::NotFound {
                name: name.to_string(),
            })
    }
}

/// Registers the builtin action names expected by the supported Talon subset.
///
/// This is a compatibility shim; the actual behavior will be implemented later.
pub fn register_builtin_actions(registry: &mut ActionRegistry) {
    // Core text / keyboard-ish actions referenced frequently.
    registry.register_builtin("insert");
    registry.register_builtin("key");
    registry.register_builtin("sleep");

    // Common Talon community patterns.
    registry.register_builtin("app.notify");
    registry.register_builtin("edit.paste");
    registry.register_builtin("edit.copy");
}

/// Resolves an action by name within a resolved context.
///
/// In v1, context does not affect builtin lookup; context-specific resolution lives in
/// `PythonCompatibilityHost`.
pub fn resolve_action(
    registry: &ActionRegistry,
    name: &str,
    _context: &ResolvedContext,
) -> Result<ActionHandle, ActionError> {
    registry.resolve(name)
}
