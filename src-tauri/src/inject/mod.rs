pub mod approve;
pub mod confirm;
// 通用 N 选项审批对话框屏读解析（批次丙 T5）：纯函数跨平台可测，屏读源在
// windows_console::read_screen_window（仅 Windows 有屏读 → macOS 自然降级二元卡）。
// 丁T3 起同模块承载「对话框在场 = 控制类注入红线」的单点判据
// （`blocks_control_injection` + 屏读探测 `probe_screen_dialog`，§2.7 裁8/9）与
// `RemoteState.dialog_probe` 缝的生产实现。
//
// **守卫覆盖面（丁T3 现状 + 已知缺口登记，勿误读为全覆盖）**：
// - **有守卫**：`remote::api::session_mode_switch`（模式切换的 shift+tab 与斜杠两路
//   ——投递前屏读，在场即 409 `blocked_by_dialog`，零注入零审计）；
// - **无守卫（已知缺口，下批收口点见 `queue` 模块文档的同名小节）**：队列放行
//   （`queue::flush_one` / `try_flush`）与「立即发送」（`remote::api::session_queue_jump`）
//   ——两条都在可输入态投递，而对话框在场时状态同样可能是 Waiting。
pub mod dialog;
pub mod engine;
pub mod families;
// 模式切换内核（批次丙 T6）：统一模式枚举 + 各工具切换机制映射 + 屏读回显解析
pub mod mode;
pub mod normalize;
pub mod question;
pub mod queue;
// R5 一键 resume 窗口（M6R–M9R Task 11）：命令表 + 终端 spawn 核心（spawner 缝）
pub mod resume;
pub mod routing;
#[cfg(windows)]
pub mod windows_console;

/// 实机 E2E 专用测试支撑面（M9R–M9R 批次 Task 12，`tests/m9r_e2e.rs` 唯一消费方）。
/// **非公开 API 承诺**：`doc(hidden)` 不进文档；只 re-export Windows 执行层的
/// spec 感知入口与统计类型（四例 E2E 直调引擎所需的最小面），零新逻辑零转发。
/// 生产代码不得消费本模块——生产注入一律经 `engine::Injector` 缝（`RealInjector`
/// 装配）与旧薄壳（`locate_and_inject` / `locate_and_send_key`）。
#[doc(hidden)]
#[cfg(windows)]
pub mod e2e_support {
    pub use super::windows_console::{inject_key_spec, inject_text_spec, InjectStats};
    /// 丁T5：问答阶段机的屏读能力（E2E 夹具的 `screen_probe` 缝要按 pid 真读一屏）。
    /// 与 [`super::dialog::probe_screen_dialog`] 的区别：后者返回**解析结论**，本项
    /// 返回**逐行原文**——阶段机的各段判据在内核里，缝要给的是能力（理由见
    /// `remote::server::ScreenProbeFn` 的文档）。薄壳而非 re-export：底层
    /// `read_screen_window` 是 `pub(crate)`，re-export 不出 crate（且本模块的
    /// `doc(hidden)` 面不该扩底层可见性）。
    pub fn read_screen_lines(pid: u32) -> Option<Vec<String>> {
        super::windows_console::read_screen_window(pid).ok()
    }
}

/// 桌面端写审计查看（W5 只读入口）：返回最近 limit 条（缺省 100），最新在前。
/// AuditRow serde camelCase 序列化即前端载荷；无 device_id 字段（设备标识不外泄，
/// 展示侧只用 device_name）。st 不需要：审计读不走注入器/远端状态，全局 DB 直查。
#[tauri::command]
pub fn inject_list_audit(limit: Option<usize>) -> serde_json::Value {
    // 锁自愈取锁（P3 统一）：毒锁不连坐——前一次持锁 panic 后审计查看仍可用
    let conn = crate::database::connection::DB
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let rows = crate::database::dao::write_audit::recent_conn(
        &conn, // IPC 入参封顶：LIMIT 超大值等于全表读入内存
        limit.unwrap_or(100).min(1000) as i64,
    );
    serde_json::json!({ "items": rows })
}

/// 审计写口（DB + events::audit 日志并行，M4 T2e 惯例）：channel = 注入器名，
/// summary 只存摘要（W5 防审计库膨胀）。conn 由调用方短临界区传入（锁内只 SQL）。
/// 原 Task 5 的 queue.rs 私有版提升至此（Task 6）：flush/jump/fail 落账（queue::settle）
/// 与 session-send / queue / retract 端点审计两处共用，单一出口防词表漂移。
/// 字段化入参而非 QueueRow：端点侧审计（send/queue/retract）没有整行可传。
/// action 词表（W5）：send|queue|flush|jump|retract|approve|reject|fail|key|open
/// （open = Task 11 一键 resume，Task 7 的预留标注已兑现）。批次乙 T8 追加
/// `answer`（AskUserQuestion 问答应答，select/toggle/submit/cancel 四动作统一
/// 记 answer，摘要区分见 question::AnswerAction::audit_label）。批次丙追加
/// `mode`（T6 模式切换，摘要=「切换模式至 <target>」）。**T9 打断式插队不新增
/// 词**：Esc 中断是 `jump` 动作的**内部分步**（先中断再投递），审计仍记 `jump`
/// ——用户视角是一个动作（见 queue::try_flush_with 的 interrupt_first 分支）。
/// **丁T3 追加 `slash`**（裁2：`/` 开头消息裸注入不带签名——前后缀都会破坏命令与
/// 参数；终端不留痕是可接受的，溯源走审计页：action=slash + device_name 列。
/// 判据与实际注入形态同源单点：`normalize::is_slash_message`，见 session_send 的
/// action 选择处）。
#[allow(clippy::too_many_arguments)]
pub(crate) fn audit_write(
    conn: &rusqlite::Connection,
    st: &crate::remote::server::RemoteState,
    device_id: &str,
    device_name: &str,
    agent_type: &str,
    session_id: &str,
    content: &str,
    action: &str,
    result: &str,
) {
    let channel = st.injector.name();
    let summary = normalize::summarize(content, normalize::AUDIT_SUMMARY_CHARS);
    crate::database::dao::write_audit::record_conn(
        conn,
        chrono::Utc::now().timestamp_millis(),
        device_id,
        device_name,
        agent_type,
        session_id,
        channel,
        action,
        &summary,
        result,
    );
    // 日志留痕并行（remote_audit target 可 grep 追溯）
    crate::remote::events::audit(
        action,
        &format!("sid={session_id} channel={channel} result={result}"),
    );
}
