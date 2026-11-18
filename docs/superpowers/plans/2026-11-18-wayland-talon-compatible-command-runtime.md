# Wayland-Native Talon-Compatible Command Runtime Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a separate Wayland-native command runtime that supports a Talon-compatible subset for VS Code / Cursorless, browser / Rango, and terminal workflows using a command-oriented recognizer.

**Architecture:** The runtime is a Rust-first daemon/application with a Talon-compatible grammar and action layer, plus app-specific backends for VS Code, browser, and terminal control. It uses automatic mode selection from foreground-app detection when available and falls back to explicit mode switching when not, while keeping dictation out of scope because Handy already covers that job.

**Tech Stack:** Rust core runtime, TypeScript UI/extension glue where needed, VS Code extension integration, browser-extension messaging, Wayland-native desktop integration, command-oriented local speech engine adapter.

**Spec:** `docs/superpowers/specs/2026-11-18-wayland-talon-compatible-command-runtime-design.md`

## Global Constraints

- Build a separate application; do not embed the first version into Handy.
- First-class workflows are only VS Code / Cursorless, browser / Rango, and terminal workflows.
- Compatibility target is limited to `talonhub/community`, `cursorless-talon`, and `rango-talon`.
- Use a command-oriented recognizer; a freeform dictation-first engine without grammar/context constraints is not acceptable.
- Support both automatic foreground-app mode selection and manual mode switching via cycle and direct shortcuts.
- Keep the runtime useful even if GNOME Wayland foreground detection is incomplete or flaky.
- Do not attempt full Talon parity, full desktop automation parity, eye tracking, noise commands, or arbitrary GUI control in v1.
- Keep dictation out of scope; Handy remains the dictation tool.

## Review Focus

- Commands that exist in supported Talon grammars but depend on unsupported Python action semantics should fail clearly rather than silently doing the wrong thing.
- Foreground-app detection must never leave the user trapped in the wrong mode without a visible indicator and a manual recovery path.
- Speech recognition biasing must not collapse distinct short commands into the same command candidate under realistic coding / browser command vocabularies.
- Browser and VS Code backends must reject stale or missing extension connections predictably instead of hanging command execution.
- Terminal command execution must distinguish safe text insertion from destructive synthetic key behavior and avoid surprising shell-side effects.

---

### Task 1: Create the workspace skeleton and product boundaries

**Files:**
- Create: `Cargo.toml`
- Create: `src/main.rs`
- Create: `src/lib.rs`
- Create: `src/app/mod.rs`
- Create: `src/config/mod.rs`
- Create: `src/mode/mod.rs`
- Create: `src/runtime/mod.rs`
- Create: `src/speech/mod.rs`
- Create: `src/backend/mod.rs`
- Create: `src/frontend/mod.rs`
- Create: `tests/smoke_startup.rs`
- Create: `README.md`

**Interfaces:**
- Consumes: none
- Produces:
  - `pub struct AppConfig`
  - `pub enum RuntimeMode { Global, Code, Browser, Terminal }`
  - `pub trait SpeechEngine`
  - `pub trait AppBackend`
  - `pub struct RuntimeApp`

- [ ] **Step 1: Write the failing startup test**

```rust
#[test]
fn starts_with_no_backends_connected() {
    let app = voice_runtime::app::RuntimeApp::new_for_test();
    assert_eq!(app.active_mode().to_string(), "global");
    assert!(app.backends().is_empty());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --manifest-path Cargo.toml starts_with_no_backends_connected -- --exact --nocapture`
Expected: FAIL because the crate and `RuntimeApp::new_for_test()` do not exist yet

- [ ] **Step 3: Implement the workspace skeleton and `RuntimeApp::new_for_test() -> RuntimeApp`**

Create a standalone Rust crate with focused modules matching the design doc boundaries. `RuntimeApp` should expose `active_mode()` and `backends()` so later tasks can wire mode selection and backend registration without reshaping the crate.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --manifest-path Cargo.toml starts_with_no_backends_connected -- --exact --nocapture`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add docs/superpowers/plans/2026-11-18-wayland-talon-compatible-command-runtime.md docs/superpowers/specs/2026-11-18-wayland-talon-compatible-command-runtime-design.md
git commit -m "feat: scaffold wayland talon-compatible runtime"
```

