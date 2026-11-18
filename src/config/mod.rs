pub mod repos;
pub mod validation;

use repos::CompatibilityReposConfig;
use std::path::{Path, PathBuf};

/// Shortcuts used for manual mode recovery when Wayland focus detection is unavailable or wrong.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModeShortcutsConfig {
    pub cycle_mode: Option<String>,
    pub code_mode: Option<String>,
    pub browser_mode: Option<String>,
    pub terminal_mode: Option<String>,
}

/// Application configuration.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AppConfig {
    pub repos: Option<CompatibilityReposConfig>,
    pub shortcuts: ModeShortcutsConfig,
    pub audio_device: Option<String>,
}

impl AppConfig {
    pub fn for_test_missing_rango_repo() -> Self {
        Self {
            repos: Some(CompatibilityReposConfig {
                community: PathBuf::from("repos/community"),
                cursorless_talon: PathBuf::from("repos/cursorless-talon"),
                rango_talon: PathBuf::new(),
            }),
            shortcuts: Self::default_test_shortcuts(),
            audio_device: None,
        }
    }

    pub fn for_test_with_mode_shortcuts() -> Self {
        Self {
            repos: Some(CompatibilityReposConfig {
                community: PathBuf::from("repos/community"),
                cursorless_talon: PathBuf::from("repos/cursorless-talon"),
                rango_talon: PathBuf::from("repos/rango-talon"),
            }),
            shortcuts: Self::default_test_shortcuts(),
            audio_device: None,
        }
    }

    fn default_test_shortcuts() -> ModeShortcutsConfig {
        ModeShortcutsConfig {
            cycle_mode: Some("Super+Alt+M".into()),
            code_mode: Some("Super+Alt+1".into()),
            browser_mode: Some("Super+Alt+2".into()),
            terminal_mode: Some("Super+Alt+3".into()),
        }
    }
}

fn resolve_config_dir(custom_dir: Option<&Path>) -> PathBuf {
    match custom_dir {
        Some(dir) => dir.to_path_buf(),
        None => std::env::var("VOICE_RUNTIME_CONFIG_DIR")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var("XDG_CONFIG_HOME")
                    .ok()
                    .filter(|s| !s.trim().is_empty())
                    .map(|s| PathBuf::from(s).join("voice-runtime"))
            })
            .or_else(|| {
                std::env::var("HOME")
                    .ok()
                    .filter(|s| !s.trim().is_empty())
                    .map(|s| PathBuf::from(s).join(".config/voice-runtime"))
            })
            .unwrap_or_else(|| PathBuf::from("/tmp/voice-runtime")),
    }
}

/// Returns the file path used to persist the user's active audio input device selection.
pub fn selected_audio_device_file(custom_dir: Option<&Path>) -> PathBuf {
    resolve_config_dir(custom_dir).join("selected_audio_device")
}

/// Persists the selected audio input device name (or removes any override when `None`/empty).
pub fn save_selected_audio_device(
    device: Option<&str>,
    custom_dir: Option<&Path>,
) -> std::io::Result<PathBuf> {
    let path = selected_audio_device_file(custom_dir);
    match device.map(str::trim).filter(|s| !s.is_empty()) {
        Some(name) => {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&path, format!("{name}\n"))?;
        }
        None => {
            if path.exists() {
                std::fs::remove_file(&path)?;
            }
        }
    }
    Ok(path)
}

/// Loads the previously persisted audio input device selection, if configured.
pub fn load_selected_audio_device(custom_dir: Option<&Path>) -> Option<String> {
    let path = selected_audio_device_file(custom_dir);
    let content = std::fs::read_to_string(path).ok()?;
    let trimmed = content.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Returns the file path used to persist selected Talon compatibility repository folders.
pub fn selected_repositories_file(custom_dir: Option<&Path>) -> PathBuf {
    resolve_config_dir(custom_dir).join("selected_repositories")
}

/// Persists selected Talon compatibility repository paths (or clears the file when empty).
pub fn save_selected_repositories(
    repos: &[PathBuf],
    custom_dir: Option<&Path>,
) -> std::io::Result<PathBuf> {
    let path = selected_repositories_file(custom_dir);
    let non_empty: Vec<String> = repos
        .iter()
        .filter(|p| !p.as_os_str().is_empty())
        .map(|p| p.display().to_string())
        .collect();
    if non_empty.is_empty() {
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
    } else {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, format!("{}\n", non_empty.join("\n")))?;
    }
    Ok(path)
}

/// Loads previously persisted Talon compatibility repository folders.
pub fn load_selected_repositories(custom_dir: Option<&Path>) -> Vec<PathBuf> {
    let path = selected_repositories_file(custom_dir);
    let Ok(content) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    content
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .collect()
}

/// Returns the file path used to persist the selected speech model path.
pub fn selected_model_file(custom_dir: Option<&Path>) -> PathBuf {
    resolve_config_dir(custom_dir).join("selected_model")
}

/// Persists the selected speech model path (or clears it when `None`/empty).
pub fn save_selected_model(
    model: Option<&Path>,
    custom_dir: Option<&Path>,
) -> std::io::Result<PathBuf> {
    let path = selected_model_file(custom_dir);
    match model.filter(|p| !p.as_os_str().is_empty()) {
        Some(model_path) => {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&path, format!("{}\n", model_path.display()))?;
        }
        None => {
            if path.exists() {
                std::fs::remove_file(&path)?;
            }
        }
    }
    Ok(path)
}

/// Loads the previously persisted speech model path, if configured.
pub fn load_selected_model(custom_dir: Option<&Path>) -> Option<PathBuf> {
    let path = selected_model_file(custom_dir);
    let content = std::fs::read_to_string(path).ok()?;
    let trimmed = content.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(PathBuf::from(trimmed))
    }
}
