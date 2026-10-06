// 导出动作条（计划② Task 11 步骤 5 + Task 12 步骤 7；spec P4）——复制文本 / 导出图片 / 复制图片 / 导出 CSV。
//
// 通道纪律（§3 第 24–28 条）：
//  * 导出**必须走 Rust 落盘**（`saveText` / `saveBytes` → 契约的两条落盘命令），**不走** `<a download>`；
//    本组件不碰对象 URL，也不自己拼下载链接。
//  * 定位复用既有 `revealDir`（经 `openContainingDir` 包装），传的是**文件路径本身**——不是父目录。
//  * 剪贴板（`copyText` / `copyImage`）只作**副本**入口：两条复制链一次 IPC 都不发。
//  * CSV 内容**全部来自后端**：`usageExportCsv` 返回成品字符串，本组件只给文件名并落盘 ——
//    不拼列、不改列序、不加表头（`.csv` 的 UTF-8 BOM 由 Rust 侧 `with_bom_if_csv` 加）。
//  * 导出失败是 `String` **自由文本**（不是 `{code, detail}`）⇒ 透传 `String(e)` 原因，**不得**喂给
//    `usageErrMsg`；而取数那一步（`usageExportCsv`）是**用量命令**，它的结构化码必须走 `usageErrMsg`
//    （自己 `String(e)` 会印出 `[object Object]`）。两条错误口径各归其位，见 `onCsv` 的两段。
//
// 交互纪律：
//  * 反馈是**行内** `role="status"`（testid `usage-export-toast`），**不用** toast —— 大看板是二级窗口，
//    未必挂载 `Toaster`，用 toast 会把反馈丢进虚空。
//  * `openContainingDir` **失败上抛**（Task 10 口径：本层不吞、也不翻文案）⇒ 处理器自带 `try`/`catch`：
//    写成 `onClick={() => void openContainingDir(path)}` 会把 rejection 吞掉、界面毫无反应。
//    定位失败时撤掉「打开所在目录」入口并显示原因 —— 文件其实已经落盘，但入口再留着只会一直失败。
//  * 记录页签**只留 CSV**：文本摘要与分享图都是看板口径的产物，放在记录页会产生口径歧义。
//  * 按钮一律 `@/components/ui/button` 原语、取色走主题 token（§3 第 19/40 条）。
//
// ⚠️ 依赖方向（**硬约束**，Task 12 起）：`useExportShare`（分享图整条链）**只允许**本文件 import ——
//    页面与其它组件一律不得 import，免得同一条导出链出现第二个装配点。Task 11 落盘时这里写的是
//    「**不得** import `useExportShare`」（那时模块还不存在，提前 import 会得到不可解析的 import，
//    而 `pnpm lint` 拦不住 ⇒ 以「绿」的错觉提交一个坏构建）；Task 12 把两个图片按钮接回来，
//    本文件就是那个唯一入口。props 形状**只增不改**：图片按钮不需要新字段
//    （`useExportShare(t)` 在组件内部按需读设置，看板数据由调用方逐次传进来）。
import { useState } from "react";
import { Button } from "@/components/ui/button";
import { usageErrMsg } from "@/components/usage/usageErrors";
import { usageExportCsv } from "@/lib/api/usage";
import { buildTextSummary, csvFilename } from "@/lib/usage/exportText";
import { copyText, openContainingDir, saveText } from "@/lib/usage/saveFile";
import { useExportShare } from "@/lib/usage/useExportShare";
import type { TFn } from "@/lib/usage/range";
import type { UsageDashboard, UsageFilters, UsageGroupBy, UsageRange } from "@/types/usage";

/** 页签：与页面同形的两个字面量（不 import 页面的私有类型，避免组件反向依赖页面） */
type ExportTab = "board" | "records";

