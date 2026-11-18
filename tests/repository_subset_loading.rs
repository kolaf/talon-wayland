use std::path::PathBuf;
use voice_runtime::app::RuntimeApp;
use voice_runtime::config::repos::{load_supported_repositories, CompatibilityReposConfig};
use voice_runtime::config::{load_selected_repositories, save_selected_repositories};
use voice_runtime::mode::RuntimeMode;

fn test_repo_config() -> CompatibilityReposConfig {
    CompatibilityReposConfig {
        community: PathBuf::from("fixtures/community"),
        cursorless_talon: PathBuf::from("fixtures/cursorless-talon"),
        rango_talon: PathBuf::from("fixtures/rango-talon"),
    }
}

#[test]
fn loads_only_supported_repositories() {
    let loaded = load_supported_repositories(&test_repo_config()).unwrap();
    assert!(loaded.repo_names().contains(&"community".to_string()));
    assert!(loaded
        .repo_names()
        .contains(&"cursorless-talon".to_string()));
    assert!(loaded.repo_names().contains(&"rango-talon".to_string()));
}

#[test]
fn loads_real_talon_and_python_files_from_supported_repo_dirs() {
    let base = std::env::temp_dir().join(format!(
        "voice_runtime_repo_load_{}",
        std::process::id()
    ));
    let comm = base.join("community");
    let curs = base.join("cursorless-talon");
    let rang = base.join("rango-talon");
    std::fs::create_dir_all(&comm).unwrap();
    std::fs::create_dir_all(&curs).unwrap();
    std::fs::create_dir_all(&rang).unwrap();

    std::fs::write(
        comm.join("core.py"),
        "from talon import Module, Context\nmod = Module()\nmod.list('letter')\nctx = Context()\nctx.lists['user.letter'] = {'air': 'a'}\n",
    )
    .unwrap();
    std::fs::write(
        curs.join("cursorless.talon"),
        "app: vscode\n-\ntake line: user.cursorless_take_line()\n",
    )
    .unwrap();
    std::fs::write(
        rang.join("rango.talon"),
        "tag: browser\n-\nopen hint {user.letter}: user.rango_open_hint(letter)\n",
    )
    .unwrap();

    let cfg = CompatibilityReposConfig {
        community: comm,
        cursorless_talon: curs,
        rango_talon: rang,
    };
    let loaded = load_supported_repositories(&cfg).unwrap();
    assert_eq!(loaded.inventory.commands_for_mode(RuntimeMode::Code).len(), 1);
    assert_eq!(
        loaded.inventory.commands_for_mode(RuntimeMode::Browser).len(),
        1
    );
    assert_eq!(
        loaded.python_host.list_values("user.letter").unwrap(),
        &["air".to_string()]
    );

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn repository_selection_persists_and_clears_in_config_dir() {
    let dir = std::env::temp_dir().join(format!(
        "voice_runtime_repo_cfg_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);

    assert!(load_selected_repositories(Some(&dir)).is_empty());

    let repos = vec![
        PathBuf::from("/tmp/community"),
        PathBuf::from("/tmp/cursorless-talon"),
        PathBuf::from("/tmp/rango-talon"),
    ];
    save_selected_repositories(&repos, Some(&dir)).unwrap();
    assert_eq!(load_selected_repositories(Some(&dir)), repos);

    save_selected_repositories(&[], Some(&dir)).unwrap();
    assert!(load_selected_repositories(Some(&dir)).is_empty());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn runtime_app_reloads_repositories_and_dispatches_new_commands() {
    let base = std::env::temp_dir().join(format!(
        "voice_runtime_app_reload_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();

    std::fs::write(
        base.join("rango.talon"),
        "tag: browser\n-\nhop next: user.rango_open_hint(\"n\")\n",
    )
    .unwrap();

    let mut app = RuntimeApp::with_default_backends(RuntimeMode::Browser, &[]).unwrap();
    let summary = app.reload_repositories(std::slice::from_ref(&base)).unwrap();

    assert_eq!(summary.repo_count, 1);
    assert_eq!(summary.files_loaded, 1);
    assert_eq!(summary.total_commands, 1);
    assert_eq!(app.configured_repos(), &[base.clone()]);

    app.handle_recognized_phrase("hop next").unwrap();
    assert_eq!(app.last_backend_invocation_for_test(), "browser");

    let _ = std::fs::remove_dir_all(&base);
}
