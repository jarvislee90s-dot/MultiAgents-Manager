// 导入后 frontmatter 专属预填建议确认弹窗（spec §6；2026-09-15 裁决：
// 手动导入弹提示当场确认——Dialog 非 toast）。确认 = setResourceBinding
// 按声明工具写绑定表；忽略 = 关闭不写。「自动识别只建议不强制，
// 真值永远是 resource_bindings 手动值」
import { useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { formatInvokeError } from "@/lib/invokeError";
import { setResourceBinding } from "@/lib/api/preset";
import { BINDINGS_KEY } from "@/lib/query/queries/bindings";
import type { FrontmatterSuggestion } from "@/types/extension";

interface Props {
  suggestion: FrontmatterSuggestion | null;
  onClose: () => void;
}

export function ImportSuggestionDialog({ suggestion, onClose }: Props) {
  const { t } = useTranslation();
  const qc = useQueryClient();
  const [saving, setSaving] = useState(false);

  // 确认：写绑定表（reason 留空——frontmatter 声明即来源）+ 失效绑定查询
  //（ResourceByKindView 专属徽标的数据源）。成功 toast 复用既有键组合
  //（presets.exclusiveBadge），不新增文案键
  const confirm = async () => {
    if (!suggestion || saving) return;
    setSaving(true);
    try {
      await setResourceBinding(suggestion.extensionId, suggestion.tools);
      await qc.invalidateQueries({ queryKey: BINDINGS_KEY });
      toast.success(
        `${suggestion.extensionId.replace(/^skill-/, "")} · ${t("presets.exclusiveBadge")}: ${suggestion.tools.join(", ")}`
      );
      onClose();
    } catch (e) {
      toast.error(formatInvokeError(e, t));
    } finally {
      setSaving(false);
    }
  };

  return (
    <Dialog open={suggestion !== null} onOpenChange={(v) => !v && onClose()}>
      <DialogContent className="max-w-sm">
        <DialogHeader>
          <DialogTitle className="text-sm">{t("resources.health.suggestion")}</DialogTitle>
        </DialogHeader>
        {suggestion && (
          <p className="text-muted-foreground text-xs">
            <span className="text-foreground font-medium">
              {suggestion.extensionId.replace(/^skill-/, "")}
            </span>
            {" · "}
            {t("presets.exclusiveBadge")}: {suggestion.tools.join(", ")}
          </p>
        )}
        <div className="flex justify-end gap-2">
          <Button size="sm" variant="outline" onClick={onClose}>
            {t("common.cancel")}
          </Button>
          <Button size="sm" onClick={() => void confirm()} disabled={saving}>
            {t("common.confirm")}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
