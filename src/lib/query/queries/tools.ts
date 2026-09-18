import { useQuery } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import type { AgentType } from "@/types/session";

/** 后端 EnabledTool（serde camelCase）：勾选状态驱动的工具列下发项。
 * id 类型派生自 AgentType 联合（P2-9）：后端 TOOL_IDS 与前端工具 id 同源，
 * 声音配置等消费方可直接以 tool.id 索引，无需 as keyof 强转 */
export interface EnabledTool {
  id: AgentType;
  label: string;
  /** 资源能力标志：dsh 的 skill 启停写通道暂未开放；mcp/plugin 按工具实际能力 */
  skillToggleSupported: boolean;
  mcpSupported: boolean;
  pluginSupported: boolean;
}

export const ENABLED_TOOLS_KEY = ["enabled-tools"] as const;

/** 技能/资源管理 UI 的显示名覆盖（用户裁决 2026-09-16）：技能域不区分 Codex APP/CLI
 *  → 统一「Codex」；DSH 统一大写。按 id 映射（而非 label 文本），后端改名不受影响。
 *  会话卡片的 Codex APP/CLI 区分走 agentBadge（agentType+form），不经此处 */
const TOOL_LABEL_OVERRIDES: Partial<Record<AgentType, string>> = {
  codex: "Codex",
  dsh: "DSH",
};

/** 启用工具列表（后端 list_enabled_tools，仅勾选工具，TOOL_IDS 顺序） */
export function useEnabledToolsQuery() {
  return useQuery({
    queryKey: ENABLED_TOOLS_KEY,
    queryFn: async () => {
      // 兜底 null：浏览器/Playwright mock 下未注册命令会 resolve null
      const tools = (await invoke<EnabledTool[]>("list_enabled_tools")) ?? [];
      return tools.map((t) => ({ ...t, label: TOOL_LABEL_OVERRIDES[t.id] ?? t.label }));
    },
    staleTime: 5000,
    // P2-8：查询刷新/重新挂载期间沿用上一次数据，避免「工具全部停用」的瞬时误渲染
    placeholderData: (prev) => prev,
  });
}
