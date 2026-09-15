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

/// Patch 2（评审裁决 2）：取消勾选时前置恢复失败（Err）→ 不清理、不落 disabled，
/// 工具保持启用并计入 skipped_kept（可重试）
#[test]
fn apply_tool_changes_keeps_tool_enabled_when_preset_restore_fails() {
    let _guard = PRESET_V2_TEST_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    support::setup();
    use multi_agents_manager_lib::database;
    use multi_agents_manager_lib::services::tool_settings::{
        apply_tool_changes_with, ToolSettingChange,
    };
    use multi_agents_manager_lib::services::{disable_skill_for_tool, enable_skill_for_tool};

    // 现场：MAM skill 已启用（有链接可被清理），工具启用中
    let ssot = dirs::home_dir().unwrap().join(".mam/skills/v2m1-w5-a");
    std::fs::create_dir_all(&ssot).unwrap();
    std::fs::write(ssot.join("SKILL.md"), "x").unwrap();
    database::insert_extension(&database::ExtensionRecord {
        id: "skill-v2m1-w5-a".into(),
        kind: "skill".into(),
        name: "v2m1-w5-a".into(),
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
    enable_skill_for_tool("v2m1-w5-a", "claude").unwrap();
    assert!(database::get_tool_enabled("claude"), "前置：工具启用中");
    let claude_link = dirs::home_dir().unwrap().join(".claude/skills/v2m1-w5-a");
    assert!(claude_link.exists(), "前置：链接在场");

    // 注入恢复失败
    let changes = vec![ToolSettingChange {
        tool_id: "claude".into(),
        enabled: false,
    }];
    let result = apply_tool_changes_with(changes, &|_tool: &str| Err("boom".to_string()));

    assert!(
        database::get_tool_enabled("claude"),
        "恢复失败时工具必须保持启用"
    );
    assert!(
        result
            .skipped_kept
            .iter()
            .any(|s| s.contains("claude") && s.contains("快照销账失败") && s.contains("重试可愈")),
        "skipped_kept 应含精确文案: {:?}",
        result.skipped_kept
    );
    // W5 清理未执行：工具目录里仍是 MAM 链接（未被还原/未被动过）
    let link_meta = std::fs::symlink_metadata(&claude_link).expect("链接应仍在场");
    assert!(
        link_meta.file_type().is_symlink(),
        "恢复失败时链接必须原样保留"
    );
    // 清场：注入 Ok 走正常清理；W5 还原会把链接还原成真实内容且 assignment 行
    // 仍为 enabled——必须禁用 assignment 并移除真实目录，否则后续测试扫描 claude
    // 会出现同 ext_id 双条目（拍基底 UNIQUE 失败）
    let _ = apply_tool_changes_with(
        vec![ToolSettingChange {
            tool_id: "claude".into(),
            enabled: false,
        }],
        &|_tool: &str| Ok(Default::default()),
    );
    let _ = disable_skill_for_tool("v2m1-w5-a", "claude");
    let _ = std::fs::remove_dir_all(&claude_link);
    database::set_tool_enabled("claude", true);
}

/// Patch 2（评审裁决 2）真实路径：无激活预设时 restore 返回 Ok（空 conflicts）
/// → 取消勾选照常清理并落 disabled
#[test]
fn apply_tool_changes_disables_tool_when_preset_restore_succeeds() {
    let _guard = PRESET_V2_TEST_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    support::setup();
    use multi_agents_manager_lib::database;
    use multi_agents_manager_lib::services::{
        disable_skill_for_tool, enable_skill_for_tool, tool_settings,
    };

    let ssot = dirs::home_dir().unwrap().join(".mam/skills/v2m1-w5-b");
    std::fs::create_dir_all(&ssot).unwrap();
    std::fs::write(ssot.join("SKILL.md"), "x").unwrap();
    database::insert_extension(&database::ExtensionRecord {
        id: "skill-v2m1-w5-b".into(),
        kind: "skill".into(),
        name: "v2m1-w5-b".into(),
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
    enable_skill_for_tool("v2m1-w5-b", "claude").unwrap();

    let changes = vec![tool_settings::ToolSettingChange {
        tool_id: "claude".into(),
        enabled: false,
    }];
    let result = tool_settings::apply_tool_changes(changes);

    assert!(
        !database::get_tool_enabled("claude"),
        "恢复 Ok 时应照常落 disabled"
    );
    // W5 清理语义：链接被还原为真实内容（内容保留、MAM 接管解除），而非删除
    let w5_path = dirs::home_dir().unwrap().join(".claude/skills/v2m1-w5-b");
    let w5_meta = std::fs::symlink_metadata(&w5_path).expect("W5 还原后原位应有真实内容");
    assert!(
        !w5_meta.file_type().is_symlink(),
        "MAM 链接应已解除（还原为真实目录）"
    );
    assert!(w5_path.join("SKILL.md").exists(), "SSOT 内容应落回原位");
    assert!(
        result.restored.contains(&"v2m1-w5-b".to_string()),
        "清理结果应报告还原项: {:?}",
        result.restored
    );
    // 清场：禁用 assignment 并移除 W5 还原出的真实目录（同上，避免污染
    // 后续测试的 scan_tool_state），再恢复工具启用态
    let _ = disable_skill_for_tool("v2m1-w5-b", "claude");
    let _ = std::fs::remove_dir_all(&w5_path);
    database::set_tool_enabled("claude", true);
}

/// Patch 3（评审裁决 3）：幂等重保存 MCP 不得抹掉用户编辑过的元数据——
/// INSERT OR REPLACE 会清空 description/suite，必须走 INSERT OR IGNORE
#[test]
fn save_mcp_config_preserves_user_metadata() {
    let _guard = PRESET_V2_TEST_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    support::setup();
    use multi_agents_manager_lib::commands::resource::save_mcp_config;
    use multi_agents_manager_lib::database;

    // 预置带用户备注的 MCP 行（历史编辑过的元数据）
    database::insert_extension(&database::ExtensionRecord {
        id: "mcp-v2m1-meta".into(),
        kind: "mcp".into(),
        name: "v2m1-meta".into(),
        description: Some("用户备注".into()),
        source_path: "/tmp/old".into(),
        source_url: None,
        version: None,
        tags: None,
        suite: None,
        source_tool: None,
        is_native: false,
    })
    .unwrap();

    // 同名重保存配置
    save_mcp_config(
        "v2m1-meta".into(),
        "node".into(),
        vec![],
        Default::default(),
    )
    .unwrap();

    let row = database::list_extensions()
        .into_iter()
        .find(|e| e.id == "mcp-v2m1-meta")
        .expect("重保存后行应存在");
    assert_eq!(
        row.description.as_deref(),
        Some("用户备注"),
        "重保存不得抹掉用户元数据"
    );
}

/// Patch 5 修改 1（设计裁决：磁盘实况优先）：账本 enabled 但磁盘为真目录的
/// 漂移态，拍基底不再 UNIQUE 崩——剔除 MAM 条目、按 native 记录恰一条，
/// sweep 对它的处置走「暂存」而非「停用」
#[test]
fn scan_tool_state_dedups_drifted_ledger_entry_to_native() {
    let _guard = PRESET_V2_TEST_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    support::setup();
    use multi_agents_manager_lib::database;
    use multi_agents_manager_lib::services::preset::{snapshot, sweep};
    use multi_agents_manager_lib::services::{disable_skill_for_tool, enable_skill_for_tool};

    // 漂移现场：账本 enabled + 同名真目录（W5 还原内容后名册未销的形态）
    let ssot = dirs::home_dir().unwrap().join(".mam/skills/v2m1-drift-x");
    std::fs::create_dir_all(&ssot).unwrap();
    std::fs::write(ssot.join("SKILL.md"), "x").unwrap();
    database::insert_extension(&database::ExtensionRecord {
        id: "skill-v2m1-drift-x".into(),
        kind: "skill".into(),
        name: "v2m1-drift-x".into(),
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
    enable_skill_for_tool("v2m1-drift-x", "claude").unwrap();
    let drift = dirs::home_dir()
        .unwrap()
        .join(".claude/skills/v2m1-drift-x");
    std::fs::remove_file(&drift).unwrap();
    std::fs::create_dir_all(&drift).unwrap();
    std::fs::write(drift.join("SKILL.md"), "real").unwrap();

    // 当前实现：MAM 条目 + 原生条目同 ext_id → 拍基底 UNIQUE 崩（红）
    snapshot::capture_base_snapshot("claude").unwrap();
    let (_, items) = database::get_base_snapshot("claude").unwrap();
    let hits: Vec<_> = items
        .iter()
        .filter(|i| i.extension_id == "skill-v2m1-drift-x")
        .collect();
    assert_eq!(hits.len(), 1, "漂移项快照恰一条: {:?}", items);
    assert_eq!(hits[0].origin, "native", "按磁盘实况记为 native");

    // sweep 计划：走暂存（native 路径）而非停用
    let plan = sweep::plan_sweep("claude", &[]);
    assert!(
        plan.stash_native.contains(&"v2m1-drift-x".to_string()),
        "漂移项应进暂存计划: {:?}",
        plan.stash_native
    );
    assert!(
        !plan
            .disable_mam
            .iter()
            .any(|(id, _)| id == "skill-v2m1-drift-x"),
        "漂移项不得进停用计划: {:?}",
        plan.disable_mam
    );

    // 清场：禁用 assignment、移除真目录与快照，补删 SSOT 目录与 extensions 行
    database::destroy_base_snapshot("claude").unwrap();
    let _ = disable_skill_for_tool("v2m1-drift-x", "claude");
    let _ = std::fs::remove_dir_all(&drift);
    let _ = std::fs::remove_dir_all(&ssot);
    let _ = database::delete_extension("skill-v2m1-drift-x");
}

/// Patch 5 修改 2（评审 Minor 1）：首次应用的瞬态（快照已存、active 未设）
/// 也视为会话中，孤儿恢复不得回移——否则启动线程会拆掉进行中的应用
#[test]
fn recover_orphans_treats_unactivated_snapshot_as_in_session() {
    let _guard = PRESET_V2_TEST_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    support::setup();
    use multi_agents_manager_lib::database;
    use multi_agents_manager_lib::services::preset::stash;

    let codex_dir = dirs::home_dir().unwrap().join(".codex/skills");
    std::fs::create_dir_all(codex_dir.join("v2m1-tr-native")).unwrap();
    std::fs::write(codex_dir.join("v2m1-tr-native/SKILL.md"), "n").unwrap();
    stash::stash_native_skill("codex", "v2m1-tr-native", &codex_dir.join("v2m1-tr-native"))
        .unwrap();
    // 瞬态：快照在、active 未设（apply 的拍基底与设激活之间）
    database::save_base_snapshot("codex", None, &[]).unwrap();

    let n = stash::recover_orphans();
    assert_eq!(n, 0, "瞬态期不得回移");
    assert!(
        stash::stash_dir("codex").join("v2m1-tr-native").is_dir(),
        "瞬态期暂存项应保留"
    );
    assert_eq!(database::unrestored_stash(Some("codex")).len(), 1);

    // 清场：销快照（守卫解除）后回移，恢复现场
    database::destroy_base_snapshot("codex").unwrap();
    stash::restore_all_for_tool("codex");
    assert!(codex_dir.join("v2m1-tr-native/SKILL.md").exists());
}

/// Patch 6（终审 Critical 数据丢失修复）：恢复对账循环的漂移防护——
/// 快照内任意 origin 的项都不属于「会话新增」，按 native 记录的漂移项
/// （账本 enabled + 真目录）恢复后真目录已回移，走 disable 会把真目录删掉
#[test]
fn restore_roundtrip_preserves_drifted_real_dir() {
    let _guard = PRESET_V2_TEST_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    support::setup();
    use multi_agents_manager_lib::database;
    use multi_agents_manager_lib::services::enable_skill_for_tool;
    use multi_agents_manager_lib::services::preset::{apply_preset, restore_tool};

    // 漂移现场：skill-v2m1-dl-x 账本 enabled + 同名真目录（有内容）
    let ssot = dirs::home_dir().unwrap().join(".mam/skills/v2m1-dl-x");
    std::fs::create_dir_all(&ssot).unwrap();
    std::fs::write(ssot.join("SKILL.md"), "x").unwrap();
    database::insert_extension(&database::ExtensionRecord {
        id: "skill-v2m1-dl-x".into(),
        kind: "skill".into(),
        name: "v2m1-dl-x".into(),
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
    enable_skill_for_tool("v2m1-dl-x", "claude").unwrap();
    let drift_dir = dirs::home_dir().unwrap().join(".claude/skills/v2m1-dl-x");
    std::fs::remove_file(&drift_dir).unwrap();
    std::fs::create_dir_all(&drift_dir).unwrap();
    std::fs::write(drift_dir.join("SKILL.md"), "用户的真实内容").unwrap();

    // 预设只含另一项（不含漂移项）→ 漂移项按 native 暂存
    let ssot_a = dirs::home_dir().unwrap().join(".mam/skills/v2m1-dl-a");
    std::fs::create_dir_all(&ssot_a).unwrap();
    std::fs::write(ssot_a.join("SKILL.md"), "x").unwrap();
    database::insert_extension(&database::ExtensionRecord {
        id: "skill-v2m1-dl-a".into(),
        kind: "skill".into(),
        name: "v2m1-dl-a".into(),
        description: None,
        source_path: ssot_a.to_string_lossy().to_string(),
        source_url: None,
        version: None,
        tags: None,
        suite: None,
        source_tool: None,
        is_native: false,
    })
    .unwrap();
    let pid =
        database::create_preset("v2m1-dl", &[("skill-v2m1-dl-a".into(), "skill".into())]).unwrap();
    apply_preset(&pid, "claude").unwrap();
    assert!(
        !drift_dir.exists()
            && database::unrestored_stash(Some("claude"))
                .iter()
                .any(|e| e.skill_name == "v2m1-dl-x"),
        "前置：漂移真目录应已被独占清扫暂存"
    );

    // 恢复默认：真目录回移 = 基底态两项并存（账本 enabled + 真目录）
    restore_tool("claude").unwrap();

    assert!(
        drift_dir.join("SKILL.md").exists()
            && std::fs::read_to_string(drift_dir.join("SKILL.md")).unwrap() == "用户的真实内容",
        "漂移真目录必须原样保留（当前实现会被 disable 循环删除）"
    );
    let assignments = database::list_assignments("claude");
    assert!(
        assignments
            .iter()
            .any(|a| a.extension_id == "skill-v2m1-dl-x" && a.enabled),
        "账本 enabled 是基底态的一部分，不得被停用"
    );
    assert!(
        database::get_base_snapshot("claude").is_none(),
        "恢复后快照销毁（会话级生命周期不变）"
    );

    // 清场
    let _ = database::upsert_assignment("skill-v2m1-dl-x", "claude", false, "missing");
    let _ = database::upsert_assignment("skill-v2m1-dl-a", "claude", false, "missing");
    let _ = std::fs::remove_dir_all(&drift_dir);
    let _ = std::fs::remove_dir_all(&ssot_a);
}

/// M2 开工前置回归锁（裁决 2 / plan Task 1）：MCP 配置段独占往返——
/// 空预设 apply 清扫：assignment disabled + ~/.claude.json mcpServers 段移除；
/// restore_tool 精确重建：restored_mam 含之 + assignment enabled + 配置段重写
#[test]
fn mcp_sweep_restore_roundtrip() {
    let _guard = PRESET_V2_TEST_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    support::setup();
    use multi_agents_manager_lib::database;
    use multi_agents_manager_lib::services::mcp::tool_mcp_config_path;
    use multi_agents_manager_lib::services::preset::{apply_preset, restore_tool};
    use multi_agents_manager_lib::services::toggle_mcp;

    let claude_json = tool_mcp_config_path("claude").unwrap();
    let section_has = |name: &str| {
        let content = std::fs::read_to_string(&claude_json).unwrap_or_default();
        let root: serde_json::Value = serde_json::from_str(&content).unwrap_or_default();
        root["mcpServers"].get(name).is_some()
    };

    // 基底：MCP 经真实导入路径入仓（save_mcp_config 落仓库文件 + 注册表行），再为 claude 启用
    multi_agents_manager_lib::commands::resource::save_mcp_config(
        "v2m2-mcp-a".into(),
        "npx".into(),
        vec![],
        Default::default(),
    )
    .unwrap();
    assert!(
        database::list_extensions()
            .iter()
            .any(|e| e.id == "mcp-v2m2-mcp-a"),
        "前置：注册表行应在（save_mcp_config 落表）"
    );
    toggle_mcp("v2m2-mcp-a", "claude", true).unwrap();
    assert!(
        database::list_assignments("claude")
            .iter()
            .any(|a| a.extension_id == "mcp-v2m2-mcp-a" && a.enabled),
        "前置：assignment 应 enabled"
    );
    assert!(section_has("v2m2-mcp-a"), "前置：mcpServers 段应已写入");
    assert!(database::get_tool_enabled("claude"), "前置：工具启用中");

    // 空预设 apply → 独占清扫：assignment disabled + 配置段移除
    //（断言用 contains——共享 HOME 下其他测试可能残留启用项同被清扫）
    let pid = database::create_preset("v2m2-mcp-sweep", &[]).unwrap();
    let r = apply_preset(&pid, "claude").unwrap();
    assert!(
        r.disabled.contains(&"mcp-v2m2-mcp-a".to_string()),
        "disabled 应含 MCP 项: {:?}",
        r.disabled
    );
    assert!(
        !r.failures.iter().any(|f| f.contains("v2m2-mcp-a")),
        "MCP 清扫不得失败: {:?}",
        r.failures
    );
    let asg = database::list_assignments("claude")
        .into_iter()
        .find(|a| a.extension_id == "mcp-v2m2-mcp-a")
        .expect("assignment 行应存在");
    assert!(!asg.enabled, "清扫后 assignment 应 disabled");
    assert_eq!(asg.link_status, "missing");
    assert!(!section_has("v2m2-mcp-a"), "清扫后配置段应移除");

    // restore → 精确重建：配置段重写 + assignment enabled
    let rr = restore_tool("claude").unwrap();
    assert!(
        rr.restored_mam.contains(&"mcp-v2m2-mcp-a".to_string()),
        "restored_mam 应含 MCP 项: {:?}",
        rr.restored_mam
    );
    let asg = database::list_assignments("claude")
        .into_iter()
        .find(|a| a.extension_id == "mcp-v2m2-mcp-a")
        .expect("assignment 行应存在");
    assert!(asg.enabled, "恢复后 assignment 应 enabled");
    assert_eq!(asg.link_status, "valid");
    assert!(section_has("v2m2-mcp-a"), "恢复后配置段应重写");
    let root: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&claude_json).unwrap()).unwrap();
    assert_eq!(
        root["mcpServers"]["v2m2-mcp-a"]["command"],
        "npx",
        "重写的配置段内容应与仓库一致"
    );

    // 清场
    let _ = toggle_mcp("v2m2-mcp-a", "claude", false);
    let _ = database::delete_extension("mcp-v2m2-mcp-a");
    let _ = std::fs::remove_file(dirs::home_dir().unwrap().join(".mam/mcp/v2m2-mcp-a.json"));
}

/// M2 开工前置回归锁（裁决 2 / plan Task 1）：file 型插件独占往返——
/// 空预设 apply 清扫断链（工具插件目录项消失）→ restore 链接回来（enabled）
#[test]
fn plugin_file_sweep_restore_v2m2_plug() {
    let _guard = PRESET_V2_TEST_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    support::setup();
    use multi_agents_manager_lib::database;
    use multi_agents_manager_lib::services::preset::{apply_preset, restore_tool};
    use multi_agents_manager_lib::services::toggle_plugin;

    let home = dirs::home_dir().unwrap();
    // SSOT 仓库放真目录 + 注册表行（与生产扫描登记 file 型插件同形态：tags=Some("file")）
    let ssot = home.join(".mam/plugins/v2m2-plug-a");
    std::fs::create_dir_all(&ssot).unwrap();
    std::fs::write(ssot.join("plugin.json"), "{}").unwrap();
    database::insert_extension(&database::ExtensionRecord {
        id: "plugin-v2m2-plug-a".into(),
        kind: "plugin".into(),
        name: "v2m2-plug-a".into(),
        description: None,
        source_path: ssot.to_string_lossy().to_string(),
        source_url: None,
        version: None,
        tags: Some("file".into()),
        suite: None,
        source_tool: Some("claude".into()),
        is_native: false,
    })
    .unwrap();
    toggle_plugin("v2m2-plug-a", "claude", true, "file").unwrap();
    let target = home.join(".claude/plugins/v2m2-plug-a");
    assert!(
        target.join("plugin.json").exists(),
        "前置：链接在场且可穿透"
    );
    assert!(database::get_tool_enabled("claude"), "前置：工具启用中");

    // 空预设 apply → 独占清扫：断链 + assignment disabled
    let pid = database::create_preset("v2m2-plug-sweep", &[]).unwrap();
    let r = apply_preset(&pid, "claude").unwrap();
    assert!(
        r.disabled.contains(&"plugin-v2m2-plug-a".to_string()),
        "disabled 应含插件项: {:?}",
        r.disabled
    );
    assert!(
        !r.failures.iter().any(|f| f.contains("v2m2-plug-a")),
        "插件清扫不得失败: {:?}",
        r.failures
    );
    assert!(
        std::fs::symlink_metadata(&target).is_err(),
        "清扫后工具插件目录项应消失"
    );
    let asg = database::list_assignments("claude")
        .into_iter()
        .find(|a| a.extension_id == "plugin-v2m2-plug-a")
        .expect("assignment 行应存在");
    assert!(!asg.enabled, "清扫后 assignment 应 disabled");

    // restore → 链接回来 + assignment enabled
    let rr = restore_tool("claude").unwrap();
    assert!(
        rr.restored_mam.contains(&"plugin-v2m2-plug-a".to_string()),
        "restored_mam 应含插件项: {:?}",
        rr.restored_mam
    );
    let meta = std::fs::symlink_metadata(&target).expect("恢复后工具插件目录项应在场");
    assert!(meta.file_type().is_symlink(), "恢复的应是符号链接");
    assert!(target.join("plugin.json").exists(), "链接应可穿透到 SSOT");
    let asg = database::list_assignments("claude")
        .into_iter()
        .find(|a| a.extension_id == "plugin-v2m2-plug-a")
        .expect("assignment 行应存在");
    assert!(asg.enabled, "恢复后 assignment 应 enabled");
    assert_eq!(asg.link_status, "valid");

    // 清场
    let _ = toggle_plugin("v2m2-plug-a", "claude", false, "file");
    let _ = std::fs::remove_dir_all(&ssot);
    let _ = database::delete_extension("plugin-v2m2-plug-a");
}

/// M2 开工前置回归锁（裁决 2 / plan Task 1）：config 型插件独占往返——
/// SSOT 为仓库内 .json 条目文件（生产扫描对文件形态登记 tags=Some("config")），
/// 空预设 apply 清扫摘 ~/.claude/settings.json plugins 段条目 → restore 重写 + enabled
#[test]
fn plugin_config_sweep_restore_v2m2_plugcfg() {
    let _guard = PRESET_V2_TEST_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    support::setup();
    use multi_agents_manager_lib::database;
    use multi_agents_manager_lib::services::preset::{apply_preset, restore_tool};
    use multi_agents_manager_lib::services::toggle_plugin;

    let home = dirs::home_dir().unwrap();
    // config 型插件的 SSOT 是仓库内的 .json 条目文件（toggle 从中读 entries）
    let repo_json = home.join(".mam/plugins/v2m2-plug-cfg.json");
    std::fs::write(&repo_json, r#"{"source":"v2m2"}"#).unwrap();
    database::insert_extension(&database::ExtensionRecord {
        id: "plugin-v2m2-plug-cfg".into(),
        kind: "plugin".into(),
        name: "v2m2-plug-cfg".into(),
        description: None,
        source_path: repo_json.to_string_lossy().to_string(),
        source_url: None,
        version: None,
        tags: Some("config".into()),
        suite: None,
        source_tool: Some("claude".into()),
        is_native: false,
    })
    .unwrap();
    toggle_plugin("v2m2-plug-cfg", "claude", true, "config").unwrap();
    let settings = home.join(".claude/settings.json");
    let plugins_has = |name: &str| {
        let content = std::fs::read_to_string(&settings).unwrap_or_default();
        let root: serde_json::Value = serde_json::from_str(&content).unwrap_or_default();
        root["plugins"].get(name).is_some()
    };
    assert!(
        plugins_has("v2m2-plug-cfg"),
        "前置：plugins 段条目应已写入"
    );
    assert!(database::get_tool_enabled("claude"), "前置：工具启用中");

    // 空预设 apply → 独占清扫：条目摘除 + assignment disabled
    let pid = database::create_preset("v2m2-plugcfg-sweep", &[]).unwrap();
    let r = apply_preset(&pid, "claude").unwrap();
    assert!(
        r.disabled.contains(&"plugin-v2m2-plug-cfg".to_string()),
        "disabled 应含插件项: {:?}",
        r.disabled
    );
    assert!(
        !r.failures.iter().any(|f| f.contains("v2m2-plug-cfg")),
        "插件清扫不得失败: {:?}",
        r.failures
    );
    assert!(!plugins_has("v2m2-plug-cfg"), "清扫后 plugins 段条目应移除");
    let asg = database::list_assignments("claude")
        .into_iter()
        .find(|a| a.extension_id == "plugin-v2m2-plug-cfg")
        .expect("assignment 行应存在");
    assert!(!asg.enabled, "清扫后 assignment 应 disabled");

    // restore → 条目重写 + assignment enabled
    let rr = restore_tool("claude").unwrap();
    assert!(
        rr.restored_mam.contains(&"plugin-v2m2-plug-cfg".to_string()),
        "restored_mam 应含插件项: {:?}",
        rr.restored_mam
    );
    assert!(plugins_has("v2m2-plug-cfg"), "恢复后 plugins 段条目应重写");
    let root: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
    assert_eq!(
        root["plugins"]["v2m2-plug-cfg"]["source"],
        "v2m2",
        "重写的条目内容应与仓库一致"
    );
    let asg = database::list_assignments("claude")
        .into_iter()
        .find(|a| a.extension_id == "plugin-v2m2-plug-cfg")
        .expect("assignment 行应存在");
    assert!(asg.enabled, "恢复后 assignment 应 enabled");
    assert_eq!(asg.link_status, "valid");

    // 清场
    let _ = toggle_plugin("v2m2-plug-cfg", "claude", false, "config");
    let _ = std::fs::remove_file(&repo_json);
    let _ = database::delete_extension("plugin-v2m2-plug-cfg");
}
