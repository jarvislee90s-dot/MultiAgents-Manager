import { useSessionsQuery } from "@/lib/query/queries/sessions";

export function useSessions() {
  return useSessionsQuery();
}
