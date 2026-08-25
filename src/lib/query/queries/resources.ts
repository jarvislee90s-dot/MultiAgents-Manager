import { useQuery } from "@tanstack/react-query";
import { listSsotResources } from "@/lib/api/resource";
import type { SsotResources } from "@/types/extension";

export const SSOT_RESOURCES_KEY = ["ssotResources"] as const;

export function useSsotResourcesQuery() {
  return useQuery<SsotResources>({
    queryKey: SSOT_RESOURCES_KEY,
    queryFn: listSsotResources,
    staleTime: 5000,
  });
}
