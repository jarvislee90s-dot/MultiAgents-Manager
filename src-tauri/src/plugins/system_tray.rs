use tauri::menu::{CheckMenuItem, IsMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{
    plugin::{Builder, TauriPlugin},
    AppHandle, Emitter, Manager, Runtime,
};

/// 远程区菜单项（M4 T4）：开关（Check，勾选态=当前远程状态）+ 地址展示（点击经
/// 事件走前端剪贴板，复制在主窗口完成）。文本由调用方传入（前端本地化），
/// 状态/地址 Rust 侧自查（remote::tray_display：隧道开地址优先，关=空串）。
/// 泛型 R 为最小适配：init 默认菜单在泛型 Runtime 上构造（蓝本硬编码 Wry 对不上）；
/// 显式 'a 为编译适配：`&mut Vec<&dyn>` 不变性要求三个 owned 容器与收集表同享
/// 元素生存期（蓝本省略 'a 过不了编译）；容器由调用方声明——菜单项必须存活到
/// Menu 构建完成（借用语义）
fn push_remote_items<'a, R: Runtime>(
    app: &AppHandle<R>,
    items: &mut Vec<&'a dyn IsMenuItem<R>>,
    owned: &'a mut Vec<CheckMenuItem<R>>, // owned 存活容器
    addr_owned: &'a mut Vec<MenuItem<R>>,
    sep_owned: &'a mut Vec<PredefinedMenuItem<R>>,
    remote_on_text: &str,
) -> Result<(), String> {
    let (enabled, addr) = crate::remote::tray_display();
    let sep = PredefinedMenuItem::separator(app).map_err(|e| e.to_string())?;
    let toggle = CheckMenuItem::with_id(app, "remote", remote_on_text, true, enabled, None::<&str>)
        .map_err(|e| e.to_string())?;
    // 地址为空（远程关）时展示占位「—」并禁用点击复制
    // （先取 enabled 与标签再构造：addr 在拼标签时被移动，蓝本的行内写法
    // `&if …else { addr }` 后再读 addr 是 E0382，语义不变仅调序）
    let addr_enabled = !addr.is_empty();
    let addr_label = if addr.is_empty() {
        "—".to_string()
    } else {
        addr
    };
    let addr_item = MenuItem::with_id(app, "remote-addr", &addr_label, addr_enabled, None::<&str>)
        .map_err(|e| e.to_string())?;
    sep_owned.push(sep);
    owned.push(toggle);
    addr_owned.push(addr_item);
    items.push(sep_owned.last().unwrap() as _);
    items.push(owned.last().unwrap() as _);
    items.push(addr_owned.last().unwrap() as _);
    Ok(())
}

// Update tray menu with localized text
pub fn update_tray_menu(
    app: &AppHandle,
    show_text: &str,
    quit_text: &str,
    pet_text: &str,
    remote_on_text: &str,
) -> Result<(), String> {
    // owned 容器 + 引用收集（与 update_tray_with_presets 同模式）：远程区项
    // （sep → 开关 → 地址）统一插在 quit 之前——本路径漏加则每次语言/桌宠刷新抹掉远程项
    let show =
        MenuItem::with_id(app, "show", show_text, true, None::<&str>).map_err(|e| e.to_string())?;
    let pet =
        MenuItem::with_id(app, "pet", pet_text, true, None::<&str>).map_err(|e| e.to_string())?;
    let sep1 = PredefinedMenuItem::separator(app).map_err(|e| e.to_string())?;
    let quit =
        MenuItem::with_id(app, "quit", quit_text, true, None::<&str>).map_err(|e| e.to_string())?;

    let mut items: Vec<&dyn IsMenuItem<tauri::Wry>> = vec![&show, &pet, &sep1];
    let mut remote_owned: Vec<CheckMenuItem<tauri::Wry>> = Vec::new();
    let mut remote_addr_owned: Vec<MenuItem<tauri::Wry>> = Vec::new();
    let mut remote_seps: Vec<PredefinedMenuItem<tauri::Wry>> = Vec::new();
    push_remote_items(
        app,
        &mut items,
        &mut remote_owned,
        &mut remote_addr_owned,
        &mut remote_seps,
        remote_on_text,
    )?;
    items.push(&quit);

    let menu = Menu::with_id_and_items(app, "system-tray", &items).map_err(|e| e.to_string())?;

    if let Some(tray) = app.tray_by_id("main-tray") {
        tray.set_menu(Some(menu)).map_err(|e| e.to_string())?;
    }

    Ok(())
}

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("system-tray")
        .setup(|app, _| {
            // Create tray menu with default English text
            // 三重建路径统一（M4 T4）：默认菜单同样追加远程区项，否则前端首次重建前
            // 托盘缺远程入口（文本为英文占位，主窗口挂载后由前端本地化重建）
            let show = MenuItem::with_id(app, "show", "Show Window", true, None::<&str>)?;
            let pet = MenuItem::with_id(app, "pet", "Show Pet", true, None::<&str>)?;
            let sep1 = PredefinedMenuItem::separator(app)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;

            let mut items: Vec<&dyn IsMenuItem<R>> = vec![&show, &pet, &sep1];
            let mut remote_owned: Vec<CheckMenuItem<R>> = Vec::new();
            let mut remote_addr_owned: Vec<MenuItem<R>> = Vec::new();
            let mut remote_seps: Vec<PredefinedMenuItem<R>> = Vec::new();
            push_remote_items(
                app,
                &mut items,
                &mut remote_owned,
                &mut remote_addr_owned,
                &mut remote_seps,
                "Remote Access",
            )?;
            items.push(&quit);

            let menu = Menu::with_id_and_items(app, "system-tray", &items)?;

            // Build tray icon
            TrayIconBuilder::with_id("main-tray")
                .menu(&menu)
                .icon(app.default_window_icon().unwrap().clone())
                .tooltip("MultiAgents Manager")
                .show_menu_on_left_click(false)
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        // Left click to show main window
                        let app = tray.app_handle();
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.unminimize();
                            let _ = window.set_focus();
                        }
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
                    "pet" => {
                        // 托盘切换桌宠显隐：以窗口实际可见性为准，并广播给前端同步（spec §10.2）
                        if let Some(w) = app.get_webview_window("pet") {
                            let visible = w.is_visible().unwrap_or(false);
                            let next = !visible;
                            if next {
                                let _ = w.show();
                            } else {
                                let _ = w.hide();
                            }
                            let _ = app.emit(
                                "pet-visibility-changed",
                                serde_json::json!({ "visible": next }),
                            );
                        }
                    }
                    "quit" => {
                        app.exit(0);
                    }
                    "remote" => {
                        // 托盘开关：与设置页同一命令链路（remote_toggle 内含 TLS 门与回滚）。
                        // 菜单勾选态经前端回环刷新（remote-changed 监听重建，见 home.tsx 接线）
                        let next = !crate::remote::tray_display().0;
                        match crate::remote::remote_toggle(next) {
                            Ok(()) => {
                                crate::remote::events::emit_ui(
                                    "remote-changed",
                                    serde_json::json!({ "enabled": next }),
                                );
                            }
                            Err(e) => {
                                crate::remote::events::emit_ui(
                                    "remote-toggle-failed",
                                    serde_json::json!({ "error": e }),
                                );
                            }
                        }
                    }
                    "remote-addr" => {
                        let (_, addr) = crate::remote::tray_display();
                        if !addr.is_empty() {
                            crate::remote::events::emit_ui(
                                "remote-copy-addr",
                                serde_json::json!({ "url": addr }),
                            );
                        }
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
            format!(
                "MultiAgents Manager \u{2014} {} sessions, {} waiting",
                total_count, waiting_count
            )
        } else {
            format!("MultiAgents Manager \u{2014} {} sessions", total_count)
        };
        let _ = tray.set_tooltip(Some(&tooltip));
    }
}

