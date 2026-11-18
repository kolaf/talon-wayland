use crate::mode::RuntimeMode;
use std::collections::HashSet;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TalonContextHeader {
    pub app: Option<String>,
    pub app_name: Option<String>,
    pub app_exe: Option<String>,
    pub app_bundle: Option<String>,
    pub title: Option<String>,
    pub hostname: Option<String>,
    pub os: Option<String>,
    pub mode: Option<RuntimeMode>,
    pub talon_modes: Vec<String>,
    pub not_modes: Vec<String>,
    pub tags: Vec<String>,
    pub not_tags: Vec<String>,
    pub code_language: Option<String>,
}

impl TalonContextHeader {
    /// Returns true when the header has no workflow-specific constraints
    /// (`mode: code/browser/terminal/global`, `app*`, `title`, `code_language`, or `tags`).
    pub fn is_global_only(&self) -> bool {
        self.mode.is_none()
            && self.app.is_none()
            && self.app_name.is_none()
            && self.app_exe.is_none()
            && self.app_bundle.is_none()
            && self.title.is_none()
            && self.code_language.is_none()
            && self.tags.is_empty()
            && !self.talon_modes.iter().any(|m| {
                matches!(
                    m.to_ascii_lowercase().as_str(),
                    "global" | "code" | "browser" | "terminal"
                )
            })
    }

    /// Specificity score used to rank matching context overrides (higher wins).
    pub fn specificity_score(&self) -> usize {
        let mut score = 0;
        if self.app.is_some() {
            score += 10;
        }
        if self.app_name.is_some() {
            score += 10;
        }
        if self.app_exe.is_some() {
            score += 10;
        }
        if self.app_bundle.is_some() {
            score += 10;
        }
        score += self.tags.len() * 5;
        if self.code_language.is_some() {
            score += 4;
        }
        if self.title.is_some() {
            score += 3;
        }
        if self.mode.is_some() || !self.talon_modes.is_empty() {
            score += 2;
        }
        if self.os.is_some() || self.hostname.is_some() {
            score += 1;
        }
        score
    }

    /// Evaluates whether the common platform/mode/tag constraints match the given
    /// `RuntimeMode`, active tag set, and optional active code language.
    pub fn matches_context(
        &self,
        mode: RuntimeMode,
        active_tags: &HashSet<String>,
        active_language: Option<&str>,
    ) -> bool {
        if let Some(os_spec) = &self.os {
            if !matches_disjunction_ci(os_spec, &["linux"]) {
                return false;
            }
        }

        if let Some(explicit_mode) = self.mode {
            if explicit_mode != mode {
                return false;
            }
        }

        if self
            .not_modes
            .iter()
            .any(|m| m.eq_ignore_ascii_case("command") || m.eq_ignore_ascii_case(&mode.to_string()))
        {
            return false;
        }

        for m in &self.talon_modes {
            if !matches_talon_mode(m, mode) {
                return false;
            }
        }

        if let Some(lang_spec) = &self.code_language {
            let Some(lang) = active_language else {
                return false;
            };
            if !matches_disjunction_ci(lang_spec, &[lang]) {
                return false;
            }
        }

        for tag in &self.tags {
            if !active_tags.contains(tag) {
                return false;
            }
        }

        for tag in &self.not_tags {
            if active_tags.contains(tag) {
                return false;
            }
        }

        if !matches_mode_app_profile(self, mode) {
            return false;
        }

        true
    }

    /// Evaluates whether a command or file belongs to `mode` under `CommandInventory::commands_for_mode`.
    pub fn matches_workflow_mode(
        &self,
        mode: RuntimeMode,
        active_tags: &HashSet<String>,
        active_language: Option<&str>,
    ) -> bool {
        if !self.matches_context(mode, active_tags, active_language) {
            return false;
        }

        if let Some(explicit_mode) = self.mode {
            return explicit_mode == mode;
        }

        if self.is_global_only() {
            return mode == RuntimeMode::Global;
        }

        true
    }
}

/// Returns the built-in workflow tags implicitly active for a `RuntimeMode`.
pub fn base_tags_for_mode(mode: RuntimeMode) -> HashSet<String> {
    let mut tags = HashSet::new();
    match mode {
        RuntimeMode::Global => {}
        RuntimeMode::Code => {
            tags.insert("user.cursorless".to_string());
        }
        RuntimeMode::Browser => {
            tags.insert("browser".to_string());
            tags.insert("user.rango".to_string());
        }
        RuntimeMode::Terminal => {
            tags.insert("terminal".to_string());
        }
    }
    tags
}

fn matches_talon_mode(spec: &str, mode: RuntimeMode) -> bool {
    let lower = spec.trim().to_ascii_lowercase();
    match lower.as_str() {
        "command" | "all" => true,
        "global" => mode == RuntimeMode::Global,
        "code" => mode == RuntimeMode::Code,
        "browser" => mode == RuntimeMode::Browser,
        "terminal" => mode == RuntimeMode::Terminal,
        _ => false,
    }
}

