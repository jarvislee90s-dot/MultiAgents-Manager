import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { CheckCircle, XCircle } from "lucide-react";
import type { CompatibilityReport } from "@/types/extension";

interface Props {
  open: boolean;
  presetId: string;
  toolId: string;
  toolName: string;
  onClose: () => void;
  onConfirm: () => void;
}

export function CompatibilityDialog({ open, presetId, toolId, toolName, onClose, onConfirm }: Props) {
  const [report, setReport] = useState<CompatibilityReport | null>(null);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    if (open) {
      loadReport();
    }
  }, [open]);

  const loadReport = async () => {
    setLoading(true);
    try {
      const data = await invoke<CompatibilityReport>("check_preset_compatibility", { presetId, toolId });
      setReport(data);
    } catch (e) {
      console.error("Failed to check compatibility:", e);
    } finally {
      setLoading(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={(v) => !v && onClose()}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle className="text-sm">应用预设组到 {toolName}</DialogTitle>
        </DialogHeader>

        {loading ? (
          <div className="text-center py-4 text-xs text-muted-foreground">检查兼容性...</div>
        ) : report ? (
          <div className="space-y-3">
            {/* Compatible resources */}
            {report.compatible.length > 0 && (
              <div>
                <h4 className="text-xs font-medium text-green-600 mb-1">
                  <CheckCircle className="inline h-3 w-3 mr-1" />
                  兼容的资源 ({report.compatible.length})
                </h4>
                <div className="space-y-1">
                  {report.compatible.map((item) => (
                    <div key={item.id} className="flex items-center gap-2 text-xs">
                      <span className="rounded bg-green-50 px-1.5 py-0.5 text-[10px]">{item.kind}</span>
                      <span>{item.name}</span>
                    </div>
                  ))}
                </div>
              </div>
            )}

            {/* Incompatible resources */}
            {report.incompatible.length > 0 && (
              <div>
                <h4 className="text-xs font-medium text-orange-600 mb-1">
                  <XCircle className="inline h-3 w-3 mr-1" />
                  不兼容的资源 ({report.incompatible.length})
                </h4>
                <div className="space-y-1">
                  {report.incompatible.map((item) => (
                    <div key={item.id} className="flex items-center gap-2 text-xs">
                      <span className="rounded bg-orange-50 px-1.5 py-0.5 text-[10px]">{item.kind}</span>
                      <span>{item.name}</span>
                      <span className="text-muted-foreground text-[10px]">({item.reason})</span>
                    </div>
                  ))}
                </div>
              </div>
            )}

            <div className="flex justify-end gap-2 mt-4">
              <Button size="sm" variant="outline" onClick={onClose}>取消</Button>
              <Button size="sm" onClick={onConfirm} disabled={report.compatible.length === 0}>
                确认应用 ({report.compatible.length} 项)
              </Button>
            </div>
          </div>
        ) : null}
      </DialogContent>
    </Dialog>
  );
}
