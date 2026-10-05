//! 账本集成测试：走真实 DB 文件验证「分批落库 → 明细/日聚合/会话维度/游标落位 → 区间查询 →
//! 保留期清理」全链。
//!
//! **本文件的行数与存在性断言一律用 `support::open_ledger_db(tag)` 的私有库**
//! （每条用例一个库文件）——`support::setup()` 的库是**整个二进制共用的**，绝对行数断言
//! 会在用例之间互相污染（§3.2.2 阻塞 2）。**会碰全局设置库**的用例（Task 11/13 追加到本文件的
//! 两条采集器落库用例——**集成构建**下采集器经 `ctx.provider_rules()` 回落到 `settings::load()` 读全局
//! `DB`；claude 那条还要 `settings::save`）再额外调 `support::setup()`，两者互不干扰。
//! （lib 单测构建不走这条路：`CollectContext::new` 注入空规则 → 采集器不读设置库，
//! 见 GC 21 / §3.2.3 FIX-6。）
// `setup()` 被两条用例调用（全局入口冒烟 + Task 11 的 claude 三态落库）：全局 `DB` 是 `Lazy`，
// 不重定向 HOME/MAM_HOME 就会读写开发机真实 `~/.mam/mam.db`（D-07）；其余用例全走私有库、
// 不碰 `setup()`，因此 `INIT` / `setup` 在本 target 有调用方、不产生 dead_code 告警（零新告警门禁）。
mod support;

use std::collections::BTreeMap;

use multi_agents_manager_lib::database::dao::usage as dao;
use multi_agents_manager_lib::services::usage::delta::{
    CursorDelta, DetailDelta, DetailKey, SessionCounters, SessionDimDelta, SourceDelta,
};
use multi_agents_manager_lib::services::usage::ledger;
use multi_agents_manager_lib::services::usage::model::{SourceKind, UsageBuckets, UsageSourceId};
use multi_agents_manager_lib::services::usage::semantics::CacheSemantics;

/// 明细行构造器：`hour` = `"YYYY-MM-DDTHH"`，`day_key` 取前 10 字符（与采集器同一派生规则）
fn detail_row(session_id: &str, hour: &str, input_fresh: i64) -> DetailDelta {
    DetailDelta {
        source_id: "claude".into(),
        provider_kind: SourceKind::Inferred,
        key: DetailKey {
            session_id: session_id.into(),
            hour_key: hour.into(),
            day_key: hour.chars().take(10).collect(),
            project_key: "proj".into(),
            model: "m".into(),
            provider: "p".into(),
        },
        buckets: UsageBuckets {
            input_fresh,
            ..Default::default()
        },
        request_total: input_fresh,
        requests: 1,
        user_est: None,
        cache_semantics: CacheSemantics::Exclusive,
        counters: SessionCounters::default(),
        ts_ms: 1_700_000_000_000,
    }
}

/// 带 turn 时长样本与一次工具调用的明细行（`merge_samples` 的读改写面）
fn detail_with_samples(
    session_id: &str,
    hour: &str,
    turn_ms: i64,
    tool: &str,
    tool_ms: i64,
) -> DetailDelta {
    let mut r = detail_row(session_id, hour, 1);
    r.counters = SessionCounters {
        turns: 1,
        tool_calls: 1,
        tool_ms,
        turn_ms: vec![turn_ms],
        tool_stats: [(tool.to_string(), (1, tool_ms))].into_iter().collect(),
        ..Default::default()
    };
    r
}

