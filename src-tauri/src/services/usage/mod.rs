// 用量采集与存储底座（计划①）：契约 §2 值对象 + 口径层 + 7 采集器 + 查询 + 账本落库。
// 纪律：本域的一切文件读取只经 stream.rs::read_incremental；一切口径计算只在
// semantics / dedup / project / provider / range 五个纯函数模块里（采集器只上报原始字段）。
// 模块声明随任务增量补齐（每个任务在 Files 里写明自己要 append 的那一行），
// 避免出现「声明了但文件还没建」的中间态编译失败。
pub mod caps;
pub mod collect;
pub mod collectors;
pub mod cursor;
pub mod dedup;
pub mod delta;
pub mod error;
pub mod ledger;
pub mod model;
pub mod project;
pub mod provider;
pub mod query;
pub mod range;
pub mod semantics;
pub mod settings;
pub mod stream;

pub use error::UsageError;
pub use model::*;

/// 供应商不可得时的行标签 = **i18n 键**（zh/en 同父同键，前端 `t(label)` 渲染）。
/// **不得**在 Rust 侧硬编码任何中文文案（W5）；CSV 导出不落 i18n 键（该列留空，见 Task 19）。
pub const UNKNOWN_PROVIDER_LABEL_KEY: &str = "usage.label.unknownProvider";

/// 当前毫秒（全域唯一时钟入口，便于测试注入）
pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// **源码级自省锁的共享判据面**（第 5 轮：用**显式哨兵标记**取代全部「结构推断」）。
///
/// 五轮教训一句话：判据先前一直用**文本启发式去推断 Rust 结构**（首个 `#[cfg(test)]` 字面量 →
/// 前缀匹配的 `mod tests` → 大括号「配平到 EOF」），每一轮都被**更早的假锚点 / 字符串字面量 /
/// 块注释**骗过；而且**盲区永远是 `[锚点, EOF)`**——「收口校验」管不到锚点与收口点之间装了什么
/// （生产代码可以写在测试模块**之后**，Rust 不看声明顺序）。
/// 第 5 轮把「结构推断」**整个删掉**：面由**一对显式标记**划出，判据只做「计数 + 顺序 + 非空」
/// 这类**无条件断言**，不再猜任何 Rust 结构。**两份手抄判据也随之下岗**（改为一处共享实现，
/// 见下）——从此不存在「只改一处不会变红」的问题：只有一处可改。
#[cfg(test)]
pub(crate) mod self_lock {
    /// 排除区**开始**标记（每个文件**恰好 1 行**）。用 `concat!` 拆开写：判据自己的源码也在
    /// 扫描面内，写成整行字面量会让「恰好 1 次」变成 2 次 ⇒ 响亮失败。
    pub(crate) const EXCLUDE_BEGIN: &str = concat!(
        "// ==== usage 自省锁：",
        "以下为排除区（测试代码），勿删勿复制 ===="
    );

    /// 排除区**结束**标记（每个文件**恰好 1 行**）。位置：有测试模块的文件 = 其测试模块的收口
    /// `}` 之后（本域 19 个文件里它就是文件最后一行）；**无测试模块的文件**（5 个）= **紧邻
    /// BEGIN 之后**（排除区为空 ⇒ 整文件都在面内）。
    pub(crate) const EXCLUDE_END: &str =
        concat!("// ==== usage 自省锁：", "排除区结束，勿删勿复制 ====");

    /// 取一个文件的**判据面**：`[文件头, BEGIN)` ∪ `(END, 文件尾]`，去行内空白、剥**整行** `//` 注释。
    ///
    /// `(END, 文件尾]` **必须在面内**：那是第 4 轮两条致盲构造（`R8_COLLECT` / `R8_STREAM`，
    /// 回归追加在文件末尾、落在旧「收口点」之外）的修法。
    ///
    /// **断言（任何一条不满足都 panic = 响亮失败，绝不静默退化）**：
    /// ① 两个标记各**恰好 1 行**（0 行 = 标记被删；≥2 行 = 被复制或被他处整行引用 ⇒ 面不可信）；
    /// ② BEGIN 在 END **之前**；③ 判据面**非空**。
    ///
    /// **威胁模型（如实声明，别再宣称「结构上不可能」）**：本锁防的是**顺手回归**与**无意的
    /// 习惯性改回去**，**不防蓄意改写本文件内的锁面标记**——对抗性攻击者本来就可以直接不写回归。
    /// 具体地：删标记 / 复制标记 / 调换顺序 ⇒ **响亮失败**；把 BEGIN **下移**或把 END **上移**
    /// （= 排除区缩小）⇒ **多扫**（安全方向，最坏是假阳性）；**唯一**能重新造出盲区的形态是
    /// **把 BEGIN 上移 / 把 END 下移（= 排除区扩大、盖住生产代码）**，而那需要把标记**搬到**生产
    /// 代码两侧——属上面那句「蓄意改写锁面标记」，**不在威胁模型内**，且 diff 里一眼可见。
    pub(crate) fn scan_face(name: &str, src: &str) -> Vec<(usize, String)> {
        let raw: Vec<&str> = src.lines().collect();
        let begins = raw.iter().filter(|l| l.trim() == EXCLUDE_BEGIN).count();
        let ends = raw.iter().filter(|l| l.trim() == EXCLUDE_END).count();
        assert!(
            begins == 1 && ends == 1,
            "`{name}` 的排除区标记必须**各恰好 1 行**，实际 BEGIN={begins} / END={ends}：\
             0 行 = 标记被删（面不可信）、≥2 行 = 标记被复制或被他处整行引用 ⇒ **一律响亮失败，\
             不许静默退化**。修法：两行标记按原样放回——BEGIN 在测试模块前一行，END 在其收口 `}}`\
             之后（无测试模块的文件：两行**相邻**放在文件末尾）"
        );
        let b = raw
            .iter()
            .position(|l| l.trim() == EXCLUDE_BEGIN)
            .expect("BEGIN 计数已断言为 1");
        let e = raw
            .iter()
            .position(|l| l.trim() == EXCLUDE_END)
            .expect("END 计数已断言为 1");
        assert!(
            b < e,
            "`{name}` 的排除区标记顺序反了（BEGIN 在源码第 {} 行、END 在第 {} 行）：\
             顺序错了「面」就没有意义，**直接红**，不许猜",
            b + 1,
            e + 1
        );
        let face: Vec<(usize, String)> = raw
            .iter()
            .enumerate()
            .filter(|(i, _)| *i < b || *i > e)
            .filter(|(_, l)| !l.trim_start().starts_with("//"))
            .map(|(i, l)| {
                (
                    i + 1,
                    l.chars().filter(|c| !c.is_whitespace()).collect::<String>(),
                )
            })
            .collect();
        assert!(
            !face.is_empty(),
            "`{name}` 的判据面为空（BEGIN 在第 {} 行、END 在第 {} 行）：面被清空 = 该文件的\
             反向锁彻底失效，**直接红**（这正是「哨兵写错位置把整个文件变成盲区」那条）",
            b + 1,
            e + 1
        );
        face
    }
}
// ==== usage 自省锁：以下为排除区（测试代码），勿删勿复制 ====
// ==== usage 自省锁：排除区结束，勿删勿复制 ====
