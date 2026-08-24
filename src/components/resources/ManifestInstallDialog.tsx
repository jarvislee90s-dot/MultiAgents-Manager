import { useState, useEffect } from "react";
import { useTranslation } from "react-i18next";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogFooter,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { PermissionBadge } from "./PermissionBadge";
import { validateManifestPath, installResource, type ValidateResult } from "@/lib/api/manifest";

interface Props {
  path: string | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onInstalled?: () => void;
}

export function ManifestInstallDialog({ path, open, onOpenChange, onInstalled }: Props) {
  const { t } = useTranslation();
  const [result, setResult] = useState<ValidateResult | null>(null);
  const [installing, setInstalling] = useState(false);

  useEffect(() => {
    if (path && open) {
      validateManifestPath(path).then(setResult);
    }
  }, [path, open]);

  const handleInstall = async () => {
    if (!path) return;
    setInstalling(true);
    try {
      await installResource(path);
      onInstalled?.();
      onOpenChange(false);
    } finally {
      setInstalling(false);
    }
  };

  const manifest = result?.manifest as
    | {
        name?: string;
        version?: string;
        kind?: string;
        permissions?: string[];
        compatibility?: { tool: string }[];
      }
    | undefined;
  const hasHighRisk = manifest?.permissions?.some((p) => p === "shell" || p === "settings.write");

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{t("manifest.installConfirm")}</DialogTitle>
        </DialogHeader>
        {result?.valid && manifest ? (
          <div className="space-y-4">
            <div>
              <p className="font-medium">
                {manifest.name} v{manifest.version}
              </p>
              <p className="text-muted-foreground text-sm">
                {t("manifest.kindLabel", { kind: manifest.kind })}
              </p>
            </div>
            {manifest.permissions && manifest.permissions.length > 0 && (
              <div>
                <p className="mb-1 text-sm font-medium">{t("manifest.permissions")}</p>
                <div className="flex flex-wrap gap-1">
                  {manifest.permissions.map((p) => (
                    <PermissionBadge key={p} permission={p} />
                  ))}
                </div>
              </div>
            )}
            {manifest.compatibility && (
              <div>
                <p className="mb-1 text-sm font-medium">{t("manifest.compatibleTools")}</p>
                <p className="text-muted-foreground text-sm">
                  {manifest.compatibility.map((c) => c.tool).join(", ")}
                </p>
              </div>
            )}
            {hasHighRisk && (
              <div className="rounded border border-red-300 bg-red-50 p-3 dark:border-red-700 dark:bg-red-950">
                <p className="text-sm text-red-700 dark:text-red-300">
                  {t("manifest.highRiskWarning")}
                </p>
              </div>
            )}
          </div>
        ) : (
          <div className="space-y-2">
            <p className="text-sm text-red-600">{t("manifest.validationFailed")}</p>
            {result?.errors?.map((e, i) => (
              <p key={i} className="text-muted-foreground text-sm">
                {e.field}: {e.message} ({e.code})
              </p>
            ))}
          </div>
        )}
        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            {t("common.cancel")}
          </Button>
          {result?.valid && (
            <Button onClick={handleInstall} disabled={installing}>
              {installing ? t("manifest.installing") : t("manifest.confirmInstall")}
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
