mod support;

use std::sync::{Mutex, OnceLock};

/// reconcile_test 是独立测试二进制（自己的进程 + 自己的 fake HOME + 全局 SQLite），
/// 二进制内各测试并行跑会共享同一 DB 与磁盘目录互相踩踏 —— 互斥锁强制串行
/// （同 preset_v2_test.rs 的 OnceLock<Mutex<()>> 锁惯例）
static RECONCILE_TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

/// 每个测试第一条语句先拿锁，再 support::setup()
fn acquire_lock() -> std::sync::MutexGuard<'static, ()> {
    RECONCILE_TEST_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|p| p.into_inner())
}

/// L1 缺链：账本 enabled + 磁盘既无链接也无真目录 → kind "L1"
#[test]
fn reconcile_scan_l1_missing_link() {
    let _guard = acquire_lock();
    support::setup();
    use multi_agents_manager_lib::database;
    use multi_agents_manager_lib::services::resource::reconcile::scan_drift;

    let tool = "codex";
    database::set_tool_enabled(tool, true);
    std::fs::create_dir_all(multi_agents_manager_lib::adapter::primary_skill_dir(tool).unwrap())
        .unwrap();
    database::upsert_assignment("skill-v2m2-rc-l1", tool, true, "valid").unwrap();

    let items = scan_drift();
    let hit = items.iter().find(|d| d.extension_id == "skill-v2m2-rc-l1");
    let hit = hit.expect("L1 缺链应被报告（账本 enabled、磁盘无条目）");
    assert_eq!(hit.kind, "L1");
    assert_eq!(hit.tool_id, tool);
    assert!(
        hit.path.contains("v2m2-rc-l1"),
        "path 应指向缺失条目: {}",
        hit.path
    );

    // 清理（尽力而为，镜像 preset_v2_test.rs 纪律）
    let _ = database::delete_assignments_for("skill-v2m2-rc-l1");
}

/// L2 占位：账本 enabled + 磁盘同名真目录 → kind "L2"
#[test]
fn reconcile_scan_l2_placeholder_dir() {
    let _guard = acquire_lock();
    support::setup();
    use multi_agents_manager_lib::database;
    use multi_agents_manager_lib::services::resource::reconcile::scan_drift;

    let tool = "codex";
    database::set_tool_enabled(tool, true);
    let dir = multi_agents_manager_lib::adapter::primary_skill_dir(tool).unwrap();
    std::fs::create_dir_all(dir.join("v2m2-rc-l2")).unwrap();
    database::upsert_assignment("skill-v2m2-rc-l2", tool, true, "valid").unwrap();

    let items = scan_drift();
    let hit = items.iter().find(|d| d.extension_id == "skill-v2m2-rc-l2");
    let hit = hit.expect("L2 占位应被报告（账本 enabled、磁盘同名真目录）");
    assert_eq!(hit.kind, "L2");
    assert_eq!(hit.tool_id, tool);
    assert!(
        hit.path.contains("v2m2-rc-l2"),
        "path 应指向占位目录: {}",
        hit.path
    );

    // 清理
    let _ = std::fs::remove_dir_all(dir.join("v2m2-rc-l2"));
    let _ = database::delete_assignments_for("skill-v2m2-rc-l2");
}

/// L3 多链：磁盘有指向 ~/.mam 的 MAM 链接但账本无 enabled 行 → kind "L3"
#[test]
fn reconcile_scan_l3_unmanaged_link() {
    let _guard = acquire_lock();
    support::setup();
    use multi_agents_manager_lib::services::resource::reconcile::scan_drift;

    let tool = "codex";
    multi_agents_manager_lib::database::set_tool_enabled(tool, true);
    let home = dirs::home_dir().unwrap();
    let tool_dir = multi_agents_manager_lib::adapter::primary_skill_dir(tool).unwrap();
    std::fs::create_dir_all(&tool_dir).unwrap();
    let ssot = home.join(".mam/skills/v2m2-rc-l3");
    std::fs::create_dir_all(&ssot).unwrap();
    let link = tool_dir.join("v2m2-rc-l3");
    std::os::unix::fs::symlink(&ssot, &link).unwrap(); // 无 assignment 行（scan 只读 assignments + 磁盘，无需 extensions 行）

    let items = scan_drift();
    let hit = items.iter().find(|d| d.extension_id == "skill-v2m2-rc-l3");
    let hit = hit.expect("L3 多链应被报告（磁盘 MAM 链接、账本无行）");
    assert_eq!(hit.kind, "L3");
    assert_eq!(hit.tool_id, tool);
    assert!(
        hit.path.contains("v2m2-rc-l3"),
        "path 应指向链接: {}",
        hit.path
    );

    // 清理
    let _ = std::fs::remove_file(&link);
    let _ = std::fs::remove_dir_all(&ssot);
}

/// L4 外链：磁盘链接指向 ~/.mam 之外 → kind "L4"（只报告不接管，账本无关）
#[test]
fn reconcile_scan_l4_external_link() {
    let _guard = acquire_lock();
    support::setup();
    use multi_agents_manager_lib::services::resource::reconcile::scan_drift;

    let tool = "codex";
    multi_agents_manager_lib::database::set_tool_enabled(tool, true);
    let tool_dir = multi_agents_manager_lib::adapter::primary_skill_dir(tool).unwrap();
    std::fs::create_dir_all(&tool_dir).unwrap();
    let outside = std::env::temp_dir().join("v2m2-rc-l4-target");
    std::fs::create_dir_all(&outside).unwrap();
    let link = tool_dir.join("v2m2-rc-l4");
    std::os::unix::fs::symlink(&outside, &link).unwrap();

    let items = scan_drift();
    let hit = items.iter().find(|d| d.extension_id == "skill-v2m2-rc-l4");
    let hit = hit.expect("L4 外链应被报告（链接指向 ~/.mam 之外）");
    assert_eq!(hit.kind, "L4");
    assert_eq!(hit.tool_id, tool);
    assert!(
        hit.path.contains("v2m2-rc-l4"),
        "path 应指向链接: {}",
        hit.path
    );

    // 清理
    let _ = std::fs::remove_file(&link);
    let _ = std::fs::remove_dir_all(&outside);
}

/// W5 名册语义：disabled 工具的同类漂移（此处 L1）不出现
#[test]
fn reconcile_scan_disabled_tool_excluded() {
    let _guard = acquire_lock();
    support::setup();
    use multi_agents_manager_lib::database;
    use multi_agents_manager_lib::services::resource::reconcile::scan_drift;

    let tool = "kimi";
    database::set_tool_enabled(tool, true);
    std::fs::create_dir_all(multi_agents_manager_lib::adapter::primary_skill_dir(tool).unwrap())
        .unwrap();
    database::upsert_assignment("skill-v2m2-rc-dis", tool, true, "valid").unwrap();

    // 先确认 enabled 时 L1 在报（构造有效性自检）
    let items = scan_drift();
    assert!(
        items
            .iter()
            .any(|d| d.extension_id == "skill-v2m2-rc-dis" && d.kind == "L1"),
        "enabled 时 L1 应在报: {:?}",
        items
    );

    // 停用工具 → 同一漂移不再出现
    database::set_tool_enabled(tool, false);
    let items = scan_drift();
    assert!(
        !items.iter().any(|d| d.extension_id == "skill-v2m2-rc-dis"),
        "disabled 工具的漂移不应出现: {:?}",
        items
    );

    // 清理
    let _ = database::delete_assignments_for("skill-v2m2-rc-dis");
}
