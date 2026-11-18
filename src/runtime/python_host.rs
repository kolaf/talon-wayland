use crate::mode::RuntimeMode;
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug)]
pub enum PythonHostError {
    Io { path: String, source: std::io::Error },
    Unsupported { message: String },
    Parse { path: String, message: String },
}

impl std::fmt::Display for PythonHostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PythonHostError::Io { path, source } => write!(f, "{path}: io error: {source}"),
            PythonHostError::Unsupported { message } => {
                write!(f, "unsupported python host op: {message}")
            }
            PythonHostError::Parse { path, message } => write!(f, "{path}: parse error: {message}"),
        }
    }
}

impl std::error::Error for PythonHostError {}

/// Narrow compatibility host that models the minimal subset of Talon's Python action system
/// required by the target repos.
///
/// This does **not** execute python. It only stores module-defined data we need:
///
/// - module-defined actions (default + context overrides)
/// - list registration
/// - capture registration
#[derive(Debug, Default)]
pub struct PythonCompatibilityHost {
    // For v1 tests, an action "implementation" is just an opaque token.
    default_impl: HashMap<String, String>,
    per_mode_impl: HashMap<(String, RuntimeMode), String>,

    // registered lists/captures (needed by grammars but not executed here)
    lists: HashMap<String, Vec<String>>,
    captures: HashMap<String, String>,
}

impl PythonCompatibilityHost {
    /// Test helper: create a host with one action name and a mode-specific override.
    pub fn from_test_override(
        action_name: &str,
        mode: &str,
        override_impl: &str,
        default_impl: &str,
    ) -> Self {
        let mut host = Self::default();
        let mode = parse_mode(mode).expect("test helper given invalid mode");
        host.register_action_default(action_name.to_string(), default_impl.to_string());
        host.register_action_override(action_name.to_string(), mode, override_impl.to_string());
        host
    }

    /// Test helper: resolve and return the selected implementation token.
    ///
    /// This is intentionally strict: it panics if the action cannot be resolved.
    pub fn resolve_for_test(&self, action_name: &str, mode: &str) -> String {
        let mode = parse_mode(mode).expect("test helper given invalid mode");
        self.resolve_action_impl(action_name, mode)
            .unwrap_or_else(|| panic!("test helper could not resolve action: {action_name}"))
    }

    pub fn register_list(&mut self, name: impl Into<String>, values: Vec<String>) {
        self.lists.insert(name.into(), values);
    }

    pub fn list_values(&self, name: &str) -> Option<&[String]> {
        self.lists.get(name).map(|v| v.as_slice())
    }

    pub fn register_capture(&mut self, name: impl Into<String>, signature: impl Into<String>) {
        self.captures.insert(name.into(), signature.into());
    }

    pub fn capture_signature(&self, name: &str) -> Option<&str> {
        self.captures.get(name).map(|s| s.as_str())
    }

    pub fn register_action_default(
        &mut self,
        name: impl Into<String>,
        implementation: impl Into<String>,
    ) {
        self.default_impl
            .insert(name.into(), implementation.into());
    }

    pub fn register_action_override(
        &mut self,
        name: impl Into<String>,
        mode: RuntimeMode,
        implementation: impl Into<String>,
    ) {
        self.per_mode_impl
            .insert((name.into(), mode), implementation.into());
    }

    pub fn resolve_action_impl(&self, name: &str, mode: RuntimeMode) -> Option<String> {
        if let Some(v) = self
            .per_mode_impl
            .get(&(name.to_string(), mode))
            .cloned()
        {
            return Some(v);
        }
        self.default_impl.get(name).cloned()
    }
}

/// Loads a “python module” in the narrow, non-executing compatibility format.
///
/// This keeps the brief's signature (returns `()`), while still performing meaningful work.
/// Downstream wiring can switch to `load_python_module_to_host()` when it needs the data.
pub fn load_python_module(path: &Path) -> Result<(), PythonHostError> {
    let _host = load_python_module_to_host(path)?;
    Ok(())
}

/// v1: parses a tiny line-oriented subset representing Talon python registrations.
///
/// Supported directives (whitespace tolerant; `#` comments supported):
///
/// - `actions: <name> = <token>`
/// - `actions.<mode>: <name> = <token>`
/// - `list: <name> = a,b,c`
/// - `capture: <name> = <signature>`
pub fn load_python_module_to_host(path: &Path) -> Result<PythonCompatibilityHost, PythonHostError> {
    let src = std::fs::read_to_string(path).map_err(|e| PythonHostError::Io {
        path: path.display().to_string(),
        source: e,
    })?;

    let mut host = PythonCompatibilityHost::default();
    for (idx, raw) in src.lines().enumerate() {
        let line_no = idx + 1;
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let (kind, rest) = line.split_once(':').ok_or_else(|| PythonHostError::Parse {
            path: path.display().to_string(),
            message: format!("line {line_no}: expected '<kind>: ...'"),
        })?;
        let kind = kind.trim();
        let rest = rest.trim();

        if kind == "actions" {
            let (name, token) = rest.split_once('=').ok_or_else(|| PythonHostError::Parse {
                path: path.display().to_string(),
                message: format!("line {line_no}: expected 'actions: name = token'"),
            })?;
            host.register_action_default(name.trim().to_string(), token.trim().to_string());
            continue;
        }

        if let Some(mode_str) = kind.strip_prefix("actions.") {
            let mode = parse_mode(mode_str.trim()).ok_or_else(|| PythonHostError::Parse {
                path: path.display().to_string(),
                message: format!("line {line_no}: unknown mode '{mode_str}'"),
            })?;
            let (name, token) = rest.split_once('=').ok_or_else(|| PythonHostError::Parse {
                path: path.display().to_string(),
                message: format!("line {line_no}: expected 'actions.<mode>: name = token'"),
            })?;
            host.register_action_override(name.trim().to_string(), mode, token.trim().to_string());
            continue;
        }

        if kind == "list" {
            let (name, values) = rest.split_once('=').ok_or_else(|| PythonHostError::Parse {
                path: path.display().to_string(),
                message: format!("line {line_no}: expected 'list: name = a,b,c'"),
            })?;
            let values: Vec<String> = values
                .split(',')
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .collect();
            host.register_list(name.trim().to_string(), values);
            continue;
        }

        if kind == "capture" {
            let (name, sig) = rest.split_once('=').ok_or_else(|| PythonHostError::Parse {
                path: path.display().to_string(),
                message: format!("line {line_no}: expected 'capture: name = signature'"),
            })?;
            host.register_capture(name.trim().to_string(), sig.trim().to_string());
            continue;
        }

        return Err(PythonHostError::Parse {
            path: path.display().to_string(),
            message: format!("line {line_no}: unknown directive '{kind}'"),
        });
    }

    Ok(host)
}

fn parse_mode(s: &str) -> Option<RuntimeMode> {
    match s {
        "global" => Some(RuntimeMode::Global),
        "code" => Some(RuntimeMode::Code),
        "browser" => Some(RuntimeMode::Browser),
        "terminal" => Some(RuntimeMode::Terminal),
        _ => None,
    }
}

