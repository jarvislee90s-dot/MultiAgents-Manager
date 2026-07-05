import { create } from "zustand";
import type { SessionsResponse, Session } from "@/types/session";

interface SessionStore {
  sessions: Session[];
  totalCount: number;
  waitingCount: number;
  loading: boolean;
  setSessions: (response: SessionsResponse) => void;
  setLoading: (loading: boolean) => void;
}

export const useSessionStore = create<SessionStore>((set) => ({
  sessions: [],
  totalCount: 0,
  waitingCount: 0,
  loading: true,
  setSessions: (response) =>
    set({
      sessions: response.sessions,
      totalCount: response.totalCount,
      waitingCount: response.waitingCount,
      loading: false,
    }),
  setLoading: (loading) => set({ loading }),
}));
