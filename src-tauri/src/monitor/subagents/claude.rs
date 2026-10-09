//! Claude 子 agent 运行判据（spec §4.1 事件序状态机；判据权威/架构基准）。
//!
//! 落盘布局（spec §2.1 已修正版——评审 P0-1：父转写在**项目层平铺**，不在会话目录内）：
//!   ~/.claude/projects/<项目>/
//!   ├── <会话uuid>.jsonl                          父转写（通知 / SendMessage 事件源）
//!   └── <会话uuid>/subagents/agent-<agentId>.meta.json   登记 spawn（存在即 running 起点）
//!       <会话uuid>/subagents/agent-<agentId>.jsonl       子转写（timestamp + usage 四桶）
//!
//! 排雷（spec §2.1）：①通知 ≠ 终态（stop 后 SendMessage 续跑，同 id 多次通知）；
//! ②启动即有 tool_result（`Async agent launched…`），不能以「无 tool_result」判运行中。
//! **不做 mtime 兜底**（v1 裁决：长静默工具调用会误伤；残留随会话中断整区消失）。

use super::{
    read_increment, sort_views, update_entry, CacheBox, IncrRead, IncrState, SubagentView,
    TokenUsage,
};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// 会话目录发现（发现层，非判据；spec §2.1 已修正版——评审 P0-1）：父转写在
/// **项目层平铺**（`projects/<项目>/<sid>.jsonl`，与 remote/content.rs:452-469
/// find_jsonl_in_projects 同一发现模式），登记源 subagents/ 在会话目录内
/// （`projects/<项目>/<sid>/subagents/`）。两处齐备才算命中：无 subagents/ 的
/// 目录（旧平铺布局 / 无子 agent 会话）天然空态。父 jsonl 文件缺失不阻断发现
/// （read_increment 对缺失文件返回空行 → 无事件 = spawn 态，残留口径 §8.2 申报）。
fn locate(projects_root: &Path, session_id: &str) -> Option<(PathBuf, PathBuf)> {
    // 信任边界（review 修复，与 kimi.rs collect_with 的 sessions 根校验对称）：
    // session_id 是 UUID，路径分隔符或 ".." 只可能来自恶意构造的查询——入口拒绝，
    // 防 ../ 逃逸把发现层引出 projects 根。（join 后 starts_with 对含 .. 分量的
    // 路径只做词法前缀比较、不解析，拦不住该形态，故取「入口拒绝」而非出口校验）
    if session_id == ".." || session_id.contains('/') || session_id.contains('\\') {
        return None;
    }
    let Ok(entries) = std::fs::read_dir(projects_root) else {
        return None;
    };
    for e in entries.flatten() {
        let proj = e.path();
        if !proj.is_dir() {
            continue;
        }
        let subagents = proj.join(session_id).join("subagents");
        if subagents.is_dir() {
            return Some((proj.join(format!("{session_id}.jsonl")), subagents));
        }
    }
    None
}

/// meta 登记（readdir subagents/*.meta.json）
struct RegAgent {
    id: String,
    meta: AgentMeta,
}
fn registered_agents(subagents_dir: &Path) -> Vec<RegAgent> {
    let Ok(entries) = std::fs::read_dir(subagents_dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if let Some(id) = agent_id_from_meta_name(&name) {
            if let Ok(text) = std::fs::read_to_string(e.path()) {
                if let Some(meta) = parse_meta(&text) {
                    out.push(RegAgent {
                        id: id.to_string(),
                        meta,
                    });
                }
            }
        }
    }
    out.sort_by(|a, b| a.id.cmp(&b.id)); // 注册序稳定（spawnTs 同值时的次序确定性）
    out
}

/// 会话级缓存条目（spec §5.1）
#[derive(Default, Clone)]
struct ClaudeCache {
    parent: IncrState,
    last: HashMap<String, LastEvent>,
    agents: HashMap<String, AgentTrack>,
}
#[derive(Default, Clone)]
struct AgentTrack {
    incr: IncrState,
    tokens: TokenUsage,
    first_ts: Option<String>,
}

