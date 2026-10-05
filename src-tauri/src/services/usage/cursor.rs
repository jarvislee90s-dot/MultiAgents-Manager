//! 采集游标：字节双水位 + 尾指纹（借鉴 Codex APP `thread_history_projection_state` 的
//! `(next_rollout_byte_offset, next_rollout_ordinal)` 设计，说明书 §7.2）。
//! * `byte_offset`：已消费字节水位，**只在完整行边界推进**；
//! * `ordinal`：已消费完整行数（同一 offset 下的记录序号，第二维水位）；
//! * `fingerprint`：已消费前缀**末尾 ≤64 字节**的 sha256 —— O(1) 校验（只读 64 字节即可
//!   判断前缀有没有被改写），比"重哈希整个前缀"便宜几个数量级（codex 全库 2.21 GB
//!   每轮重哈希等于把增量扫描打回全量）；
//! * `mtime_ms`/`file_size`：文件代际，(mtime,size) 未变即**整文件跳过**（不打开）。
//!
//! **游标行类型只有 `delta::CursorDelta` 一个**（DAO 的 `load_cursors_conn` /
//! `save_cursors_conn` 直接收发它）——本模块不另造游标 struct。
//!
//! ## 判定层是纯函数，IO 只留在三个入口
//! `shorter_than_watermark` / `fingerprint_mismatch` / `file_unchanged` 不接路径、不碰文件系统，
//! 可脱离磁盘单测（GC 6「口径层唯一且纯」）；IO 只在 `stat_of` / `tail_fingerprint` /
//! `needs_rescan` 三个入口，且由 `needs_rescan` 把两者串成一条判定。
//!
//! ## ⚠️ 已知盲区（快速路不完美，但代价不对称，故**有意保留**、不得静默）
//! `file_unchanged` 只比 `(mtime_ms, file_size)`：**同一毫秒内、同一大小的重写**会被判成
//! 「未变」→ 本轮零读取、水位不推进，改写掉的内容漏掉。这不是疏忽：
//! 1. 本域七个文件型源都是**只追加**的工具日志（claude / codex / kimi / dsh / openclaw…），
//!    同毫秒同大小重写不会发生；真正会改写文件的是「轮转 / 截断」，那类场景 size 会变小、
//!    或尾指纹会变，两条判据都还在（`needs_rescan`）。
//! 2. 代价不对称：若在快速路上也校验尾指纹，每条游标每轮都要 open + seek + sha256 ——
//!    codex 真机 425 文件 / 2.21 GB，每轮多 425 次 open+read+哈希，正是 L2「(mtime,size)
//!    内容摘要缓存」（AGENTS.md 扫描预算契约）要省掉的那笔开销。
//! 3. 该盲区在 `stream.rs` 有**可执行的锁**：
//!    `same_millisecond_same_size_rewrite_is_the_documented_fast_path_blind_spot`。
//!
//! 尾指纹本身也是**抽样判定**，还有第二处（更弱、但同样存在）残留盲区：**改写发生在窗口之外**
//! （离水位 >64 字节的前部）而窗口内那 64 字节恰好没变 —— 例如同大小地把文件前段整体换掉、
//! 尾部保留。此时 `needs_rescan` 会判「无需重扫」、水位照常推进，被改写的那段永不复读。
//! 为什么接受：整前缀重哈希就是本任务要避免的全量退化（见上），而「前部被改写、尾部 64 字节
//! 恰好一字不差」在只追加的工具日志上不发生。该盲区同样有锁：
//! `front_rewrite_outside_the_tail_window_is_the_documented_residual_blind_spot`。
//! **若将来要收紧这两处盲区，先回契约与计划——不要就地加校验**（代价落在 codex 2.21 GB 路径上）。
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use sha2::{Digest, Sha256};

use super::delta::CursorDelta;

/// 尾指纹窗口（字节）
pub const TAIL_WINDOW: u64 = 64;

/// 读取 (mtime 毫秒, size)。
/// **mtime 拿不到 = 不可得，必须报错**（评审 Minor M4 / GC 7 同族）：绝不用 `0` 冒充合法时间戳 ——
/// 那会让该文件系统上所有游标都带 `mtime_ms == 0`，`file_unchanged` 静默退化成「只看 size」
/// （= 已知盲区的放大版），且**没有任何一处会报出来**。调用方（采集器）按「该文件本轮读失败」
/// `log::warn!` + 跳过，与其它 `io::Error` 同一条路径（见 `stream.rs` 的单文件失败处理）。
pub fn stat_of(path: &Path) -> std::io::Result<(i64, u64)> {
    let meta = std::fs::metadata(path)?;
    let modified = meta.modified()?;
    let mtime_ms = modified
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("mtime 早于 UNIX_EPOCH，无法表达为毫秒时间戳: {e}"),
            )
        })?
        .as_millis() as i64;
    Ok((mtime_ms, meta.len()))
}

