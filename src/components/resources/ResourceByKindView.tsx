import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { toast } from "sonner";
import { ToolIcon } from "@/components/common/ToolIcon";
import { Button } from "@/components/ui/button";
import { Package, Link2, Plug, Info } from "lucide-react";
import { listSsotResources } from "@/lib/api/resource";
import type { SsotResources } from "@/types/extension";

const TOOLS = [
  { id: "claude", label: "Claude" },
  { id: "codex", label: "Codex" },
  { id: "opencode", label: "OpenCode" },
  { id: "openclaw", label: "OpenClaw" },
];

export function ResourceByKindView() {
  const [resources, setResources] = useState<SsotResources | null>(null);
  const [search, setSearch] = useState("");

  useEffect(() => {
    listSsotResources().then(setResources).catch(console.error);
  }, []);

  if (!resources) {
    return <div className="text-muted-foreground py-4 text-xs">加载中…</div>;
  }

  const filteredSkills = resources.skills.filter((s) => {
    if (!search.trim()) return true;
    const q = search.toLowerCase();
    return [s.name, ...s.enabledTools].some((x) => x.toLowerCase().includes(q));
  });

  const handleToggleMcp = async (name: string, toolId: string, enabled: boolean) => {
    try {
      await invoke("toggle_mcp_for_tool", { mcpName: name, toolId, enabled });
      toast.success(`${name} 已${enabled ? "启用" : "禁用"}`);
      const fresh = await listSsotResources();
      setResources(fresh);
    } catch (e) {
      toast.error(`操作失败: ${e}`);
    }
  };

  const handleTogglePlugin = async (name: string, toolId: string, enabled: boolean, kind: string) => {
    try {
      await invoke("toggle_plugin_for_tool", { pluginName: name, toolId, enabled, kind });
      toast.success(`${name} 已${enabled ? "启用" : "禁用"}`);
      const fresh = await listSsotResources();
      setResources(fresh);
    } catch (e) {
      toast.error(`操作失败: ${e}`);
    }
  };

  return (
    <div className="rounded-lg border bg-card p-4">
      <h3 className="mb-3 text-sm font-semibold">MAM 仓库</h3>

      {/* Skills */}
      <div className="mb-4">
        <div className="mb-2 flex items-center justify-between gap-2">
          <h4 className="flex items-center gap-2 text-sm font-semibold">
            <Package className="h-4 w-4" />
            Skill ({filteredSkills.length})
          </h4>
          <input
            type="text"
            placeholder="搜索 skill..."
            value={search}
            onChange={(e) => setSearch(e.currentTarget.value)}
            className="h-7 w-40 rounded border px-2 text-xs"
          />
        </div>
        {filteredSkills.length === 0 ? (
          <div className="text-muted-foreground flex items-center gap-2 py-4 text-xs">
            <Info className="h-3.5 w-3.5" />
            暂无 skill。点击"扫描原生资源"导入。
          </div>
        ) : (
          <div className="space-y-1">
            {filteredSkills.map((skill) => (
              <div key={skill.name} className="flex items-center justify-between rounded border p-2 text-sm">
                <span className="font-medium">{skill.name}</span>
                <div className="flex gap-1">
                  {TOOLS.map((tool) => {
                    const enabled = skill.enabledTools.includes(tool.id);
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
      <div className="mb-4">
        <h4 className="mb-2 flex items-center gap-2 text-sm font-semibold">
          <Link2 className="h-4 w-4" />
          MCP 服务器 ({resources.mcp.length})
        </h4>
        {resources.mcp.length === 0 ? (
          <div className="text-muted-foreground flex items-center gap-2 py-4 text-xs">
            <Info className="h-3.5 w-3.5" />
            暂无 MCP 服务器
          </div>
        ) : (
          <div className="space-y-1">
            {resources.mcp.map((mcp) => (
              <div key={mcp.name} className="flex items-center justify-between rounded border p-2 text-sm">
                <span className="font-medium">{mcp.name}</span>
                <div className="flex gap-1">
                  {TOOLS.map((tool) => {
                    const enabled = mcp.enabledTools.includes(tool.id);
                    return (
                      <Button
                        key={tool.id}
                        variant={enabled ? "default" : "outline"}
                        size="sm"
                        className="h-6 px-2 text-[10px]"
                        onClick={() => handleToggleMcp(mcp.name, tool.id, !enabled)}
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
        <h4 className="mb-2 flex items-center gap-2 text-sm font-semibold">
          <Plug className="h-4 w-4" />
          插件 ({resources.plugins.length})
        </h4>
        {resources.plugins.length === 0 ? (
          <div className="text-muted-foreground flex items-center gap-2 py-4 text-xs">
            <Info className="h-3.5 w-3.5" />
            暂无插件
          </div>
        ) : (
          <div className="space-y-1">
            {resources.plugins.map((plugin) => (
              <div key={plugin.name} className="flex items-center justify-between rounded border p-2 text-sm">
                <span className="font-medium">{plugin.name}</span>
                <div className="flex gap-1">
                  {TOOLS.map((tool) => {
                    const enabled = plugin.enabledTools.includes(tool.id);
                    return (
                      <Button
                        key={tool.id}
                        variant={enabled ? "default" : "outline"}
                        size="sm"
                        className="h-6 px-2 text-[10px]"
                        onClick={() => handleTogglePlugin(plugin.name, tool.id, !enabled, "file")}
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