mod support; // 若 tests/ 下 support.rs 非共享 mod，按 dao_test.rs 的引用方式对齐

/// P2②：兼容判定改查 resource_bindings——claude 来源的 skill 不再对 codex 误报不兼容
#[test]
fn check_compatibility_uses_bindings_not_tags() {
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
    assert_eq!(report.compatible.len(), 1, "tags=claude 不应再挡 codex: {:?}", report.incompatible);
    assert!(report.incompatible.is_empty());

    // 绑定 codex 专属后 → claude 不兼容且带原因
    database::upsert_resource_binding("skill-v2m1-compat", "codex", Some("依赖 codex App"))
        .unwrap();
    let report = check_compatibility(&pid, "claude");
    assert_eq!(report.incompatible.len(), 1);
    assert_eq!(report.incompatible[0].id, "skill-v2m1-compat");
    assert!(report.incompatible[0].reason.contains("codex"));
}
