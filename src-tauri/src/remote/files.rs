// 文件安全读取与路径提取（M3 Task 8，C2 后端）
//
// 两条职责：
// - `extract_file_paths`：从会话消息流（复用 content::read_session_messages，八工具
//   统一出口）的 tool-call 条目里提取涉及的文件路径——**泛化实现，不做 per-tool
//   提取器**（控制者裁决 2026-09-15）：递归走 toolArgs JSON 树，收集
//   file_path / path / filename / abs_path（+ Task 9 实测补键 filePath / file，
//   证据见 task-9-report.md）键下的字符串值（须含路径分隔符或 `.扩展名` 才算
//   路径候选），去重保序。对八工具统一生效；各工具真实 toolArgs 形态由
//   extract_chain_* 系列 fixture 测试锁定。
// - `read_file_safe`：文件预览的安全读取内核——canonicalize 双方后限定在会话 cwd
//   之内（越界拒绝）、只读、双阈值大小上限（图片扩展名 5MB / 其余 500KB，按
//   Global Constraints 的 mime 分支）、按扩展名给 MIME。
//
// Windows 注意：`canonicalize` 返回带 `\\?\`（及 `\\?\UNC\`）前缀的 verbatim 路径，
// 与字面 cwd 直接 starts_with 会因前缀不匹配误判越界——比较前统一剥前缀、统一
// 分隔符；Windows 文件系统大小写不敏感，语义分支下整体转小写（linker::detector
// 的 strip_verbatim_prefix / path_starts_with_ci 同款口径，因彼处私有且属 linker
// 域，此处以内聚小助手复刻，见 path_within_semantics）。
//
// 测试约束（宪法级）：一律 tempdir，绝不触真实 ~/.zcode ~/.claude 等数据目录；
// Windows 语义分支以 `windows: bool` 参数注入，darwin 上也可全分支测试。

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use super::content::SessionMessage;

/// 文件路径源函数形态（RemoteState 注入缝的类型别名，生产 = extract_file_paths）
pub type PathSourceFn = dyn Fn(&str, &str) -> Vec<String> + Send + Sync;

/// 文本类文件大小上限：500KB（Global Constraints）
const MAX_TEXT_BYTES: u64 = 500 * 1024;
/// 图片类文件大小上限：5MB（Global Constraints，mime 分支）
const MAX_IMAGE_BYTES: u64 = 5 * 1024 * 1024;
/// 路径提取的消息读取尾窗：与移动端详情页默认 limit 同量级（按需单会话读取，
/// 非 3s 轮询路径；更早的工具调用不回捞，M3 接受为已知范围）
const EXTRACT_MESSAGE_LIMIT: usize = 200;
/// 递归收集的参数键集合（小写精确匹配）。基础四键 `file_path`/`path`/`filename`/
/// `abs_path` 为控制者裁决；Task 9 按各工具真实 toolArgs 形态探测补两键（证据见
/// task-9-report.md）：`filePath` 驼峰——OpenCode 官方工具 schema（edit/write/read，
/// 本机库无 tool part 样本，文档级证据）；`file`——Kimi 旧版 Read 参数（本机
/// wire.jsonl 实测 3 例绝对路径）。键名单对八工具统一生效，不做 per-tool 分支；
/// 误收风险由 is_path_candidate 候选判定（URL/纯词/换行排除）兜底
const PATH_KEYS: &[&str] = &[
    "file_path",
    "path",
    "filename",
    "abs_path",
    "filePath",
    "file",
];
/// 单个路径候选的长度上限（超长串不是可预览文件，纯防御）
const MAX_PATH_LEN: usize = 4096;

// ============================================================
// 安全读取（read_file_safe）
// ============================================================

/// 安全读取文件（限会话 cwd、只读、双阈值大小上限）。
/// 返回 (字节, mime)。任何拒绝一律 Err（调用方 file 端点统一 403，不外泄区别）。
/// `path` 支持绝对路径与相对路径（相对者按会话 cwd 解析——部分工具 toolArgs
/// 记录项目内相对路径）。
pub fn read_file_safe(session_cwd: &str, path: &str) -> Result<(Vec<u8>, String), String> {
    if session_cwd.trim().is_empty() {
        return Err("会话 cwd 为空".into());
    }
    if path.trim().is_empty() {
        return Err("文件路径为空".into());
    }
    let cwd = Path::new(session_cwd)
        .canonicalize()
        .map_err(|e| format!("cwd 不可解析: {e}"))?;
    let mut full = PathBuf::from(path.trim());
    if full.is_relative() {
        full = cwd.join(full);
    }
    let canon = full
        .canonicalize()
        .map_err(|e| format!("文件不可解析: {e}"))?;
    if !path_within(&canon, &cwd) {
        return Err("路径越界：文件不在项目目录内".into());
    }
    let meta = canon.metadata().map_err(|e| format!("元数据不可读: {e}"))?;
    if !meta.is_file() {
        return Err("不是常规文件".into());
    }
    let ext = canon.extension().and_then(|e| e.to_str()).unwrap_or("");
    let mime = mime_from_ext(ext);
    // 双阈值（进度台账 #11 裁决）：图片扩展名 5MB / 其余 500KB
    let max = if mime.starts_with("image/") {
        MAX_IMAGE_BYTES
    } else {
        MAX_TEXT_BYTES
    };
    if meta.len() > max {
        return Err(format!("文件过大: {} bytes（上限 {max}）", meta.len()));
    }
    let bytes = std::fs::read(&canon).map_err(|e| format!("读取失败: {e}"))?;
    Ok((bytes, mime))
}

