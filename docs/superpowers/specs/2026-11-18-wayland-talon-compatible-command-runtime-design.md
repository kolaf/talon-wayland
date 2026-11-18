# Wayland-Native Talon-Compatible Command Runtime Design

## Intent

The user needs a Wayland-native voice command tool that preserves the useful parts of the Talon ecosystem without depending on Talon itself on Linux. Handy already covers dictation into arbitrary applications. The missing piece is command-oriented voice control for three primary foreground workflows:

- VS Code development, including Cursorless-style structural editing
- browser navigation in Vivaldi using Rango-style commands
- terminal workflows

The new tool should be one common application, prefer automatic mode switching based on the foreground application, and fall back to explicit manual mode switching when foreground detection is unavailable or unreliable on Wayland.

The first version should optimize for practical reuse of existing Talon assets rather than full Talon parity. The compatibility source of truth is limited to:

- `talonhub/community`
- `cursorless-talon`
- `rango-talon`

## Goals

- Build a separate Wayland-native command application rather than forcing Talon to work under Wayland.
- Reuse as much of the existing Talon spoken-command model as practical for the three target workflows.
- Support a Talon-compatible subset of `.talon` files and Python actions sufficient for:
  - VS Code / Cursorless
  - browser / Rango
  - terminal commands
- Use a command-oriented recognizer rather than freeform dictation-first STT.
- Keep one shared command runtime with app-sensitive modes, not separate per-app voice tools.
- Support both:
  - automatic mode selection from the foreground application when available
  - manual mode switching via shortcut and explicit command when not
- Keep the architecture open to sharing lower-level audio/model code with Handy later, without making this a Handy feature first.

## Non-goals

- Do not attempt full Talon desktop automation parity in the first version.
- Do not support eye tracking, noise commands, full mouse-grid workflows, or arbitrary global GUI automation.
- Do not replace Handy's dictation role.
- Do not attempt compatibility with every Talon user repository from day one.
- Do not solve every Wayland compositor/backend combination initially; prioritize the user's GNOME Wayland workflow first.
- Do not embed this into Handy in the first version.

## Constraints

- Talon on Linux/Wayland is not a viable base. The user's current Talon build still initializes through X11-specific code paths and crashes in Wayland.
- The new runtime must be useful even if Wayland foreground-window detection is incomplete or flaky.
- The speech engine must be command-oriented. A pure freeform dictation engine such as Whisper without grammar/context constraints is not acceptable for the primary command use case.
- VS Code, browser, and terminal integrations are not symmetric:
  - VS Code has extension-side command integration points
  - Rango is extension-driven inside the browser
  - terminal workflows may need controlled paste / key-dispatch rather than rich semantic APIs
- Existing Talon grammars rely on both `.talon` syntax and Python action/context logic; compatibility must therefore include both declarative grammar and imperative action plumbing.

## Approaches considered

### 1. Talon-compatible subset runtime with app-specific backends

Build one new Wayland-native command app that supports a useful subset of Talon `.talon` and Python action semantics, then routes recognized commands to app backends for VS Code/Cursorless, browser/Rango, and terminal workflows.

This gives the best balance between compatibility, delivery risk, and usefulness. It preserves the Talon mental model where it matters, while avoiding the cost of cloning Talon's full desktop control stack.

### 2. Translator/importer that compiles Talon files into a new internal DSL

This would produce a cleaner internal model, but it would be harder to preserve Talon behavior precisely, especially where the community grammars depend on Python action overrides, tags, and context-sensitive behavior. It increases up-front implementation risk before the first useful workflow exists.

### 3. Broad Talon reimplementation from the start

This offers the strongest compatibility story in theory, but it is too large and risky. It is likely to stall before the user gets working VS Code/browser/terminal support.

### 4. Extend Handy into a command runtime immediately

This reuses existing audio/model infrastructure, but it couples dictation and command control too early and forces Handy to absorb a much more ambitious runtime design before the command side is proven.

**Chosen:** approach 1.

## Design

### Product shape

Create a separate application, here referred to as the **command runtime**, with three layers:

1. **Speech layer**
   - microphone capture
   - command-oriented recognition
   - contextual phrase activation based on active mode

2. **Talon-compatible runtime layer**
   - `.talon` parser/interpreter subset
   - Python action registry / override model subset
   - context, tag, mode, and app matching

3. **Backend layer**
   - VS Code / Cursorless backend
   - browser / Rango backend
   - terminal backend

The runtime should be mode-based and foreground-sensitive, but must always allow the user to override the active mode manually.

