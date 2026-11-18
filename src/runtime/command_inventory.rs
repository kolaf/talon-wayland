use crate::mode::RuntimeMode;
use crate::runtime::talon_ast::{TalonCommand, TalonFile};
use crate::runtime::talon_parser::{parse_talon_file, TalonParseError};
use std::path::PathBuf;

#[derive(Debug)]
pub enum InventoryError {
    Io { path: PathBuf, source: std::io::Error },
    Parse { path: PathBuf, source: TalonParseError },
}

impl std::fmt::Display for InventoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InventoryError::Io { path, source } => {
                write!(f, "{}: io error: {}", path.display(), source)
            }
            InventoryError::Parse { path, source } => {
                write!(f, "{}: parse error: {}", path.display(), source)
            }
        }
    }
}

impl std::error::Error for InventoryError {}

#[derive(Debug, Default)]
pub struct CommandInventory {
    commands: Vec<TalonCommand>,
}

impl CommandInventory {
    pub fn from_test_commands(commands: Vec<TalonCommand>) -> Self {
        Self { commands }
    }

    pub fn commands_for_mode(&self, mode: RuntimeMode) -> Vec<&TalonCommand> {
        self.commands
            .iter()
            .filter(|cmd| cmd.context.mode.unwrap_or(RuntimeMode::Global) == mode)
            .collect()
    }
}

pub fn load_inventory(paths: &[PathBuf]) -> Result<CommandInventory, InventoryError> {
    let mut all: Vec<TalonCommand> = Vec::new();
    for path in paths {
        let src = std::fs::read_to_string(path)
            .map_err(|e| InventoryError::Io { path: path.clone(), source: e })?;
        let mut file: TalonFile =
            parse_talon_file(&src).map_err(|e| InventoryError::Parse { path: path.clone(), source: e })?;
        // annotate commands with path
        for cmd in &mut file.commands {
            cmd.source_path = Some(path.clone());
        }
        all.extend(file.commands);
    }
    Ok(CommandInventory { commands: all })
}
