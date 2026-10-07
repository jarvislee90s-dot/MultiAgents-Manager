//! 增量流式读取：**本域唯一的文件读入口**（采集器一律经它读文件）。
//! 「一次流式扫描产出全部指标」由此保证：一个文件一轮只读一遍，读出的行交给采集器
//! **一次遍历**同时累计四桶/turn/工具/报错/时长（见各采集器的单遍循环与
//! `CollectContext::repeat_reads` 断言）。
//!
//! 契约细节（Task 11–17 的采集器依赖这三条）：
//! * `session_id` 参数实为**游标行键**（`CollectContext` 传的是 `file_key_of(root, path)`
//!   的路径派生键；SQLite 型源传会话 id / 表名水位键）——它就是 `usage_cursor.session_id`；
//! * `rescan == true` ⇒ 本轮是全量重读（首读 / 截断 / 重写），调用方**必须重置**该会话累计：
//!   `next.ordinal` 已按新文件重算，`next.state_json` 与 `next.last_cumulative` 已清零
//!   （不得把上一代文件的续读状态算到新文件上）；
//! * `rescan == false` 且 `lines.is_empty()` ⇒ 本轮没有完整新行（未变文件，或只追加了半行）：
//!   调用方应把 `next` **原样推回游标**（`CursorDelta.state_json` 是**透传容器**，本层不解释它；
//!   采集器处理完本轮后把新状态覆写进去，见 `delta::CursorDelta.state_json` 文档）。
//!
//! ## ⚠️ 游标里的代际是**读取前**的快照（不得改成「读取后现算」）
//! `next.mtime_ms` / `next.file_size` 一律取自**本轮入口那一次 `stat`**（`read_incremental_impl`
//! 的 `gen`）。理由：读取期间文件可能继续被写（追加路是一次 `read_to_end`；全量路还夹着整文件行
//! 物化，大文件是秒级窗口）。若在读完后再 `stat` 一次写进游标，就会出现「`byte_offset` 停在旧位置、
//! `file_size` 已是新代际」的组合，而下一轮 `file_unchanged` 只看 `(mtime, size)` →
//! 直接命中快速路 → `[byte_offset, file_size)` 里的**完整行永不被读**（会话随后安静就永久缺）。
//! 记「读取前」的代际则天然安全：读取期间只要发生过写入，当前代际必然与游标里的不符 →
//! 下一轮不会走快速路，那段字节一定会被读。
//! 由此 `byte_offset > file_size` 是**合法状态**（读取期间文件还在长），不代表数据错乱。
//!
//! 锁：`cursor_records_the_pre_read_generation_so_late_appends_are_not_skipped`
//! （经 `read_incremental_impl` 的 `after_read` 缝确定性地复现「读完之后才落盘的那一行」）。
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use super::cursor::{file_unchanged, needs_rescan, stat_of, tail_fingerprint};
use super::delta::CursorDelta;

#[derive(Debug, Clone, PartialEq)]
pub struct IncrementalRead {
    /// 本轮新增的**完整行**（未闭合尾行不在此列）
    pub lines: Vec<String>,
    pub next: CursorDelta,
    /// true = 本轮是全量重读（首读 / 截断 / 重写）→ 调用方必须重置该会话累计
    pub rescan: bool,
    /// 本轮真实读取的字节数（诊断与测试断言用；未变文件必须为 0）
    pub bytes_read: u64,
}

/// 只在最后一个 '\n' 处切行；返回 (完整行, 相对 consumed_base 的消费字节数)。
/// **纯函数**：空行不进结果但字节照样消费（否则水位会被空行卡住、每轮重读同一段）；
/// 非法 UTF-8 走 `from_utf8_lossy`（降级为 U+FFFD，不 panic、不丢行）。
pub fn split_complete_lines(buf: &[u8], consumed_base: u64) -> (Vec<String>, u64) {
    let Some(pos) = buf.iter().rposition(|b| *b == b'\n') else {
        return (Vec::new(), consumed_base);
    };
    let complete = &buf[..=pos];
    let lines = complete
        .split(|b| *b == b'\n')
        .filter(|l| !l.is_empty())
        .map(|l| String::from_utf8_lossy(l).to_string())
        .collect();
    (lines, consumed_base + complete.len() as u64)
}

