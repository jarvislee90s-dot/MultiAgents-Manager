// 遗留链接识别 + 迁移/保留核心（spec 2026-09-09 §4.3 / §6）
//
// 背景：codex 注册表已切至私有目录 `~/.codex/skills`（Task 1），历史版本在
// `~/.agents/skills` 下留有 MAM 自建链接（→ `~/.mam/active/codex/<name>`）。
// 本模块为一次性迁移对话框提供后端：识别谓词 + migrate/keep 两种处置。
//
// 边界约束（F5）：DB assignments 与 Layer 2 是启用状态唯一真源，工具侧链接只是投影。
// 因此本模块是**纯文件系统操作**——全部路径由调用方注入（MigrationPaths），
// 零数据库访问、零真实家目录访问（测试全 tempdir）。

use crate::linker;
use std::path::{Path, PathBuf};

/// 迁移边界路径（全部由调用方注入；生产从 dirs::home_dir() 构建，测试用 tempdir）
pub struct MigrationPaths {
    /// `~/.agents/skills`（共享目录；MAM 只读，唯一例外是本模块对 MAM 自建链接的处置）
    pub agents_dir: PathBuf,
    /// `~/.codex/skills`（codex 私有技能目录，migrate 模式的搬迁目标）
    pub codex_skills_dir: PathBuf,
    /// `~/.mam/skills`（Layer 1 SSOT 真目录，keep 模式的改指目标）
    pub layer1_dir: PathBuf,
    /// `~/.mam/active/codex`（Layer 2 激活目录根，识别谓词的前缀边界）
    pub layer2_root: PathBuf,
}

/// 逐条结果报告（serde camelCase 序列化给前端）
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationItemReport {
    pub name: String,
    /// "ok" | "skipped" | "error"
    pub status: String,
    pub detail: Option<String>,
}

/// 识别谓词（spec §4.3）：entry 是否为「指向 Layer 2 的 MAM 遗留 codex 链接」。
/// 精确语义（勿走样）：
/// 1. 是链接（symlink / Windows junction，`link_marker_is_present` 双平台判据）；
/// 2. `read_link` 取**单跳字面 target**——绝不能用 `fs::canonicalize`：canonicalize
///    会穿透 Layer 2 解析到 Layer 1 真目录（`~/.mam/skills/<name>`），前缀判断将
///    永不匹配；这是本谓词最关键的坑；
/// 3. target 归一化后**组件级**位于 layer2_root 之下（`Path::starts_with` 逐组件
///    比较，天然避免 `/a/b` vs `/a/bc` 的字符串误判）；
/// 4. **无论链路是否可达都命中**（review I-2）：迁移前发生 disable/uninstall 会删掉
///    Layer 2，遗留链接随之悬空——若按可达性排除，这些 MAM 残链将永远脱离对话框
///    管辖、滞留共享目录。断链项由 migrate 自愈（仅清理旧链，不建悬空新链）。
fn is_legacy_codex_link(entry: &Path, layer2_root: &Path) -> bool {
    if !linker::link_marker_is_present(entry) {
        return false;
    }
    // 单跳字面 target（勿 canonicalize，见上）
    let Ok(raw) = std::fs::read_link(entry) else {
        return false;
    };
    let parent = entry.parent().unwrap_or_else(|| Path::new(""));
    let normalized = linker::normalize_link_target(&raw, parent);
    normalized.starts_with(layer2_root)
}

/// 检测 agents_dir 下的 MAM 遗留 codex 链接，返回命中的名称清单（排序）。
/// agents_dir 不存在 / 不可读 → 空结果静默（spec §6：无遗留 → 无对话框）。
pub fn detect_legacy_links(paths: &MigrationPaths) -> Vec<String> {
    let mut names = Vec::new();
    let Ok(entries) = std::fs::read_dir(&paths.agents_dir) else {
        return names;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if is_legacy_codex_link(&path, &paths.layer2_root) {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                names.push(name.to_string());
            }
        }
    }
    names.sort();
    names
}

