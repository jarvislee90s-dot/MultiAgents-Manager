mod support; // 若 tests/ 下 support.rs 非共享 mod，按 dao_test.rs 的引用方式对齐

/// P2②：兼容判定改查 resource_bindings——claude 来源的 skill 不再对 codex 误报不兼容
#[test]
fn check_compatibility_uses_bindings_not_tags() {
    let _guard = PRESET_V2_TEST_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    support::setup();
    use multi_agents_manager_lib::database;
    use multi_agents_manager_lib::services::preset::check_compatibility;

    // 造一个 tags="claude"（来源工具）的 skill —— 旧逻辑会误判 codex 不兼容
    database::insert_extension(&database::ExtensionRecord {
        id: "skill-v2m1-compat".into(),
        kind: "skill".into(),
        name: "v2m1-compat".into(),
        description: None,
        source_path: "/tmp/x".into(),
        source_url: None,
        version: None,
        tags: Some("claude".into()),
        suite: None,
        source_tool: Some("claude".into()),
        is_native: false,
    })
    .unwrap();
    let pid = database::create_preset(
        "v2m1-compat-preset",
        &[("skill-v2m1-compat".into(), "skill".into())],
    )
    .unwrap();

    // 无绑定 → 通用，codex 兼容（旧逻辑这里是 incompatible）
    let report = check_compatibility(&pid, "codex");
    assert_eq!(
        report.compatible.len(),
        1,
        "tags=claude 不应再挡 codex: {:?}",
        report.incompatible
    );
    assert!(report.incompatible.is_empty());

    // 绑定 codex 专属后 → claude 不兼容且带原因
    database::upsert_resource_binding("skill-v2m1-compat", "codex", Some("依赖 codex App"))
        .unwrap();
    let report = check_compatibility(&pid, "claude");
    assert_eq!(report.incompatible.len(), 1);
    assert_eq!(report.incompatible[0].id, "skill-v2m1-compat");
    assert!(report.incompatible[0].reason.contains("codex"));
}

use std::sync::{Mutex, OnceLock};

/// 共享进程级 fake HOME 与全局状态的测试（stash 账本按 tool_id 查询无法用目录名
/// 区分；基底快照与工具 skill 目录同为共享态），并行跑会互相踩踏 —— 用互斥锁强制串行
static PRESET_V2_TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

/// 暂存往返：真目录移入 ~/.mam/stash/<tool>/skills 再移回；账本同步
#[test]
fn stash_and_restore_roundtrip() {
    let _ledger = PRESET_V2_TEST_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    support::setup();
    use multi_agents_manager_lib::database;
    use multi_agents_manager_lib::services::preset::stash;

    // 工具原生技能目录（fake HOME 下）
    let tool_dir = dirs::home_dir().unwrap().join(".codex/skills");
    std::fs::create_dir_all(tool_dir.join("v2m1-native-a")).unwrap();
    std::fs::write(tool_dir.join("v2m1-native-a/SKILL.md"), "hi").unwrap();

    // 暂存
    stash::stash_native_skill("codex", "v2m1-native-a", &tool_dir.join("v2m1-native-a")).unwrap();
    assert!(!tool_dir.join("v2m1-native-a").exists(), "工具目录应读不到");
    let stashed = stash::stash_dir("codex").join("v2m1-native-a");
    assert!(stashed.is_dir(), "暂存区应有该目录");
    assert_eq!(database::unrestored_stash(Some("codex")).len(), 1);

    // 回移
    let (restored, conflicts) = stash::restore_all_for_tool("codex");
    assert_eq!(restored, vec!["v2m1-native-a".to_string()]);
    assert!(conflicts.is_empty());
    assert!(tool_dir.join("v2m1-native-a/SKILL.md").exists());
    assert!(database::unrestored_stash(Some("codex")).is_empty());
    assert!(!stashed.exists(), "暂存区应清空");
}

