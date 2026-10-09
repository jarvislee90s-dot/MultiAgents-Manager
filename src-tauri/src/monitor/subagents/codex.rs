//! Codex 子 agent source（spec §4.4：线程对判据）。
//! 子 agent = rollout session_meta 带 parent_thread_id（普通会话无此字段——零歧义）；
//! 运行中 = 该 rollout 无 task_complete 事件（task_complete_seen 布尔出现即置位
//! 永不复位，判据查它而非回扫文件）。
//! 发现层 = 倒排索引（parent_thread_id → rollout 路径）：sessions 目录 249+ 文件，
//! 不可能每轮全扫首行。首行摘要（session_meta）走 `SessionFileScan::parse`（T =
//! Option<ChildMeta>）——(mtime,size) 门控与**负缓存**（无 parent_thread_id 的普通
//! rollout 也只读一次首行）免费继承，**不手搓第二套 stat 比对**（评审 P2-4）；
//! 扫描窗口对齐 codex_parser 的 24h 新鲜窗口（L3：fresh 现解析 / stale 仅 peek
//! 缓存命中参与，与 codex_parser 同款分流）。
//! 桶位映射（§4.4）：input = input_tokens − cached_input_tokens（内含防重复计数）、
//! cacheRead = cached_input_tokens、cacheCreation = cache_write_input_tokens、
//! output = output_tokens；reasoning_output_tokens 不计入展示合计。

use super::{
    read_increment, sort_views, update_entry, CacheBox, IncrRead, IncrState, SubagentView,
    TokenUsage,
};
use crate::monitor::codex_parser::is_rollout_file;
use crate::monitor::session_scan::{fresh_within, SessionFileScan};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

/// 索引文件扫描（复用 SessionFileScan 的递归收集 + (mtime,size) 摘要缓存 + 负缓存；
/// 独立 namespace 不与 codex-digest 串扰）
const CODEX_IDX_SCAN: SessionFileScan = SessionFileScan::new("codex-subagent-idx");

/// 桶位映射（纯函数；§4.4 防重复计数锁）
pub(crate) fn map_codex_usage(
    input: u64,
    cached: u64,
    cache_write: u64,
    output: u64,
) -> TokenUsage {
    TokenUsage {
        input: input.saturating_sub(cached),
        cache_read: cached,
        cache_creation: cache_write,
        output,
    }
}

/// 子 rollout 行累计（纯函数）：task_complete 置位（永不复位）+ token_usage_record 直读覆盖
#[derive(Default, Clone)]
pub(crate) struct RolloutAccum {
    pub(crate) task_complete_seen: bool,
    pub(crate) tokens: TokenUsage,
}

pub(crate) fn apply_rollout_line(line: &str, acc: &mut RolloutAccum) {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
        return;
    };
    if v["type"].as_str() == Some("event_msg")
        && v["payload"]["type"].as_str() == Some("task_complete")
    {
        acc.task_complete_seen = true;
    }
    if v["type"].as_str() == Some("token_usage_record") {
        let tu = &v["payload"]["total_token_usage"];
        acc.tokens = map_codex_usage(
            tu["input_tokens"].as_u64().unwrap_or(0),
            tu["cached_input_tokens"].as_u64().unwrap_or(0),
            tu["cache_write_input_tokens"].as_u64().unwrap_or(0),
            tu["output_tokens"].as_u64().unwrap_or(0),
        );
    }
}

/// 子 rollout 首行摘要（SessionFileScan 缓存产物；None = 非子 rollout——负缓存：
/// 普通 rollout 首行只读一次，后续轮次 (mtime,size) 未变零重读，评审 P2-4）
#[derive(Clone)]
struct ChildMeta {
    child_id: String,
    parent_thread_id: String,
    spawn_ts: Option<String>,
}

/// 首行 session_meta 解析（SessionFileScan::parse 的解析函数）：子形态（带
/// parent_thread_id）才有意义，其余一律 None（负缓存条目）
fn read_child_meta(path: &Path) -> Option<ChildMeta> {
    let first = crate::monitor::jsonl::read_first_lines(path, 1)
        .first()?
        .clone();
    let v: serde_json::Value = serde_json::from_str(&first).ok()?;
    if v["type"].as_str() != Some("session_meta") {
        return None;
    }
    let id = v["payload"]["id"].as_str()?.to_string();
    let parent = v["payload"]["parent_thread_id"].as_str()?.to_string();
    let ts = v["timestamp"].as_str().map(str::to_string);
    Some(ChildMeta {
        child_id: id,
        parent_thread_id: parent,
        spawn_ts: ts,
    })
}