#[test]
fn ledger_round_trip_through_real_db_file() {
    // 本用例独占一个库文件（不再 database::open() 到全局库）
    let mut conn = support::open_ledger_db("ledger_round_trip");
    let delta = SourceDelta {
        details: vec![DetailDelta {
            source_id: "claude".into(),
            provider_kind: SourceKind::Inferred,
            key: DetailKey {
                session_id: "s1".into(),
                hour_key: "2026-10-03T09".into(),
                day_key: "2026-10-03".into(),
                project_key: "proj".into(),
                model: "m1".into(),
                provider: "p1".into(),
            },
            buckets: UsageBuckets {
                input_fresh: 100,
                cache_read: 800,
                output: 20,
                ..Default::default()
            },
            request_total: 900,
            requests: 1,
            user_est: Some(3),
            cache_semantics: CacheSemantics::Exclusive,
            counters: Default::default(),
            ts_ms: 1_700_000_000_000,
        }],
        sessions: vec![SessionDimDelta {
            session_id: "s1".into(),
            project_key: "proj".into(),
            project_label: "Proj".into(),
            project_path_raw: "/p/Proj".into(),
            project_realpath: "/p/proj".into(),
            title: Some("标题".into()),
            is_subagent: false,
            parent_session_id: None,
            originator: None,
            last_seen_at: 1_700_000_000_000,
        }],
        cursors: vec![CursorDelta {
            session_id: "f1".into(),
            fingerprint: "ab".into(),
            byte_offset: 128,
            ordinal: 3,
            last_cumulative: None,
            mtime_ms: 7,
            file_size: 128,
            state_json: "{}".into(),
        }],
        parsed_files: 1,
        new_records: 1,
    };
    let stats = ledger::apply_delta_conn(&mut conn, UsageSourceId::Claude, &delta).unwrap();
    assert!(stats.max_batch_rows <= ledger::LEDGER_BATCH_ROWS);
    // 明细 + 日聚合 + 会话维度 + 游标都落位（**同一个私有连接**回读；绝对行数成立）
    assert_eq!(dao::count_rows_conn(&conn, "usage_detail"), 1);
    assert_eq!(
        dao::count_rows_conn(&conn, "usage_daily"),
        1,
        "日聚合同事务维护"
    );
    assert_eq!(dao::count_rows_conn(&conn, "usage_session"), 1);
    let cursors = dao::load_cursors_conn(&conn, "claude");
    assert_eq!(cursors["f1"].byte_offset, 128);
    // 查询回读（含 R1/R2：记录级项目键与可空 userEst 都要能读回）
    let rows = dao::query_daily_conn(&conn, "2026-10-01", "2026-10-31");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].buckets.cache_read, 800);
    assert_eq!(rows[0].provider_kind, SourceKind::Inferred);
    assert_eq!(rows[0].project_key, "proj", "R2：日聚合按记录级项目键落位");
    assert_eq!(
        rows[0].user_est,
        Some(3),
        "R1：日档必须出真值（不是 null、不是 0）"
    );
    let detail = dao::query_detail_conn(&conn, "2026-10-03T00", "2026-10-03T23");
    assert_eq!(detail.len(), 1);
    assert_eq!(detail[0].project_key, "proj", "R2：小时档读记录级项目键");
    assert_eq!(detail[0].user_est, Some(3), "R1：小时档读可空 userEst");
    // 保留期清理：只删明细，日聚合永久
    let deleted = ledger::purge_expired_conn(&conn, 1, 1_800_000_000_000); // 1 天后 = 2026-10-03 之后
    assert_eq!(deleted, 1);
    assert_eq!(dao::count_rows_conn(&conn, "usage_detail"), 0);
    assert_eq!(dao::count_rows_conn(&conn, "usage_daily"), 1);
}

/// **B3 回归（落库面）**：`sessions` / `cursors` 也必须分批。
/// 真机形态：codex 425 个 rollout 游标 + 约 450 条 `session_meta`（25 个文件含父子双线程）。
/// 旧实现把两类塞进**一个**批次 → `rows = 875 > 200` → `apply_delta` 返回 `Err`，
/// 而明细/日聚合前几批已提交 → 账本半写。本用例断言：① 不报错；② 875 行全部落库
/// （会话 450 条、游标 425 条都能读回）——半写会在计数上立刻暴露。
#[test]
fn sessions_and_cursors_survive_the_row_limit() {
    let mut conn = support::open_ledger_db("survive_row_limit");
    let delta = SourceDelta {
        sessions: (0..450)
            .map(|i| SessionDimDelta {
                session_id: format!("sess-{i}"),
                project_key: "proj".into(),
                project_label: "Proj".into(),
                project_path_raw: "/p/Proj".into(),
                project_realpath: "/p/proj".into(),
                title: None,
                is_subagent: i % 3 == 0,
                parent_session_id: None,
                originator: None,
                last_seen_at: 1_700_000_000_000,
            })
            .collect(),
        cursors: (0..425)
            .map(|i| CursorDelta {
                session_id: format!("rollout-{i}"),
                fingerprint: "ab".into(),
                byte_offset: 128,
                ordinal: 3,
                last_cumulative: None,
                mtime_ms: 7,
                file_size: 128,
                state_json: "{}".into(),
            })
            .collect(),
        ..Default::default()
    };
    let stats = ledger::apply_delta_conn(&mut conn, UsageSourceId::Codex, &delta).unwrap();
    assert_eq!(stats.rows, 875, "875 行一行都不能丢");
    assert!(
        stats.max_batch_rows <= ledger::LEDGER_BATCH_ROWS,
        "任何单事务都不得超上限（旧实现在这里就是 875 行一批 → Err）"
    );
    assert_eq!(stats.batches, 6, "450 会话 → 3 批；425 游标 → 3 批");
    // 半写检测：会话表与游标表都必须完整
    assert_eq!(
        dao::count_rows_conn(&conn, "usage_session"),
        450,
        "会话维度不得半写"
    );
    assert_eq!(
        dao::load_cursors_conn(&conn, "codex").len(),
        425,
        "游标不得半写"
    );
}

