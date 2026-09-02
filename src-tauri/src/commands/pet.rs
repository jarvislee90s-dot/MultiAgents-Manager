// 桌宠窗口管理 — 建窗参数与显隐/置顶（spec §4.1/§4.5）
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

const PET_W: f64 = 340.0;
const PET_H: f64 = 260.0;

/// 创建桌宠窗口（隐藏态；前端加载后按 localStorage 决定显隐，避免启动闪现）
pub fn create_pet_window(app: &AppHandle) -> Result<(), String> {
    if app.get_webview_window("pet").is_some() {
        return Ok(());
    }
    // 默认主显示器右下角；前端随后按记忆位置与实测尺寸重排
    let mut mx = 1440.0;
    let mut my = 900.0;
    if let Ok(Some(m)) = app.primary_monitor() {
        mx = m.size().width as f64 / m.scale_factor();
        my = m.size().height as f64 / m.scale_factor();
    }
    let x = mx - PET_W - 24.0;
    let y = my - PET_H - 76.0;
    WebviewWindowBuilder::new(app, "pet", WebviewUrl::App("index.html#/pet".into()))
        .title("mam-pet")
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .visible(false)
        .inner_size(PET_W, PET_H)
        .position(x, y)
        .build()
        .map(|_| ())
        .map_err(|e| format!("创建桌宠窗口失败: {}", e))
}

#[tauri::command]
pub async fn set_pet_visible(app: AppHandle, visible: bool) -> Result<(), String> {
    if let Some(w) = app.get_webview_window("pet") {
        if visible {
            w.show().map_err(|e| e.to_string())?;
        } else {
            w.hide().map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn set_pet_always_on_top(app: AppHandle, on_top: bool) -> Result<(), String> {
    if let Some(w) = app.get_webview_window("pet") {
        w.set_always_on_top(on_top).map_err(|e| e.to_string())?;
    }
    Ok(())
}