/// 索引扫描（L3 分流与 codex_parser.scan_codex_sessions 同款，测试
/// codex_index_window_allows_cached_stale_entries 锁定）：fresh → parse（含负缓存
/// 命中跳过）；stale → peek（仅缓存命中参与，未缓存 = 历史首行不碰）。
/// **不手搓 stat 比对 / retain 收敛**——门控与孤儿清理全在 SessionFileScan 内。
fn scan_child_metas(
    sessions_dir: &Path,
    now: SystemTime,
) -> Vec<(PathBuf, Arc<Option<ChildMeta>>)> {
    CODEX_IDX_SCAN
        .collect(sessions_dir, is_rollout_file)
        .into_iter()
        .map(|(path, mtime)| {
            let meta = if fresh_within(mtime, now) {
                CODEX_IDX_SCAN.parse(&path, read_child_meta)
            } else {
                CODEX_IDX_SCAN
                    .peek::<Option<ChildMeta>>(&path)
                    .unwrap_or_else(|| Arc::new(None))
            };
            (path, meta)
        })
        .collect()
}

/// 会话级缓存条目：每子 rollout 一份增量游标 + 终态布尔
#[derive(Default, Clone)]
struct CodexCache {
    children: HashMap<String, ChildTrack>,
}
#[derive(Default, Clone)]
struct ChildTrack {
    incr: IncrState,
    acc: RolloutAccum,
}

pub fn collect(session_id: &str) -> Vec<SubagentView> {
    let Some(dir) = dirs::home_dir().map(|h| h.join(".codex").join("sessions")) else {
        return Vec::new();
    };
    collect_with(&dir, session_id, SystemTime::now())
}