/// **单会话 > `LEDGER_BATCH_ROWS`（首轮全量扫描的长会话就是这个形状）**：
/// 「同会话同批」（W-03）与「单事务 ≤200」（GC 2）在此不可兼得——实现退让到**六列行键**边界
/// （同一行键仍绝不跨批）。本用例锁住三件事：不因超限给整轮 `Err`（那会让该源永远采不到数）、
/// 一行不丢、单事务仍 ≤ 上限。
#[test]
fn oversized_single_session_lands_without_exceeding_the_row_limit() {
    let mut conn = support::open_ledger_db("oversized_single_session");
    // 250 行同一个会话、小时键两两不同（跨 11 个日键 → 日聚合 11 行）
    let details: Vec<DetailDelta> = (0..250)
        .map(|i| {
            detail_row(
                "long-session",
                &format!("2026-10-{:02}T{:02}", 1 + i / 24, i % 24),
                10,
            )
        })
        .collect();
    let delta = SourceDelta {
        details,
        ..Default::default()
    };
    let stats = ledger::apply_delta_conn(&mut conn, UsageSourceId::Claude, &delta).unwrap();
    assert!(
        stats.max_batch_rows <= ledger::LEDGER_BATCH_ROWS,
        "退让拆分后仍须守住 ≤{} 行，实测 {}",
        ledger::LEDGER_BATCH_ROWS,
        stats.max_batch_rows
    );
    assert_eq!(stats.batches, 3, "250 明细 → 200+50 两批；11 日聚合 → 1 批");
    assert_eq!(
        dao::count_rows_conn(&conn, "usage_detail"),
        250,
        "退让拆分不得丢行"
    );
    assert_eq!(
        dao::count_rows_conn(&conn, "usage_daily"),
        11,
        "日聚合按 (日, 项目, 供应商, 模型) 折叠"
    );
}

/// **W-03 的落库面**：同一个六列行键在**两个事务**里各落一次时，`merge_samples` 的读改写
/// 必须把样本累加——「行数不涨、数值偏小」正是它出错时的静默形态（计划正文没有用例锁它）。
/// 本用例用同一个行键跨**两轮**（两次 `apply_delta_conn`，每次一个事务）落两次，
/// 每轮各带 1 个 turn 时长样本与 1 次工具调用。
#[test]
fn same_row_key_merges_samples_across_transactions() {
    let mut conn = support::open_ledger_db("samples_across_rounds");
    let round1 = SourceDelta {
        details: vec![detail_with_samples("s1", "2026-10-03T09", 11, "Bash", 30)],
        ..Default::default()
    };
    let round2 = SourceDelta {
        details: vec![detail_with_samples("s1", "2026-10-03T09", 22, "Read", 7)],
        ..Default::default()
    };
    ledger::apply_delta_conn(&mut conn, UsageSourceId::Claude, &round1).unwrap();
    ledger::apply_delta_conn(&mut conn, UsageSourceId::Claude, &round2).unwrap();
    // 前提断言（防假绿）：两轮必须落在**同一行**，否则「两行各写一半」会冒充累加
    assert_eq!(
        dao::count_rows_conn(&conn, "usage_detail"),
        1,
        "同一行键只有一行"
    );
    let (turn_ms, tool_stats): (String, String) = conn
        .query_row(
            "SELECT turn_ms, tool_stats FROM usage_detail WHERE session_id = 's1'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        turn_ms, "11\n22",
        "跨事务的读改写必须看到上一轮已提交的样本"
    );
    let tools: BTreeMap<String, (i64, i64)> = serde_json::from_str(&tool_stats).unwrap();
    assert_eq!(tools.len(), 2, "两个工具都要在（丢一个就是静默的样本损失）");
    assert_eq!(tools.get("Bash"), Some(&(1, 30)));
    assert_eq!(tools.get("Read"), Some(&(1, 7)));
    // 请求数同样按行累加（行数不涨，正是 W-03 说的「静默」形态）
    let detail = dao::query_detail_conn(&conn, "2026-10-03T00", "2026-10-03T23");
    assert_eq!(detail.len(), 1);
    assert_eq!(detail[0].requests, 2);
}