/// 原位被占：不覆盖，留在暂存区，报告冲突
#[test]
fn stash_restore_conflict_keeps_stash() {
    let _ledger = PRESET_V2_TEST_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    support::setup();
    use multi_agents_manager_lib::database;
    use multi_agents_manager_lib::services::preset::stash;

    let tool_dir = dirs::home_dir().unwrap().join(".codex/skills");
    std::fs::create_dir_all(tool_dir.join("v2m1-native-b")).unwrap();
    std::fs::write(tool_dir.join("v2m1-native-b/SKILL.md"), "origin").unwrap();
    stash::stash_native_skill("codex", "v2m1-native-b", &tool_dir.join("v2m1-native-b")).unwrap();

    // 原位被别的目录占了
    std::fs::create_dir_all(tool_dir.join("v2m1-native-b")).unwrap();
    std::fs::write(tool_dir.join("v2m1-native-b/SKILL.md"), "intruder").unwrap();

    let (restored, conflicts) = stash::restore_all_for_tool("codex");
    assert!(restored.is_empty());
    assert_eq!(conflicts.len(), 1);
    assert!(conflicts[0].contains("v2m1-native-b"));
    // 不覆盖 + 暂存保留 + 账本未消
    assert_eq!(
        std::fs::read_to_string(tool_dir.join("v2m1-native-b/SKILL.md")).unwrap(),
        "intruder"
    );
    assert!(stash::stash_dir("codex").join("v2m1-native-b").exists());
    assert_eq!(database::unrestored_stash(Some("codex")).len(), 1);

    // 人工清场后孤儿恢复可补刀
    std::fs::remove_dir_all(tool_dir.join("v2m1-native-b")).unwrap();
    let n = stash::recover_orphans();
    assert!(n >= 1);
    assert!(tool_dir.join("v2m1-native-b/SKILL.md").exists());
    assert!(database::unrestored_stash(None)
        .iter()
        .all(|e| e.skill_name != "v2m1-native-b"));
}

/// 状态扫描：MAM 启用项 + 原生真目录都要进基底；链接不重复计为原生
#[test]
fn scan_tool_state_captures_mam_and_native() {
    let _guard = PRESET_V2_TEST_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    support::setup();
    use multi_agents_manager_lib::database;
    use multi_agents_manager_lib::services::preset::snapshot;
    use multi_agents_manager_lib::services::{disable_skill_for_tool, enable_skill_for_tool};

    // SSOT 造一个 MAM skill 并为 claude 启用（建链接）
    let ssot = dirs::home_dir().unwrap().join(".mam/skills/v2m1-scan-a");
    std::fs::create_dir_all(&ssot).unwrap();
    std::fs::write(ssot.join("SKILL.md"), "x").unwrap();
    database::insert_extension(&database::ExtensionRecord {
        id: "skill-v2m1-scan-a".into(),
        kind: "skill".into(),
        name: "v2m1-scan-a".into(),
        description: None,
        source_path: ssot.to_string_lossy().to_string(),
        source_url: None,
        version: None,
        tags: None,
        suite: None,
        source_tool: None,
        is_native: false,
    })
    .unwrap();
    enable_skill_for_tool("v2m1-scan-a", "claude").unwrap();
    // 子 Agent 分配行不得让同一 ext_id 重复入基底（快照 PK 冲突防护）
    database::upsert_assignment_with_subagent(
        "skill-v2m1-scan-a",
        "claude",
        "v2m1-scan-sub",
        true,
        "valid",
    )
    .unwrap();

    // claude 目录再放一个原生真目录
    let claude_dir = dirs::home_dir().unwrap().join(".claude/skills");
    std::fs::create_dir_all(claude_dir.join("v2m1-scan-native")).unwrap();
    std::fs::write(claude_dir.join("v2m1-scan-native/SKILL.md"), "y").unwrap();

    let state = snapshot::scan_tool_state("claude");
    let find = |id: &str| state.iter().find(|i| i.extension_id == id);

    let mam = find("skill-v2m1-scan-a").expect("MAM 启用项应入基底");
    assert_eq!(mam.origin, "mam");
    assert_eq!(mam.kind, "skill");
    assert_eq!(
        state
            .iter()
            .filter(|i| i.extension_id == "skill-v2m1-scan-a")
            .count(),
        1,
        "子 Agent 分配行不得让同一 ext_id 重复计入"
    );
    let native = find("skill-v2m1-scan-native").expect("原生真目录应入基底");
    assert_eq!(native.origin, "native");

    // 拍快照 → 可读回
    snapshot::capture_base_snapshot("claude").unwrap();
    let (active, items) = database::get_base_snapshot("claude").unwrap();
    assert!(active.is_none());
    assert!(items
        .iter()
        .any(|i| i.extension_id == "skill-v2m1-scan-a" && i.origin == "mam"));
    assert!(items
        .iter()
        .any(|i| i.extension_id == "skill-v2m1-scan-native" && i.origin == "native"));

    // 清场，避免影响其他测试
    let _ = database::disable_subagent_assignment("skill-v2m1-scan-a", "claude", "v2m1-scan-sub");
    disable_skill_for_tool("v2m1-scan-a", "claude").unwrap();
    database::destroy_base_snapshot("claude").unwrap();
}