pub(crate) fn collect_with(
    sessions_dir: &Path,
    session_id: &str,
    now: SystemTime,
) -> Vec<SubagentView> {
    // 倒排查询：全量首行摘要中过滤 parent_thread_id == 本会话（摘要已被
    // SessionFileScan 缓存，这步是内存过滤，零额外首行读）
    let children: Vec<(PathBuf, ChildMeta)> = scan_child_metas(sessions_dir, now)
        .into_iter()
        .filter_map(|(p, m)| m.as_ref().as_ref().cloned().map(|m| (p, m)))
        .filter(|(_, m)| m.parent_thread_id == session_id)
        .collect();
    update_entry("codex", session_id, move |cur| {
        let mut st = cur
            .and_then(|b| b.downcast::<CodexCache>().ok())
            .map(|a| Arc::try_unwrap(a).unwrap_or_else(|a| (*a).clone()))
            .unwrap_or_default();
        let mut views = Vec::new();
        for (path, meta) in &children {
            let track = st.children.entry(meta.child_id.clone()).or_default();
            match read_increment(path, &mut track.incr) {
                IncrRead::Unchanged => {}
                IncrRead::Lines { lines, reset } => {
                    if reset {
                        track.acc = RolloutAccum::default();
                    }
                    for line in &lines {
                        apply_rollout_line(line, &mut track.acc);
                    }
                }
            }
            if !track.acc.task_complete_seen {
                views.push(SubagentView {
                    id: meta.child_id.clone(),
                    name: meta.child_id.chars().take(8).collect(), // 无名源（C3 校准点）
                    description: None,
                    spawn_ts: meta.spawn_ts.clone(),
                    tokens: track.acc.tokens,
                });
            }
        }
        sort_views(&mut views);
        (Arc::new(st) as CacheBox, views)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn session_dir(td: &tempfile::TempDir) -> PathBuf {
        let d = td
            .path()
            .join("sessions")
            .join("2026")
            .join("10")
            .join("08");
        std::fs::create_dir_all(&d).unwrap();
        d
    }
    /// rollout 首行 session_meta（子形态带 parent_thread_id；普通会话无此字段）
    fn meta_line(id: &str, parent: Option<&str>, ts: &str) -> String {
        let p = match parent {
            Some(pt) => format!(r#","parent_thread_id":"{pt}""#),
            None => String::new(),
        };
        format!(
            r#"{{"timestamp":"{ts}","type":"session_meta","payload":{{"id":"{id}","cwd":"E:\\demo"{p}}}}}"#
        )
    }
    fn token_usage_line(inp: u64, cached: u64, cw: u64, out: u64) -> String {
        format!(
            r#"{{"timestamp":"2026-10-08T08:00:02Z","type":"token_usage_record","payload":{{"total_token_usage":{{"input_tokens":{inp},"cached_input_tokens":{cached},"cache_write_input_tokens":{cw},"output_tokens":{out},"reasoning_output_tokens":777,"total_tokens":99999}}}}}}"#
        )
    }
    fn complete_line(ts: &str) -> String {
        format!(
            r#"{{"timestamp":"{ts}","type":"event_msg","payload":{{"type":"task_complete","turn_id":"t1"}}}}"#
        )
    }
    fn write_rollout(dir: &Path, name: &str, lines: &[String]) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(
            &p,
            lines.iter().map(|l| format!("{l}\n")).collect::<String>(),
        )
        .unwrap();
        p
    }

    /// 桶位映射（§4.4）：input_tokens 内含 cached，直接求和会重复计数——P0 数字锁
    #[test]
    fn map_codex_usage_excludes_cached_from_input() {
        let t = map_codex_usage(100, 60, 10, 30);
        assert_eq!(
            t,
            TokenUsage {
                input: 40,
                cache_read: 60,
                cache_creation: 10,
                output: 30
            }
        );
        assert_eq!(t.total(), 140, "合计不含 reasoning 777（§8.5 申报）");
        assert_eq!(
            map_codex_usage(50, 60, 0, 0).input,
            0,
            "cached > input 时饱和到 0，不下溢"
        );
    }

    /// task_complete 出现即置位、永不复位；token_usage_record 直读覆盖（累计口径）
    #[test]
    fn rollout_line_applies_complete_and_usage() {
        let mut acc = RolloutAccum::default();
        apply_rollout_line(&token_usage_line(100, 60, 10, 30), &mut acc);
        assert!(!acc.task_complete_seen);
        assert_eq!(acc.tokens, map_codex_usage(100, 60, 10, 30));
        apply_rollout_line(&complete_line("2026-10-08T08:01:00Z"), &mut acc);
        assert!(acc.task_complete_seen);
        apply_rollout_line(&token_usage_line(1, 0, 0, 1), &mut acc);
        assert!(acc.task_complete_seen, "永不复位（§4.4）");
    }

    /// 倒排索引 + 判据：parent_thread_id 识别、无 task_complete 出板、换会话空
    #[test]
    fn codex_source_finds_child_by_parent_thread() {
        super::super::reset_cache_for_tests();
        let td = tempfile::tempdir().unwrap();
        let d = session_dir(&td);
        write_rollout(
            &d,
            "rollout-2026-10-08T08-00-00-parent.jsonl",
            &[meta_line("parent-1", None, "2026-10-08T08:00:00Z")],
        );
        write_rollout(
            &d,
            "rollout-2026-10-08T08-00-01-child-9abc.jsonl",
            &[
                meta_line("child-9abc", Some("parent-1"), "2026-10-08T08:00:01Z"),
                token_usage_line(100, 60, 10, 30),
            ],
        );
        let now = SystemTime::now();
        let v = collect_with(&td.path().join("sessions"), "parent-1", now);
        assert_eq!(v.len(), 1, "普通会话（无 parent_thread_id）不入索引");
        assert_eq!(v[0].id, "child-9abc");
        assert_eq!(
            v[0].name,
            "child-9a".to_string(),
            "无名源：id 前 8 位截断（C3 校准点）——样本 ≥8 字符让截断真被测到"
        );
        assert_eq!(v[0].spawn_ts.as_deref(), Some("2026-10-08T08:00:01Z"));
        assert_eq!(v[0].tokens, map_codex_usage(100, 60, 10, 30));
        assert!(collect_with(&td.path().join("sessions"), "parent-2", now).is_empty());
        // 追加 task_complete → 消失（增量：task_complete_seen 置位）
        let child = d.join("rollout-2026-10-08T08-00-01-child-9abc.jsonl");
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&child)
            .unwrap();
        f.write_all((complete_line("2026-10-08T08:05:00Z") + "\n").as_bytes())
            .unwrap();
        drop(f);
        assert!(collect_with(&td.path().join("sessions"), "parent-1", now).is_empty());
        super::super::reset_cache_for_tests();
    }

    /// 24h 窗口（L3 对齐）：新鲜 rollout 进索引；超窗文件仅缓存命中参与
    #[test]
    fn codex_index_window_allows_cached_stale_entries() {
        super::super::reset_cache_for_tests();
        let td = tempfile::tempdir().unwrap();
        let d = session_dir(&td);
        let p = write_rollout(
            &d,
            "rollout-a.jsonl",
            &[
                meta_line("c-1", Some("p-1"), "2026-10-08T08:00:01Z"),
                token_usage_line(1, 0, 0, 1),
            ],
        );
        let mtime = std::fs::metadata(&p).unwrap().modified().unwrap();
        // now1：新鲜 → 进索引
        assert_eq!(
            collect_with(
                &td.path().join("sessions"),
                "p-1",
                mtime + Duration::from_secs(10)
            )
            .len(),
            1
        );
        // now2 = mtime + 25h：文件超窗，但已缓存命中 → 仍参与（窗口外仅缓存命中参与）
        assert_eq!(
            collect_with(
                &td.path().join("sessions"),
                "p-1",
                mtime + Duration::from_secs(25 * 3600)
            )
            .len(),
            1,
            "判据本身不含时间叠加（task_complete_seen），窗口只管首行摘要的发现（stale 走 peek）"
        );
        super::super::reset_cache_for_tests();
    }
}