### Task 2: Add mode model, visible mode state, and manual switching

**Files:**
- Modify: `src/app/mod.rs`
- Modify: `src/mode/mod.rs`
- Create: `src/frontend/status.rs`
- Create: `tests/manual_mode_switching.rs`

**Interfaces:**
- Consumes:
  - `RuntimeApp`
  - `RuntimeMode`
- Produces:
  - `pub enum ModeSource { Default, Foreground, ManualOverride }`
  - `pub fn active_mode(&self) -> RuntimeMode`
  - `pub fn mode_source(&self) -> ModeSource`
  - `pub fn cycle_mode(&mut self)`
  - `pub fn set_manual_mode(&mut self, mode: RuntimeMode)`
  - `pub fn clear_manual_mode(&mut self)`

- [ ] **Step 1: Write the failing tests for manual mode switching**

```rust
#[test]
fn manual_mode_override_beats_default_mode() {
    let mut app = RuntimeApp::new_for_test();
    app.set_manual_mode(RuntimeMode::Browser);
    assert_eq!(app.active_mode(), RuntimeMode::Browser);
    assert_eq!(app.mode_source(), ModeSource::ManualOverride);
}

#[test]
fn cycle_mode_visits_code_browser_terminal_and_global() {
    let mut app = RuntimeApp::new_for_test();
    app.cycle_mode();
    assert_eq!(app.active_mode(), RuntimeMode::Code);
    app.cycle_mode();
    assert_eq!(app.active_mode(), RuntimeMode::Browser);
    app.cycle_mode();
    assert_eq!(app.active_mode(), RuntimeMode::Terminal);
    app.cycle_mode();
    assert_eq!(app.active_mode(), RuntimeMode::Global);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path Cargo.toml manual_mode -- --nocapture`
Expected: FAIL because mode override APIs do not exist yet

- [ ] **Step 3: Implement manual mode APIs and a visible mode-state surface**

Store both the current mode and the source of truth (`Default`, `Foreground`, `ManualOverride`). `frontend/status.rs` should define a small status projection that later UI work can expose directly without reaching into core internals.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --manifest-path Cargo.toml manual_mode -- --nocapture`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/app/mod.rs voice-runtime/src/mode/mod.rs voice-runtime/src/frontend/status.rs voice-runtime/tests/manual_mode_switching.rs
git commit -m "feat: add runtime mode switching primitives"
```

### Task 3: Add foreground-app detection abstraction and mode routing

**Files:**
- Modify: `src/app/mod.rs`
- Modify: `src/mode/mod.rs`
- Create: `src/frontend/focus.rs`
- Create: `tests/foreground_mode_routing.rs`

**Interfaces:**
- Consumes:
  - `RuntimeMode`
  - `ModeSource`
  - manual mode APIs from Task 2
- Produces:
  - `pub struct ForegroundAppInfo { app_id: String, title: String }`
  - `pub trait ForegroundDetector { fn current_app(&self) -> Option<ForegroundAppInfo>; }`
  - `pub fn infer_mode(app: &ForegroundAppInfo) -> Option<RuntimeMode>`
  - `pub fn refresh_mode_from_foreground(&mut self)`

- [ ] **Step 1: Write the failing tests for foreground routing**

```rust
#[test]
fn vscode_maps_to_code_mode() {
    let app = ForegroundAppInfo { app_id: "code".into(), title: "main.rs - Visual Studio Code".into() };
    assert_eq!(infer_mode(&app), Some(RuntimeMode::Code));
}

#[test]
fn vivaldi_maps_to_browser_mode() {
    let app = ForegroundAppInfo { app_id: "vivaldi-stable".into(), title: "Docs - Vivaldi".into() };
    assert_eq!(infer_mode(&app), Some(RuntimeMode::Browser));
}

#[test]
fn manual_override_is_not_replaced_by_foreground_refresh() {
    let mut app = RuntimeApp::new_for_test();
    app.set_manual_mode(RuntimeMode::Terminal);
    app.refresh_mode_from_foreground();
    assert_eq!(app.active_mode(), RuntimeMode::Terminal);
    assert_eq!(app.mode_source(), ModeSource::ManualOverride);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path Cargo.toml foreground_mode -- --nocapture`
