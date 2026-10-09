use log::debug;
use once_cell::sync::Lazy;
use rusqlite::Connection;
use std::sync::Mutex;

/// 应用数据主目录：TUVIS_HOME 环境变量仅在 debug/test 构建（debug_assertions）生效，
/// 用于集成测试重定向数据目录（Windows 下 dirs::home_dir 无法用 HOME 重定向）；
/// release 生产构建一律使用真实用户目录，防止环境变量误设导致 DB 与 skills/plugins 数据割裂
fn app_data_home() -> std::path::PathBuf {
    // release 构建里 `TUVIS_HOME` 根本不参与解析 ⇒ 连环境变量都不读（与改动前逐字同义）。
    // `cfg!` 是编译期常量，本分支在 release 会被整个折掉。
    let mam_home = if cfg!(debug_assertions) {
        std::env::var_os("TUVIS_HOME")
    } else {
        None
    };
    app_data_home_with(mam_home)
}

/// `app_data_home()` 的**纯函数内核**（Task 16 Step 2）：把「env 取值」与「路径解析」
/// 分开，单测才能对「`TUVIS_HOME` **未设置**」这一支**直接**断言——不必改进程环境
/// （`std::env::set_var` 是进程级共享状态，在并行测试里会与别的用例互踩，改它比改代码更危险）。
/// 生产入口恒为 `app_data_home()`；本函数不额外暴露任何行为。
fn app_data_home_with(mam_home: Option<std::ffi::OsString>) -> std::path::PathBuf {
    if cfg!(debug_assertions) {
        if let Some(home) = mam_home {
            if !home.is_empty() {
                return std::path::PathBuf::from(home);
            }
        }
        // **Task 16 Step 2（默认 hermetic）**：lib 单测是**独立 crate**、不经
        // `tests/support.rs::setup()` 的环境重定向 ⇒ 没有本分支时，首次取 `DB` 就把进程级
        // 连接绑到用户**真实** `~/.tuvis/tuvis.db`（2026-10-03 00:11 事故的**账本**半边；
        // 当时 `TUVIS_HOME=$(mktemp -d)` 只保住了账本，**读源**那一半没保住 → 见 `collect.rs`
        // 的采集拒绝闸）。`TUVIS_HOME` 未设置时落到**进程私有** temp 目录。
        //
        // 形态纪律（照抄既有成对范式，不自创第二套）：仍在 `cfg!(debug_assertions)`
        // 门控**之内**（否则 release 会被重定向）；**不得**实现成「未设置就 panic」——
        // 那会打断全仓每一个（哪怕只是间接）依赖真实 home 的既有 lib 单测，爆炸半径不可控。
        // 集成测试（`src-tauri/tests/*.rs`）链接的是**非 test** 构建 ⇒ 本分支在那边不存在，
        // 它们照旧走 `setup()` 的 `TUVIS_HOME` 重定向。
        #[cfg(test)]
        return std::env::temp_dir().join(format!("mam-test-{}", std::process::id()));
    }
    dirs::home_dir().unwrap_or_default()
}

/// 全局数据库连接（从 store.rs 搬移，保持原有模式）
pub static DB: Lazy<Mutex<Connection>> = Lazy::new(|| {
    let db_dir = app_data_home().join(".tuvis");
    let _ = std::fs::create_dir_all(&db_dir);
    let db_path = db_dir.join("tuvis.db");
    let conn = Connection::open(&db_path).expect("Failed to open mam database");
    crate::database::schema::init(&conn);
    Mutex::new(conn)
});

/// 初始化数据库（在应用启动时调用）
pub fn init() {
    Lazy::force(&DB);
    debug!("Database initialized at ~/.tuvis/tuvis.db");
}

/// 打开新连接（少数场景使用）
pub fn open() -> Result<Connection, String> {
    let db_path = app_data_home().join(".tuvis").join("tuvis.db");
    Connection::open(&db_path).map_err(|e| format!("打开数据库失败: {}", e))
}

/// **Task 16 Step 2 的锁（锁表第 1 条「路径隔离」）**：`cfg(test)` 下解析出的 app 数据
/// 目录**不在真实 home 之下**。
///
/// **为什么必须落在 lib 单测里**（而不是 `src-tauri/tests/`）：本用例断言的是
/// `#[cfg(test)]` 分支的行为，而集成测试链接的是**非 test** 构建 —— 那边根本没有这个分支，
/// 断言什么都是假绿（`#[cfg(test)]` 只在 lib 单测里为真）。
#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    /// 未设置 / 空串 `TUVIS_HOME` → **进程私有 temp 目录**，而不是真实 home。
    /// 这是 2026-10-03 事故的账本半边的封堵点：lib 单测不再可能把进程级 `DB` 绑到
    /// 用户真实 `~/.tuvis/tuvis.db`。
    #[test]
    fn tests_never_resolve_the_real_users_home_when_mam_home_is_unset() {
        let real_home = dirs::home_dir().expect("真实 home 必须可解析（本用例的对照物）");
        let expected = std::env::temp_dir().join(format!("mam-test-{}", std::process::id()));

        for (case, input) in [
            ("未设置", None),
            (
                "空串（与既有 `!home.is_empty()` 语义同义）",
                Some(OsString::from("")),
            ),
        ] {
            let resolved = app_data_home_with(input);
            assert_eq!(
                resolved,
                expected,
                "`TUVIS_HOME` {case} 时必须落到进程私有 temp 目录 <{}>",
                expected.display()
            );
            assert!(
                !resolved.starts_with(&real_home),
                "**路径隔离锁**：`TUVIS_HOME` {case} 时解析出的 app 数据目录 <{}> 不得落在真实 \
                 home <{}> 之下——落在那里 = 进程级 DB 又绑回用户真实账本（2026-10-03 事故）",
                resolved.display(),
                real_home.display()
            );
        }
    }

    /// 既有的 `TUVIS_HOME` 重定向语义**不得被踩掉**：非空值仍然优先（集成测试的
    /// `setup()` 与用例私有库都靠它）。
    #[test]
    fn explicit_mam_home_still_wins() {
        assert_eq!(
            app_data_home_with(Some(OsString::from("/tmp/mam-explicit"))),
            std::path::PathBuf::from("/tmp/mam-explicit"),
            "非空 `TUVIS_HOME` 必须仍然优先于新的 cfg(test) 兜底分支"
        );
    }

    /// 生产入口 `app_data_home()` 在本进程（= lib 单测构建）里也必须满足隔离性质。
    /// 与上面那条的分工：那条钉**分支语义**，这条钉**真实入口**（防有人把兜底分支删了
    /// 却保留纯函数内核 ⇒ 上一条仍绿而实际入口已回到真实 home）。
    #[test]
    fn real_entry_point_is_isolated_in_this_process() {
        let resolved = app_data_home();
        let real_home = dirs::home_dir().expect("真实 home 必须可解析");
        assert!(
            !resolved.starts_with(&real_home),
            "生产入口 `app_data_home()` 在 lib 单测进程里解析出 <{}>（真实 home = <{}>）：\
             隔离分支被删了或没接上（本用例是「分支存在」与「入口真的走它」之间的那道桥）",
            resolved.display(),
            real_home.display()
        );
    }
}
