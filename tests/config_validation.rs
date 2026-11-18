use voice_runtime::config::validation::validate_config;
use voice_runtime::config::AppConfig;

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

#[test]
fn config_rejects_missing_manual_recovery_shortcuts() {
    let mut config = AppConfig::for_test_with_mode_shortcuts();
    config.shortcuts.code_mode = None;
    let err = validate_config(&config).unwrap_err();
    assert!(err.to_string().contains("code_mode"));
}