/// 清扫差集：预设外的 MAM skill 停用、原生真目录暂存；常驻豁免两项都不动
#[test]
fn sweep_stashes_native_and_disables_mam_except_resident() {
    let _ledger = PRESET_V2_TEST_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    support::setup();
    use multi_agents_manager_lib::database;
    use multi_agents_manager_lib::services::preset::sweep;
    use multi_agents_manager_lib::services::{disable_skill_for_tool, enable_skill_for_tool};

    // MAM skill A（预设内）+ MAM skill B（预设外）为 claude 启用
    for name in ["v2m1-sw-a", "v2m1-sw-b"] {
        let ssot = dirs::home_dir().unwrap().join(".mam/skills").join(name);
        std::fs::create_dir_all(&ssot).unwrap();
        std::fs::write(ssot.join("SKILL.md"), "x").unwrap();
        database::insert_extension(&database::ExtensionRecord {
            id: format!("skill-{}", name),
            kind: "skill".into(),
            name: name.into(),
            description: None,
            source_path: ssot.to_string_lossy().to_string(),
            source_url: None,
            version: None,
            tags: None,
            suite: None,
            source_tool: None,
            is_native: false,
        })
        .unwrap();
        enable_skill_for_tool(name, "claude").unwrap();
    }
    // 原生真目录 C（预设外）与 D（常驻）
    let claude_dir = dirs::home_dir().unwrap().join(".claude/skills");
    for name in ["v2m1-sw-c", "v2m1-sw-d"] {
        std::fs::create_dir_all(claude_dir.join(name)).unwrap();
        std::fs::write(claude_dir.join(name).join("SKILL.md"), "n").unwrap();
    }
    database::set_tool_resident("claude", "skill-v2m1-sw-d", true).unwrap();

    // 计划：keep 只有 A（断言用 contains——集成测试共享 HOME，其他测试可能残留原生目录）
    let keep = vec![("skill-v2m1-sw-a".to_string(), "skill".to_string())];
    let plan = sweep::plan_sweep("claude", &keep);
    assert!(plan
        .disable_mam
        .contains(&("skill-v2m1-sw-b".to_string(), "skill".to_string())));
    assert!(plan.stash_native.contains(&"v2m1-sw-c".to_string()));
    assert!(
        !plan.stash_native.contains(&"v2m1-sw-d".to_string()),
        "常驻项不得进暂存计划"
    );

    // 执行：B 断链、C 暂存、D 不动
    let (disabled, stashed, failures) = sweep::execute_sweep("claude", &plan);
    assert!(disabled.contains(&"skill-v2m1-sw-b".to_string()));
    assert!(stashed.contains(&"v2m1-sw-c".to_string()));
    assert!(failures.is_empty(), "{:?}", failures);
    assert!(!claude_dir.join("v2m1-sw-b").exists(), "B 链接应已断");
    assert!(!claude_dir.join("v2m1-sw-c").exists(), "C 应已暂存");
    assert!(claude_dir.join("v2m1-sw-d").is_dir(), "常驻 D 不得动");

    // 清场
    database::set_tool_resident("claude", "skill-v2m1-sw-d", false).unwrap();
    disable_skill_for_tool("v2m1-sw-a", "claude").unwrap();
    let _ = database::destroy_base_snapshot("claude");
}