/// 路径包含判定（canonicalize 之后的双方）：Windows 语义自动分派
fn path_within(child: &Path, ancestor: &Path) -> bool {
    path_within_semantics(
        &child.to_string_lossy(),
        &ancestor.to_string_lossy(),
        cfg!(windows),
    )
}

/// 路径包含判定内核（字符串级、平台语义可注入——darwin 上可测全分支）：
/// - 剥 Windows verbatim 前缀（`\\?\UNC\` → `\\`、`\\?\` → 空），统一分隔符为 `/`，
///   去尾部分隔符；Windows 语义整体转小写（文件系统大小写不敏感）；
/// - 前缀命中后剩余必须为空或以 `/` 开头（`/ab` 不在 `/a` 之下——分隔符边界）；
/// - 祖先为根（`/`）时任意绝对路径都包含（根即一切）。
fn path_within_semantics(child: &str, ancestor: &str, windows: bool) -> bool {
    let norm = |s: &str| -> String {
        // 剥 verbatim 前缀（UNC 形态还原为 `\\server\share` 常规形态再统一分隔符）
        let s = if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
            format!(r"\\{rest}")
        } else if let Some(rest) = s.strip_prefix(r"\\?\") {
            rest.to_string()
        } else {
            s.to_string()
        };
        let mut s = s.replace('\\', "/");
        while s.len() > 1 && s.ends_with('/') {
            s.pop();
        }
        if windows {
            s.to_lowercase()
        } else {
            s
        }
    };
    let c = norm(child);
    let a = norm(ancestor);
    if a.is_empty() {
        return false;
    }
    if a == "/" {
        return c.starts_with('/');
    }
    match c.strip_prefix(&a) {
        Some(rest) => rest.is_empty() || rest.starts_with('/'),
        None => false,
    }
}

/// 按扩展名给 MIME（预览渲染分支依据；未知扩展回落 text/plain——按纯文本
/// 渲染比误标 image/html 安全）
fn mime_from_ext(ext: &str) -> String {
    let lower = ext.to_ascii_lowercase();
    match lower.as_str() {
        "md" => "text/markdown",
        "rs" => "text/rust",
        "ts" | "tsx" => "text/typescript",
        "js" | "mjs" | "cjs" => "text/javascript",
        "jsx" => "text/jsx",
        "py" => "text/python",
        "go" => "text/go",
        "java" => "text/java",
        "c" | "h" => "text/c",
        "cpp" | "cc" | "hpp" => "text/cpp",
        "css" => "text/css",
        "sh" => "text/shell",
        "toml" => "text/toml",
        "yaml" | "yml" => "text/yaml",
        "json" => "application/json",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        _ => "text/plain",
    }
    .to_string()
}

// ============================================================
// 路径提取（extract_file_paths）
// ============================================================

/// 生产薄壳：真实 home + env 重定向 → 注入核（zcode_home_with 先例）。
/// env 双参（DSH_HOME / KIMI_CODE_HOME）与 /session-messages 生产薄壳**同一归口**
/// （content::read_env_homes，终审 Important 1）——设了 env 的机器上文件面板与
/// 详情页消息同源，不再静默空表。提取失败（存储不可读 / 会话不存在）一律返回
/// 空表——文件面板是增强能力，不因提取失败阻塞详情页。
pub fn extract_file_paths(agent_type: &str, session_id: &str) -> Vec<String> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    let (dsh_env, kimi_env) = super::content::read_env_homes();
    extract_file_paths_with_env(
        &home,
        dsh_env.as_deref(),
        kimi_env.as_deref(),
        agent_type,
        session_id,
    )
}

/// 注入核（测试直调 tempdir home，零真实数据目录接触；env 重定向恒 None）
pub fn extract_file_paths_with(home: &Path, agent_type: &str, session_id: &str) -> Vec<String> {
    extract_file_paths_with_env(home, None, None, agent_type, session_id)
}

/// env 注入核（终审 Important 1）：与 content::read_session_messages_impl 同款 env
/// 双参——dsh/kimi 数据根重定向可注入（env 值作参，测试锁重定向链路，零真实 env
/// 接触）。生产薄壳不走 None：extract_file_paths 经 content::read_env_homes 读真实值
pub(crate) fn extract_file_paths_with_env(
    home: &Path,
    dsh_env_home: Option<&str>,
    kimi_env_home: Option<&str>,
    agent_type: &str,
    session_id: &str,
) -> Vec<String> {
    // 复用 Task 7 的八工具统一出口（数据同源：与详情页读的是同一份消息流），
    // 不另立 per-tool 查询（控制者裁决）；派发核 pub(crate) 同源复用
    match super::content::read_session_messages_impl(
        home,
        dsh_env_home,
        kimi_env_home,
        agent_type,
        session_id,
        EXTRACT_MESSAGE_LIMIT,
    ) {
        Ok(pg) => extract_paths_from_messages(&pg.messages),
        Err(_) => Vec::new(),
    }
}

