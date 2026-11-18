use crate::backend::{strip_quotes, AppBackend};
use crate::mode::RuntimeMode;
use crate::runtime::errors::ExecutionError;
use crate::runtime::executor::RuntimeValue;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyStroke(pub String);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalDispatch {
    InsertText(String),
    SendKeys(Vec<KeyStroke>),
}

const ALLOWED_TERMINAL_KEYS: &[&str] = &[
    "enter",
    "return",
    "tab",
    "esc",
    "escape",
    "up",
    "down",
    "left",
    "right",
    "backspace",
    "delete",
    "home",
    "end",
    "pageup",
    "pagedown",
    "space",
    "ctrl-c",
    "ctrl-d",
    "ctrl-l",
    "ctrl-r",
    "ctrl-u",
    "ctrl-w",
    "ctrl-z",
    "ctrl-a",
    "ctrl-e",
    "ctrl-shift-c",
    "ctrl-shift-v",
];

/// Terminal backend preferring text insertion and an explicit allowlist of safe control keys.
#[derive(Debug, Default)]
pub struct TerminalBackend {
    connected: bool,
    dispatched: Vec<TerminalDispatch>,
}

impl TerminalBackend {
    pub fn new(connected: bool) -> Self {
        Self {
            connected,
            dispatched: Vec::new(),
        }
    }

    pub fn new_for_test_connected() -> Self {
        Self::new(true)
    }

    pub fn new_for_test_disconnected() -> Self {
        Self::new(false)
    }

    pub fn disconnect(&mut self) {
        self.connected = false;
    }

    pub fn set_connected(&mut self, connected: bool) {
        self.connected = connected;
    }

    pub fn send(&mut self, dispatch: TerminalDispatch) -> Result<(), ExecutionError> {
        if !self.connected {
            return Err(ExecutionError::BackendUnavailable("terminal".into()));
        }
        self.dispatched.push(dispatch);
        Ok(())
    }

    pub fn dispatched(&self) -> &[TerminalDispatch] {
        &self.dispatched
    }
}

impl AppBackend for TerminalBackend {
    fn name(&self) -> &'static str {
        "terminal"
    }

    fn target_mode(&self) -> RuntimeMode {
        RuntimeMode::Terminal
    }

    fn is_connected(&self) -> bool {
        self.connected
    }

    fn handles_action(&self, action_name: &str) -> bool {
        matches!(action_name, "insert" | "edit.paste" | "key")
            || action_name.starts_with("user.terminal_")
            || action_name.starts_with("user.unsupported_terminal")
    }

    fn execute_action(
        &mut self,
        action_name: &str,
        args: &[RuntimeValue],
    ) -> Result<(), ExecutionError> {
        let dispatch = map_terminal_action(action_name, args)?;
        self.send(dispatch)
    }
}

pub fn map_terminal_action(
    action_name: &str,
    args: &[RuntimeValue],
) -> Result<TerminalDispatch, ExecutionError> {
    if matches!(action_name, "insert" | "edit.paste") {
        let text = args
            .iter()
            .map(|a| strip_quotes(a.as_str()))
            .collect::<Vec<_>>()
            .join(" ");
        return Ok(TerminalDispatch::InsertText(text));
    }

    if action_name == "key" {
        let raw_keys = args
            .first()
            .map(|a| strip_quotes(a.as_str()))
            .unwrap_or_default();
        let tokens: Vec<String> = raw_keys
            .split_whitespace()
            .map(|s| s.to_ascii_lowercase())
            .collect();
        if tokens.is_empty() {
            return Err(ExecutionError::UnsupportedFeature(
                "empty key sequence in terminal".into(),
            ));
        }
        for tok in &tokens {
            if !ALLOWED_TERMINAL_KEYS.contains(&tok.as_str()) {
                return Err(ExecutionError::UnsupportedFeature(format!(
                    "disallowed terminal key sequence: {tok}"
                )));
            }
        }
        return Ok(TerminalDispatch::SendKeys(
            tokens.into_iter().map(KeyStroke).collect(),
        ));
    }

    Err(ExecutionError::UnsupportedFeature(action_name.to_string()))
}
