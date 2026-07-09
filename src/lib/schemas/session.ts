import { z } from "zod";
export const SessionSchema = z.object({
  id: z.string(), tool: z.string(), projectPath: z.string().optional(),
  status: z.enum(["running", "waiting", "idle", "completed", "unknown"]),
  pid: z.number().optional(), cpuUsage: z.number().optional(),
  duration: z.number().optional(), lastMessage: z.string().optional(), gitBranch: z.string().optional(),
});
export const SessionsResponseSchema = z.object({ sessions: z.array(SessionSchema) });
export type Session = z.infer<typeof SessionSchema>;
export type SessionsResponse = z.infer<typeof SessionsResponseSchema>;
