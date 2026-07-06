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