fn next_cursor(
    path: &Path,
    session_id: &str,
    byte_offset: u64,
    added_lines: usize,
    prev: Option<&CursorDelta>,
    // **读取前**（入口 stat）的代际 —— **不得**改成「读取后现算」：读取期间若有写入，
    // 游标就会记下比 `byte_offset` 更靠后的代际，下一轮 `file_unchanged` 直接命中快速路，
    // `[byte_offset, file_size)` 里的完整行**永不被读**（静默少算，见 fix report 裁决 A）。
    gen: (i64, u64),
) -> std::io::Result<CursorDelta> {
    let (mtime_ms, file_size) = gen;
    let fp = tail_fingerprint(path, byte_offset)?;
    Ok(CursorDelta {
        session_id: session_id.to_string(),
        fingerprint: fp,
        byte_offset: byte_offset as i64,
        ordinal: prev.map(|p| p.ordinal).unwrap_or(0) + added_lines as i64,
        last_cumulative: prev.and_then(|p| p.last_cumulative),
        mtime_ms,
        // 任务书给的 `file_size,` 是 **E0308**（`stat_of` 回 u64、`CursorDelta.file_size` 是 i64）；
        // **最小修复**：边界转换，语义一字未改（见 task-9-report.md「偏差申报」）。
        file_size: file_size as i64,
        // 续读状态由采集器在本轮处理完后覆写（stream 层不解释它，只负责透传）
        state_json: prev.map(|p| p.state_json.clone()).unwrap_or_default(),
    })
}

pub fn read_incremental(
    path: &Path,
    session_id: &str,
    prev: Option<&CursorDelta>,
) -> std::io::Result<IncrementalRead> {
    read_incremental_impl(path, session_id, prev, || {})
}

/// `read_incremental` 的唯一实现。`after_read` 是**只给测试用的空操作缝**：
/// 它在「读阶段已结束、游标定稿之前」恰好被调用一次 —— 真实世界里这个窗口里可能发生写入
/// （追加路是一次 `read_to_end`；全量路还夹着整文件行物化，大文件是秒级窗口）。
/// 生产入口传 `|| {}`，**零行为差异**；它存在的唯一目的是让
/// `cursor_records_the_pre_read_generation_so_late_appends_are_not_skipped`
/// 能在单线程里确定性地复现那场竞态（否则该竞态只能靠碰运气）。
fn read_incremental_impl(
    path: &Path,
    session_id: &str,
    prev: Option<&CursorDelta>,
    after_read: impl FnOnce(),
) -> std::io::Result<IncrementalRead> {
    // 读取**前**的代际：整轮只取这一次 stat，并以它作为游标里的文件代际（裁决 A）
    let gen = stat_of(path)?;
    let (mtime_ms, size) = gen;

    if let Some(p) = prev {
        if file_unchanged(mtime_ms, size, p) {
            // L2 语义：内容未变 → 零读取
            return Ok(IncrementalRead {
                lines: Vec::new(),
                next: p.clone(),
                rescan: false,
                bytes_read: 0,
            });
        }
        if !needs_rescan(path, p)? {
            // 追加读：只读 [byte_offset, EOF)
            let start = p.byte_offset.max(0) as u64;
            let mut f = std::fs::File::open(path)?;
            f.seek(SeekFrom::Start(start))?;
            let mut buf = Vec::new();
            f.read_to_end(&mut buf)?;
            let bytes_read = buf.len() as u64;
            let (lines, consumed) = split_complete_lines(&buf, start);
            let added = lines.len();
            after_read();
            return Ok(IncrementalRead {
                lines,
                next: next_cursor(path, session_id, consumed, added, Some(p), gen)?,
                rescan: false,
                bytes_read,
            });
        }
    }

    // 全量（首读 / 截断 / 重写）
    let buf = std::fs::read(path)?;
    let bytes_read = buf.len() as u64;
    let (lines, consumed) = split_complete_lines(&buf, 0);
    let added = lines.len();
    after_read();
    Ok(IncrementalRead {
        lines,
        next: next_cursor(path, session_id, consumed, added, None, gen)?,
        rescan: true,
        bytes_read,
    })
}