/// 从统一消息流提取文件路径（纯函数）：只看 tool-call 条目的 toolArgs JSON，
/// 递归收集 PATH_KEYS 键下的字符串值，去重保序。
pub fn extract_paths_from_messages(msgs: &[SessionMessage]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for m in msgs {
        if m.kind != "tool-call" {
            continue;
        }
        let Some(args) = &m.tool_args else { continue };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(args) else {
            continue; // 参数损坏 → 跳过该条（防御）
        };
        collect_path_values(&v, &mut out, &mut seen);
    }
    out
}

/// 递归走 JSON 树：对象键命中 PATH_KEYS 时收字符串值，其余结构下钻
fn collect_path_values(v: &serde_json::Value, out: &mut Vec<String>, seen: &mut HashSet<String>) {
    match v {
        serde_json::Value::Object(map) => {
            for (k, val) in map {
                if PATH_KEYS.contains(&k.as_str()) {
                    if let Some(s) = val.as_str() {
                        if is_path_candidate(s) && seen.insert(s.to_string()) {
                            out.push(s.to_string());
                        }
                        continue; // 命中键的字符串值已收，不再下钻
                    }
                    // 命中键的非字符串值（对象/数组包裹形态）继续下钻
                }
                collect_path_values(val, out, seen);
            }
        }
        serde_json::Value::Array(items) => {
            for it in items {
                collect_path_values(it, out, seen);
            }
        }
        _ => {}
    }
}

/// 路径候选判定（控制者裁决口径）：含路径分隔符**或**带 `.扩展名`；排除空串 /
/// 超长 / 含换行与 NUL / URL（`://`，MCP 参数常见，非本地文件）。
fn is_path_candidate(s: &str) -> bool {
    let s = s.trim();
    if s.is_empty() || s.len() > MAX_PATH_LEN {
        return false;
    }
    if s.contains(['\n', '\r', '\0']) {
        return false;
    }
    if s.contains("://") {
        return false;
    }
    s.contains('/') || s.contains('\\') || has_ext_like(s)
}

/// 形如带扩展名的文件名：末段含 `.字母数字后缀`（1-12 位）。
/// 让 "README.md" / "Cargo.toml" 这类裸文件名也算候选（read_file_safe 会按
/// 会话 cwd 解析相对路径）
fn has_ext_like(s: &str) -> bool {
    let last = s.rsplit(['/', '\\']).next().unwrap_or(s);
    match last.rsplit_once('.') {
        Some((stem, ext)) => {
            !stem.is_empty()
                && !ext.is_empty()
                && ext.len() <= 12
                && ext.chars().all(|c| c.is_ascii_alphanumeric())
        }
        None => false,
    }
}

