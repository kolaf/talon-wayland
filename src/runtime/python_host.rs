use crate::mode::RuntimeMode;
use crate::runtime::talon_ast::{base_tags_for_mode, TalonContextHeader};
use crate::runtime::talon_parser::parse_context_matches;
use pyo3::exceptions::PyValueError;
use pyo3::ffi::c_str;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyModule};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Mutex, OnceLock};

#[derive(Debug)]
pub enum PythonHostError {
    Io { path: String, source: std::io::Error },
    Unsupported { message: String },
    Parse { path: String, message: String },
    Python { message: String },
}

impl std::fmt::Display for PythonHostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PythonHostError::Io { path, source } => write!(f, "{path}: io error: {source}"),
            PythonHostError::Unsupported { message } => {
                write!(f, "unsupported python host op: {message}")
            }
            PythonHostError::Parse { path, message } => write!(f, "{path}: parse error: {message}"),
            PythonHostError::Python { message } => write!(f, "python execution error: {message}"),
        }
    }
}

impl std::error::Error for PythonHostError {}

/// A primitive `actions.*` invocation recorded while executing a Python action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedActionCall {
    pub name: String,
    pub args: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PythonActionOverride {
    context: TalonContextHeader,
    token: String,
    order: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PythonCaptureOverride {
    context: TalonContextHeader,
    rule: String,
}

/// Narrow compatibility host that bridges Talon's Python action, context, list,
/// and capture ecosystem via an embedded Python 3 interpreter (`pyo3`).
///
/// Supports both:
/// - Executing real `.py` files against a synthetic `talon` compatibility module
/// - Lightweight in-memory registration helpers for isolated unit tests
pub struct PythonCompatibilityHost {
    // Token-based overrides / defaults (used by lightweight unit tests and synced metadata).
    default_impl: HashMap<String, String>,
    per_mode_impl: HashMap<(String, RuntimeMode), String>,

    // Context-aware Python overrides synced from embedded Python execution.
    python_default_actions: HashMap<String, String>,
    python_action_overrides: HashMap<String, Vec<PythonActionOverride>>,
    python_capture_overrides: HashMap<String, Vec<PythonCaptureOverride>>,

    // Registered lists (spoken values + spoken->target mapping) and captures.
    lists: HashMap<String, Vec<String>>,
    list_mappings: HashMap<String, HashMap<String, String>>,
    captures: HashMap<String, String>,

    // Tags, settings, and recorded action calls.
    declared_tags: Vec<String>,
    context_tags: Vec<(TalonContextHeader, Vec<String>)>,
    settings: HashMap<String, String>,
    last_recorded: Mutex<Vec<RecordedActionCall>>,

    // Per-host isolated Python state (`talon._HostState` instance).
    py_state: Option<Py<PyAny>>,
}

impl std::fmt::Debug for PythonCompatibilityHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PythonCompatibilityHost")
            .field("default_impl", &self.default_impl)
            .field("per_mode_impl", &self.per_mode_impl)
            .field("python_default_actions", &self.python_default_actions)
            .field("lists", &self.lists)
            .field("captures", &self.captures)
            .field("has_py_state", &self.py_state.is_some())
            .finish()
    }
}

