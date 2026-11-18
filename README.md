# voice-runtime

Wayland-native, command-oriented voice runtime.

## Scope (v1)

- Focus: command recognition + execution for three workflows:
  - VS Code / Cursorless
  - Browser / Rango
  - Terminal
- Live microphone audio capture (`cpal`) + energy/VAD utterance segmentation + local `transcribe-cpp` recognition biased by the active mode's command inventory.
- Manual mode switching (cycle + direct shortcuts + voice mode commands) and automatic mode selection when possible.
- Compatibility subset for `talonhub/community`, `cursorless-talon`, and `rango-talon`, including an embedded Python 3 (`pyo3`) host with a synthetic `talon` module.
- Not in scope: dictation (Handy remains the dictation tool), full Talon parity, arbitrary GUI automation.

## Development & Testing

Run the full automated test suite:

```bash
cargo test
```

Launch the GTK3 graphical control window (includes folder pickers for `community`, `cursorless-talon`, and `rango-talon`, active microphone dropdown, listening ON/OFF toggle, active mode, and last recognized command; selections persist under `~/.config/voice-runtime/`):

```bash
cargo run -- gui --model /path/to/model.gguf
```

Bind a Wayland global shortcut (e.g., GNOME Settings -> Keyboard -> Custom Shortcuts) to toggle listening or switch modes against the running GUI or `listen` session:

```bash
voice-runtime toggle       # toggle listening ON / OFF
voice-runtime mode cycle   # cycle mode (global -> code -> browser -> terminal)
voice-runtime status       # inspect current listening state, mode, and microphone
```

List available audio input devices and select the active microphone from the CLI:

```bash
cargo run -- devices
cargo run -- select-device            # interactive prompt
cargo run -- select-device 1          # select by 1-based index
cargo run -- select-device "USB Mic"  # select by name or substring
cargo run -- select-device --clear    # revert to system default
```

Listen from the active (or specified) microphone (or test against a `.wav` recording) using a local `.gguf` speech model (auto-discovered from `VOICE_RUNTIME_MODEL` or Handy/voice-runtime model directories, or passed via `--model`):

```bash
cargo run -- listen --model /path/to/model.gguf --mode browser
cargo run -- listen --model /path/to/model.gguf --select-device --mode code
cargo run -- listen --model /path/to/model.gguf --mode code --repo /path/to/community --repo /path/to/cursorless-talon
cargo run -- listen --model /path/to/model.gguf --mode browser --wav /path/to/command.wav
```

Simulate phrase matching and backend dispatch without audio hardware:

```bash
cargo run -- simulate --mode browser "open hint air"
cargo run -- simulate --mode code "take line"
cargo run -- simulate --mode terminal "status check"
```

Validate loading a local Talon repository (`community`, `cursorless-talon`, or `rango-talon`):

```bash
cargo run -- check-repo /path/to/talon/repo
```

See [GNOME Wayland Setup](docs/gnome-wayland-setup.md) for configuration and operator requirements.