/// 执行迁移/保留（mode: "migrate" | "keep"），返回逐条报告。
/// 与 detect 之间状态可能变化：对每条**重新跑谓词**，不再命中 → `skipped`
/// （detail 说明链接已不存在/不再匹配，spec §4.3）。
pub fn migrate_legacy_links(paths: &MigrationPaths, mode: &str) -> Vec<MigrationItemReport> {
    // IPC 层已校验 mode；核心对未知值防御性返回空报告（不执行任何文件系统动作）
    if mode != "migrate" && mode != "keep" {
        log::warn!("migrate_legacy_links 收到未知模式: {}", mode);
        return Vec::new();
    }
    detect_legacy_links(paths)
        .into_iter()
        .map(|name| {
            let entry = paths.agents_dir.join(&name);
            // 动作前重跑谓词：detect 与动作之间状态可能变化（TOCTOU）
            if !is_legacy_codex_link(&entry, &paths.layer2_root) {
                return MigrationItemReport {
                    name,
                    status: "skipped".to_string(),
                    detail: Some("链接已不存在或不再匹配迁移条件，跳过".to_string()),
                };
            }
            match mode {
                "migrate" => migrate_one(paths, name, &entry),
                _ => keep_one(paths, name, &entry),
            }
        })
        .collect()
}

/// 动作一（情景 A，spec §4.3）：在 `codex_skills_dir/<name>` 重建链接（target 仍指
/// Layer 2 原字面路径），再删 .agents 侧旧链。顺序必须先建新链后删旧链；
/// 单条失败不回滚其余（§6：该条计入报告，其余继续）。
fn migrate_one(paths: &MigrationPaths, name: String, agents_entry: &Path) -> MigrationItemReport {
    let codex_dest = paths.codex_skills_dir.join(&name);
    // 新链 target = .agents 旧链的单跳字面 target（Layer 2 原路径），启停链路不变（F5）
    let layer2_target = match std::fs::read_link(agents_entry) {
        Ok(t) => t,
        Err(e) => {
            return MigrationItemReport {
                name,
                status: "error".to_string(),
                detail: Some(format!("读取旧链接 target 失败: {}", e)),
            }
        }
    };
    // 断链自愈（review I-2，优先于冲突检查）：Layer 2 已被 disable/uninstall 删除 →
    // 迁移本应建的 .codex 链只会是悬空死链，不该创建；MAM 自建残链无论 codex 侧
    // 状态如何都应清理（否则永久滞留共享目录），报 ok（自熄灭）
    if std::fs::metadata(agents_entry).is_err() {
        if let Err(e) = linker::remove_link(agents_entry) {
            return MigrationItemReport {
                name,
                status: "error".to_string(),
                detail: Some(format!("断链清理失败: {}", e)),
            };
        }
        return MigrationItemReport {
            name,
            status: "ok".to_string(),
            detail: Some("断链清理：Layer 2 目标已不存在，仅移除 .agents 残链".to_string()),
        };
    }
    // 冲突：目标已存在（真目录/链接/文件均算）→ 跳过、旧链保留，由用户自行处置
    // （已知个案：手装真目录同名 frontend-design）
    if codex_dest.exists() || codex_dest.is_symlink() {
        // 竞态豁免（review C-1）：lib.rs 的后台导入线程与对话框并发，sync 补链可能在
        // 用户点击前就已在 .codex/skills 建好同 target 链接。此时 codex_dest 为链接且
        // 单跳 target 与旧链一致 → 等效迁移已达成，仅删旧链即可；否则升级机首启会
        // 全部误报冲突，.agents 清不掉、对话框每次启动重弹，自熄灭机制失效。
        // 真目录 / 异 target 链接仍维持冲突语义。
        if codex_dest.is_symlink() {
            let agents_parent = agents_entry.parent().unwrap_or_else(|| Path::new(""));
            let codex_parent = codex_dest.parent().unwrap_or_else(|| Path::new(""));
            if let (Ok(old_t), Ok(new_t)) = (
                std::fs::read_link(agents_entry),
                std::fs::read_link(&codex_dest),
            ) {
                if linker::normalize_link_target(&new_t, codex_parent)
                    == linker::normalize_link_target(&old_t, agents_parent)
                {
                    if let Err(e) = linker::remove_link(agents_entry) {
                        return MigrationItemReport {
                            name,
                            status: "error".to_string(),
                            detail: Some(format!("等效迁移已达成，但移除旧链失败: {}", e)),
                        };
                    }
                    return MigrationItemReport {
                        name,
                        status: "ok".to_string(),
                        detail: Some(
                            "后台补链已提前在 .codex/skills 建立同 target 链接，等效迁移达成，旧链已清理"
                                .to_string(),
                        ),
                    };
                }
            }
        }
        return MigrationItemReport {
            name,
            status: "skipped".to_string(),
            detail: Some(format!(
                "冲突：{} 已存在，.agents 旧链保留",
                codex_dest.display()
            )),
        };
    }
    // 先建新链
    if let Err(e) = linker::create_link(&layer2_target, &codex_dest) {
        return MigrationItemReport {
            name,
            status: "error".to_string(),
            detail: Some(format!("创建新链接失败: {}（旧链保留）", e)),
        };
    }
    // 后删旧链
    if let Err(e) = linker::remove_link(agents_entry) {
        return MigrationItemReport {
            name,
            status: "error".to_string(),
            detail: Some(format!("新链已建、旧链删除失败: {}", e)),
        };
    }
    MigrationItemReport {
        name,
        status: "ok".to_string(),
        detail: None,
    }
}

