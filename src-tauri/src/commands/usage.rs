//! 用量域 IPC（契约 §3）。纪律：
//! * **只有 `usage_collect` 会扫描**，且它有单飞 + 最小间隔保护（Task 10）；三个查询命令
//!   只读本地账本（零文件扫描）；
//! * 采集经 `spawn_blocking` 出主线程（同步命令在主线程执行，codex 全量可达 1–3 分钟）；
//! * 错误一律结构化 `UsageError`（前端按 `usage.rpc.<code>` 翻译）。
//!
//! ## ⚠️ 锁序（W-26，本层最容易踩的一条）
//! 本层**不得**自己取 `DB` 锁：三个查询命令把锁定留给 `query::dashboard` / `records` /
//! `export_csv`（它们的安全形式是「先持 `DB` 锁、guard 存活期间只调 `collect::cached_result()`」），
//! 而**持 `DB` 锁时绝不能再调采集入口**（`collect()` / `collect_with()` / `run_collection()`）——
//! 那会与在飞扫描的「飞行锁 → DB」构成 ABBA 死锁，挂死**整个 app 的 DB 访问**。
//! 采集入口在本文件里只有一处、且在**不持任何锁**的命令入口上（`usage_collect`）。
//! 锁：本模块 `mod tests` 的 `command_layer_never_locks_db_or_calls_collection_entries`。
//!
//! ## ⚠️ GC 3 / AGENTS.md 扫描预算契约
//! 用量采集**绝不进入 3 秒会话轮询热路径**：启动采集是 `lib.rs` setup 里
//! `spawn_initial_collection()` 的**延迟 3s 后台线程**（刻意如此，**不要**改成同步调用）。
//! 锁：`collect::collect_calls()` 的行为式用例在 `tests/usage_polling_budget_test.rs`。

use crate::services::usage::error::UsageError;
use crate::services::usage::model::*;
use crate::services::usage::{collect, query, settings};

#[tauri::command]
pub async fn usage_collect(force: bool) -> Result<UsageCollectResult, UsageError> {
    tauri::async_runtime::spawn_blocking(move || collect::collect(force))
        .await
        .map_err(|e| UsageError::internal(format!("采集任务异常: {e}")))
}

#[tauri::command]
pub async fn usage_dashboard(
    range: UsageRange,
    group_by: UsageGroupBy,
) -> Result<UsageDashboard, UsageError> {
    tauri::async_runtime::spawn_blocking(move || {
        query::dashboard(&range, group_by, crate::services::usage::now_ms())
    })
    .await
    .map_err(|e| UsageError::internal(format!("查询任务异常: {e}")))?
}

#[tauri::command]
pub async fn usage_records(
    range: UsageRange,
    group_by: UsageGroupBy,
    filters: UsageFilters,
) -> Result<UsageRecords, UsageError> {
    tauri::async_runtime::spawn_blocking(move || {
        query::records(&range, group_by, &filters, crate::services::usage::now_ms())
    })
    .await
    .map_err(|e| UsageError::internal(format!("查询任务异常: {e}")))?
}

#[tauri::command]
pub async fn usage_export_csv(
    range: UsageRange,
    group_by: UsageGroupBy,
    filters: UsageFilters,
) -> Result<String, UsageError> {
    tauri::async_runtime::spawn_blocking(move || {
        query::export_csv(&range, group_by, &filters, crate::services::usage::now_ms())
    })
    .await
    .map_err(|e| UsageError::internal(format!("导出任务异常: {e}")))?
}

/// 读用量设置。
///
/// **A-5（P2-3）**：改成 `async` + `spawn_blocking` —— `settings::load()` 会取全局
/// `DB.lock()` 并读 `mam.db`，同步命令在**主线程**上执行 ⇒ 最坏 0.5s UI 冻结
/// （与 `commands/session.rs:10-12` 同一个坑）。**JS 侧签名与返回形状一字未动**
/// （命令名 / 入参 / 返回都不变），契约 §3 冻结的是这三样、不含 sync/async。
#[tauri::command]
pub async fn usage_get_settings() -> UsageSettings {
    tauri::async_runtime::spawn_blocking(settings::load)
        .await
        .unwrap_or_else(|e| {
            // 任务 panic：不得让命令静默无返回。设置是**可空态友好**的读路径（前端有默认值
            // 兜底），这里响亮记一笔后回落 `Default` —— 与 `commands/session.rs` 的
            // `get_all_sessions` 同款取舍。
            ::log::error!("用量设置读取任务异常: {e}");
            Default::default()
        })
}

