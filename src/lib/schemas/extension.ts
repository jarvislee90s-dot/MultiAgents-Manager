import { z } from "zod";
export const ExtensionSchema = z.object({
  id: z.string(), kind: z.enum(["skill", "mcp", "plugin"]), name: z.string(),
  description: z.string().optional(), sourcePath: z.string().optional(),
  sourceUrl: z.string().optional(), suite: z.string().optional(),
});
export type Extension = z.infer<typeof ExtensionSchema>;
