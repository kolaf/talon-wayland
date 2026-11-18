use voice_runtime::runtime::actions::{register_builtin_actions, ActionRegistry};

#[test]
fn resolves_builtin_action_by_name() {
    let mut registry = ActionRegistry::default();
    register_builtin_actions(&mut registry);
    assert!(registry.resolve("insert").is_ok());
}
