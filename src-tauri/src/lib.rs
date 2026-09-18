pub mod adapter;
pub mod commands;
pub mod database;
pub mod inject;
pub mod linker;
pub mod monitor;
pub mod plugins;
pub mod remote;
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
    remote_on_text: String,
) -> Result<(), String> {
    plugins::system_tray::update_tray_menu(&app, &show_text, &quit_text, &pet_text, &remote_on_text)
}

/// 托盘菜单统一重建（Task 16）：基础项 + 预设项（带开/关选中态）一次成型。
/// 标签合并语义：传入的标签覆盖持久化值，未传的沿用最近一次值——预设增删/
/// 开关变化处可只追加 presetsLabel 一行调用，无需关心基础项文案。
/// M4 T4 并入：新增 remote_on_text（远程开关项标签，勾选态/地址 Rust 侧自查）。
/// `update_tray_menu` 保留但前端已不再调用（保留至下个清理窗口移除）
#[tauri::command]
fn refresh_tray(
    app: tauri::AppHandle,
    presets_label: Option<String>,
    show_text: Option<String>,
    pet_text: Option<String>,
    quit_text: Option<String>,
    remote_on_text: Option<String>,
) -> Result<(), String> {
    let labels = plugins::system_tray::TrayLabels::merged(
        presets_label,
        show_text,
        pet_text,
        quit_text,
        remote_on_text,
    );
    plugins::system_tray::update_tray_with_presets_labeled(&app, &labels)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .try_init();
    database::init();
    // 后台增量导入（仅导入 DB 中不存在的 name）+ 补链，不阻塞启动
    std::thread::spawn(|| {
        // 清扫 .import-staging 崩溃残留（issue #32-3）：必须先于增量导入/补链执行，
        // 二者可能耗时数秒，期间 IPC 已可用、用户可能已发起导入，晚清扫会误删活跃暂存区
        services::pet::sweep_staging();
        // 预设 v2：孤儿暂存回移 + 注册表回填（先于导入/补链，保证表口径就绪）
        services::preset::stash::recover_orphans();
        for msg in services::preset::check_snapshot_invariants() {
            log::warn!("预设快照不变量违背: {}", msg);
        }
        services::resource::backfill_registry();
        // 预设 v2（spec §13）：账本-磁盘漂移扫描，启动时 warn 收口
        for d in services::resource::reconcile::scan_drift() {
            log::warn!("[漂移{}] {} {}", d.kind, d.extension_id, d.path);
        }
        services::auto_import_extensions(false);
        services::sync_imported_skill_links();
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
            // M4：全局句柄落位（先于 restore_on_launch——隧道自启即可发通知）
            let _ = crate::remote::events::APP_HANDLE.set(app.handle().clone());
            // M2 远程接入：按设置恢复远程服务器（开机自启语义；内部用
            // tauri::async_runtime，无 runtime 上下文的主线程可安全调用）
            crate::remote::restore_on_launch();
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
        refresh_tray,
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
        commands::resource::list_frontmatter_suggestions,
        commands::resource::list_tool_resources,
        commands::resource::check_preset_compatibility,
        commands::resource::list_ssot_resources,
        commands::resource::scan_ledger_drift,
        commands::resource::reconcile_item,
        commands::resource::reconcile_tool_batch,
        commands::resource::scan_empty_dirs,
        commands::resource::clean_empty_dirs,
        commands::resource::reveal_dir,
        commands::resource::detect_duplicate_skills,
        commands::resource::cleanup_duplicate_skills,
        commands::resource::check_skill_target_type,
        commands::resource::disable_skill_for_tool,
        commands::resource::enable_skill_for_tool_cmd,
        commands::resource::import_mcp_to_ssot,
        commands::resource::save_mcp_config,
        commands::resource::detect_legacy_agents_links,
        commands::resource::migrate_legacy_agents_links,
        commands::preset::create_preset,
        commands::preset::get_preset,
        commands::preset::update_preset,
        commands::preset::restore_preset,
        commands::preset::get_active_preset,
        commands::preset::list_active_presets,
        commands::preset::get_tool_active_resources,
        commands::preset::preview_apply_preset,
        commands::preset::set_resource_binding,
        commands::preset::list_resource_bindings,
        commands::preset::delete_resource_binding,
        commands::preset::set_tool_resident,
        commands::preset::list_tool_residents,
        commands::preset::delete_preset,
        commands::preset::list_presets,
        commands::preset::apply_preset,
        commands::preset::deactivate_preset,
        commands::preset::apply_preset_to_subagent,
        commands::preset::deactivate_preset_from_subagent,
        commands::preset::get_preset_health,
        commands::preset::restore_stash_entry,
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
        commands::settings::set_theme,
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
        remote::remote_toggle,
        remote::remote_status,
        remote::remote_confirm_public,
        remote::remote_devices,
        remote::remote_revoke_device,
        remote::remote_revoke_all_devices,
        // M5 A4：访问密码设置 / 重置设备 / 设备重命名（吊销收窄后的新口径命令）
        remote::remote_set_pin,
        remote::remote_reset_devices,
        remote::remote_rename_device,
        // M5 A5：三通道独立开关（旧 remote_set_channel 单值三选一已随之下线）
        remote::remote_toggle_channel,
        // M7 W5：桌面端写审计查看（最近 N 条，只读）
        inject::inject_list_audit,
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

    // M4：退出钩子（spec §8 应用退出清理子进程与电源锁）——旧 `.run(ctx)` 无事件回调，
    // 改为 build + run 回调：RunEvent::Exit 时停隧道（M5 A5 双通道 stop_all；
    // kill_on_drop 兜不住进程级退出）与电源锁（caffeinate kill / 执行状态清除 +
    // 磁盘代设还原），不留孤儿进程、不失电源锁
    let app = builder
        .build(tauri::generate_context!())
        .expect("error while building tauri application");
    app.run(|_app, event| {
        if let tauri::RunEvent::Exit = event {
            crate::remote::tunnel::stop_all();
            crate::remote::power::release();
        }
    });
}
