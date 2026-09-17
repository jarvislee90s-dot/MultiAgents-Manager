// 资源独占绑定 / 常驻锁查询（React Query）
import { useQuery } from "@tanstack/react-query";
import { listResourceBindings, listToolResidents } from "@/lib/api/preset";
import { listFrontmatterSuggestions } from "@/lib/api/resource";

export const BINDINGS_KEY = ["resource-bindings"] as const;

export const useResourceBindingsQuery = () =>
  useQuery({ queryKey: BINDINGS_KEY, queryFn: listResourceBindings, staleTime: 10000 });

// frontmatter 存量「待确认专属建议」（spec §6/§13，Task 17）：独立轻量 query，
// 不并入 preset-health 三源聚合——后端是独立扫描命令，30s staleTime 防抖即可；
// 「设为专属」写绑定后失效本查询 + BINDINGS_KEY
export const FM_SUGGESTIONS_KEY = ["fm-suggestions"] as const;

export const useFrontmatterSuggestionsQuery = () =>
  useQuery({
    queryKey: FM_SUGGESTIONS_KEY,
    queryFn: listFrontmatterSuggestions,
    staleTime: 30000,
  });

// 常驻锁：toolId → 该工具下常驻资源 extensionId 列表（实际键 = 前缀 + toolId）
export const TOOL_RESIDENTS_KEY = ["tool-residents"] as const;

export const useToolResidentsQuery = (toolId: string) =>
  useQuery({
    queryKey: [...TOOL_RESIDENTS_KEY, toolId],
    queryFn: () => listToolResidents(toolId),
    staleTime: 10000,
  });
