// 自定义通知浮窗 — 独立无边框置顶小窗，轮转 3 个槽位在右下角纵向堆叠。
// 跨平台实现：不夺键盘焦点（focusable(false)），点击卡片联动跳转会话终端。

use serde::Deserialize;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationPayload {
    pub agent_type: String,  // 原始类型（claude/codex/...），跳转标题打分用
    pub agent_label: String, // 显示名（Claude Code 等专有名词，不翻译）
    pub project_name: String,
    pub status_color: String,
    pub status: String, // 原始状态枚举，由通知页 i18n 翻译
    pub last_message: String,
    pub pid: u32,
    pub session_id: String,
}

const SLOTS: usize = 3;
const W: f64 = 360.0;
const H: f64 = 110.0;
const MARGIN: f64 = 16.0;

/// 槽位占用表：值 = 占用时刻。
/// 不用 is_visible() 判忙：窗口要等事件送达 + 页面 show() 之后才可见（约 300ms+），
/// 同批第二条通知会误判空闲而覆盖第一条。时间戳占用 10 秒自动过期（通知只活 6 秒）。
static SLOT_OCCUPANCY: once_cell::sync::Lazy<
    std::sync::Mutex<std::collections::HashMap<usize, std::time::Instant>>,
> = once_cell::sync::Lazy::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

const SLOT_TTL: std::time::Duration = std::time::Duration::from_secs(10);

#[tauri::command]
pub fn show_notification_window(
    app: AppHandle,
    payload: NotificationPayload,
) -> Result<(), String> {
    // 槽位选择：空闲优先；全忙则顶替最早占用的（时间戳判定，规避可见性竞态）
    let now = std::time::Instant::now();
    let slot = {
        let mut occupancy = SLOT_OCCUPANCY.lock().unwrap();
        occupancy.retain(|_, at| now.duration_since(*at) < SLOT_TTL);
        let s = (0..SLOTS)
            .find(|i| !occupancy.contains_key(i))
            .unwrap_or_else(|| {
                occupancy
                    .iter()
                    .min_by_key(|(_, at)| **at)
                    .map(|(i, _)| *i)
                    .unwrap_or(0)
            });
        occupancy.insert(s, now);
        s
    };
    // 右下角定位 + 槽位纵向堆叠（逻辑坐标；再上移避开任务栏高度）
    let mut mx = 1920.0;
    let mut my = 1080.0;
    if let Ok(Some(m)) = app.primary_monitor() {
        mx = m.size().width as f64 / m.scale_factor();
        my = m.size().height as f64 / m.scale_factor();
    }
    let x = mx - W - MARGIN;
    let y = my - H - MARGIN - 48.0 - (slot as f64) * (H + 8.0);

    let label = format!("notification-{slot}");
    match app.get_webview_window(&label) {
        Some(w) => {
            let _ = w.set_position(tauri::Position::Logical(tauri::LogicalPosition::new(x, y)));
        }
        None => {
            let _ = WebviewWindowBuilder::new(
                &app,
                &label,
                WebviewUrl::App("index.html#/notification".into()),
            )
            .title("mam-notification")
            .decorations(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .resizable(false)
            .transparent(true)
            .focusable(false) // 不夺键盘焦点的关键：通知永不参与焦点切换
            .visible(false) // 页面收到事件后才 show，避免白屏
            .inner_size(W, H)
            .position(x, y)
            .build()
            .map_err(|e| format!("创建通知窗口失败: {}", e))?;
        }
    }
    // 定向发送到该槽位窗口（emit 全局广播会让所有槽位同时弹出）
    // 延迟发送规避"页面 JS 尚未注册 listener"的创建竞态；偶发丢失则该条不显示，下一条正常
    let app2 = app.clone();
    let payload2 = payload.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(300));
        let _ = app2.emit_to(&label, "notification:new", &payload2);
    });
    Ok(())
}
