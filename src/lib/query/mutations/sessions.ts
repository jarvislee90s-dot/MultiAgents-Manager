import { useMutation, useQueryClient } from "@tanstack/react-query";
import { focusSession } from "@/lib/api/session";
export function useFocusSessionMutation() {
  const qc = useQueryClient();
  return useMutation({ mutationFn: (pid: number) => focusSession(pid), onSuccess: () => qc.invalidateQueries({ queryKey: ["sessions"] }) });
}
