// 资源独占绑定 / 常驻锁查询（React Query）
import { useQuery } from "@tanstack/react-query";
import { listResourceBindings, listToolResidents } from "@/lib/api/preset";

export const BINDINGS_KEY = ["resource-bindings"] as const;

export const useResourceBindingsQuery = () =>
  useQuery({ queryKey: BINDINGS_KEY, queryFn: listResourceBindings, staleTime: 10000 });

// 常驻锁：toolId → 该工具下常驻资源 extensionId 列表（实际键 = 前缀 + toolId）
export const TOOL_RESIDENTS_KEY = ["tool-residents"] as const;

export const useToolResidentsQuery = (toolId: string) =>
  useQuery({
    queryKey: [...TOOL_RESIDENTS_KEY, toolId],
    queryFn: () => listToolResidents(toolId),
    staleTime: 10000,
  });