Expected: FAIL because foreground detector and inference APIs do not exist yet

- [ ] **Step 3: Implement `ForegroundDetector`, `ForegroundAppInfo`, and mode inference**

Start with deterministic mapping tables for known app ids / title patterns for VS Code, VSCodium, Vivaldi, and common terminal emulators. Keep the detector itself behind a trait so GNOME Wayland-specific implementations can be added later without touching mode logic.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --manifest-path Cargo.toml foreground_mode -- --nocapture`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/app/mod.rs voice-runtime/src/mode/mod.rs voice-runtime/src/frontend/focus.rs voice-runtime/tests/foreground_mode_routing.rs
git commit -m "feat: add foreground-driven mode routing"
```

### Task 4: Build the `.talon` parser and command inventory subset

**Files:**
- Create: `src/runtime/talon_ast.rs`
- Create: `src/runtime/talon_parser.rs`
- Create: `src/runtime/command_inventory.rs`
- Create: `tests/talon_parser_subset.rs`
- Create: `tests/command_inventory_contexts.rs`

**Interfaces:**
- Consumes:
  - `RuntimeMode`
- Produces:
  - `pub struct TalonFile`
  - `pub struct TalonContextHeader`
  - `pub struct TalonCommand`
  - `pub enum TalonBodyStmt`
  - `pub fn parse_talon_file(source: &str) -> Result<TalonFile, TalonParseError>`
  - `pub struct CommandInventory`
  - `pub fn load_inventory(paths: &[PathBuf]) -> Result<CommandInventory, InventoryError>`
  - `pub fn commands_for_mode(&self, mode: RuntimeMode) -> Vec<&TalonCommand>`

- [ ] **Step 1: Write the failing parser and inventory tests**

```rust
#[test]
fn parses_context_header_and_simple_command() {
    let src = "app.name: Code\ntag: user.tabs\n-\nfocus left: key(ctrl-left)\n";
    let file = parse_talon_file(src).unwrap();
    assert_eq!(file.context.app_name.as_deref(), Some("Code"));
    assert_eq!(file.context.tags, vec!["user.tabs"]);
    assert_eq!(file.commands[0].spoken_form, "focus left");
}

#[test]
fn inventory_filters_code_commands_by_mode() {
    let inventory = CommandInventory::from_test_commands(vec![
        test_command(RuntimeMode::Code, "take line"),
        test_command(RuntimeMode::Browser, "open hint"),
    ]);
    let spoken = inventory.commands_for_mode(RuntimeMode::Code)
        .into_iter()
        .map(|cmd| cmd.spoken_form.as_str())
        .collect::<Vec<_>>();
    assert_eq!(spoken, vec!["take line"]);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path Cargo.toml talon_parser_subset command_inventory_contexts -- --nocapture`
Expected: FAIL because parser and inventory APIs do not exist yet

- [ ] **Step 3: Implement the supported `.talon` subset and inventory loader**

Support only the subset named in the spec:
- context headers for `app`, `app.name`, `os`, `mode`, `tag`, and `code.language`
- command definitions
- command bodies containing action invocations and tag activation needed by supported repos

Do not guess unsupported constructs; return explicit parse or support errors for features outside the subset.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --manifest-path Cargo.toml talon_parser_subset command_inventory_contexts -- --nocapture`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/runtime voice-runtime/tests/talon_parser_subset.rs voice-runtime/tests/command_inventory_contexts.rs
git commit -m "feat: add talon grammar subset parser"
```

### Task 5: Add action registry and Python compatibility subset

**Files:**
- Create: `src/runtime/actions.rs`
- Create: `src/runtime/python_host.rs`
- Create: `tests/action_registry.rs`
- Create: `tests/python_context_overrides.rs`

