import { invoke } from "@tauri-apps/api/core";
export async function toggleMcpForTool(mcpName: string, toolId: string, enabled: boolean) { return await invoke("toggle_mcp_for_tool", { mcpName, toolId, enabled }); }
export async function readMcpServers(toolId: string) { return await invoke("read_mcp_servers", { toolId }); }
export async function writeMcpServer(toolId: string, mcpName: string, command: string, args: string[], env: Record<string, string>) { return await invoke("write_mcp_server", { toolId, mcpName, command, args, env }); }
export async function removeMcpServer(toolId: string, mcpName: string) { return await invoke("remove_mcp_server", { toolId, mcpName }); }
