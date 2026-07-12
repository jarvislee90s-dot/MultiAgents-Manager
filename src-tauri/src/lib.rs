pub mod adapter;
pub mod monitor;
pub mod database;
pub mod window;
pub mod linker;
pub mod services;
pub mod commands;
pub mod plugins;
pub mod session;

use tauri::Manager;
#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

#[tauri::command]
fn update_tray_menu(
    app: tauri::AppHandle,
    show_text: String,
    quit_text: String,
) -> Result<(), String> {
    plugins::system_tray::update_tray_menu(&app, &show_text, &quit_text)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let _ = env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("info")
    ).try_init();
    database::init();
    services::auto_import_extensions(false);  // 首次启动，不强制
    monitor::hooks::register_all_hooks();

    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_focus();
                let _ = window.unminimize();
                let _ = window.show();
            }
        }))
        .setup(|app| {
            // 在 dev 模式下自动打开 devtools，启用 CDP 远程调试
            #[cfg(debug_assertions)]
            {
                if let Some(window) = app.get_webview_window("main") {
                    window.open_devtools();
                }
            }
            Ok(())
        })
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_notification::init())
        .plugin(plugins::system_tray::init())
        ;
    let builder = builder.invoke_handler(tauri::generate_handler![
        greet, update_tray_menu,
        commands::session::get_all_sessions,
        commands::session::focus_session,
        commands::session::kill_session,
        commands::resource::list_extensions_with_assignments,
        commands::resource::scan_native_resources,
        commands::resource::import_native_resources,
        commands::resource::list_tool_resources,
        commands::resource::check_preset_compatibility,
        commands::resource::list_ssot_resources,
        commands::resource::detect_duplicate_skills,
        commands::resource::cleanup_duplicate_skills,
        commands::resource::check_skill_target_type,
        commands::resource::disable_skill_for_tool,
        commands::resource::enable_skill_for_tool_cmd,
        commands::resource::import_mcp_to_ssot,
        commands::resource::save_mcp_config,
        commands::preset::create_preset,
        commands::preset::delete_preset,
        commands::preset::list_presets,
        commands::preset::apply_preset,
        commands::preset::deactivate_preset,
        commands::preset::apply_preset_to_subagent,
        commands::preset::deactivate_preset_from_subagent,
        commands::skill::list_repo_skills,
        commands::skill::install_skill,
        commands::skill::rescan_skills,
        commands::skill::assign_skill_to_subagent,
        commands::mcp::toggle_mcp_for_tool,
        commands::mcp::read_mcp_servers,
        commands::mcp::write_mcp_server,
        commands::mcp::remove_mcp_server,
        commands::plugin::toggle_plugin_for_tool,
        commands::settings::get_setting,
        commands::settings::set_setting,
        commands::settings::detect_tools,
        commands::settings::detect_subagents,
        commands::settings::list_sub_agents,
        commands::screenshot::capture_window_screenshot,
        commands::screenshot::list_screenshots,
        commands::manifest::validate_manifest,
        commands::manifest::install_resource_from_manifest,
        commands::manifest::uninstall_resource,
        commands::manifest::get_store_index,
    ]);

    #[cfg(not(debug_assertions))]
    let builder = {
        // 仅在设置了签名密钥时才注册 updater，否则占位 URL 会 panic
        if std::env::var("TAURI_SIGNING_PRIVATE_KEY").is_ok() {
            builder.plugin(tauri_plugin_updater::Builder::new().build())
        } else {
            builder
        }
    };

    builder
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}