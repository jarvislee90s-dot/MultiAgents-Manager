// 用量看板的三态原子（计划② Task 6 步骤 4）——加载 / 错误（可重试）/ 空态。
// 消费方：看板页（本任务）、Task 7 的 `UsageTrendCard`（无点时走 `UsageEmpty`，`emptyLabel` 直接传入）、
// Task 9 / 13 的记录页与浮窗空态。
//
// 纪律：
//  * **文案全由调用方给**（label / title / message / retryLabel）：本文件不碰 i18n，也不猜默认值——
//    空态「暂无数据」与错误态标题各有各的 i18n 键，硬编码会把它们钉死在一个语言上。
//  * 空态**显示文案而不是 0**（spec P8）：这里没有任何数字格式化，填 0 在结构上不可能。
//  * 错误正文由调用方的 `usageErrMsg(e, t)` 译好再进来（结构化 `{code, detail}` 直接 String 会印
//    `[object Object]`，计划② Task 6「关键坑」第 3 条）。
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";

/** 加载态：`role="status"` + `aria-busy`（读屏可感知「正在取数」而不是「空」） */
export function UsageLoading({ label }: { label: string }) {
  return (
    <Card role="status" aria-busy="true" data-testid="usage-loading">
      <CardContent className="text-muted-foreground text-sm">{label}</CardContent>
    </Card>
  );
}

/** 错误态：`role="alert"` + 可见错误正文 + 可点重试（spec P8：不得静默回落成当日口径） */
export function UsageError({
  title,
  message,
  retryLabel,
  onRetry,
}: {
  title: string;
  message: string;
  retryLabel: string;
  onRetry(): void;
}) {
  return (
    <Card role="alert" data-testid="usage-error">
      <CardContent className="space-y-2">
        <p className="text-destructive text-sm font-medium">{title}</p>
        <p className="text-muted-foreground text-sm">{message}</p>
        <Button variant="outline" size="sm" onClick={onRetry}>
          {retryLabel}
        </Button>
      </CardContent>
    </Card>
  );
}

/** 空态：该区间无数据 → 出文案（不显示 0、不留空白） */
export function UsageEmpty({ label }: { label: string }) {
  return (
    <Card data-testid="usage-empty">
      <CardContent className="text-muted-foreground text-sm">{label}</CardContent>
    </Card>
  );
}
