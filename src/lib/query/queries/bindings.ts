// 资源独占绑定查询（React Query）
import { useQuery } from "@tanstack/react-query";
import { listResourceBindings } from "@/lib/api/preset";

export const BINDINGS_KEY = ["resource-bindings"] as const;

export const useResourceBindingsQuery = () =>
  useQuery({ queryKey: BINDINGS_KEY, queryFn: listResourceBindings, staleTime: 10000 });