/// 写用量设置。
///
/// **A-5（P2-3）**：同 `usage_get_settings` —— `merge_patch` + `save`（写后回读）整段都在
/// `spawn_blocking` 里，主线程只 await。错误语义不变：补丁越界 →
/// `usage-settings-invalid`、写失败 → `usage-db-failed`（裁决 F）。
#[tauri::command]
pub async fn usage_set_settings(patch: UsageSettingsPatch) -> Result<UsageSettings, UsageError> {
    tauri::async_runtime::spawn_blocking(move || {
        let merged = settings::merge_patch(&settings::load(), &patch)?;
        // 裁决 F：写失败**不得**报成成功（DAO 的 `let _ =` 会吞掉 execute 错误 →
        // `settings::save` 做写后回读，不一致即以 `usage-db-failed` 报错）。
        settings::save(&merged)?;
        Ok(merged)
    })
    .await
    .map_err(|e| UsageError::internal(format!("设置写入任务异常: {e}")))?
}

#[cfg(test)]
mod tests {
    // **D-01**：`use super::*;` 拿不到父模块那条 `use crate::services::usage::model::*;`
    // 的 glob 再导出（glob 不传递 glob，实测 E0433/E0422 一片），故显式导入所需值对象。
    use crate::services::usage::model::{
        CompareBlock, LongestTurn, RecentSessionUsage, SourceKind, TopTool, TopToolMs, TrendPoint,
        UsageAvailability, UsageBuckets, UsageCard, UsageDashboard, UsageGroupBy, UsageMetric,
        UsageMetrics, UsageRange, UsageRangePreset, UsageRecords, UsageRow, WorkSummary,
    };

    /// 本文件源码（自省锁的输入）
    const SELF_SRC: &str = include_str!("usage.rs");
    /// 命令注册表所在文件（`generate_handler!` 与 `setup` 闭包）
    const LIB_SRC: &str = include_str!("../lib.rs");

    /// 去掉注释行与全部空白后的「代码骨架」：自省锁只看代码、不看注释——
    /// 否则把文档注释里的签名抄一遍就能骗过签名锁（假绿）。
    /// 顺带把参数表尾逗号 `,)` 归一成 `)`，于是「一行写」与「多行写」等价（fmt 不制造假红）。
    fn code_only(src: &str) -> String {
        src.lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n")
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>()
            .replace(",)", ")")
    }

    /// 本文件的**生产半边**（`#[cfg(test)] mod tests` 之前）。
    /// 自省锁必须只看生产代码，否则断言里写的字面量（`"#[tauri::command]"`、`"DB.lock("`……）
    /// 会被自己数进去 → 反向锁恒红 / 正向锁恒绿（两种假性）。
    fn prod_only(src: &str) -> String {
        let marker = concat!("#[cfg", "(test)]");
        match src.find(marker) {
            Some(i) => src[..i].to_string(),
            None => src.to_string(),
        }
    }

    /// 契约 §3 的 6 条命令签名（**逐字冻结**）。参数名即 Tauri 的 JS 侧键名
    /// （默认 camelCase 映射）：`group_by` → `groupBy`、`patch` → `patch`……改一个字母，
    /// 前端 `invoke` 就会静默 reject（只有 mock 会掩盖它）。
    ///
    /// **A-5 追记**：末两行由 `pub fn` 改为 `pub async fn`（P2-3：同步命令在主线程等
    /// `DB.lock()`）。契约 §3 冻结的是**命令名 + 入参 + 返回形状**（不含 sync/async），
    /// ⇒ 本改动**不违约**，属**计划级冻结签名的显式偏离**（plan:11659-11660），
    /// 已在 FINAL-FIX-report 的「偏离申报」点名；前端键名/参数/返回一字未动。
    const FROZEN_SIGNATURES: [&str; 6] = [
        "pub async fn usage_collect(force: bool) -> Result<UsageCollectResult, UsageError>",
        "pub async fn usage_dashboard(range: UsageRange, group_by: UsageGroupBy) -> Result<UsageDashboard, UsageError>",
        "pub async fn usage_records(range: UsageRange, group_by: UsageGroupBy, filters: UsageFilters) -> Result<UsageRecords, UsageError>",
        "pub async fn usage_export_csv(range: UsageRange, group_by: UsageGroupBy, filters: UsageFilters) -> Result<String, UsageError>",
        "pub async fn usage_get_settings() -> UsageSettings",
        "pub async fn usage_set_settings(patch: UsageSettingsPatch) -> Result<UsageSettings, UsageError>",
    ];