/// 完整生命周期（spec §3.2/§5）：开（拍基底+独占）→ 切换（沿用基底）→ 关（精确回基底+销毁）→ 再开（重拍新基底）
#[test]
fn apply_switch_restore_full_lifecycle() {
    let _guard = PRESET_V2_TEST_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    support::setup();
    use multi_agents_manager_lib::database;
    use multi_agents_manager_lib::services::preset::{apply_preset, restore_tool};
    use multi_agents_manager_lib::services::{disable_skill_for_tool, enable_skill_for_tool};

    let home = dirs::home_dir().unwrap();
    let claude_dir = home.join(".claude/skills");

    // 基底现场：MAM skill base-1 已启用（含一条子 Agent 分配行）+ 原生真目录 native-1
    //（保留单元素 for：与多资源场景的 setup 写法同构，便于扩展）
    #[allow(clippy::single_element_loop)]
    for name in ["v2m1-lc-base1"] {
        let ssot = dirs::home_dir().unwrap().join(".mam/skills").join(name);
        std::fs::create_dir_all(&ssot).unwrap();
        std::fs::write(ssot.join("SKILL.md"), "x").unwrap();
        database::insert_extension(&database::ExtensionRecord {
            id: format!("skill-{}", name),
            kind: "skill".into(),
            name: name.into(),
            description: None,
            source_path: ssot.to_string_lossy().to_string(),
            source_url: None,
            version: None,
            tags: None,
            suite: None,
            source_tool: None,
            is_native: false,
        })
        .unwrap();
        enable_skill_for_tool(name, "claude").unwrap();
    }
    // 子 Agent 分配行：应用预设时随工具级清扫断链，恢复默认后必须重建回来
    database::upsert_assignment_with_subagent(
        "skill-v2m1-lc-base1",
        "claude",
        "v2m1-lc-sub",
        true,
        "valid",
    )
    .unwrap();
    let sub_target = claude_dir.join("subagents/v2m1-lc-sub/v2m1-lc-base1");
    std::fs::create_dir_all(claude_dir.join("v2m1-lc-native1")).unwrap();
    std::fs::write(claude_dir.join("v2m1-lc-native1/SKILL.md"), "n").unwrap();

    // 预设 A：skill-a
    let mk = |name: &str| {
        let ssot = home.join(".mam/skills").join(name);
        std::fs::create_dir_all(&ssot).unwrap();
        std::fs::write(ssot.join("SKILL.md"), "x").unwrap();
        database::insert_extension(&database::ExtensionRecord {
            id: format!("skill-{}", name),
            kind: "skill".into(),
            name: name.into(),
            description: None,
            source_path: ssot.to_string_lossy().to_string(),
            source_url: None,
            version: None,
            tags: None,
            suite: None,
            source_tool: None,
            is_native: false,
        })
        .unwrap();
    };
    mk("v2m1-lc-a");
    mk("v2m1-lc-b");
    let preset_a =
        database::create_preset("v2m1-lc-A", &[("skill-v2m1-lc-a".into(), "skill".into())])
            .unwrap();
    let preset_b =
        database::create_preset("v2m1-lc-B", &[("skill-v2m1-lc-b".into(), "skill".into())])
            .unwrap();

    // 开 A：base-1 断链（含子 Agent 链级联清理）、native-1 暂存、a 启用；快照在、active=A
    let r = apply_preset(&preset_a, "claude").unwrap();
    assert!(r.success >= 1);
    assert!(
        r.disabled.contains(&"skill-v2m1-lc-base1".to_string()),
        "{:?}",
        r.disabled
    );
    assert!(
        r.stashed.contains(&"v2m1-lc-native1".to_string()),
        "{:?}",
        r.stashed
    );
    assert!(claude_dir.join("v2m1-lc-a").exists());
    assert!(!claude_dir.join("v2m1-lc-base1").exists());
    let (active, items) = database::get_base_snapshot("claude").unwrap();
    assert_eq!(active.as_deref(), Some(preset_a.as_str()));
    assert!(items
        .iter()
        .any(|i| i.extension_id == "skill-v2m1-lc-base1" && i.origin == "mam"));
    assert!(items
        .iter()
        .any(|i| i.extension_id == "skill-v2m1-lc-native1" && i.origin == "native"));

    // 切 B：a 断、b 启；基底沿用（native-1 仍暂存，base-1 仍不在）
    let r2 = apply_preset(&preset_b, "claude").unwrap();
    assert!(r2.disabled.contains(&"skill-v2m1-lc-a".to_string()));
    assert!(!claude_dir.join("v2m1-lc-a").exists());
    assert!(claude_dir.join("v2m1-lc-b").exists());
    assert!(
        !claude_dir.join("v2m1-lc-native1").exists(),
        "切换不清算基底，原生仍暂存"
    );
    let (active_b, _) = database::get_base_snapshot("claude").unwrap();
    assert_eq!(active_b.as_deref(), Some(preset_b.as_str()));

    // 会话期间手动漂移：再启用 base-1
    enable_skill_for_tool("v2m1-lc-base1", "claude").unwrap();
    assert!(claude_dir.join("v2m1-lc-base1").exists());

    // 关：精确回基底——base-1 回来（含子 Agent 链接重建）、native-1 回来、b/a 都不在；快照销毁
    let rr = restore_tool("claude").unwrap();
    assert!(
        claude_dir.join("v2m1-lc-base1").exists(),
        "MAM 基底项应重建"
    );
    assert!(
        claude_dir.join("v2m1-lc-native1").exists(),
        "原生暂存应回移"
    );
    assert!(rr.restored_native.contains(&"v2m1-lc-native1".to_string()));
    assert!(sub_target.exists(), "子 Agent 链接应随基底重建（Layer3）");
    assert!(!claude_dir.join("v2m1-lc-a").exists());
    assert!(!claude_dir.join("v2m1-lc-b").exists());
    assert!(
        database::get_base_snapshot("claude").is_none(),
        "恢复后快照销毁（会话级）"
    );

    // 再开 A：重拍新基底（= 刚恢复的状态）
    apply_preset(&preset_a, "claude").unwrap();
    let (active2, items2) = database::get_base_snapshot("claude").unwrap();
    assert_eq!(active2.as_deref(), Some(preset_a.as_str()));
    assert!(items2
        .iter()
        .any(|i| i.extension_id == "skill-v2m1-lc-base1"));
    // 清场
    let _ = restore_tool("claude");
    disable_skill_for_tool("v2m1-lc-base1", "claude").unwrap();
}