/// 倒序分块扫描（重建路径，spec §5.1）：从文件尾向头按 256KB 取块，行序「新→旧」。
/// visitor 返回 true = 登记集收齐即停（不再读更早的块）。
/// 块边界语义：JSONL 行以 '\n' **右终结**——[块 + carry] 切段后，最后一段（其后无
/// '\n'）是跨块半行，进 carry 与左邻块拼接；其余段皆完整可访问。
/// **返回值 = 文件尾未终结半行字节**（空 = 以 '\n' 收尾）——carry 恒等于已处理
/// 区域最后一个 '\n' 之后缀，而处理区域总达文件尾，故 carry 非空 ⟺ 文件尾有未终结
/// 行（早停亦然）。调用方据此把增量游标对齐到 size − carry.len()（T1 不变式），该半行的
/// 事件留给增量路径补齐后消费——评审 P1-1：skip_to(size) 会把它永久丢失。
fn scan_parent_rev(path: &Path, mut visit: impl FnMut(&str) -> bool) -> std::io::Result<Vec<u8>> {
    use std::io::{Read, Seek, SeekFrom};
    const BLOCK: u64 = 256 * 1024;
    let mut f = std::fs::File::open(path)?;
    let size = f.metadata()?.len();
    let mut pos = size;
    let mut carry: Vec<u8> = Vec::new(); // 右邻块的跨块半行（文件序在块后）
    while pos > 0 {
        let start = pos.saturating_sub(BLOCK);
        f.seek(SeekFrom::Start(start))?;
        let take = (pos - start) as usize;
        let mut buf = vec![0u8; take];
        f.read_exact(&mut buf)?;
        buf.extend_from_slice(&carry);
        carry.clear();
        let mut complete: Vec<(usize, usize)> = Vec::new(); // [a,b) 完整行字节区间
        let mut seg = 0usize;
        for (i, &b) in buf.iter().enumerate() {
            if b == b'\n' {
                complete.push((seg, i));
                seg = i + 1;
            }
        }
        carry = buf[seg..].to_vec();
        for (a, b) in complete.iter().rev() {
            if visit(&String::from_utf8_lossy(&buf[*a..*b])) {
                return Ok(carry); // 登记集收齐即停（carry 已是本块尾半行）
            }
        }
        pos = start;
    }
    Ok(carry)
}

/// 生产入口（T7 经 RemoteState 缝调用）
pub fn collect(session_id: &str) -> Vec<SubagentView> {
    let Some(root) = dirs::home_dir().map(|h| h.join(".claude").join("projects")) else {
        return Vec::new();
    };
    collect_with(&root, session_id)
}

/// 可注入核心（projects 根注入；测试 tempdir）
pub(crate) fn collect_with(projects_root: &Path, session_id: &str) -> Vec<SubagentView> {
    let Some((parent_jsonl, subagents_dir)) = locate(projects_root, session_id) else {
        return Vec::new();
    };
    let reg = registered_agents(&subagents_dir);
    // 闭包 move parent_jsonl + subagents_dir + reg（发现结果一次取齐，step 不再反推路径）
    update_entry("claude", session_id, move |cur| {
        let fresh = cur.is_none(); // 缓存缺失/被清（MAM 重启 / 硬上限清空）→ 走重建
        let mut st = cur
            .and_then(|b| b.downcast::<ClaudeCache>().ok())
            .map(|a| Arc::try_unwrap(a).unwrap_or_else(|a| (*a).clone()))
            .unwrap_or_default();
        if fresh {
            rebuild(&parent_jsonl, &reg, &mut st); // 倒序扫父文件收齐事件 + 游标对齐尾半行
                                                   // agent 转写无游标 → 全量读累计（rebuild 后 step 的 IncrState 从 0 起，
                                                   // 首轮 read_increment 自然全量消费——无需专门分支）
        }
        let out = step(&parent_jsonl, &subagents_dir, &reg, &mut st);
        (Arc::new(st) as CacheBox, out)
    })
}