    const COMMAND_NAMES: [&str; 6] = [
        "usage_collect",
        "usage_dashboard",
        "usage_records",
        "usage_export_csv",
        "usage_get_settings",
        "usage_set_settings",
    ];

    /// 契约 §3 的 6 条命令**逐字冻结**：少一条 / 改名 / 改参数名与类型 → 本用例真红。
    /// （Tauri 命令名 = 函数名；参数名 = JS 侧 camelCase 键名。两者都不会有编译错，
    /// 只会在运行时静默 reject —— 所以必须用自省锁钉住。）
    #[test]
    fn six_commands_keep_frozen_signatures() {
        let code = code_only(&prod_only(SELF_SRC));
        // 按**前缀**计数（`#[tauri::command` 而非 `#[tauri::command]`）：这样
        // `#[tauri::command(rename_all = …)]` 仍算「一条命令」，于是下面的 rename_all 反锁
        // 才是**独立承重**的第二道（否则计数断言先红，反锁形同虚设、永远到不了）。
        assert_eq!(
            code.matches("#[tauri::command").count(),
            6,
            "用量域必须恰好 6 条 `#[tauri::command]`（契约 §3）"
        );
        for sig in FROZEN_SIGNATURES {
            let want = code_only(sig);
            assert!(
                code.contains(&want),
                "契约 §3 的命令签名被改动或缺失：`{sig}`\n（参数名 = 前端 invoke 的键名，改它前端会静默 reject）"
            );
        }
        for name in COMMAND_NAMES {
            let pat = format!("fn{name}(");
            assert_eq!(
                code.matches(&pat).count(),
                1,
                "命令 `{name}` 必须恰好声明一次（契约 §3）"
            );
        }
        // **Minor 5**：签名锁挡不住 `#[tauri::command(rename_all = "snake_case")]`——
        // 加上它签名一字不变，而前端 `groupBy` / `filters` / `patch` 会**静默 reject**
        // （mock 仍然全绿，正是最阴的一类）。契约 §1 明文：命令名 snake_case、
        // 前端 invoke 用同名；**参数一律走 Tauri 默认的 camelCase 映射**，故此处禁掉 rename_all。
        assert!(
            !code.contains("rename_all"),
            "6 条命令不得使用 `rename_all`：前端按默认 camelCase 传 groupBy/filters/patch，改了就是静默 reject（契约 §1）"
        );
    }