impl Default for PythonCompatibilityHost {
    fn default() -> Self {
        Self {
            default_impl: HashMap::new(),
            per_mode_impl: HashMap::new(),
            python_default_actions: HashMap::new(),
            python_action_overrides: HashMap::new(),
            python_capture_overrides: HashMap::new(),
            lists: HashMap::new(),
            list_mappings: HashMap::new(),
            captures: HashMap::new(),
            declared_tags: Vec::new(),
            context_tags: Vec::new(),
            settings: HashMap::new(),
            last_recorded: Mutex::new(Vec::new()),
            py_state: None,
        }
    }
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
        let name = name.into();
        let mapping: HashMap<String, String> = values
            .iter()
            .map(|v| (v.clone(), v.clone()))
            .collect();
        self.list_mappings.insert(name.clone(), mapping);
        self.lists.insert(name, values);
    }

    pub fn list_values(&self, name: &str) -> Option<&[String]> {
        self.lists.get(name).map(|v| v.as_slice())
    }

    pub fn list_mapping(&self, name: &str) -> Option<&HashMap<String, String>> {
        self.list_mappings.get(name)
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

    pub fn declared_tags(&self) -> &[String] {
        &self.declared_tags
    }

    pub fn settings(&self) -> &HashMap<String, String> {
        &self.settings
    }

    pub fn last_recorded_actions(&self) -> Vec<RecordedActionCall> {
        self.last_recorded
            .lock()
            .expect("recorded actions mutex poisoned")
            .clone()
    }

    pub fn active_context_tags(&self, mode: RuntimeMode) -> HashSet<String> {
        let mut tags = base_tags_for_mode(mode);
        loop {
            let mut added = false;
            for (ctx, activated) in &self.context_tags {
                if ctx.matches_context(mode, &tags, None) {
                    for tag in activated {
                        if tags.insert(tag.clone()) {
                            added = true;
                        }
                    }
                }
            }
            if !added {
                break;
            }
        }
        tags
    }

    pub fn resolve_action_impl(&self, name: &str, mode: RuntimeMode) -> Option<String> {
        if let Some(v) = self
            .per_mode_impl
            .get(&(name.to_string(), mode))
            .cloned()
        {
            return Some(v);
        }

        if let Some(overrides) = self.python_action_overrides.get(name) {
            let active_tags = self.active_context_tags(mode);
            let best = overrides
                .iter()
                .filter(|ov| ov.context.matches_context(mode, &active_tags, None))
                .max_by_key(|ov| (ov.context.specificity_score(), ov.order));
            if let Some(ov) = best {
                return Some(ov.token.clone());
            }
        }

        if let Some(v) = self.default_impl.get(name).cloned() {
            return Some(v);
        }

        self.python_default_actions.get(name).cloned()
    }

    /// Loads and executes a `.py` file (or legacy directive test file) into this host.
    pub fn load_python_file(&mut self, path: &Path) -> Result<(), PythonHostError> {
        let src = std::fs::read_to_string(path).map_err(|e| PythonHostError::Io {
            path: path.display().to_string(),
            source: e,
        })?;
        if is_legacy_directive_format(&src) {
            return load_legacy_directives_into(self, &src, &path.display().to_string());
        }
        let parent_dir = path.parent().map(|p| p.display().to_string());
        let filename = path.display().to_string();
        self.load_python_internal(&src, &filename, parent_dir.as_deref())
    }

    /// Loads and executes Talon Python source code into this host.
    pub fn load_python_source(
        &mut self,
        source: &str,
        module_name: &str,
    ) -> Result<(), PythonHostError> {
        if is_legacy_directive_format(source) {
            return load_legacy_directives_into(self, source, module_name);
        }
        self.load_python_internal(source, module_name, None)
    }

    fn load_python_internal(
        &mut self,
        source: &str,
        filename: &str,
        parent_dir: Option<&str>,
    ) -> Result<(), PythonHostError> {
        let talon_py = ensure_talon_initialized(filename)?;
        Python::with_gil(|py| {
            let talon_mod = talon_py.bind(py);
            let state = match &self.py_state {
                Some(existing) => existing.clone_ref(py),
                None => {
                    let new_state = talon_mod
                        .getattr("_HostState")
                        .and_then(|cls| cls.call0())
                        .map_err(|e| py_err_to_host(py, e, filename))?
                        .unbind();
                    self.py_state = Some(new_state.clone_ref(py));
                    new_state
                }
            };

            talon_mod
                .getattr("_exec_user_source")
                .and_then(|f| f.call1((state.bind(py), source, filename, parent_dir)))
                .map_err(|e| py_err_to_host(py, e, filename))?;

            self.sync_from_py_state(py, state.bind(py), filename)
        })
    }

    fn sync_from_py_state(
        &mut self,
        py: Python<'_>,
        state: &Bound<'_, PyAny>,
        source_label: &str,
    ) -> Result<(), PythonHostError> {
        // Default actions
        let default_actions: HashMap<String, (String, Py<PyAny>)> = state
            .getattr("default_actions")
            .and_then(|obj| obj.extract())
            .map_err(|e| py_err_to_host(py, e, source_label))?;
        for (name, (qualname, _)) in default_actions {
            self.python_default_actions.insert(name, qualname);
        }

        // Action overrides: list of (action_name, matches_str, qualname, fn)
        let overrides: Vec<(String, String, String, Py<PyAny>)> = state
            .getattr("action_overrides")
            .and_then(|obj| obj.extract())
            .map_err(|e| py_err_to_host(py, e, source_label))?;
        self.python_action_overrides.clear();
        for (order, (action_name, matches_str, qualname, _)) in overrides.into_iter().enumerate() {
            let ctx = parse_context_matches(&matches_str).map_err(|e| PythonHostError::Parse {
                path: source_label.to_string(),
                message: e.to_string(),
            })?;
            self.python_action_overrides
                .entry(action_name)
                .or_default()
                .push(PythonActionOverride {
                    context: ctx,
                    token: qualname,
                    order,
                });
        }

        // Lists: dict[str, dict[str, str]]
        let py_lists: HashMap<String, HashMap<String, String>> = state
            .getattr("lists")
            .and_then(|obj| obj.extract())
            .map_err(|e| py_err_to_host(py, e, source_label))?;
        for (name, mapping) in py_lists {
            let mut keys: Vec<String> = mapping.keys().cloned().collect();
            keys.sort();
            self.lists.insert(name.clone(), keys);
            self.list_mappings.insert(name, mapping);
        }

        // Captures: dict[str, (rule, qualname, fn)]
        let py_captures: HashMap<String, (String, String, Py<PyAny>)> = state
            .getattr("captures")
            .and_then(|obj| obj.extract())
            .map_err(|e| py_err_to_host(py, e, source_label))?;
        for (name, (rule, _qualname, _)) in py_captures {
            self.captures.insert(name, rule);
        }

        // Capture overrides: list of (name, matches, rule, qualname, fn)
        let py_cap_overrides: Vec<(String, String, String, String, Py<PyAny>)> = state
            .getattr("capture_overrides")
            .and_then(|obj| obj.extract())
            .map_err(|e| py_err_to_host(py, e, source_label))?;
        self.python_capture_overrides.clear();
        for (name, matches_str, rule, _qualname, _) in py_cap_overrides {
            let ctx = parse_context_matches(&matches_str).map_err(|e| PythonHostError::Parse {
                path: source_label.to_string(),
                message: e.to_string(),
            })?;
            self.python_capture_overrides
                .entry(name)
                .or_default()
                .push(PythonCaptureOverride { context: ctx, rule });
        }

        // Declared tags & context tags
        let tags: Vec<String> = state
            .getattr("tags")
            .and_then(|obj| obj.extract())
            .map_err(|e| py_err_to_host(py, e, source_label))?;
        self.declared_tags = tags;

        let ctx_tags: Vec<(String, Vec<String>)> = state
            .getattr("context_tags")
            .and_then(|obj| obj.extract())
            .map_err(|e| py_err_to_host(py, e, source_label))?;
        self.context_tags.clear();
        for (matches_str, list) in ctx_tags {
            let ctx = parse_context_matches(&matches_str).map_err(|e| PythonHostError::Parse {
                path: source_label.to_string(),
                message: e.to_string(),
            })?;
            self.context_tags.push((ctx, list));
        }

        // Settings
        let py_settings: HashMap<String, Py<PyAny>> = state
            .getattr("settings")
            .and_then(|obj| obj.extract())
            .map_err(|e| py_err_to_host(py, e, source_label))?;
        for (k, v) in py_settings {
            let v_str = v
                .bind(py)
                .str()
                .map_err(|e| py_err_to_host(py, e, source_label))?
                .to_string();
            self.settings.insert(k, v_str);
        }

        Ok(())
    }

    /// Invokes a Python action (resolving context overrides for `mode`) and returns its stringified
    /// return value (if non-None). Any primitive `actions.*` calls made during execution are
    /// captured in `last_recorded_actions()`.
    pub fn invoke_action(
        &self,
        name: &str,
        mode: RuntimeMode,
        args: &[String],
    ) -> Result<Option<String>, PythonHostError> {
        let (ret, _trace) = self.invoke_action_with_trace(name, mode, args)?;
        Ok(ret)
    }

    /// Invokes a Python action for `mode` and returns both its return value and the primitive
    /// `actions.*` calls recorded during that invocation.
    pub fn invoke_action_with_trace(
        &self,
        name: &str,
        mode: RuntimeMode,
        args: &[String],
    ) -> Result<(Option<String>, Vec<RecordedActionCall>), PythonHostError> {
        let Some(py_state) = &self.py_state else {
            return Err(PythonHostError::Unsupported {
                message: format!("no python modules loaded to invoke `{name}`"),
            });
        };

        let active_tags: Vec<String> = self.active_context_tags(mode).into_iter().collect();
        let mode_str = mode.to_string();
        let talon_py = ensure_talon_initialized(name)?;

        Python::with_gil(|py| {
            let talon_mod = talon_py.bind(py);
            let py_args = PyList::new(py, args).map_err(|e| py_err_to_host(py, e, name))?;
            let py_tags =
                PyList::new(py, &active_tags).map_err(|e| py_err_to_host(py, e, name))?;

            let tuple = talon_mod
                .getattr("_invoke_action")
                .and_then(|f| f.call1((py_state.bind(py), name, mode_str, py_tags, py_args)))
                .map_err(|e| py_err_to_host(py, e, name))?;

            let (ret_opt, raw_trace): (Option<String>, Vec<(String, Vec<String>)>) =
                tuple.extract().map_err(|e| py_err_to_host(py, e, name))?;

            let trace: Vec<RecordedActionCall> = raw_trace
                .into_iter()
                .map(|(n, a)| RecordedActionCall { name: n, args: a })
                .collect();

            *self
                .last_recorded
                .lock()
                .expect("recorded actions mutex poisoned") = trace.clone();

            Ok((ret_opt, trace))
        })
    }

    /// Invokes a Python `@mod.capture` or `@ctx.capture` transformation on `matched_words`.
    pub fn invoke_capture(
        &self,
        name: &str,
        mode: RuntimeMode,
        matched_words: &[String],
    ) -> Result<String, PythonHostError> {
        let Some(py_state) = &self.py_state else {
            return Err(PythonHostError::Unsupported {
                message: format!("no python modules loaded to invoke capture `{name}`"),
            });
        };

        let active_tags: Vec<String> = self.active_context_tags(mode).into_iter().collect();
        let mode_str = mode.to_string();
        let talon_py = ensure_talon_initialized(name)?;

        Python::with_gil(|py| {
            let talon_mod = talon_py.bind(py);
            let py_words =
                PyList::new(py, matched_words).map_err(|e| py_err_to_host(py, e, name))?;
            let py_tags =
                PyList::new(py, &active_tags).map_err(|e| py_err_to_host(py, e, name))?;

            let result: String = talon_mod
                .getattr("_invoke_capture")
                .and_then(|f| f.call1((py_state.bind(py), name, mode_str, py_tags, py_words)))
                .and_then(|r| r.extract())
                .map_err(|e| py_err_to_host(py, e, name))?;

            Ok(result)
        })
    }
}

