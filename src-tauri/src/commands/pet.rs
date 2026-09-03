// 桌宠窗口管理 — 建窗参数与显隐/置顶（spec §4.1/§4.5）
//
// Windows 实测问题（2026-09-02）：setup 期紧跟主窗口创建桌宠的透明 webview 时，
// WebView2 控制器初始化存在竞态，偶发 E_INVALIDARG 失败；且 tauri-runtime-wry 会
// 吞错——build() 仍返回 Ok，OS 层窗口不存在但注册表留下"幽灵"，此后 show/hide
// 全部对幽灵返回成功，桌宠永远出不来且无任何报错。本模块三层防御：
//   1. window_alive：OS 层校验（hwnd 失效即未落地），不信任 build() 的返回值
//   2. create_pet_window：幽灵销毁重建 + 重试（5 次 × 300ms）
//   3. set_pet_visible(true) 兜底：窗口缺失/幽灵时现场重建，保证开关必然拉出桌宠
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

const PET_W: f64 = 340.0;
const PET_H: f64 = 260.0;

/// 建窗/重建串行化：开关连点与启动延迟建窗并发时避免同标签竞争
static PET_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(windows)]
mod win {
    #[link(name = "user32")]
    extern "system" {
        pub fn IsWindow(hwnd: isize) -> i32;
    }
}

/// 窗口真实落地校验：hwnd 拿不到（注册表幽灵无句柄）或句柄已失效（IsWindow=0，
/// 窗口对象在但 OS 层从未建成）都判为未落地。非 Windows 无此竞态，恒真。
#[cfg(windows)]
fn window_alive(w: &tauri::WebviewWindow) -> bool {
    match w.hwnd() {
        Ok(h) => unsafe { win::IsWindow(h.0 as isize) != 0 },
        Err(_) => false,
    }
}
#[cfg(not(windows))]
fn window_alive(_w: &tauri::WebviewWindow) -> bool {
    true
}

/// 清除可能存在的幽灵/旧窗口（幂等）：destroy 后注册表条目经事件循环异步清除，
/// 轮询等待消失（最多 300ms），避免同标签重建冲突
fn clear_pet_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("pet") {
        let _ = w.destroy();
        for _ in 0..10 {
            if app.get_webview_window("pet").is_none() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
    }
}

/// 创建桌宠窗口（隐藏态；前端加载后按 localStorage 决定显隐，避免启动闪现）。
/// 带落地校验与重试：已存在且真实落地则直接复用；幽灵或创建失败则销毁重建
pub fn create_pet_window(app: &AppHandle) -> Result<(), String> {
    let _guard = PET_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(existing) = app.get_webview_window("pet") {
        if window_alive(&existing) {
            return Ok(());
        }
        log::warn!("pet window 未落地（启动竞态幽灵），销毁重建");
        clear_pet_window(app);
    }
    let mut last_err = String::new();
    for attempt in 1..=5u8 {
        match build_pet_window(app) {
            Ok(w) => {
                if window_alive(&w) {
                    if attempt > 1 {
                        log::info!("pet window 第 {} 次尝试创建成功", attempt);
                    }
                    return Ok(());
                }
                last_err = "窗口未落地（幽灵）".to_string();
            }
            Err(e) => last_err = e,
        }
        clear_pet_window(app);
        std::thread::sleep(std::time::Duration::from_millis(300));
    }
    Err(format!("桌宠窗口创建失败（已重试 5 次）: {}", last_err))
}

fn build_pet_window(app: &AppHandle) -> Result<tauri::WebviewWindow, String> {
    // 默认主显示器右下角；前端随后按记忆位置与实测尺寸重排
    let mut mx = 1440.0;
    let mut my = 900.0;
    if let Ok(Some(m)) = app.primary_monitor() {
        mx = m.size().width as f64 / m.scale_factor();
        my = m.size().height as f64 / m.scale_factor();
    }
    let x = mx - PET_W - 24.0;
    let y = my - PET_H - 76.0;
    let w = WebviewWindowBuilder::new(app, "pet", WebviewUrl::App("index.html#/pet".into()))
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
        .map_err(|e| format!("创建桌宠窗口失败: {}", e))?;
    // dev 模式下 WebView2 可能给 webview 自动附加 DevTools 独立窗口，显式关闭
    #[cfg(debug_assertions)]
    w.close_devtools();
    Ok(w)
}

