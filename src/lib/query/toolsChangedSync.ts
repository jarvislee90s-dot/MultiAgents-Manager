// 跨窗口缓存失效（N2 根因修复）：设置窗口与主窗口是独立 WebView、各持 QueryClient，
// 设置页 applyChanges 的 invalidateQueries 无法触达主窗口；工具勾选保存后由后端
// `update_tool_settings` 广播 "tools-changed"，各窗口（通知浮窗/宠物除外）据此失效
// 本窗口 react-query 缓存（enabled-tools / ssot-resources / sessions 等）
import { listen } from "@tauri-apps/api/event";
import type { QueryClient } from "@tanstack/react-query";

/** 注册 tools-changed 监听：事件到达 → 全量失效本窗口查询缓存。
 * 返回 unlisten 函数（组件卸载时清理）；非 Tauri 环境（浏览器/mock）静默空操作 */
export async function setupToolsChangedListener(queryClient: QueryClient): Promise<() => void> {
  try {
    return await listen("tools-changed", () => {
      void queryClient.invalidateQueries();
    });
  } catch {
    return () => {};
  }
}
