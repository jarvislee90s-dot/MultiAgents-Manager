import { useState, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Scan, Import } from "lucide-react";
import type { NativeExtension, ToolResources } from "@/types/extension";

const TOOLS = [
  { id: "claude", label: "Claude Code", icon: "🤖" },
  { id: "codex", label: "Codex CLI", icon: "📝" },
  { id: "opencode", label: "OpenCode", icon: "🖥️" },
  { id: "openclaw", label: "OpenClaw", icon: "🦾" },
];

export function ResourceByToolView() {
  const [toolResources, setToolResources] = useState<Record<string, ToolResources>>({});
  const [scanning, setScanning] = useState<Record<string, boolean>>({});

  const loadToolResources = useCallback(async (toolId: string) => {
    try {
      const data = await invoke<ToolResources>("list_tool_resources", { toolId });
      setToolResources((prev) => ({ ...prev, [toolId]: data }));
    } catch (e) {
      console.error(`Failed to load resources for ${toolId}:`, e);
    }
  }, []);

  const handleScan = async (toolId: string) => {
    setScanning((prev) => ({ ...prev, [toolId]: true }));
    try {
      const native = await invoke<NativeExtension[]>("scan_native_resources", { toolId });
      if (native.length > 0) {
        toast.info(`${TOOLS.find(t => t.id === toolId)?.label} 发现 ${native.length} 个原生资源，点击导入`);
      } else {
        toast.info("未发现新的原生资源");
      }
      await loadToolResources(toolId);
    } catch (e) {
      toast.error(`扫描失败: ${e}`);
    } finally {
      setScanning((prev) => ({ ...prev, [toolId]: false }));
    }
  };

  return (
    <div className="space-y-4">
      {TOOLS.map((tool) => (
        <div key={tool.id} className="rounded border p-3">
          <div className="mb-2 flex items-center justify-between">
            <h3 className="flex items-center gap-2 text-sm font-semibold">
              <span>{tool.icon}</span>
              {tool.label}
            </h3>
            <Button
              size="sm"
              variant="ghost"
              className="h-6 px-2 text-[10px]"
              onClick={() => handleScan(tool.id)}
              disabled={scanning[tool.id]}
            >
              <Scan className={`mr-1 h-3 w-3 ${scanning[tool.id] ? "animate-spin" : ""}`} />
              扫描
            </Button>
          </div>

          <ToolResourceList toolId={tool.id} resources={toolResources[tool.id]} />
        </div>
      ))}
    </div>
  );
}

function ToolResourceList({ resources }: { toolId: string; resources?: ToolResources }) {
  if (!resources) {
    return <div className="text-muted-foreground py-2 text-xs">点击"扫描"加载资源</div>;
  }

  const globalSkills = resources.global.filter((e) => e.kind === "skill");
  const nativeSkills = resources.native.filter((n) => n.kind === "skill");
  const globalMcps = resources.global.filter((e) => e.kind === "mcp");
  const nativeMcps = resources.native.filter((n) => n.kind === "mcp");
  const globalPlugins = resources.global.filter((e) => e.kind === "plugin");
  const nativePlugins = resources.native.filter((n) => n.kind === "plugin");

  return (
    <div className="space-y-2">
      {/* Skills */}
      <div>
        <h4 className="text-xs font-medium text-muted-foreground mb-1">Skills ({globalSkills.length + nativeSkills.length})</h4>
        <div className="space-y-1">
          {globalSkills.map((s) => (
            <div key={s.id} className="flex items-center justify-between rounded bg-accent/50 px-2 py-1 text-xs">
              <span>{s.name} <span className="text-green-600">✓ 全局仓库</span></span>
            </div>
          ))}
          {nativeSkills.map((s) => (
            <div key={s.id} className="flex items-center justify-between rounded bg-muted px-2 py-1 text-xs">
              <span>{s.name} <span className="text-orange-500">⚠ 原生</span></span>
              <Button size="sm" variant="ghost" className="h-5 px-1 text-[10px]">
                <Import className="h-3 w-3" />
                导入
              </Button>
            </div>
          ))}
        </div>
      </div>

      {/* MCP */}
      {(globalMcps.length > 0 || nativeMcps.length > 0) && (
        <div>
          <h4 className="text-xs font-medium text-muted-foreground mb-1">MCP ({globalMcps.length + nativeMcps.length})</h4>
          <div className="space-y-1">
            {globalMcps.map((m) => (
              <div key={m.id} className="flex items-center justify-between rounded bg-accent/50 px-2 py-1 text-xs">
                <span>{m.name} <span className="text-green-600">✓</span></span>
              </div>
            ))}
            {nativeMcps.map((m) => (
              <div key={m.id} className="flex items-center justify-between rounded bg-muted px-2 py-1 text-xs">
                <span>{m.name} <span className="text-orange-500">⚠ 原生</span></span>
                <Button size="sm" variant="ghost" className="h-5 px-1 text-[10px]">
                  <Import className="h-3 w-3" />
                  导入
                </Button>
              </div>
            ))}
          </div>
        </div>
      )}

      {/* Plugins */}
      {(globalPlugins.length > 0 || nativePlugins.length > 0) && (
        <div>
          <h4 className="text-xs font-medium text-muted-foreground mb-1">Plugins ({globalPlugins.length + nativePlugins.length})</h4>
          <div className="space-y-1">
            {globalPlugins.map((p) => (
              <div key={p.id} className="flex items-center justify-between rounded bg-accent/50 px-2 py-1 text-xs">
                <span>{p.name} <span className="text-green-600">✓</span></span>
              </div>
            ))}
            {nativePlugins.map((p) => (
              <div key={p.id} className="flex items-center justify-between rounded bg-muted px-2 py-1 text-xs">
                <span>{p.name} <span className="text-orange-500">⚠ 原生</span></span>
                <Button size="sm" variant="ghost" className="h-5 px-1 text-[10px]">
                  <Import className="h-3 w-3" />
                  导入
                </Button>
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
