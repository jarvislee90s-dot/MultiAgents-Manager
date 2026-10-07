// 五档时间范围条（计划② Task 6 步骤 5）。档位顺序**即** `USAGE_PRESETS` 顺序（范围层是唯一出处，
// 本组件不得自行排列或增删档位）。
//
//  * 预设四档（last5h / today / last7d / last30d）**只把档位交出去**：窗口边界（含上一等长周期）
//    由后端裁定，前端不自行推算（契约要点 1 / §3 第 17 条）。
//  * 自定义档出两个 `<input type="date">`：起 `max=止`、止 `min=起` 且 `max=今天`——把 31 天上限的
//    常见形态挡在原生校验层（真正的夹取在页面 `clampCustomRange`，输入框显示的必须是实际查询的区间）。
//  * `clamped` 只在**真被上限截断**时为真（range.ts 的 `clampCustomRange` 判据），提示常驻到用户
//    下次编辑日期（不是一帧即逝）——状态归页面持有，本组件只负责渲染。
//  * 按钮一律 `@/components/ui/button` 原语；日期输入无仓内原语（§3 第 40 条：MAM 无数字输入组件、
//    无 shadcn Select），用原生 input。
import { Button } from "@/components/ui/button";
import { dayKeyOf, USAGE_PRESETS, type TFn } from "@/lib/usage/range";
import type { UsageRangePreset } from "@/types/usage";

export function UsageRangeBar(props: {
  preset: UsageRangePreset;
  custom: { from: string; to: string };
  clamped: boolean;
  t: TFn;
  onSelectPreset(preset: UsageRangePreset): void;
  onCustomChange(custom: { from: string; to: string }): void;
}) {
  const { preset, custom, clamped, t, onSelectPreset, onCustomChange } = props;
  // 止的 `max`：不给出未来日期（查询窗口的「今天」= 宿主本地日，与 range.ts 同源）
  const todayKey = dayKeyOf(new Date());

  return (
    <div className="space-y-2">
      <div className="flex flex-wrap items-center gap-2">
        {USAGE_PRESETS.map((p) => (
          <Button
            key={p}
            size="sm"
            variant={preset === p ? "default" : "outline"}
            aria-pressed={preset === p}
            data-testid={`usage-range-${p}`}
            onClick={() => onSelectPreset(p)}
          >
            {t(`usage.range.${p}`)}
          </Button>
        ))}
      </div>

      {preset === "custom" ? (
        <div className="flex flex-wrap items-center gap-3">
          <label className="text-muted-foreground flex items-center gap-1.5 text-xs">
            {t("usage.range.from")}
            <input
              type="date"
              data-testid="usage-range-from"
              className="bg-background rounded-md border px-2 py-1 text-xs"
              value={custom.from}
              max={custom.to}
              onChange={(e) => onCustomChange({ ...custom, from: e.target.value })}
            />
          </label>
          <label className="text-muted-foreground flex items-center gap-1.5 text-xs">
            {t("usage.range.to")}
            <input
              type="date"
              data-testid="usage-range-to"
              className="bg-background rounded-md border px-2 py-1 text-xs"
              value={custom.to}
              min={custom.from}
              max={todayKey}
              onChange={(e) => onCustomChange({ ...custom, to: e.target.value })}
            />
          </label>
        </div>
      ) : null}

      {clamped ? (
        <p
          role="status"
          data-testid="usage-range-clamped"
          className="text-xs text-amber-600 dark:text-amber-500"
        >
          {t("usage.rangeClamped")}
        </p>
      ) : null}
    </div>
  );
}