fn matches_mode_app_profile(ctx: &TalonContextHeader, mode: RuntimeMode) -> bool {
    let has_app_constraint = ctx.app.is_some()
        || ctx.app_name.is_some()
        || ctx.app_exe.is_some()
        || ctx.app_bundle.is_some()
        || ctx.title.is_some();

    if !has_app_constraint {
        return true;
    }

    let (apps, app_names, app_exes, app_bundles, titles): (
        &[&str],
        &[&str],
        &[&str],
        &[&str],
        &[&str],
    ) = match mode {
        RuntimeMode::Global => return false,
        RuntimeMode::Code => (
            &["vscode", "code", "codium", "vscodium"],
            &["Visual Studio Code", "Code", "VSCodium", "Code - OSS"],
            &["code", "code.exe", "codium", "vscodium"],
            &["com.microsoft.VSCode", "com.vscodium"],
            &["Visual Studio Code", "VSCodium"],
        ),
        RuntimeMode::Browser => (
            &["vivaldi", "vivaldi-stable", "vivaldi-snapshot"],
            &["Vivaldi", "Vivaldi-stable"],
            &["vivaldi", "vivaldi-bin"],
            &["com.vivaldi.Vivaldi"],
            &["Vivaldi"],
        ),
        RuntimeMode::Terminal => (
            &[
                "gnome-terminal",
                "org.gnome.terminal",
                "kgx",
                "org.gnome.console",
                "konsole",
                "org.kde.konsole",
                "alacritty",
                "kitty",
                "wezterm",
                "org.wezfurlong.wezterm",
                "foot",
                "ghostty",
                "com.mitchellh.ghostty",
                "xterm",
                "terminal",
            ],
            &[
                "Terminal",
                "GNOME Terminal",
                "Console",
                "Konsole",
                "Alacritty",
                "kitty",
                "WezTerm",
                "Foot",
                "Ghostty",
                "XTerm",
            ],
            &[
                "gnome-terminal",
                "gnome-terminal-server",
                "kgx",
                "konsole",
                "alacritty",
                "kitty",
                "wezterm",
                "foot",
                "ghostty",
                "xterm",
            ],
            &[
                "org.gnome.Terminal",
                "org.gnome.Console",
                "com.mitchellh.ghostty",
                "org.wezfurlong.wezterm",
            ],
            &["Terminal", "Console"],
        ),
    };

    if let Some(pat) = &ctx.app {
        if !matches_disjunction_ci(pat, apps) {
            return false;
        }
    }
    if let Some(pat) = &ctx.app_name {
        if !matches_disjunction_ci(pat, app_names) {
            return false;
        }
    }
    if let Some(pat) = &ctx.app_exe {
        if !matches_disjunction_ci(pat, app_exes) {
            return false;
        }
    }
    if let Some(pat) = &ctx.app_bundle {
        if !matches_disjunction_ci(pat, app_bundles) {
            return false;
        }
    }
    if let Some(pat) = &ctx.title {
        if !matches_title_ci(pat, titles) {
            return false;
        }
    }

    true
}

fn matches_disjunction_ci(pattern: &str, candidates: &[&str]) -> bool {
    pattern
        .split('|')
        .map(|part| part.trim())
        .filter(|part| !part.is_empty())
        .any(|part| matches_single_pattern(part, candidates, false))
}

fn matches_title_ci(pattern: &str, candidates: &[&str]) -> bool {
    pattern
        .split('|')
        .map(|part| part.trim())
        .filter(|part| !part.is_empty())
        .any(|part| matches_single_pattern(part, candidates, true))
}

fn matches_single_pattern(pattern: &str, candidates: &[&str], substring_literal: bool) -> bool {
    if let Some(regex_body) = strip_regex_delimiters(pattern) {
        let (raw_pat, case_insensitive) = regex_body;
        let anchored_start = raw_pat.starts_with('^');
        let anchored_end = raw_pat.ends_with('$');
        let core = raw_pat
            .trim_start_matches('^')
            .trim_end_matches('$')
            .replace(".*", "");
        return candidates.iter().any(|cand| {
            let (hay, needle) = if case_insensitive {
                (cand.to_ascii_lowercase(), core.to_ascii_lowercase())
            } else {
                ((*cand).to_string(), core.clone())
            };
            match (anchored_start, anchored_end) {
                (true, true) => hay == needle,
                (true, false) => hay.starts_with(&needle),
                (false, true) => hay.ends_with(&needle),
                (false, false) => hay.contains(&needle),
            }
        });
    }

    candidates.iter().any(|cand| {
        if substring_literal {
            cand.to_ascii_lowercase()
                .contains(&pattern.to_ascii_lowercase())
        } else {
            cand.eq_ignore_ascii_case(pattern)
        }
    })
}

fn strip_regex_delimiters(s: &str) -> Option<(&str, bool)> {
    if let Some(rest) = s.strip_prefix('/') {
        if let Some(core) = rest.strip_suffix("/i") {
            return Some((core, true));
        }
        if let Some(core) = rest.strip_suffix('/') {
            return Some((core, false));
        }
    }
    None
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpokenToken {
    Word(String),
    ListRef(String),
    CaptureRef(String),
    Optional(Vec<SpokenToken>),
    Alternation(Vec<Vec<SpokenToken>>),
    Repeat {
        token: Box<SpokenToken>,
        one_or_more: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SpokenRule {
    pub anchored_start: bool,
    pub anchored_end: bool,
    pub tokens: Vec<SpokenToken>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TalonFile {
    pub context: TalonContextHeader,
    pub commands: Vec<TalonCommand>,
    pub activated_tags: Vec<String>,
    pub settings: Vec<(String, String)>,
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TalonCommand {
    pub spoken_form: String,
    pub rule: SpokenRule,
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
    /// Local variable assignment like `x = user.foo(arg)`.
    Assign { var: String, expr: String },
    /// Setting entry inside a `settings():` block.
    Setting { name: String, value: String },
}
