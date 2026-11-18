use crate::config::AppConfig;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError(pub String);

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ConfigError {}

/// Validates startup configuration: all three supported repository paths must be set,
/// and both cycle + direct manual mode shortcuts must be configured for fallback recovery.
pub fn validate_config(config: &AppConfig) -> Result<(), ConfigError> {
    let Some(repos) = &config.repos else {
        return Err(ConfigError(
            "compatibility repositories configuration missing (community, cursorless-talon, rango-talon)"
                .into(),
        ));
    };

    if repos.community.as_os_str().is_empty() {
        return Err(ConfigError("missing repository path for `community`".into()));
    }
    if repos.cursorless_talon.as_os_str().is_empty() {
        return Err(ConfigError(
            "missing repository path for `cursorless-talon`".into(),
        ));
    }
    if repos.rango_talon.as_os_str().is_empty() {
        return Err(ConfigError(
            "missing repository path for `rango-talon`".into(),
        ));
    }

    let check_shortcut = |opt: &Option<String>, label: &str| -> Result<(), ConfigError> {
        match opt {
            Some(s) if !s.trim().is_empty() => Ok(()),
            _ => Err(ConfigError(format!(
                "missing manual mode recovery shortcut: `{label}`"
            ))),
        }
    };

    check_shortcut(&config.shortcuts.cycle_mode, "cycle_mode")?;
    check_shortcut(&config.shortcuts.code_mode, "code_mode")?;
    check_shortcut(&config.shortcuts.browser_mode, "browser_mode")?;
    check_shortcut(&config.shortcuts.terminal_mode, "terminal_mode")?;

    Ok(())
}
