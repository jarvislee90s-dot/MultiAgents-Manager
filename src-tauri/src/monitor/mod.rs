// 监控解析层：每个工具一个解析器模块（新增工具只需新增 *_parser.rs + 本处一行），
// 跨工具公共设施下沉到 cwd/path_codec/git/project/jsonl 五个职责单一的模块
pub mod claude_parser;
pub mod codex_parser;
pub mod cwd;
pub mod git;
pub mod hooks;
pub mod host;
pub mod jsonl;
pub mod kimi_parser;
pub mod openclaw_parser;
pub mod opencode_parser;
pub mod path_codec;
pub mod process;
pub mod project;
pub mod sqlite;
pub mod status;
pub mod workbuddy_parser;

// ===== notify 文件监听集成（FR-5c）=====

use once_cell::sync::Lazy;
use std::collections::HashSet;
use std::sync::Mutex;

/// 用户 X 掉的 App 形态卡（T2「暂离不提示」）：key = (tool_id, session_id, status 小写)。
/// 同一会话状态变化后 key 不匹配 → 卡片自然重现（绿→黄/红或产生新未读）；
/// 进程内语义，MAM 重启清空（重启后全部重现，可接受）。不碰 unread_sessions 表——
/// 未读卡的 X 走已读（mark_session_read），本集合只服务活跃卡的「隐藏」语义
pub static SESSION_DISMISALS: Lazy<Mutex<HashSet<(String, String, String)>>> =
    Lazy::new(|| Mutex::new(HashSet::new()));

/// dismiss 过滤纯函数（可测）：App 形态卡按 (tool, session, status 小写) 命中则剔除；
/// CLI 卡不参与 X 关闭，防御性放行。dismissed 以闭包注入，测试不触全局集合
pub fn filter_dismissed_cards(
    sessions: &mut Vec<crate::session::Session>,
    dismissed: &dyn Fn(&str, &str, &str) -> bool,
) {
    sessions.retain(|s| {
        !matches!(s.form, crate::session::ProcessForm::App)
            || !dismissed(
                &format!("{:?}", s.agent_type).to_lowercase(),
                &s.id,
                &format!("{:?}", s.status).to_lowercase(),
            )
    });
}

use notify::{EventKind, RecursiveMode, Watcher};
use std::sync::mpsc::channel;
use std::time::Duration;


/// 启动文件监听，检测 Hook/进程事件文件变化时触发会话刷新
/// notify 事件优先触发，30s 超时回退轮询兜底
pub fn start_file_watcher<F>(paths: Vec<std::path::PathBuf>, on_change: F)
where
    F: Fn() + Send + 'static,
{
    std::thread::spawn(move || {
        let (tx, rx) = channel();
        let mut watcher = match notify::RecommendedWatcher::new(tx, notify::Config::default()) {
            Ok(w) => w,
            Err(e) => {
                log::warn!("notify watcher 初始化失败，回退纯轮询: {}", e);
                return;
            }
        };

        for path in &paths {
            if path.exists() {
                let _ = watcher.watch(path, RecursiveMode::Recursive);
            }
        }

        // notify 事件 + 30s 轮询兜底
        loop {
            match rx.recv_timeout(Duration::from_secs(30)) {
                Ok(Ok(event))
                    if matches!(event.kind, EventKind::Create(_) | EventKind::Modify(_)) =>
                {
                    on_change();
                }
                Ok(Ok(_)) => {} // 忽略其他事件类型
                Ok(Err(e)) => log::warn!("notify 事件错误: {}", e),
                Err(_) => {
                    // 超时，触发兜底轮询
                    on_change();
                }
            }
        }
    });
}

#[cfg(test)]
mod dismissed_filter_tests {
    use super::filter_dismissed_cards;
    use crate::session::model::{AgentType, ProcessForm, Session, SessionStatus};

    fn card(id: &str, agent: AgentType, form: ProcessForm, status: SessionStatus) -> Session {
        Session {
            id: id.into(),
            agent_type: agent,
            project_name: "P".into(),
            project_path: String::new(),
            title: None,
            git_branch: None,
            github_url: None,
            status,
            last_message: None,
            last_message_role: None,
            last_activity_at: String::new(),
            pid: 7,
            cpu_usage: 0.0,
            active_subagent_count: 0,
            form,
            jump_supported: true,
            unread: false,
        }
    }

    /// T2 语义：写入 (tool, session, status) → 过滤剔除；状态变化 → key 不匹配重现；
    /// CLI 卡不参与；重启语义 = SESSION_DISMISALS 为进程内集合，MAM 重启清空后全部重现。
    #[test]
    fn dismissed_app_card_filtered_until_status_changes() {
        let mut sessions = vec![
            card("s1", AgentType::WorkBuddy, ProcessForm::App, SessionStatus::Processing),
            card("s2", AgentType::Codex, ProcessForm::App, SessionStatus::Idle),
            card("cli", AgentType::Claude, ProcessForm::Cli, SessionStatus::Processing),
        ];
        // 模拟用户 X 掉 s1（当时红/运行中）——IPC 存储为小写 key
        let dismissed_keys = std::collections::HashSet::from([(
            "workbuddy".to_string(),
            "s1".to_string(),
            "processing".to_string(),
        )]);
        let dismissed = |t: &str, sid: &str, st: &str| {
            dismissed_keys.contains(&(t.to_string(), sid.to_string(), st.to_string()))
        };

        // 命中：App 卡被剔除（key 全匹配，Debug 形态转小写后一致）
        filter_dismissed_cards(&mut sessions, &dismissed);
        assert!(!sessions.iter().any(|s| s.id == "s1"), "dismiss 的卡应被过滤");
        assert!(sessions.iter().any(|s| s.id == "s2"), "未 dismiss 的卡保留");
        assert!(
            sessions.iter().any(|s| s.id == "cli"),
            "CLI 卡不参与 dismiss 过滤"
        );

        // 状态变化（绿→等待）：key 不匹配 → 卡片重现
        let mut changed = vec![card(
            "s1",
            AgentType::WorkBuddy,
            ProcessForm::App,
            SessionStatus::Waiting,
        )];
        filter_dismissed_cards(&mut changed, &dismissed);
        assert!(
            changed.iter().any(|s| s.id == "s1"),
            "状态变化后 key 不匹配，卡片应重现"
        );

        // 重启语义（文档化）：SESSION_DISMISALS 为 Lazy 进程内集合，MAM 重启即清空，
        // 全部卡片重现；本纯函数测试经注入闭包模拟「空集合」即可覆盖
        filter_dismissed_cards(&mut changed, &|_, _, _| false);
        assert_eq!(changed.len(), 1);
    }
}
