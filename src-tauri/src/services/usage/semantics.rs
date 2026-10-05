//! 口径层（计划①）：缓存语义 / 四桶归一 / 命中率派生 … 的纯函数与枚举。
//! 本步（Task 3）只落 `CacheSemantics` 枚举与 `as_db/from_db` 两个映射（DAO 的编译前置），
//! 其余口径函数由 Task 5 在此文件补全。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheSemantics {
    Exclusive,
    Subset,
    TotalOnly,
}

impl CacheSemantics {
    pub fn as_db(&self) -> &'static str {
        match self {
            CacheSemantics::Exclusive => "exclusive",
            CacheSemantics::Subset => "subset",
            CacheSemantics::TotalOnly => "total-only",
        }
    }
    pub fn from_db(s: &str) -> Self {
        match s {
            "subset" => CacheSemantics::Subset,
            "total-only" => CacheSemantics::TotalOnly,
            _ => CacheSemantics::Exclusive,
        }
    }
}
