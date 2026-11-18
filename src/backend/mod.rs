pub mod browser;
pub mod terminal;
pub mod vscode;

use crate::mode::RuntimeMode;
use crate::runtime::errors::ExecutionError;
use crate::runtime::executor::RuntimeValue;

/// Backend integration point for controlling external tools (e.g. editor, browser, terminal).
pub trait AppBackend {
    fn name(&self) -> &'static str;
    fn target_mode(&self) -> RuntimeMode;
    fn is_connected(&self) -> bool;
    fn handles_action(&self, action_name: &str) -> bool;
    fn execute_action(
        &mut self,
        action_name: &str,
        args: &[RuntimeValue],
    ) -> Result<(), ExecutionError>;
}

pub(crate) fn snake_to_camel(snake: &str) -> String {
    let mut out = String::new();
    for (idx, part) in snake.split('_').filter(|s| !s.is_empty()).enumerate() {
        if idx == 0 {
            out.push_str(part);
        } else {
            let mut chars = part.chars();
            if let Some(first) = chars.next() {
                out.extend(first.to_uppercase());
                out.push_str(chars.as_str());
            }
        }
    }
    out
}

pub(crate) fn strip_quotes(s: &str) -> String {
    let trimmed = s.trim();
    if trimmed.len() >= 2
        && ((trimmed.starts_with('"') && trimmed.ends_with('"'))
            || (trimmed.starts_with('\'') && trimmed.ends_with('\'')))
    {
        trimmed[1..trimmed.len() - 1].to_string()
    } else {
        trimmed.to_string()
    }
}