**Interfaces:**
- Consumes:
  - `TalonCommand`
  - `TalonBodyStmt`
  - mode/context primitives from Tasks 2 and 3
- Produces:
  - `pub trait RuntimeAction`
  - `pub struct ActionRegistry`
  - `pub fn register_builtin_actions(registry: &mut ActionRegistry)`
  - `pub struct PythonCompatibilityHost`
  - `pub fn load_python_module(path: &Path) -> Result<(), PythonHostError>`
  - `pub fn resolve_action(name: &str, context: &ResolvedContext) -> Result<ActionHandle, ActionError>`

- [ ] **Step 1: Write the failing tests for action registration and context override resolution**

```rust
#[test]
fn resolves_builtin_action_by_name() {
    let mut registry = ActionRegistry::default();
    register_builtin_actions(&mut registry);
    assert!(registry.resolve("insert").is_ok());
}

#[test]
fn context_override_wins_over_default_action() {
    let host = PythonCompatibilityHost::from_test_override(
        "user.open_target",
        "browser",
        "browser_impl",
        "default_impl",
    );
    assert_eq!(host.resolve_for_test("user.open_target", "browser"), "browser_impl");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path Cargo.toml action_registry python_context_overrides -- --nocapture`
Expected: FAIL because the action system does not exist yet

- [ ] **Step 3: Implement the supported Talon action/Python subset**

Implement only the Python/Talon constructs needed by the chosen compatibility sources:
- module-defined actions
- context overrides
- list/capture registration needed by supported grammars

Keep this behind a host layer so a future embedded Python engine can be replaced or narrowed without rewriting the runtime.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --manifest-path Cargo.toml action_registry python_context_overrides -- --nocapture`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/runtime/actions.rs voice-runtime/src/runtime/python_host.rs voice-runtime/tests/action_registry.rs voice-runtime/tests/python_context_overrides.rs
git commit -m "feat: add talon action compatibility layer"
```

### Task 6: Add speech engine abstraction and command-oriented candidate matching

**Files:**
- Modify: `src/speech/mod.rs`
- Create: `src/speech/candidate.rs`
- Create: `src/speech/engine.rs`
- Create: `src/speech/matcher.rs`
- Create: `tests/command_candidate_matching.rs`
- Create: `tests/mode_specific_phrase_inventory.rs`

**Interfaces:**
- Consumes:
  - `CommandInventory`
  - `RuntimeMode`
- Produces:
  - `pub struct SpeechCandidate { transcript: String, confidence: f32 }`
  - `pub trait SpeechEngine { fn recognize_command(&mut self, phrases: &[String]) -> Result<Vec<SpeechCandidate>, SpeechError>; }`
  - `pub fn phrase_inventory_for_mode(inventory: &CommandInventory, mode: RuntimeMode) -> Vec<String>`
  - `pub fn best_command_match(candidates: &[SpeechCandidate], inventory: &CommandInventory, mode: RuntimeMode) -> Option<MatchedCommand>`

- [ ] **Step 1: Write the failing tests for command-oriented matching**

```rust
#[test]
fn code_mode_phrase_inventory_excludes_browser_only_commands() {
    let inventory = test_inventory([
        (RuntimeMode::Code, "take line"),
        (RuntimeMode::Browser, "open hint"),
    ]);
    assert_eq!(phrase_inventory_for_mode(&inventory, RuntimeMode::Code), vec!["take line"]);
}

#[test]
fn best_match_prefers_exact_mode_specific_command() {
    let inventory = test_inventory([
        (RuntimeMode::Code, "take line"),
        (RuntimeMode::Global, "take"),
    ]);
    let candidates = vec![SpeechCandidate { transcript: "take line".into(), confidence: 0.81 }];
    assert_eq!(best_command_match(&candidates, &inventory, RuntimeMode::Code).unwrap().spoken_form(), "take line");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path Cargo.toml command_candidate_matching mode_specific_phrase_inventory -- --nocapture`
Expected: FAIL because the speech matching APIs do not exist yet

- [ ] **Step 3: Implement the speech-engine adapter and phrase-matching layer**

