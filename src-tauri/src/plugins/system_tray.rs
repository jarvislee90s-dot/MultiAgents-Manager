use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{
    plugin::{Builder, TauriPlugin},
    AppHandle, Manager, Runtime,
};

// Update tray menu with localized text
pub fn update_tray_menu(app: &AppHandle, show_text: &str, quit_text: &str) -> Result<(), String> {
    let menu = Menu::with_id_and_items(
        app,
        "system-tray",
        &[
            &MenuItem::with_id(app, "show", show_text, true, None::<&str>)
                .map_err(|e| e.to_string())?,
            &PredefinedMenuItem::separator(app).map_err(|e| e.to_string())?,
            &MenuItem::with_id(app, "quit", quit_text, true, None::<&str>)
                .map_err(|e| e.to_string())?,
        ],
    )
    .map_err(|e| e.to_string())?;

    if let Some(tray) = app.tray_by_id("main-tray") {
        tray.set_menu(Some(menu)).map_err(|e| e.to_string())?;
    }

    Ok(())
}

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("system-tray")
        .setup(|app, _| {
            // Create tray menu with default English text
            let menu = Menu::with_id_and_items(
                app,
                "system-tray",
                &[
                    &MenuItem::with_id(app, "show", "Show Window", true, None::<&str>)?,
                    &PredefinedMenuItem::separator(app)?,
                    &MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?,
                ],
            )?;

            // Build tray icon
            TrayIconBuilder::with_id("main-tray")
                .menu(&menu)
                .icon(app.default_window_icon().unwrap().clone())
                .tooltip("MultiAgents Manager")
                .show_menu_on_left_click(false)
                .on_tray_icon_event(|tray, event| {
                    match event {
                        TrayIconEvent::Click {
                            button: MouseButton::Left,
                            button_state: MouseButtonState::Up,
                            ..
                        } => {
                            // Left click to show main window
                            let app = tray.app_handle();
                            if let Some(window) = app.get_webview_window("main") {
                                let _ = window.show();
                                let _ = window.unminimize();
                                let _ = window.set_focus();
                            }
                        }
                        _ => {}
                    }
                })
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.unminimize();
                            let _ = window.set_focus();
                        }
                    }
                    "quit" => {
                        app.exit(0);
                    }
                    _ => {}
                })
                .build(app)?;
            Ok(())
        })
        .on_window_ready(move |window| {
            let window_clone = window.clone();
            window.on_window_event(move |event| {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    // Hide window instead of exiting when close is requested
                    let _ = window_clone.hide();
                    api.prevent_close();
                }
            });
        })
        .build()
}

/// 更新托盘聚合状态（红/黄/绿灯 + 等待数）
pub fn update_tray_status(
    app: &AppHandle,
    waiting_count: usize,
    total_count: usize,
    has_processing: bool,
) {
    if let Some(tray) = app.tray_by_id("main-tray") {
        // 托盘标题：🔴N 等待 / 🟡 运行中 / 🟢 空闲 / 空
        let title = if waiting_count > 0 {
            format!("\u{1F534}{}", waiting_count)
        } else if has_processing {
            "\u{1F7E1}".to_string()
        } else if total_count > 0 {
            "\u{1F7E2}".to_string()
        } else {
            String::new()
        };
        let _ = tray.set_title(if title.is_empty() { None } else { Some(&title) });

        // 托盘提示
        let tooltip = if total_count == 0 {
            "MultiAgents Manager".to_string()
        } else if waiting_count > 0 {
            format!("MultiAgents Manager \u{2014} {} sessions, {} waiting", total_count, waiting_count)
        } else {
            format!("MultiAgents Manager \u{2014} {} sessions", total_count)
        };
        let _ = tray.set_tooltip(Some(&tooltip));
    }
}


/// 更新托盘菜单，加入预设组列表
pub fn update_tray_with_presets(app: &AppHandle) -> Result<(), String> {
    use tauri::menu::{IsMenuItem, Menu, MenuItem, PredefinedMenuItem};

    let presets = crate::database::list_presets();

    // 创建菜单项（owned，存活于本函数作用域内）
    let show = MenuItem::with_id(app, "show", "显示窗口", true, None::<&str>).map_err(|e| e.to_string())?;
    let sep1 = PredefinedMenuItem::separator(app).map_err(|e| e.to_string())?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>).map_err(|e| e.to_string())?;

    let mut preset_items: Vec<MenuItem<tauri::Wry>> = Vec::new();
    let mut sep2: Option<PredefinedMenuItem<tauri::Wry>> = None;

    if !presets.is_empty() {
        for preset in &presets {
            let id = format!("preset-{}", preset.id);
            let label = format!("预设: {}", preset.name);
            preset_items.push(MenuItem::with_id(app, &id, &label, true, None::<&str>).map_err(|e| e.to_string())?);
        }
        sep2 = Some(PredefinedMenuItem::separator(app).map_err(|e| e.to_string())?);
    }

    // 收集引用
    let mut items: Vec<&dyn IsMenuItem<tauri::Wry>> = Vec::new();
    items.push(&show);
    items.push(&sep1);
    for item in &preset_items {
        items.push(item);
    }
    if let Some(ref s) = sep2 {
        items.push(s);
    }
    items.push(&quit);

    let menu = Menu::with_items(app, &items).map_err(|e| e.to_string())?;

    if let Some(tray) = app.tray_by_id("main-tray") {
        tray.set_menu(Some(menu)).map_err(|e| e.to_string())?;
    }

    Ok(())
}
