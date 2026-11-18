use crate::backend::browser::BrowserBackend;
use crate::backend::terminal::TerminalBackend;
use crate::backend::vscode::VsCodeBackend;
use crate::backend::AppBackend;
use crate::config::repos::{load_repository_dir, LoadError};
use crate::frontend::status::RuntimeStatus;
use crate::mode::{infer_mode, ForegroundDetector, ModeSource, RuntimeMode};
use crate::runtime::actions::{register_builtin_actions, ActionRegistry};
use crate::runtime::command_inventory::CommandInventory;
use crate::runtime::errors::ExecutionError;
use crate::runtime::executor::{execute_matched_command, ExecutionContext};
use crate::runtime::python_host::PythonCompatibilityHost;
use crate::runtime::talon_ast::TalonFile;
use crate::runtime::talon_parser::parse_talon_file;
use crate::speech::{
    best_command_match_with_host, phrase_inventory_for_mode_with_host,
    resolve_input_device_selector, AudioDeviceInfo, MatchedCommand, SpeechCandidate, SpeechEngine,
    SpeechError,
};
use std::path::PathBuf;

/// Summary returned after loading or reloading Talon compatibility repositories.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RepoLoadSummary {
    pub repo_count: usize,
    pub files_loaded: usize,
    pub total_commands: usize,
    pub diagnostics: Vec<String>,
}

/// The main application object.
pub struct RuntimeApp {
    active_mode: RuntimeMode,
    mode_source: ModeSource,
    backends: Vec<Box<dyn AppBackend>>,
    foreground: Option<Box<dyn ForegroundDetector>>,
    inventory: CommandInventory,
    action_registry: ActionRegistry,
    python_host: PythonCompatibilityHost,
    last_error: Option<ExecutionError>,
    last_backend_invocation: Option<String>,
    last_matched_command: Option<String>,
    active_audio_device: Option<String>,
    listening: bool,
    configured_repos: Vec<PathBuf>,
}

impl RuntimeApp {
    /// Create a minimal app instance suitable for unit tests.
    pub fn new_for_test() -> Self {
        let mut action_registry = ActionRegistry::default();
        register_builtin_actions(&mut action_registry);
        action_registry.register_builtin("user.test_noop");

        Self {
            active_mode: RuntimeMode::Global,
            mode_source: ModeSource::Default,
            backends: Vec::new(),
            foreground: None,
            inventory: CommandInventory::default(),
            action_registry,
            python_host: PythonCompatibilityHost::default(),
            last_error: None,
            last_backend_invocation: None,
            last_matched_command: None,
            active_audio_device: None,
            listening: true,
            configured_repos: Vec::new(),
        }
    }

    /// Creates an app instance with connected VS Code, Browser, and Terminal backends,
    /// loading `repo_dirs` if non-empty or built-in sample commands otherwise.
    pub fn with_default_backends(mode: RuntimeMode, repo_dirs: &[PathBuf]) -> Result<Self, LoadError> {
        if repo_dirs.is_empty() {
            return Ok(Self::new_for_end_to_end_test(mode));
        }
        let mut app = Self::new_for_test();
        app.set_manual_mode(mode);
        app.register_backend(Box::new(VsCodeBackend::new_for_test_connected()));
        app.register_backend(Box::new(BrowserBackend::new_for_test_connected()));
        app.register_backend(Box::new(TerminalBackend::new_for_test_connected()));
        app.reload_repositories(repo_dirs)?;
        Ok(app)
    }

