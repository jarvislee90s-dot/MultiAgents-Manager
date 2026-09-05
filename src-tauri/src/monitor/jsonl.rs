// JSONL 会话文件读取公共件 — 尾部读取、cwd 提取、子 agent 计数、文件枚举
// 各工具 JSONL 解析器（claude/codex/kimi）共用；不包含任何工具协议判定逻辑

use crate::session::model::JsonlMessage;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// 读取 JSONL 文件尾部最多 max_lines 行（按文件顺序返回）。
/// 超过 512KB 的文件从尾部定位并跳过首条截断行，与既有解析器行为逐字节一致；
/// 调用方按 .iter().rev() 即得"最新在前"的遍历序
pub(crate) fn read_recent_lines(path: &Path, max_lines: usize) -> Vec<String> {
    let Ok(file) = File::open(path) else {
        return Vec::new();
    };
    let Ok(file_size) = file.metadata().map(|m| m.len()) else {
        return Vec::new();
    };
    let mut reader = BufReader::new(file);

    const TAIL_BYTES: u64 = 512 * 1024;
    if file_size > TAIL_BYTES {
        let _ = reader.seek(SeekFrom::End(-(TAIL_BYTES as i64)));
        let mut partial = String::new();
        let _ = reader.read_line(&mut partial);
    }

    let lines: Vec<String> = reader.lines().map_while(Result::ok).collect();
    let start = lines.len().saturating_sub(max_lines);
    lines[start..].to_vec()
}

/// 读取 JSONL 文件头部最多 max_lines 行（按文件顺序返回）。
/// 「取首条」场景专用（issue #35-6）：长会话的首条 user 消息不在尾部窗口内，
/// 标题降级需从文件头读取；文件缺失/不可读 → 空集
pub(crate) fn read_first_lines(path: &Path, max_lines: usize) -> Vec<String> {
    let Ok(file) = File::open(path) else {
        return Vec::new();
    };
    BufReader::new(file)
        .lines()
        .take(max_lines)
        .map_while(Result::ok)
        .collect()
}

/// 从 JSONL 文件头部提取首个有效 cwd（Claude 协议消息携带 cwd 字段）
pub(crate) fn extract_cwd_from_jsonl(jsonl_path: &Path) -> Option<String> {
    let file = File::open(jsonl_path).ok()?;
    let reader = BufReader::new(file);
    for line in reader.lines().take(20).flatten() {
        if let Ok(msg) = serde_json::from_str::<JsonlMessage>(&line) {
            if let Some(cwd) = msg.cwd {
                if super::project::is_valid_cwd(&cwd) {
                    return Some(cwd);
                }
            }
        }
    }
    None
}

/// Claude 子 agent 会话文件命名：agent-*.jsonl
pub(crate) fn is_subagent_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .map(|name| name.starts_with("agent-") && name.ends_with(".jsonl"))
        .unwrap_or(false)
}

/// 统计项目目录内 30 秒内有写入、且首条消息 session_id 匹配父会话的子 agent 文件数
pub(crate) fn count_active_subagents(project_dir: &Path, parent_session_id: &str) -> usize {
    use std::time::{Duration, SystemTime};
    let threshold = Duration::from_secs(30);
    let now = SystemTime::now();
    fs::read_dir(project_dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| is_subagent_file(&e.path()))
        .filter(|e| {
            e.metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|m| now.duration_since(m).ok())
                .map(|d| d < threshold)
                .unwrap_or(false)
        })
        .filter(|e| {
            let file = File::open(e.path()).ok();
            file.and_then(|f| {
                BufReader::new(f)
                    .lines()
                    .take(5)
                    .flatten()
                    .find_map(|line| serde_json::from_str::<JsonlMessage>(&line).ok())
                    .and_then(|m| m.session_id)
                    .map(|id| id == parent_session_id)
            })
            .unwrap_or(false)
        })
        .count()
}

/// 枚举目录内最近修改的会话 JSONL 文件（排除子 agent 文件，按 mtime 倒序）
pub(crate) fn get_recent_jsonl_files(project_dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<_> = fs::read_dir(project_dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| {
            let p = e.path();
            p.extension().map(|ext| ext == "jsonl").unwrap_or(false) && !is_subagent_file(&p)
        })
        .filter_map(|e| {
            let path = e.path();
            let modified = e.metadata().and_then(|m| m.modified()).ok()?;
            Some((path, modified))
        })
        .collect();
    files.sort_by_key(|f| std::cmp::Reverse(f.1));
    files.into_iter().map(|(p, _)| p).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// issue #35-6：头部读取按文件顺序返回，行数上限生效，缺失文件 → 空集
    #[test]
    fn read_first_lines_returns_head_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.jsonl");
        std::fs::write(&path, "l1\nl2\nl3\nl4\n").unwrap();
        assert_eq!(
            read_first_lines(&path, 2),
            vec!["l1".to_string(), "l2".to_string()]
        );
        // 上限宽于文件行数 → 全量返回
        assert_eq!(read_first_lines(&path, 10).len(), 4);
        // 文件缺失 → 空集（不 panic）
        assert!(read_first_lines(&dir.path().join("missing.jsonl"), 5).is_empty());
    }
}
