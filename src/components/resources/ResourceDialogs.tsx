import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";

/** 待停用确认上下文（skill 亮→灰弹窗前的 checkSkillTargetType 结果） */
export type PendingDisable = {
  skillName: string;
  toolId: string;
  toolLabel: string;
  displayName: string;
  targetType: "symlink" | "native";
};

/** 卸载确认上下文：initial = 常规卸载确认；sharedLink = .agents 直链引用二级确认（spec §4.4） */
export type PendingUninstall = {
  kind: string;
  name: string;
  count: number;
  stage: "initial" | "sharedLink";
};

/** 新增 MCP 表单草稿：四字段均为原始字符串，提交时再解析（args 按空白拆 / env 按 KEY=VALUE 行拆） */
export type NewMcpDraft = {
  name: string;
  command: string;
  args: string;
  env: string;
};

/** 空草稿常量：初始态 / 提交后 / 取消时共用（不可变值，更新一律整体替换） */
export const EMPTY_NEW_MCP: NewMcpDraft = { name: "", command: "", args: "", env: "" };

/** skill「亮 → 灰」停用确认弹窗：symlink 走移除链接文案，native 走红色删除警示。
 *  取消按钮 = 关弹窗 + 清空 pending；Dialog 自身关闭（X / esc）仅关弹窗（与原内联行为一致） */
export function SkillDisableConfirmDialog(props: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  pending: PendingDisable | null;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  const { t } = useTranslation();
  const { pending } = props;
  return (
    <Dialog open={props.open} onOpenChange={props.onOpenChange}>
      <DialogContent className="max-w-sm">
        {pending?.targetType === "native" ? (
          <>
            <DialogHeader>
              <DialogTitle className="text-red-600">{t("resources.deleteNativeTitle")}</DialogTitle>
              <DialogDescription className="space-y-2 pt-2 text-sm">
                <p className="text-red-500">
                  {t("resources.deleteNativeDesc1", { name: pending?.displayName })}
                </p>
                <p>{t("resources.deleteNativeDesc2", { tool: pending?.toolLabel })}</p>
              </DialogDescription>
            </DialogHeader>
            <DialogFooter className="gap-2">
              <Button variant="outline" size="sm" onClick={props.onCancel}>
                {t("common.cancel")}
              </Button>
              <Button variant="destructive" size="sm" onClick={props.onConfirm}>
                {t("resources.trashAndRemove")}
              </Button>
            </DialogFooter>
          </>
        ) : (
          <>
            <DialogHeader>
              <DialogTitle>{t("resources.removeLinkTitle")}</DialogTitle>
              <DialogDescription className="pt-2 text-sm">
                {t("resources.removeLinkDesc", {
                  name: pending?.displayName,
                  tool: pending?.toolLabel,
                })}
              </DialogDescription>
            </DialogHeader>
            <DialogFooter className="gap-2">
              <Button variant="outline" size="sm" onClick={props.onCancel}>
                {t("common.cancel")}
              </Button>
              <Button variant="default" size="sm" onClick={props.onConfirm}>
                {t("resources.removeLink")}
              </Button>
            </DialogFooter>
          </>
        )}
      </DialogContent>
    </Dialog>
  );
}

/** 添加 MCP 到 SSOT 仓库弹窗：受控表单（草稿状态留在父级），取消 = 关弹窗 + 重置草稿 */
export function McpAddDialog(props: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  value: NewMcpDraft;
  onChange: (next: NewMcpDraft) => void;
  onCancel: () => void;
  onSubmit: () => void;
}) {
  const { t } = useTranslation();
  const { value, onChange } = props;
  return (
    <Dialog open={props.open} onOpenChange={props.onOpenChange}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>{t("mcp.addTitle")}</DialogTitle>
          <DialogDescription className="pt-2 text-xs">
            {t("resources.addMcpDesc")}
          </DialogDescription>
        </DialogHeader>
        <div className="space-y-3 py-2">
          <div>
            <label className="text-xs font-medium">{t("mcp.nameLabel")}</label>
            <input
              value={value.name}
              onChange={(e) => onChange({ ...value, name: e.currentTarget.value })}
              placeholder="firecrawl"
              className="h-8 w-full rounded border px-2 text-xs"
            />
          </div>
          <div>
            <label className="text-xs font-medium">{t("mcp.commandLabel")}</label>
            <input
              value={value.command}
              onChange={(e) => onChange({ ...value, command: e.currentTarget.value })}
              placeholder="npx"
              className="h-8 w-full rounded border px-2 text-xs"
            />
          </div>
          <div>
            <label className="text-xs font-medium">{t("mcp.argsLabelSpace")}</label>
            <input
              value={value.args}
              onChange={(e) => onChange({ ...value, args: e.currentTarget.value })}
              placeholder="-y firecrawl-mcp"
              className="h-8 w-full rounded border px-2 text-xs"
            />
          </div>
          <div>
            <label className="text-xs font-medium">{t("mcp.envLabel")}</label>
            <textarea
              value={value.env}
              onChange={(e) => onChange({ ...value, env: e.currentTarget.value })}
              placeholder="API_KEY=xxx"
              className="h-16 w-full rounded border px-2 text-xs"
            />
          </div>
        </div>
        <DialogFooter className="gap-2">
          <Button variant="outline" size="sm" onClick={props.onCancel}>
            {t("common.cancel")}
          </Button>
          <Button size="sm" onClick={props.onSubmit}>
            {t("resources.addToRepo")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

/** 卸载确认弹窗（stage=sharedLink 为 ~/.agents/skills 直链引用二级确认，spec §4.4；
 *  取消/关闭不产生任何变更） */
export function UninstallConfirmDialog(props: {
  pending: PendingUninstall | null;
  onCancel: () => void;
  onConfirm: (force: boolean) => void;
}) {
  const { t } = useTranslation();
  const { pending } = props;
  return (
    <Dialog open={!!pending} onOpenChange={(o) => !o && props.onCancel()}>
      <DialogContent className="max-w-sm">
        {pending?.stage === "sharedLink" ? (
          <>
            <DialogHeader>
              <DialogTitle className="text-red-600">
                {t("resources.sharedLinkConfirmTitle")}
              </DialogTitle>
              <DialogDescription className="pt-2 text-sm">
                {t("resources.sharedLinkConfirmDesc", { name: pending?.name })}
              </DialogDescription>
            </DialogHeader>
            <DialogFooter className="gap-2">
              <Button variant="outline" size="sm" onClick={props.onCancel}>
                {t("common.cancel")}
              </Button>
              <Button variant="destructive" size="sm" onClick={() => props.onConfirm(true)}>
                {t("resources.sharedLinkConfirmContinue")}
              </Button>
            </DialogFooter>
          </>
        ) : (
          <>
            <DialogHeader>
              <DialogTitle className="text-red-600">{t("resources.uninstallTitle")}</DialogTitle>
              <DialogDescription className="pt-2 text-sm">
                {t("resources.uninstallDesc", {
                  name: pending?.name,
                  n: pending?.count ?? 0,
                })}
              </DialogDescription>
            </DialogHeader>
            <DialogFooter className="gap-2">
              <Button variant="outline" size="sm" onClick={props.onCancel}>
                {t("common.cancel")}
              </Button>
              <Button variant="destructive" size="sm" onClick={() => props.onConfirm(false)}>
                {t("resources.uninstall")}
              </Button>
            </DialogFooter>
          </>
        )}
      </DialogContent>
    </Dialog>
  );
}
