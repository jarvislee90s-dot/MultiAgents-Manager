// 只读 `~/.claude.json` 信任库小工具（T3 终审发现 B：重开 cwd 信任归一）。
//
// 背景：claude 按 cwd 的**精确字符串**在 `~/.claude.json` 的 `projects` 键里查目录
// 信任（`hasTrustDialogAccepted`），而该键存储对盘符大小写/分隔符**脆弱**——实证
// 场景同目录双条并存：`E:/…Test2 = false` 与 `e:/…Test2 = true`。兔维斯 远程重开以
// 会话记录的 cwd（大写 `E:\…`）spawn → claude 查到 false 条款 → 弹**交互式信任
// TUI**（手机注入答不了）→ 重开挂起。
//
// 修法（决策 5）：spawn 前把 cwd 与 projects 键做大小写不敏感 + 分隔符归一匹配；
// 命中多条时**优先取 `hasTrustDialogAccepted=true` 条款**复用其精确 casing（命中
// false 条款则归一白做）；全 false / 未命中 → 原样（全新目录的首次信任属正常流程）。
//
// **红线：兔维斯 只读该文件，永不写**（写面完全归 claude 本体；本模块无任何写路径）。

/// 归一 projects 键 / cwd 用于匹配（**纯函数**）：
/// - `\` 与 `/` 等价（claude 键存正斜杠形态，会话记录可能是反斜杠）；
/// - 去尾部路径分隔符；
/// - 整体转小写——盘符/路径大小写在两支持平台（Windows NTFS / macOS APFS 默认）
///   文件系统均不敏感，而 claude 自己的键存储恰是大小写脆弱面（本 bug 根因），
///   匹配必须跨大小写折叠。
fn normalize_project_key(p: &str) -> String {
    p.trim_end_matches(['/', '\\'])
        .replace('\\', "/")
        .to_lowercase()
}

/// 只读解析 `~/.claude.json` 并以闭包摘取产物。文件缺失 / JSON 畸形 / 读失败 →
/// None（调用方一律按「无信息」保守降级：cwd 原样、不提醒）。**只读，永不写**。
pub(crate) fn read_with<R>(
    claude_json: &std::path::Path,
    f: impl FnOnce(&serde_json::Value) -> R,
) -> Option<R> {
    let raw = std::fs::read_to_string(claude_json).ok()?;
    let v: serde_json::Value = serde_json::from_str(&raw).ok()?;
    Some(f(&v))
}

/// 内部：收集与 cwd 归一匹配的 projects 条款 `(精确键, hasTrustDialogAccepted)`。
/// None = 配置不可读/无 projects 键（与「零命中」区分——零命中 = Some(空表)）。
/// 条款缺 `hasTrustDialogAccepted` 字段按未信任计（claude 默认未放行）。
fn project_trust_hits(
    claude_json: &std::path::Path,
    cwd: &std::path::Path,
) -> Option<Vec<(String, bool)>> {
    read_with(claude_json, |v| {
        let projects = v.get("projects")?.as_object()?;
        let want = normalize_project_key(&cwd.to_string_lossy());
        let hits: Vec<(String, bool)> = projects
            .iter()
            .filter(|(k, _)| normalize_project_key(k) == want)
            .map(|(k, val)| {
                let trusted = val
                    .get("hasTrustDialogAccepted")
                    .and_then(|t| t.as_bool())
                    .unwrap_or(false);
                (k.clone(), trusted)
            })
            .collect();
        Some(hits)
    })?
}

/// 命中条款中**已信任条款的精确 casing**（多命中择优 = 优先
/// `hasTrustDialogAccepted=true`；实证场景恰是 `E:`=false / `e:`=true 并存——
/// 复用 true 条款 casing 后 claude 查信任即命中，不再弹窗）。
/// 全 false / 未命中 / 配置不可读 → None（cwd 原样）。
pub(crate) fn trusted_casing_for(home: &std::path::Path, cwd: &std::path::Path) -> Option<String> {
    let hits = project_trust_hits(&home.join(".claude.json"), cwd)?;
    hits.iter().find(|(_, t)| *t).map(|(k, _)| k.clone())
}