/// Loads a Talon Python module (or legacy directive test file) and discards the resulting host.
pub fn load_python_module(path: &Path) -> Result<(), PythonHostError> {
    let _host = load_python_module_to_host(path)?;
    Ok(())
}

/// Loads a Talon Python module (or legacy directive test file) into a new `PythonCompatibilityHost`.
pub fn load_python_module_to_host(path: &Path) -> Result<PythonCompatibilityHost, PythonHostError> {
    let mut host = PythonCompatibilityHost::default();
    host.load_python_file(path)?;
    Ok(host)
}

fn is_legacy_directive_format(src: &str) -> bool {
    let mut saw_directive = false;
    for raw in src.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((kind, rest)) = line.split_once(':') else {
            return false;
        };
        let kind = kind.trim();
        let is_known_kind = kind == "actions"
            || kind.starts_with("actions.")
            || kind == "list"
            || kind == "capture";
        if !is_known_kind || !rest.contains('=') {
            return false;
        }
        saw_directive = true;
    }
    saw_directive
}

fn load_legacy_directives_into(
    host: &mut PythonCompatibilityHost,
    src: &str,
    path_label: &str,
) -> Result<(), PythonHostError> {
    for (idx, raw) in src.lines().enumerate() {
        let line_no = idx + 1;
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let (kind, rest) = line.split_once(':').ok_or_else(|| PythonHostError::Parse {
            path: path_label.to_string(),
            message: format!("line {line_no}: expected '<kind>: ...'"),
        })?;
        let kind = kind.trim();
        let rest = rest.trim();

        if kind == "actions" {
            let (name, token) = rest.split_once('=').ok_or_else(|| PythonHostError::Parse {
                path: path_label.to_string(),
                message: format!("line {line_no}: expected 'actions: name = token'"),
            })?;
            host.register_action_default(name.trim().to_string(), token.trim().to_string());
            continue;
        }

        if let Some(mode_str) = kind.strip_prefix("actions.") {
            let mode = parse_mode(mode_str.trim()).ok_or_else(|| PythonHostError::Parse {
                path: path_label.to_string(),
                message: format!("line {line_no}: unknown mode '{mode_str}'"),
            })?;
            let (name, token) = rest.split_once('=').ok_or_else(|| PythonHostError::Parse {
                path: path_label.to_string(),
                message: format!("line {line_no}: expected 'actions.<mode>: name = token'"),
            })?;
            host.register_action_override(name.trim().to_string(), mode, token.trim().to_string());
            continue;
        }

        if kind == "list" {
            let (name, values) = rest.split_once('=').ok_or_else(|| PythonHostError::Parse {
                path: path_label.to_string(),
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
                path: path_label.to_string(),
                message: format!("line {line_no}: expected 'capture: name = signature'"),
            })?;
            host.register_capture(name.trim().to_string(), sig.trim().to_string());
            continue;
        }

        return Err(PythonHostError::Parse {
            path: path_label.to_string(),
            message: format!("line {line_no}: unknown directive '{kind}'"),
        });
    }

    Ok(())
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

#[pyfunction]
fn validate_context_matches(matches: &str) -> PyResult<()> {
    parse_context_matches(matches).map_err(|e| PyValueError::new_err(e.to_string()))?;
    Ok(())
}

#[pyfunction]
fn matches_context_for_mode(
    matches: &str,
    mode_str: &str,
    active_tags: Vec<String>,
) -> PyResult<(bool, usize)> {
    let ctx = parse_context_matches(matches).map_err(|e| PyValueError::new_err(e.to_string()))?;
    let mode = parse_mode(mode_str).ok_or_else(|| {
        PyValueError::new_err(format!("unknown runtime mode `{mode_str}`"))
    })?;
    let tag_set: HashSet<String> = active_tags.into_iter().collect();
    Ok((ctx.matches_context(mode, &tag_set, None), ctx.specificity_score()))
}

fn py_err_to_host(py: Python<'_>, err: PyErr, path_label: &str) -> PythonHostError {
    if err.is_instance_of::<pyo3::exceptions::PySyntaxError>(py)
        || err.is_instance_of::<PyValueError>(py)
    {
        PythonHostError::Parse {
            path: path_label.to_string(),
            message: err.to_string(),
        }
    } else {
        PythonHostError::Python {
            message: format!("{path_label}: {err}"),
        }
    }
}

static TALON_MODULE_INIT: OnceLock<Result<Py<PyModule>, String>> = OnceLock::new();

fn ensure_talon_initialized(label: &str) -> Result<&'static Py<PyModule>, PythonHostError> {
    let init = TALON_MODULE_INIT.get_or_init(|| {
        Python::with_gil(|py| {
            ensure_talon_module(py)
                .map(|m| m.unbind())
                .map_err(|e| e.to_string())
        })
    });
    match init {
        Ok(m) => Ok(m),
        Err(msg) => Err(PythonHostError::Python {
            message: format!("{label}: failed to initialize talon python shim: {msg}"),
        }),
    }
}

