import { useState, useCallback, useEffect } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Scan, Import, FolderOpen } from "lucide-react";
import { ToolIcon } from "@/components/common/ToolIcon";
import { detectDuplicateSkills, cleanupDuplicateSkills } from "@/lib/api/resource";
import { useEnabledToolsQuery } from "@/lib/query/queries/tools";
import type { NativeExtension, ToolResources, ImportStats } from "@/types/extension";

function formatSkillName(name: string): string {
  return name.includes("/") ? name.replace("/", ": ") : name;
}

export function ResourceByToolView() {
  const { t } = useTranslation();
  // 工具列由后端下发（勾选状态驱动，W5）
  const { data: tools = [] } = useEnabledToolsQuery();
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

  // 挂载时（及工具勾选变化后）自动加载所有启用工具的已有全局资源
  useEffect(() => {
    tools.forEach((tool) => {
      loadToolResources(tool.id);
    });
  }, [tools, loadToolResources]);

  const handleScan = async (toolId: string) => {
    setScanning((prev) => ({ ...prev, [toolId]: true }));
    try {
      const native = await invoke<NativeExtension[]>("scan_native_resources", { toolId });
      if (native.length > 0) {
        toast.info(
          t("resources.foundNative", {
            tool: tools.find((tool) => tool.id === toolId)?.label,
            n: native.length,
          })
        );
      } else {
        toast.info(t("resources.noNewNative"));
      }
      await loadToolResources(toolId);
    } catch (e) {
      toast.error(t("common.scanFailed", { error: e }));
    } finally {
      setScanning((prev) => ({ ...prev, [toolId]: false }));
    }
  };

  const handleImport = async (toolId: string, item: NativeExtension) => {
    try {
      const result = await invoke<ImportStats>("import_native_resources", {
        items: [[item.sourcePath, item.name, toolId]],
      });
      if (result.imported > 0) {
        toast.success(t("resources.importSuccess", { name: item.name }));
        await loadToolResources(toolId);
        await loadDuplicates(toolId);
      } else {
        toast.info(t("resources.alreadyExists", { name: item.name }));
      }
    } catch (e) {
      toast.error(t("resources.importFailed", { error: e }));
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

  // 挂载时（及工具勾选变化后）检测所有启用工具的重复
  useEffect(() => {
    tools.forEach((tool) => {
      loadDuplicates(tool.id);
    });
  }, [tools, loadDuplicates]);

  const handleCleanupSingle = async (toolId: string, name: string) => {
    try {
      await cleanupDuplicateSkills(toolId, [name]);
      toast.success(t("resources.cleanedOne", { name }));
      await loadDuplicates(toolId);
      await loadToolResources(toolId);
    } catch (e) {
      toast.error(t("resources.cleanupFailed", { error: e }));
    }
  };

  const handleCleanupAll = async (toolId: string) => {
    const dups = duplicates[toolId] || [];
    if (dups.length === 0) return;
    try {
      await cleanupDuplicateSkills(toolId, dups);
      toast.success(t("resources.cleanedCount", { n: dups.length }));
      await loadDuplicates(toolId);
      await loadToolResources(toolId);
    } catch (e) {
      toast.error(t("resources.cleanupFailed", { error: e }));
    }
  };

  const handleOpenDir = async (toolId: string) => {
    try {
      const path = await invoke<string>("open_tool_resource", { toolId, kind: "skill" });
      toast.success(path);
    } catch (e) {
      toast.error(t("common.operationFailed", { error: e }));
    }
  };

  return (
    <div className="space-y-4">
      {tools.map((tool) => (
        <div key={tool.id} className="rounded border p-3">
          <div className="mb-2 flex items-center justify-between">
            <h3 className="flex items-center gap-2 text-sm font-semibold">
              <ToolIcon toolId={tool.id} size={18} />
              {tool.label}
            </h3>
            <div className="flex gap-1">
              <Button
                size="sm"
                variant="ghost"
                className="h-6 px-2 text-[10px]"
                title={t("resources.openToolDir", { tool: tool.label, kind: "skills" })}
                onClick={() => handleOpenDir(tool.id)}
              >
                <FolderOpen className="mr-1 h-3 w-3" />
                {t("resources.openDir")}
              </Button>
              <Button
                size="sm"
                variant="ghost"
                className="h-6 px-2 text-[10px]"
                onClick={() => handleScan(tool.id)}
                disabled={scanning[tool.id]}
              >
                <Scan className={`mr-1 h-3 w-3 ${scanning[tool.id] ? "animate-spin" : ""}`} />
                {t("common.scan")}
              </Button>
            </div>
          </div>

          <ToolResourceList
            toolId={tool.id}
            resources={toolResources[tool.id]}
            onImport={handleImport}
          />

          {/* 重复 skill 清理区 */}
          {(duplicates[tool.id]?.length ?? 0) > 0 && (
            <div className="mt-2 rounded border border-orange-500/30 bg-orange-500/5 p-2">
              <div className="mb-1 flex items-center justify-between">
                <span className="text-xs font-medium text-orange-600">
                  {t("resources.duplicatesWarning", { n: duplicates[tool.id]!.length })}
                </span>
                <Button
                  size="sm"
                  variant="ghost"
                  className="h-5 px-1 text-[10px] text-orange-600"
                  onClick={() => handleCleanupAll(tool.id)}
                >
                  {t("common.cleanupAll")}
                </Button>
              </div>
              <div className="space-y-0.5">
                {duplicates[tool.id]!.map((name) => (
                  <div key={name} className="flex items-center justify-between text-xs">
                    <span className="text-muted-foreground">{formatSkillName(name)}</span>
                    <Button
                      size="sm"
                      variant="ghost"
                      className="h-5 px-1 text-[10px]"
                      onClick={() => handleCleanupSingle(tool.id, name)}
                    >
                      {t("common.cleanup")}
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
  const { t } = useTranslation();
  if (!resources) {
    return <div className="text-muted-foreground py-2 text-xs">{t("common.loading")}</div>;
  }

  const globalSkills = resources.global.filter((e) => e.kind === "skill");
  const nativeSkills = resources.native.filter((n) => n.kind === "skill");
  const globalMcps = resources.global.filter((e) => e.kind === "mcp");
  const globalPlugins = resources.global.filter((e) => e.kind === "plugin");

  return (
    <div className="space-y-2">
      {/* Skills */}
      <div>
        <h4 className="text-muted-foreground mb-1 text-xs font-medium">
          Skills ({globalSkills.length + nativeSkills.length})
        </h4>
        <div className="space-y-1">
          {globalSkills.map((s) => (
            <div
              key={s.id}
              className="bg-accent/50 flex items-center justify-between rounded px-2 py-1 text-xs"
            >
              <span>
                {formatSkillName(s.name)}{" "}
                <span className="text-green-600">{t("resources.inRepo")}</span>
              </span>
            </div>
          ))}
          {nativeSkills.map((s) => (
            <div
              key={s.id}
              className="bg-muted flex items-center justify-between rounded px-2 py-1 text-xs"
            >
              <span>
                {formatSkillName(s.name)}{" "}
                <span className="text-orange-500">{t("resources.nativeTag")}</span>
              </span>
              <Button
                size="sm"
                variant="ghost"
                className="h-5 px-1 text-[10px]"
                onClick={() => onImport(toolId, s)}
              >
                <Import className="h-3 w-3" />
                {t("common.import")}
              </Button>
            </div>
          ))}
        </div>
      </div>

      {/* MCP */}
      <div>
        <h4 className="text-muted-foreground mb-1 text-xs font-medium">
          MCP ({globalMcps.length})
        </h4>
        <div className="space-y-1">
          {globalMcps.length === 0 ? (
            <div className="text-muted-foreground px-2 py-1 text-[11px]">
              {t("resources.noMcpHint")}
            </div>
          ) : (
            globalMcps.map((m) => (
              <div
                key={m.id}
                className="bg-accent/50 flex items-center justify-between rounded px-2 py-1 text-xs"
              >
                <span>
                  {m.name} <span className="text-green-600">{t("resources.inRepo")}</span>
                </span>
              </div>
            ))
          )}
        </div>
      </div>

      {/* Plugins */}
      <div>
        <h4 className="text-muted-foreground mb-1 text-xs font-medium">
          Plugins ({globalPlugins.length})
        </h4>
        <div className="space-y-1">
          {globalPlugins.length === 0 ? (
            <div className="text-muted-foreground px-2 py-1 text-[11px]">
              {t("resources.noPluginHint")}
            </div>
          ) : (
            globalPlugins.map((p) => (
              <div
                key={p.id}
                className="bg-accent/50 flex items-center justify-between rounded px-2 py-1 text-xs"
              >
                <span>
                  {p.name} <span className="text-green-600">{t("resources.inRepo")}</span>
                </span>
              </div>
            ))
          )}
        </div>
      </div>
    </div>
  );
}