fn step(
    parent_jsonl: &Path,
    subagents_dir: &Path,
    reg: &[RegAgent],
    st: &mut ClaudeCache,
) -> Vec<SubagentView> {
    // ① 父文件事件（增量；reset → 清空状态机后按全量行重放）
    match read_increment(parent_jsonl, &mut st.parent) {
        IncrRead::Unchanged => {}
        IncrRead::Lines { lines, reset } => {
            if reset {
                st.last.clear();
            }
            for line in &lines {
                if let Some(ev) = classify_parent_line(line) {
                    apply_event(&mut st.last, &ev);
                }
            }
        }
    }
    // ② 每 agent 转写增量累计（reset → 清空累计后按全量行重放）
    for a in reg {
        let track = st.agents.entry(a.id.clone()).or_default();
        // 评审 P0-1：subagents 目录来自 locate() 的发现结果，不从父路径反推
        // （父在项目层平铺，parent() 反推会指到错误目录）
        let path = subagents_dir.join(format!("agent-{}.jsonl", a.id));
        match read_increment(&path, &mut track.incr) {
            IncrRead::Unchanged => {}
            IncrRead::Lines { lines, reset } => {
                if reset {
                    track.tokens = TokenUsage::default();
                    track.first_ts = None;
                }
                for line in &lines {
                    accumulate_agent_line(line, &mut track.tokens, &mut track.first_ts);
                }
            }
        }
    }
    // ③ 视图：已登记 ∧ 运行中（spawnTs 升序，None 排尾）
    let mut views: Vec<SubagentView> = reg
        .iter()
        .filter(|a| is_running(true, st.last.get(&a.id).copied()))
        .map(|a| {
            let t = st.agents.get(&a.id);
            SubagentView {
                id: a.id.clone(),
                name: a
                    .meta
                    .agent_type
                    .clone()
                    .unwrap_or_else(|| format!("agent-{}", a.id)),
                description: a.meta.description.clone(),
                spawn_ts: t.and_then(|t| t.first_ts.clone()),
                tokens: t.map(|t| t.tokens).unwrap_or_default(),
            }
        })
        .collect();
    sort_views(&mut views);
    views
}

/// 首次进缓存（重建路径）：倒序分块扫父文件收齐登记集事件 + agent 转写全量读
fn rebuild(parent_jsonl: &Path, reg: &[RegAgent], st: &mut ClaudeCache) {
    let wanted: HashSet<&str> = reg.iter().map(|a| a.id.as_str()).collect();
    let mut got: HashSet<String> = HashSet::new();
    let tail_partial = scan_parent_rev(parent_jsonl, |line| {
        if let Some(ev) = classify_parent_line(line) {
            let id = match &ev {
                ParentEvent::Stop { agent_id } | ParentEvent::Resume { agent_id } => {
                    agent_id.clone()
                }
            };
            apply_event_rev(&mut st.last, &ev);
            if wanted.contains(id.as_str()) {
                got.insert(id);
            }
        }
        wanted.len() == got.len() // 登记集收齐即停；扫到头仍有未命中 = spawn 态（is_running 对 None 放行）
    })
    .unwrap_or_default(); // 读失败 → 无尾半行可续（下次 collect 由 last_size 兜底语义接管）
                          // 增量游标对齐 T1 不变式（评审 P1-1）：partial = 文件尾未终结半行——重建时刻
                          // 它的事件未被消费，skip_to(size) 会永久跳过；补齐换行后由增量路径消费
    if let Ok(m) = std::fs::metadata(parent_jsonl) {
        st.parent.resume_at(m.len(), tail_partial);
    }
}

/// 父 JSONL 行事件
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ParentEvent {
    /// 子 agent 停止通知（文本含 <task-id>该id</task-id>）
    Stop { agent_id: String },
    /// SendMessage 续跑（tool_use name=SendMessage, input.to=该id）
    Resume { agent_id: String },
}

/// 每子 agent 的最近事件（运行中 = 已登记 ∧ last ∈ {spawn, resume}）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LastEvent {
    /// 无任何事件（meta 已登记）——扫到头未命中即此态。**spawn 态由 None 表示**
    /// （重建收齐即停扫到头、无事件即 spawn），本变体保留状态空间文档语义、
    /// 不被构造（spec §4.1 三态 {spawn, stop, resume}）
    #[allow(dead_code)]
    Spawn,
    Stop,
    Resume,
}

/// 提取消息全部文本（content 字符串 / text 块数组两种形态）
fn message_text(v: &serde_json::Value) -> String {
    match &v["message"]["content"] {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(blocks) => blocks
            .iter()
            .filter(|b| b["type"].as_str() == Some("text"))
            .filter_map(|b| b["text"].as_str())
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    }
}