fn ensure_talon_module<'py>(py: Python<'py>) -> PyResult<Bound<'py, PyModule>> {
    let sys = py.import("sys")?;
    let sys_modules: Bound<'py, PyDict> = sys.getattr("modules")?.downcast_into()?;

    let bootstrap_cstr =
        std::ffi::CString::new(TALON_BOOTSTRAP_PY).expect("bootstrap python contains no NUL");
    let talon_mod =
        PyModule::from_code(py, bootstrap_cstr.as_c_str(), c_str!("talon.py"), c_str!("talon"))?;
    talon_mod.add_function(wrap_pyfunction!(validate_context_matches, &talon_mod)?)?;
    talon_mod.add_function(wrap_pyfunction!(matches_context_for_mode, &talon_mod)?)?;

    sys_modules.set_item("talon", &talon_mod)?;

    let grammar_mod = talon_mod.getattr("grammar")?;
    sys_modules.set_item("talon.grammar", grammar_mod)?;

    let scripting_mod = talon_mod.getattr("scripting")?;
    sys_modules.set_item("talon.scripting", scripting_mod)?;

    let ui_mod = talon_mod.getattr("ui")?;
    sys_modules.set_item("talon.ui", ui_mod)?;

    Ok(talon_mod)
}

const TALON_BOOTSTRAP_PY: &str = r#"
import contextlib
import inspect
import re
import sys
import threading
import types