### Mode model

The app should expose at least these primary modes:

- `code`
- `browser`
- `terminal`
- `global` (shared commands that are safe across workflows)

Mode selection order:

1. explicit manual override (highest priority)
2. foreground-app detection
3. fallback default mode

Manual control should support both:

- a single cycle shortcut
- dedicated shortcuts for direct selection of code/browser/terminal mode

The active mode should be visibly exposed in the UI so the user can recover from ambiguous focus state.

### Foreground-app detection

Foreground detection should be treated as a pluggable service, not baked into grammar resolution.

Initial strategy:

- use Wayland-compatible foreground metadata sources available on GNOME first
- map known foreground applications to runtime modes:
  - VS Code / VSCodium → `code`
  - Vivaldi / supported browsers with Rango installed → `browser`
  - supported terminal emulators → `terminal`

Fallback:

- manual mode selection when detection is missing, delayed, or wrong

The runtime must not assume that automatic foreground detection is always available.

### Talon compatibility scope

The first version should support the smallest Talon subset that makes the chosen compatibility sources useful.

#### `.talon` subset

Support:

- both headered `.talon` files (with a `-` separator) and **headerless `.talon` files** (no `-` separator; body starts on line 1)
- separation between Talon's `mode:` context header (`command`, `dictation`, `sleep`, `all`) and the runtime's `RuntimeMode` (`Global`, `Code`, `Browser`, `Terminal`), filtering out non-command modes (`dictation`, `sleep`) and non-Linux OS contexts (`os: windows`, `os: mac`)
- community context headers and modifiers:
  - `app`, `app.name`, `app.exe`, `app.bundle`
  - `title` (including `/regex/` patterns)
  - `os`, `mode`, `tag`, `code.language`, `hostname`
  - `not tag`, `not mode`, and `and` header modifiers
- **transitive tag activation**: top-level `tag(): <name>` statements in `.talon` bodies activate tags whenever that file's context header matches, enabling tag-gated files (`tag: user.tabs`, `tag: user.splits`, `tag: browser`, `tag: terminal`, `tag: user.cursorless`, etc.)
- `settings():` blocks parsed into structured file settings rather than failing file load; out-of-scope hardware/noise event triggers (`gamepad(...)`, `parrot(...)`, `noise(...)`, `face(...)`, `deck(...)`) skipped with structured diagnostics instead of aborting repository loading
- spoken rule parsing into a **`SpokenRule` AST** supporting literals, lists (`{user.letter}`), captures (`<user.cursorless_target>`, `<number_small>`), optionals (`[...]`), alternations (`(a | b)`), repetitions (`+`, `*`), and anchors (`^`, `$`)
- community command bodies: quote- and parenthesis-aware argument lexing, single- and double-quoted strings with escapes and `{var}` string interpolation, nested action calls, local variable assignments (`x = user.foo(...)`), and basic arithmetic/concatenation (`number_small - 1`)

Not required initially:

- broad hotkey reimplementation beyond mode switching
- arbitrary desktop-level mouse actions
- hardware/noise triggers (`parrot`, `gamepad`, `face`, `deck`)

#### Python compatibility subset

Use **in-process embedded Python 3 via `pyo3`** hosting a synthetic `talon` Python package (`Module`, `Context`, `actions`, `settings`, `app`, `clip`, `ui`, `cron`, `imgui`, `registry`, `scope`, `speech_system`) so real `.py` files from `talonhub/community`, `cursorless-talon`, and `rango-talon` execute natively in Python while delegating primitive actions back to Rust:

- `Module` (`list`, `tag`, `setting`, `mode`, `@mod.capture`, `@mod.action_class`, `@mod.action`)
- `Context` (`ctx.matches`, `ctx.lists[...]`, `ctx.tags`, `ctx.settings`, `@ctx.action_class`, `@ctx.capture`)
- CSV-loaded list population and Python capture transformation functions (including Cursorless and Rango target builders)
- context-matched action overrides (`@ctx.action_class`) evaluated against the active `RuntimeMode` and tags
- a bridged `actions.*` call surface routing primitive actions (`key`, `insert`, `sleep`, `edit.*`, and backend bridges) back into the Rust runtime

Out of scope initially:

- full GUI canvas rendering (`imgui` / Skia drawing beyond compatibility stubs)
- eye tracker APIs
- advanced desktop/window inspection APIs outside the needs of the three workflows

#### Compatibility source policy

Treat the following as the supported compatibility baseline:

- `talonhub/community` core grammar and action patterns used by code/browser/terminal flows
- `cursorless-talon`
- `rango-talon`

Anything outside those repositories is best-effort only in the first version.

### Speech engine design

The speech layer must be **command-oriented**, not generic dictation-first.

Requirements:

- low-latency local recognition
- phrase/grammar biasing or constrained decoding from the currently active command inventory
- fast mode-specific vocabulary switching
- good short-command accuracy for structured command phrases

Architecture:

- define a speech engine adapter interface
- runtime provides the active phrase inventory / grammar view for the current mode and context
- engine returns normalized command candidates to the Talon-compatible runtime

The exact first model should remain a technical validation item, but the design assumes a **Conformer-style or otherwise command-optimized engine**, not Whisper-style unconstrained dictation.

The runtime should keep speech recognition separate from grammar interpretation so engines can be swapped without rewriting the Talon-compatibility layer.

### VS Code / Cursorless backend

VS Code is the cleanest backend.

Use open integration seams from the Cursorless ecosystem:

- `cursorless`
- `command-server`
- `talon-command-client`

Design direction:

- avoid simulating generic key presses when a semantic command channel exists
- translate supported Talon-style actions into Cursorless/VS Code commands over a local command bridge
- preserve the existing spoken model as much as possible by keeping grammar compatibility in the runtime and only replacing Talon's transport/client role

This backend should be the reference implementation for the runtime.

### Browser / Rango backend

Rango is also open source and already has a Talon-side repository.

Design direction:

- integrate through a browser-extension messaging channel rather than brittle DOM scraping in the host app
- map Talon-style browser commands into the extension's command surface
- keep the runtime responsible for parsing spoken commands, while the extension remains responsible for DOM hinting, navigation targets, and in-browser actions

This should support Vivaldi as the primary browser target if the extension and transport path are compatible.

### Terminal backend

The terminal backend should optimize for practical workflows, not perfect semantic introspection.

Initial capability:

- insert/paste recognized command text into the foreground terminal
- dispatch a small set of control keys safely where Wayland permits it
- preserve existing terminal-oriented Talon grammar where it maps to text insertion or limited key actions

Later capability can add terminal-specific helpers, but the first version should accept that terminal support is less semantic than VS Code and browser integration.

The terminal backend may share mechanisms with Handy's existing Wayland typing/paste strategies if code sharing becomes useful later.

### Relationship to Handy

This should be a separate application first.

However, the design should allow later reuse of:

- audio capture helpers
- model management concepts
- packaging/build patterns
- input/paste helpers where appropriate

The runtime should not depend on Handy internals in the first version, but it should avoid unnecessary divergence where shared utility crates/modules would make sense later.

### State and configuration

Persist:

- active manual mode override (when set)
- backend connection settings
- enabled grammar sets / compatibility repositories
- speech engine selection and model settings
- user overrides for command conflicts and unsupported actions

The runtime should distinguish clearly between:

- imported compatibility grammar
- local user overrides
- backend availability

### Failure and recovery behavior

Because mode ambiguity is expected on Wayland, the runtime must make recovery cheap:

- visible current mode indicator
- quick command/shortcut to set mode explicitly
- clear reporting when a command matched grammatically but the backend for that app is unavailable
- clear reporting when a required Talon action or grammar feature is unsupported

## Risks / review focus

- The chosen Talon subset may be too small for the real command sets and cause hidden incompatibility.
- Foreground-app detection on GNOME Wayland may be too flaky for seamless automatic switching, increasing reliance on manual mode changes.
- Browser/Rango integration may require more extension-side work than expected.
- Terminal command ergonomics may be weaker than expected if Wayland input constraints block needed key behaviors.
- A command-oriented recognizer with good local grammar switching may be harder to source than hoped; the runtime must therefore keep the engine pluggable.
- Pulling too much Handy infrastructure into the first version could blur product boundaries before the command runtime is proven.

## Builder check (self-review)

Questions I checked before handing this over:

- **Is the scope narrow enough to ship something useful?** Yes: only code/browser/terminal workflows are first-class.
- **Does the chosen approach preserve future compatibility growth?** Yes: speech, Talon runtime, and app backends are separated.
- **Is full Talon parity accidentally implied?** No: the document repeatedly frames the first version as a supported subset.
- **Is there a fallback when Wayland foreground detection is weak?** Yes: both cycle and direct manual mode selection are included by design.
- **Are the integration seams real rather than hypothetical?** Yes: Cursorless, its command server, Rango, and their Talon-side repos are open and identifiable.