    /// **A-5 结构锁（P2-3）**：两条设置命令必须是 **`pub async fn` + `spawn_blocking`**
    /// —— 同步命令在**主线程**上执行，会等全局 `DB.lock()`（最坏 0.5s UI 冻结）；
    /// 仓内 `commands/session.rs:10-12` 正是踩过这个坑才把轮询命令改成 async。
    ///
    /// **JS 侧签名与返回形状不变**（本次只动 Rust 侧），契约 §3 冻结的是**命令名 + 入参 + 返回**
    /// —— 不含 sync/async ⇒ 本改动**不违约**，属**计划级冻结签名的显式偏离**（plan:11659-11660），
    /// 已在 FINAL-FIX-report 的「偏离申报」里点名。
    ///
    /// 断言方式（沿用本文件既有自省锁模式）：
    /// ① 两条命令的声明必须是 `pubasyncfn<name>(`（`code_only` 去掉全部空白）；
    /// ② 函数体内必须出现 `spawn_blocking`（阻塞工作已移出主线程）；
    /// ③ **`spawn_blocking` 之前不得出现 `settings::`** —— 否则阻塞调用仍在主线程上跑，
    ///    ② 就成了装样子（"把 await 写在后面"骗不过这一条）；
    /// ④ `settings::load` / `settings::save` 必须都在函数体里（别为了过锁把逻辑删了）。
    ///
    /// **能变红**：把命令改回同步（或把 `settings::load()` 提到 `spawn_blocking` 之外）→ 红。
    #[test]
    fn settings_commands_are_async_and_do_not_block_the_main_thread() {
        let code = code_only(&prod_only(SELF_SRC));
        for name in ["usage_get_settings", "usage_set_settings"] {
            let sig = format!("pubasyncfn{name}(");
            assert!(
                code.contains(&sig),
                "`{name}` 必须是 `pub async fn`（同步命令在主线程等 DB 锁 → UI 冻结，P2-3）"
            );
            // 取函数体：从签名起到下一个 `#[tauri::command]`（或文件末尾）
            let start = code.find(&sig).expect("签名在场（上一条已断言）");
            let rest = &code[start..];
            let end = rest.find("#[tauri::command").unwrap_or(rest.len());
            let body = &rest[..end];
            let Some(off) = body.find("spawn_blocking") else {
                panic!("`{name}` 必须把阻塞工作放进 `spawn_blocking`（否则仍在主线程上跑）");
            };
            assert!(
                !body[..off].contains("settings::"),
                "`{name}` 在 `spawn_blocking` **之前**就碰了 `settings::` —— \
                 阻塞调用仍在主线程上执行（把 load/save 移进闭包）"
            );
            assert!(
                body[off..].contains("settings::"),
                "`{name}` 的 `spawn_blocking` 闭包里必须真的做设置读写（别为了过锁掏空逻辑）"
            );
        }
    }

    /// 三处登记之一：`lib.rs` 的 `generate_handler!` 必须逐条登记 6 条命令**各一次**。
    /// 漏登记 = 前端 `invoke` 静默 reject（mock 会掩盖），不是编译错。
    #[test]
    fn six_commands_are_registered_in_generate_handler_once() {
        let lib = code_only(LIB_SRC);
        let handler_start = lib
            .find("tauri::generate_handler![")
            .expect("lib.rs 必须存在 generate_handler! 块");
        let handler = &lib[handler_start..];
        for name in COMMAND_NAMES {
            let pat = format!("commands::usage::{name},");
            assert_eq!(
                handler.matches(&pat).count(),
                1,
                "`generate_handler!` 必须登记 `{pat}`（契约 §1：三处登记）"
            );
        }
        // 反向：不得把用量命令登记到别处（例如误放进 setup 闭包）
        assert_eq!(
            lib.matches("commands::usage::").count(),
            6,
            "lib.rs 全文件只应有 6 处 `commands::usage::` 引用（逐条登记、无重复）"
        );
    }

    /// GC 3 / AGENTS.md：启动采集是**延迟 3s 的后台线程**（`spawn_initial_collection`），
    /// 且 `lib.rs` 里**不得**出现任何直接采集调用（`collect()` / `run_collection()`）——
    /// 那会把采集塞进启动路径，或（更糟）被误接进 3 秒轮询。
    #[test]
    fn initial_collection_is_the_only_launch_time_entry() {
        let lib = code_only(LIB_SRC);
        assert_eq!(
            lib.matches("crate::services::usage::collect::spawn_initial_collection();")
                .count(),
            1,
            "setup 闭包必须恰好调用一次 spawn_initial_collection()（延迟入口，GC 3）"
        );
        for forbidden in ["collect::collect(", "run_collection(", "collect_with("] {
            assert!(
                !lib.contains(forbidden),
                "lib.rs 不得出现 `{forbidden}`：启动路径只能走 spawn_initial_collection 的延迟线程（GC 3）"
            );
        }
        // 位置锚定：接在既有 remote 恢复之后（计划① Step 4 的落点）
        let restore = lib
            .find("crate::remote::restore_on_launch();")
            .expect("lib.rs setup 里应有 restore_on_launch()");
        let spawn = lib
            .find("crate::services::usage::collect::spawn_initial_collection();")
            .expect("已断言存在");
        assert!(
            spawn > restore,
            "spawn_initial_collection() 必须接在 restore_on_launch() 之后（计划① Step 4 的落点）"
        );
    }

