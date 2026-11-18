use crate::mode::RuntimeMode;
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug)]
pub enum PythonHostError {
    Io { path: String, source: std::io::Error },
    Unsupported { message: String },
}

impl std::fmt::Display for PythonHostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PythonHostError::Io { path, source } => write!(f, "{path}: io error: {source}"),
            PythonHostError::Unsupported { message } => write!(f, "unsupported python host op: {message}"),
        }
    }
}

impl std::error::Error for PythonHostError {}

/// Narrow compatibility host that models the minimal subset of Talon's Python action system
/// required by the target repos.
///
/// The intent is to keep this layer replaceable once an embedded python engine exists.
#[derive(Debug, Default)]
pub struct PythonCompatibilityHost {
    // action_name -> (optional mode override -> implementation token)
    //
    // For v1 tests, the implementation is represented as an opaque string.
    default_impl: HashMap<String, String>,
    per_mode_impl: HashMap<(String, RuntimeMode), String>,
    // registered lists/captures (stubs; required by grammars but not executed here)
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
        let mode = parse_mode(mode);
        host.default_impl
            .insert(action_name.to_string(), default_impl.to_string());
        host.per_mode_impl.insert(
            (action_name.to_string(), mode),
            override_impl.to_string(),
        );
        host
    }

    /// Test helper: resolve and return the selected implementation token.
    pub fn resolve_for_test(&self, action_name: &str, mode: &str) -> String {
        let mode = parse_mode(mode);
        self.resolve_action_impl(action_name, mode)
            .unwrap_or_else(|| "".to_string())
    }

    pub fn register_list(&mut self, name: impl Into<String>, values: Vec<String>) {
        self.lists.insert(name.into(), values);
    }

    pub fn register_capture(&mut self, name: impl Into<String>, signature: impl Into<String>) {
        self.captures.insert(name.into(), signature.into());
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

pub fn load_python_module(path: &Path) -> Result<(), PythonHostError> {
    // v1: keep parsing/executing python out of scope. This is a placeholder boundary.
    std::fs::read_to_string(path).map_err(|e| PythonHostError::Io {
        path: path.display().to_string(),
        source: e,
    })?;
    Err(PythonHostError::Unsupported {
        message: "loading python modules is not implemented in v1".to_string(),
    })
}

fn parse_mode(s: &str) -> RuntimeMode {
    match s {
        "global" => RuntimeMode::Global,
        "code" => RuntimeMode::Code,
        "browser" => RuntimeMode::Browser,
        "terminal" => RuntimeMode::Terminal,
        other => panic!("unknown mode string for test: {other}"),
    }
}
