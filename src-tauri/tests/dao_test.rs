mod support;

use multi_agents_manager_lib::database;

/// 创建测试用 ExtensionRecord
fn create_test_extension() -> multi_agents_manager_lib::database::ExtensionRecord {
    multi_agents_manager_lib::database::ExtensionRecord {
        id: "test-skill-1".to_string(),
        kind: "skill".to_string(),
        name: "Test Skill".to_string(),
        description: Some("测试用 skill".to_string()),
        source_path: "/tmp/test-skill".to_string(),
        source_url: None,
        version: None,
        tags: None,
        suite: None,
        source_tool: None,
        is_native: false,
    }
}

#[test]
fn test_settings_get_set() {
    support::setup();
    assert!(database::get_setting("nonexistent").is_none());
    database::set_setting("test_key", "test_value");
    assert_eq!(
        database::get_setting("test_key"),
        Some("test_value".to_string())
    );
    database::set_setting("test_key", "updated");
    assert_eq!(
        database::get_setting("test_key"),
        Some("updated".to_string())
    );
}

#[test]
fn test_extension_crud() {
    support::setup();
    let ext = create_test_extension();
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
    let preset_id = database::create_preset(
        "测试组",
        &[
            ("skill-a".to_string(), "skill".to_string()),
            ("mcp-b".to_string(), "mcp".to_string()),
        ],
    )
    .unwrap();
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

#[test]
fn test_preset_v2_crud_roundtrip() {
    support::setup();

    let items = vec![
        ("skill-v2m1-a".to_string(), "skill".to_string()),
        ("mcp-v2m1-b".to_string(), "mcp".to_string()),
    ];
    // 带元信息创建：工具私有预设
    let id = database::create_preset_with_meta(
        "v2m1-设计套件",
        "做设计时的备忘",
        "tool",
        Some("codex"),
        &items,
    )
    .unwrap();
    let p = database::get_preset(&id).expect("get_preset 应返回");
    assert_eq!(p.name, "v2m1-设计套件");
    assert_eq!(p.description, "做设计时的备忘");
    assert_eq!(p.scope, "tool");
    assert_eq!(p.bound_tool.as_deref(), Some("codex"));
    assert_eq!(p.items.len(), 2);

    // 更新：改类型为 universal + 换 items
    database::update_preset(
        &id,
        "v2m1-改名",
        "新描述",
        "universal",
        None,
        &[("skill-v2m1-c".to_string(), "skill".to_string())],
    )
    .unwrap();
    let p2 = database::get_preset(&id).unwrap();
    assert_eq!(p2.name, "v2m1-改名");
    assert_eq!(p2.scope, "universal");
    assert!(p2.bound_tool.is_none());
    assert_eq!(p2.items.len(), 1);
    assert_eq!(p2.items[0].extension_id, "skill-v2m1-c");

    // 旧入口仍可用，默认 universal
    let legacy_id = database::create_preset("v2m1-旧入口", &items).unwrap();
    assert_eq!(database::get_preset(&legacy_id).unwrap().scope, "universal");

    // list_presets 返回扩展字段
    let listed = database::list_presets();
    assert!(listed.iter().any(|p| p.id == id && p.scope == "universal"));

    database::delete_preset(&id).unwrap();
    assert!(database::get_preset(&id).is_none());
}