Do not bind to a specific model yet. Build the runtime-facing abstraction that accepts a mode-specific phrase inventory and returns matched commands. The matching layer should fail clearly when no command candidate is supported rather than guessing a dictation-like fallback.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --manifest-path Cargo.toml command_candidate_matching mode_specific_phrase_inventory -- --nocapture`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/speech voice-runtime/tests/command_candidate_matching.rs voice-runtime/tests/mode_specific_phrase_inventory.rs
git commit -m "feat: add command-oriented speech matching layer"
```

### Task 7: Add command execution pipeline and explicit unsupported-feature errors

**Files:**
- Modify: `src/app/mod.rs`
- Create: `src/runtime/executor.rs`
- Create: `src/runtime/errors.rs`
- Create: `tests/unsupported_command_reporting.rs`
- Create: `tests/execution_pipeline.rs`

**Interfaces:**
- Consumes:
  - `MatchedCommand`
  - `ActionRegistry`
  - mode state from Tasks 2 and 3
- Produces:
  - `pub enum ExecutionError { UnsupportedFeature(String), BackendUnavailable(String), ActionNotFound(String), ContextMismatch(String) }`
  - `pub fn execute_command(&mut self, command: &MatchedCommand) -> Result<(), ExecutionError>`
  - `pub fn last_error(&self) -> Option<&ExecutionError>`

- [ ] **Step 1: Write the failing tests for execution and unsupported-feature handling**

```rust
#[test]
fn unsupported_action_returns_explicit_error() {
    let mut app = RuntimeApp::new_for_test();
    let command = test_matched_command("unsupported thing", "user.unsupported_action");
    let err = app.execute_command(&command).unwrap_err();
    assert!(matches!(err, ExecutionError::ActionNotFound(_)|ExecutionError::UnsupportedFeature(_)));
}

#[test]
fn successful_execution_clears_previous_error() {
    let mut app = RuntimeApp::new_for_test();
    app.set_last_error_for_test(ExecutionError::BackendUnavailable("browser".into()));
    let command = test_matched_command("noop", "user.test_noop");
    app.execute_command(&command).unwrap();
    assert!(app.last_error().is_none());
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path Cargo.toml unsupported_command_reporting execution_pipeline -- --nocapture`
Expected: FAIL because the execution pipeline does not exist yet

- [ ] **Step 3: Implement the command execution pipeline and structured errors**

Every unsupported Talon construct or missing backend/action should return a precise error that can be surfaced in UI/logging. Do not silently drop commands.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --manifest-path Cargo.toml unsupported_command_reporting execution_pipeline -- --nocapture`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/app/mod.rs voice-runtime/src/runtime/executor.rs voice-runtime/src/runtime/errors.rs voice-runtime/tests/unsupported_command_reporting.rs voice-runtime/tests/execution_pipeline.rs
git commit -m "feat: add command execution pipeline"
```

### Task 8: Implement the VS Code / Cursorless backend

**Files:**
- Modify: `src/backend/mod.rs`
- Create: `src/backend/vscode.rs`
- Create: `tests/vscode_backend.rs`
- Create: `tests/cursorless_command_mapping.rs`

**Interfaces:**
- Consumes:
  - `AppBackend`
  - `execute_command()` pipeline
  - command/action compatibility layers from Tasks 4, 5, and 7
- Produces:
  - `pub struct VsCodeBackend`
  - `impl AppBackend for VsCodeBackend`
  - `pub fn map_cursorless_action(action_name: &str, args: &[RuntimeValue]) -> Result<VsCodeCommand, ExecutionError>`

- [ ] **Step 1: Write the failing backend tests**

```rust
#[test]
fn cursorless_style_action_maps_to_vscode_command() {
    let cmd = map_cursorless_action("user.cursorless_take_line", &[]).unwrap();
    assert_eq!(cmd.command_name, "cursorless.takeLine");
}

#[test]
fn vscode_backend_rejects_send_when_disconnected() {
    let backend = VsCodeBackend::new_for_test_disconnected();
    let err = backend.send_for_test(test_vscode_command()).unwrap_err();
    assert!(matches!(err, ExecutionError::BackendUnavailable(_)));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path Cargo.toml vscode_backend cursorless_command_mapping -- --nocapture`
