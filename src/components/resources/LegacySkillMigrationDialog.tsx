// 遗留 codex 技能链接一次性迁移对话框（spec §4.3 / §6）
// 选择视图：技能清单 + 双动作（各带语义说明）+ 取消；执行后切换为逐条结果视图。
// 关闭（取消 / 右上角 X）= 零副作用：不做任何变更，下次启动谓词仍命中会再提示。
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
import type { LegacySkillMigrationState } from "@/hooks/useLegacySkillMigration";
import type { MigrationItemReport } from "@/lib/api/skillMigration";

// 逐条状态 → 词条键（后端契约固定为 ok | skipped | error）
const STATUS_KEY: Record<MigrationItemReport["status"], string> = {
  ok: "resources.migration.statusOk",
  skipped: "resources.migration.statusSkipped",
  error: "resources.migration.statusError",
};

interface LegacySkillMigrationDialogProps {
  migration: LegacySkillMigrationState;
}

export function LegacySkillMigrationDialog({ migration }: LegacySkillMigrationDialogProps) {
  const { t } = useTranslation();
  const { skills, visible, running, reports, runError, run, close } = migration;

  return (
    <Dialog
      open={visible}
      onOpenChange={(open) => {
        // 仅拦截「关闭」方向；打开完全由 detect 结果驱动。
        // 执行中不接受任何关闭入口（X/Esc/遮罩/取消统一禁用）：in-flight 迁移的
        // 逐条结果此刻只能在此对话框展示，中途关闭将永久不可见（review Minor）
        if (!open && !running) close();
      }}
    >
      <DialogContent className="max-w-md">
        {reports === null ? (
          // 选择视图：清单 + 双动作 + 取消
          <>
            <DialogHeader>
              <DialogTitle>{t("resources.migration.title")}</DialogTitle>
              {/* 说明正文放 div（DialogDescription 默认渲染 <p>，内嵌块级元素属非法
                  HTML 模型，会触发 validateDOMNesting 警告，review Minor） */}
              <DialogDescription asChild>
                <div className="space-y-2 pt-2 text-sm">
                  <p>{t("resources.migration.description", { n: skills.length })}</p>
                  <p className="text-foreground font-medium">
                    {t("resources.migration.listTitle")}
                  </p>
                  <ul className="max-h-40 list-disc overflow-y-auto pl-5 text-left">
                    {skills.map((name) => (
                      <li key={name}>{name}</li>
                    ))}
                  </ul>
                </div>
              </DialogDescription>
            </DialogHeader>

            <div className="space-y-3">
              {/* 动作一：迁移到 codex 私有目录（spec §4.3 动作一） */}
              <div className="space-y-1">
                <Button className="w-full" disabled={running} onClick={() => void run("migrate")}>
                  {t("resources.migration.migrateBtn")}
                </Button>
                <p className="text-muted-foreground text-xs">
                  {t("resources.migration.migrateDesc")}
                </p>
              </div>
              {/* 动作二：保留为共享（spec §4.3 动作二） */}
              <div className="space-y-1">
                <Button
                  variant="outline"
                  className="w-full"
                  disabled={running}
                  onClick={() => void run("keep")}
                >
                  {t("resources.migration.keepBtn")}
                </Button>
                <p className="text-muted-foreground text-xs">{t("resources.migration.keepDesc")}</p>
              </div>
              {runError && (
                <p className="text-destructive text-xs">
                  {t("resources.migration.runFailed", { detail: runError })}
                </p>
              )}
            </div>

            <DialogFooter className="gap-2">
              {/* 第三元素：关闭 = 不变更，下次启动再提示（spec §6） */}
              <Button variant="ghost" size="sm" disabled={running} onClick={close}>
                {t("resources.migration.cancel")}
              </Button>
            </DialogFooter>
          </>
        ) : (
          // 结果视图：逐条 name + status + detail
          <>
            <DialogHeader>
              <DialogTitle>{t("resources.migration.resultTitle")}</DialogTitle>
              <DialogDescription asChild>
                <ul className="space-y-2 pt-2 text-sm">
                  {reports.map((item) => (
                    <li key={item.name}>
                      <div>
                        <span className="font-medium">{item.name}</span>
                        <span className="text-muted-foreground"> — </span>
                        {/* 状态独立成节点，避免与分隔符混排（也便于测试精确断言） */}
                        <span>{t(STATUS_KEY[item.status])}</span>
                      </div>
                      {item.detail && (
                        <p className="text-muted-foreground text-xs">{item.detail}</p>
                      )}
                    </li>
                  ))}
                </ul>
              </DialogDescription>
            </DialogHeader>
            <DialogFooter className="gap-2">
              <Button size="sm" onClick={close}>
                {t("resources.migration.close")}
              </Button>
            </DialogFooter>
          </>
        )}
      </DialogContent>
    </Dialog>
  );
}