    /// W-26：命令层**不得**自己取 `DB` 锁、也不得调采集内核；采集入口只允许一处。
    /// （取 `DB` 锁后再调采集 = ABBA 死锁；`std::sync::Mutex` 不可重入，整个 app 的 DB 访问挂死。）
    #[test]
    fn command_layer_never_locks_db_or_calls_collection_entries() {
        let code = code_only(&prod_only(SELF_SRC));
        for forbidden in [
            "DB.lock(",
            "cached_result(",
            "run_collection(",
            "collect_with(",
            "load_ledger_rows(",
        ] {
            assert!(
                !code.contains(forbidden),
                "命令层不得出现 `{forbidden}`（W-26：查询层负责持锁，采集入口只能在不持锁时调）"
            );
        }
        assert_eq!(
            code.matches("collect::collect(").count(),
            1,
            "`collect::collect(` 只允许出现在 `usage_collect` 一处（单飞 + 最小间隔的唯一入口）"
        );
    }

    /// 三处登记之二/之三的一致性：前端码表 `KNOWN_USAGE_CODES`
    /// 必须与 Rust 的 `USAGE_CODES` **同序同集合**——漏一个码，前端会把它收敛成
    /// `usage-internal`（静默显示成通用错误，用户看到的 detail 也对不上）。
    #[test]
    fn usage_codes_match_frontend_table() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../src/components/usage/usageErrors.ts");
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("无法读取 {}: {}", path.display(), e));
        let start = text
            .find("KNOWN_USAGE_CODES = [")
            .expect("usageErrors.ts 必须声明 KNOWN_USAGE_CODES 数组");
        let tail = &text[start..];
        let end = tail
            .find("] as const;")
            .expect("KNOWN_USAGE_CODES 数组必须以 `] as const;` 收尾");
        let body = &tail[..end];
        let codes: Vec<String> = body
            .split('"')
            .skip(1)
            .step_by(2)
            .map(|s| s.to_string())
            .collect();
        assert!(
            !codes.is_empty(),
            "KNOWN_USAGE_CODES 必须非空（解析失败就是空表）"
        );
        assert_eq!(
            codes,
            crate::services::usage::error::USAGE_CODES,
            "前端码表 KNOWN_USAGE_CODES 必须与 Rust 的 USAGE_CODES 同序同集合"
        );
    }

    /// 契约 §2 的前端镜像：**可空性**是这里最容易漂的一处
    /// （`| null` 写成 `?` → 前端把「不可得」当缺键，空态分支与 GC 7 一起失效）。
    #[test]
    fn frontend_type_mirror_keeps_nullability() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../src/types/usage.ts");
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("无法读取 {}: {}", path.display(), e));
        let flat = code_only(&text);
        for want in [
            "compare:CompareBlock|null;",
            "recentSession:RecentSessionUsage|null;",
            "userEst:number|null;",
            "title:string|null;",
            "sessions:number|null;",
            "errorCode?:string;",
            "from?:string;",
            "to?:string;",
            "subagentMode?:\"include\"|\"parentsOnly\";",
            "exporttypeUsageAvailabilityMetric=UsageMetric;",
        ] {
            assert!(
                flat.contains(&code_only(want)),
                "src/types/usage.ts 缺少契约字段/可空性：`{want}`"
            );
        }
    }

    /// 递归收集 `src/` 下的 `.ts` / `.tsx` 源文件（`(相对路径, 全文)`）。
    /// 第一版这把锁**瞄错了文件**（只扫 `hooks/useSessions.ts` + `stores/sessionStore.ts`，
    /// 而它们只同步/存状态、**不 invoke**）→ 评审 Important #2：往**真** poll 点插一行
    /// `invoke("usage_collect")` 时三条锁全绿，而那正是 GC 3 唯一会成真的「每 3 秒全量扫描」路径。
    /// 现在改成**按内容发现**：凡含 `refetchInterval` / `POLL_INTERVAL` 的文件一律入清单。
    fn collect_src_files(
        dir: &std::path::Path,
        root: &std::path::Path,
        out: &mut Vec<(String, String)>,
    ) {
        let entries = std::fs::read_dir(dir)
            .unwrap_or_else(|e| panic!("无法读取目录 {}: {}", dir.display(), e));
        for entry in entries {
            let path = entry.expect("目录项读取失败").path();
            if path.is_dir() {
                collect_src_files(&path, root, out);
                continue;
            }
            if !matches!(
                path.extension().and_then(|x| x.to_str()),
                Some("ts") | Some("tsx")
            ) {
                continue;
            }
            let text = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("无法读取 {}: {}", path.display(), e));
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            out.push((rel, text));
        }
    }

    /// W-22 的**静态半边**：3 秒轮询热路径不得引用用量命令。行为式用例在
    /// `tests/usage_polling_budget_test.rs`（真跑扫描后 `collect_calls()` 仍为 0）。
    ///
    /// 清单口径（评审 Important #2 的修法）：**所有**含 `refetchInterval` / `POLL_INTERVAL`
    /// 的前端文件（真 poll 点是 `lib/query/queries/sessions.ts` → `lib/api/session.ts` 的
    /// `invoke("get_all_sessions")`）+ 三个状态侧文件。含**前提断言**：若清单没命中真 poll 点
    /// 或递归扫描读不到源码，本用例自己先红（防「瞄错文件仍然全绿」）。
    #[test]
    fn polling_hot_path_never_references_usage_collect() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../src");
        let mut files: Vec<(String, String)> = Vec::new();
        collect_src_files(&root, &root, &mut files);
        assert!(
            files.len() > 50,
            "前提：递归扫描必须真的读到前端源码（读到 {} 个文件）",
            files.len()
        );

        // 前提 1：内容发现必须命中真 poll 点
        let poll_files: Vec<&(String, String)> = files
            .iter()
            .filter(|(_, t)| t.contains("refetchInterval") || t.contains("POLL_INTERVAL"))
            .collect();
        assert!(
            poll_files
                .iter()
                .any(|(p, _)| p == "lib/query/queries/sessions.ts"),
            "前提：轮询点清单必须包含 src/lib/query/queries/sessions.ts（含 refetchInterval），实际 {:?}",
            poll_files.iter().map(|(p, _)| p.clone()).collect::<Vec<_>>()
        );
        // 前提 2：必须命中真正 invoke("get_all_sessions") 的文件
        assert!(
            files
                .iter()
                .any(|(p, t)| p == "lib/api/session.ts" && t.contains("get_all_sessions")),
            "前提：必须命中 invoke(\"get_all_sessions\") 的真实调用点 src/lib/api/session.ts"
        );

        // 反向：热路径清单（真 poll 点 + 状态侧三文件）一律不得引用用量命令
        let extra = [
            "lib/api/session.ts",
            "hooks/useSessions.ts",
            "stores/sessionStore.ts",
        ];
        let mut checked: Vec<&str> = Vec::new();
        for (p, text) in files.iter().filter(|(p, t)| {
            t.contains("refetchInterval")
                || t.contains("POLL_INTERVAL")
                || extra.contains(&p.as_str())
        }) {
            for forbidden in [
                "usage_collect",
                "usage_dashboard",
                "usage_records",
                "usage_export_csv",
            ] {
                assert!(
                    !text.contains(forbidden),
                    "{p} 属于 3 秒轮询热路径，不得引用用量命令 `{forbidden}`（GC 3 / AGENTS.md / W-22）"
                );
            }
            checked.push(p);
        }
        assert!(
            checked.contains(&"lib/query/queries/sessions.ts"),
            "前提：真 poll 点必须被反向检查覆盖（实际 {:?}）",
            checked
        );
    }

    /// **裁决 D**：嵌套值对象的 wire 形状在 Rust 侧必须**有锁**（此前只有顶层键被钉住，
    /// 而空账本让 `rows` / `trend` / `cards` 为空、`compare` / `recentSession` 为 null →
    /// `UsageRow` / `UsageMetrics` / `UsageBuckets` / `UsageAvailability` / `TrendPoint` /
    /// `UsageCard` / `CompareBlock` / `RecentSessionUsage` 的键名**从未在真实序列化输出上被断言**）。
    ///
    /// 为什么要紧：给 `UsageRow` 去掉 `rename_all` 或改任一嵌套字段名 → **Rust 全绿 + TS 全绿**，
    /// 而真机 wire 与 mock/TS **静默分叉**（`isSubagent` 假值、`userEst` 由 `null` 变**缺键**
    /// → GC 7 在 UI 上失效）。这里用**构造值**（零 DB、零落库）直接断 serde 输出。
    #[test]
    fn nested_value_objects_keep_camel_case_wire_keys() {
        fn keys(v: &serde_json::Value) -> Vec<String> {
            let mut k: Vec<String> = v
                .as_object()
                .expect("必须是 JSON 对象")
                .keys()
                .cloned()
                .collect();
            k.sort();
            k
        }
        fn obj<T: serde::Serialize>(v: T) -> serde_json::Value {
            serde_json::to_value(v).expect("值对象必须可序列化")
        }

        let buckets = UsageBuckets {
            input_fresh: 1,
            cache_read: 2,
            cache_write: 3,
            output: 4,
        };
        assert_eq!(
            keys(&obj(buckets)),
            vec!["cacheRead", "cacheWrite", "inputFresh", "output"]
        );

        let metrics = UsageMetrics {
            request_total: 10,
            cache_hit_rate: 0.5,
            user_est: None, // **不可得 = null 且必须出现**（GC 7）
            requests: 2,
        };
        let mv = obj(metrics);
        assert_eq!(
            keys(&mv),
            vec!["cacheHitRate", "requestTotal", "requests", "userEst"]
        );
        assert!(
            mv.get("userEst").is_some(),
            "userEst 必须出现（缺键 = 前端 undefined）"
        );
        assert!(
            mv["userEst"].is_null(),
            "userEst 不可得必须是 null（GC 7，不得填 0/缺键）"
        );

        let row = UsageRow {
            key: "codex".into(),
            label: "codex".into(),
            buckets,
            metrics,
            source_kind: SourceKind::Inferred,
            // **算不出**该位（日档 / 记录页卡内行）→ `None`：wire 上必须是显式 `null`
            is_subagent: None,
        };
        let rv = obj(row);
        assert_eq!(
            keys(&rv),
            vec![
                "buckets",
                "isSubagent",
                "key",
                "label",
                "metrics",
                "sourceKind"
            ],
            "UsageRow 的键名/大小写逐字冻结（去掉 rename_all 会全变 snake_case）"
        );
        assert_eq!(rv["sourceKind"], "inferred", "三态序列化形态");
        // —— 新口径锁（契约 §2 2026-10-03 用户裁决：`T | null` 必须序列化出显式 null）——
        // 变异敏感性：构造处改回 `Some(false)` 或加上 `skip_serializing_if` → 下面的断言真红。
        assert!(
            rv.get("isSubagent").is_some(),
            "isSubagent 必须出现（缺键 = 前端 undefined）"
        );
        assert!(
            rv["isSubagent"].is_null(),
            "不可得必须序列化成 null（GC 7 同族；不得 skip、不得填 false）"
        );
        // 反向对照（同一用例内）：`Some(true)` 在 wire 上是**真布尔**，不得被可空化牵连成 null
        let row_true = UsageRow {
            key: "codex".into(),
            label: "codex".into(),
            buckets,
            metrics,
            source_kind: SourceKind::Inferred,
            is_subagent: Some(true),
        };
        assert_eq!(
            obj(row_true)["isSubagent"],
            serde_json::json!(true),
            "Some(true) 必须是真布尔 true（`Option` 只在 None 时出 null）"
        );

        let avail = UsageAvailability {
            metric: UsageMetric::UserEst,
            available: false,
            reason: None,
            per_source: None,
        };
        let av = obj(avail);
        assert_eq!(keys(&av), vec!["available", "metric"], "`?:` 缺省不出现");
        assert_eq!(av["metric"], "userEst");
        let avail_full = UsageAvailability {
            metric: UsageMetric::ErrorModel,
            available: true,
            reason: Some("无判据".into()),
            per_source: Some(std::collections::BTreeMap::from([(
                "kimi".to_string(),
                false,
            )])),
        };
        assert_eq!(
            keys(&obj(avail_full)),
            vec!["available", "metric", "perSource", "reason"]
        );

        let trend = TrendPoint {
            key: "2026-10-03T09".into(),
            label: "09:00".into(),
            buckets,
            metrics,
        };
        assert_eq!(
            keys(&obj(trend)),
            vec!["buckets", "key", "label", "metrics"]
        );

        let compare = CompareBlock {
            prev_buckets: buckets,
            prev_metrics: metrics,
        };
        let cv = obj(compare);
        assert_eq!(
            keys(&cv),
            vec!["prevBuckets", "prevMetrics"],
            "totalsBuckets/prevBuckets 同族"
        );

        let recent = RecentSessionUsage {
            source_id: "claude".into(),
            session_id: "s1".into(),
            title: None, // 契约 `title: string | null` → 必须出现
            buckets,
            metrics,
        };
        let rcv = obj(recent);
        assert_eq!(
            keys(&rcv),
            vec!["buckets", "metrics", "sessionId", "sourceId", "title"]
        );
        assert!(rcv["title"].is_null(), "title 可空但必须出现（GC 7）");

        let card = UsageCard {
            tool_id: "claude".into(),
            tool_label: "claude".into(),
            buckets,
            metrics,
            rows: vec![],
        };
        assert_eq!(
            keys(&obj(card)),
            vec!["buckets", "metrics", "rows", "toolId", "toolLabel"]
        );

        // WorkSummary：契约「全字段必须出现；不可得 = null」
        let ws = obj(WorkSummary::default());
        assert_eq!(
            keys(&ws),
            vec![
                "errorModel",
                "errorTool",
                "errorTurn",
                "interrupted",
                "longestTurnPerTool",
                "sessions",
                "toolAvgMs",
                "toolCalls",
                "topTool",
                "topToolMs",
                "turnsPerTool",
            ]
        );
        for k in [
            "sessions",
            "errorModel",
            "errorTurn",
            "errorTool",
            "interrupted",
            "toolCalls",
            "toolAvgMs",
            "topTool",
            "topToolMs",
        ] {
            assert!(ws[k].is_null(), "WorkSummary.{k} 不可得必须是 null（GC 7）");
        }
        let ws_full = obj(WorkSummary {
            sessions: Some(3),
            turns_per_tool: std::collections::BTreeMap::from([("claude".to_string(), Some(2))]),
            error_model: Some(1),
            error_turn: Some(1),
            error_tool: Some(1),
            interrupted: Some(1),
            tool_calls: Some(1),
            tool_avg_ms: Some(1),
            top_tool: Some(TopTool {
                name: "Bash".into(),
                count: 1,
            }),
            top_tool_ms: Some(TopToolMs {
                name: "Bash".into(),
                ms: 1,
            }),
            longest_turn_per_tool: std::collections::BTreeMap::from([(
                "claude".to_string(),
                Some(LongestTurn { p50: 1, max: 2 }),
            )]),
        });
        assert_eq!(keys(&ws_full["topTool"]), vec!["count", "name"]);
        assert_eq!(keys(&ws_full["topToolMs"]), vec!["ms", "name"]);
        assert_eq!(
            keys(&ws_full["longestTurnPerTool"]["claude"]),
            vec!["max", "p50"]
        );

        // 顶层容器里的 camelCase 容器键（dashboard 的 totalsBuckets / records 的 cards）
        let d = UsageDashboard {
            range: UsageRange {
                preset: UsageRangePreset::Today,
                from: None,
                to: None,
            },
            group_by: UsageGroupBy::Tool,
            rows: vec![],
            totals: metrics,
            totals_buckets: buckets,
            hero: 0,
            trend: vec![],
            compare: None,
            recent_session: None,
            work_summary: WorkSummary::default(),
            availability: vec![],
            collected_at: 0,
        };
        let dv = obj(d);
        assert_eq!(
            keys(&dv["totalsBuckets"]),
            vec!["cacheRead", "cacheWrite", "inputFresh", "output"]
        );
        assert!(
            dv["compare"].is_null() && dv["recentSession"].is_null(),
            "契约 `| null` 必须出现"
        );
        let rec = UsageRecords {
            range: UsageRange {
                preset: UsageRangePreset::Today,
                from: None,
                to: None,
            },
            group_by: UsageGroupBy::Tool,
            cards: vec![],
            availability: vec![],
            collected_at: 0,
        };
        assert!(keys(&obj(rec)).contains(&"cards".to_string()));
    }
}