_tls = threading.local()


def _get_active_host():
    return getattr(_tls, "active_host", None)


def _get_active_mode():
    return getattr(_tls, "active_mode", "global")


def _get_active_tags():
    return getattr(_tls, "active_tags", [])


def _require_host():
    host = _get_active_host()
    if host is None:
        raise RuntimeError("No active voice-runtime host")
    return host


class _HostState:
    def __init__(self):
        self.default_actions = {}
        self.action_overrides = []
        self.lists = {}
        self.list_entries = []
        self.captures = {}
        self.capture_overrides = []
        self.settings = {}
        self.context_settings = []
        self.tags = []
        self.context_tags = []
        self.modes = []
        self.apps = {}
        self.recorded_actions = []
        self.call_stack = []


class _AppsProxy:
    def __init__(self, host):
        self._host = host

    def __setattr__(self, name, value):
        if name == "_host":
            super().__setattr__(name, value)
        else:
            self._host.apps[name] = value


class Module:
    def __init__(self):
        self._host = _require_host()
        self.apps = _AppsProxy(self._host)

    def list(self, name, desc=""):
        full = name if "." in name else f"user.{name}"
        self._host.lists.setdefault(full, {})

    def tag(self, name, desc=""):
        full = name if "." in name else f"user.{name}"
        if full not in self._host.tags:
            self._host.tags.append(full)

    def setting(self, name, type=str, default=None, desc=""):
        full = name if "." in name else f"user.{name}"
        self._host.settings.setdefault(full, default)

    def mode(self, name, desc=""):
        if name not in self._host.modes:
            self._host.modes.append(name)

    def capture(self, rule=None):
        if callable(rule):
            fn = rule
            rule_str = (fn.__doc__ or f"<user.{fn.__name__}>").strip()
            self._host.captures[f"user.{fn.__name__}"] = (rule_str, fn.__qualname__, fn)
            return fn

        def decorator(fn):
            rule_str = rule if isinstance(rule, str) else (fn.__doc__ or f"<user.{fn.__name__}>").strip()
            self._host.captures[f"user.{fn.__name__}"] = (rule_str, fn.__qualname__, fn)
            return fn

        return decorator

    def action_class(self, cls=None):
        def register(target_cls):
            for attr_name, val in target_cls.__dict__.items():
                if attr_name.startswith("_"):
                    continue
                fn = val.__func__ if isinstance(val, (staticmethod, classmethod)) else val
                if callable(fn):
                    full_name = f"user.{attr_name}"
                    self._host.default_actions[full_name] = (f"{target_cls.__name__}.{attr_name}", fn)
            return target_cls

        if cls is None:
            return register
        return register(cls)

    def action(self, fn):
        self._host.default_actions[f"user.{fn.__name__}"] = (fn.__qualname__, fn)
        return fn