/// scope 守卫：tool 私有预设不可应用到别的工具（硬错误）
#[test]
fn apply_rejects_cross_tool_for_tool_scoped_preset() {
    let _guard = PRESET_V2_TEST_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    support::setup();
    use multi_agents_manager_lib::database;
    use multi_agents_manager_lib::services::preset::apply_preset;

    let id =
        database::create_preset_with_meta("v2m1-scope", "", "tool", Some("codex"), &[]).unwrap();
    let err = apply_preset(&id, "claude").unwrap_err();
    assert!(err.contains("绑定"), "应拒绝跨工具: {}", err);
}

/// 专属过滤：预设项对目标工具不兼容 → 剔除进 conflicts，不启用
#[test]
fn apply_filters_incompatible_items() {
    let _guard = PRESET_V2_TEST_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    support::setup();
    use multi_agents_manager_lib::database;
    use multi_agents_manager_lib::services::preset::apply_preset;

    let home = dirs::home_dir().unwrap();
    let ssot = home.join(".mam/skills/v2m1-filt-a");
    std::fs::create_dir_all(&ssot).unwrap();
    std::fs::write(ssot.join("SKILL.md"), "x").unwrap();
    database::insert_extension(&database::ExtensionRecord {
        id: "skill-v2m1-filt-a".into(),
        kind: "skill".into(),
        name: "v2m1-filt-a".into(),
        description: None,
        source_path: ssot.to_string_lossy().to_string(),
        source_url: None,
        version: None,
        tags: None,
        suite: None,
        source_tool: None,
        is_native: false,
    })
    .unwrap();
    database::upsert_resource_binding("skill-v2m1-filt-a", "codex", Some("专属 codex")).unwrap();

    let pid = database::create_preset("v2m1-filt", &[("skill-v2m1-filt-a".into(), "skill".into())])
        .unwrap();
    let r = apply_preset(&pid, "claude").unwrap();
    assert_eq!(r.success, 0);
    assert_eq!(r.conflicts.len(), 1);
    assert!(r.conflicts[0].contains("v2m1-filt-a"));
    let claude_dir = home.join(".claude/skills");
    assert!(!claude_dir.join("v2m1-filt-a").exists(), "被过滤项不得启用");
    // 清场辅助返回 ()，let _ 仅为表达「忽略即丢弃」的意图
    #[allow(clippy::let_unit_value)]
    let _ = restore_tool_cleanup_only("claude");
}

/// 测试辅助：只销毁快照不动文件（清场用）
fn restore_tool_cleanup_only(tool_id: &str) {
    let _ = multi_agents_manager_lib::database::destroy_base_snapshot(tool_id);
    let _ = multi_agents_manager_lib::services::preset::stash::restore_all_for_tool(tool_id);
}