/// 文本中提取 <task-id>…</task-id>（无 regex 依赖，朴素查找）
fn task_id_in(text: &str) -> Option<&str> {
    const OPEN: &str = "<task-id>";
    const CLOSE: &str = "</task-id>";
    let start = text.find(OPEN)? + OPEN.len();
    let end = start + text[start..].find(CLOSE)?;
    Some(&text[start..end])
}

/// 行分类（纯函数）：坏 JSON / 无关行 → None
pub(crate) fn classify_parent_line(line: &str) -> Option<ParentEvent> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    // resume 优先：assistant 的 SendMessage tool_use（input.to）
    if v["message"]["role"].as_str() == Some("assistant") {
        if let Some(blocks) = v["message"]["content"].as_array() {
            for b in blocks {
                if b["type"].as_str() == Some("tool_use")
                    && b["name"].as_str() == Some("SendMessage")
                {
                    if let Some(to) = b["input"]["to"].as_str() {
                        return Some(ParentEvent::Resume {
                            agent_id: to.to_string(),
                        });
                    }
                }
            }
        }
    }
    // stop：任意消息文本含 task-id 通知块
    let text = message_text(&v);
    task_id_in(&text).map(|id| ParentEvent::Stop {
        agent_id: id.to_string(),
    })
}

/// 增量路径：文件序应用（后到覆盖——last 语义）
pub(crate) fn apply_event(last: &mut HashMap<String, LastEvent>, ev: &ParentEvent) {
    let (id, val) = match ev {
        ParentEvent::Stop { agent_id } => (agent_id, LastEvent::Stop),
        ParentEvent::Resume { agent_id } => (agent_id, LastEvent::Resume),
    };
    last.insert(id.clone(), val);
}

/// 重建路径：新→旧扫描，每 id 首个（=最新）事件胜出
pub(crate) fn apply_event_rev(seen: &mut HashMap<String, LastEvent>, ev: &ParentEvent) {
    let (id, val) = match ev {
        ParentEvent::Stop { agent_id } => (agent_id, LastEvent::Stop),
        ParentEvent::Resume { agent_id } => (agent_id, LastEvent::Resume),
    };
    seen.entry(id.clone()).or_insert(val);
}

/// 运行中判据（spec §4.1）：已登记 ∧ last ∈ {spawn, resume}。
/// registered=false（meta 消失/未登记）一律不在跑。
pub(crate) fn is_running(registered: bool, last: Option<LastEvent>) -> bool {
    registered
        && matches!(
            last,
            None | Some(LastEvent::Spawn) | Some(LastEvent::Resume)
        )
}

/// 子 agent jsonl 单行累计（纯函数）：usage 四桶累加 + 首条 timestamp 捕获
pub(crate) fn accumulate_agent_line(
    line: &str,
    acc: &mut TokenUsage,
    first_ts: &mut Option<String>,
) {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
        return;
    };
    if first_ts.is_none() {
        if let Some(ts) = v["timestamp"].as_str() {
            *first_ts = Some(ts.to_string());
        }
    }
    let u = &v["message"]["usage"];
    if !u.is_object() {
        return; // 无 usage 的消息（user/tool_result 等）不累计
    }
    acc.input += u["input_tokens"].as_u64().unwrap_or(0);
    acc.cache_read += u["cache_read_input_tokens"].as_u64().unwrap_or(0);
    acc.cache_creation += u["cache_creation_input_tokens"].as_u64().unwrap_or(0);
    acc.output += u["output_tokens"].as_u64().unwrap_or(0);
}

/// subagents/agent-<id>.meta.json 内容（字段缺失容忍）。
/// **不解析 spawnDepth、不区分深度**（口径明示，评审 P3-5）：spec §3 只收一层
/// 子 agent——claude 的 subagents/ 登记源天然只有一层（深层派生不落此目录），
/// 其余三工具无深度信息可过滤。
#[derive(Deserialize)]
pub(crate) struct AgentMeta {
    #[serde(rename = "agentType")]
    pub(crate) agent_type: Option<String>,
    pub(crate) description: Option<String>,
}

pub(crate) fn parse_meta(text: &str) -> Option<AgentMeta> {
    serde_json::from_str(text).ok()
}

