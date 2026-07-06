mod adapter;
mod monitor;
mod store;
mod terminal;
mod linker;
mod manager;
mod commands;
mod plugins;
mod session;

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
    store::init();
    manager::auto_import_extensions(false);  // 首次启动，不强制
    monitor::hooks::register_all_hooks();

    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_focus();
                let _ = window.unminimize();
                let _ = window.show();
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_notification::init())
        .plugin(plugins::system_tray::init())
        .invoke_handler(tauri::generate_handler![
            greet,
            update_tray_menu,
            commands::get_all_sessions,
            commands::focus_session,
            commands::get_setting,
            commands::set_setting,
            commands::list_extensions_with_assignments,
            commands::list_repo_skills,
            commands::install_skill,
            commands::toggle_mcp_for_tool,
            commands::toggle_plugin_for_tool,
            commands::list_presets,
            commands::create_preset,
            commands::delete_preset,
            commands::apply_preset,
            commands::deactivate_preset,
            commands::apply_preset_to_subagent,
            commands::deactivate_preset_from_subagent,
            commands::detect_subagents,
            commands::kill_session,
            commands::list_sub_agents,
            commands::read_mcp_servers,
            commands::write_mcp_server,
            commands::remove_mcp_server,
            commands::detect_tools,
            commands::assign_skill_to_subagent,
            commands::rescan_skills,
        ]);

    #[cfg(not(debug_assertions))]
    let builder = builder.plugin(tauri_plugin_updater::Builder::new().build());

    builder
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}