/// **全局入口（`apply_delta`）的冒烟覆盖**：它是本任务唯一的**生产落库路径**
/// （「取全局锁 → 提交 → drop 锁 → 让出 1ms」的循环），别处只在 Task 22 的锁竞争测量与
/// Task 23B 的 `run_collection` 两轮增量里被量化验收——本条补住「零覆盖」。
///
/// GC 19 口径：断言全部取自**入参增量与 `LedgerWriteStats`**（不读共享库的行数、不用下标），
/// 属「自洽/相对式」断言，因此可以走 `setup()` 的共享库；`setup()` 是 `Once`，
/// 与同二进制里那些只碰私有库的用例互不干扰（HOME/MAM_HOME 只被重定向一次）。
#[test]
fn global_entry_point_writes_the_same_batches_and_yields() {
    support::setup();
    let delta = SourceDelta {
        details: vec![detail_row("g1", "2026-10-03T09", 5)],
        sessions: vec![SessionDimDelta {
            session_id: "g1".into(),
            project_key: "proj".into(),
            project_label: "Proj".into(),
            project_path_raw: "/p/Proj".into(),
            project_realpath: "/p/proj".into(),
            title: None,
            is_subagent: false,
            parent_session_id: None,
            originator: None,
            last_seen_at: 1_700_000_000_000,
        }],
        cursors: vec![CursorDelta {
            session_id: "g1".into(),
            fingerprint: "ab".into(),
            byte_offset: 128,
            ordinal: 3,
            last_cumulative: None,
            mtime_ms: 7,
            file_size: 128,
            state_json: "{}".into(),
        }],
        ..Default::default()
    };
    let stats = ledger::apply_delta(UsageSourceId::Claude, &delta).unwrap();
    assert_eq!(
        stats.rows, 4,
        "1 明细 + 1 日聚合（同批折叠出）+ 1 会话 + 1 游标"
    );
    assert_eq!(stats.batches, 4, "四类各自一批，每批 1 行");
    assert_eq!(
        stats.max_batch_rows, 1,
        "全局入口与私有连接入口的批次切片必须一致"
    );
}

/// 会话维度行构造器（`usage_session` 的主键是 `(source_id, session_id)`）
fn session_row(session_id: &str) -> SessionDimDelta {
    SessionDimDelta {
        session_id: session_id.into(),
        project_key: "proj".into(),
        project_label: "Proj".into(),
        project_path_raw: "/p/Proj".into(),
        project_realpath: "/p/proj".into(),
        title: None,
        is_subagent: false,
        parent_session_id: None,
        originator: None,
        last_seen_at: 1_700_000_000_000,
    }
}

/// 游标行构造器（`usage_cursor` 的主键是 `(source_id, session_id)`）
fn cursor_row(session_id: &str) -> CursorDelta {
    CursorDelta {
        session_id: session_id.into(),
        fingerprint: "ab".into(),
        byte_offset: 128,
        ordinal: 3,
        last_cumulative: None,
        mtime_ms: 7,
        file_size: 128,
        state_json: "{}".into(),
    }
}

