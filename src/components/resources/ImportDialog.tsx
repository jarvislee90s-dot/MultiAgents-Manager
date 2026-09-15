import { useState, useEffect } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { toast } from "sonner";
import { formatInvokeError } from "@/lib/invokeError";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Checkbox } from "@/components/ui/checkbox";
import type { FrontmatterSuggestion, ImportStats, NativeExtension } from "@/types/extension";
import { ToolIcon } from "@/components/common/ToolIcon";
import { ImportSuggestionDialog } from "@/components/resources/ImportSuggestionDialog";
// review M2：工具列改后端下发（勾选状态驱动），与 PresetList/资源视图同源
import { useEnabledToolsQuery } from "@/lib/query/queries/tools";

interface Props {
  open: boolean;
  onClose: () => void;
  onImported: () => void;
}

export function ImportDialog({ open, onClose, onImported }: Props) {
  const { t } = useTranslation();
  const { data: enabledTools = [] } = useEnabledToolsQuery();
  const [resources, setResources] = useState<Record<string, NativeExtension[]>>({});
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [loading, setLoading] = useState(false);
  // frontmatter 专属预填建议（spec §6，Task 17）：导入成功后若有命中，
  // 关闭本弹窗的同时弹出确认 Dialog（2026-09-15 裁决：当场确认，非 toast）
  const [suggestion, setSuggestion] = useState<FrontmatterSuggestion | null>(null);

  useEffect(() => {
    if (open) {
      // 重开导入弹窗即新一轮导入：上一轮遗留的建议弹窗状态清零
      setSuggestion(null);
      loadAllResources();
    }
  }, [open]);

  const loadAllResources = async () => {
    setLoading(true);
    const all: Record<string, NativeExtension[]> = {};
    for (const tool of enabledTools) {
      try {
        const data = await invoke<NativeExtension[]>("scan_native_resources", { toolId: tool.id });
        all[tool.id] = data;
      } catch (e) {
        console.error(`Failed to scan ${tool.id}:`, e);
      }
    }
    setResources(all);
    setLoading(false);
  };

  const toggleSelect = (id: string) => {
    const next = new Set(selected);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    setSelected(next);
  };

  const selectAll = () => {
    const all = new Set<string>();
    Object.values(resources)
      .flat()
      .forEach((r) => all.add(r.id));
    setSelected(all);
  };

  const selectNone = () => {
    setSelected(new Set());
  };

  const handleImport = async () => {
    const items: [string, string, string][] = [];
    for (const toolId of Object.keys(resources)) {
      for (const res of resources[toolId]) {
        if (selected.has(res.id)) {
          items.push([res.sourcePath, res.name, toolId]);
        }
      }
    }

    if (items.length === 0) {
      toast.error(t("resources.selectAtLeastOne"));
      return;
    }

    try {
      const stats = await invoke<ImportStats>("import_native_resources", { items });
      toast.success(
        stats.skippedDup > 0
          ? t("resources.importedWithSkipped", { n: stats.imported, skipped: stats.skippedDup })
          : t("resources.importedCount", { n: stats.imported })
      );
      onImported();
      // 建议弹窗须在本弹窗关闭后仍可见：ImportDialog 常驻挂载（父级 open 控制），
      // state 置值后自身 onClose 不影响子 Dialog
      if (stats.suggestion) setSuggestion(stats.suggestion);
      onClose();
    } catch (e) {
      toast.error(t("resources.importFailed", { error: formatInvokeError(e, t) }));
    }
  };

  const totalCount = Object.values(resources).flat().length;
  const selectedCount = selected.size;

  return (
    <Dialog open={open} onOpenChange={(v) => !v && onClose()}>
      <DialogContent className="max-h-[80vh] max-w-lg overflow-y-auto">
        <DialogHeader>
          <DialogTitle className="text-sm">{t("resources.importDialogTitle")}</DialogTitle>
        </DialogHeader>

        <div className="mb-2 flex gap-2">
          <Button size="sm" variant="outline" className="h-6 text-[10px]" onClick={selectAll}>
            {t("resources.selectAll")}
          </Button>
          <Button size="sm" variant="outline" className="h-6 text-[10px]" onClick={selectNone}>
            {t("resources.selectNone")}
          </Button>
        </div>

        {loading ? (
          <div className="text-muted-foreground py-4 text-center text-xs">
            {t("common.scanning")}
          </div>
        ) : totalCount === 0 ? (
          <div className="text-muted-foreground py-4 text-center text-xs">
            {t("resources.noNativeFound")}
          </div>
        ) : (
          <div className="space-y-3">
            {enabledTools.map((tool) => {
              const toolResources = resources[tool.id] || [];
              if (toolResources.length === 0) return null;
              return (
                <div key={tool.id}>
                  <h4 className="mb-1 flex items-center gap-1.5 text-xs font-medium">
                    <ToolIcon toolId={tool.id} size={14} />
                    {tool.label}
                  </h4>
                  <div className="space-y-1">
                    {toolResources.map((res) => (
                      <label
                        key={res.id}
                        className="flex cursor-pointer items-center gap-2 text-xs"
                      >
                        <Checkbox
                          checked={selected.has(res.id)}
                          onCheckedChange={() => toggleSelect(res.id)}
                        />
                        <span className="bg-muted rounded px-1.5 py-0.5 text-[10px]">
                          {res.kind}
                        </span>
                        <span>{res.name}</span>
                      </label>
                    ))}
                  </div>
                </div>
              );
            })}
          </div>
        )}

        <div className="mt-4 flex justify-end gap-2">
          <Button size="sm" variant="outline" onClick={onClose}>
            {t("common.cancel")}
          </Button>
          <Button size="sm" onClick={handleImport} disabled={selectedCount === 0}>
            {t("resources.importSelected", { n: selectedCount })}
          </Button>
        </div>
      </DialogContent>

      {/* frontmatter 专属预填建议确认（Task 17）：确认=写绑定，忽略=关闭不写 */}
      <ImportSuggestionDialog suggestion={suggestion} onClose={() => setSuggestion(null)} />
    </Dialog>
  );
}
