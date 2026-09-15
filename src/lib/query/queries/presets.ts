// 预设组 / 激活预设查询（React Query）
import { useQuery } from "@tanstack/react-query";
import { listActivePresets, listPresets } from "@/lib/api/preset";

export const PRESETS_KEY = ["presets"] as const;
export const ACTIVE_PRESETS_KEY = ["active-presets"] as const;

export const usePresetsQuery = () =>
  useQuery({ queryKey: PRESETS_KEY, queryFn: listPresets, staleTime: 5000 });

export const useActivePresetsQuery = () =>
  useQuery({ queryKey: ACTIVE_PRESETS_KEY, queryFn: listActivePresets, staleTime: 5000 });
