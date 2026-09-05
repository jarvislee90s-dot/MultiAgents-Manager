// 设置、工具检测、子 Agent 命令

use crate::database::SubAgentRecord;
use tauri::Emitter;

#[tauri::command]
pub fn get_setting(key: String) -> Option<String> {
    crate::database::get_setting(&key)
}

#[tauri::command]
pub fn set_setting(key: String, value: String) {
    crate::database::set_setting(&key, &value);
}

#[tauri::command]
pub fn detect_tools() -> Vec<crate::linker::detector::ToolDetection> {
    crate::linker::detector::detect_all_tools()
}

#[tauri::command]
pub fn detect_subagents(tool_id: String) -> Vec<String> {
    crate::services::detect_subagents(&tool_id)
}

#[tauri::command]
pub fn list_sub_agents(tool_id: String) -> Vec<SubAgentRecord> {
    crate::database::list_sub_agents(&tool_id)
}

/// 手动关闭未读卡（X 按钮）→ 标记已读（删除未读行）。
/// T1：删行后广播 session-read——看板/宠物是独立 WebView，宠物窗口凭此事件
/// 同步已读置位（否则宠物头顶卡在别处已读后仍滞留，与看板不一致）
#[tauri::command]
pub fn mark_session_read(app: tauri::AppHandle, agent_type: String, session_id: String) {
    crate::database::dao::unread::delete(&agent_type.to_lowercase(), &session_id);
    // issue #35-1：已读墓碑——缓存失忆（长间隙清缓存 / MAM 重启）后
    // Insert 边沿与补偿据此不再复活已读未读卡
    crate::database::dao::unread::mark_read(&agent_type.to_lowercase(), &session_id);
    let _ = app.emit(
        "session-read",
        serde_json::json!({ "agentType": agent_type, "sessionId": session_id }),
    );
}

/// 工具勾选列表（含 managed 标志：是否存在启用的分配）
#[tauri::command]
pub fn get_tool_settings() -> Vec<crate::services::tool_settings::ToolSetting> {
    crate::services::tool_settings::get_tool_settings()
}

/// 批量保存重入互斥（review-2 Important 1）：异步化后两次调用可真并行，而
/// 还原/回滚路径的 create_link 是「先删后建」（linker/mod.rs），并发方会在
/// 竞态窗口内删掉对方刚还原的真实目录——同一时刻只允许一次批量保存
static APPLY_GUARD: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// 批量保存工具勾选（取消勾选清理 / 重新勾选重建，返回明细）
#[tauri::command]
pub async fn update_tool_settings(
    app: tauri::AppHandle,
    changes: Vec<crate::services::tool_settings::ToolSettingChange>,
) -> Result<crate::services::tool_settings::ApplyResult, String> {
    // issue #36-2：非 async 命令在主线程执行，apply 内含 copy_dir_recursive 等大目录
    // 文件操作，保存期间会冻结全部窗口的 IPC——移到运行时阻塞线程池执行（Tauri 官方
    // 对重活的建议），主线程与 async worker 均不受阻。
    // 守卫在阻塞线程内获取（std MutexGuard 不可跨 await）
    let result = tauri::async_runtime::spawn_blocking(move || {
        // review-3：区分两种 try_lock 失败——WouldBlock 是真重入（拒绝）；
        // Poisoned 是前次持有者 panic 后锁已空闲，而守卫数据是 ()（无可损坏状态），
        // 中毒保护无意义，into_inner 恢复放行重试，否则一次 panic 会让后续保存
        // 永久误报「进行中」直到重启
        let _guard = match APPLY_GUARD.try_lock() {
            Ok(guard) => guard,
            Err(std::sync::TryLockError::Poisoned(p)) => p.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => {
                return Err(
                    "W5_APPLY_IN_PROGRESS: tool settings save already in progress".to_string(),
                )
            }
        };
        Ok::<crate::services::tool_settings::ApplyResult, String>(
            crate::services::tool_settings::apply_tool_changes(changes),
        )
    })
    .await
    .map_err(|e| {
        // JoinError（后台任务 panic）不得伪装成「已应用」：走 Err 让前端报错
        //（review-2 Important 2：空 ApplyResult + 无条件 success toast = 伪成功）。
        // 结构化错误码由前端 i18n 渲染（同 issue #36-3 模式，review-3）
        log::error!("update_tool_settings background task failed: {e}");
        format!("W5_APPLY_TASK_FAILED: {e}")
    })??;
    // N2：跨窗口广播工具勾选变化。设置窗口与主窗口是独立 WebView、各持 QueryClient，
    // 设置页本地的 invalidateQueries 触达不到主窗口——主窗口靠此事件失效缓存
    let _ = app.emit("tools-changed", ());
    Ok(result)
}

/// 前端工具列下发项（资源视图 / 设置声音区的唯一样式来源）
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnabledTool {
    pub id: String,
    pub label: String,
}

/// 前端工具列的唯一下发源（W5：勾选状态驱动，替代三处硬编码 TOOLS）
#[tauri::command]
pub fn list_enabled_tools() -> Vec<EnabledTool> {
    crate::adapter::TOOL_IDS
        .iter()
        .filter(|id| crate::database::dao::agent_tool::get_tool_enabled(id))
        .filter_map(|id| {
            crate::adapter::adapter_by_id(id).map(|a| EnabledTool {
                id: id.to_string(),
                label: a.name().to_string(),
            })
        })
        .collect()
}