/// 激活中的预设不可删除（spec §5.4）——先恢复默认再删
#[test]
fn delete_rejects_active_preset() {
    let _guard = PRESET_V2_TEST_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    support::setup();
    use multi_agents_manager_lib::commands::preset as cmd;
    use multi_agents_manager_lib::database;
    use multi_agents_manager_lib::services::preset::apply_preset;

    let home = dirs::home_dir().unwrap();
    let ssot = home.join(".mam/skills/v2m1-del-a");
    std::fs::create_dir_all(&ssot).unwrap();
    std::fs::write(ssot.join("SKILL.md"), "x").unwrap();
    database::insert_extension(&database::ExtensionRecord {
        id: "skill-v2m1-del-a".into(),
        kind: "skill".into(),
        name: "v2m1-del-a".into(),
        description: None,
        source_path: ssot.to_string_lossy().to_string(),
        source_url: None,
        version: None,
        tags: None,
        suite: None,
        source_tool: None,
        is_native: false,
    })
    .unwrap();
    let pid = database::create_preset("v2m1-del", &[("skill-v2m1-del-a".into(), "skill".into())])
        .unwrap();
    apply_preset(&pid, "claude").unwrap();

    let err = cmd::delete_preset(pid.clone()).unwrap_err();
    assert!(err.contains("激活"), "{}", err);

    let _ = multi_agents_manager_lib::services::preset::restore_tool("claude");
    cmd::delete_preset(pid).unwrap(); // 恢复后可删
}

/// 预览（dry-run）不动现场；get_active_preset 反映开关状态
#[test]
fn preview_is_dryrun_and_active_preset_queryable() {
    let _guard = PRESET_V2_TEST_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    support::setup();
    use multi_agents_manager_lib::commands::preset as cmd;
    use multi_agents_manager_lib::database;
    use multi_agents_manager_lib::services::preset::{apply_preset, preview_apply, restore_tool};

    assert_eq!(cmd::get_active_preset("claude".into()), None);

    let home = dirs::home_dir().unwrap();
    let claude_dir = home.join(".claude/skills");
    std::fs::create_dir_all(claude_dir.join("v2m1-pv-native")).unwrap();
    let ssot = home.join(".mam/skills/v2m1-pv-a");
    std::fs::create_dir_all(&ssot).unwrap();
    std::fs::write(ssot.join("SKILL.md"), "x").unwrap();
    database::insert_extension(&database::ExtensionRecord {
        id: "skill-v2m1-pv-a".into(),
        kind: "skill".into(),
        name: "v2m1-pv-a".into(),
        description: None,
        source_path: ssot.to_string_lossy().to_string(),
        source_url: None,
        version: None,
        tags: None,
        suite: None,
        source_tool: None,
        is_native: false,
    })
    .unwrap();
    let pid =
        database::create_preset("v2m1-pv", &[("skill-v2m1-pv-a".into(), "skill".into())]).unwrap();

    let pv = preview_apply(&pid, "claude").unwrap();
    assert_eq!(pv.to_enable, vec!["skill-v2m1-pv-a".to_string()]);
    assert!(pv.to_stash.contains(&"v2m1-pv-native".to_string()));
    assert!(pv.filtered.is_empty());
    assert!(claude_dir.join("v2m1-pv-native").exists(), "预览不得动现场");

    apply_preset(&pid, "claude").unwrap();
    assert_eq!(cmd::get_active_preset("claude".into()), Some(pid.clone()));
    let _ = restore_tool("claude");
    assert_eq!(cmd::get_active_preset("claude".into()), None);
}

