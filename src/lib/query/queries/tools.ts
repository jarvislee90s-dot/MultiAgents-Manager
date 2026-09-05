import { useQuery } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import type { AgentType } from "@/types/session";

/** 后端 EnabledTool（serde camelCase）：勾选状态驱动的工具列下发项。
 * id 类型派生自 AgentType 联合（P2-9）：后端 TOOL_IDS 与前端工具 id 同源，
 * 声音配置等消费方可直接以 tool.id 索引，无需 as keyof 强转 */
export interface EnabledTool {
  id: AgentType;
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
    // P2-8：查询刷新/重新挂载期间沿用上一次数据，避免「工具全部停用」的瞬时误渲染
    placeholderData: (prev) => prev,
  });
}
