import { z } from "zod";
export const PresetSchema = z.object({ id: z.string(), name: z.string(), items: z.array(z.tuple([z.string(), z.string()])) });
export type Preset = z.infer<typeof PresetSchema>;
