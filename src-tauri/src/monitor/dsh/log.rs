// dsh 会话日志读取层：三代际并存（v0/v2/v3，取代际最大者——M0 F2），
// header 必读（目录名 ~XXXX 转义不可反解 id，评审漏项 #3；header 兼得子 Agent 过滤字段）

use serde::Deserialize;
use std::path::{Path, PathBuf};

use super::decode::decode_zstd_frames;

#[derive(Debug, Clone, Deserialize)]
pub struct DshHeader {
    pub id: String,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(rename = "parentSession", default)]
    pub parent_session: Option<String>,
    #[serde(default)]
    pub origin: Option<String>,
    #[serde(rename = "delegationDepth", default)]
    pub delegation_depth: i64,
    #[serde(rename = "createdAt", default)]
    pub created_at: Option<i64>,
    #[serde(rename = "isSeeded", default)]
    pub is_seeded: Option<bool>,
    #[serde(default)]
    pub version: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DshEvent {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub seq: Option<i64>,
    #[serde(default)]
    pub time: Option<i64>,
    #[serde(default)]
    pub data: serde_json::Value,
}

/// 子 Agent 会话过滤（M0 F7：真实 home 33/76 是子 Agent，不出卡）
pub fn is_subagent(h: &DshHeader) -> bool {
    h.origin.as_deref() == Some("subagent") || h.delegation_depth > 0
}

/// 文件名 → 代际号；不识别的文件名忽略（版本门：未来 v4 只需放宽正则）
fn parse_generation(name: &str) -> Option<i64> {
    if name == "session.jsonl" || name == "session.jsonl.zstd" {
        return Some(0);
    }
    let rest = name.strip_prefix("session.v")?;
    let rest = rest
        .strip_suffix(".jsonl.zstd")
        .or_else(|| rest.strip_suffix(".jsonl"))?;
    rest.parse::<i64>().ok()
}

pub fn generation_logs(dir: &Path) -> Vec<(i64, PathBuf)> {
    let mut out: Vec<(i64, PathBuf)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            parse_generation(&name).map(|v| (v, e.path()))
        })
        .collect();
    out.sort_by_key(|(v, _)| *v);
    out
}

pub struct GenerationRead {
    pub version: i64,
    pub text: String,
    pub torn_frames: usize,
    /// 日志 mtime（毫秒）：v0 兜底判定的静默时长输入
    pub mtime_ms: i64,
}

pub fn read_best_generation(dir: &Path) -> Option<GenerationRead> {
    let (version, path) = generation_logs(dir).pop()?;
    let bytes = std::fs::read(&path).ok()?;
    let mtime_ms = std::fs::metadata(&path)
        .ok()?
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_millis() as i64;
    let (text, torn_frames) = if path.extension().map(|e| e == "zstd").unwrap_or(false) {
        let d = decode_zstd_frames(&bytes).ok()?;
        (d.text, d.torn_frames)
    } else {
        (String::from_utf8_lossy(&bytes).to_string(), 0)
    };
    Some(GenerationRead {
        version,
        text,
        torn_frames,
        mtime_ms,
    })
}

pub fn parse_header(text: &str) -> Option<DshHeader> {
    serde_json::from_str(text.lines().next()?).ok()
}

pub fn parse_events(text: &str) -> Vec<DshEvent> {
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> String {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/dsh")
            .join(name);
        std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("读取夹具失败 {p:?}: {e}"))
    }

    #[test]
    fn golden_header_and_subagent_filter() {
        // 黄金样本 sample1：header 含 id/cwd；非子 Agent
        let text = fixture("sample1-completed.sanitized.jsonl");
        let h = parse_header(&text).expect("header 可解析");
        assert!(h.id.starts_with("session-"));
        assert!(h.cwd.as_deref().unwrap_or("").len() > 0);
        assert!(!is_subagent(&h));
        let events = parse_events(&text);
        assert_eq!(events.len(), 21); // M0 实测 21 事件
    }

    #[test]
    fn subagent_detected_by_origin_and_depth() {
        let mut h = DshHeader {
            id: "x".into(),
            cwd: None,
            parent_session: None,
            origin: None,
            delegation_depth: 0,
            created_at: None,
            is_seeded: None,
            version: None,
        };
        assert!(!is_subagent(&h));
        h.origin = Some("subagent".into());
        assert!(is_subagent(&h));
        h.origin = None;
        h.delegation_depth = 1;
        assert!(is_subagent(&h));
    }

    #[test]
    fn picks_max_generation() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        // 未压缩 v0（旧存量）
        std::fs::write(
            d.join("session.jsonl"),
            "{\"type\":\"session\",\"version\":0,\"id\":\"a\"}\n",
        )
        .unwrap();
        // 压缩 v3（新）—— 用解码器测试同款编码
        let frame = zstd::stream::encode_all(
            b"{\"type\":\"session\",\"version\":3,\"id\":\"a\"}\n".as_slice(),
            3,
        )
        .unwrap();
        std::fs::write(d.join("session.v3.jsonl.zstd"), &frame).unwrap();
        let read = read_best_generation(d).expect("应读到");
        assert_eq!(read.version, 3);
        let h = parse_header(&read.text).unwrap();
        assert_eq!(h.version, Some(3));
    }

    #[test]
    fn empty_dir_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read_best_generation(dir.path()).is_none());
        assert!(generation_logs(dir.path()).is_empty());
    }
}