Expected: FAIL because the VS Code backend does not exist yet

- [ ] **Step 3: Implement `VsCodeBackend` and initial Cursorless command mapping**

Use the open Cursorless command-server / client ecosystem as the reference seam. Prefer semantic command dispatch over synthetic key presses whenever the semantic path exists.

- [ ] **Step 4: Add review-focus coverage for stale backend connections**

Add a test proving that a previously connected VS Code backend returns `BackendUnavailable` after simulated disconnect instead of hanging or reporting success.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --manifest-path Cargo.toml vscode_backend cursorless_command_mapping -- --nocapture`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add src/backend/mod.rs voice-runtime/src/backend/vscode.rs voice-runtime/tests/vscode_backend.rs voice-runtime/tests/cursorless_command_mapping.rs
git commit -m "feat: add vscode cursorless backend"
```

### Task 9: Implement the browser / Rango backend

**Files:**
- Modify: `src/backend/mod.rs`
- Create: `src/backend/browser.rs`
- Create: `tests/browser_backend.rs`
- Create: `tests/rango_command_mapping.rs`

**Interfaces:**
- Consumes:
  - `AppBackend`
  - execution pipeline from Task 7
- Produces:
  - `pub struct BrowserBackend`
  - `impl AppBackend for BrowserBackend`
  - `pub fn map_rango_action(action_name: &str, args: &[RuntimeValue]) -> Result<BrowserCommand, ExecutionError>`

- [ ] **Step 1: Write the failing browser backend tests**

```rust
#[test]
fn rango_open_hint_action_maps_to_browser_command() {
    let cmd = map_rango_action("user.rango_open_hint", &[]).unwrap();
    assert_eq!(cmd.command_name, "rango.openHint");
}

#[test]
fn browser_backend_reports_missing_extension() {
    let backend = BrowserBackend::new_for_test_disconnected();
    let err = backend.send_for_test(test_browser_command()).unwrap_err();
    assert!(matches!(err, ExecutionError::BackendUnavailable(_)));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path Cargo.toml browser_backend rango_command_mapping -- --nocapture`
Expected: FAIL because the browser backend does not exist yet

- [ ] **Step 3: Implement the browser backend and initial Rango mapping**

Use extension messaging as the integration seam. Keep DOM hinting and in-browser state in the extension side; the runtime is responsible only for mapping Talon-style commands into extension commands.

- [ ] **Step 4: Add review-focus coverage for stale browser extension connections**

Add a test proving that an extension disconnect after startup is surfaced as `BackendUnavailable` rather than an ignored command.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --manifest-path Cargo.toml browser_backend rango_command_mapping -- --nocapture`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add src/backend/browser.rs voice-runtime/src/backend/mod.rs voice-runtime/tests/browser_backend.rs voice-runtime/tests/rango_command_mapping.rs
git commit -m "feat: add browser rango backend"
```

### Task 10: Implement the terminal backend

**Files:**
- Modify: `src/backend/mod.rs`
- Create: `src/backend/terminal.rs`
- Create: `tests/terminal_backend.rs`
- Create: `tests/terminal_command_policy.rs`

**Interfaces:**
- Consumes:
  - execution pipeline from Task 7
  - mode routing from Task 3
- Produces:
  - `pub struct TerminalBackend`
  - `impl AppBackend for TerminalBackend`
  - `pub enum TerminalDispatch { InsertText(String), SendKeys(Vec<KeyStroke>) }`
  - `pub fn map_terminal_action(action_name: &str, args: &[RuntimeValue]) -> Result<TerminalDispatch, ExecutionError>`

- [ ] **Step 1: Write the failing terminal backend tests**

```rust
#[test]
fn terminal_insert_prefers_text_dispatch() {
    let dispatch = map_terminal_action("insert", &[RuntimeValue::String("git status".into())]).unwrap();
    assert!(matches!(dispatch, TerminalDispatch::InsertText(_)));
}

#[test]
fn destructive_terminal_key_action_requires_explicit_supported_mapping() {
    let err = map_terminal_action("user.unsupported_terminal_combo", &[]).unwrap_err();
    assert!(matches!(err, ExecutionError::UnsupportedFeature(_)));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path Cargo.toml terminal_backend terminal_command_policy -- --nocapture`
