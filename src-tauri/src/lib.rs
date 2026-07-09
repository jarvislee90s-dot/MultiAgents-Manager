mod adapter;
mod monitor;
mod database;
mod window;
mod linker;
mod services;
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
    let builder = commands::add_commands(builder);
    let builder = builder.invoke_handler(tauri::generate_handler![
        greet, update_tray_menu, commands::session::get_all_sessions, commands::screenshot::capture_window_screenshot
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