/// `agent-<id>.meta.json` → `<id>`（其余文件名 → None）
pub(crate) fn agent_id_from_meta_name(name: &str) -> Option<&str> {
    name.strip_prefix("agent-")?
        .strip_suffix(".meta.json")
        .filter(|id| !id.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::path::PathBuf;

    /// 夹具（spec §2.1 已修正版布局，评审 P0-1）：父转写在项目层平铺
    /// （proj/<sid>.jsonl），subagents/ 在会话目录内（proj/<sid>/subagents/）；
    /// 持有 TempDir 随 drop 清理
    struct Fixture {
        /// 仅借 Drop 清理 tempdir（RAII），运行期不读
        #[allow(dead_code)]
        td: tempfile::TempDir,
        root: PathBuf,
    }
    fn claude_fixture(sid: &str) -> Fixture {
        let td = tempfile::tempdir().unwrap();
        let root = td.path().to_path_buf();
        let proj = root.join("proj--e--demo");
        std::fs::create_dir_all(proj.join(sid).join("subagents")).unwrap();
        // 父转写：项目层平铺（与 remote/content.rs find_jsonl_in_projects 的发现位置同层）
        std::fs::write(proj.join(format!("{sid}.jsonl")), "").unwrap();
        Fixture { td, root }
    }
    impl Fixture {
        /// 父转写路径（项目层平铺）
        fn parent(&self, sid: &str) -> PathBuf {
            self.root.join("proj--e--demo").join(format!("{sid}.jsonl"))
        }
        /// 会话目录（内含 subagents/）
        fn session(&self, sid: &str) -> PathBuf {
            self.root.join("proj--e--demo").join(sid)
        }
        fn add_agent(&self, sid: &str, id: &str, meta: &str, transcript: &str) {
            let d = self.session(sid).join("subagents");
            std::fs::write(d.join(format!("agent-{id}.meta.json")), meta).unwrap();
            std::fs::write(d.join(format!("agent-{id}.jsonl")), transcript).unwrap();
        }
        fn append(&self, p: &Path, s: &str) {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new().append(true).open(p).unwrap();
            f.write_all(s.as_bytes()).unwrap();
        }
    }

    /// 全链路：登记 → 出 chip（名/时长/token 四桶）；追加通知 → 消失；追加 SendMessage → 复现（P0 集成层）
    #[test]
    fn claude_source_lifecycle() {
        super::super::reset_cache_for_tests();
        let f = claude_fixture("sess-1");
        f.add_agent(
            "sess-1",
            "a86f",
            r#"{"agentType":"Plan","description":"设计新建会话两改动实现方案"}"#,
            &format!(
                "{}\n{}\n",
                agent_line("2026-10-08T07:36:35.508Z", 5124, 56448, 0, 9312),
                agent_line("2026-10-08T07:37:00Z", 100, 0, 0, 200)
            ),
        );
        let v = collect_with(&f.root, "sess-1");
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].id, "a86f");
        assert_eq!(v[0].name, "Plan");
        assert_eq!(
            v[0].description.as_deref(),
            Some("设计新建会话两改动实现方案")
        );
        assert_eq!(v[0].spawn_ts.as_deref(), Some("2026-10-08T07:36:35.508Z"));
        assert_eq!(
            v[0].tokens,
            TokenUsage {
                input: 5224,
                cache_read: 56448,
                cache_creation: 0,
                output: 9512
            }
        );
        // ② 追加停止通知 → 消失
        f.append(&f.parent("sess-1"), &(notify("a86f") + "\n"));
        assert!(collect_with(&f.root, "sess-1").is_empty());
        // ③ 追加 SendMessage → 复现（增量路径，P0）
        f.append(&f.parent("sess-1"), &(sendmsg("a86f") + "\n"));
        assert_eq!(collect_with(&f.root, "sess-1").len(), 1);
        // ④ token 增量：转写追加一行，四桶只加新增
        f.append(
            &f.session("sess-1")
                .join("subagents")
                .join("agent-a86f.jsonl"),
            &(agent_line("2026-10-08T07:38:00Z", 1, 2, 3, 4) + "\n"),
        );
        let v = collect_with(&f.root, "sess-1");
        assert_eq!(
            v[0].tokens,
            TokenUsage {
                input: 5225,
                cache_read: 56450,
                cache_creation: 3,
                output: 9516
            }
        );
        super::super::reset_cache_for_tests();
    }

    /// 重启重建（缓存缺失）：倒序分块扫父文件收齐事件；agent 无任何事件 → spawn 态运行中
    #[test]
    fn claude_source_rebuild_after_restart() {
        super::super::reset_cache_for_tests();
        let f = claude_fixture("sess-2");
        f.add_agent(
            "sess-2",
            "b1",
            r#"{"agentType":"Explore"}"#,
            &(agent_line("2026-10-08T07:00:00Z", 1, 0, 0, 1) + "\n"),
        );
        f.add_agent("sess-2", "b2", r#"{"agentType":"Plan"}"#, "");
        // b1 有停止通知、b2 无事件（→ spawn 态运行中）
        f.append(&f.parent("sess-2"), &(notify("b1") + "\n"));
        let v = collect_with(&f.root, "sess-2");
        assert_eq!(v.len(), 1, "b1 已停止不上板，b2 无事件 = spawn 态在板");
        assert_eq!(v[0].id, "b2");
        super::super::reset_cache_for_tests();
    }

    /// P1-1 回归锁：重建时刻父文件尾恰为半行通知 → 增量游标不得跳过它——
    /// 半行补齐换行后，增量路径必须消费到该 stop 事件（chip 消失）
    #[test]
    fn claude_source_rebuild_consumes_tail_partial_line() {
        super::super::reset_cache_for_tests();
        let f = claude_fixture("sess-6");
        f.add_agent("sess-6", "d1", r#"{"agentType":"Plan"}"#, "");
        // 完整无关行 + 半行通知（无结尾换行）
        std::fs::write(
            f.parent("sess-6"),
            format!("{}\n{}", r#"{"filler":1}"#, notify("d1")),
        )
        .unwrap();
        // 重建：半行不可解析 → d1 无事件 = spawn 态在板
        assert_eq!(collect_with(&f.root, "sess-6").len(), 1);
        // 补齐换行：增量路径消费该通知 → 下板
        f.append(&f.parent("sess-6"), "\n");
        assert!(
            collect_with(&f.root, "sess-6").is_empty(),
            "尾半行事件不得被 skip_to 丢弃"
        );
        super::super::reset_cache_for_tests();
    }

    /// 护栏：父文件重写变短（size<offset）→ 作废重建，不 panic、不丢登记
    #[test]
    fn claude_source_parent_truncated_rebuilds() {
        super::super::reset_cache_for_tests();
        let f = claude_fixture("sess-3");
        f.add_agent("sess-3", "c1", r#"{"agentType":"Plan"}"#, "");
        f.append(&f.parent("sess-3"), &(notify("c1") + "\n"));
        assert!(collect_with(&f.root, "sess-3").is_empty());
        std::fs::write(f.parent("sess-3"), "x\n").unwrap(); // 重写变短
        let v = collect_with(&f.root, "sess-3");
        assert_eq!(v.len(), 1, "护栏作废后重建：c1 无事件 → spawn 态在板");
        super::super::reset_cache_for_tests();
    }

    /// 会话目录不存在 / 无 subagents 目录 → 空态（发现层防御，非判据）
    #[test]
    fn claude_source_missing_layout_is_empty() {
        super::super::reset_cache_for_tests();
        let f = claude_fixture("sess-4");
        assert!(collect_with(&f.root, "nope").is_empty());
        std::fs::create_dir_all(f.root.join("proj--e--demo").join("flat")).unwrap(); // 无 subagents/ 的目录
        assert!(collect_with(&f.root, "flat").is_empty());
        super::super::reset_cache_for_tests();
    }

    /// spawnTs 升序（None 排尾）
    #[test]
    fn claude_source_orders_by_spawn_ts() {
        super::super::reset_cache_for_tests();
        let f = claude_fixture("sess-5");
        f.add_agent(
            "sess-5",
            "late",
            r#"{"agentType":"A"}"#,
            &(agent_line("2026-10-08T09:00:00Z", 1, 0, 0, 1) + "\n"),
        );
        f.add_agent(
            "sess-5",
            "early",
            r#"{"agentType":"B"}"#,
            &(agent_line("2026-10-08T07:00:00Z", 1, 0, 0, 1) + "\n"),
        );
        let v = collect_with(&f.root, "sess-5");
        let ids: Vec<&str> = v.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["early", "late"]);
        super::super::reset_cache_for_tests();
    }

    /// review 修复：session_id 含路径分隔符 / .. → 空态（信任边界与 kimi.rs
    /// collect_with 的 sessions 根校验对称——防 ../ 逃逸把发现层引出 projects 根）
    #[test]
    fn claude_source_rejects_traversal_session_id() {
        super::super::reset_cache_for_tests();
        let f = claude_fixture("sess-9");
        f.add_agent("sess-9", "ok1", r#"{"agentType":"Plan"}"#, "");
        // 逃逸目标真实存在：proj--e--demo/../escape/subagents/（= projects 根下的
        // root/escape/…）有登记源——无守卫时 locate 会命中它
        let escape = f.root.join("escape");
        std::fs::create_dir_all(escape.join("subagents")).unwrap();
        std::fs::write(
            escape.join("subagents").join("agent-esc.meta.json"),
            r#"{"agentType":"Plan"}"#,
        )
        .unwrap();
        std::fs::write(escape.join("subagents").join("agent-esc.jsonl"), "").unwrap();
        assert!(
            collect_with(&f.root, "../escape").is_empty(),
            "../ 逃逸拒绝"
        );
        assert!(collect_with(&f.root, "..").is_empty());
        assert!(collect_with(&f.root, "..\\escape").is_empty());
        // 正常 session_id 不受影响（守卫只拦路径形态）
        assert_eq!(collect_with(&f.root, "sess-9").len(), 1);
        super::super::reset_cache_for_tests();
    }

    /// 倒序分块扫描边界：新→旧、收齐即停、以换行收尾的文件无尾半行
    #[test]
    fn scan_rev_visits_newest_first_and_stops_early() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("parent.jsonl");
        let mut body = String::new();
        for i in 0..100 {
            body.push_str(&format!("{{\"n\":{i}}}\n"));
        }
        std::fs::write(&p, body).unwrap();
        let mut seen: Vec<u32> = Vec::new();
        let carry = scan_parent_rev(&p, |line| {
            let n: u32 = serde_json::from_str::<serde_json::Value>(line).unwrap()["n"]
                .as_u64()
                .unwrap() as u32;
            seen.push(n);
            n == 90 // 第 90 行（新→旧第 10 个）命中即停
        })
        .unwrap();
        assert!(
            carry.is_empty(),
            "以换行收尾的文件无尾半行（P1-1 返回值语义）"
        );
        assert_eq!(seen.first(), Some(&99), "新→旧");
        assert_eq!(seen.last(), Some(&90));
        assert_eq!(seen.len(), 10, "收齐即停：不扫到头");
    }

    fn notify(id: &str) -> String {
        // 父 JSONL 通知行（spec §2.1：子 agent 停止追加 task-notification；实盘同 id 多次）
        format!(
            r#"{{"type":"user","timestamp":"2026-10-08T07:40:00Z","message":{{"role":"user","content":"<task-notification><task-id>{id}</task-id><summary>Agent stopped</summary></task-notification>"}}}}"#
        )
    }
    fn sendmsg(id: &str) -> String {
        // 父 JSONL SendMessage 续跑行（assistant tool_use，input.to 指向子 agent id）
        format!(
            r#"{{"type":"assistant","timestamp":"2026-10-08T07:41:00Z","message":{{"role":"assistant","content":[{{"type":"tool_use","id":"tu1","name":"SendMessage","input":{{"to":"{id}","body":"continue"}}}}]}}}}"#
        )
    }
    fn agent_line(ts: &str, inp: u64, cr: u64, cc: u64, out: u64) -> String {
        // 子 agent 转写行（每条消息带 timestamp + usage 四桶——spec §2.1）
        format!(
            r#"{{"timestamp":"{ts}","type":"assistant","message":{{"role":"assistant","usage":{{"input_tokens":{inp},"cache_read_input_tokens":{cr},"cache_creation_input_tokens":{cc},"output_tokens":{out}}}}}}}"#
        )
    }

    /// P0 回归锁（spec §2.1 排雷① + §7）：通知 ≠ 终态——stop 后 SendMessage 复现 running
    #[test]
    fn stop_then_sendmessage_is_running_again() {
        let mut last: HashMap<String, LastEvent> = HashMap::new();
        for line in [notify("a88"), sendmsg("a88")] {
            if let Some(ev) = classify_parent_line(&line) {
                apply_event(&mut last, &ev);
            }
        }
        assert!(
            is_running(true, last.get("a88").copied()),
            "stop → resume 后必须回到运行中（实盘 a588fb48 案例）"
        );
        // 反向：只有通知 → 不在跑
        let mut last2: HashMap<String, LastEvent> = HashMap::new();
        if let Some(ev) = classify_parent_line(&notify("a88")) {
            apply_event(&mut last2, &ev);
        }
        assert!(!is_running(true, last2.get("a88").copied()));
        // 同 id 多次通知（实盘 3 条）仍是 Stop
        let mut last3: HashMap<String, LastEvent> = HashMap::new();
        for _ in 0..3 {
            if let Some(ev) = classify_parent_line(&notify("a88")) {
                apply_event(&mut last3, &ev);
            }
        }
        assert_eq!(last3.get("a88"), Some(&LastEvent::Stop));
    }

    /// 倒序重建语义（spec §5.1）：新→旧扫描，每 id 取最新事件；无事件 → Spawn
    #[test]
    fn reverse_rebuild_takes_latest_event() {
        let mut seen: HashMap<String, LastEvent> = HashMap::new();
        // 倒序喂：新（resume）→ 旧（stop）
        for line in [sendmsg("b1"), notify("b1")] {
            if let Some(ev) = classify_parent_line(&line) {
                apply_event_rev(&mut seen, &ev);
            }
        }
        assert_eq!(seen.get("b1"), Some(&LastEvent::Resume));
        assert!(is_running(true, None), "扫到头无事件 = spawn 态（运行中）");
    }

    /// 行分类：无关行/坏 JSON/别的工具调用 → None；SendMessage 给别的 id 不影响本 id
    #[test]
    fn classify_ignores_irrelevant_lines() {
        assert_eq!(classify_parent_line("not json"), None);
        assert_eq!(
            classify_parent_line(r#"{"type":"user","message":{"role":"user","content":"hi"}}"#),
            None
        );
        let other = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","name":"Read","input":{"to":"x"}}]}}"#;
        assert_eq!(
            classify_parent_line(other),
            None,
            "非 SendMessage 工具调用不构成 resume"
        );
    }

    /// usage 四桶累计 + 首条 timestamp 捕获（timestamp 优先于 usage 在场性）
    #[test]
    fn accumulate_usage_and_first_ts() {
        let mut acc = TokenUsage::default();
        let mut first = None;
        accumulate_agent_line(
            r#"{"timestamp":"2026-10-08T07:36:35.508Z","type":"user","message":{"role":"user","content":"go"}}"#,
            &mut acc,
            &mut first,
        );
        assert_eq!(
            first.as_deref(),
            Some("2026-10-08T07:36:35.508Z"),
            "首条 user 消息也带 timestamp"
        );
        accumulate_agent_line(
            &agent_line("2026-10-08T07:36:40Z", 1000, 2000, 0, 500),
            &mut acc,
            &mut first,
        );
        accumulate_agent_line(
            &agent_line("2026-10-08T07:37:00Z", 24, 56000, 7, 8812),
            &mut acc,
            &mut first,
        );
        assert_eq!(
            acc,
            TokenUsage {
                input: 1024,
                cache_read: 58000,
                cache_creation: 7,
                output: 9312
            }
        );
        assert_eq!(
            first.as_deref(),
            Some("2026-10-08T07:36:35.508Z"),
            "首条 timestamp 不被覆盖"
        );
    }

    /// meta 解析（spec §2.1：{"agentType":"Plan","description":…}）+ 文件名提 id
    #[test]
    fn parse_meta_and_id() {
        let m = parse_meta(
            r#"{"agentType":"Plan","description":"设计新建会话两改动实现方案","toolUseId":"tu9","spawnDepth":1}"#,
        )
        .unwrap();
        assert_eq!(m.agent_type.as_deref(), Some("Plan"));
        assert_eq!(
            agent_id_from_meta_name("agent-a86f0c2d.meta.json"),
            Some("a86f0c2d")
        );
        assert_eq!(agent_id_from_meta_name("plan.md"), None);
        assert!(parse_meta("broken").is_none());
    }
}