/// 命中条款的信任值：`Some(true)` = 命中且存在已信任条款；`Some(false)` = 命中但
/// 全部未信任（**预检提醒触发条件**——重开会挂信任弹窗）；`None` = 无条目/配置
/// 不可读（全新目录首次信任属正常流程，不提醒）。
pub(crate) fn is_trusted(home: &std::path::Path, cwd: &std::path::Path) -> Option<bool> {
    let hits = project_trust_hits(&home.join(".claude.json"), cwd)?;
    if hits.is_empty() {
        return None;
    }
    Some(hits.iter().any(|(_, t)| *t))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// 写 tempdir 假 `~/.claude.json`（**永不触真实文件**）：
    /// projects 以 `entries` 原样落盘（键大小写/形态逐字保留——本模块的被测面）
    fn fake_home(entries: &[(&str, bool)]) -> (tempfile::TempDir, std::path::PathBuf) {
        let home = tempfile::tempdir().unwrap();
        let json = format!(
            r#"{{"projects":{{{}}},"numStartups":7}}"#,
            entries
                .iter()
                .map(|(k, t)| format!(r#""{k}":{{"hasTrustDialogAccepted":{t}}}"#))
                .collect::<Vec<_>>()
                .join(",")
        );
        std::fs::write(home.path().join(".claude.json"), json).unwrap();
        let path = home.path().to_path_buf();
        (home, path)
    }

    /// 归一纯函数：分隔符方向 / 盘符大小写 / 尾部分隔符三面折叠
    #[test]
    fn normalize_project_key_unifies_separators_case_and_trailing() {
        // 反斜杠 → 正斜杠；尾部 `\` 与 `/` 同去
        assert_eq!(
            normalize_project_key(r"E:\LLMproject\Test2\"),
            "e:/llmproject/test2"
        );
        assert_eq!(
            normalize_project_key("E:/LLMproject/Test2/"),
            "e:/llmproject/test2"
        );
        // 尾部无分隔符同形
        assert_eq!(
            normalize_project_key(r"E:\LLMproject\Test2"),
            "e:/llmproject/test2"
        );
        // 盘符大小写折叠（实证 bug 面：E: 与 e: 双条并存）
        assert_eq!(normalize_project_key("e:/x"), normalize_project_key("E:/X"));
        // 根与空串
        assert_eq!(normalize_project_key("/"), "");
        assert_eq!(normalize_project_key(""), "");
        // Unix 形态路径同样可用
        assert_eq!(normalize_project_key("/tmp/proj-m/"), "/tmp/proj-m");
    }

    /// 实证形态（终审发现 B 现场）：双 casing 条款一真一假并存 → 择优取**真条款**
    /// 的精确 casing；is_trusted=Some(true)（复用后不再弹窗）
    #[test]
    fn dual_casing_entries_prefer_trusted_casing() {
        let (_home, home) = fake_home(&[
            (r"E:/LLMproject/Test2", false), // 大写条款：未信任（实证的挂起点）
            ("e:/LLMproject/Test2", true),   // 小写条款：已信任
        ]);
        // 查询 cwd 用会话记录形态（反斜杠 + 大写盘符）——归一后命中双条款
        let cwd = Path::new(r"E:\LLMproject\Test2");
        assert_eq!(
            trusted_casing_for(&home, cwd).as_deref(),
            Some("e:/LLMproject/Test2"),
            "必须复用真条款的精确 casing（逐字）"
        );
        assert_eq!(is_trusted(&home, cwd), Some(true));
    }

    /// 命中但全部未信任 → trusted_casing_for=None（无可复用 casing，cwd 原样）；
    /// is_trusted=Some(false)——**预检提醒触发条件被锁定**
    #[test]
    fn untrusted_entry_flags_reminder() {
        let (_home, home) = fake_home(&[("E:/LLMproject/Test2", false)]);
        let cwd = Path::new(r"E:\LLMproject\Test2");
        assert_eq!(trusted_casing_for(&home, cwd), None);
        assert_eq!(is_trusted(&home, cwd), Some(false), "Some(false)=提醒触发");
    }

    /// 条款缺 hasTrustDialogAccepted 字段 → 按未信任计（claude 默认未放行）
    #[test]
    fn entry_without_trust_field_counts_untrusted() {
        let home = tempfile::tempdir().unwrap();
        std::fs::write(
            home.path().join(".claude.json"),
            r#"{"projects":{"/tmp/proj":{}}}"#,
        )
        .unwrap();
        assert_eq!(
            is_trusted(home.path(), Path::new(r"\tmp\proj")),
            Some(false)
        );
    }

    /// 无条目 / 无 projects 键 / 文件缺失 → None 三面（全新目录首次信任属正常
    /// 流程，不提醒）
    #[test]
    fn no_entry_is_none() {
        // 文件缺失（空 tempdir home）
        let home = tempfile::tempdir().unwrap();
        assert_eq!(trusted_casing_for(home.path(), Path::new("/tmp/x")), None);
        assert_eq!(is_trusted(home.path(), Path::new("/tmp/x")), None);
        // 文件在但无 projects 键 / projects 非对象
        let home2 = tempfile::tempdir().unwrap();
        std::fs::write(home2.path().join(".claude.json"), r#"{"numStartups":3}"#).unwrap();
        assert_eq!(is_trusted(home2.path(), Path::new("/tmp/x")), None);
        // 有 projects 但无该目录条目
        let (_h3, home3) = fake_home(&[("/other/proj", true)]);
        assert_eq!(trusted_casing_for(&home3, Path::new("/tmp/x")), None);
        assert_eq!(is_trusted(&home3, Path::new("/tmp/x")), None);
    }

    /// JSON 畸形 → 优雅 None（保守降级：cwd 原样、不提醒），不 panic 不写文件
    #[test]
    fn malformed_json_is_graceful_none() {
        let home = tempfile::tempdir().unwrap();
        std::fs::write(home.path().join(".claude.json"), "{projects: 残缺").unwrap();
        assert_eq!(trusted_casing_for(home.path(), Path::new("/tmp/x")), None);
        assert_eq!(is_trusted(home.path(), Path::new("/tmp/x")), None);
        // read_with 本体：畸形 → None
        assert!(read_with(&home.path().join(".claude.json"), |_| ()).is_none());
    }

    /// read_with 摘取面：合法文件闭包可见全量 Value（projects 之外的字段共存——
    /// 真实 .claude.json 是大杂烩，只摘 projects 不碰其余）
    #[test]
    fn read_with_exposes_parsed_value() {
        let (_h, home) = fake_home(&[("/tmp/p", true)]);
        let seen = read_with(&home.join(".claude.json"), |v| {
            (
                v.get("numStartups").and_then(|n| n.as_u64()),
                v.get("projects").is_some(),
            )
        })
        .unwrap();
        assert_eq!(seen, (Some(7), true));
    }
}