class _ContextListsProxy(dict):
    def __init__(self, ctx):
        super().__init__()
        self._ctx = ctx

    def _normalize(self, value):
        if isinstance(value, dict):
            return {str(k): str(v) for k, v in value.items()}
        return {str(item): str(item) for item in value}

    def __setitem__(self, name, value):
        norm = self._normalize(value)
        super().__setitem__(name, norm)
        self._ctx._host.lists[name] = norm
        self._ctx._host.list_entries.append((name, self._ctx._matches, norm))

    def update(self, other=(), **kwargs):
        if isinstance(other, dict):
            for k, v in other.items():
                self[k] = v
        else:
            for k, v in other:
                self[k] = v
        for k, v in kwargs.items():
            self[k] = v


class _ContextSettingsProxy(dict):
    def __init__(self, ctx):
        super().__init__()
        self._ctx = ctx

    def __setitem__(self, name, value):
        super().__setitem__(name, value)
        self._ctx._host.settings[name] = value
        self._ctx._host.context_settings.append((self._ctx._matches, {name: value}))


class Context:
    def __init__(self):
        self._host = _require_host()
        self._matches = ""
        self._tags = []
        self.lists = _ContextListsProxy(self)
        self.settings = _ContextSettingsProxy(self)

    @property
    def matches(self):
        return self._matches

    @matches.setter
    def matches(self, value):
        validate_context_matches(str(value))
        self._matches = str(value)

    @property
    def tags(self):
        return list(self._tags)

    @tags.setter
    def tags(self, value):
        self._tags = [str(t) for t in value]
        self._host.context_tags.append((self._matches, list(self._tags)))

    def action_class(self, path="user"):
        if callable(path):
            target_cls = path
            ns = "user"
            return self._register_action_class(ns, target_cls)

        def decorator(target_cls):
            return self._register_action_class(path, target_cls)

        return decorator

    def _register_action_class(self, ns, target_cls):
        for attr_name, val in target_cls.__dict__.items():
            if attr_name.startswith("_"):
                continue
            fn = val.__func__ if isinstance(val, (staticmethod, classmethod)) else val
            if callable(fn):
                full_name = f"{ns}.{attr_name}" if ns else attr_name
                self._host.action_overrides.append(
                    (full_name, self._matches, f"{target_cls.__name__}.{attr_name}", fn)
                )
        return target_cls

    def capture(self, name=None, rule=None):
        def decorator(fn):
            cap_name = name if isinstance(name, str) and not name.startswith("{") and not name.startswith("<") else f"user.{fn.__name__}"
            if "." not in cap_name:
                cap_name = f"user.{cap_name}"
            rule_str = rule if isinstance(rule, str) else (fn.__doc__ or f"<{cap_name}>").strip()
            self._host.capture_overrides.append(
                (cap_name, self._matches, rule_str, fn.__qualname__, fn)
            )
            return fn

        return decorator


