use crate::mode::RuntimeMode;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TalonContextHeader {
    pub app: Option<String>,
    pub app_name: Option<String>,
    pub os: Option<String>,
    pub mode: Option<RuntimeMode>,
    pub tags: Vec<String>,
    pub code_language: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TalonFile {
    pub context: TalonContextHeader,
    pub commands: Vec<TalonCommand>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TalonCommand {
    pub spoken_form: String,
    pub body: Vec<TalonBodyStmt>,
    pub context: TalonContextHeader,
    pub source_path: Option<PathBuf>,
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TalonBodyStmt {
    /// An action invocation like `key(ctrl-left)` or `user.some_action()`.
    ActionInvoke { name: String, args: Vec<String> },
    /// Tag activation like `tag(): user.tabs`.
    TagActivate { tag: String },
}
