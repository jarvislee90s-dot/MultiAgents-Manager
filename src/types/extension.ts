export interface AssignmentSummary {
  agentToolId: string;
  enabled: boolean;
  linkStatus: string;
}

export interface ExtensionWithAssignments {
  id: string;
  kind: string;
  name: string;
  description: string | null;
  sourcePath: string;
  sourceTool: string | null;
  suite: string | null;
  tags: string | null;
  /** 原生（未纳管）资源：工具私有预设编辑弹窗「原生技能」分组的数据源（T7） */
  isNative: boolean;
  assignments: AssignmentSummary[];
}

export interface NativeExtension {
  id: string;
  kind: string;
  name: string;
  description: string | null;
  sourcePath: string;
  sourceTool: string;
  detectedAt: string;
  imported: boolean;
}

export interface ToolResources {
  global: ExtensionWithAssignments[];
  native: NativeExtension[];
}

export interface CompatibilityReport {
  compatible: CompatibleItem[];
  incompatible: IncompatibleItem[];
}

export interface CompatibleItem {
  id: string;
  name: string;
  kind: string;
}

export interface IncompatibleItem {
  id: string;
  name: string;
  kind: string;
  reason: string;
}

export interface McpServerConfig {
  command: string;
  args: string[];
  env: Record<string, string>;
}

export interface McpServer {
  name: string;
  config: McpServerConfig;
}

/** frontmatter 专属预填建议（Rust FrontmatterSuggestion，serde camelCase）。
 *  自动识别只建议不强制，真值永远是 resource_bindings 手动值（spec §6） */
export interface FrontmatterSuggestion {
  extensionId: string;
  tools: string[];
}

export interface ImportStats {
  imported: number;
  newlyAdded: number;
  skippedDup: number;
  sourceCounts: [string, number][];
  /** 仅手动导入路径（import_native_resources）取首个命中项；自动导入（rescan）恒 null */
  suggestion: FrontmatterSuggestion | null;
}

/** install_skill 返回（Rust commands::skill::ImportOutcome）：成功标记 + 预填建议 */
export interface ImportOutcome {
  success: boolean;
  suggestion: FrontmatterSuggestion | null;
}

export interface SsotResource {
  name: string;
  kind: string;
  enabledTools: string[];
  brokenTools?: string[];
  /** plugin 子类型（file | config），仅 kind === "plugin" 时由后端返回 */
  pluginType?: string;
  /** MCP 存储 JSON 带 enable:false（工具侧停用标记原样入库，M7 口径），
   *  仅 kind === "mcp" 且为 true 时返回，UI 标记「源已停用」 */
  sourceDisabled?: boolean;
}

export interface SsotResources {
  skills: SsotResource[];
  mcp: SsotResource[];
  plugins: SsotResource[];
}

/** 资源独占绑定（exclusiveTools 为逗号 join 的工具 id 串，与后端存储同形） */
export interface ResourceBinding {
  extensionId: string;
  exclusiveTools: string;
  reason: string | null;
  updatedAt: string;
}

/** 工具当前激活的预设（开关状态批量下发项） */
export interface ActivePreset {
  toolId: string;
  presetId: string;
}
