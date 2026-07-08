import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Checkbox } from "@/components/ui/checkbox";
import type { NativeExtension } from "@/types/extension";

const TOOLS = [
  { id: "claude", label: "Claude Code" },
  { id: "codex", label: "Codex CLI" },
  { id: "opencode", label: "OpenCode" },
  { id: "openclaw", label: "OpenClaw" },
];

interface Props {
  open: boolean;
  onClose: () => void;
  onImported: () => void;
}

export function ImportDialog({ open, onClose, onImported }: Props) {
  const [resources, setResources] = useState<Record<string, NativeExtension[]>>({});
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    if (open) {
      loadAllResources();
    }
  }, [open]);

  const loadAllResources = async () => {
    setLoading(true);
    const all: Record<string, NativeExtension[]> = {};
    for (const tool of TOOLS) {
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
    Object.values(resources).flat().forEach((r) => all.add(r.id));
    setSelected(all);
  };

  const selectNone = () => {
    setSelected(new Set());
  };

  const handleImport = async () => {
    const items: [string, string][] = [];
    for (const toolId of Object.keys(resources)) {
      for (const res of resources[toolId]) {
        if (selected.has(res.id)) {
          items.push([res.sourcePath, res.name]);
        }
      }
    }

    if (items.length === 0) {
      toast.error("请选择至少一个资源");
      return;
    }

    try {
      const stats = await invoke<{ imported: number; skippedDup: number }>("import_native_resources", { items });
      toast.success(`成功导入 ${stats.imported} 个资源${stats.skippedDup > 0 ? `，跳过 ${stats.skippedDup} 个` : ""}`);
      onImported();
      onClose();
    } catch (e) {
      toast.error(`导入失败: ${e}`);
    }
  };

  const totalCount = Object.values(resources).flat().length;
  const selectedCount = selected.size;

  return (
    <Dialog open={open} onOpenChange={(v) => !v && onClose()}>
      <DialogContent className="max-w-lg max-h-[80vh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle className="text-sm">导入原生资源</DialogTitle>
        </DialogHeader>

        <div className="flex gap-2 mb-2">
          <Button size="sm" variant="outline" className="h-6 text-[10px]" onClick={selectAll}>
            全选
          </Button>
          <Button size="sm" variant="outline" className="h-6 text-[10px]" onClick={selectNone}>
            全不选
          </Button>
        </div>

        {loading ? (
          <div className="text-center py-4 text-xs text-muted-foreground">扫描中...</div>
        ) : totalCount === 0 ? (
          <div className="text-center py-4 text-xs text-muted-foreground">未发现原生资源</div>
        ) : (
          <div className="space-y-3">
            {TOOLS.map((tool) => {
              const toolResources = resources[tool.id] || [];
              if (toolResources.length === 0) return null;
              return (
                <div key={tool.id}>
                  <h4 className="text-xs font-medium mb-1">{tool.label}</h4>
                  <div className="space-y-1">
                    {toolResources.map((res) => (
                      <label key={res.id} className="flex items-center gap-2 text-xs cursor-pointer">
                        <Checkbox
                          checked={selected.has(res.id)}
                          onCheckedChange={() => toggleSelect(res.id)}
                        />
                        <span className="rounded bg-muted px-1.5 py-0.5 text-[10px]">{res.kind}</span>
                        <span>{res.name}</span>
                      </label>
                    ))}
                  </div>
                </div>
              );
            })}
          </div>
        )}

        <div className="flex justify-end gap-2 mt-4">
          <Button size="sm" variant="outline" onClick={onClose}>取消</Button>
          <Button size="sm" onClick={handleImport} disabled={selectedCount === 0}>
            导入选中 ({selectedCount})
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
