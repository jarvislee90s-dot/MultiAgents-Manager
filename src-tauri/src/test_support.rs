//! 测试专用临时目录工厂（仅 `#[cfg(test)]` 编译，不进发布产物）。
//!
//! **由来（存量技术债清理）**：本仓多处测试用
//! `std::env::temp_dir().join(format!("<前缀>-{}-{}", process::id(), SystemTime::now()
//! .duration_since(UNIX_EPOCH).unwrap().as_nanos()))` 拼临时目录名，把「纳秒」当
//! 唯一性来源。但 `CLOCK_REALTIME` 在 macOS 上粒度是 **1µs**，`as_nanos()` 的末三位
//! 实际恒为 0：本机实测 20 万次相邻采样中 **176399 次（88.2%）取到与上一次完全相同的
//! 纳秒值**（机器越忙比例越高，另一时刻测得 189879/200000 ≈ 94.9%）。
//!
//! 后果不是「名字难看」，而是**并行跑的测试互相踩**：同一微秒内的两次调用拼出同一条
//! 路径 → 后创建者与前创建者共用目录，谁先收尾就 `remove_dir_all` 掉对方的数据。
//! 实测（修前，本机）：`commands::data_management::tests` 族 10 次跑**红 3 次**、
//! `remote::attachments::tests` 族 10 次跑**红 5 次**，症状是随机某条用例
//! `unknown_project` / 索引条数对不上 / `create_dir_all` 失败。
//!
//! **修法**：命名与时钟**彻底解耦**——唯一性来自「进程号 + 进程内原子序号」，
//! 不再读 `SystemTime`；再叠 `tempfile` 的随机尾 + `O_EXCL` 建目录语义做第二重保险
//! （覆盖「上次运行残留同名目录」与「pid 复用」）；返回 `TempDir` 守卫，测试 panic
//! 时目录也随 Drop 回收（旧实现漏清理，残留会跨运行累积）。

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

/// 进程内单调序号（跨线程安全）：保证「同一微秒内的连续调用」序号必不同。
static DIR_SEQ: AtomicU64 = AtomicU64::new(0);

/// 目录名的**确定性**部分：`<前缀>-<pid>-<序号>`。
///
/// 拆成纯函数是为了让「同一微秒内两次命名必不同」这条不变量能被**确定性**地测住
/// （见本模块 `same_microsecond_names_differ_because_of_seq_not_clock`），
/// 而不是靠「跑得够快」去撞时钟粒度。
pub(crate) fn unique_dir_name(prefix: &str, pid: u32, seq: u64) -> String {
    format!("{prefix}-{pid}-{seq}")
}

/// 测试临时目录守卫：`tempfile::TempDir` 的薄封装。
///
/// **为什么包一层**：`tempfile 3.27` 的 `TempDir` 只实现 `AsRef<Path>`、
/// **没有** `Deref<Target = Path>`（旧版有，升级时被移除）——直接用会让每个调用点
/// 都得写一次 `.path()`。这里把 `Deref` 补回来：`&dir` 可直接当 `&Path` 传参、
/// `dir.join(..)` / `dir.to_string_lossy()` 可直接调 `Path` 的方法，调用点保持原样。
///
/// 刻意**不实现 `Clone`**：复制守卫会变成双重回收，需要自有 `PathBuf`
/// （跨线程传参等）时用 `dir.path().to_path_buf()`。
pub(crate) struct TestDir(tempfile::TempDir);

impl TestDir {
    pub(crate) fn path(&self) -> &Path {
        self.0.path()
    }
}

impl std::ops::Deref for TestDir {
    type Target = Path;
    fn deref(&self) -> &Path {
        self.0.path()
    }
}

impl AsRef<Path> for TestDir {
    fn as_ref(&self) -> &Path {
        self.0.path()
    }
}

/// 创建一个**进程内唯一**的测试临时目录；守卫 Drop 时自动回收。
///
/// 用法：`let root = crate::test_support::temp_dir("mam-attach-test");`
pub(crate) fn temp_dir(prefix: &str) -> TestDir {
    let name = unique_dir_name(
        prefix,
        std::process::id(),
        DIR_SEQ.fetch_add(1, Ordering::Relaxed),
    );
    let dir = tempfile::Builder::new()
        .prefix(&name)
        .tempdir()
        .expect("创建测试临时目录失败");
    TestDir(dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// **确定性回归锁**（正对旧实现根因）：命名不再读时钟——同一 pid、
    /// 「同一微秒」下，序号不同则名字必不同。旧实现把 `as_nanos()` 当唯一性来源，
    /// 此刻两次命名会得到**完全相同**的字符串。
    #[test]
    fn same_microsecond_names_differ_because_of_seq_not_clock() {
        assert_eq!(unique_dir_name("mam-x", 42, 0), "mam-x-42-0");
        // 同一微秒（pid 相同、时刻相同）内两次命名 → 序号兜底，必不同
        assert_ne!(
            unique_dir_name("mam-x", 42, 0),
            unique_dir_name("mam-x", 42, 1)
        );
        // 跨进程（pid 复用场景）由 tempfile 随机尾兜底，确定性部分也仍不同
        assert_ne!(
            unique_dir_name("mam-x", 42, 7),
            unique_dir_name("mam-x", 43, 7)
        );
    }

    /// **行为回归锁**：连续快速创建 256 个目录（本机实测相邻两次 `as_nanos()` 有
    /// 88% 相同 ⇒ 绝大多数落在同一微秒内）→ 路径必须两两不同且都是已建好的真目录。
    /// 谁把工厂改回「pid + 纳秒」命名，本用例在 1µs 粒度机器上必红。
    #[test]
    fn rapid_creations_are_pairwise_distinct() {
        const N: usize = 256;
        let dirs: Vec<_> = (0..N).map(|_| temp_dir("mam-unique-test")).collect();
        let paths: HashSet<_> = dirs.iter().map(|d| d.path().to_path_buf()).collect();
        assert_eq!(paths.len(), N, "同一微秒内创建的临时目录不得重名");
        assert!(
            paths.iter().all(|p| p.is_dir()),
            "每个返回路径都必须是已建好的目录"
        );
        // 名字里带前缀 + pid + 序号（可读、可定位残留）
        let head = format!("mam-unique-test-{}-", std::process::id());
        assert!(
            paths
                .iter()
                .all(|p| p.file_name().unwrap().to_string_lossy().starts_with(&head)),
            "目录名应带前缀+pid+序号: {head:?}"
        );
    }

    /// 守卫语义：落地即真目录，Drop 即回收（含测试 panic 的早退路径）。
    #[test]
    fn guard_creates_and_removes() {
        let path;
        {
            let d = temp_dir("mam-guard-test");
            path = d.path().to_path_buf();
            assert!(path.is_dir(), "守卫构造后目录应已存在");
        }
        assert!(!path.exists(), "守卫 Drop 后目录应被回收");
    }
}