/// P2①：MCP 入 SSOT 必须落 extensions 行——预设创建列表与卡片从此同源；
/// 注册表回填把历史无行的 skill/mcp 补进表
#[test]
fn mcp_import_and_backfill_register_rows() {
    let _guard = PRESET_V2_TEST_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    support::setup();
    use multi_agents_manager_lib::database;
    use multi_agents_manager_lib::services::resource::backfill_registry;

    // 手工放一个 MCP 配置文件（历史上 toggle_mcp 只写 assignment 不写表）
    let repo = dirs::home_dir().unwrap().join(".mam/mcp");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::write(repo.join("v2m1-backfill-mcp.json"), r#"{"command":"x"}"#).unwrap();
    // 手工放一个无行的 skill 目录（历史残留/手工放置）
    let skill = dirs::home_dir()
        .unwrap()
        .join(".mam/skills/v2m1-backfill-skill");
    std::fs::create_dir_all(&skill).unwrap();
    std::fs::write(skill.join("SKILL.md"), "x").unwrap();

    assert!(database::list_extensions()
        .iter()
        .all(|e| e.id != "mcp-v2m1-backfill-mcp"));
    backfill_registry();

    let ids: Vec<String> = database::list_extensions()
        .iter()
        .map(|e| e.id.clone())
        .collect();
    assert!(
        ids.contains(&"mcp-v2m1-backfill-mcp".to_string()),
        "MCP 应回填入表"
    );
    assert!(
        ids.contains(&"skill-v2m1-backfill-skill".to_string()),
        "无行 skill 应回填"
    );

    // 幂等：再跑不重复
    backfill_registry();
    let n_mcp = database::list_extensions()
        .iter()
        .filter(|e| e.id == "mcp-v2m1-backfill-mcp")
        .count();
    assert_eq!(n_mcp, 1);

    // save_mcp_config 命令直接落表（新导入路径）
    multi_agents_manager_lib::commands::resource::save_mcp_config(
        "v2m1-direct-mcp".into(),
        "node".into(),
        vec![],
        Default::default(),
    )
    .unwrap();
    assert!(
        database::list_extensions()
            .iter()
            .any(|e| e.id == "mcp-v2m1-direct-mcp"),
        "save_mcp_config 应写 extensions 行"
    );
}

/// 不变量（spec §3.2）：快照在而 active 为空（或反之）→ 检查器报告
#[test]
fn snapshot_invariant_detector_reports_broken_state() {
    let _guard = PRESET_V2_TEST_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    support::setup();
    use multi_agents_manager_lib::database::{self, BaseSnapshotItemRecord};
    use multi_agents_manager_lib::services::preset::check_snapshot_invariants;

    // 人为构造破坏态：快照在、active 为 None
    //（检查器只扫已注册工具 TOOL_IDS，故工具 id 须取注册表内的 dsh）
    database::save_base_snapshot(
        "dsh",
        None,
        &[BaseSnapshotItemRecord {
            extension_id: "skill-v2m1-inv".into(),
            kind: "skill".into(),
            origin: "mam".into(),
        }],
    )
    .unwrap();
    let broken = check_snapshot_invariants();
    assert!(broken.iter().any(|s| s.contains("dsh")), "{:?}", broken);
    database::destroy_base_snapshot("dsh").unwrap();
    assert!(check_snapshot_invariants().is_empty());
}

/// Patch 1a（评审裁决 1）会话守卫：激活会话进行中，孤儿恢复不得回移该工具的
/// 暂存项；销毁快照（会话结束）后恢复
#[test]
fn recover_orphans_skips_active_session_then_restores_after_destroy() {
    let _guard = PRESET_V2_TEST_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    support::setup();
    use multi_agents_manager_lib::database;
    use multi_agents_manager_lib::services::preset::stash;

    let codex_dir = dirs::home_dir().unwrap().join(".codex/skills");
    std::fs::create_dir_all(codex_dir.join("v2m1-gd-native")).unwrap();
    std::fs::write(codex_dir.join("v2m1-gd-native/SKILL.md"), "n").unwrap();
    stash::stash_native_skill("codex", "v2m1-gd-native", &codex_dir.join("v2m1-gd-native"))
        .unwrap();
    // 激活会话进行中：快照在且 active 非空
    database::save_base_snapshot("codex", Some("preset-v2m1-gd"), &[]).unwrap();

    let n_guarded = stash::recover_orphans();
    assert!(
        stash::stash_dir("codex").join("v2m1-gd-native").is_dir(),
        "激活会话期间孤儿恢复不得回移暂存项"
    );
    assert!(!codex_dir.join("v2m1-gd-native").exists(), "原位不得被触碰");
    assert_eq!(database::unrestored_stash(Some("codex")).len(), 1);

    // 会话结束（快照销毁）→ 孤儿恢复放行
    database::destroy_base_snapshot("codex").unwrap();
    let n = stash::recover_orphans();
    assert!(
        codex_dir.join("v2m1-gd-native/SKILL.md").exists(),
        "会话结束后应回移"
    );
    assert!(
        !stash::stash_dir("codex").join("v2m1-gd-native").exists(),
        "暂存区应清空"
    );
    assert!(database::unrestored_stash(Some("codex")).is_empty());
    assert!(
        n >= 1 && n_guarded == 0,
        "守卫期计数 0，放行后计数 >= 1（{n_guarded}/{n}）"
    );
}

/// Patch 1b（评审裁决 1）账本自愈：回移已落位但未销账的硬崩溃残留——
/// 暂存文件不在且原位在 → 销账，unrestored 不再含它（只写账不动文件）
#[test]
fn recover_orphans_self_heals_stale_ledger_entries() {
    let _guard = PRESET_V2_TEST_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    support::setup();
    use multi_agents_manager_lib::database;
    use multi_agents_manager_lib::services::preset::stash;

    let codex_dir = dirs::home_dir().unwrap().join(".codex/skills");
    std::fs::create_dir_all(codex_dir.join("v2m1-heal-orig")).unwrap();
    std::fs::write(codex_dir.join("v2m1-heal-orig/SKILL.md"), "n").unwrap();
    // 账目在而暂存文件不在（伪造「移完没销账即崩溃」）
    let ghost_stash = stash::stash_dir("codex").join("v2m1-heal-ghost");
    database::record_stash(
        "codex",
        "v2m1-heal-orig",
        &ghost_stash.to_string_lossy(),
        &codex_dir.join("v2m1-heal-orig").to_string_lossy(),
    )
    .unwrap();
    assert!(database::unrestored_stash(Some("codex"))
        .iter()
        .any(|e| e.skill_name == "v2m1-heal-orig"));

    stash::recover_orphans();
    assert!(
        !database::unrestored_stash(Some("codex"))
            .iter()
            .any(|e| e.skill_name == "v2m1-heal-orig"),
        "自愈后该账目应已销账"
    );
    assert!(
        codex_dir.join("v2m1-heal-orig/SKILL.md").exists(),
        "自愈不动文件"
    );
}

/// Patch 1c（评审裁决 1）无账孤儿对账：rename 与记账之间硬崩溃留下的
/// 无账暂存目录 → 移回原位 + 补记审计；原位被占则保留待人工
#[test]
fn recover_orphans_reclaims_unledgered_stash_dirs() {
    let _guard = PRESET_V2_TEST_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    support::setup();
    use multi_agents_manager_lib::database;
    use multi_agents_manager_lib::services::preset::stash;

    // 无账孤儿 A（原位空闲）与无账孤儿 B（原位被占）
    let ghost_a = stash::stash_dir("codex").join("v2m1-ghost-a");
    std::fs::create_dir_all(&ghost_a).unwrap();
    std::fs::write(ghost_a.join("SKILL.md"), "g").unwrap();
    let ghost_b = stash::stash_dir("codex").join("v2m1-ghost-b");
    std::fs::create_dir_all(&ghost_b).unwrap();
    std::fs::write(ghost_b.join("SKILL.md"), "g").unwrap();
    let codex_dir = dirs::home_dir().unwrap().join(".codex/skills");
    std::fs::create_dir_all(codex_dir.join("v2m1-ghost-b")).unwrap();
    std::fs::write(codex_dir.join("v2m1-ghost-b/SKILL.md"), "intruder").unwrap();

    stash::recover_orphans();

    // A：移回原位 + 补记审计（已销账，不在 unrestored）
    assert!(
        codex_dir.join("v2m1-ghost-a/SKILL.md").exists(),
        "无账孤儿应移回原位"
    );
    assert!(!ghost_a.exists(), "暂存区应清空");
    assert!(
        !database::unrestored_stash(Some("codex"))
            .iter()
            .any(|e| e.skill_name == "v2m1-ghost-a"),
        "补记审计应已销账（restored 状态）"
    );
    // B：原位被占 → 不覆盖、留在暂存区
    assert!(ghost_b.is_dir(), "原位被占的孤儿应留在暂存区");
    assert_eq!(
        std::fs::read_to_string(codex_dir.join("v2m1-ghost-b/SKILL.md")).unwrap(),
        "intruder",
        "原位内容不得被覆盖"
    );
}