def _resolve_action_candidates(host, action_name, mode_str, active_tags):
    matched = []
    for idx, (name, matches_str, _qualname, fn) in enumerate(host.action_overrides):
        if name != action_name:
            continue
        ok, score = matches_context_for_mode(matches_str, mode_str, active_tags)
        if ok:
            matched.append((score, idx, fn))
    matched.sort(key=lambda item: (item[0], item[1]), reverse=True)
    candidates = [item[2] for item in matched]
    if action_name in host.default_actions:
        candidates.append(host.default_actions[action_name][1])
    return candidates


def _dispatch_action(action_name, args, kwargs):
    host = _require_host()
    if action_name == "next":
        if host.call_stack:
            top_name, candidates, idx = host.call_stack[-1]
            if idx + 1 < len(candidates):
                host.call_stack.append((top_name, candidates, idx + 1))
                try:
                    return candidates[idx + 1](*args, **kwargs)
                finally:
                    host.call_stack.pop()
        return None

    candidates = _resolve_action_candidates(host, action_name, _get_active_mode(), _get_active_tags())
    if candidates:
        host.call_stack.append((action_name, candidates, 0))
        try:
            return candidates[0](*args, **kwargs)
        finally:
            host.call_stack.pop()

    host.recorded_actions.append((action_name, [str(a) for a in args]))
    return None


class _ActionNamespace:
    def __init__(self, path=""):
        self._path = path

    def __getattr__(self, name):
        next_path = f"{self._path}.{name}" if self._path else name
        return _ActionNamespace(next_path)

    def __call__(self, *args, **kwargs):
        if not self._path:
            raise TypeError("actions object is not directly callable")
        return _dispatch_action(self._path, args, kwargs)


actions = _ActionNamespace()


class _SettingsProxy:
    def get(self, name, default=None):
        host = _get_active_host()
        if host is not None and name in host.settings:
            val = host.settings[name]
            return default if val is None else val
        return default


settings = _SettingsProxy()


class _AppProxy:
    platform = "linux"

    def register(self, *_args, **_kwargs):
        def decorator(fn):
            return fn
        return decorator

    def notify(self, body="", title=""):
        host = _get_active_host()
        if host is not None:
            host.recorded_actions.append(("app.notify", [str(body), str(title)]))


app = _AppProxy()


class _ClipProxy:
    def __init__(self):
        self._text = ""

    def text(self):
        return self._text

    def set_text(self, value):
        self._text = str(value)

    @contextlib.contextmanager
    def revert(self):
        saved = self._text
        try:
            yield
        finally:
            self._text = saved


clip = _ClipProxy()


class _CronProxy:
    def after(self, *_args, **_kwargs):
        return "cron-job"

    def interval(self, *_args, **_kwargs):
        return "cron-job"

    def cancel(self, *_args, **_kwargs):
        return None


cron = _CronProxy()


class _ImguiProxy:
    def open(self, **_kwargs):
        def decorator(fn):
            return fn
        return decorator


imgui = _ImguiProxy()


class _ScopeProxy:
    def get(self, key, default=None):
        if key == "os":
            return "linux"
        if key == "mode":
            return ["command"]
        if key == "tag":
            return list(_get_active_tags())
        return default


scope = _ScopeProxy()


class _RegistryProxy:
    @property
    def lists(self):
        host = _get_active_host()
        return host.lists if host is not None else {}

    @property
    def captures(self):
        host = _get_active_host()
        return host.captures if host is not None else {}

    @property
    def actions(self):
        host = _get_active_host()
        return host.default_actions if host is not None else {}

    commands = {}


