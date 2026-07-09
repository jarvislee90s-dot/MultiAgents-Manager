import { useQuery } from "@tanstack/react-query";
import { listPresets } from "@/lib/api/preset";
export function usePresetsQuery() {
  return useQuery({ queryKey: ["presets"], queryFn: listPresets, staleTime: 10000 });
}