Expected: FAIL because the terminal backend does not exist yet

- [ ] **Step 3: Implement the terminal backend and dispatch policy**

Start with conservative behavior: prefer text insertion / paste for shell commands and support only a small explicit allowlist of terminal control keys. Do not infer synthetic key behavior for unsupported terminal actions.

- [ ] **Step 4: Add review-focus coverage for unsafe terminal dispatch**

Add a test showing that an unsupported terminal action returns `UnsupportedFeature` instead of being downgraded to guessed key presses.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --manifest-path Cargo.toml terminal_backend terminal_command_policy -- --nocapture`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add src/backend/terminal.rs voice-runtime/src/backend/mod.rs voice-runtime/tests/terminal_backend.rs voice-runtime/tests/terminal_command_policy.rs
git commit -m "feat: add terminal backend"
```

### Task 11: Wire compatible repository loading and end-to-end mode/backend integration

**Files:**
- Modify: `src/app/mod.rs`
- Create: `src/config/repos.rs`
- Create: `tests/repository_subset_loading.rs`
- Create: `tests/end_to_end_mode_dispatch.rs`

**Interfaces:**
- Consumes:
  - `CommandInventory`
  - `PythonCompatibilityHost`
  - all three backends
- Produces:
  - `pub struct CompatibilityReposConfig { community: PathBuf, cursorless_talon: PathBuf, rango_talon: PathBuf }`
  - `pub fn load_supported_repositories(config: &CompatibilityReposConfig) -> Result<LoadedCompatibility, LoadError>`
  - `pub fn handle_recognized_phrase(&mut self, phrase: &str) -> Result<(), ExecutionError>`

- [ ] **Step 1: Write the failing integration tests**

```rust
#[test]
fn loads_only_supported_repositories() {
    let loaded = load_supported_repositories(&test_repo_config()).unwrap();
    assert!(loaded.repo_names().contains(&"community".to_string()));
    assert!(loaded.repo_names().contains(&"cursorless-talon".to_string()));
    assert!(loaded.repo_names().contains(&"rango-talon".to_string()));
}

#[test]
fn recognized_phrase_routes_to_backend_for_active_mode() {
    let mut app = RuntimeApp::new_for_end_to_end_test(RuntimeMode::Browser);
    app.handle_recognized_phrase("open hint air").unwrap();
    assert_eq!(app.last_backend_invocation_for_test(), "browser");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path Cargo.toml repository_subset_loading end_to_end_mode_dispatch -- --nocapture`
Expected: FAIL because compatibility repo loading and phrase dispatch integration do not exist yet

- [ ] **Step 3: Implement supported repository loading and phrase dispatch integration**

Only the approved compatibility sources should load by default. Unsupported external repos should not be silently treated as first-class supported inputs.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --manifest-path Cargo.toml repository_subset_loading end_to_end_mode_dispatch -- --nocapture`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/app/mod.rs voice-runtime/src/config/repos.rs voice-runtime/tests/repository_subset_loading.rs voice-runtime/tests/end_to_end_mode_dispatch.rs
git commit -m "feat: wire supported talon repository loading"
```

### Task 12: Add GNOME Wayland integration wiring, startup configuration, and operator docs

**Files:**
- Modify: `README.md`
- Create: `docs/gnome-wayland-setup.md`
- Create: `tests/config_validation.rs`
- Create: `src/config/validation.rs`

**Interfaces:**
- Consumes:
  - `AppConfig`
  - `CompatibilityReposConfig`
  - manual/foreground mode systems
- Produces:
  - `pub fn validate_config(config: &AppConfig) -> Result<(), ConfigError>`
  - documented startup requirements for GNOME Wayland, mode switching, and backend setup

- [ ] **Step 1: Write the failing config validation tests**