export function UsageExportActions(props: {
  /** 看板数据；`null` = 查询未就绪 / 记录页签（三个看板口径按钮据此禁用，CSV 不依赖它） */
  dash: UsageDashboard | null;
  range: UsageRange;
  tab: ExportTab;
  /**
   * 导出筛选。**由页面按当前页签给**（2026-10-06 用户裁决「CSV 要接收记录页的筛选条件」）：
   * * 记录页签 → `useRecordsFilters` 的**第三层守卫产物**（工具 chips + 子代理模式；日档不含
   *   `subagentMode`）—— 屏幕上筛什么，这份文件就是什么；
   * * 看板页签 → `{}`（看板没有筛选控件）。
   * 本组件**只透传**，不自行拼装、也不加第二层筛选。
   */
  filters: UsageFilters;
  groupBy: UsageGroupBy;
  t: TFn;
}) {
  const { t, tab, dash, range, groupBy, filters } = props;
  /** 行内反馈（成功与失败共用一条通路：后一条覆盖前一条） */
  const [note, setNote] = useState("");
  /** 落盘绝对路径；空串 = 没有可定位的文件（失败态与初始态都是它；复制图片**不**置它） */
  const [savedPath, setSavedPath] = useState("");
  /** 分享图（出图 + 落盘 / 出图 + 剪贴板）：两个动作都是点击时才干活，挂载期零 IPC（见 hook 注释） */
  const share = useExportShare(t);

  const onCopyText = async () => {
    if (!dash) return;
    const r = await copyText(buildTextSummary(dash, t));
    // 复制失败没有专属文案（`usage.export.*` 只有 savedTo / saveFailed 两条结果键）：如实报原因，
    // 不谎报「已复制」——那是这条路径最坏的失败形态（用户以为粘得到，实际剪贴板里是旧内容）
    setNote(r.ok ? t("usage.export.copied") : t("usage.export.saveFailed", { error: r.error }));
  };

  const onCsv = async () => {
    let csv: string;
    try {
      csv = await usageExportCsv(range, groupBy, filters);
    } catch (e) {
      // 用量命令：结构化 `{ code, detail }` → 码表文案（§3 第 28 条）
      setSavedPath("");
      setNote(t("usage.export.saveFailed", { error: usageErrMsg(e, t) }));
      return;
    }
    // ⚠️ **CSV 是「没量到 token 就不显示数值」这条裁决的明确例外**（2026-10-06）：它是**数据文件**
    // ——11 列原始整数、给 Excel / 脚本吃的，**保留原始 0**，不做展示态替换。理由：把 `—` 写进数字列会
    // 让下游（求和、透视、脚本）整列变成文本、并且把「0」与「没有这一列」混为一谈；而展示面
    // （看板 / 浮窗 / 记录页 / 文本摘要 / 分享图）才需要 `—`。CSV 的内容**全部由后端生成**
    // （`usageExportCsv` → `usage_export_csv`），前端只给文件名并落盘，故这条口径的落点在后端与这里。
    const r = await saveText(csv, csvFilename(range));
    if (r.ok) {
      setSavedPath(r.path);
      // 落盘绝对路径**可见**（用户可以自己复制；「打开所在目录」只是便捷入口）
      setNote(t("usage.export.savedTo", { path: r.path }));
    } else {
      // 落盘命令：自由文本原因 → 原样透传（**不得**走 usageErrMsg）
      setSavedPath("");
      setNote(t("usage.export.saveFailed", { error: r.error }));
    }
  };

  const onImage = async () => {
    if (!dash) return;
    try {
      const r = await share.exportImage(dash);
      if (r.ok) {
        setSavedPath(r.path);
        setNote(t("usage.export.savedTo", { path: r.path }));
      } else {
        // 落盘失败：自由文本原因 → 原样透传（与 `onCsv` 同款）
        setSavedPath("");
        setNote(t("usage.export.saveFailed", { error: r.error }));
      }
    } catch (e) {
      // 出图这条链**会**抛（渲染 / `toBlob` 失败都走 rejection）：处理器是 `onClick={() => void onX()}`，
      // `void` 会把 rejection 吞掉 —— 没有这个 catch 就只剩控制台 unhandled rejection、界面「点了没反应」
      setSavedPath("");
      setNote(t("usage.export.saveFailed", { error: String(e) }));
    }
  };

  const onCopyImage = async () => {
    if (!dash) return;
    try {
      const r = await share.copyImage(dash);
      if (r.ok) {
        // best-effort 副本：成功**也不**置 `savedPath` —— 剪贴板里没有路径，没什么可「打开所在目录」的
        setNote(t("usage.export.copiedImage"));
      } else {
        setNote(t("usage.export.saveFailed", { error: r.error }));
      }
    } catch (e) {
      // 与 `onImage` 同因：`void` 不吞错误，失败必须行内可见
      setNote(t("usage.export.saveFailed", { error: String(e) }));
    }
  };

  const onOpenDir = async () => {
    if (!savedPath) return;
    try {
      await openContainingDir(savedPath);
    } catch (e) {
      // 定位失败不改「导出成功」的结论（文件已在磁盘上），但入口撤掉 + 如实报原因：
      // 留着入口只会让用户反复点、反复失败（开发机设了 MAM_HOME 时导出目录会重定向，正会走到这里）
      setSavedPath("");
      setNote(t("usage.export.saveFailed", { error: String(e) }));
    }
  };

  return (
    <div className="flex flex-wrap items-center gap-2">
      {tab === "board" ? (
        <Button
          size="sm"
          variant="outline"
          data-testid="usage-export-copy-text"
          disabled={!dash}
          onClick={() => void onCopyText()}
        >
          {t("usage.export.copyText")}
        </Button>
      ) : null}

      {tab === "board" ? (
        <>
          <Button
            size="sm"
            variant="outline"
            data-testid="usage-export-image"
            disabled={!dash}
            onClick={() => void onImage()}
          >
            {t("usage.export.image")}
          </Button>
          <Button
            size="sm"
            variant="outline"
            data-testid="usage-export-copy-image"
            disabled={!dash}
            onClick={() => void onCopyImage()}
          >
            {t("usage.export.copyImage")}
          </Button>
        </>
      ) : null}

      <Button
        size="sm"
        variant="outline"
        data-testid="usage-export-csv"
        onClick={() => void onCsv()}
      >
        {t("usage.export.csv")}
      </Button>

      {savedPath ? (
        <Button
          size="sm"
          variant="ghost"
          data-testid="usage-export-open-dir"
          onClick={() => void onOpenDir()}
        >
          {t("usage.export.openDir")}
        </Button>
      ) : null}

      {note ? (
        <span
          role="status"
          data-testid="usage-export-toast"
          className="text-muted-foreground max-w-full text-xs break-all"
        >
          {note}
        </span>
      ) : null}
    </div>
  );
}
