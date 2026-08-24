import { useState, useEffect } from "react";
import { useTranslation } from "react-i18next";
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

export function CompatibilityDialog({
  open,
  presetId,
  toolId,
  toolName,
  onClose,
  onConfirm,
}: Props) {
  const { t } = useTranslation();
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
      const data = await invoke<CompatibilityReport>("check_preset_compatibility", {
        presetId,
        toolId,
      });
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
          <DialogTitle className="text-sm">{t("presets.applyTo", { tool: toolName })}</DialogTitle>
        </DialogHeader>

        {loading ? (
          <div className="text-muted-foreground py-4 text-center text-xs">
            {t("common.checking")}
          </div>
        ) : report ? (
          <div className="space-y-3">
            {/* Compatible resources */}
            {report.compatible.length > 0 && (
              <div>
                <h4 className="mb-1 text-xs font-medium text-green-600">
                  <CheckCircle className="mr-1 inline h-3 w-3" />
                  {t("presets.compatibleCount", { n: report.compatible.length })}
                </h4>
                <div className="space-y-1">
                  {report.compatible.map((item) => (
                    <div key={item.id} className="flex items-center gap-2 text-xs">
                      <span className="rounded bg-green-50 px-1.5 py-0.5 text-[10px]">
                        {item.kind}
                      </span>
                      <span>{item.name}</span>
                    </div>
                  ))}
                </div>
              </div>
            )}

            {/* Incompatible resources */}
            {report.incompatible.length > 0 && (
              <div>
                <h4 className="mb-1 text-xs font-medium text-orange-600">
                  <XCircle className="mr-1 inline h-3 w-3" />
                  {t("presets.incompatibleCount", { n: report.incompatible.length })}
                </h4>
                <div className="space-y-1">
                  {report.incompatible.map((item) => (
                    <div key={item.id} className="flex items-center gap-2 text-xs">
                      <span className="rounded bg-orange-50 px-1.5 py-0.5 text-[10px]">
                        {item.kind}
                      </span>
                      <span>{item.name}</span>
                      <span className="text-muted-foreground text-[10px]">({item.reason})</span>
                    </div>
                  ))}
                </div>
              </div>
            )}

            <div className="mt-4 flex justify-end gap-2">
              <Button size="sm" variant="outline" onClick={onClose}>
                {t("common.cancel")}
              </Button>
              <Button size="sm" onClick={onConfirm} disabled={report.compatible.length === 0}>
                {t("presets.confirmApply", { n: report.compatible.length })}
              </Button>
            </div>
          </div>
        ) : null}
      </DialogContent>
    </Dialog>
  );
}
