// 自定义通知浮窗 — 独立无边框置顶小窗，轮转 3 个槽位在右下角纵向堆叠。
// 跨平台实现：不夺键盘焦点（focusable(false)），点击卡片联动跳转会话终端。

use serde::Deserialize;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationPayload {
    pub agent_type: String,
    pub project_name: String,
    pub status_color: String,
    pub status_label: String,
    pub last_message: String,
    pub pid: u32,
    pub session_id: String,
}

const SLOTS: usize = 3;
const W: f64 = 360.0;
const H: f64 = 110.0;
const MARGIN: f64 = 16.0;

#[tauri::command]
pub fn show_notification_window(
    app: AppHandle,
    payload: NotificationPayload,
) -> Result<(), String> {
    // 找一个隐藏的槽位；全忙则复用第 0 个（顶替最旧，简化策略）
    let mut slot = 0;
    for i in 0..SLOTS {
        if let Some(w) = app.get_webview_window(&format!("notification-{i}")) {
            if !w.is_visible().unwrap_or(false) {
                slot = i;
                break;
            }
        } else {
            slot = i;
            break;
        }
    }
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
