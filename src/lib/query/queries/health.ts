// 一致性体检查询（React Query；资源页卡片与设置页摘要共用同一 key，惯例照 bindings.ts/presets.ts）
import { useQuery } from "@tanstack/react-query";
import { getPresetHealth } from "@/lib/api/preset";
import type { PresetHealth } from "@/types/preset";

export const PRESET_HEALTH_KEY = ["preset-health"] as const;

// 体检无持久化、每次现算：非轮询数据，短 staleTime 防抖即可（照 bindings.ts 的 10s）
export function usePresetHealthQuery() {
  return useQuery<PresetHealth>({
    queryKey: PRESET_HEALTH_KEY,
    queryFn: getPresetHealth,
    staleTime: 10000,
  });
}
