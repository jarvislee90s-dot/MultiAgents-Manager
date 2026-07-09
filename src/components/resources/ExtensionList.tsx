import { useState, useEffect, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Scan, LayoutGrid, List } from "lucide-react";
import type { ExtensionWithAssignments } from "@/types/extension";
import { ResourceByKindView } from "./ResourceByKindView";
import { ResourceByToolView } from "./ResourceByToolView";
import { ImportDialog } from "./ImportDialog";
import { PresetList } from "../presets/PresetList";

export function ExtensionList() {
  const [view, setView] = useState<"byKind" | "byTool">("byKind");
  const [extensions, setExtensions] = useState<ExtensionWithAssignments[]>([]);
  const [showImport, setShowImport] = useState(false);

  const load = useCallback(async () => {
    try {
      const data = await invoke<ExtensionWithAssignments[]>("list_extensions_with_assignments");
      setExtensions(data);
    } catch (e) {
      console.error("Failed to load extensions:", e);
    }
  }, []);

  useEffect(() => {
    load();
  }, []);

  const handleToggleMcp = async (mcpName: string, toolId: string, enabled: boolean) => {
    try {
      await invoke("toggle_mcp_for_tool", { mcpName, toolId, enabled });
      toast.success(`${mcpName} 已${enabled ? "启用" : "禁用"}`);
      load();
    } catch (e) {
      toast.error(`操作失败: ${e}`);
    }
  };

  const handleTogglePlugin = async (pluginName: string, toolId: string, enabled: boolean, kind: string) => {
    try {
      await invoke("toggle_plugin_for_tool", { pluginName, toolId, enabled, kind });
      toast.success(`${pluginName} 已${enabled ? "启用" : "禁用"}`);
      load();
    } catch (e) {
      toast.error(`操作失败: ${e}`);
    }
  };

  return (
    <div className="space-y-4">
      {/* Toolbar */}
      <div className="flex items-center justify-between">
        <div className="flex gap-1">
          <Button
            size="sm"
            variant={view === "byKind" ? "default" : "outline"}
            className="h-7 px-2 text-[10px]"
            onClick={() => setView("byKind")}
          >
            <List className="mr-1 h-3 w-3" />
            按资源
          </Button>
          <Button
            size="sm"
            variant={view === "byTool" ? "default" : "outline"}
            className="h-7 px-2 text-[10px]"
            onClick={() => setView("byTool")}
          >
            <LayoutGrid className="mr-1 h-3 w-3" />
            按工具
          </Button>
        </div>
        <Button
          size="sm"
          variant="outline"
          className="h-7 px-2 text-[10px]"
          onClick={() => setShowImport(true)}
        >
          <Scan className="mr-1 h-3 w-3" />
          扫描原生资源
        </Button>
      </div>

      {/* View content */}
      {view === "byKind" ? (
        <ResourceByKindView
          extensions={extensions}
          onToggleMcp={handleToggleMcp}
          onTogglePlugin={handleTogglePlugin}
        />
      ) : (
        <ResourceByToolView />
      )}

      {/* Presets */}
      <PresetList extensions={extensions} />

      {/* Import dialog */}
      <ImportDialog
        open={showImport}
        onClose={() => setShowImport(false)}
        onImported={load}
      />
    </div>
  );
}
