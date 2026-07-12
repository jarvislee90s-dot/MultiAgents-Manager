// 截图命令

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenshotResult {
    pub success: bool,
    pub path: Option<String>,
    pub error: Option<String>,
}

#[tauri::command]
pub fn capture_window_screenshot(app: tauri::AppHandle) -> ScreenshotResult {
    #[cfg(target_os = "macos")]
    {
        use std::process::Command;
        let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
        let screenshot_dir = dirs::home_dir().unwrap_or_default().join(".mam").join("screenshots");
        if let Err(e) = std::fs::create_dir_all(&screenshot_dir) {
            return ScreenshotResult { success: false, path: None, error: Some(format!("创建截图目录失败: {}", e)) };
        }
        let screenshot_path = screenshot_dir.join(format!("screenshot_{}.png", timestamp));
        let path_str = screenshot_path.to_string_lossy().to_string();
        let window_id_output = Command::new("sh")
            .args(["-c", "osascript -e 'tell application \"System Events\" to get id of first process whose name contains \"multi-agents-manager\"' 2>/dev/null || echo ''"])
            .output();
        let capture_result = if let Ok(output) = window_id_output {
            let window_id = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !window_id.is_empty() && window_id.parse::<u32>().is_ok() {
                Command::new("screencapture").args(["-x", "-o", "-l", &window_id, &path_str]).output()
            } else {
                Command::new("screencapture").args(["-x", "-o", &path_str]).output()
            }
        } else {
            Command::new("screencapture").args(["-x", "-o", &path_str]).output()
        };
        match capture_result {
            Ok(output) if output.status.success() => {
                log::info!("截图已保存到: {}", path_str);
                ScreenshotResult { success: true, path: Some(path_str), error: None }
            }
            Ok(output) => {
                let error_msg = String::from_utf8_lossy(&output.stderr).to_string();
                ScreenshotResult { success: false, path: None, error: Some(format!("截图命令失败: {}", error_msg)) }
            }
            Err(e) => {
                ScreenshotResult { success: false, path: None, error: Some(format!("截图命令执行失败: {}", e)) }
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        ScreenshotResult { success: false, path: None, error: Some("截图功能目前仅支持 macOS".to_string()) }
    }
}

#[tauri::command]
pub fn list_screenshots() -> Vec<String> {
    let screenshot_dir = dirs::home_dir().unwrap_or_default().join(".mam").join("screenshots");
    if !screenshot_dir.exists() { return Vec::new(); }
    let mut paths: Vec<String> = std::fs::read_dir(&screenshot_dir)
        .ok()
        .map(|entries| entries.filter_map(|e| e.ok())
            .filter(|e| e.path().extension().and_then(|ext| ext.to_str()) == Some("png"))
            .map(|e| e.path().to_string_lossy().to_string())
            .collect())
        .unwrap_or_default();
    paths.sort_by(|a, b| {
        let meta_a = std::fs::metadata(a).ok();
        let meta_b = std::fs::metadata(b).ok();
        match (meta_a, meta_b) {
            (Some(a), Some(b)) => b.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH).cmp(
                &a.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH)),
            _ => std::cmp::Ordering::Equal,
        }
    });
    paths
}