/// 动作二（情景 B，spec §4.3）：把 .agents 侧链接 target 从 Layer 2 改指 Layer 1
/// （实现为「删旧链 + 建新链」），从此脱钩 codex 启停。
fn keep_one(paths: &MigrationPaths, name: String, agents_entry: &Path) -> MigrationItemReport {
    let layer1 = paths.layer1_dir.join(&name);
    // 防御损坏态：Layer 1 目标缺失 → 跳过并报告（spec §4.3）
    if !layer1.exists() {
        return MigrationItemReport {
            name,
            status: "skipped".to_string(),
            detail: Some(format!("Layer 1 缺失：{} 不存在，跳过", layer1.display())),
        };
    }
    // 删旧链 + 建新链（任一步失败 → error，detail 带错误）
    if let Err(e) = linker::remove_link(agents_entry) {
        return MigrationItemReport {
            name,
            status: "error".to_string(),
            detail: Some(format!("移除旧链接失败: {}", e)),
        };
    }
    if let Err(e) = linker::create_link(&layer1, agents_entry) {
        return MigrationItemReport {
            name,
            status: "error".to_string(),
            detail: Some(format!("重建链接失败: {}", e)),
        };
    }
    MigrationItemReport {
        name,
        status: "ok".to_string(),
        detail: None,
    }
}

#[cfg(test)]
mod migration_tests {
    use super::*;

    /// fixture：tempdir 内构建三层链路（零真实家目录访问）：
    /// `layer1_dir/<name>`（真目录 + SKILL.md）← `layer2_root/<name>` ← `agents_dir/<name>`
    /// 返回 TempDir 保活句柄 + 注入路径
    fn setup(name: &str) -> (tempfile::TempDir, MigrationPaths) {
        let tmp = tempfile::tempdir().expect("创建临时目录失败");
        let paths = MigrationPaths {
            agents_dir: tmp.path().join("home/.agents/skills"),
            codex_skills_dir: tmp.path().join("home/.codex/skills"),
            layer1_dir: tmp.path().join("home/.mam/skills"),
            layer2_root: tmp.path().join("home/.mam/active/codex"),
        };
        // Layer 1 真目录
        let layer1_skill = paths.layer1_dir.join(name);
        std::fs::create_dir_all(&layer1_skill).unwrap();
        std::fs::write(
            layer1_skill.join("SKILL.md"),
            format!("---\nname: {}\n---\n正文", name),
        )
        .unwrap();
        // Layer 2 → Layer 1
        linker::create_link(&layer1_skill, &paths.layer2_root.join(name)).unwrap();
        // .agents → Layer 2（遗留 MAM 链接）
        std::fs::create_dir_all(&paths.agents_dir).unwrap();
        linker::create_link(&paths.layer2_root.join(name), &paths.agents_dir.join(name)).unwrap();
        (tmp, paths)
    }