```rust
#[test]
fn config_requires_all_supported_repository_paths() {
    let config = AppConfig::for_test_missing_rango_repo();
    let err = validate_config(&config).unwrap_err();
    assert!(err.to_string().contains("rango"));
}

#[test]
fn config_accepts_manual_mode_shortcut_and_direct_shortcuts() {
    let config = AppConfig::for_test_with_mode_shortcuts();
    assert!(validate_config(&config).is_ok());
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path Cargo.toml config_validation -- --nocapture`
Expected: FAIL because config validation and setup docs do not exist yet

- [ ] **Step 3: Implement config validation and write GNOME Wayland setup docs**

Document exactly what the operator must configure for:
- GNOME Wayland startup
- backend connections
- manual mode cycle shortcut
- direct mode shortcuts
- supported compatibility repository locations

Do not guess platform-wide automation support that has not been implemented.

- [ ] **Step 4: Add review-focus coverage for manual recovery from wrong foreground mode**

Add a test showing that a config with both cycle and direct mode shortcuts passes validation and is treated as satisfying the recovery requirement.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --manifest-path Cargo.toml config_validation -- --nocapture`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add README.md voice-runtime/docs/gnome-wayland-setup.md voice-runtime/src/config/validation.rs voice-runtime/tests/config_validation.rs
git commit -m "docs: add runtime setup and validation guidance"
```

## Self-Review

### 1. Spec coverage

- Intent and product shape → Tasks 1, 2, 3, 11, 12
- Separate application rather than Handy integration → Task 1 structure and Task 12 docs
- Automatic foreground mode + manual fallback → Tasks 2 and 3
- Talon-compatible subset rather than full parity → Tasks 4, 5, 7, 11
- VS Code / Cursorless support → Task 8
- Browser / Rango support → Task 9
- Terminal workflows → Task 10
- Command-oriented recognizer abstraction → Task 6
- Unsupported features must fail clearly → Task 7

No spec section is left without an owning task.

### 2. Step scan

- Every test step includes concrete assertions.
- Every code step names concrete files and interfaces.
- Verification steps include exact commands and expected pass/fail conditions.
- No step says only "handle edge cases" or "add validation" without naming the exact result.

### 3. Type consistency

- `RuntimeMode`, `ModeSource`, `ForegroundAppInfo`, `CommandInventory`, `SpeechCandidate`, `ExecutionError`, and backend structs are introduced before downstream tasks consume them.
- Backend tasks consume the same `ExecutionError` and `AppBackend` interface defined earlier.

### 4. Review Focus

- Unsupported Talon semantics → Task 7 explicit unsupported-feature tests
- Wrong foreground mode / no recovery → Tasks 2, 3, and 12 manual override and shortcut validation
- Speech candidate collapse across modes → Task 6 phrase-inventory and exact-match tests
- Stale backend connections → Tasks 8 and 9 disconnected/reconnect failure tests
- Unsafe terminal dispatch → Task 10 explicit unsupported-terminal-action test

### 5. Proportion

The plan is longer than the spec because it introduces exact files, interfaces, and tests for a greenfield project, but steps are still signature-and-assertion oriented rather than fully coding the solution.

### 6. Deferred work

- The exact first speech model is intentionally deferred, but the engine abstraction and command-oriented matching contract are not. Task 6 explicitly blocks model choice behind the adapter layer.
- Broader Talon compatibility outside the three approved repositories is intentionally deferred and called out in Task 11.

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-11-18-wayland-talon-compatible-command-runtime.md`. Please review the plan. Which execution approach would you prefer?

- **Subagent-driven** - A fresh subagent implements each task and a fresh reviewer checks it before the next one starts, then a whole-branch review at the end. Most thorough; costs a fresh context per task and per review.
- **Native** - I implement every task myself in this session, the way this harness runs work, then one fresh reviewer on the most capable model checks the whole branch. Cheapest and fastest; no independent review until the end. Runs well with a mid-tier session model, since the plan carries the design.

**For this plan I recommend Native, because the tasks define a lot of tightly coupled interfaces across one new Rust codebase and extension/back-end seams, so staying in one session reduces interface drift while the plan still gives us a structured reviewable path. Does the plan capture what you want, and which approach should we use?**