/// **评审 Important #1 的回归（行级失败必须响亮失败）**：DAO 的行写入失败只在库内 `log::warn!` 后
/// 跳过（`dao/usage.rs:225/377/441`），该行就没了。若 `write_batch` 不比对这些返回值，
/// 本批照常 `commit` → 游标批照常提交 → `apply_delta` 返回 `Ok` 且 `stats.rows` 仍按**计划行数**上报
/// → 采集器推进水位 → **该行永久缺失**（本任务要消灭的「账本半写」只是降到了行粒度）。
///
/// 构造：给 `usage_detail` 挂一个只拦「毒行」的 BEFORE INSERT 触发器（**表结构不动**），
/// 批里放 1 条正常行 + 1 条毒行 —— 正常行必须先写成功，才会真正走到「部分写入」这个分支。
#[test]
fn detail_row_write_failure_aborts_the_whole_round() {
    let mut conn = support::open_ledger_db("detail_write_failure");
    conn.execute_batch(
        "CREATE TRIGGER tmp_boom_detail BEFORE INSERT ON usage_detail
         WHEN NEW.provider = 'boom'
         BEGIN SELECT RAISE(ABORT, 'boom-detail'); END;",
    )
    .unwrap();

    let good = detail_row("s1", "2026-10-03T09", 5);
    let mut bad = detail_row("s1", "2026-10-03T10", 7);
    bad.key.provider = "boom".into();

    // 前提断言（防假绿）：毒行**真的**写不进去、正常行**真的**写得进去
    let probe = "INSERT INTO usage_detail (source_id, session_id, hour_key, project_key, day_key,
                                           model, provider, first_seen_at, updated_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,0,0)";
    assert!(
        conn.execute(
            probe,
            rusqlite::params![
                good.source_id,
                good.key.session_id,
                good.key.hour_key,
                good.key.project_key,
                good.key.day_key,
                good.key.model,
                good.key.provider
            ]
        )
        .is_ok(),
        "前提不成立：正常行都写不进去，本用例测不到「部分写入」分支"
    );
    assert!(
        conn.execute(
            probe,
            rusqlite::params![
                bad.source_id,
                bad.key.session_id,
                bad.key.hour_key,
                bad.key.project_key,
                bad.key.day_key,
                bad.key.model,
                bad.key.provider
            ]
        )
        .is_err(),
        "前提不成立：毒行能写进去，触发器没生效"
    );
    conn.execute("DELETE FROM usage_detail", []).unwrap(); // 清掉前提探针留下的行

    let delta = SourceDelta {
        details: vec![good, bad],
        sessions: vec![session_row("s1")],
        cursors: vec![cursor_row("f1")],
        ..Default::default()
    };
    let err = ledger::apply_delta_conn(&mut conn, UsageSourceId::Claude, &delta)
        .expect_err("批内有一行写失败 → 整轮必须 Err");
    assert_eq!(err.code, "usage-internal");
    assert!(
        err.detail.contains("1/2"),
        "错误信息要报出实际写入行数，实测：{}",
        err.detail
    );
    // 该批**整体回滚**（连正常行也不落）→ 不留半写
    assert_eq!(
        dao::count_rows_conn(&conn, "usage_detail"),
        0,
        "失败批必须整体回滚"
    );
    // 整轮 Err → 后面的批次根本没跑：日聚合 / 会话 / 游标一律不推进
    assert_eq!(dao::count_rows_conn(&conn, "usage_daily"), 0);
    assert_eq!(dao::count_rows_conn(&conn, "usage_session"), 0);
    assert!(
        dao::load_cursors_conn(&conn, "claude").is_empty(),
        "游标不得推进"
    );
}

