// 截图服务 — 调用 Tauri 后端截图命令
import { invoke } from "@tauri-apps/api/core";

export interface ScreenshotResult {
  success: boolean;
  path: string | null;
  error: string | null;
}

/**
 * 捕获应用窗口截图
 */
export async function captureScreenshot(): Promise<ScreenshotResult> {
  try {
    const result = await invoke<ScreenshotResult>("capture_window_screenshot");
    return result;
  } catch (e) {
    return {
      success: false,
      path: null,
      error: String(e),
    };
  }
}

/**
 * 获取最近的截图列表
 */
export async function listScreenshots(): Promise<string[]> {
  try {
    const result = await invoke<string[]>("list_screenshots");
    return result;
  } catch (e) {
    console.error("Failed to list screenshots:", e);
    return [];
  }
}