    /// §7.3 谓词：遗留 MAM 链接 ✓ / 手装真目录 ✗ / 外部无关链接 ✗ / 断链 ✓（review I-2：
    /// disable/uninstall 会先行删掉 Layer 2，悬空遗留链接必须入选对话框，由 migrate 自愈，
    /// 否则永久滞留共享目录）
    #[test]
    fn predicate_four_states() {
        let (tmp, paths) = setup("my-skill");

        // 手装真目录（含 SKILL.md）✗：不是链接
        let handmade = paths.agents_dir.join("hand-made");
        std::fs::create_dir_all(&handmade).unwrap();
        std::fs::write(handmade.join("SKILL.md"), "---\nname: hand-made\n---\n").unwrap();

        // 外部无关链接 ✗：是链接但 target 不在 layer2_root 之下
        let elsewhere = tmp.path().join("elsewhere");
        std::fs::create_dir_all(&elsewhere).unwrap();
        linker::create_link(&elsewhere, &paths.agents_dir.join("unrelated")).unwrap();

        // 断链 ✓：agents 链 → layer2 链 → Layer 1，删掉 layer2 链接后 agents 链悬空，
        // 但单跳 target 字面前缀仍在 active/codex 下 → 入选（migrate 负责清理）
        linker::create_link(
            &paths.layer1_dir.join("my-skill"),
            &paths.layer2_root.join("ghost"),
        )
        .unwrap();
        linker::create_link(
            &paths.layer2_root.join("ghost"),
            &paths.agents_dir.join("broken"),
        )
        .unwrap();
        linker::remove_link(&paths.layer2_root.join("ghost")).unwrap();

        // 遗留 MAM 链接 + 断链遗留均命中（排序）
        assert_eq!(
            detect_legacy_links(&paths),
            vec!["broken".to_string(), "my-skill".to_string()],
            "遗留 MAM 链接与断链遗留均应命中，手装真目录与外部链接不命中"
        );
    }

    /// review I-2 断链自愈：悬空遗留链接入选后，migrate 仅清理 .agents 残链、
    /// 不建悬空 .codex 新链（Layer 2 已被 disable/uninstall 删除）
    #[test]
    fn migrate_self_heals_dangling_legacy_link() {
        let (_tmp, paths) = setup("my-skill");
        // 构造断链遗留（命名与现实一致：.agents/<技能名> → active/codex/<技能名>，
        // 随后 Layer 2 被 disable/uninstall 删除）：
        // layer1_dir/dangling-skill（真目录）← layer2_root/dangling-skill ← agents/dangling-skill
        let layer1_dangling = paths.layer1_dir.join("dangling-skill");
        std::fs::create_dir_all(&layer1_dangling).unwrap();
        linker::create_link(&layer1_dangling, &paths.layer2_root.join("dangling-skill")).unwrap();
        linker::create_link(
            &paths.layer2_root.join("dangling-skill"),
            &paths.agents_dir.join("dangling-skill"),
        )
        .unwrap();
        linker::remove_link(&paths.layer2_root.join("dangling-skill")).unwrap();
        assert_eq!(
            detect_legacy_links(&paths),
            vec!["dangling-skill".to_string(), "my-skill".to_string()]
        );

        let reports = migrate_legacy_links(&paths, "migrate");
        let dangling = reports.iter().find(|r| r.name == "dangling-skill").unwrap();
        assert_eq!(dangling.status, "ok", "报告: {:?}", dangling);
        assert!(dangling.detail.as_deref().unwrap().contains("断链清理"));

        // 悬空残链已清、未在 .codex/skills 创建悬空新链、自熄灭
        assert!(!paths.agents_dir.join("dangling-skill").is_symlink());
        assert!(!paths.codex_skills_dir.join("dangling-skill").exists());
        assert!(detect_legacy_links(&paths).is_empty());
    }

    /// review I-2：断链项选「保留为共享」且 Layer 1 仍在 → 改指 Layer 1（脱钩 codex）
    #[test]
    fn keep_repoints_dangling_legacy_link_to_layer1() {
        let (_tmp, paths) = setup("my-skill");
        let layer1_dangling = paths.layer1_dir.join("dangling-skill");
        std::fs::create_dir_all(&layer1_dangling).unwrap();
        linker::create_link(&layer1_dangling, &paths.layer2_root.join("dangling-skill")).unwrap();
        linker::create_link(
            &paths.layer2_root.join("dangling-skill"),
            &paths.agents_dir.join("dangling-skill"),
        )
        .unwrap();
        linker::remove_link(&paths.layer2_root.join("dangling-skill")).unwrap();

        let reports = migrate_legacy_links(&paths, "keep");
        let dangling = reports.iter().find(|r| r.name == "dangling-skill").unwrap();
        // Layer 1（~/.mam/skills/dangling-skill）仍在 → keep 改指 Layer 1 成功
        assert_eq!(dangling.status, "ok", "报告: {:?}", dangling);
        assert!(paths.agents_dir.join("dangling-skill").is_symlink());
        assert_eq!(
            std::fs::read_link(&paths.agents_dir.join("dangling-skill")).unwrap(),
            layer1_dangling
        );
    }