registry = _RegistryProxy()
speech_system = types.SimpleNamespace(register=lambda *a, **kw: (lambda fn: fn) if not a else None)


class Rect:
    def __init__(self, x=0, y=0, width=0, height=0):
        self.x = x
        self.y = y
        self.width = width
        self.height = height


class _UiProxy:
    Rect = Rect

    def register(self, *_args, **_kwargs):
        def decorator(fn):
            return fn
        return decorator

    def active_app(self):
        return types.SimpleNamespace(name="", bundle="", exe="")

    def active_window(self):
        return types.SimpleNamespace(title="", rect=Rect())


ui = _UiProxy()


class Phrase:
    pass


grammar = types.SimpleNamespace(Phrase=Phrase)
scripting = types.SimpleNamespace(rctx=None)


class CaptureMatch:
    def __init__(self, raw_words, mapped_words, short_names):
        self._raw = list(raw_words)
        self._mapped = list(mapped_words)
        self._short_names = set(short_names)

    def __str__(self):
        return " ".join(self._mapped)

    def __iter__(self):
        return iter(self._mapped)

    def __len__(self):
        return len(self._mapped)

    def __getitem__(self, idx):
        return self._mapped[idx]

    def __getattr__(self, name):
        if name.startswith("_"):
            raise AttributeError(name)
        if name.endswith("_list"):
            return list(self._mapped)
        if name in self._short_names:
            return self._mapped[0] if self._mapped else ""
        raise AttributeError(name)


def _exec_user_source(host_state, source, filename, parent_dir):
    prev_host = _get_active_host()
    _tls.active_host = host_state
    added_sys_path = False
    if parent_dir and parent_dir not in sys.path:
        sys.path.insert(0, parent_dir)
        added_sys_path = True
    try:
        ns = {
            "__name__": "__talon_module__",
            "__file__": filename,
        }
        code = compile(source, filename, "exec")
        exec(code, ns, ns)
    finally:
        if added_sys_path:
            try:
                sys.path.remove(parent_dir)
            except ValueError:
                pass
        _tls.active_host = prev_host


def _invoke_action(host_state, action_name, mode_str, active_tags, args):
    prev_host, prev_mode, prev_tags = _get_active_host(), _get_active_mode(), _get_active_tags()
    _tls.active_host = host_state
    _tls.active_mode = mode_str
    _tls.active_tags = list(active_tags)
    host_state.recorded_actions = []
    try:
        result = _dispatch_action(action_name, tuple(args), {})
        ret_str = None if result is None else str(result)
        return (ret_str, list(host_state.recorded_actions))
    finally:
        _tls.active_host = prev_host
        _tls.active_mode = prev_mode
        _tls.active_tags = prev_tags


def _invoke_capture(host_state, capture_name, mode_str, active_tags, words):
    prev_host, prev_mode, prev_tags = _get_active_host(), _get_active_mode(), _get_active_tags()
    _tls.active_host = host_state
    _tls.active_mode = mode_str
    _tls.active_tags = list(active_tags)
    try:
        matched = []
        for idx, (name, matches_str, rule_str, _qualname, fn) in enumerate(host_state.capture_overrides):
            if name != capture_name:
                continue
            ok, score = matches_context_for_mode(matches_str, mode_str, active_tags)
            if ok:
                matched.append((score, idx, rule_str, fn))
        if matched:
            matched.sort(key=lambda item: (item[0], item[1]), reverse=True)
            _, _, rule_str, fn = matched[0]
        elif capture_name in host_state.captures:
            rule_str, _qualname, fn = host_state.captures[capture_name]
        else:
            raise KeyError(f"capture not found: {capture_name}")

        list_refs = re.findall(r"\{([^{}]+)\}", rule_str)
        cap_refs = re.findall(r"<([^<>]+)>", rule_str)
        short_names = [ref.split(".")[-1] for ref in list_refs + cap_refs]

        combined_map = {}
        for ref in list_refs:
            if ref in host_state.lists:
                combined_map.update(host_state.lists[ref])

        mapped_words = [combined_map.get(w, w) for w in words]
        match_obj = CaptureMatch(words, mapped_words, short_names)
        result = fn(match_obj)
        return "" if result is None else str(result)
    finally:
        _tls.active_host = prev_host
        _tls.active_mode = prev_mode
        _tls.active_tags = prev_tags
"#;
