use tauri::menu::{
    CheckMenuItem, IsMenuItem, Menu, MenuItem, MenuItemKind, PredefinedMenuItem, Submenu,
};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{
    plugin::{Builder, TauriPlugin},
    AppHandle, Emitter, Manager, Runtime,
};

/// 托盘菜单标签集（Task 16 i18n 参数化）：真值源在前端（i18next），经 refresh_tray
/// 下发并持久化到 settings；托盘事件线程 / session.rs 启发式等无前端语境的重建
/// 路径沿用最近一次持久化值（此前预设重建会把基础项打回中文占位、语言切换会
/// 丢预设项——统一重建后两个问题一并消除）
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct TrayLabels {
    /// 预设分区标签（"预设" / "Presets"）
    pub presets: String,
    pub show: String,
    pub pet: String,
    pub quit: String,
}

impl Default for TrayLabels {
    fn default() -> Self {
        // 中文兜底：与历史硬编码占位一致（任何前端调用之前的首次重建使用）
        Self {
            presets: "预设".into(),
            show: "显示窗口".into(),
            pet: "显示/隐藏桌宠".into(),
            quit: "退出".into(),
        }
    }
}

const TRAY_LABELS_KEY: &str = "tray_labels";

fn saved_tray_labels() -> TrayLabels {
    crate::database::get_setting(TRAY_LABELS_KEY)
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

impl TrayLabels {
    /// 合并语义：传入的标签覆盖持久化值，None 沿用最近一次值，并回写持久化。
    /// 供 refresh_tray 使用——预设变更处可只追加 presetsLabel 一行，
    /// 无需关心基础项文案
    pub fn merged(
        presets: Option<String>,
        show: Option<String>,
        pet: Option<String>,
        quit: Option<String>,
    ) -> Self {
        let mut labels = saved_tray_labels();
        if let Some(v) = presets {
            labels.presets = v;
        }
        if let Some(v) = show {
            labels.show = v;
        }
        if let Some(v) = pet {
            labels.pet = v;
        }
        if let Some(v) = quit {
            labels.quit = v;
        }
        // set_setting 返回 ()（内部自兜错），持久化失败不影响本次重建
        crate::database::set_setting(
            TRAY_LABELS_KEY,
            &serde_json::to_string(&labels).unwrap_or_default(),
        );
        labels
    }
}

/// 托盘点击翻转判定（spec §5.5 开关模型 + 裁决 2026-09-15「点击=直接执行」）：
/// 该工具当前激活的正是此预设 → "restore"（关 = 恢复默认）；否则 → "apply"
///（开 = 独占应用；激活的是其他预设时同样走 apply，独占语义下旧开关自动翻关）
pub fn preset_toggle_action(active: Option<&str>, preset_id: &str) -> &'static str {
    if active == Some(preset_id) {
        "restore"
    } else {
        "apply"
    }
}

/// 通用预设子项 id（`preset-tool-{preset_id}|{tool_id}`）解析 → (preset_id, tool_id)。
/// 工具 id 与预设 id（`preset-{ts}-{seq}`）均不含 `|`，split_once 划分无歧义
fn parse_universal_child_id(id: &str) -> Option<(&str, &str)> {
    id.strip_prefix("preset-tool-")
        .and_then(|rest| rest.split_once('|'))
}

/// 工具私有预设顶层项 id（`preset-{preset_id}`）解析 → preset_id。
/// 双前缀陷阱：preset.id 本身以 "preset-" 开头（DAO 生成 `preset-{ts}-{seq}`），
/// 菜单 id 因此形如 `preset-preset-…`——strip_prefix 恰好一次即得 preset.id
fn parse_private_item_id(id: &str) -> Option<&str> {
    id.strip_prefix("preset-")
}

/// 工具当前激活预设（托盘选中态数据源；无基底快照 / 无激活 = None）
fn active_preset_of(tool_id: &str) -> Option<String> {
    crate::database::get_base_snapshot(tool_id).and_then(|(active, _)| active)
}

