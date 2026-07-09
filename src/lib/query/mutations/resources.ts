import { useMutation, useQueryClient } from "@tanstack/react-query";
import { toggleMcpForTool } from "@/lib/api/mcp";
export function useToggleMcpMutation() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ mcpName, toolId, enabled }: { mcpName: string; toolId: string; enabled: boolean }) => toggleMcpForTool(mcpName, toolId, enabled),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["extensions"] }),
  });
}