/// **评审 Important #1 的后果链**：失败发生在**游标批**时，明细 / 日聚合 / 会话三批已经提交
/// （各自是独立事务）。若不比对 DAO 返回值，正常的 `f1` 会照常落库 → **水位推进**，
/// 而毒行对应的源数据从此不再被读（「永久缺失」）。
/// 本用例断言：游标批**整体回滚**（连正常的 `f1` 也不落）→ 水位一步都不推进 → 下一轮整源重扫。
#[test]
fn cursor_row_write_failure_keeps_the_watermark_unadvanced() {
    let mut conn = support::open_ledger_db("cursor_write_failure");
    conn.execute_batch(
        "CREATE TRIGGER tmp_boom_cursor BEFORE INSERT ON usage_cursor
         WHEN NEW.session_id = 'rollout-boom'
         BEGIN SELECT RAISE(ABORT, 'boom-cursor'); END;",
    )
    .unwrap();

    let good = cursor_row("f1");
    let bad = cursor_row("rollout-boom");
    let probe = "INSERT INTO usage_cursor (source_id, session_id, fingerprint, byte_offset, ordinal,
                                           last_cumulative, mtime_ms, file_size, state_json, updated_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,0)";
    assert!(
        conn.execute(
            probe,
            rusqlite::params![
                "claude",
                good.session_id,
                good.fingerprint,
                good.byte_offset,
                good.ordinal,
                good.last_cumulative,
                good.mtime_ms,
                good.file_size,
                good.state_json
            ]
        )
        .is_ok(),
        "前提不成立：正常游标都写不进去，本用例测不到「部分写入」分支"
    );
    assert!(
        conn.execute(
            probe,
            rusqlite::params![
                "claude",
                bad.session_id,
                bad.fingerprint,
                bad.byte_offset,
                bad.ordinal,
                bad.last_cumulative,
                bad.mtime_ms,
                bad.file_size,
                bad.state_json
            ]
        )
        .is_err(),
        "前提不成立：毒游标能写进去，触发器没生效"
    );
    conn.execute("DELETE FROM usage_cursor", []).unwrap();

    let delta = SourceDelta {
        details: vec![detail_row("s1", "2026-10-03T09", 5)],
        sessions: vec![session_row("s1")],
        cursors: vec![good, bad],
        ..Default::default()
    };
    let err = ledger::apply_delta_conn(&mut conn, UsageSourceId::Claude, &delta)
        .expect_err("游标批里有一行写失败 → 整轮必须 Err");
    assert_eq!(err.code, "usage-internal");
    assert!(
        err.detail.contains("1/2"),
        "错误信息要报出游标写入行数，实测：{}",
        err.detail
    );
    // 前提断言（防假绿）：失败确实发生在**游标批**——前三批已提交；
    // 否则「游标没推进」可能只是因为前面就失败了，本用例什么也没测到
    assert_eq!(
        dao::count_rows_conn(&conn, "usage_detail"),
        1,
        "明细批应已提交"
    );
    assert_eq!(
        dao::count_rows_conn(&conn, "usage_daily"),
        1,
        "日聚合批应已提交"
    );
    assert_eq!(
        dao::count_rows_conn(&conn, "usage_session"),
        1,
        "会话批应已提交"
    );
    // 游标批整体回滚：连正常的 f1 也不落 → 水位一步都不推进
    assert!(
        dao::load_cursors_conn(&conn, "claude").is_empty(),
        "游标批必须整体回滚：半推进的水位会让毒行对应的源数据永久丢失"
    );
}