/// 前缀末尾 ≤TAIL_WINDOW 字节的 sha256（十六进制小写）。
/// `up_to` = 已消费前缀的长度（**不是文件长度**）：文件继续追加不影响旧前缀的指纹；
/// `up_to == 0` → 空串（还没有已消费前缀，无可校验者）。
pub fn tail_fingerprint(path: &Path, up_to: u64) -> std::io::Result<String> {
    if up_to == 0 {
        return Ok(String::new());
    }
    let start = up_to.saturating_sub(TAIL_WINDOW);
    let mut f = std::fs::File::open(path)?;
    f.seek(SeekFrom::Start(start))?;
    let mut buf = Vec::new();
    f.take(up_to - start).read_to_end(&mut buf)?;
    let mut hasher = Sha256::new();
    hasher.update(&buf);
    Ok(format!("{:x}", hasher.finalize()))
}

/// (mtime,size) 未变 → 文件内容未变（L2 内容摘要缓存同款语义）→ 整文件跳过。
/// 纯函数（不碰文件系统）；同毫秒同大小重写的盲区见模块文档。
pub fn file_unchanged(mtime_ms: i64, size: u64, prev: &CursorDelta) -> bool {
    // 任务书给的 `prev.file_size == size` 是 **E0308 + E0277**（`CursorDelta.file_size` 是 i64、
    // `stat_of` 的 size 是 u64，Rust 没有跨整型 `PartialEq`）；**最小修复**：在边界处把 u64
    // 转成 i64 比较，判据与语义一字未改（见 task-9-report.md「偏差申报」）。文件大小不可能
    // 逼近 i64::MAX，转换无损。
    prev.mtime_ms == mtime_ms && prev.file_size == size as i64
}

/// 纯判定①：文件比水位短 = 截断 / 轮转 ⇒ 必须全量重读。
/// 快路（省一次 open+read）：这类文件连尾指纹都不用算就能定案。**注意**它语义上被尾指纹冗余
/// 覆盖（被哈希的字节数不同 ⇒ sha256 必不同），保留它是为省 IO，不是唯一判据（变异记录见报告）。
pub fn shorter_than_watermark(size: u64, prev: &CursorDelta) -> bool {
    size < prev.byte_offset.max(0) as u64
}

/// 纯判定②：已消费前缀的尾指纹与游标记录的不符 = 前缀被改写（同大小重写 / 追加式改写）⇒
/// 必须全量重读。空指纹（库里没有 / 水位为 0 之后的旧行）与实算指纹必然不等，方向正确。
pub fn fingerprint_mismatch(now_fp: &str, prev: &CursorDelta) -> bool {
    now_fp != prev.fingerprint
}

/// 是否需要全量重读：① 文件比水位短（截断/轮转）；② 前缀尾指纹不符（同大小或追加式改写）。
/// 判定本身是纯函数，这里只负责取 (size, 尾指纹) 两件事实。
pub fn needs_rescan(path: &Path, prev: &CursorDelta) -> std::io::Result<bool> {
    let (_, size) = stat_of(path)?;
    if shorter_than_watermark(size, prev) {
        return Ok(true);
    }
    if prev.byte_offset == 0 {
        return Ok(false);
    }
    let now_fp = tail_fingerprint(path, prev.byte_offset as u64)?;
    Ok(fingerprint_mismatch(&now_fp, prev))
}

