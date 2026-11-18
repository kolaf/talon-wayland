use crate::mode::RuntimeMode;
use crate::runtime::talon_ast::{base_tags_for_mode, TalonCommand, TalonFile};
use crate::runtime::talon_parser::{parse_talon_file, TalonParseError};
use std::collections::HashSet;
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
    files: Vec<TalonFile>,
    commands: Vec<TalonCommand>,
}

impl CommandInventory {
    pub fn from_test_commands(commands: Vec<TalonCommand>) -> Self {
        Self {
            files: Vec::new(),
            commands,
        }
    }

    pub fn from_files(files: Vec<TalonFile>) -> Self {
        let mut commands = Vec::new();
        for file in &files {
            commands.extend(file.commands.iter().cloned());
        }
        Self { files, commands }
    }

    pub fn files(&self) -> &[TalonFile] {
        &self.files
    }

    pub fn diagnostics(&self) -> Vec<&str> {
        self.files
            .iter()
            .flat_map(|f| f.diagnostics.iter().map(String::as_str))
            .collect()
    }

    /// Computes all active tags for `mode`, including transitive `tag(): ...`
    /// activations from matching `.talon` files.
    pub fn active_tags_for_mode(&self, mode: RuntimeMode) -> HashSet<String> {
        let mut tags = base_tags_for_mode(mode);
        loop {
            let mut added = false;
            for file in &self.files {
                if file.activated_tags.is_empty() {
                    continue;
                }
                if file.context.matches_workflow_mode(mode, &tags, None) {
                    for tag in &file.activated_tags {
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

    /// Returns commands belonging specifically to `mode` (with OS, Talon speech mode,
    /// app profile, and transitive tag filtering applied).
    pub fn commands_for_mode(&self, mode: RuntimeMode) -> Vec<&TalonCommand> {
        let active_tags = self.active_tags_for_mode(mode);
        self.commands
            .iter()
            .filter(|cmd| cmd.context.matches_workflow_mode(mode, &active_tags, None))
            .collect()
    }

    /// Returns workflow-specific commands for `mode` plus shared `Global` commands
    /// when `mode != RuntimeMode::Global`.
    pub fn effective_commands_for_mode(&self, mode: RuntimeMode) -> Vec<&TalonCommand> {
        let mut cmds = self.commands_for_mode(mode);
        if mode != RuntimeMode::Global {
            cmds.extend(self.commands_for_mode(RuntimeMode::Global));
        }
        cmds
    }
}

pub fn load_inventory(paths: &[PathBuf]) -> Result<CommandInventory, InventoryError> {
    let mut files: Vec<TalonFile> = Vec::with_capacity(paths.len());
    let mut all: Vec<TalonCommand> = Vec::new();
    for path in paths {
        let src = std::fs::read_to_string(path).map_err(|e| InventoryError::Io {
            path: path.clone(),
            source: e,
        })?;
        let mut file: TalonFile = parse_talon_file(&src).map_err(|e| InventoryError::Parse {
            path: path.clone(),
            source: e,
        })?;
        for cmd in &mut file.commands {
            cmd.source_path = Some(path.clone());
        }
        all.extend(file.commands.iter().cloned());
        files.push(file);
    }
    Ok(CommandInventory {
        files,
        commands: all,
    })
}