/// **B6 回归（Task 11 Step 6）**：供应商三态必须由采集器经 `DeltaBuilder::provider_of` 落进
/// `usage_detail.provider_kind`。采集器若直接调 `provider::resolve_provider`，
/// `kind_by_provider` 永远是空的 → `DeltaBuilder::detail()` 只能 `unwrap_or(Unknown)`
/// → 明细与日聚合的 `provider_kind` 恒 `unknown` → 说明书 D8 / §4.3 的
/// 「实测 / 推断 / 未知」三态在 UI 上永久退化（这是需求项，不是诊断列）。
#[test]
fn claude_provider_kind_is_persisted_to_usage_detail() {
    // 全局设置库：本用例要 settings::save(...)，且**集成构建**下采集器经 `ctx.provider_rules()`
    // 回落到 `settings::load()` → 必须先重定向 HOME/MAM_HOME（否则写/读开发机真实库）
    // （lib 单测构建不走这条路：`CollectContext::new` 注入空规则，见 §3.2.3 FIX-6）
    support::setup();
    // 账本库：**本用例私有**（不用全局 DB —— 见 Global Constraints 19 / §3.2.2 阻塞 2）
    let mut conn = support::open_ledger_db("claude_provider_kind");
    // claude 没有供应商字段（矩阵：❌ 无字段 → 推断）→ 配一条用户规则，期望判定为 Inferred
    // Task 20 fix round 1（裁决 F）：`settings::save` 现在返回 `Result`（写后回读校验），
    // 此处是本任务之外的唯一调用点（Task 11 的用例夹具）→ 显式 `expect`，不让夹具静默失败。
    multi_agents_manager_lib::services::usage::settings::save(
        &multi_agents_manager_lib::services::usage::model::UsageSettings {
            provider_map_rules: r#"{"rules":[{"prefix":"claude-","provider":"anthropic"}]}"#.into(),
            ..Default::default()
        },
    )
    .expect("测试夹具：写入用量设置必须成功");
    let home = tempfile::tempdir().unwrap();
    let proj = home.path().join(".claude/projects/-p-A");
    std::fs::create_dir_all(&proj).unwrap();
    std::fs::write(
        proj.join("sess-kind.jsonl"),
        format!(
            "{}\n",
            serde_json::json!({"type":"assistant","sessionId":"s-kind","cwd":"/p/A","uuid":"u1",
                "message":{"id":"m1","model":"claude-sonnet-4",
                    "usage":{"input_tokens":10,"cache_read_input_tokens":0,
                             "cache_creation_input_tokens":0,"output_tokens":1},"content":[]},
                "timestamp":"2026-10-03T09:00:00Z"})
        ),
    )
    .unwrap();
    let cursors = std::collections::HashMap::new();
    let ctx = multi_agents_manager_lib::services::usage::collect::CollectContext::new(
        home.path(),
        1_700_000_000_000,
        &cursors,
    );
    use multi_agents_manager_lib::services::usage::collect::UsageCollector;
    let delta = multi_agents_manager_lib::services::usage::collectors::claude::ClaudeCollector
        .collect(&ctx)
        .unwrap();
    ledger::apply_delta_conn(&mut conn, UsageSourceId::Claude, &delta).unwrap();

    // 明细回读用**全时段哨兵窗口**：本用例的 hour_key 由 fixture 的记录时间戳
    // （`"timestamp":"2026-10-03T09:00:00Z"`）经 `hour_key_of(ts, &TZ)`（`TZ = HostLocal`）换算，
    // 键值随宿主时区漂（UTC+8 → `2026-10-03T17`）。字面量窗口 `2026-10-03T00..T23` 只在
    // UTC-9..+14 的宿主上同代，UTC-10 以西就滑到前一天 → `.expect` 假红。
    // 哨兵窗口与宿主时区无关，**永远不可能再漂**（§3.2.3 FIX-3 同类）。
    let rows = dao::query_detail_conn(&conn, "0000-00-00T00", "9999-99-99T99");
    let row = rows
        .iter()
        .find(|r| r.session_id == "s-kind")
        .expect("采集器产出的明细必须落库");
    assert_eq!(row.provider, "anthropic");
    assert_eq!(
        row.provider_kind,
        SourceKind::Inferred,
        "provider_kind 必须经 provider_of 落库；恒 Unknown = 采集器绕过了唯一入口（B6）"
    );
    // 日聚合仍用**同代字面量窗口**（2026-10）：它同时锁住"明细/日聚合的日键来自 fixture 的
    // 2026-10-03 记录时间戳"——若 ts 解析失败退化到 ctx.now_ms（2023-11-14），这里立刻空集。
    let daily = dao::query_daily_conn(&conn, "2026-10-01", "2026-10-31");
    assert!(
        daily
            .iter()
            .any(|r| r.provider == "anthropic" && r.provider_kind == SourceKind::Inferred),
        "日聚合必须带三态（B6）：{daily:?}"
    );
}

