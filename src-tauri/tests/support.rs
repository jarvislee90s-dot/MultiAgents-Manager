use std::sync::Once;

static INIT: Once = Once::new();

/// 初始化测试环境：设置 HOME 到临时目录，初始化全局数据库
/// TempDir 通过 Box::leak 保持存活，避免被清理
pub fn setup() {
    INIT.call_once(|| {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().to_path_buf();
        std::env::set_var("HOME", &home);
        // 创建 ~/.mam 目录结构
        std::fs::create_dir_all(home.join(".mam/skills")).unwrap();
        std::fs::create_dir_all(home.join(".mam/mcp")).unwrap();
        std::fs::create_dir_all(home.join(".mam/plugins")).unwrap();
        std::fs::create_dir_all(home.join(".mam/active")).unwrap();
        // 初始化数据库（Lazy 只初始化一次）
        multi_agents_manager_lib::database::init();
        // 泄漏 TempDir 防止它被清理（测试期间需要保持数据库文件存在）
        std::mem::forget(temp);
    });
}

/// 创建测试用 ExtensionRecord
pub fn create_test_extension() -> multi_agents_manager_lib::database::ExtensionRecord {
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
