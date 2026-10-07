// 导出设置（2026-10-07 用户裁决 B1 + 本轮追加）：
// **凡是只在导出时起作用的东西，入口就放在导出所在的地方**，不留在设置页里。
//
// 本弹层现在装两项，两项都从「设置 → 用量统计」搬过来（**整个迁移，不在两处保留**）：
//  * **分享图评语**（B1）：留空即用默认评语池，可用 `{range} {tokens} {hitPct} {models}` 变量；
//  * **分享图姿态**（本轮）：token 用量里的那只宠物，在分享图上摆什么姿势。
//    原来在设置页叫「分享图姿态」，用户反馈「不太理解是什么的姿态」⇒ 标签写明「token 用量里的
//    宠物姿态」，并给两处都加 `title` 提示（用户建议的 information 浮窗做法）。
//
// **数据通路与设置页同一条**（`usageGetSettings` 读 + `usageSetSettings` 写），**不自立第二套**：
// 两处各写各的就会漂。**刻意不做 query 缓存失效** —— 这两个字段已没有任何「读出来展示」的消费方
// （设置页那两格就是被本项删掉的），导出路径每次动作都现读设置，失效通知没有对象。
// 副作用正好是想要的：本组件**不依赖 `QueryClientProvider`**，而导出条在既有用例里是裸渲染的。
//
// ⚠️ **读是惰性的（点开才读一次）**，刻意不用 `useUsageSettingsQuery`：那个 hook 一挂载就发 IPC，
// 本组件住在看板吸顶头里 ⇒ 每次渲染平白多一次 `usage_get_settings`，还会打乱既有导出用例
// 按调用序编排的一次性 mock（「先给设置、再让落盘失败」）。
import { useState } from "react";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { useAppTranslation } from "@/hooks/use-app-translation";
import { usageGetSettings, usageSetSettings } from "@/lib/api/usage";
import { usageErrMsg } from "@/components/usage/usageErrors";
import { poseKeysFor } from "@/lib/usage/sheet";

export function UsageExportSettings(props: { disabled?: boolean; title?: string }) {
  const { t } = useAppTranslation();
  const [open, setOpen] = useState(false);
  const [quote, setQuote] = useState("");
  const [pose, setPose] = useState("random");
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState("");
  const [err, setErr] = useState("");

  /** 点开才读当前值（见文件头「读是惰性的」）；读失败也要如实可见，不能装作空评语 */
  const start = async () => {
    setOpen(true);
    setNote("");
    setErr("");
    setBusy(true);
    try {
      const s = await usageGetSettings();
      setQuote(s.exportQuote ?? "");
      setPose(s.exportPose || "random");
    } catch (e) {
      setErr(usageErrMsg(e, t));
      console.error("[usage] 导出设置读取失败", e);
    } finally {
      setBusy(false);
    }
  };

  const save = async () => {
    setBusy(true);
    setErr("");
    try {
      // 两项**一次 patch** 提交：分两次发会让「保存成功」的语义变成「一半成功」✗
      await usageSetSettings({ exportQuote: quote, exportPose: pose });
      setOpen(false);
      setNote(t("usage.export.settingsSaved"));
    } catch (e) {
      // 与设置页同一条码表（`usage.rpc.<code>` + detail），不自造第二套错误文案
      setErr(usageErrMsg(e, t));
      console.error("[usage] 导出设置保存失败", e);
    } finally {
      setBusy(false);
    }
  };

  return (
    <>
      <Button
        size="sm"
        variant="outline"
        data-testid="usage-export-settings-open"
        disabled={props.disabled}
        // 与另三个看板口径按钮同纪律（X7）：禁用时必须说明原因
        title={props.title}
        onClick={() => void start()}
      >
        {t("usage.export.settings")}
      </Button>
      {note ? (
        <span
          role="status"
          data-testid="usage-export-settings-note"
          className="text-muted-foreground text-xs"
        >
          {note}
        </span>
      ) : null}

      <Dialog open={open} onOpenChange={(v) => (v ? setOpen(true) : setOpen(false))}>
        <DialogContent data-testid="usage-export-settings-dialog">
          <DialogHeader>
            <DialogTitle>{t("usage.export.settingsTitle")}</DialogTitle>
          </DialogHeader>

          {/* 评语 */}
          <label className="text-sm font-medium" htmlFor="usage-export-quote-input">
            {t("usage.export.quote")}
          </label>
          <Input
            id="usage-export-quote-input"
            data-testid="usage-export-quote-input"
            // 占位符提示可用变量（单花括号是字面量，不是 i18next 插值）
            placeholder={t("usage.export.quotePlaceholder")}
            value={quote}
            onChange={(e) => setQuote(e.currentTarget.value)}
          />
          <p className="text-muted-foreground text-xs">{t("usage.export.quoteHint")}</p>

          {/* 姿态：键族取自 Task 12 的 `poseKeysFor(11)`（**不得**自造第二份）；`look` 项保留 */}
          <label
            className="mt-2 text-sm font-medium"
            htmlFor="usage-export-pose"
            title={t("usage.export.poseHint")}
          >
            {t("usage.export.poseLabel")}
            <span className="text-muted-foreground ml-1 text-xs">ⓘ</span>
          </label>
          <select
            id="usage-export-pose"
            data-testid="usage-export-pose"
            title={t("usage.export.poseHint")}
            className="bg-background h-8 rounded-md border px-2 text-sm"
            value={pose}
            onChange={(e) => setPose(e.currentTarget.value)}
          >
            {poseKeysFor(11).map((key) => (
              <option key={key} value={key}>
                {t(`usage.pose.${key}`)}
              </option>
            ))}
          </select>
          <p className="text-muted-foreground text-xs">{t("usage.export.poseHint")}</p>

          {err ? (
            <p className="text-sm text-red-500" data-testid="usage-export-settings-error">
              {err}
            </p>
          ) : null}
          <div className="flex justify-end gap-2">
            <Button
              size="sm"
              variant="ghost"
              data-testid="usage-export-settings-cancel"
              onClick={() => setOpen(false)}
            >
              {t("usage.export.quoteCancel")}
            </Button>
            <Button
              size="sm"
              data-testid="usage-export-settings-save"
              disabled={busy}
              onClick={() => void save()}
            >
              {t("usage.export.quoteSave")}
            </Button>
          </div>
        </DialogContent>
      </Dialog>
    </>
  );
}