#[tauri::command]
pub async fn set_pet_visible(app: AppHandle, visible: bool) -> Result<(), String> {
    if visible {
        // 开启兜底：窗口缺失或幽灵（启动竞态被吞错）→ 现场重建再显示，
        // 保证无论启动抽签结果如何，开关一定能把桌宠拉出来
        let alive = app
            .get_webview_window("pet")
            .map(|w| window_alive(&w))
            .unwrap_or(false);
        if !alive {
            create_pet_window(&app)?;
        }
    }
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

// ===== 外部宠物 IPC（spec 2026-09-03-external-pet-import §5.1）=====
use crate::services::pet::{self, import, manifest, petdex, scan};

fn root() -> std::path::PathBuf {
    pet::pets_root()
}

#[tauri::command]
pub async fn pet_list_pets() -> Result<Vec<scan::PetSummary>, String> {
    Ok(scan::list_pets_in(&root()))
}

#[tauri::command]
pub async fn pet_list_codex_pets() -> Result<Vec<scan::CodexPetInfo>, String> {
    let codex = dirs::home_dir().unwrap_or_default().join(".codex").join("pets");
    Ok(scan::list_codex_pets_in(&codex, &root()))
}

#[tauri::command]
pub async fn pet_scan(id: String) -> Result<scan::PetScan, String> {
    scan::scan_pet_in(&root(), &id)
}

#[tauri::command]
pub async fn pet_read_manifest(id: String) -> Result<Option<manifest::PetManifest>, String> {
    Ok(manifest::load(&pet::pet_dir(&root(), &id)))
}

#[tauri::command]
pub async fn pet_stage_from_folder(path: String) -> Result<import::StagedPet, String> {
    import::stage_from_folder_in(&root(), std::path::Path::new(&path))
}

#[tauri::command]
pub async fn pet_stage_from_zip(path: String) -> Result<import::StagedPet, String> {
    import::stage_from_zip_in(&root(), std::path::Path::new(&path))
}

#[tauri::command]
pub async fn pet_stage_from_codex(codex_id: String) -> Result<import::StagedPet, String> {
    let codex = dirs::home_dir().unwrap_or_default().join(".codex").join("pets");
    import::stage_from_codex_in(&root(), &codex, &codex_id)
}

#[tauri::command]
pub async fn pet_stage_from_petdex(url: String) -> Result<import::StagedPet, String> {
    petdex::stage_from_url(&root(), &url).await
}

#[tauri::command]
pub async fn pet_stage_audio(
    staging_id: String,
    src_paths: Vec<String>,
    group: String,
) -> Result<Vec<import::StagedVoiceFile>, String> {
    import::stage_audio_in(&root(), &staging_id, &src_paths, &group)
}

#[tauri::command]
pub async fn pet_remove_staged_audio(staging_id: String, rel: String) -> Result<(), String> {
    import::remove_audio_in(&root(), &staging_id, &rel, true)
}

#[tauri::command]
pub async fn pet_finalize_import(
    staging_id: String,
    name: String,
    manifest: manifest::PetManifest,
) -> Result<scan::PetSummary, String> {
    import::finalize_in(&root(), &staging_id, &name, manifest)
}

#[tauri::command]
pub async fn pet_cancel_import(staging_id: String) -> Result<(), String> {
    import::cancel_in(&root(), &staging_id)
}

#[tauri::command]
pub async fn pet_update_manifest(
    id: String,
    mut manifest: manifest::PetManifest,
    backup: bool,
) -> Result<(), String> {
    manifest.id = id.clone();
    manifest::write_with_backup(&pet::pet_dir(&root(), &id), &manifest, backup)
}

#[tauri::command]
pub async fn pet_rename_pet(old_id: String, new_id: String) -> Result<(), String> {
    pet::rename_pet_in(&root(), &old_id, &new_id)
}

#[tauri::command]
pub async fn pet_delete_pet(id: String) -> Result<(), String> {
    pet::delete_pet_in(&root(), &id)
}

#[tauri::command]
pub async fn pet_add_voice_files(
    id: String,
    src_paths: Vec<String>,
    group: String,
) -> Result<Vec<import::StagedVoiceFile>, String> {
    import::add_voice_files_in(&root(), &id, &src_paths, &group)
}

#[tauri::command]
pub async fn pet_remove_voice_file(id: String, rel: String) -> Result<(), String> {
    import::remove_audio_in(&root(), &id, &rel, false)
}

#[tauri::command]
pub async fn pet_reveal_folder(id: String) -> Result<(), String> {
    let dir = pet::pet_dir(&root(), &id);
    tauri_plugin_opener::open_path(dir.to_string_lossy().to_string(), None::<&str>)
        .map_err(|e| e.to_string())
}
