// 监控解析层：每个工具一个解析器模块（新增工具只需新增 *_parser.rs + 本处一行），
// 跨工具公共设施下沉到 cwd/path_codec/git/project/jsonl 五个职责单一的模块
pub mod claude_parser;
pub mod codex_parser;
pub mod cwd;
pub mod git;
pub mod hooks;
pub mod jsonl;
pub mod kimi_parser;
pub mod openclaw_parser;
pub mod opencode_parser;
pub mod path_codec;
pub mod process;
pub mod project;
pub mod status;

// ===== notify 文件监听集成（FR-5c）=====

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