// ============================================================
// 测试（Task 8 Step 1）：一律 tempdir / 合成数据，零真实数据目录接触。
// Windows 语义以参数注入，darwin 上全分支可测。
// ============================================================
#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote::content::SessionMessage;

    /// content.rs 的构造器是私有的——测试本地直接按字段构造（字段全 pub，勿改生产可见性）
    fn tool_call(args: &str) -> SessionMessage {
        SessionMessage {
            seq: 1,
            role: "assistant".into(),
            kind: "tool-call".into(),
            content: "调用 Write".into(),
            ts: Some(1),
            tool_name: Some("Write".into()),
            tool_args: Some(args.into()),
            collapsed: true,
        }
    }

    fn text_msg(kind: &str, content: &str) -> SessionMessage {
        SessionMessage {
            seq: 0,
            role: if kind == "user" { "user" } else { "assistant" }.into(),
            kind: kind.into(),
            content: content.into(),
            ts: Some(1),
            tool_name: None,
            tool_args: None,
            collapsed: kind == "thinking",
        }
    }

    // ==== read_file_safe ====

    #[test]
    fn read_file_safe_allows_file_inside_cwd() {
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("hello.txt");
        std::fs::write(&f, "hello mam").unwrap();
        let (bytes, mime) =
            read_file_safe(tmp.path().to_str().unwrap(), f.to_str().unwrap()).unwrap();
        assert_eq!(bytes, b"hello mam");
        assert_eq!(mime, "text/plain");
    }

    #[test]
    fn read_file_safe_resolves_relative_path_against_cwd() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("src")).unwrap();
        std::fs::write(tmp.path().join("src").join("m.rs"), "fn main() {}").unwrap();
        let (bytes, mime) = read_file_safe(tmp.path().to_str().unwrap(), "src/m.rs").unwrap();
        assert_eq!(bytes, b"fn main() {}");
        assert_eq!(mime, "text/rust");
    }

    #[test]
    fn read_file_safe_rejects_escape_and_missing_and_dir() {
        // 布局：outer/{proj(cwd), secret.txt}——越界目标须真实存在于 cwd 外
        let outer = tempfile::tempdir().unwrap();
        let proj = outer.path().join("proj");
        std::fs::create_dir_all(&proj).unwrap();
        let secret = outer.path().join("secret.txt");
        std::fs::write(&secret, "x").unwrap();
        let cwd = proj.to_str().unwrap();
        // (1) 越界：../ 逃出 cwd（按会话 cwd 解析相对路径后仍指向 cwd 外真实文件）
        let r = read_file_safe(cwd, "../secret.txt");
        assert!(r.unwrap_err().contains("越界"));
        // (2) 绝对路径越界（cwd 外真实文件）
        let r = read_file_safe(cwd, secret.to_str().unwrap());
        assert!(r.unwrap_err().contains("越界"));
        // (3) 不存在
        let r = read_file_safe(cwd, "nope.txt");
        assert!(r.is_err(), "不存在的文件必须拒绝");
        // (4) 目录（canonicalize 成功但非常规文件）
        let r = read_file_safe(cwd, ".");
        assert!(r.is_err(), "目录必须拒绝");
        // (5) 空参
        assert!(read_file_safe("", "x").is_err());
        assert!(read_file_safe(cwd, "  ").is_err());
    }

    /// 双阈值（进度台账 #11）：文本 500KB 上限、图片 5MB 上限——
    /// 各造阈值两侧样本，图片在「文本会拒」的体量下必须放行
    #[test]
    fn read_file_safe_enforces_dual_size_thresholds() {
        let tmp = tempfile::tempdir().unwrap();
        let cwd = tmp.path().to_str().unwrap();
        // 文本：500KB 恰好放行、+1 字节拒绝
        let txt = tmp.path().join("big.txt");
        std::fs::write(&txt, vec![b'a'; 500 * 1024]).unwrap();
        assert!(
            read_file_safe(cwd, txt.to_str().unwrap()).is_ok(),
            "恰 500KB 放行"
        );
        std::fs::write(&txt, vec![b'a'; 500 * 1024 + 1]).unwrap();
        assert!(
            read_file_safe(cwd, txt.to_str().unwrap()).is_err(),
            "超 500KB 拒"
        );
        // 图片：600KB（超文本上限、低于图片上限）必须放行；>5MB 拒
        let png = tmp.path().join("pic.png");
        std::fs::write(&png, vec![0u8; 600 * 1024]).unwrap();
        let (bytes, mime) = read_file_safe(cwd, png.to_str().unwrap()).unwrap();
        assert_eq!(mime, "image/png");
        assert_eq!(bytes.len(), 600 * 1024);
        std::fs::write(&png, vec![0u8; 5 * 1024 * 1024 + 1]).unwrap();
        assert!(
            read_file_safe(cwd, png.to_str().unwrap()).is_err(),
            "超 5MB 拒"
        );
    }

    #[test]
    fn mime_from_ext_covers_brief_list_and_falls_back() {
        assert_eq!(mime_from_ext("md"), "text/markdown");
        assert_eq!(mime_from_ext("rs"), "text/rust");
        assert_eq!(mime_from_ext("ts"), "text/typescript");
        assert_eq!(mime_from_ext("tsx"), "text/typescript");
        assert_eq!(mime_from_ext("js"), "text/javascript");
        assert_eq!(mime_from_ext("py"), "text/python");
        assert_eq!(mime_from_ext("png"), "image/png");
        assert_eq!(mime_from_ext("jpg"), "image/jpeg");
        assert_eq!(mime_from_ext("gif"), "image/gif");
        assert_eq!(mime_from_ext("svg"), "image/svg+xml");
        assert_eq!(mime_from_ext("json"), "application/json");
        // Task 8 评审 Minor①：yaml/yml 从 toml 拆出，不再误标 text/toml
        assert_eq!(mime_from_ext("toml"), "text/toml");
        assert_eq!(mime_from_ext("yaml"), "text/yaml");
        assert_eq!(mime_from_ext("yml"), "text/yaml");
        assert_eq!(mime_from_ext(""), "text/plain");
        assert_eq!(mime_from_ext("unknownxyz"), "text/plain");
        // 大写扩展名（IMG.PNG 实测形态）不误判为文本
        assert_eq!(mime_from_ext("PNG"), "image/png");
    }

    // ==== path_within_semantics（Windows 语义注入，darwin 全分支可测）====

    #[test]
    fn path_within_unix_semantics_with_boundary() {
        assert!(path_within_semantics("/a/b/c", "/a/b", false));
        assert!(
            path_within_semantics("/a/b", "/a/b", false),
            "自身包含自身（目录拒绝由 is_file 兜）"
        );
        assert!(
            path_within_semantics("/a/x", "/", false),
            "根包含一切绝对路径"
        );
        // 分隔符边界：/ab 不在 /a 之下
        assert!(!path_within_semantics("/ab", "/a", false));
        assert!(!path_within_semantics("/a/b", "/a/c", false));
        assert!(
            !path_within_semantics("/a/b", "", false),
            "空祖先不含任何路径"
        );
        // 相对路径形态不误判（canonicalize 后不会出现，防御性只判字面）
        assert!(!path_within_semantics("a/b", "/a", false));
    }

    #[test]
    fn path_within_windows_semantics_strips_verbatim_and_case_insensitive() {
        // verbatim 前缀剥离 + 大小写不敏感（canonicalize 恒返 \\?\ 形态 vs 字面 cwd）
        assert!(path_within_semantics(
            r"\\?\C:\proj\src\m.rs",
            r"C:\proj",
            true
        ));
        assert!(path_within_semantics(
            r"C:\proj\src\m.rs",
            r"\\?\C:\PROJ",
            true
        ));
        // UNC 形态还原
        assert!(path_within_semantics(
            r"\\?\UNC\server\share\x.txt",
            r"\\server\share",
            true
        ));
        // 边界：C:\proj2 不在 C:\proj 之下
        assert!(!path_within_semantics(r"\\?\C:\proj2\a", r"C:\proj", true));
        assert!(!path_within_semantics(r"\\?\D:\proj\a", r"C:\proj", true));
        // 尾部分隔符的祖先
        assert!(path_within_semantics(r"\\?\C:\proj\a", r"C:\proj\", true));
    }

    // ==== extract_paths_from_messages ====

    #[test]
    fn extract_collects_known_keys_recursively_and_dedups_in_order() {
        let msgs = vec![
            text_msg("user", "帮我改"),
            tool_call(r#"{"file_path":"/tmp/proj/src/a.rs","content":"x"}"#),
            tool_call(r#"{"command":"cat","path":"/tmp/proj/src/a.rs"}"#), // 同路径去重
            tool_call(
                r#"{"edits":[{"filename":"/tmp/proj/src/b.rs"},{"path":{"file_path":"/tmp/proj/lib/c.py"}}]}"#,
            ),
            tool_call(r#"{"abs_path":"/tmp/proj/README.md"}"#),
            text_msg("assistant", "done"), // 非 tool-call 忽略
        ];
        assert_eq!(
            extract_paths_from_messages(&msgs),
            vec![
                "/tmp/proj/src/a.rs".to_string(),
                "/tmp/proj/src/b.rs".to_string(),
                "/tmp/proj/lib/c.py".to_string(),
                "/tmp/proj/README.md".to_string(),
            ],
            "命中键递归收集 + 去重保序"
        );
    }

    #[test]
    fn extract_skips_non_path_values() {
        let msgs = vec![
            // 命中键但值不是路径候选：URL / 纯词 / 换行串 / 空
            tool_call(r#"{"path":"https://example.com/x.rs"}"#),
            tool_call(r#"{"path":"main"}"#),
            tool_call(r#"{"path":"line1\nline2.rs"}"#),
            tool_call(r#"{"path":""}"#),
            tool_call(r#"{"path":42}"#),
            tool_call(r#"{"cmd":"ls -la /etc"}"#), // 非命中键不下手
            tool_call("not-json"),                 // 参数损坏
            tool_call(r#"{}"#),                    // 无参数对象
        ];
        // "main" 无分隔符无扩展名 → 排除；其余全部排除 → 空表
        assert!(extract_paths_from_messages(&msgs).is_empty());
    }

    #[test]
    fn extract_accepts_relative_paths_and_bare_filenames_with_ext() {
        let msgs = vec![
            tool_call(r#"{"file_path":"src/lib/util.ts"}"#),
            tool_call(r#"{"path":"Cargo.toml"}"#),
        ];
        assert_eq!(
            extract_paths_from_messages(&msgs),
            vec!["src/lib/util.ts".to_string(), "Cargo.toml".to_string()],
            "相对路径与裸扩展名文件名都是候选（端点侧按 cwd 解析相对路径）"
        );
    }

    // ==== Task 9：七工具提取链路 fixture（真实 toolArgs 形态口径，逐工具探测证据
    // 见 task-9-report.md）。全链路 = tempdir home → content 层统一出口 → 泛化提取。
    // claude 已有 extract_file_paths_with_reads_through_content_layer（Task 8 实机验证）。====

    const T9_UUID: &str = "1f2e3d4c-5b6a-4948-8276-9a0b8c7d6e5f";

    /// ZCode tmp sqlite（message + part 表，content.rs 测试同款 schema）
    fn t9_zcode_db(home: &Path) -> rusqlite::Connection {
        let db = home.join(".zcode/cli/db/db.sqlite");
        std::fs::create_dir_all(db.parent().unwrap()).unwrap();
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute_batch(
            "CREATE TABLE message (
                id TEXT PRIMARY KEY, session_id TEXT, sequence INTEGER,
                time_created INTEGER, data TEXT
             );
             CREATE TABLE part (
                id TEXT PRIMARY KEY, message_id TEXT, sequence INTEGER, data TEXT
             );",
        )
        .unwrap();
        conn
    }

    /// 实测形态（本机 ~/.zcode 真实库，4646+758+3370 命中）：Edit/Write/Read 的
    /// state.input 用 file_path 键；Bash 的 command 键不是路径参数——不得误收
    #[test]
    fn extract_chain_zcode_tool_part_input_file_path() {
        let tmp = tempfile::tempdir().unwrap();
        let conn = t9_zcode_db(tmp.path());
        conn.execute(
            "INSERT INTO message VALUES ('m1', 'sess-t9', 1, 100, '{\"role\":\"assistant\",\"semantics\":{\"kind\":\"assistant\"}}')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO part VALUES ('p1', 'm1', 1, '{\"type\":\"tool\",\"tool\":\"Edit\",\"state\":{\"input\":{\"file_path\":\"/tmp/proj/src/main.rs\",\"old_string\":\"a\",\"new_string\":\"b\"}}}')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO part VALUES ('p2', 'm1', 2, '{\"type\":\"tool\",\"tool\":\"Bash\",\"state\":{\"input\":{\"command\":\"cat /etc/hostname\"}}}')",
            [],
        )
        .unwrap();
        assert_eq!(
            extract_file_paths_with(tmp.path(), "zcode", "sess-t9"),
            vec!["/tmp/proj/src/main.rs".to_string()],
            "zcode state.input.file_path 收集；bash command 不收"
        );
    }

    /// 实测形态（本机 ~/.codex 真实 rollout 抽样 40 份）：view_image 的 arguments.path
    /// 是唯一成规模的文件路径键；apply_patch 的 command 是补丁全文（路径只是嵌在
    /// 补丁字符串里，不是路径参数键）——嵌在串内的路径不得被误收
    #[test]
    fn extract_chain_codex_rollout_function_call_arguments() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join(".codex/sessions/2026/09/15");
        std::fs::create_dir_all(&dir).unwrap();
        let rollout = dir.join(format!("rollout-2026-09-15T00-00-00-{T9_UUID}.jsonl"));
        let meta = format!(
            r#"{{"timestamp":"2026-09-15T00:00:00.000Z","type":"session_meta","payload":{{"id":"{T9_UUID}","cwd":"/tmp/proj"}}}}"#
        );
        let view_image = r#"{"timestamp":"2026-09-15T00:00:10.000Z","type":"response_item","payload":{"type":"function_call","name":"view_image","arguments":"{\"path\":\"/tmp/shots/page1.png\"}"}}"#;
        let apply_patch = r#"{"timestamp":"2026-09-15T00:00:20.000Z","type":"response_item","payload":{"type":"function_call","name":"apply_patch","arguments":"{\"command\":\"*** Add File: /tmp/proj/embedded.rs\"}"}}"#;
        std::fs::write(&rollout, format!("{meta}\n{view_image}\n{apply_patch}\n")).unwrap();
        assert_eq!(
            extract_file_paths_with(tmp.path(), "codex", T9_UUID),
            vec!["/tmp/shots/page1.png".to_string()],
            "codex arguments.path 收集；补丁串内嵌路径不收"
        );
    }

    /// dsh tmp home zstd 代际（content.rs 测试同款 header/events 编码）。
    /// 实测形态（本机 ~/.dsh 21 份代际日志）：read/write/edit/read_image 的
    /// arguments.file_path；present 的 files[].path（数组嵌套递归）；bash 的
    /// command 是命令串——不得误收
    #[test]
    fn extract_chain_dsh_tool_call_arguments() {
        let tmp = tempfile::tempdir().unwrap();
        let header = r#"{"type":"session","version":3,"id":"session-t9","cwd":"/tmp/proj","createdAt":1000,"isSeeded":false}"#;
        let events = concat!(
            r#"{"type":"tool/call","seq":10,"time":1112,"data":{"name":"edit","callId":"c1","arguments":"{\"file_path\":\"/tmp/proj/src/a.rs\",\"old_string\":\"x\",\"new_string\":\"y\"}"}}"#,
            "\n",
            r#"{"type":"tool/call","seq":11,"time":1113,"data":{"name":"bash","callId":"c2","arguments":"{\"command\":\"ls /etc\"}"}}"#,
            "\n",
            r#"{"type":"tool/call","seq":12,"time":1114,"data":{"name":"present","callId":"c3","arguments":"{\"files\":[{\"path\":\"/tmp/proj/docs/report.md\",\"description\":\"d\"}]}"}}"#,
            "\n",
        );
        let frame = zstd::stream::encode_all(format!("{header}\n{events}").as_bytes(), 3).unwrap();
        let sess = tmp.path().join(".dsh/sessions/--proj--/escaped~dir");
        std::fs::create_dir_all(&sess).unwrap();
        std::fs::write(sess.join("session.v3.jsonl.zstd"), &frame).unwrap();
        assert_eq!(
            extract_file_paths_with(tmp.path(), "dsh", "session-t9"),
            vec![
                "/tmp/proj/src/a.rs".to_string(),
                "/tmp/proj/docs/report.md".to_string(),
            ],
            "dsh arguments.file_path 收集 + files[].path 嵌套收集；bash command 不收"
        );
    }

    /// Kimi tmp home（session_index 定位 + wire.jsonl）。实测形态（本机
    /// ~/.kimi-code 35 份 wire）：Read/Edit/Write/Grep 的 args.path；另有旧版
    /// Read 用 args.file（3 实测绝对路径样本）——file 键须在收集名单内
    #[test]
    fn extract_chain_kimi_wire_tool_call_args() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join(".kimi-code");
        let session_dir = home
            .join("sessions/wd_t9")
            .join(format!("session_{T9_UUID}"));
        std::fs::create_dir_all(session_dir.join("agents/main")).unwrap();
        let lines = concat!(
            r#"{"type":"context.append_loop_event","event":{"type":"tool.call","toolCallId":"c1","name":"Edit","args":{"path":"/tmp/proj/src/m.rs","old_string":"a","new_string":"b"}},"time":100}"#,
            "\n",
            r#"{"type":"context.append_loop_event","event":{"type":"tool.call","toolCallId":"c2","name":"Read","args":{"file":"/tmp/proj/README.md","offset":10,"limit":80}},"time":200}"#,
            "\n",
            r#"{"type":"context.append_loop_event","event":{"type":"tool.call","toolCallId":"c3","name":"Bash","args":{"command":"ls /tmp"}},"time":300}"#,
            "\n",
        );
        std::fs::write(session_dir.join("agents/main/wire.jsonl"), lines).unwrap();
        let index = serde_json::json!({
            "sessionId": T9_UUID,
            "sessionDir": session_dir.to_string_lossy(),
            "workDir": "/tmp/proj",
        })
        .to_string();
        std::fs::write(home.join("session_index.jsonl"), format!("{index}\n")).unwrap();
        assert_eq!(
            extract_file_paths_with(tmp.path(), "kimi", T9_UUID),
            vec![
                "/tmp/proj/src/m.rs".to_string(),
                "/tmp/proj/README.md".to_string(),
            ],
            "kimi args.path 与旧版 args.file 均收集；bash command 不收"
        );
    }

    /// WorkBuddy tmp home（projects 扫描定位）。实测形态（本机 ~/.workbuddy）：
    /// OpenAI 风格 function_call 的 arguments.file_path（Read/Write/Edit）
    #[test]
    fn extract_chain_workbuddy_function_call_arguments() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join(".workbuddy/projects/-tmp-proj");
        std::fs::create_dir_all(&dir).unwrap();
        let lines = concat!(
            r#"{"type":"message","role":"user","content":[{"type":"text","text":"改一下"}]}"#,
            "\n",
            r#"{"type":"function_call","name":"Edit","arguments":"{\"file_path\":\"/tmp/proj/src/wb.rs\",\"old_string\":\"a\",\"new_string\":\"b\"}"}"#,
            "\n",
            r#"{"type":"function_call","name":"Bash","arguments":"{\"command\":\"cat /etc/hosts\"}"}"#,
            "\n",
        );
        std::fs::write(dir.join(format!("{T9_UUID}.jsonl")), lines).unwrap();
        assert_eq!(
            extract_file_paths_with(tmp.path(), "workbuddy", T9_UUID),
            vec!["/tmp/proj/src/wb.rs".to_string()],
            "workbuddy arguments.file_path 收集；bash command 不收"
        );
    }

    /// OpenCode tmp sqlite。本机库无 tool part 样本（实测只有 text/patch）——
    /// 形态按官方文档合成：edit/write/read 的 state.input 用 filePath 驼峰键
    /// （opencode.ai/docs/tools 与 issue #729 交叉确认），须在收集名单内
    #[test]
    fn extract_chain_opencode_tool_part_state_input() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join(".local/share/opencode/opencode.db");
        std::fs::create_dir_all(db.parent().unwrap()).unwrap();
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute_batch(
            "CREATE TABLE message (id TEXT PRIMARY KEY, session_id TEXT, time_created INTEGER, data TEXT);
             CREATE TABLE part (message_id TEXT, data TEXT, time_created INTEGER);",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message VALUES ('m1', ?1, 100, '{\"role\":\"assistant\"}')",
            ["sess-t9-oc"],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO part VALUES ('m1', '{\"type\":\"tool\",\"tool\":\"edit\",\"state\":{\"input\":{\"filePath\":\"/tmp/proj/src/oc.ts\",\"oldString\":\"a\",\"newString\":\"b\"}}}', 1)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO part VALUES ('m1', '{\"type\":\"tool\",\"tool\":\"bash\",\"state\":{\"input\":{\"command\":\"ls\"}}}', 2)",
            [],
        )
        .unwrap();
        assert_eq!(
            extract_file_paths_with(tmp.path(), "opencode", "sess-t9-oc"),
            vec!["/tmp/proj/src/oc.ts".to_string()],
            "opencode state.input.filePath（驼峰）收集；bash command 不收"
        );
    }

    /// OpenClaw tmp sqlite（acp_replay_events）。协议形态（sessionUpdate/rawInput）
    /// 为 2026-09-15 实机探测确认，但本机库无 tool_call 事件样本、rawInput 内的
    /// 路径键名无实测依据——合成用 file_path 锁「replay → toolArgs → 提取」链路，
    /// 真实键名若不同，泛化名单（path/file/filePath 等）大概率兜住（低置信度备案）
    #[test]
    fn extract_chain_openclaw_acp_tool_call_raw_input() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join(".openclaw/state/openclaw.sqlite");
        std::fs::create_dir_all(db.parent().unwrap()).unwrap();
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute_batch(
            "CREATE TABLE acp_replay_events (
                session_id TEXT NOT NULL, seq INTEGER NOT NULL, at INTEGER NOT NULL,
                session_key TEXT NOT NULL, run_id TEXT, update_json TEXT NOT NULL,
                PRIMARY KEY (session_id, seq)
             );",
        )
        .unwrap();
        let ins = |seq: i64, update: &str| {
            conn.execute(
                "INSERT INTO acp_replay_events (session_id, seq, at, session_key, update_json) VALUES ('s-t9', ?1, ?2, 'k', ?3)",
                rusqlite::params![seq, seq * 100, update],
            )
            .unwrap();
        };
        ins(
            1,
            r#"{"sessionUpdate":"tool_call","toolCallId":"c1","title":"write","rawInput":{"file_path":"/tmp/proj/src/oclaw.rs","content":"x"}}"#,
        );
        ins(
            2,
            r#"{"sessionUpdate":"tool_call","toolCallId":"c2","title":"bash","rawInput":{"command":"ls /tmp"}}"#,
        );
        assert_eq!(
            extract_file_paths_with(tmp.path(), "openclaw", "s-t9"),
            vec!["/tmp/proj/src/oclaw.rs".to_string()],
            "openclaw rawInput 经 ACP 映射后收集；bash command 不收"
        );
    }

    /// 计划测试名保留：注入核全链路（tempdir home 的 claude fixture → 统一出口 → 提取）。
    /// Task 9 已按同款为其余七工具补 fixture（上方 extract_chain_* 系列）。
    #[test]
    fn extract_file_paths_with_reads_through_content_layer() {
        let tmp = tempfile::tempdir().unwrap();
        let proj = tmp
            .path()
            .join(".claude")
            .join("projects")
            .join("-tmp-proj");
        std::fs::create_dir_all(&proj).unwrap();
        let sid = "0f1e2d3c-4b5a-4948-8276-9a0b8c7d6e5f";
        let line = r#"{"type":"assistant","timestamp":"2026-09-15T00:00:00Z","message":{"role":"assistant","content":[{"type":"tool_use","id":"t1","name":"Write","input":{"file_path":"/tmp/proj/src/main.rs","content":"fn main(){}"}}]}}"#;
        std::fs::write(proj.join(format!("{sid}.jsonl")), format!("{line}\n")).unwrap();
        assert_eq!(
            extract_file_paths_with(tmp.path(), "claude", sid),
            vec!["/tmp/proj/src/main.rs".to_string()],
            "全链路：content 层读取 → tool-call 参数提取"
        );
        // 会话不存在 → 空表（不 Err，文件面板是增强能力）
        assert!(extract_file_paths_with(tmp.path(), "claude", "missing").is_empty());
    }

    /// 终审 Important 1：/session-files 与 /session-messages 数据同源——DSH_HOME /
    /// KIMI_CODE_HOME 数据源重定向必须同样作用到文件面板提取。env 值以**参数**注入
    /// （dsh_env_home_redirects_data_root 同款模式，测试零真实 env 接触）。旧实现恒走
    /// None env（read_session_messages_with）——设了 env 的机器上详情页消息正常、
    /// 文件面板静默空表，本测试在旧实现下必红
    #[test]
    fn extract_file_paths_env_redirect_reaches_file_panel() {
        // ---- dsh：会话数据只在 DSH_HOME 根下（home 下无任何 dsh 数据）----
        let tmp = tempfile::tempdir().unwrap();
        let dsh_env_dir = tempfile::tempdir().unwrap();
        let header = r#"{"type":"session","version":3,"id":"session-env","cwd":"/tmp/proj","createdAt":1000,"isSeeded":false}"#;
        let events = r#"{"type":"tool/call","seq":10,"time":1112,"data":{"name":"edit","callId":"c1","arguments":"{\"file_path\":\"/tmp/proj/src/env.rs\",\"old_string\":\"x\",\"new_string\":\"y\"}"}}"#;
        let frame =
            zstd::stream::encode_all(format!("{header}\n{events}\n").as_bytes(), 3).unwrap();
        let sess = dsh_env_dir.path().join("sessions/--proj--/escaped~dir");
        std::fs::create_dir_all(&sess).unwrap();
        std::fs::write(sess.join("session.v3.jsonl.zstd"), &frame).unwrap();

        let dsh_env = dsh_env_dir.path().to_str().unwrap().to_string();
        assert_eq!(
            extract_file_paths_with_env(
                tmp.path(),
                Some(dsh_env.as_str()),
                None,
                "dsh",
                "session-env"
            ),
            vec!["/tmp/proj/src/env.rs".to_string()],
            "DSH_HOME 重定向必须作用到文件面板提取（与详情页消息同源）"
        );
        // env 未设形态（None）→ 回落 home/.dsh → 无数据 → 空表
        assert!(
            extract_file_paths_with_env(tmp.path(), None, None, "dsh", "session-env").is_empty()
        );

        // ---- kimi：会话数据只在 KIMI_CODE_HOME 根下 ----
        let kimi_env_dir = tempfile::tempdir().unwrap();
        let session_dir = kimi_env_dir
            .path()
            .join("sessions/wd_env")
            .join(format!("session_{T9_UUID}"));
        std::fs::create_dir_all(session_dir.join("agents/main")).unwrap();
        let wire = concat!(
            r#"{"type":"context.append_loop_event","event":{"type":"tool.call","toolCallId":"c1","name":"Edit","args":{"path":"/tmp/proj/src/kimi-env.rs","old_string":"a","new_string":"b"}},"time":100}"#,
            "\n",
        );
        std::fs::write(session_dir.join("agents/main/wire.jsonl"), wire).unwrap();
        let index = serde_json::json!({
            "sessionId": T9_UUID,
            "sessionDir": session_dir.to_string_lossy(),
            "workDir": "/tmp/proj",
        })
        .to_string();
        std::fs::write(
            kimi_env_dir.path().join("session_index.jsonl"),
            format!("{index}\n"),
        )
        .unwrap();

        let kimi_env = kimi_env_dir.path().to_str().unwrap().to_string();
        assert_eq!(
            extract_file_paths_with_env(tmp.path(), None, Some(kimi_env.as_str()), "kimi", T9_UUID),
            vec!["/tmp/proj/src/kimi-env.rs".to_string()],
            "KIMI_CODE_HOME 重定向必须作用到文件面板提取（与详情页消息同源）"
        );
        assert!(
            extract_file_paths_with_env(tmp.path(), None, None, "kimi", T9_UUID).is_empty(),
            "env 未设形态回落 ~/.kimi-code → 无数据 → 空表"
        );
    }
}
