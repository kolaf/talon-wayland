# GNOME Wayland Setup & Operator Guide

This guide covers configuring and validating `voice-runtime` on GNOME Wayland.

## 1. Compatibility Repositories

`voice-runtime` v1 expects three supported compatibility repositories configured in `AppConfig::repos` (`CompatibilityReposConfig`):

- `community` (`talonhub/community`)
- `cursorless-talon` (`cursorless-dev/cursorless-talon`)
- `rango-talon` (`david-tejada/rango-talon`)

You can pick these folders directly from the **Talon Repositories** section in `voice-runtime gui` (persisted to `~/.config/voice-runtime/selected_repositories`) or inspect any local clone from the terminal before running commands:

```bash
cargo run -- check-repo /path/to/community /path/to/cursorless-talon /path/to/rango-talon
```

## 2. Mode Switching & Recovery Shortcuts (GNOME Wayland)

Automatic foreground detection on Wayland uses a pluggable `ForegroundDetector` (`src/frontend/focus.rs`). Because Wayland window metadata can be delayed or restricted depending on GNOME Shell extensions/permissions, `voice-runtime` exposes local Unix socket IPC commands you can bind directly in **GNOME Settings -> Keyboard -> View and Customize Shortcuts -> Custom Shortcuts**:

- **Toggle listening ON/OFF**: `/path/to/voice-runtime toggle` (or `cargo run --manifest-path /home/kolaf/dev/talon-wayland/Cargo.toml -- toggle`)
- **Cycle mode**: `/path/to/voice-runtime mode cycle`
- **Direct mode selection**: `voice-runtime mode code`, `voice-runtime mode browser`, `voice-runtime mode terminal`
- **Query status**: `voice-runtime status`

Both `voice-runtime gui` and `voice-runtime listen` listen on `$XDG_RUNTIME_DIR/voice-runtime.sock` (overridable via `VOICE_RUNTIME_SOCKET`). Manual overrides always beat foreground auto-switching until cleared (`voice-runtime mode auto`).

## 3. Backends

- **VS Code / Cursorless (`VsCodeBackend`)**: Routes `user.cursorless_*` and `user.vscode*` actions over the VS Code command-server seam (`cursorless.*` semantic commands).
- **Browser / Rango (`BrowserBackend`)**: Routes `user.rango_*` actions to the Rango browser extension messaging channel (`rango.*` commands for Vivaldi).
- **Terminal (`TerminalBackend`)**: Dispatches shell commands via text insertion/paste (`insert`, `edit.paste`) and restricts `key(...)` dispatch to an explicit allowlist of safe terminal navigation/control keys.

Disconnecting or missing extension bridges fail explicitly with `ExecutionError::BackendUnavailable` instead of silently dropping commands.

## 4. Live Audio Input & Speech Model

Live microphone input uses `cpal` (resampling incoming audio to 16 kHz mono and segmenting commands by energy/VAD) and `transcribe-cpp` for local speech recognition biased toward the active mode's phrase inventory.

- List microphones (with index, sample rate, default/active status): `cargo run -- devices`
- Select and persist the active microphone: `cargo run -- select-device` (interactive), `cargo run -- select-device <index-or-name>`, or `cargo run -- select-device --clear`
- Start live listening (uses saved selection unless `--device` or `--select-device` is given): `cargo run -- listen --model /path/to/model.gguf`
- If `--model` is omitted, `voice-runtime` checks `VOICE_RUNTIME_MODEL` and standard local model folders under `$XDG_DATA_HOME` / `~/.local/share` (`com.pais.handy`, `handy`, `voice-runtime`) and `./models`.
- Spoken mode-switch commands (`code mode`, `browser mode`, `terminal mode`, `global mode`, `cycle mode`, `auto mode`) work directly during live listening.

## 5. Python 3.12 Runtime Requirement

Embedded Python support uses `pyo3` linked against system Python 3 (`python3-dev` / `libpython3.12`) with an in-process synthetic `talon` compatibility module (`Module`, `Context`, `actions`, `settings`, `app`, `clip`, `ui`, `cron`, `imgui`, `registry`, `scope`, `speech_system`).
