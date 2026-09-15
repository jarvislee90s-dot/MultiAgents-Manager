// 文件安全读取与路径提取（M3 Task 8，C2 后端）
//
// 两条职责：
// - `extract_file_paths`：从会话消息流（复用 content::read_session_messages，八工具
//   统一出口）的 tool-call 条目里提取涉及的文件路径——**泛化实现，不做 per-tool
//   提取器**（控制者裁决 2026-09-15）：递归走 toolArgs JSON 树，收集
//   file_path / path / filename / abs_path 键下的字符串值（须含路径分隔符或
//   `.扩展名` 才算路径候选），去重保序。对八工具统一生效，Task 9 只需补各工具
//   的 fixture 测试。
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
/// 递归收集的参数键集合（控制者裁决的四个键；小写精确匹配）
const PATH_KEYS: &[&str] = &["file_path", "path", "filename", "abs_path"];
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
        "toml" | "yaml" | "yml" => "text/toml",
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

/// 生产薄壳：真实 home → 注入核（zcode_home_with 先例）。
/// 提取失败（存储不可读 / 会话不存在）一律返回空表——文件面板是增强能力，
/// 不因提取失败阻塞详情页。
pub fn extract_file_paths(agent_type: &str, session_id: &str) -> Vec<String> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    extract_file_paths_with(&home, agent_type, session_id)
}

/// 注入核（测试直调 tempdir home，零真实数据目录接触）
pub fn extract_file_paths_with(home: &Path, agent_type: &str, session_id: &str) -> Vec<String> {
    // 复用 Task 7 的八工具统一出口（数据同源：与详情页读的是同一份消息流），
    // 不另立 per-tool 查询（控制者裁决）
    match super::content::read_session_messages_with(
        home,
        agent_type,
        session_id,
        EXTRACT_MESSAGE_LIMIT,
    ) {
        Ok(msgs) => extract_paths_from_messages(&msgs),
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

    /// 计划测试名保留：注入核全链路（tempdir home 的 claude fixture → 统一出口 → 提取）。
    /// Task 9 将按同款为其余七工具补 fixture。
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
}
