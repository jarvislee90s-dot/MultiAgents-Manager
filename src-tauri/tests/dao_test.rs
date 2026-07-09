mod support;

use multi_agents_manager_lib::database;

#[test]
fn test_settings_get_set() {
    support::setup();
    assert!(database::get_setting("nonexistent").is_none());
    database::set_setting("test_key", "test_value");
    assert_eq!(database::get_setting("test_key"), Some("test_value".to_string()));
    database::set_setting("test_key", "updated");
    assert_eq!(database::get_setting("test_key"), Some("updated".to_string()));
}

#[test]
fn test_extension_crud() {
    support::setup();
    let ext = support::create_test_extension();
    database::insert_extension(&ext).unwrap();
    let list = database::list_extensions();
    assert!(list.iter().any(|e| e.id == "test-skill-1"));
    let assignments = database::list_assignments("claude");
    assert!(assignments.is_empty());
    database::upsert_assignment("test-skill-1", "claude", true, "linked").unwrap();
    let assignments = database::list_assignments("claude");
    assert_eq!(assignments.len(), 1);
    assert!(assignments[0].enabled);
}

#[test]
fn test_preset_crud() {
    support::setup();
    let preset_id = database::create_preset("测试组", &[
        ("skill-a".to_string(), "skill".to_string()),
        ("mcp-b".to_string(), "mcp".to_string()),
    ]).unwrap();
    assert!(!preset_id.is_empty());
    let presets = database::list_presets();
    assert!(presets.iter().any(|p| p.name == "测试组"));
    let items = database::get_preset_items(&preset_id);
    assert_eq!(items.len(), 2);
    database::delete_preset(&preset_id).unwrap();
    let presets = database::list_presets();
    assert!(!presets.iter().any(|p| p.id == preset_id));
}

#[test]
fn test_session_status() {
    support::setup();
    let prev = database::update_session_status("session-1", "claude", "running");
    assert!(prev.is_none()); // 首次记录
    let prev = database::update_session_status("session-1", "claude", "waiting");
    assert_eq!(prev, Some("running".to_string())); // 状态变化
    // 清理
    let mut active = std::collections::HashSet::new();
    active.insert("session-1".to_string());
    database::cleanup_stale_sessions(&active);
}
