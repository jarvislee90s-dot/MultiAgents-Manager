import { useState, useCallback, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Scan, Import } from "lucide-react";
import { ToolIcon } from "@/components/common/ToolIcon";
import { detectDuplicateSkills, cleanupDuplicateSkills } from "@/lib/api/resource";
import type { NativeExtension, ToolResources, ImportStats } from "@/types/extension";

const TOOLS = [
  { id: "claude", label: "Claude Code" },
  { id: "codex", label: "Codex CLI" },
  { id: "opencode", label: "OpenCode" },
  { id: "openclaw", label: "OpenClaw" },
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

  // 挂载时自动加载所有工具的已有全局资源
  useEffect(() => {
    TOOLS.forEach((tool) => {
      loadToolResources(tool.id);
    });
  }, [loadToolResources]);

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

  const handleImport = async (toolId: string, item: NativeExtension) => {
    try {
      const result = await invoke<ImportStats>("import_native_resources", {
        items: [[item.sourcePath, item.name]],
      });
      if (result.imported > 0) {
        toast.success(`"${item.name}" 导入成功`);
        await loadToolResources(toolId);
        await loadDuplicates(toolId);
      } else {
        toast.info(`"${item.name}" 已存在`);
      }
    } catch (e) {
      toast.error(`导入失败: ${e}`);
    }
  };

  const [duplicates, setDuplicates] = useState<Record<string, string[]>>({});

  const loadDuplicates = useCallback(async (toolId: string) => {
    try {
      const dups = await detectDuplicateSkills(toolId);
      setDuplicates((prev) => ({ ...prev, [toolId]: dups }));
    } catch (e) {
      console.error(`Failed to detect duplicates for ${toolId}:`, e);
    }
  }, []);

  // 挂载时检测所有工具的重复
  useEffect(() => {
    TOOLS.forEach((tool) => {
      loadDuplicates(tool.id);
    });
  }, [loadDuplicates]);

  const handleCleanupSingle = async (toolId: string, name: string) => {
    try {
      await cleanupDuplicateSkills(toolId, [name]);
      toast.success(`"${name}" 已清理`);
      await loadDuplicates(toolId);
      await loadToolResources(toolId);
    } catch (e) {
      toast.error(`清理失败: ${e}`);
    }
  };

  const handleCleanupAll = async (toolId: string) => {
    const dups = duplicates[toolId] || [];
    if (dups.length === 0) return;
    try {
      await cleanupDuplicateSkills(toolId, dups);
      toast.success(`已清理 ${dups.length} 个重复 skill`);
      await loadDuplicates(toolId);
      await loadToolResources(toolId);
    } catch (e) {
      toast.error(`清理失败: ${e}`);
    }
  };

  return (
    <div className="space-y-4">
      {TOOLS.map((tool) => (
        <div key={tool.id} className="rounded border p-3">
          <div className="mb-2 flex items-center justify-between">
            <h3 className="flex items-center gap-2 text-sm font-semibold">
              <ToolIcon toolId={tool.id} size={18} />
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

          <ToolResourceList toolId={tool.id} resources={toolResources[tool.id]} onImport={handleImport} />

          {/* 重复 skill 清理区 */}
          {(duplicates[tool.id]?.length ?? 0) > 0 && (
            <div className="mt-2 rounded border border-orange-500/30 bg-orange-500/5 p-2">
              <div className="mb-1 flex items-center justify-between">
                <span className="text-xs font-medium text-orange-600">
                  ⚠ {duplicates[tool.id]!.length} 个重复 skill（SSOT 和本地同时存在）
                </span>
                <Button
                  size="sm"
                  variant="ghost"
                  className="h-5 px-1 text-[10px] text-orange-600"
                  onClick={() => handleCleanupAll(tool.id)}
                >
                  全部清理
                </Button>
              </div>
              <div className="space-y-0.5">
                {duplicates[tool.id]!.map((name) => (
                  <div key={name} className="flex items-center justify-between text-xs">
                    <span className="text-muted-foreground">{name}</span>
                    <Button
                      size="sm"
                      variant="ghost"
                      className="h-5 px-1 text-[10px]"
                      onClick={() => handleCleanupSingle(tool.id, name)}
                    >
                      清理
                    </Button>
                  </div>
                ))}
              </div>
            </div>
          )}
        </div>
      ))}
    </div>
  );
}

function ToolResourceList({
  toolId,
  resources,
  onImport,
}: {
  toolId: string;
  resources?: ToolResources;
  onImport: (toolId: string, item: NativeExtension) => void;
}) {
  if (!resources) {
    return <div className="text-muted-foreground py-2 text-xs">加载中…</div>;
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
              <Button
                size="sm"
                variant="ghost"
                className="h-5 px-1 text-[10px]"
                onClick={() => onImport(toolId, s)}
              >
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
                <Button
                  size="sm"
                  variant="ghost"
                  className="h-5 px-1 text-[10px]"
                  onClick={() => onImport(toolId, m)}
                >
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
                <Button
                  size="sm"
                  variant="ghost"
                  className="h-5 px-1 text-[10px]"
                  onClick={() => onImport(toolId, p)}
                >
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