// ==== usage 自省锁：以下为排除区（测试代码），勿删勿复制 ====
#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str, content: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("mam_usage_cursor_{}_{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("wire.jsonl");
        std::fs::write(&p, content).unwrap();
        p
    }

    fn cursor_of(byte_offset: i64, fingerprint: &str) -> CursorDelta {
        CursorDelta {
            byte_offset,
            fingerprint: fingerprint.to_string(),
            ..Default::default()
        }
    }

    /// 尾指纹窗口是**接口常量**（契约 § Interfaces：`TAIL_WINDOW: u64 = 64`），改动即改契约。
    #[test]
    fn tail_window_is_the_contract_value() {
        assert_eq!(TAIL_WINDOW, 64);
    }

    /// `stat_of` 的 mtime 是**毫秒**（不是秒）、size 是字节数；文件不存在时是 Err（不静默给 (0,0)）。
    #[test]
    fn stat_of_reports_millisecond_mtime_and_byte_size() {
        let p = tmp("stat", "0123456");
        let (mtime_ms, size) = stat_of(&p).unwrap();
        assert_eq!(size, 7);
        assert!(
            mtime_ms > 1_600_000_000_000,
            "mtime 必须是毫秒时间戳（秒级实现 ≈1.7e9 会在此变红），实得 {mtime_ms}"
        );
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        assert!((now_ms - mtime_ms).abs() < 60_000, "mtime 应接近当前时刻");
        let ghost = p.parent().unwrap().join("nope.jsonl");
        assert!(!ghost.exists(), "前提：目标文件确实不存在");
        assert!(
            stat_of(&ghost).is_err(),
            "文件不存在必须是 Err，不得静默返回 (0,0)"
        );
        std::fs::remove_dir_all(p.parent().unwrap()).ok();
    }

    /// **评审 Minor M4 的锁**：mtime 不可得（1970 之前 / FS 不报）时 `stat_of` 必须**报错**，
    /// 绝不用 `0` 冒充合法时间戳 —— 那会让所有游标都带 `mtime_ms == 0`，
    /// `file_unchanged` 静默退化成「只看 size」（= 已知盲区放大版）且无人知晓。
    #[test]
    fn stat_of_never_fakes_mtime_zero() {
        let p = tmp("pre-epoch", "x");
        let f = std::fs::File::options().write(true).open(&p).unwrap();
        let pre_epoch = std::time::UNIX_EPOCH - std::time::Duration::from_secs(3600);
        let set_ok = f
            .set_times(std::fs::FileTimes::new().set_modified(pre_epoch))
            .is_ok();
        drop(f);
        assert!(
            set_ok,
            "前提：本平台允许把 mtime 设到 1970 之前（否则本用例构造不出「mtime 不可得」）"
        );
        // 前提②：文件系统真的把它存成了 1970 之前（有的 FS 会夹取到 0，那就退化成另一回事）
        let meta = std::fs::metadata(&p).unwrap();
        assert!(
            meta.modified().unwrap() < std::time::UNIX_EPOCH,
            "前提：磁盘上的 mtime 确实早于 1970"
        );
        let err = stat_of(&p).expect_err("mtime 不可得时必须报错，绝不用 0 冒充合法时间戳");
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
        std::fs::remove_dir_all(p.parent().unwrap()).ok();
    }

    /// 尾指纹 = 已消费前缀**末尾 ≤64 字节**的 sha256（十六进制小写）；水位 0 → 空串。
    /// 双侧锁：窗口外的差异**不得**影响指纹（否则就是「重哈希整个前缀」，codex 2.21 GB 每轮全量）；
    /// 窗口内差 1 字节**必须**改变指纹（否则指纹没有鉴别力）。
    #[test]
    fn tail_fingerprint_is_bounded_to_the_last_tail_window_bytes() {
        let tail = "T".repeat(TAIL_WINDOW as usize);
        let a = tmp("tail-a", &format!("AAAAAA{tail}"));
        let b = tmp("tail-b", &format!("BBBBBB{tail}"));
        let c = tmp(
            "tail-c",
            &format!("AAAAAA{}{}", "T".repeat(TAIL_WINDOW as usize - 1), "Z"),
        );
        let up = 6 + TAIL_WINDOW;
        // 前提：三个文件同大小；a/b 只在窗口外（前 6 字节）不同；c 只在窗口内差 1 字节
        assert_eq!(stat_of(&a).unwrap().1, up);
        assert_eq!(stat_of(&b).unwrap().1, up);
        assert_eq!(stat_of(&c).unwrap().1, up);
        assert_ne!(
            std::fs::read(&a).unwrap()[0..6],
            std::fs::read(&b).unwrap()[0..6]
        );
        assert_eq!(
            std::fs::read(&a).unwrap()[TAIL_WINDOW as usize..],
            std::fs::read(&b).unwrap()[TAIL_WINDOW as usize..]
        );
        let fa = tail_fingerprint(&a, up).unwrap();
        let fb = tail_fingerprint(&b, up).unwrap();
        let fc = tail_fingerprint(&c, up).unwrap();
        assert_eq!(fa.len(), 64, "sha256 十六进制形态（32 字节 → 64 字符）");
        assert!(
            fa.chars()
                .all(|ch| ch.is_ascii_hexdigit() && !ch.is_ascii_uppercase()),
            "必须是小写十六进制小写形态：{fa}"
        );
        assert_eq!(
            fa, fb,
            "窗口外（前 6 字节）不同不得影响指纹 —— 证明只哈希末尾 ≤64 字节"
        );
        assert_ne!(fa, fc, "窗口内差 1 字节必须改变指纹（灵敏度对照）");
        assert_eq!(
            tail_fingerprint(&a, 0).unwrap(),
            "",
            "水位 0 → 空指纹（无已消费前缀）"
        );
        assert_ne!(
            tail_fingerprint(&a, TAIL_WINDOW).unwrap(),
            fa,
            "上界不同 → 窗口不同"
        );
        std::fs::remove_dir_all(a.parent().unwrap()).ok();
    }

    /// 指纹上界 = `up_to`（**已消费前缀**），不是文件尾：文件继续追加不得改变旧前缀的指纹。
    /// 这是增量设计的地基——每轮只校验「我读过的那一段」有没有被改写。
    #[test]
    fn tail_fingerprint_hashes_only_the_consumed_prefix() {
        let a = tmp("prefix-a", "AAAAAAXXXXXXXXXX");
        let b = tmp("prefix-b", "AAAAAAYYYYYYYYYY");
        assert_eq!(
            tail_fingerprint(&a, 6).unwrap(),
            tail_fingerprint(&b, 6).unwrap(),
            "同为前 6 字节 → 指纹必须相同（只看已消费前缀，不看其后的追加内容）"
        );
        assert_ne!(
            tail_fingerprint(&a, 16).unwrap(),
            tail_fingerprint(&b, 16).unwrap(),
            "对照：把水位推到 16 就必须能看出差异"
        );
        std::fs::remove_dir_all(a.parent().unwrap()).ok();
        std::fs::remove_dir_all(b.parent().unwrap()).ok();
    }

    /// 判定层是**纯函数**、IO 只在三个入口里（GC 6 / 本任务「口径层唯一且纯」）。
    /// ① 签名锁：编译器强制这两个判定不接 `&Path` —— 谁把 IO 混回来就编译失败；
    /// ② 接口锁：三个 IO 入口的签名与任务书 Interfaces 逐字一致；
    /// ③ 行为：纯函数在**不存在的路径**上照常给答案，而 IO 入口必须报错。
    #[test]
    fn rescan_predicates_are_pure_and_io_stays_in_the_entry_points() {
        let _: fn(u64, &CursorDelta) -> bool = shorter_than_watermark;
        let _: fn(&str, &CursorDelta) -> bool = fingerprint_mismatch;
        let _: fn(i64, u64, &CursorDelta) -> bool = file_unchanged;
        let _: fn(&Path) -> std::io::Result<(i64, u64)> = stat_of;
        let _: fn(&Path, u64) -> std::io::Result<String> = tail_fingerprint;
        let _: fn(&Path, &CursorDelta) -> std::io::Result<bool> = needs_rescan;

        let ghost = std::env::temp_dir().join("mam_usage_cursor_ghost_never_exists.jsonl");
        assert!(!ghost.exists(), "前提：探针路径确实不存在");
        let c = cursor_of(10, "abc");
        assert!(shorter_than_watermark(9, &c), "短于水位 → 必须重扫");
        assert!(!shorter_than_watermark(10, &c), "恰好等于水位不算截断");
        assert!(!shorter_than_watermark(11, &c));
        assert!(fingerprint_mismatch("xyz", &c), "指纹不符 → 必须重扫");
        assert!(!fingerprint_mismatch("abc", &c));
        assert!(
            needs_rescan(&ghost, &c).is_err(),
            "needs_rescan 是 IO 入口：文件不在必须报错（不静默判「无需重扫」）"
        );
        // 库里的脏数据（负数水位）不得 panic，也不得恒判重扫
        assert!(!shorter_than_watermark(0, &cursor_of(-5, "")));
        assert!(shorter_than_watermark(0, &cursor_of(1, "")));
    }
}
// ==== usage 自省锁：排除区结束，勿删勿复制 ====
