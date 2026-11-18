use crate::mode::RuntimeMode;

/// Minimal information about the current foreground application.
///
/// This is intentionally small and deterministic so it can be provided by
/// platform-specific detectors (GNOME Wayland, X11, etc.) without pulling
/// in GUI dependencies at the mode layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForegroundAppInfo {
    pub app_id: String,
    pub title: String,
}

/// Abstraction over platform-specific ways to detect the foreground app.
///
/// Returning `None` means the platform could not determine the active app.
pub trait ForegroundDetector {
    fn current_app(&self) -> Option<ForegroundAppInfo>;
}

/// Infer a runtime mode from a foreground application.
///
/// This is a best-effort mapping using deterministic tables (app ids and
/// a few title patterns). Unknown apps return `None` to preserve fallback.
pub fn infer_mode(app: &ForegroundAppInfo) -> Option<RuntimeMode> {
    let app_id = app.app_id.to_ascii_lowercase();
    let title = app.title.to_ascii_lowercase();

    // Code editors (Cursorless workflows)
    if matches!(app_id.as_str(), "code" | "vscode" | "codium" | "vscodium") {
        return Some(RuntimeMode::Code);
    }
    if title.contains("visual studio code") || title.contains("vscodium") {
        return Some(RuntimeMode::Code);
    }

    // Browsers (Rango workflows)
    if matches!(
        app_id.as_str(),
        "vivaldi" | "vivaldi-stable" | "vivaldi-snapshot"
    ) {
        return Some(RuntimeMode::Browser);
    }
    if title.contains(" - vivaldi") {
        return Some(RuntimeMode::Browser);
    }

    // Terminals
    // Common terminals across GNOME/KDE and popular alternatives.
    if matches!(
        app_id.as_str(),
        "org.gnome.terminal" |
        "gnome-terminal" |
        "org.gnome.console" |
        "kgx" |
        "org.kde.konsole" |
        "konsole" |
        "xterm" |
        "alacritty" |
        "kitty" |
        "wezterm" |
        "foot" |
        "org.wezfurlong.wezterm" |
        "com.mitchellh.ghostty"
    ) {
        return Some(RuntimeMode::Terminal);
    }
    if title.contains("terminal") || title.contains("console") {
        return Some(RuntimeMode::Terminal);
    }

    None
}