/// 托盘预设点击 = 直接执行（spec §7.5 + 裁决 2026-09-15，无窗口语境不弹确认）。
/// 执行链路与 commands::preset::apply_preset / restore_preset 完全一致（工具启用
/// 守卫 → service 层；restore 走 restore_tool 而非 command）；成败只记日志不
/// panic；最后统一重建菜单刷新选中态——失败时项也回到当前真实状态，可重试
fn toggle_preset_for_tool<R: Runtime>(app: &AppHandle<R>, preset_id: &str, tool_id: &str) {
    let active = active_preset_of(tool_id);
    let action = preset_toggle_action(active.as_deref(), preset_id);
    let outcome = match action {
        "restore" => crate::services::tool_settings::ensure_tool_enabled(tool_id)
            .and_then(|()| crate::services::preset::restore_tool(tool_id).map(|_| ())),
        _ => crate::services::tool_settings::ensure_tool_enabled(tool_id)
            .and_then(|()| crate::services::preset::apply_preset(preset_id, tool_id).map(|_| ())),
    };
    match outcome {
        Ok(()) => log::info!("托盘预设翻转完成（{action}）: {preset_id} @ {tool_id}"),
        Err(e) => log::warn!("托盘预设翻转失败（{action}）: {preset_id} @ {tool_id}: {e}"),
    }
    let labels = saved_tray_labels();
    if let Err(e) = update_tray_with_presets_labeled(app, &labels) {
        log::warn!("托盘菜单重建失败: {e}");
    }
}

