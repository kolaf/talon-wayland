use crate::runtime::talon_ast::TalonCommand;
use std::collections::HashMap;

/// A candidate transcript produced by a command-oriented `SpeechEngine`.
#[derive(Debug, Clone, PartialEq)]
pub struct SpeechCandidate {
    pub transcript: String,
    pub confidence: f32,
}

/// A `TalonCommand` matched against a `SpeechCandidate`, together with any
/// captured list/capture variable bindings extracted from the spoken rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchedCommand {
    pub command: TalonCommand,
    pub captures: HashMap<String, String>,
}

impl MatchedCommand {
    pub fn new(command: TalonCommand, captures: HashMap<String, String>) -> Self {
        Self { command, captures }
    }

    pub fn spoken_form(&self) -> &str {
        &self.command.spoken_form
    }

    pub fn command(&self) -> &TalonCommand {
        &self.command
    }

    pub fn captures(&self) -> &HashMap<String, String> {
        &self.captures
    }
}
