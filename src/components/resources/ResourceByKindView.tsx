import { useState } from "react";
import { ToolIcon } from "@/components/common/ToolIcon";
import { Button } from "@/components/ui/button";
import { Package, Link2, Plug, Info } from "lucide-react";
import type { ExtensionWithAssignments } from "@/types/extension";

const TOOLS = [
  { id: "claude", label: "Claude" },
  { id: "codex", label: "Codex" },
  { id: "opencode", label: "OpenCode" },
  { id: "openclaw", label: "OpenClaw" },
];

interface Props {
  extensions: ExtensionWithAssignments[];
  onToggleMcp: (name: string, toolId: string, enabled: boolean) => Promise<void>;
  onTogglePlugin: (name: string, toolId: string, enabled: boolean, kind: string) => Promise<void>;
}

export function ResourceByKindView({ extensions, onToggleMcp, onTogglePlugin }: Props) {
  const [search, setSearch] = useState("");

  const skillFilter = (e: ExtensionWithAssignments) => {
    if (!search.trim()) return true;
    const q = search.toLowerCase();
    return [e.name, e.description ?? "", e.sourceTool ?? "", e.suite ?? ""]
      .some((s) => s.toLowerCase().includes(q));
  };

  const skills = extensions.filter((e) => e.kind === "skill" && skillFilter(e));
  const mcps = extensions.filter((e) => e.kind === "mcp");
  const plugins = extensions.filter((e) => e.kind === "plugin");

  return (
    <div className="space-y-4">
      {/* Skills */}
      <div>
        <div className="mb-2 flex items-center justify-between gap-2">
          <h3 className="flex items-center gap-2 text-sm font-semibold">
            <Package className="h-4 w-4" />
            Skill ({skills.length})
          </h3>
          <input
            type="text"
            placeholder="搜索 skill..."
            value={search}
            onChange={(e) => setSearch(e.currentTarget.value)}
            className="h-7 w-40 rounded border px-2 text-xs"
          />
        </div>
        {skills.length === 0 ? (
          <div className="text-muted-foreground flex items-center gap-2 py-4 text-xs">
            <Info className="h-3.5 w-3.5" />
            暂无 skill。点击"扫描原生资源"导入。
          </div>
        ) : (
          <div className="space-y-1">
            {skills.map((skill) => (
              <div key={skill.id} className="flex items-center justify-between rounded border p-2 text-sm">
                <div>
                  <span className="font-medium">{skill.name}</span>
                  {skill.tags && (
                    <span className="text-muted-foreground ml-2 text-[10px]">
                      支持: {skill.tags}
                    </span>
                  )}
                </div>
                <div className="flex gap-1">
                  {TOOLS.map((tool) => {
                    const assignment = skill.assignments.find((a) => a.agentToolId === tool.id);
                    const enabled = assignment?.enabled ?? false;
                    return (
                      <Button
                        key={tool.id}
                        variant={enabled ? "default" : "outline"}
                        size="sm"
                        className="h-6 px-2 text-[10px]"
                        title={`${tool.label}: ${enabled ? "已启用" : "未启用"}`}
                      >
                        <ToolIcon toolId={tool.id} size={14} className="mr-1" />
                        {tool.label}
                      </Button>
                    );
                  })}
                </div>
              </div>
            ))}
          </div>
        )}
      </div>

      {/* MCP */}
      <div>
        <h3 className="mb-2 flex items-center gap-2 text-sm font-semibold">
          <Link2 className="h-4 w-4" />
          MCP 服务器 ({mcps.length})
        </h3>
        {mcps.length === 0 ? (
          <div className="text-muted-foreground flex items-center gap-2 py-4 text-xs">
            <Info className="h-3.5 w-3.5" />
            暂无 MCP 服务器
          </div>
        ) : (
          <div className="space-y-1">
            {mcps.map((mcp) => (
              <div key={mcp.id} className="flex items-center justify-between rounded border p-2 text-sm">
                <span className="font-medium">{mcp.name}</span>
                <div className="flex gap-1">
                  {TOOLS.map((tool) => {
                    const assignment = mcp.assignments.find((a) => a.agentToolId === tool.id);
                    const enabled = assignment?.enabled ?? false;
                    return (
                      <Button
                        key={tool.id}
                        variant={enabled ? "default" : "outline"}
                        size="sm"
                        className="h-6 px-2 text-[10px]"
                        onClick={() => onToggleMcp(mcp.name, tool.id, !enabled)}
                      >
                        <ToolIcon toolId={tool.id} size={14} className="mr-1" />
                        {tool.label}
                      </Button>
                    );
                  })}
                </div>
              </div>
            ))}
          </div>
        )}
      </div>

      {/* Plugins */}
      <div>
        <h3 className="mb-2 flex items-center gap-2 text-sm font-semibold">
          <Plug className="h-4 w-4" />
          插件 ({plugins.length})
        </h3>
        {plugins.length === 0 ? (
          <div className="text-muted-foreground flex items-center gap-2 py-4 text-xs">
            <Info className="h-3.5 w-3.5" />
            暂无插件
          </div>
        ) : (
          <div className="space-y-1">
            {plugins.map((plugin) => (
              <div key={plugin.id} className="flex items-center justify-between rounded border p-2 text-sm">
                <span className="font-medium">{plugin.name}</span>
                <div className="flex gap-1">
                  {TOOLS.map((tool) => {
                    const assignment = plugin.assignments.find((a) => a.agentToolId === tool.id);
                    const enabled = assignment?.enabled ?? false;
                    const pluginSubtype = plugin.tags || "file";
                    return (
                      <Button
                        key={tool.id}
                        variant={enabled ? "default" : "outline"}
                        size="sm"
                        className="h-6 px-2 text-[10px]"
                        onClick={() => onTogglePlugin(plugin.name, tool.id, !enabled, pluginSubtype)}
                      >
                        <ToolIcon toolId={tool.id} size={14} className="mr-1" />
                        {tool.label}
                      </Button>
                    );
                  })}
                </div>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
