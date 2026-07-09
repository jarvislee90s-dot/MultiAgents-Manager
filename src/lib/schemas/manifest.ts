import { z } from "zod";

export const PermissionSchema = z.enum([
  "filesystem.read", "filesystem.write", "network", "shell",
  "env.read", "settings.write", "symlink.create",
]);
export const KindSchema = z.enum(["skill", "mcp", "plugin"]);

export const CompatibilityEntrySchema = z.object({
  tool: z.string(),
  minVersion: z.string().optional(),
  mcpFormat: z.enum(["json", "toml", "jsonc"]).optional(),
  subAgentSupport: z.boolean().optional(),
  notes: z.string().optional(),
});

const commonFields = {
  id: z.string().regex(/^[a-zA-Z0-9._-]+$/),
  name: z.string().min(1),
  version: z.string().regex(/^\d+\.\d+\.\d+/),
  kind: KindSchema,
  description: z.string().optional(),
  permissions: z.array(PermissionSchema).optional(),
  compatibility: z.array(CompatibilityEntrySchema).optional(),
};

export const SkillManifestSchema = z.object({
  ...commonFields, kind: z.literal("skill"),
  skill: z.object({ entry: z.string(), includes: z.array(z.string()).optional() }),
});
export const McpManifestSchema = z.object({
  ...commonFields, kind: z.literal("mcp"),
  mcp: z.object({ command: z.string(), args: z.array(z.string()).optional(), formats: z.array(z.enum(["json","toml","jsonc"])).optional() }),
});
export const PluginManifestSchema = z.object({
  ...commonFields, kind: z.literal("plugin"),
  plugin: z.object({ entry: z.string(), type: z.enum(["file","config","mixed"]), configTemplate: z.string().optional() }),
});

export const ManifestSchema = z.discriminatedUnion("kind", [SkillManifestSchema, McpManifestSchema, PluginManifestSchema]);
export type Manifest = z.infer<typeof ManifestSchema>;
export type Permission = z.infer<typeof PermissionSchema>;

export const PERMISSION_RISK: Record<string, "low" | "medium" | "high"> = {
  "filesystem.read": "low", "filesystem.write": "medium", "network": "medium",
  "shell": "high", "env.read": "medium", "settings.write": "high", "symlink.create": "low",
};
export const PERMISSION_DESCRIPTION: Record<string, string> = {
  "filesystem.read": "读取文件", "filesystem.write": "写入文件", "network": "发起网络请求",
  "shell": "执行 shell 命令（高风险）", "env.read": "读取环境变量",
  "settings.write": "写入工具配置文件（高风险）", "symlink.create": "创建符号链接",
};