// Update tray menu with localized text
pub fn update_tray_menu(
    app: &AppHandle,
    show_text: &str,
    quit_text: &str,
    pet_text: &str,
) -> Result<(), String> {
    let menu = Menu::with_id_and_items(
        app,
        "system-tray",
        &[
            &MenuItem::with_id(app, "show", show_text, true, None::<&str>)
                .map_err(|e| e.to_string())?,
            &MenuItem::with_id(app, "pet", pet_text, true, None::<&str>)
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
                    &MenuItem::with_id(app, "pet", "Show Pet", true, None::<&str>)?,
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
                    // 预设接线（spec §7.5，P5）：点击 = 直接执行翻转（裁决 2026-09-15）。
                    // 通用预设子项 id `preset-tool-{preset_id}|{tool_id}` → 对该工具翻转
                    id if id.starts_with("preset-tool-") => {
                        if let Some((preset_id, tool_id)) = parse_universal_child_id(id) {
                            toggle_preset_for_tool(app, preset_id, tool_id);
                        } else {
                            log::warn!("托盘预设子项 id 无法解析: {id}");
                        }
                    }
                    // 通用预设子菜单标题：点击仅展开，无动作（防御性吞掉，防止误入下方私有分支）
                    id if id.starts_with("preset-group-") => {}
                    // 工具私有预设顶层项：strip 一次得 preset.id（双前缀陷阱见
                    // parse_private_item_id 注释），作用于其绑定工具
                    id if id.starts_with("preset-") => {
                        if let Some(preset_id) = parse_private_item_id(id) {
                            match crate::database::get_preset(preset_id) {
                                Some(preset) => match preset.bound_tool.as_deref() {
                                    Some(tool_id) => {
                                        toggle_preset_for_tool(app, preset_id, tool_id);
                                    }
                                    None => {
                                        log::warn!("预设 {} 无绑定工具，忽略托盘点击", preset.id)
                                    }
                                },
                                None => log::warn!("托盘预设项无对应预设: {preset_id}"),
                            }
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

/// 兜底重建（无前端语境的调用方：session.rs 计数启发式 / 托盘翻转后自刷新）：
/// 沿用最近一次前端下发的标签集（TrayLabels），首次（无持久化值）用中文默认
pub fn update_tray_with_presets(app: &AppHandle) -> Result<(), String> {
    let labels = saved_tray_labels();
    update_tray_with_presets_labeled(app, &labels)
}

/// 统一重建托盘菜单（Task 16）：基础项 + 预设项合并成型，预设项带开/关选中态
///（CheckMenuItem，checked = 该预设在此工具上激活）。通用预设 = 每个已启用工具
/// 一个子项（子菜单展开，子项 id `preset-tool-{preset_id}|{tool_id}`）；
/// 工具私有预设 = 顶层 CheckMenuItem（id `preset-{preset_id}`，作用于绑定工具）。
/// 点击语义在 on_menu_event 的 preset 臂（直接执行，不弹确认）
pub fn update_tray_with_presets_labeled<R: Runtime>(
    app: &AppHandle<R>,
    labels: &TrayLabels,
) -> Result<(), String> {
    let presets = crate::database::list_presets();

    // 基础项（owned，存活于本函数作用域内）
    let show = MenuItem::with_id(app, "show", &labels.show, true, None::<&str>)
        .map_err(|e| e.to_string())?;
    let pet = MenuItem::with_id(app, "pet", &labels.pet, true, None::<&str>)
        .map_err(|e| e.to_string())?;
    let sep1 = PredefinedMenuItem::separator(app).map_err(|e| e.to_string())?;
    let quit = MenuItem::with_id(app, "quit", &labels.quit, true, None::<&str>)
        .map_err(|e| e.to_string())?;

    // 已启用工具（通用预设子菜单的子项集；与 commands::settings::list_enabled_tools 同口径）
    let enabled_tools: Vec<(String, String)> = crate::adapter::TOOL_IDS
        .iter()
        .filter(|id| crate::database::dao::agent_tool::get_tool_enabled(id))
        .filter_map(|id| {
            crate::adapter::adapter_by_id(id).map(|a| (id.to_string(), a.name().to_string()))
        })
        .collect();

    // 预设项统一收集为 MenuItemKind 保序（通用 = 子菜单 / 私有 = CheckMenuItem，
    // 与 list_presets 的 created_at DESC 序一致）
    let mut preset_items: Vec<MenuItemKind<R>> = Vec::new();

    for preset in &presets {
        let label = format!("{}: {}", labels.presets, preset.name);
        match preset.scope.as_str() {
            "universal" => {
                let submenu =
                    Submenu::with_id(app, format!("preset-group-{}", preset.id), &label, true)
                        .map_err(|e| e.to_string())?;
                let mut children = 0usize;
                for (tool_id, tool_name) in &enabled_tools {
                    let checked = active_preset_of(tool_id).as_deref() == Some(preset.id.as_str());
                    let child = CheckMenuItem::with_id(
                        app,
                        format!("preset-tool-{}|{}", preset.id, tool_id),
                        tool_name,
                        true,
                        checked,
                        None::<&str>,
                    )
                    .map_err(|e| e.to_string())?;
                    submenu.append(&child).map_err(|e| e.to_string())?;
                    children += 1;
                }
                // 无已启用工具时不渲染空子菜单
                if children > 0 {
                    preset_items.push(submenu.kind());
                }
            }
            "tool" => {
                let Some(tool_id) = preset.bound_tool.as_deref() else {
                    // bound_tool 缺失的脏数据：无处作用的开关不渲染
                    continue;
                };
                let checked = active_preset_of(tool_id).as_deref() == Some(preset.id.as_str());
                let item = CheckMenuItem::with_id(
                    app,
                    format!("preset-{}", preset.id),
                    &label,
                    true,
                    checked,
                    None::<&str>,
                )
                .map_err(|e| e.to_string())?;
                preset_items.push(item.kind());
            }
            other => log::warn!("未知预设 scope `{other}`（id={}），托盘跳过", preset.id),
        }
    }

    // 预设区与基础区之间的分隔线（仅确有预设项时插入，避免相邻双分隔线）
    let sep2 = if preset_items.is_empty() {
        None
    } else {
        Some(PredefinedMenuItem::separator(app).map_err(|e| e.to_string())?)
    };

    // 收集引用
    let mut items: Vec<&dyn IsMenuItem<R>> = Vec::new();
    items.push(&show);
    items.push(&pet);
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

#[cfg(test)]
mod preset_toggle_tests {
    use super::{parse_private_item_id, parse_universal_child_id, preset_toggle_action};

    /// spec §5.5：无激活 → 开（应用）
    #[test]
    fn no_active_means_apply() {
        assert_eq!(preset_toggle_action(None, "preset-1726000000-0"), "apply");
    }

    /// spec §5.5：激活的是其他预设 → 仍开（独占切换，旧开关自动翻关）
    #[test]
    fn other_active_means_apply() {
        assert_eq!(
            preset_toggle_action(Some("preset-1726000000-1"), "preset-1726000000-0"),
            "apply"
        );
    }

    /// spec §5.5：激活的正是本预设 → 关（恢复默认）
    #[test]
    fn same_active_means_restore() {
        assert_eq!(
            preset_toggle_action(Some("preset-1726000000-0"), "preset-1726000000-0"),
            "restore"
        );
    }

    /// 通用子项 id 解析：`preset-tool-{preset_id}|{tool_id}` 按首个 `|` 二分
    #[test]
    fn universal_child_id_splits_on_first_pipe() {
        let (preset_id, tool_id) =
            parse_universal_child_id("preset-tool-preset-1726000000-0|codex").unwrap();
        assert_eq!(preset_id, "preset-1726000000-0");
        assert_eq!(tool_id, "codex");
    }

    /// 无 `|`（或缺前缀）的子项 id 必须解析失败，走 warn 而非 panic
    #[test]
    fn universal_child_id_without_pipe_is_none() {
        assert!(parse_universal_child_id("preset-tool-preset-1726000000-0").is_none());
        assert!(parse_universal_child_id("show").is_none());
    }

    /// 双前缀陷阱回归锁：preset.id 以 "preset-" 开头，菜单 id 形如
    /// `preset-preset-…`——strip 恰好一次必须还原完整 preset.id
    #[test]
    fn private_item_id_strips_exactly_once() {
        assert_eq!(
            parse_private_item_id("preset-preset-1726000000-0"),
            Some("preset-1726000000-0")
        );
        assert_eq!(parse_private_item_id("show"), None);
    }
}
