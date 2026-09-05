pub mod adapter;
pub mod commands;
pub mod database;
pub mod linker;
pub mod monitor;
pub mod plugins;
pub mod services;
pub mod session;
pub mod window;

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
    pet_text: String,
) -> Result<(), String> {
    plugins::system_tray::update_tray_menu(&app, &show_text, &quit_text, &pet_text)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .try_init();
    database::init();
    // 后台增量导入（仅导入 DB 中不存在的 name）+ 补链，不阻塞启动
    std::thread::spawn(|| {
        services::auto_import_extensions(false);
        services::sync_imported_skill_links();
        // 清扫 .import-staging 崩溃残留（issue #32-3）：启动时机无运行中导入，安全
        services::pet::sweep_staging();
    });
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
            // 桌宠窗口：延迟创建（spec §4.1）。不能在 setup 里立即建——Windows 上主窗口
            // WebView2 控制器初始化期存在竞态，立即创建会偶发 E_INVALIDARG 且被
            // tauri 吞错成幽灵窗口（详见 commands/pet.rs 模块注释）；延迟 800ms 避开
            let pet_handle = app.handle().clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(800));
                if let Err(e) = commands::pet::create_pet_window(&pet_handle) {
                    log::warn!("pet window create failed: {}", e);
                }
            });
            Ok(())
        })
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(plugins::system_tray::init());
    let builder = builder.invoke_handler(tauri::generate_handler![
        greet,
        update_tray_menu,
        commands::session::get_all_sessions,
        commands::session::focus_session,
        commands::session::focus_hwnd,
        commands::session::kill_session,
        commands::session::dismiss_session_card,
        commands::notification::show_notification_window,
        commands::pet::set_pet_visible,
        commands::pet::set_pet_always_on_top,
        commands::pet::pet_list_pets,
        commands::pet::pet_list_codex_pets,
        commands::pet::pet_scan,
        commands::pet::pet_read_manifest,
        commands::pet::pet_stage_from_folder,
        commands::pet::pet_stage_from_zip,
        commands::pet::pet_stage_from_codex,
        commands::pet::pet_stage_from_petdex,
        commands::pet::pet_stage_audio,
        commands::pet::pet_remove_staged_audio,
        commands::pet::pet_finalize_import,
        commands::pet::pet_cancel_import,
        commands::pet::pet_update_manifest,
        commands::pet::pet_rename_pet,
        commands::pet::pet_delete_pet,
        commands::pet::pet_add_voice_files,
        commands::pet::pet_remove_voice_file,
        commands::pet::pet_reveal_folder,
        commands::resource::list_extensions_with_assignments,
        commands::resource::open_tool_resource,
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
        commands::settings::mark_session_read,
        commands::settings::get_tool_settings,
        commands::settings::update_tool_settings,
        commands::settings::list_enabled_tools,
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
