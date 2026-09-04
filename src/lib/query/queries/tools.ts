import { useQuery } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";

/** 后端 EnabledTool（serde camelCase）：勾选状态驱动的工具列下发项 */
export interface EnabledTool {
  id: string;
  label: string;
}

export const ENABLED_TOOLS_KEY = ["enabled-tools"] as const;

/** 启用工具列表（后端 list_enabled_tools，仅勾选工具，TOOL_IDS 顺序） */
export function useEnabledToolsQuery() {
  return useQuery({
    queryKey: ENABLED_TOOLS_KEY,
    queryFn: async () => {
      // 兜底 null：浏览器/Playwright mock 下未注册命令会 resolve null
      return (await invoke<EnabledTool[]>("list_enabled_tools")) ?? [];
    },
    staleTime: 5000,
  });
}