    /// §7.4 migrate 全链路：codex 侧新链指向 Layer 2 字面路径、.agents 旧链消失、报告 ok
    #[test]
    fn migrate_relinks_codex_and_removes_agents_link() {
        let (_tmp, paths) = setup("my-skill");
        assert_eq!(detect_legacy_links(&paths), vec!["my-skill".to_string()]);

        let reports = migrate_legacy_links(&paths, "migrate");
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].name, "my-skill");
        assert_eq!(reports[0].status, "ok", "报告: {:?}", reports[0]);

        // codex 侧新链存在，target == 原 Layer 2 字面路径（单跳，未穿透解析到 Layer 1）
        let codex_dest = paths.codex_skills_dir.join("my-skill");
        assert!(codex_dest.is_symlink(), "codex 侧应为链接");
        assert_eq!(
            std::fs::read_link(&codex_dest).unwrap(),
            paths.layer2_root.join("my-skill")
        );
        // .agents 旧链已删（exists 与 is_symlink 双判，防悬空链漏检）
        assert!(!paths.agents_dir.join("my-skill").exists());
        assert!(!paths.agents_dir.join("my-skill").is_symlink());
        // 自熄灭：迁移后谓词无匹配
        assert!(detect_legacy_links(&paths).is_empty());
    }

    /// §7.5 keep：target 改指 Layer 1、.agents 侧链接仍在、报告 ok
    #[test]
    fn keep_repoints_agents_link_to_layer1() {
        let (_tmp, paths) = setup("my-skill");

        let reports = migrate_legacy_links(&paths, "keep");
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].status, "ok", "报告: {:?}", reports[0]);

        let entry = paths.agents_dir.join("my-skill");
        assert!(entry.is_symlink(), ".agents 侧链接应保留");
        assert_eq!(
            std::fs::read_link(&entry).unwrap(),
            paths.layer1_dir.join("my-skill"),
            "target 应改指 Layer 1"
        );
        // 自熄灭：target 前缀已是 Layer 1，不再命中 active/codex 谓词
        assert!(detect_legacy_links(&paths).is_empty());
    }

    /// review C-1 竞态：启动后台补链线程可能抢在用户点击前于 .codex/skills 建好
    /// 同 target 链接——必须视为「等效迁移已达成」（删旧链、报 ok），而非误报冲突。
    /// 否则升级机首启全部 skipped、对话框每次启动重弹，自熄灭失效
    #[test]
    fn migrate_treats_same_target_codex_link_as_equivalent() {
        let (_tmp, paths) = setup("my-skill");
        // 模拟后台补链抢先：codex 侧已存在同 target 链接（→ Layer 2 字面路径）
        let codex_dest = paths.codex_skills_dir.join("my-skill");
        std::fs::create_dir_all(&paths.codex_skills_dir).unwrap();
        linker::create_link(&paths.layer2_root.join("my-skill"), &codex_dest).unwrap();

        let reports = migrate_legacy_links(&paths, "migrate");
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].status, "ok", "报告: {:?}", reports[0]);

        // .agents 旧链已清（自熄灭），codex 链接保持且 target 不变
        assert!(!paths.agents_dir.join("my-skill").is_symlink());
        assert!(codex_dest.is_symlink());
        assert_eq!(
            std::fs::read_link(&codex_dest).unwrap(),
            paths.layer2_root.join("my-skill")
        );
        assert!(detect_legacy_links(&paths).is_empty());
    }

    /// review C-1 反例：codex 侧已存在链接但 target 不同（指向其他位置）→
    /// 仍按冲突跳过、旧链保留
    #[test]
    fn migrate_still_conflicts_on_different_target_link() {
        let (_tmp, paths) = setup("my-skill");
        let codex_dest = paths.codex_skills_dir.join("my-skill");
        std::fs::create_dir_all(&paths.codex_skills_dir).unwrap();
        let elsewhere = _tmp.path().join("elsewhere-target");
        std::fs::create_dir_all(&elsewhere).unwrap();
        linker::create_link(&elsewhere, &codex_dest).unwrap();

        let reports = migrate_legacy_links(&paths, "migrate");
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].status, "skipped", "报告: {:?}", reports[0]);
        assert!(reports[0].detail.as_deref().unwrap().contains("冲突"));

        // 旧链保留、codex 异 target 链接未被动过
        assert!(paths.agents_dir.join("my-skill").is_symlink());
        assert_eq!(std::fs::read_link(&codex_dest).unwrap(), elsewhere);
    }

    /// §7.6 同名冲突：codex 侧预置真目录 → skipped（detail 注明冲突）、旧链保留、真目录未破坏
    #[test]
    fn migrate_skips_on_codex_name_conflict() {
        let (_tmp, paths) = setup("my-skill");
        // 预置 codex 侧同名真目录（已知个案：手装 frontend-design）
        let dest = paths.codex_skills_dir.join("my-skill");
        std::fs::create_dir_all(&dest).unwrap();
        std::fs::write(dest.join("SKILL.md"), "手装内容").unwrap();

        let reports = migrate_legacy_links(&paths, "migrate");
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].status, "skipped");
        assert!(
            reports[0]
                .detail
                .as_deref()
                .unwrap_or_default()
                .contains("已存在"),
            "detail 应注明冲突: {:?}",
            reports[0]
        );
        // .agents 旧链保留且仍指向 Layer 2
        let entry = paths.agents_dir.join("my-skill");
        assert!(entry.is_symlink());
        assert_eq!(
            std::fs::read_link(&entry).unwrap(),
            paths.layer2_root.join("my-skill")
        );
        // codex 真目录未被破坏
        assert_eq!(
            std::fs::read_to_string(dest.join("SKILL.md")).unwrap(),
            "手装内容"
        );
    }

    /// §6 补充：agents_dir 不存在 → detect 空结果、migrate 空报告（静默不报错）
    #[test]
    fn missing_agents_dir_is_silent() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = MigrationPaths {
            agents_dir: tmp.path().join("no-such-agents/skills"),
            codex_skills_dir: tmp.path().join("home/.codex/skills"),
            layer1_dir: tmp.path().join("home/.mam/skills"),
            layer2_root: tmp.path().join("home/.mam/active/codex"),
        };
        assert!(detect_legacy_links(&paths).is_empty());
        assert!(migrate_legacy_links(&paths, "migrate").is_empty());
        assert!(migrate_legacy_links(&paths, "keep").is_empty());
    }

    /// §6 补充：keep 模式 Layer 1 缺失 → skipped（spec §4.3 防御损坏态）、agents 链原样保留。
    /// fixture：Layer 2 改指他处真目录、Layer 1 无同名条目——链路仍可达（谓词命中），
    /// 但 Layer 1 目标不存在
    #[test]
    fn keep_skips_when_layer1_missing() {
        let (tmp, paths) = setup("my-skill");
        let orphan = tmp.path().join("orphan-ssot");
        std::fs::create_dir_all(&orphan).unwrap();
        linker::remove_link(&paths.layer2_root.join("my-skill")).unwrap();
        linker::create_link(&orphan, &paths.layer2_root.join("my-skill")).unwrap();
        std::fs::remove_dir_all(paths.layer1_dir.join("my-skill")).unwrap();

        let reports = migrate_legacy_links(&paths, "keep");
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].status, "skipped");
        assert!(
            reports[0]
                .detail
                .as_deref()
                .unwrap_or_default()
                .contains("Layer 1"),
            "detail 应注明 Layer 1 缺失: {:?}",
            reports[0]
        );
        // agents 链接原样保留（仍指向 Layer 2）
        let entry = paths.agents_dir.join("my-skill");
        assert!(entry.is_symlink());
        assert_eq!(
            std::fs::read_link(&entry).unwrap(),
            paths.layer2_root.join("my-skill")
        );
    }
}