/// **B6 回归（第二态，Task 13 Step 5）**：kimi 的供应商内嵌在模型名前缀里
/// （`ollama-cloud/glm-5.3-flash` 实测）→ `provider_kind` 必须是 **Measured**（直接字段/前缀），
/// 并且要真的落进 `usage_detail`。claude 用用户规则得 `Inferred`、kimi 靠前缀得 `Measured`
/// ——两态都锁住，才说明 `DeltaBuilder::provider_of` 真的在写 `kind_by_provider`
/// （绕过它的实现两态都退化成 `Unknown`）。
#[test]
fn kimi_provider_kind_measured_is_persisted() {
    // 全局设置库：**集成构建**下采集器经 `ctx.provider_rules()` 回落到 `settings::load()`
    // → 读**全局 DB**，因此必须先重定向 HOME/MAM_HOME（否则读/首次创建开发机真实
    // `~/.mam/mam.db`）。lib 单测构建不读设置库（§3.2.3 FIX-6）。
    support::setup();
    // 账本库：**本用例私有**（Global Constraints 19 / §3.2.2 阻塞 2）
    let mut conn = support::open_ledger_db("kimi_provider_kind");
    let home = tempfile::tempdir().unwrap();
    let wire_dir = home
        .path()
        .join(".kimi-code/sessions/wd_p/session_k/agents/main");
    std::fs::create_dir_all(&wire_dir).unwrap();
    std::fs::write(
        wire_dir.join("wire.jsonl"),
        format!(
            "{}\n",
            serde_json::json!({"type":"usage.record","model":"ollama-cloud/glm-5.3-flash",
                "usageScope":"turn",
                "usage":{"inputOther":100,"output":10,"inputCacheRead":0,"inputCacheCreation":0},
                "time":1699996400000i64})
        ),
    )
    .unwrap();
    std::fs::write(
        home.path().join(".kimi-code/session_index.jsonl"),
        format!(
            "{}\n",
            serde_json::json!({"sessionId":"session_k","sessionDir":"sessions/wd_p/session_k",
                "workDir":"/p/Kimi"})
        ),
    )
    .unwrap();
    let cursors = std::collections::HashMap::new();
    let ctx = multi_agents_manager_lib::services::usage::collect::CollectContext::new(
        home.path(),
        1_700_000_000_000,
        &cursors,
    );
    use multi_agents_manager_lib::services::usage::collect::UsageCollector;
    let delta = multi_agents_manager_lib::services::usage::collectors::kimi::KimiCollector
        .collect(&ctx)
        .unwrap();
    ledger::apply_delta_conn(&mut conn, UsageSourceId::Kimi, &delta).unwrap();

    // **回读窗口必须与 fixture 同代**：本用例的明细 `hour_key` 由 fixture 的记录时间戳
    // （`"time":1699996400000` = `T0 - 1h` = 2023-11-14T21:13:20Z）经 `hour_key_of(ts, &TZ)`
    // （`TZ = SourceTz::HostLocal`）换算得到 → **2023-11-15T05**（UTC+8）。
    // 用全时段哨兵窗口（本文件 Task 23B 的既有写法）而不是字面量月份：字面量窗口
    // （旧正文写的是 `"2026-10-01T00".."2026-10-31T23"`）与 fixture 不同代 → SQL
    // `WHERE hour_key BETWEEN ?1 AND ?2` 命中空集 → 下面的 `.expect("kimi 明细必须落库")` panic。
    // 哨兵窗口与 fixture 的时间戳**永远不可能再漂**（§3.2.3 FIX-3）。
    let rows = dao::query_detail_conn(&conn, "0000-00-00T00", "9999-99-99T99");
    let row = rows
        .iter()
        .find(|r| r.session_id == "session_k")
        .expect("kimi 明细必须落库");
    assert_eq!(row.provider, "ollama-cloud");
    assert_eq!(row.model, "glm-5.3-flash", "D9：模型名去掉结构性前缀");
    assert_eq!(
        row.provider_kind,
        SourceKind::Measured,
        "模型名前缀 = 实测（B6）"
    );
    // **把上面那句"逐值推演"变成运行期锁**：明细的 hour_key 必须由 fixture 的记录时间戳
    // （1699996400000）在**宿主时区**下换算而来。用同一个函数现算，所以断言与宿主时区无关，
    // 但一旦有人把 fixture 的时间戳改掉、或让 hour_key 走了别的输入（如 ctx.now_ms =
    // 1700000000000，差 1 小时 → 键就不同），这里立刻红（§3.2.3 FIX-3）。
    assert_eq!(
        row.hour_key,
        multi_agents_manager_lib::services::usage::range::hour_key_of(
            1_699_996_400_000,
            &multi_agents_manager_lib::services::usage::range::SourceTz::HostLocal,
        ),
        "明细小时桶必须由 fixture 的记录时间戳换算（不得取自 ctx.now_ms / 不得换时区口径）"
    );
}