    /// Create an app instance pre-wired with connected backends, test lists, and
    /// representative workflow commands for end-to-end phrase dispatch tests.
    pub fn new_for_end_to_end_test(mode: RuntimeMode) -> Self {
        let mut app = Self::new_for_test();
        app.set_manual_mode(mode);
        app.register_backend(Box::new(VsCodeBackend::new_for_test_connected()));
        app.register_backend(Box::new(BrowserBackend::new_for_test_connected()));
        app.register_backend(Box::new(TerminalBackend::new_for_test_connected()));

        app.python_host
            .load_python_source(
                r#"
from talon import Module, Context

mod = Module()
mod.list("letter")

ctx = Context()
ctx.lists["user.letter"] = {"air": "a", "bat": "b", "cap": "c"}
"#,
                "e2e_lists.py",
            )
            .expect("embedded python test lists should load");

        let browser_file = parse_talon_file(
            "\
tag: browser
-
open hint {user.letter}: user.rango_open_hint(letter)
",
        )
        .unwrap();
        let code_file = parse_talon_file(
            "\
app: vscode
-
take line: user.cursorless_take_line()
",
        )
        .unwrap();
        let terminal_file = parse_talon_file(
            "\
tag: terminal
-
status check: insert(\"git status\")
",
        )
        .unwrap();

        app.set_inventory(CommandInventory::from_files(vec![
            browser_file,
            code_file,
            terminal_file,
        ]));
        app
    }

    pub fn active_mode(&self) -> RuntimeMode {
        self.active_mode
    }

    pub fn mode_source(&self) -> ModeSource {
        self.mode_source
    }

    /// Cycle through the supported modes in a fixed, user-visible order.
    pub fn cycle_mode(&mut self) {
        self.active_mode = self.active_mode.next_in_cycle();
        self.mode_source = ModeSource::ManualOverride;
    }

    /// Manually set the active mode. This should beat any automatic selection.
    pub fn set_manual_mode(&mut self, mode: RuntimeMode) {
        self.active_mode = mode;
        self.mode_source = ModeSource::ManualOverride;
    }

    /// Clear a previous manual mode override.
    pub fn clear_manual_mode(&mut self) {
        if self.mode_source.is_manual_override() {
            self.active_mode = RuntimeMode::Global;
            self.mode_source = ModeSource::Default;
        }
    }

    /// Configure a foreground detector implementation.
    pub fn set_foreground_detector(&mut self, detector: Box<dyn ForegroundDetector>) {
        self.foreground = Some(detector);
    }

    /// Refresh the active mode based on detected foreground application.
    ///
    /// Manual override always wins. If detection fails or inference yields no
    /// mapping, this is a no-op.
    pub fn refresh_mode_from_foreground(&mut self) {
        if self.mode_source.is_manual_override() {
            return;
        }

        let Some(detector) = self.foreground.as_ref() else {
            return;
        };
        let Some(app) = detector.current_app() else {
            return;
        };
        let Some(mode) = infer_mode(&app) else {
            return;
        };

        self.active_mode = mode;
        self.mode_source = ModeSource::Foreground;
    }

    pub fn backends(&self) -> &[Box<dyn AppBackend>] {
        &self.backends
    }

    pub fn register_backend(&mut self, backend: Box<dyn AppBackend>) {
        self.backends.push(backend);
    }

    pub fn set_inventory(&mut self, inventory: CommandInventory) {
        self.inventory = inventory;
    }

    pub fn inventory(&self) -> &CommandInventory {
        &self.inventory
    }

    pub fn configured_repos(&self) -> &[PathBuf] {
        &self.configured_repos
    }

    /// Reloads `.py` and `.talon` files from `repo_dirs` into the runtime's Python host
    /// and command inventory.
    pub fn reload_repositories(
        &mut self,
        repo_dirs: &[PathBuf],
    ) -> Result<RepoLoadSummary, LoadError> {
        let dirs: Vec<PathBuf> = repo_dirs
            .iter()
            .filter(|p| !p.as_os_str().is_empty())
            .cloned()
            .collect();

        let mut python_host = PythonCompatibilityHost::default();
        let mut files: Vec<TalonFile> = Vec::new();
        let mut diagnostics: Vec<String> = Vec::new();

        for dir in &dirs {
            load_repository_dir(dir, &mut files, &mut python_host, &mut diagnostics)?;
        }

        let files_loaded = files.len();
        let total_commands: usize = files.iter().map(|f| f.commands.len()).sum();
        let repo_count = dirs.len();

        self.python_host = python_host;
        self.inventory = CommandInventory::from_files(files);
        self.configured_repos = dirs;

        Ok(RepoLoadSummary {
            repo_count,
            files_loaded,
            total_commands,
            diagnostics,
        })
    }

    /// Restores the built-in fallback sample commands when all repository folders are cleared.
    pub fn restore_fallback_commands(&mut self) {
        let sample = Self::new_for_end_to_end_test(self.active_mode);
        self.python_host = sample.python_host;
        self.inventory = sample.inventory;
        self.configured_repos.clear();
    }

    pub fn python_host(&self) -> &PythonCompatibilityHost {
        &self.python_host
    }

    pub fn python_host_mut(&mut self) -> &mut PythonCompatibilityHost {
        &mut self.python_host
    }

    pub fn action_registry(&self) -> &ActionRegistry {
        &self.action_registry
    }

    pub fn action_registry_mut(&mut self) -> &mut ActionRegistry {
        &mut self.action_registry
    }

    pub fn last_error(&self) -> Option<&ExecutionError> {
        self.last_error.as_ref()
    }

    pub fn set_last_error_for_test(&mut self, err: ExecutionError) {
        self.last_error = Some(err);
    }

    pub fn last_backend_invocation_for_test(&self) -> &str {
        self.last_backend_invocation.as_deref().unwrap_or("")
    }

    pub fn last_matched_command_for_test(&self) -> Option<&str> {
        self.last_matched_command.as_deref()
    }

    pub fn active_audio_device(&self) -> Option<&str> {
        self.active_audio_device.as_deref()
    }

    pub fn set_audio_device(&mut self, device: impl Into<String>) {
        self.active_audio_device = Some(device.into());
    }

    pub fn clear_audio_device(&mut self) {
        self.active_audio_device = None;
    }

    /// Resolves `selector` (1-based index, exact name, or substring) against `available`
    /// and sets the active audio input device on the runtime.
    pub fn select_audio_device(
        &mut self,
        selector: &str,
        available: &[AudioDeviceInfo],
    ) -> Result<AudioDeviceInfo, SpeechError> {
        let info = resolve_input_device_selector(selector, available)?;
        self.active_audio_device = Some(info.name.clone());
        Ok(info)
    }

    pub fn is_listening(&self) -> bool {
        self.listening
    }

    pub fn set_listening(&mut self, listening: bool) {
        self.listening = listening;
    }

    pub fn toggle_listening(&mut self) -> bool {
        self.listening = !self.listening;
        self.listening
    }

    pub fn status(&self) -> RuntimeStatus {
        RuntimeStatus {
            mode: self.active_mode,
            mode_source: self.mode_source,
            audio_device: self.active_audio_device.clone(),
            listening: self.listening,
        }
    }

    /// Executes a matched command against the active mode, backends, Python host, and registry.
    pub fn execute_command(&mut self, command: &MatchedCommand) -> Result<(), ExecutionError> {
        self.last_backend_invocation = None;
        let mut ctx = ExecutionContext {
            mode: self.active_mode,
            action_registry: &self.action_registry,
            python_host: &self.python_host,
            backends: &mut self.backends,
            invoked_backend: None,
        };

        match execute_matched_command(command, &mut ctx) {
            Ok(()) => {
                self.last_error = None;
                self.last_backend_invocation = ctx.invoked_backend;
                self.last_matched_command = Some(command.spoken_form().to_string());
                Ok(())
            }
            Err(err) => {
                self.last_error = Some(err.clone());
                Err(err)
            }
        }
    }

    /// Matches a recognized phrase in the active mode and executes it.
    pub fn handle_recognized_phrase(&mut self, phrase: &str) -> Result<(), ExecutionError> {
        let candidates = [SpeechCandidate {
            transcript: phrase.to_string(),
            confidence: 1.0,
        }];
        let _ = self.execute_candidates(&candidates)?;
        Ok(())
    }

    /// Captures/recognizes one utterance via `engine` using the active mode's phrase
    /// inventory and executes the matched command (or built-in voice mode command).
    ///
    /// Returns `Ok(None)` when speech recognition yields no non-empty transcript, or
    /// `Ok(Some(transcript))` when a command executed.
    pub fn recognize_and_execute_once(
        &mut self,
        engine: &mut dyn SpeechEngine,
    ) -> Result<Option<String>, ExecutionError> {
        if !self.listening {
            return Ok(None);
        }
        let phrases =
            phrase_inventory_for_mode_with_host(&self.inventory, &self.python_host, self.active_mode);
        let candidates = engine.recognize_command(&phrases).map_err(|e| {
            let err = ExecutionError::BackendUnavailable(e.to_string());
            self.last_error = Some(err.clone());
            err
        })?;
        if candidates.is_empty() {
            return Ok(None);
        }
        self.execute_candidates(&candidates)
    }

    /// Executes already-recognized speech candidates against mode voice commands and active rules.
    pub fn execute_candidates(
        &mut self,
        candidates: &[SpeechCandidate],
    ) -> Result<Option<String>, ExecutionError> {
        let heard = candidates
            .iter()
            .max_by(|a, b| a.confidence.total_cmp(&b.confidence))
            .map(|c| c.transcript.clone())
            .unwrap_or_default();

        if let Some(handled_mode) = self.try_handle_mode_voice_command(candidates) {
            return Ok(Some(handled_mode));
        }

        let Some(matched) = best_command_match_with_host(
            candidates,
            &self.inventory,
            Some(&self.python_host),
            self.active_mode,
        ) else {
            let err = ExecutionError::ActionNotFound(format!(
                "no command matched phrase `{heard}` in mode `{}`",
                self.active_mode
            ));
            self.last_error = Some(err.clone());
            return Err(err);
        };

        self.execute_command(&matched)?;
        Ok(Some(heard))
    }

    fn try_handle_mode_voice_command(&mut self, candidates: &[SpeechCandidate]) -> Option<String> {
        let mut ordered: Vec<&SpeechCandidate> = candidates.iter().collect();
        ordered.sort_by(|a, b| b.confidence.total_cmp(&a.confidence));

        for cand in ordered {
            let norm = cand.transcript.trim().to_ascii_lowercase();
            let handled = match norm.as_str() {
                "code mode" => {
                    self.set_manual_mode(RuntimeMode::Code);
                    true
                }
                "browser mode" => {
                    self.set_manual_mode(RuntimeMode::Browser);
                    true
                }
                "terminal mode" => {
                    self.set_manual_mode(RuntimeMode::Terminal);
                    true
                }
                "global mode" => {
                    self.set_manual_mode(RuntimeMode::Global);
                    true
                }
                "cycle mode" => {
                    self.cycle_mode();
                    true
                }
                "auto mode" => {
                    self.clear_manual_mode();
                    true
                }
                "default microphone" => {
                    self.clear_audio_device();
                    self.last_error = None;
                    self.last_backend_invocation = None;
                    self.last_matched_command = Some("audio -> system default".into());
                    return Some(cand.transcript.clone());
                }
                _ => false,
            };
            if handled {
                self.last_error = None;
                self.last_backend_invocation = None;
                self.last_matched_command = Some(format!("mode -> {}", self.active_mode));
                return Some(cand.transcript.clone());
            }
        }
        None
    }
}
