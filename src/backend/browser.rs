use crate::backend::{snake_to_camel, strip_quotes, AppBackend};
use crate::mode::RuntimeMode;
use crate::runtime::errors::ExecutionError;
use crate::runtime::executor::RuntimeValue;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserCommand {
    pub command_name: String,
    pub args: Vec<String>,
}

/// Browser / Rango extension-messaging backend.
#[derive(Debug, Default)]
pub struct BrowserBackend {
    connected: bool,
    dispatched: Vec<BrowserCommand>,
}

impl BrowserBackend {
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

    pub fn send(&mut self, cmd: BrowserCommand) -> Result<(), ExecutionError> {
        if !self.connected {
            return Err(ExecutionError::BackendUnavailable("browser".into()));
        }
        self.dispatched.push(cmd);
        Ok(())
    }

    pub fn send_for_test(&self, _cmd: BrowserCommand) -> Result<(), ExecutionError> {
        if !self.connected {
            return Err(ExecutionError::BackendUnavailable("browser".into()));
        }
        Ok(())
    }

    pub fn dispatched_commands(&self) -> &[BrowserCommand] {
        &self.dispatched
    }
}

impl AppBackend for BrowserBackend {
    fn name(&self) -> &'static str {
        "browser"
    }

    fn target_mode(&self) -> RuntimeMode {
        RuntimeMode::Browser
    }

    fn is_connected(&self) -> bool {
        self.connected
    }

    fn handles_action(&self, action_name: &str) -> bool {
        action_name.starts_with("user.rango_")
    }

    fn execute_action(
        &mut self,
        action_name: &str,
        args: &[RuntimeValue],
    ) -> Result<(), ExecutionError> {
        let cmd = map_rango_action(action_name, args)?;
        self.send(cmd)
    }
}

pub fn map_rango_action(
    action_name: &str,
    args: &[RuntimeValue],
) -> Result<BrowserCommand, ExecutionError> {
    if action_name == "user.rango_open_hint" {
        return Ok(BrowserCommand {
            command_name: "rango.openHint".into(),
            args: args.iter().map(|a| strip_quotes(a.as_str())).collect(),
        });
    }

    if matches!(
        action_name,
        "user.rango_command_without_target" | "user.rango_command_with_target"
    ) {
        let command_name = match args.first() {
            Some(first) => format!("rango.{}", strip_quotes(first.as_str())),
            None => "rango.command".into(),
        };
        let rest_args = if args.len() > 1 {
            args[1..].iter().map(|a| strip_quotes(a.as_str())).collect()
        } else {
            Vec::new()
        };
        return Ok(BrowserCommand {
            command_name,
            args: rest_args,
        });
    }

    if let Some(rest) = action_name.strip_prefix("user.rango_") {
        return Ok(BrowserCommand {
            command_name: format!("rango.{}", snake_to_camel(rest)),
            args: args.iter().map(|a| strip_quotes(a.as_str())).collect(),
        });
    }

    Err(ExecutionError::UnsupportedFeature(action_name.to_string()))
}
