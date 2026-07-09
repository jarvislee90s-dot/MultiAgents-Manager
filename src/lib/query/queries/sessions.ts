import { useQuery } from "@tanstack/react-query";
import { getAllSessions } from "@/lib/api/session";
import { POLL_INTERVAL } from "@/config/constants";
export function useSessionsQuery() {
  return useQuery({ queryKey: ["sessions"], queryFn: getAllSessions, refetchInterval: POLL_INTERVAL, refetchIntervalInBackground: false, staleTime: 1000 });
}
