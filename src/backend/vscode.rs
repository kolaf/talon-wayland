use crate::backend::{snake_to_camel, strip_quotes, AppBackend};
use crate::mode::RuntimeMode;
use crate::runtime::errors::ExecutionError;
use crate::runtime::executor::RuntimeValue;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VsCodeCommand {
    pub command_name: String,
    pub args: Vec<String>,
}

/// VS Code / Cursorless command-server backend.
#[derive(Debug, Default)]
pub struct VsCodeBackend {
    connected: bool,
    dispatched: Vec<VsCodeCommand>,
}

impl VsCodeBackend {
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

    pub fn send(&mut self, cmd: VsCodeCommand) -> Result<(), ExecutionError> {
        if !self.connected {
            return Err(ExecutionError::BackendUnavailable("vscode".into()));
        }
        self.dispatched.push(cmd);
        Ok(())
    }

    pub fn send_for_test(&self, _cmd: VsCodeCommand) -> Result<(), ExecutionError> {
        if !self.connected {
            return Err(ExecutionError::BackendUnavailable("vscode".into()));
        }
        Ok(())
    }

    pub fn dispatched_commands(&self) -> &[VsCodeCommand] {
        &self.dispatched
    }
}

impl AppBackend for VsCodeBackend {
    fn name(&self) -> &'static str {
        "vscode"
    }

    fn target_mode(&self) -> RuntimeMode {
        RuntimeMode::Code
    }

    fn is_connected(&self) -> bool {
        self.connected
    }

    fn handles_action(&self, action_name: &str) -> bool {
        action_name.starts_with("user.cursorless_") || action_name.starts_with("user.vscode")
    }

    fn execute_action(
        &mut self,
        action_name: &str,
        args: &[RuntimeValue],
    ) -> Result<(), ExecutionError> {
        let cmd = map_cursorless_action(action_name, args)?;
        self.send(cmd)
    }
}

pub fn map_cursorless_action(
    action_name: &str,
    args: &[RuntimeValue],
) -> Result<VsCodeCommand, ExecutionError> {
    if action_name == "user.cursorless_take_line" {
        return Ok(VsCodeCommand {
            command_name: "cursorless.takeLine".into(),
            args: args.iter().map(|a| strip_quotes(a.as_str())).collect(),
        });
    }

    if action_name == "user.cursorless_command" {
        return Ok(VsCodeCommand {
            command_name: "cursorless.command".into(),
            args: args.iter().map(|a| strip_quotes(a.as_str())).collect(),
        });
    }

    if let Some(rest) = action_name.strip_prefix("user.cursorless_") {
        return Ok(VsCodeCommand {
            command_name: format!("cursorless.{}", snake_to_camel(rest)),
            args: args.iter().map(|a| strip_quotes(a.as_str())).collect(),
        });
    }

    if matches!(
        action_name,
        "user.vscode"
            | "user.vscode_and_wait"
            | "user.vscode_with_plugin"
            | "user.vscode_with_plugin_and_wait"
    ) {
        let Some(first) = args.first() else {
            return Err(ExecutionError::UnsupportedFeature(format!(
                "{action_name} requires a VS Code command id"
            )));
        };
        return Ok(VsCodeCommand {
            command_name: strip_quotes(first.as_str()),
            args: args[1..].iter().map(|a| strip_quotes(a.as_str())).collect(),
        });
    }

    Err(ExecutionError::UnsupportedFeature(action_name.to_string()))
}
