//! 通用去重（说明书 §4.3 / §5.3.4 / §7.2）：
//! * **跨源通用原则**：去重键必须含「源 + 会话/文件」，不能只用消息 id
//!   （claude `message.id` 跨会话复用，本机 279 个重复 id）；
//! * **codex 相邻重复**：真实重放（1,945 条 / 114 文件），相邻四桶完全相同则丢弃，
//!   **禁止用 turn_id 去重四桶**（turn_context 非每轮写入、turn_id 缺失 1.5%）；
//! * **turn 级去重**：合并统计前按 (源, 会话 id, turn id) 去重（codex 同 turn 多条
//!   task_complete：2,885 → 2,308）——作用对象是**时长样本**，与上一条是两件事。
use std::collections::HashSet;

use super::model::{UsageBuckets, UsageSourceId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DedupKind {
    Request,
    Turn,
    ToolCall,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DedupKey {
    pub source_id: String,
    pub session_id: String,
    pub kind: DedupKind,
    pub local_id: String,
}

#[derive(Debug, Default)]
pub struct DedupSet {
    seen: HashSet<DedupKey>,
}

impl DedupSet {
    pub fn new() -> Self {
        Self {
            seen: HashSet::new(),
        }
    }
    /// true = 首次出现（应计入）；false = 重复（丢弃）
    pub fn admit(&mut self, key: DedupKey) -> bool {
        self.seen.insert(key)
    }
    pub fn len(&self) -> usize {
        self.seen.len()
    }
    pub fn is_empty(&self) -> bool {
        self.seen.is_empty()
    }
}

/// codex 相邻重复事件丢弃器（**只比较相邻**；非相邻重复必须保留）
#[derive(Debug, Default)]
pub struct AdjacentDrop {
    last: Option<UsageBuckets>,
}

impl AdjacentDrop {
    pub fn new() -> Self {
        Self { last: None }
    }
    /// true = 应计入（与上一条不同）；false = 相邻重放，丢弃
    pub fn admit(&mut self, b: UsageBuckets) -> bool {
        if self.last == Some(b) {
            return false;
        }
        self.last = Some(b);
        true
    }

    /// **跨轮续读（W-12）**：用上一轮最后一条四桶预置本比较器。否则每轮的第一条都会与
    /// 「无上一条」比较（恒为 `true`）——**跨轮边界的那一对相邻重放会漏判**（静默多算）。
    /// 本类型仍是**单文件 / 单轮**比较器：每个文件、每一轮各自新建（W-11）。
    pub fn seed(&mut self, last: Option<UsageBuckets>) {
        self.last = last;
    }

    /// 取本轮的「最后一条四桶」，供采集器写进游标 `state_json` 并**在下一轮 `seed` 回来**。
    pub fn last(&self) -> Option<UsageBuckets> {
        self.last
    }
}

pub fn claude_request_key(session_id: &str, message_id: &str) -> DedupKey {
    DedupKey {
        // fix round 1/5（Minor #4）：源 id 不再写字面量，复用 model.rs 的 `db_id()`
        // （`model.rs` 明文「不新造 id 字符串」，且源 id 变更时不会静默漂移）
        source_id: UsageSourceId::Claude.db_id().into(),
        session_id: session_id.into(),
        kind: DedupKind::Request,
        local_id: message_id.into(),
    }
}

pub fn turn_key(source_id: &str, session_id: &str, turn_id: &str) -> DedupKey {
    DedupKey {
        source_id: source_id.into(),
        session_id: session_id.into(),
        kind: DedupKind::Turn,
        local_id: turn_id.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 说明书 §4.3：**去重键必须含「源 + 会话/文件」**——claude `message.id` 会跨会话复用
    /// （本机 279 个重复 id），仅按 id 去重会误删别的会话的真实用量。
    #[test]
    fn dedup_key_includes_session_so_cross_session_ids_survive() {
        let mut set = DedupSet::new();
        assert!(set.admit(claude_request_key("s1", "msg_A")));
        assert!(
            !set.admit(claude_request_key("s1", "msg_A")),
            "同会话同 id 是流式重复，只计一次"
        );
        assert!(
            set.admit(claude_request_key("s2", "msg_A")),
            "跨会话复用同 id 必须各自计入"
        );
        // 流式重复量级（本机 851 条 assistant 行只对应 317 个 message.id，2.68×）
        let mut set2 = DedupSet::new();
        let admitted = (0..851)
            .filter(|i| set2.admit(claude_request_key("s1", &format!("m{}", i % 317))))
            .count();
        assert_eq!(admitted, 317);
        assert!(
            (851.0 / admitted as f64 - 2.68).abs() < 0.02,
            "实测流式重复 2.68×"
        );
    }

    /// codex 相邻重复事件（真实重放：1,945 条 / 114 文件）——**相邻四桶完全相同则丢弃**。
    /// **禁止用 turn_id 去重**（§5.3.4：`turn_context` 非每轮写入、turn_id 缺失 1.5%）。
    #[test]
    fn codex_adjacent_duplicates_are_dropped_without_turn_id() {
        let mut drop = AdjacentDrop::new();
        let b = UsageBuckets {
            input_fresh: 200,
            cache_read: 800,
            cache_write: 0,
            output: 200,
        };
        let c = UsageBuckets {
            input_fresh: 300,
            cache_read: 900,
            cache_write: 0,
            output: 100,
        };
        assert!(drop.admit(b));
        assert!(!drop.admit(b), "相邻完全相同 → 重放，丢弃");
        assert!(!drop.admit(b), "连续重放多份也只算一次");
        assert!(drop.admit(c), "值变了就是新记录");
        // 非相邻重复（中间夹一条别的）必须保留
        let mut drop3 = AdjacentDrop::new();
        assert!(drop3.admit(b));
        assert!(drop3.admit(c));
        assert!(drop3.admit(b), "非相邻重复不得丢弃（只丢相邻）");
        // fix round 1/5（Minor #2）：原 `drop2` 那对断言已删。它在**全新** `AdjacentDrop`（`last == None`）
        // 上连做两次 admit，恒为 true，任何「不总是返回 false」的实现都能过 → 退化断言、零信息。
        // 原注释宣称的「同一 turn_id 的两条不同四桶都必须计入」在本类型上**不可表达**（`admit` 只收四桶、
        // 不收也不存 turn_id）；该情形真正的行为锁是上面 `drop3` 的三行（不同值不被丢 + 非相邻重复保留）。
        // 补强 1（Task 6 的 D-1）：**只比 total / 只比 input_fresh+output** 的退化解必须被抓住——
        // b 与 e 的 total（1200）与 input_fresh/output 全同，仅缓存读写的拆分不同 → 不是重放。
        let mut drop4 = AdjacentDrop::new();
        let e = UsageBuckets {
            input_fresh: 200,
            cache_read: 700,
            cache_write: 100,
            output: 200,
        };
        assert!(drop4.admit(b));
        assert!(
            drop4.admit(e),
            "四桶任一不同即非重放（只比 total / 只比部分桶会错杀）"
        );
        // 补强 2（fix round 1/5 的 Minor #3）：**丢掉单个 cache_read 的比较器**也必须被抓住——
        // f 与 b **仅 cache_read 不同**（input_fresh / cache_write / output 逐字相同）→ 不是重放。
        let mut drop5 = AdjacentDrop::new();
        let f = UsageBuckets {
            input_fresh: 200,
            cache_read: 700,
            cache_write: 0,
            output: 200,
        };
        assert!(drop5.admit(b));
        assert!(
            drop5.admit(f),
            "仅 cache_read 不同也不得算重放（比三字段子集会错杀）"
        );
    }

    /// 合并统计前按 (源, 会话 id, turn id) 去重——codex 同 turn 多条 task_complete
    /// （本机 2,885 → 2,308），这是**时长样本**的去重，与四桶的相邻去重是两件事。
    #[test]
    fn turn_dedup_is_scoped_by_source_and_session() {
        let mut set = DedupSet::new();
        assert!(set.admit(turn_key("codex", "s1", "t1")));
        assert!(!set.admit(turn_key("codex", "s1", "t1")));
        // fix round 1/5（裁决 A）：GC 9 首句是「**源** + 会话/文件」——异源同会话同 turn id
        // 必须各自计入（源不进键 → 跨源误删）
        assert!(
            set.admit(turn_key("claude", "s1", "t1")),
            "同会话同 turn id、异源不得误删"
        );
        assert!(
            set.admit(turn_key("codex", "s2", "t1")),
            "同 id 跨会话不得误删"
        );
        // 2,885 → 2,308 的规模断言（重复率 = 20.0%）
        let mut set2 = DedupSet::new();
        let admitted = (0..2_885)
            .filter(|i| set2.admit(turn_key("codex", "s1", &format!("t{}", i % 2_308))))
            .count();
        assert_eq!(admitted, 2_308, "去重后应恰好是 2,308 个 turn");
    }
}
