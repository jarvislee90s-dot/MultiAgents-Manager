import { useEffect } from "react";
import { useSessionsQuery } from "@/lib/query/queries/sessions";
import { useSessionStore } from "@/stores/sessionStore";

export function useSessions() {
  const query = useSessionsQuery();
  const setSessions = useSessionStore((s) => s.setSessions);
  const setLoading = useSessionStore((s) => s.setLoading);

  // React Query 轮询获取数据后，同步到 Zustand store
  // home.tsx 和 useNotification 都从 store 读取
  useEffect(() => {
    setLoading(query.isLoading);
    if (query.data) {
      setSessions(query.data);
    }
  }, [query.data, query.isLoading, setSessions, setLoading]);

  return query;
}
