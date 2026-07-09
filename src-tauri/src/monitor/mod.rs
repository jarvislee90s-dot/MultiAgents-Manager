pub mod process;
pub mod parser;
pub mod status;
pub mod opencode_parser;
pub mod openclaw_parser;
pub mod hooks;


// ===== notify 文件监听集成（FR-5c）=====

use notify::{Watcher, RecursiveMode, EventKind, RecommendedWatcher};
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
                Ok(Ok(event)) if matches!(event.kind, EventKind::Create(_) | EventKind::Modify(_)) => {
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
