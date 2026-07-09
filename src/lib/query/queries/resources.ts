import { useQuery } from "@tanstack/react-query";
import { listExtensionsWithAssignments } from "@/lib/api/resource";
export function useExtensionsQuery() {
  return useQuery({ queryKey: ["extensions"], queryFn: listExtensionsWithAssignments, staleTime: 5000 });
}