// ==== usage 自省锁：以下为排除区（测试代码），勿删勿复制 ====
#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str, content: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("mam_usage_stream_{}_{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("wire.jsonl");
        std::fs::write(&p, content).unwrap();
        p
    }
    fn append(p: &std::path::Path, s: &str) {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new().append(true).open(p).unwrap();
        f.write_all(s.as_bytes()).unwrap();
    }

    /// 首读 = 全量（rescan=true），水位推进到最后一个完整行之后
    #[test]
    fn first_read_is_full_and_stops_at_last_complete_line() {
        let p = tmp("first", "a\nb\nc\n");
        let r = read_incremental(&p, "s1", None).unwrap();
        assert_eq!(r.lines, vec!["a", "b", "c"]);
        assert!(r.rescan);
        assert_eq!(r.next.byte_offset, 6);
        assert_eq!(r.next.ordinal, 3);
        assert!(!r.next.fingerprint.is_empty());
        assert_eq!(r.bytes_read, 6);
    }

    /// 追加只读新增字节（**增量**：不重读文件头）——codex 2.21 GB 全靠这条
    #[test]
    fn append_reads_only_new_bytes() {
        let p = tmp("append", "a\nb\n");
        let first = read_incremental(&p, "s1", None).unwrap();
        append(&p, "c\nd\n");
        let second = read_incremental(&p, "s1", Some(&first.next)).unwrap();
        assert_eq!(second.lines, vec!["c", "d"]);
        assert!(!second.rescan);
        assert_eq!(second.bytes_read, 4, "只允许读新增的 4 字节");
        assert_eq!(second.next.byte_offset, 8);
        assert_eq!(second.next.ordinal, 4);
    }

    /// (mtime,size) 未变 → **零读取**（L2 内容摘要语义，不打开文件）
    #[test]
    fn unchanged_file_is_not_read_at_all() {
        let p = tmp("unchanged", "a\nb\n");
        let first = read_incremental(&p, "s1", None).unwrap();
        let again = read_incremental(&p, "s1", Some(&first.next)).unwrap();
        assert!(again.lines.is_empty());
        assert_eq!(again.bytes_read, 0, "未变文件必须零字节读取");
        assert_eq!(again.next.byte_offset, first.next.byte_offset);
    }

    /// 文件被截断（外部重写/轮转）→ rescan=true，全量重读，调用方必须重置该会话累计
    #[test]
    fn truncation_triggers_rescan() {
        let p = tmp("trunc", "aaaa\nbbbb\ncccc\n");
        let first = read_incremental(&p, "s1", None).unwrap();
        std::fs::write(&p, "x\n").unwrap();
        let r = read_incremental(&p, "s1", Some(&first.next)).unwrap();
        assert!(r.rescan, "截断必须全量重读");
        assert_eq!(r.lines, vec!["x"]);
        assert_eq!(r.next.byte_offset, 2);
    }

    /// 同大小重写（尾指纹不符）→ rescan=true；尾指纹是 O(1) 校验（只读窗口 64 字节）
    #[test]
    fn equal_size_rewrite_is_detected_by_tail_fingerprint() {
        let p = tmp("rewrite", "aaaa\nbbbb\n");
        let first = read_incremental(&p, "s1", None).unwrap();
        // 说明：快速路 (mtime_ms,size) 同刻同大小时会判定"未变"——本场景的工具日志只追加，
        // 同毫秒同大小重写不会发生；这里 sleep 3ms 让 mtime 明确前进，从而走到尾指纹判定分支
        std::thread::sleep(std::time::Duration::from_millis(3));
        std::fs::write(&p, "aaaa\nZZZZ\n").unwrap(); // 同样 10 字节
                                                     // 前提断言（评审 Minor M2）：mtime 必须真前进 —— 否则本用例走的是**快速路**而不是尾指纹
                                                     // 分支，在粗粒度 mtime 的文件系统上会以「assert!(r.rescan) 失败」的形式**假红**
                                                     // （不是假绿：本用例断言的正是「必须 rescan」，走错分支会直接失败）。
        let (_, size_now) = assert_mtime_advanced(&p, first.next.mtime_ms);
        assert_eq!(size_now, 10, "前提：同大小重写");
        let r = read_incremental(&p, "s1", Some(&first.next)).unwrap();
        assert!(r.rescan);
        assert_eq!(r.lines, vec!["aaaa", "ZZZZ"]);
    }

    /// 半行不入账：未闭合尾行留给下一轮（避免 JSON 半行解析失败）
    #[test]
    fn partial_tail_line_waits_for_next_round() {
        let p = tmp("partial", "a\nb\n{\"partial\"");
        let first = read_incremental(&p, "s1", None).unwrap();
        assert_eq!(first.lines, vec!["a", "b"]);
        assert_eq!(first.next.byte_offset, 4, "半行不得计入水位");
        append(&p, ":1}\n");
        let second = read_incremental(&p, "s1", Some(&first.next)).unwrap();
        assert_eq!(second.lines, vec!["{\"partial\":1}"]);
    }

    /// 纯函数：split_complete_lines 只在最后一个 '\n' 处切
    #[test]
    fn split_lines_only_at_newline() {
        let (lines, consumed) = split_complete_lines(b"a\nb\nc", 0);
        assert_eq!(lines, vec!["a", "b"]);
        assert_eq!(consumed, 4);
        let (none, c0) = split_complete_lines(b"no newline", 0);
        assert!(none.is_empty());
        assert_eq!(c0, 0);
    }

    // ------------------------------------------------------------------
    // 以下为本任务新增的回归锁（任务书只给了上面 7 条；判据逐条对锁见 task-9-report.md
    // 的「回归锁对照表」。每条都实测过「改坏判据 → 变红」，变异记录见报告）。
    // ------------------------------------------------------------------

    /// 前提断言：mtime 必须真的前进。快速路 (mtime_ms,size) 同刻同大小时会判「未变」，
    /// 若不校验 mtime 真前进，走尾指纹分支的用例会静默走错分支：
    /// 断言「必须重扫」的用例会**假红**（`assert!(r.rescan)` 直接失败），
    /// 断言「不该重扫 / 零读取」的用例会**假绿**（走快速路也满足期望）。
    /// 两种都不该以「莫名其妙红/绿」收场，故一律先断前提（评审 Minor M2）。
    fn assert_mtime_advanced(p: &std::path::Path, prev_mtime_ms: i64) -> (i64, u64) {
        let (mtime_now, size_now) = stat_of(p).unwrap();
        assert_ne!(
            mtime_now, prev_mtime_ms,
            "前提：mtime 必须前进 —— 否则本用例根本没走到尾指纹判定分支（断言「必须重扫」的会假红、断言「不该重扫」的会假绿）"
        );
        (mtime_now, size_now)
    }

    /// GC 4「同一文件只允许读一遍」在本层的账：多轮读取字节数之和 == 文件最终字节数。
    /// 任何一轮重读旧字节（或未变文件仍读）都会让总账 > 文件大小。
    #[test]
    fn append_rounds_never_reread_a_byte() {
        let p = tmp("once", "a\nb\n");
        let r1 = read_incremental(&p, "s1", None).unwrap();
        let mut total = r1.bytes_read;
        let mut cur = r1.next.clone();
        assert_eq!(total, 4, "首轮 = 全量 = 文件大小");
        assert_eq!(r1.next.ordinal, 2);

        append(&p, "c\n");
        let r2 = read_incremental(&p, "s1", Some(&cur)).unwrap();
        assert_eq!(r2.lines, vec!["c"]);
        assert_eq!(r2.bytes_read, 2, "只读新增字节");
        total += r2.bytes_read;
        cur = r2.next.clone();

        let r3 = read_incremental(&p, "s1", Some(&cur)).unwrap();
        assert_eq!(r3.bytes_read, 0, "未变文件零读取");
        assert_eq!(r3.next, cur, "未变文件必须原样返回上一轮游标");
        total += r3.bytes_read;

        append(&p, "d\ne\n");
        let r4 = read_incremental(&p, "s1", Some(&cur)).unwrap();
        assert_eq!(r4.lines, vec!["d", "e"]);
        assert_eq!(r4.bytes_read, 4);
        total += r4.bytes_read;

        assert_eq!(stat_of(&p).unwrap().1, 10, "4 + 2 + 4 字节");
        assert_eq!(
            total, 10,
            "多轮读取字节数之和必须等于文件字节数（同一文件只读一遍）"
        );
        assert_eq!(r4.next.byte_offset, 10);
        assert_eq!(
            r4.next.ordinal, 5,
            "双水位第二维：跨轮累计完整行数（a,b | c | d,e = 5 行）"
        );
        std::fs::remove_dir_all(p.parent().unwrap()).ok();
    }

    /// 仅 mtime 前进（内容一字未改）→ **不得**判重扫：追加路读 0 字节，并把新代际写回游标
    /// （下一轮才走快速路）。这是尾指纹判定的**反向对照**：只看 mtime 就重扫的实现会红。
    #[test]
    fn mtime_only_change_settles_with_zero_read_in_one_round() {
        let p = tmp("touch", "aaaa\nbbbb\n");
        let first = read_incremental(&p, "s1", None).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(3));
        std::fs::write(&p, "aaaa\nbbbb\n").unwrap(); // 逐字节相同
        let (mtime_now, size_now) = assert_mtime_advanced(&p, first.next.mtime_ms);
        assert_eq!(size_now, first.next.file_size as u64, "前提：同大小");
        assert!(
            !needs_rescan(&p, &first.next).unwrap(),
            "内容未变 ⇒ 不得重扫（只看 mtime 就重扫 = 假阳性，会把 codex 打回全量）"
        );
        let r = read_incremental(&p, "s1", Some(&first.next)).unwrap();
        assert!(!r.rescan, "仅 mtime 变化不得判为截断/重写");
        assert!(r.lines.is_empty());
        assert_eq!(r.bytes_read, 0, "水位已在 EOF，追加路读 0 字节");
        assert_eq!(r.next.byte_offset, first.next.byte_offset);
        assert_eq!(
            r.next.mtime_ms, mtime_now,
            "新代际必须写回游标，下一轮才能走快速路"
        );
        let again = read_incremental(&p, "s1", Some(&r.next)).unwrap();
        assert_eq!(again.bytes_read, 0);
        assert_eq!(again.next, r.next, "新代际写回后，下一轮命中快速路");
        std::fs::remove_dir_all(p.parent().unwrap()).ok();
    }

    /// 前缀被改写**且**文件同时变长（追加式改写）→ 尾指纹不符 → rescan。
    /// 这一支 size 比较救不了（size 更大），只有尾指纹能抓。
    #[test]
    fn prefix_rewrite_that_also_grows_is_detected() {
        let p = tmp("grow-rewrite", "aaaa\nbbbb\n");
        let first = read_incremental(&p, "s1", None).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(3));
        std::fs::write(&p, "xxxx\nbbbb\ncccc\n").unwrap();
        let (_, size_now) = assert_mtime_advanced(&p, first.next.mtime_ms);
        assert_eq!(size_now, 15);
        assert!(
            size_now > first.next.byte_offset as u64,
            "前提：不是截断（size 更大），只能靠尾指纹抓"
        );
        assert!(needs_rescan(&p, &first.next).unwrap());
        let r = read_incremental(&p, "s1", Some(&first.next)).unwrap();
        assert!(r.rescan, "前缀改写 + 变长也必须全量重读");
        assert_eq!(r.lines, vec!["xxxx", "bbbb", "cccc"]);
        assert_eq!(r.next.byte_offset, 15);
        assert_eq!(r.next.ordinal, 3);
        std::fs::remove_dir_all(p.parent().unwrap()).ok();
    }

    /// 截断/轮转 → rescan + **两个水位与续读状态一起重置**（不得继承上一代的 ordinal /
    /// state_json / last_cumulative —— 继承了就是拿旧文件的续读状态算新文件）。
    #[test]
    fn truncation_resets_both_watermarks_and_carry_over_state() {
        let p = tmp("trunc-reset", "a\nb\nc\n");
        let first = read_incremental(&p, "s1", None).unwrap();
        assert_eq!(first.next.byte_offset, 6);
        assert_eq!(first.next.ordinal, 3);
        // 模拟采集器把续读状态写回游标（真实形态见 Task 11 的 push_cursor(next)）
        let mut carried = first.next.clone();
        carried.state_json = r#"{"last_model":"m1"}"#.to_string();
        carried.last_cumulative = Some(42);
        std::fs::write(&p, "z\n").unwrap();
        let r = read_incremental(&p, "s1", Some(&carried)).unwrap();
        assert!(r.rescan);
        assert_eq!(r.lines, vec!["z"]);
        assert_eq!(r.next.byte_offset, 2);
        assert_eq!(
            r.next.ordinal, 1,
            "ordinal 必须按新文件重算，不得在旧值上累加"
        );
        assert_eq!(
            r.next.state_json, "",
            "全量重读 ⇒ 续读状态清零（不得继承上一代）"
        );
        assert_eq!(r.next.last_cumulative, None, "全量重读 ⇒ 累计基线清零");
        std::fs::remove_dir_all(p.parent().unwrap()).ok();
    }

    /// `state_json` / `last_cumulative` 是**透传容器**：本轮没有完整新行（只追加了半行）时
    /// 必须原样带过去 —— 采集器在那一轮会把 `read.next` 原封不动推回游标（Task 11 的
    /// `if read.lines.is_empty() && !read.rescan { push_cursor(read.next); continue; }`），
    /// 清空它就等于把模型继承 / 未配对工具调用的续读状态**永久丢失**。
    #[test]
    fn carry_over_state_survives_append_rounds() {
        let p = tmp("state", "a\nb\n");
        let r1 = read_incremental(&p, "s1", None).unwrap();
        let mut carried = r1.next.clone();
        carried.state_json = r#"{"last_model":"m1","pending_tool":"t1"}"#.to_string();
        carried.last_cumulative = Some(1234);
        // 第 2 轮：只追加一个**半行**（无 '\n'）→ 无完整行，游标原样回推
        append(&p, "{\"partial\"");
        let r2 = read_incremental(&p, "s1", Some(&carried)).unwrap();
        assert!(r2.lines.is_empty());
        assert!(!r2.rescan);
        assert_eq!(r2.bytes_read, 10, "半行字节要读进来但不得入账");
        assert_eq!(r2.next.byte_offset, carried.byte_offset, "半行不得推进水位");
        assert_eq!(
            r2.next.state_json, carried.state_json,
            "无完整新行的一轮必须透传续读状态"
        );
        assert_eq!(r2.next.last_cumulative, Some(1234));
        // 第 3 轮：补齐这一行
        append(&p, ":1}\n");
        let r3 = read_incremental(&p, "s1", Some(&r2.next)).unwrap();
        assert_eq!(r3.lines, vec!["{\"partial\":1}"]);
        assert_eq!(r3.next.byte_offset, 18);
        assert_eq!(
            r3.next.ordinal, 3,
            "半行不计入 ordinal（第 2 轮 0 行 + 第 3 轮 1 行）"
        );
        assert_eq!(
            r3.next.state_json, carried.state_json,
            "有完整行的一轮同样透传（采集器随后覆写）"
        );
        assert_eq!(r3.next.last_cumulative, Some(1234));
        std::fs::remove_dir_all(p.parent().unwrap()).ok();
    }

    /// 文件消失/读不到 → `Err`（采集器据此 log::warn + 跳过该文件），**不得**静默当成「无新数据」。
    #[test]
    fn missing_file_is_an_error_not_an_empty_read() {
        let p = tmp("missing", "a\n");
        let cur = read_incremental(&p, "s1", None).unwrap().next;
        let ghost = p.parent().unwrap().join("gone.jsonl");
        assert!(!ghost.exists(), "前提：探针文件确实不存在");
        assert!(read_incremental(&ghost, "s1", None).is_err());
        assert!(read_incremental(&ghost, "s1", Some(&cur)).is_err());
        std::fs::remove_dir_all(p.parent().unwrap()).ok();
    }

    /// **已注明的快速路盲区**：同毫秒 + 同大小的重写对 (mtime_ms,size) 不可见 → 零字节读取。
    /// 工具日志只追加，故本场景在真实源上不发生（见 cursor.rs 模块文档「已知盲区」）；
    /// 本用例把这条限制从「注释里的一句话」变成可执行的判定，并给出「若真去算尾指纹本会不符」
    /// 的前提证据。**若将来决定在快速路上也校验尾指纹，请改写本用例并重新评估 codex
    /// 2.21 GB 路径的代价，不要为了让用例变绿而放宽它。**
    #[test]
    fn same_millisecond_same_size_rewrite_is_the_documented_fast_path_blind_spot() {
        let p = tmp("blindspot", "aaaa\nbbbb\n");
        let first = read_incremental(&p, "s1", None).unwrap();
        assert_eq!(first.next.byte_offset, 10);
        std::fs::write(&p, "aaaa\nZZZZ\n").unwrap(); // 同大小改写
        let (mtime_now, size_now) = stat_of(&p).unwrap();
        // 把游标强行对齐到「当前代际」= 确定性地模拟同毫秒写入（不靠时序碰运气）
        let mut blind = first.next.clone();
        blind.mtime_ms = mtime_now;
        blind.file_size = size_now as i64;
        // 前提①：内容确实被改写了（此刻若真去算尾指纹，是会不符的）
        assert_eq!(size_now, 10, "前提：同大小");
        assert!(
            needs_rescan(&p, &blind).unwrap(),
            "前提：尾指纹能看出版本差异 —— 盲区只在快速路，不在判定层"
        );
        assert!(
            file_unchanged(mtime_now, size_now, &blind),
            "前提：快速路确实判「未变」（这正是盲区）"
        );
        let r = read_incremental(&p, "s1", Some(&blind)).unwrap();
        assert_eq!(
            r.bytes_read, 0,
            "同毫秒同大小重写走快速路 → 0 字节（盲区，见 cursor.rs 文档）"
        );
        assert!(r.lines.is_empty());
        assert_eq!(r.next.byte_offset, blind.byte_offset);
        std::fs::remove_dir_all(p.parent().unwrap()).ok();
    }

    /// **已注明的残留盲区之二（尾指纹是抽样判定）**：改写发生在**窗口之外**（离水位 >64 字节的
    /// 前部）、且窗口内那 64 字节一字未变 → `needs_rescan` 判「无需重扫」，水位照常推进。
    /// 只追加的工具日志不会出现这种形态（见 cursor.rs 模块文档）；本用例把这条限制也变成
    /// 可执行判定，并给出双侧对照：**碰了窗口就一定抓得住**。
    #[test]
    fn front_rewrite_outside_the_tail_window_is_the_documented_residual_blind_spot() {
        let head = 80usize;
        // 末 64 字节（= 尾窗口）用 63 个 'T' + '\n' 铺满：末尾必须有 '\n' 水位才会推进到 EOF
        let tail = format!("{}\n", "T".repeat(63));
        let old = format!("{}{tail}", "A".repeat(head));
        let p = tmp("front-rewrite", &old);
        let first = read_incremental(&p, "s1", None).unwrap();
        let up = first.next.byte_offset as u64;
        assert_eq!(up, 144);
        assert!(up > 64, "前提：窗口起点在文件中部，前段确实在窗口之外");
        std::thread::sleep(std::time::Duration::from_millis(3));
        let new = format!("{}{tail}", "Z".repeat(head));
        std::fs::write(&p, &new).unwrap();
        let (_, size_now) = assert_mtime_advanced(&p, first.next.mtime_ms);
        // 前提①：同大小、且前 80 字节确实从 'A' 被改写成了 'Z'
        assert_eq!(size_now, up);
        let on_disk = std::fs::read(&p).unwrap();
        assert!(
            on_disk.starts_with(&vec![b'Z'; head]) && !on_disk.starts_with(b"AAAA"),
            "前提：窗口外的那 80 字节确实被改写了"
        );
        assert_eq!(
            tail_fingerprint(&p, up).unwrap(),
            first.next.fingerprint,
            "前提：窗口内（末 64 字节）一字未变，故尾指纹判不出这次改写"
        );
        assert!(
            !needs_rescan(&p, &first.next).unwrap(),
            "前提：判定层判「无需重扫」（这正是残留盲区）"
        );
        let r = read_incremental(&p, "s1", Some(&first.next)).unwrap();
        assert!(!r.rescan);
        assert_eq!(
            r.bytes_read, 0,
            "窗口外的改写看不到 → 0 字节（盲区，见 cursor.rs 文档）"
        );
        assert!(r.lines.is_empty());
        // 对照：改写一旦碰到窗口（末字节变了），同一套判定必须抓住
        let mut touched = new.clone().into_bytes();
        *touched.last_mut().unwrap() = b'X';
        std::fs::write(&p, &touched).unwrap();
        assert!(
            needs_rescan(&p, &r.next).unwrap(),
            "对照：窗口内改一字节必须判重扫（否则指纹形同虚设）"
        );
        std::fs::remove_dir_all(p.parent().unwrap()).ok();
    }

    /// **裁决 A 的锁：游标记的是「读取前」的代际。**
    /// 用 `read_incremental_impl` 的 `after_read` 缝在单线程里确定性复现真实竞态：
    /// 入口 stat 时文件 4 字节（`b\n` 刚落），本轮读走 `[2,4)` 之后、游标定稿之前，
    /// 写入器又落了完整一行 `"c\n"`（磁盘 6 字节）。
    /// 修复前（`next_cursor` 内部再 stat 一次）：游标记成读取后的 (m2, 6)，而 `byte_offset` 停在 4
    /// → 下一轮 `file_unchanged` 命中快速路 → `[4,6)` 里的 `"c"` **永不被读**（静默少算）。
    /// 修复后：游标记 (m1, 4) ≠ 当前代际 → 下一轮必然走追加路，把那行补读回来。
    #[test]
    fn cursor_records_the_pre_read_generation_so_late_appends_are_not_skipped() {
        let p = tmp("gen-race", "a\n");
        let r1 = read_incremental(&p, "s1", None).unwrap();
        let c1 = r1.next.clone();
        assert_eq!(
            (c1.byte_offset, c1.file_size),
            (2, 2),
            "首读：水位与代际都在 2"
        );

        // 入口 stat 之前落盘的一行：让本轮**必须**离开快速路、进入读阶段
        append(&p, "b\n");
        let (m1, s1) = stat_of(&p).unwrap();
        assert_eq!(s1, 4);
        assert!(
            !file_unchanged(m1, s1, &c1),
            "前提：(mtime,size) 已与游标不符 ⇒ 本轮不得走快速路（size 2→4 已足以判「已变」）"
        );

        // 本轮：入口快照 = (m1, 4)；`after_read` 里落盘的那一行 = 「读取期间发生的写入」
        let late = p.clone();
        let r2 = read_incremental_impl(&p, "s1", Some(&c1), move || append(&late, "c\n")).unwrap();
        assert!(!r2.rescan);
        assert_eq!(r2.lines, vec!["b"], "本轮只读到入口快照那一刻已有的那一行");
        assert_eq!(r2.next.byte_offset, 4, "水位只能推进到本轮真正读到的完整行");
        // ★ 命门：必须记「读取前」的代际（修复前这里是读取后的 (m2, 6)）
        assert_eq!(r2.next.file_size, 4, "游标必须记读取前的 file_size");
        assert_eq!(r2.next.mtime_ms, m1, "游标必须记读取前的 mtime");

        // 前提：磁盘上确实多了那一行，且当前代际与游标不符（否则下一轮走快速路是合理的）
        let (now_mtime, now_size) = stat_of(&p).unwrap();
        assert_eq!(now_size, 6, "前提：这一行确实在本轮读取之后才落盘");
        assert!(
            !file_unchanged(now_mtime, now_size, &r2.next),
            "前提：当前代际与游标不符 ⇒ 下一轮不得走快速路"
        );
        assert!(
            !needs_rescan(&p, &r2.next).unwrap(),
            "前提：不是截断/重写，只能靠代际不符补读"
        );

        // 下一轮：磁盘没再变，但那一行必须被读到（修复前永久漏掉）
        let r3 = read_incremental(&p, "s1", Some(&r2.next)).unwrap();
        assert!(!r3.rescan);
        assert_eq!(r3.lines, vec!["c"], "「读取期间落盘」的那一行必须补读回来");
        assert_eq!(r3.bytes_read, 2);
        assert_eq!(r3.next.byte_offset, 6);
        std::fs::remove_dir_all(p.parent().unwrap()).ok();
    }

    /// **裁决 B 的锁：`next.session_id` 是游标行键**（`usage_cursor.session_id`）。
    /// Task 10+ 的 `push_cursor(read.next)` 直接落库：键写空会让所有文件的游标挤进同一行
    /// 或写进查不回的行（要么每轮全量重扫、要么跨文件串味）。四条路径逐条锁。
    #[test]
    fn cursor_row_key_is_the_file_key_on_all_four_paths() {
        let p = tmp("row-key", "a\nb\n");
        // ① 首读（全量路）
        let r1 = read_incremental(&p, "s1", None).unwrap();
        assert!(r1.rescan);
        assert_eq!(r1.next.session_id, "s1", "全量路：游标行键必须是 file_key");
        // ② 追加读
        append(&p, "c\n");
        let r2 = read_incremental(&p, "s1", Some(&r1.next)).unwrap();
        assert!(!r2.rescan);
        assert_eq!(r2.lines, vec!["c"]);
        assert_eq!(r2.next.session_id, "s1", "追加路：游标行键必须是 file_key");
        // ③ 快速路（未变 → 原样复用上一轮游标）
        let r3 = read_incremental(&p, "s1", Some(&r2.next)).unwrap();
        assert_eq!(r3.bytes_read, 0);
        assert_eq!(r3.next.session_id, "s1", "快速路：游标行键必须是 file_key");
        // ④ rescan（截断 → 全量重读）
        std::fs::write(&p, "z\n").unwrap();
        let r4 = read_incremental(&p, "s1", Some(&r3.next)).unwrap();
        assert!(r4.rescan);
        assert_eq!(
            r4.next.session_id, "s1",
            "rescan 后：游标行键必须是 file_key"
        );
        std::fs::remove_dir_all(p.parent().unwrap()).ok();
    }

    /// 纯函数边界：空 buf / 无 '\n' / consumed_base 透传 / 空行 / CRLF。
    #[test]
    fn split_complete_lines_covers_empty_blank_and_base_offset() {
        assert_eq!(split_complete_lines(b"", 7), (Vec::<String>::new(), 7));
        assert_eq!(split_complete_lines(b"abc", 7), (Vec::<String>::new(), 7));
        assert_eq!(
            split_complete_lines(b"a\n", 100),
            (vec!["a".to_string()], 102)
        );
        // 空行不进结果，但**字节照样被消费**（否则水位会被空行卡住、每轮重读同一段）
        let (lines, consumed) = split_complete_lines(b"a\n\nb\n", 0);
        assert_eq!(lines, vec!["a", "b"]);
        assert_eq!(consumed, 5);
        // '\r' 不做特殊处理（JSONL 里它属于行内容，由采集器的解析器决定）
        let (crlf, n) = split_complete_lines(b"a\r\n", 0);
        assert_eq!(crlf, vec!["a\r"]);
        assert_eq!(n, 3);
    }

    /// UTF-8：多字节字符整行保留；非法字节 → U+FFFD（不 panic、不丢整行）——日志偶有半截字节。
    #[test]
    fn split_complete_lines_is_lossy_not_panicking_on_invalid_utf8() {
        let (lines, n) = split_complete_lines("中文\n".as_bytes(), 0);
        assert_eq!(lines, vec!["中文"]);
        assert_eq!(n, 7, "3 + 3 + 1 字节");
        let (bad, n2) = split_complete_lines(&[0x41, 0xFF, 0x0A], 0);
        assert_eq!(bad.len(), 1);
        assert!(
            bad[0].contains('\u{FFFD}'),
            "非法字节必须降级为替换字符而不是丢行：{:?}",
            bad[0]
        );
        assert_eq!(n2, 3);
    }
}
// ==== usage 自省锁：排除区结束，勿删勿复制 ====