/// 更新托盘菜单，加入预设组列表
pub fn update_tray_with_presets(app: &AppHandle) -> Result<(), String> {
    let presets = crate::database::list_presets();

    // 创建菜单项（owned，存活于本函数作用域内）
    let show = MenuItem::with_id(app, "show", "显示窗口", true, None::<&str>)
        .map_err(|e| e.to_string())?;
    // 桌宠显隐项（与 update_tray_menu 同序：show → pet → separator...）；
    // 前端轮询/语言切换会用本地化文本重建菜单，此处占位中文标签保持一致
    let pet = MenuItem::with_id(app, "pet", "显示/隐藏桌宠", true, None::<&str>)
        .map_err(|e| e.to_string())?;
    let sep1 = PredefinedMenuItem::separator(app).map_err(|e| e.to_string())?;
    let quit =
        MenuItem::with_id(app, "quit", "退出", true, None::<&str>).map_err(|e| e.to_string())?;

    let mut preset_items: Vec<MenuItem<tauri::Wry>> = Vec::new();
    let mut sep2: Option<PredefinedMenuItem<tauri::Wry>> = None;

    if !presets.is_empty() {
        for preset in &presets {
            let id = format!("preset-{}", preset.id);
            let label = format!("预设: {}", preset.name);
            preset_items.push(
                MenuItem::with_id(app, &id, &label, true, None::<&str>)
                    .map_err(|e| e.to_string())?,
            );
        }
        sep2 = Some(PredefinedMenuItem::separator(app).map_err(|e| e.to_string())?);
    }

    // 收集引用
    let mut items: Vec<&dyn IsMenuItem<tauri::Wry>> = Vec::new();
    items.push(&show);
    items.push(&pet);
    items.push(&sep1);
    for item in &preset_items {
        items.push(item);
    }
    if let Some(ref s) = sep2 {
        items.push(s);
    }
    // 远程区项插在 quit 之前（M4 T4 三重建路径统一）：本路径漏加则每次预设组
    // 刷新即抹掉远程项；文本与既有占位标签同风格用中文（前端重建时本地化）
    let mut remote_owned: Vec<CheckMenuItem<tauri::Wry>> = Vec::new();
    let mut remote_addr_owned: Vec<MenuItem<tauri::Wry>> = Vec::new();
    let mut remote_seps: Vec<PredefinedMenuItem<tauri::Wry>> = Vec::new();
    push_remote_items(
        app,
        &mut items,
        &mut remote_owned,
        &mut remote_addr_owned,
        &mut remote_seps,
        "远程接入",
    )?;
    items.push(&quit);

    let menu = Menu::with_items(app, &items).map_err(|e| e.to_string())?;

    if let Some(tray) = app.tray_by_id("main-tray") {
        tray.set_menu(Some(menu)).map_err(|e| e.to_string())?;
    }

    Ok(())
}
