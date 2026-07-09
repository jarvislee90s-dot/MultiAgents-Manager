import { useState, useEffect } from "react";
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogFooter } from "@/components/ui/dialog";
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
  const [result, setResult] = useState<ValidateResult | null>(null);
  const [installing, setInstalling] = useState(false);

  useEffect(() => {
    if (path && open) { validateManifestPath(path).then(setResult); }
  }, [path, open]);

  const handleInstall = async () => {
    if (!path) return;
    setInstalling(true);
    try {
      await installResource(path);
      onInstalled?.();
      onOpenChange(false);
    } finally { setInstalling(false); }
  };

  const manifest = result?.manifest as { name?: string; version?: string; kind?: string; permissions?: string[]; compatibility?: { tool: string }[] } | undefined;
  const hasHighRisk = manifest?.permissions?.some((p) => p === "shell" || p === "settings.write");

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader><DialogTitle>安装确认</DialogTitle></DialogHeader>
        {result?.valid && manifest ? (
          <div className="space-y-4">
            <div>
              <p className="font-medium">{manifest.name} v{manifest.version}</p>
              <p className="text-sm text-muted-foreground">类型: {manifest.kind}</p>
            </div>
            {manifest.permissions && manifest.permissions.length > 0 && (
              <div>
                <p className="mb-1 text-sm font-medium">权限声明:</p>
                <div className="flex flex-wrap gap-1">
                  {manifest.permissions.map((p) => <PermissionBadge key={p} permission={p} />)}
                </div>
              </div>
            )}
            {manifest.compatibility && (
              <div>
                <p className="mb-1 text-sm font-medium">兼容工具:</p>
                <p className="text-sm text-muted-foreground">{manifest.compatibility.map((c) => c.tool).join(", ")}</p>
              </div>
            )}
            {hasHighRisk && (
              <div className="rounded border border-red-300 bg-red-50 p-3 dark:border-red-700 dark:bg-red-950">
                <p className="text-sm text-red-700 dark:text-red-300">此资源声明了高风险权限，请确认你信任此资源的来源。</p>
              </div>
            )}
          </div>
        ) : (
          <div className="space-y-2">
            <p className="text-sm text-red-600">Manifest 校验失败:</p>
            {result?.errors?.map((e, i) => (
              <p key={i} className="text-sm text-muted-foreground">{e.field}: {e.message} ({e.code})</p>
            ))}
          </div>
        )}
        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>取消</Button>
          {result?.valid && (
            <Button onClick={handleInstall} disabled={installing}>
              {installing ? "安装中..." : "确认安装"}
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
