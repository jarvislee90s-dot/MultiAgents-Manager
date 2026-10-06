// 分享图评语编辑器（2026-10-07 用户裁决 B1：从「设置 → 用量统计」搬到**导出所在的地方**）。
//
// **为什么搬**：用户原话是「用户可能会经常修改，或者想写点什么东西，他们会在这个导出的地方找，
// 而不是在设置里面」。评语只在导出时起作用 ⇒ 入口与四个导出按钮同处（看板吸顶导出条）。
//
// **数据通路与设置页同一条**（`usageGetSettings` 读 + `usageSetSettings` 写），**不自立第二套**：
// 两处若各写各的，就会出现「设置页改了、导出处还是旧值」这类漂移。
// **刻意不做 query 缓存失效**：B1 之后 `exportQuote` 已无任何「读出来展示」的消费方
// （设置页那一格就是被本项删掉的），导出路径每次动作都现读设置 ⇒ 失效通知没有对象。
// 副作用是本组件**不依赖 QueryClientProvider**，而导出条在既有用例里是**裸渲染**的。
// 契约侧 `UsageSettings.exportQuote` 字段本身**不变**（仍是 DB 列 + 仍走同一条 `usage_set_settings`），
// 变的只是界面入口 —— 故 `usageMockParity` / `usage-contract-parity` 那些字段清单用例不受影响。
//
// ⚠️ **读是惰性的（点开才读）**，刻意不用 `useUsageSettingsQuery`：那个 hook 一挂载就发 IPC，
// 而本组件住在看板的导出条里 ⇒ 每次渲染都会平白多一次 `usage_get_settings`，
// 既浪费又会**打乱既有导出用例的一次性 mock 序列**（那些用例按调用序编排「先给设置、再让落盘失败」）。
// 惰性读的语义也更对：不点「评语」就不需要这个值。
import { useState } from "react";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { useAppTranslation } from "@/hooks/use-app-translation";
import { usageGetSettings, usageSetSettings } from "@/lib/api/usage";
import { usageErrMsg } from "@/components/usage/usageErrors";

export function UsageQuoteEditor(props: { disabled?: boolean; title?: string }) {
  const { t } = useAppTranslation();
  const [open, setOpen] = useState(false);
  const [draft, setDraft] = useState("");
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
      setDraft(s.exportQuote ?? "");
    } catch (e) {
      setErr(usageErrMsg(e, t));
      console.error("[usage] 评语读取失败", e);
    } finally {
      setBusy(false);
    }
  };

  const save = async () => {
    setBusy(true);
    setErr("");
    try {
      await usageSetSettings({ exportQuote: draft });
      // 成功**不常驻**提示：对话框一关，「已保存」就没有承载物了；失败才需要留在原地可读
      setOpen(false);
      setNote(t("usage.export.quoteSaved"));
    } catch (e) {
      // 与设置页同一条码表（`usage.rpc.<code>` + detail），不自造第二套错误文案
      setErr(usageErrMsg(e, t));
      console.error("[usage] 评语保存失败", e);
    } finally {
      setBusy(false);
    }
  };

  return (
    <>
      <Button
        size="sm"
        variant="outline"
        data-testid="usage-export-quote-open"
        disabled={props.disabled}
        // 与另三个看板口径按钮同纪律（X7）：禁用时**必须**说明原因，否则用户看到的是「按钮灰了但不知为何」
        title={props.title}
        onClick={() => void start()}
      >
        {t("usage.export.quote")}
      </Button>
      {note ? (
        <span
          role="status"
          data-testid="usage-export-quote-note"
          className="text-muted-foreground text-xs"
        >
          {note}
        </span>
      ) : null}

      <Dialog open={open} onOpenChange={(v) => (v ? setOpen(true) : setOpen(false))}>
        <DialogContent data-testid="usage-export-quote-dialog">
          <DialogHeader>
            <DialogTitle>{t("usage.export.quoteTitle")}</DialogTitle>
          </DialogHeader>
          <Input
            data-testid="usage-export-quote-input"
            // 占位符提示可用变量（单花括号是字面量，不是 i18next 插值）
            placeholder={t("usage.export.quotePlaceholder")}
            value={draft}
            onChange={(e) => setDraft(e.currentTarget.value)}
          />
          <p className="text-muted-foreground text-xs">{t("usage.export.quoteHint")}</p>
          {err ? (
            <p className="text-sm text-red-500" data-testid="usage-export-quote-error">
              {err}
            </p>
          ) : null}
          <div className="flex justify-end gap-2">
            <Button
              size="sm"
              variant="ghost"
              data-testid="usage-export-quote-cancel"
              onClick={() => setOpen(false)}
            >
              {t("usage.export.quoteCancel")}
            </Button>
            <Button
              size="sm"
              data-testid="usage-export-quote-save"
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
