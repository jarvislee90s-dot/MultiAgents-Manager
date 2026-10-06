// 设置页「用量统计」分区（计划② Task 14）：落地说明书 §P7 的 8 项设置。
//
// 与计划① 的 `UsageStatusSection` **分工**（契约 §5 / 计划 §3 第 43 条）：那一节是「采集状态 + 立即采集」
// 的验收面（id `usageStatus`），本节是**设置面**（id `usage`，图标 `BarChart3`）——8 项里**不含**
// 「采集状态 / 立即采集」，重复实现即越界。
//
// 八项（每项 `onChange` **即时保存**）：
//  1 `enabled`               ui/switch 开关（总开关；后端关闭时零扫描）
//  2 `miniBarRange`          原生 `<select>`：当日 / 近 5 小时 / 近 7 天
//  3 `miniBarToolRows`       `inputMode="numeric"` + 手动过滤数字，夹取 **1–7**（默认 3；上限 7 是
//                            浮窗可用高度的硬约束，与 D20 的固定 5 行共同保证不越界）
//  4 `detailRetentionDays`   同上，夹取 **1–3650**；提示明示「N 天前无明细」
//  5 `collectIntervalMin`    同上，夹取 **1–1440**
//  6 `providerMapRules`      JSON 文本域，形态固定 `{"rules":[{"prefix":…,"provider":…}]}`
//  7 `exportQuote`           文本框，占位符提示 `{range} {tokens} {hitPct} {models}`
//  8 `exportPose`            原生 `<select>`，选项 = `poseKeysFor(11)` 的 11 项（键族**唯一**出处
//                            是 Task 12 的 `sheet.ts`，**不得**自造第二份键表；`look` 项保留——
//                            9 行图集上它回落待机，文案里写明）
//
// 三条纪律：
//  * **读**只用 `useUsageSettingsQuery`（用量设置的唯一入口，键在 `USAGE_KEY` 前缀下）；**写**用
//    `usageSetSettings(patch)`，成功后失效整个 `["usage"]` 前缀并把**返回的合并对象**回写本地态
//    （后端 `merge_patch` 返回的是完整 8 字段，本地 patch 合并会把别处的改动丢掉）。受控控件在
//    IPC 往返期间先乐观置位（不然打字有延迟），往返结束再以合并对象为准。
//  * **校验只提示不阻断**：规则判据与后端 `merge_patch`（`services/usage/settings.rs` →
//    `provider::parse_provider_rules`）**同判据**——空串合法；否则 JSON 必须能反序列化成
//    `{rules:[{prefix,provider}]}`（整篇全有或全无，serde 语义）且滤掉空 prefix/provider 后仍非空。
//    不合法只显示 `providerMapRulesInvalid`，patch **照发**；真正的裁决在后端，整包拒绝时原因经
//    `usage-set-error` / `settings.usage.saveFailed` 显示（**不静默**）。
//  * **换宠物 → 姿态回退**：姿态键按图集行数给出（`look` 只在 11 行图集上存在），换宠物后沿用旧姿态
//    可能落到不存在的行。订阅 `subscribeConfig`（同窗口局部事件 + 跨窗口 storage 两条路径），激活
//    指针真的变了且当前姿态非 `random` → 回退 `random` 并提示 `poseResetOnSwitch`；已是 `random`
//    则不重复提交也不打扰。
//
// ⚠️ 仓内没有数字输入组件（`type="number"` 全仓零命中）→ 一律 `inputMode="numeric"` + `onChange`
// 手动过滤数字，**过滤后仍要夹取**（夹取是测试断言的那一层）；也没有 shadcn Select → 用原生
// `<select>`。零新增前端依赖、零新增 Tauri 命令、不改 Rust。
//
// ⚠️ 本文件**刻意不写**轮询字段的两个字面量（具体拼法与扫描面见 `src/lib/query/queries/usage.ts`
// 文件头的源码锁说明）：① 的源码锁按纯文本扫描「含轮询字段字面量的前端文件」，并要求这类文件里不出现
// 用量命令的 snake_case 字面量。本文件不写那两个字面量，就不会被卷进那张清单；命令名一律用
// `src/lib/api/usage.ts` 的 camelCase 包装名称呼（`usageGetSettings` / `usageSetSettings`）。
import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { Button } from "@/components/ui/button";
import { useAppTranslation } from "@/hooks/use-app-translation";
import { usageErrMsg } from "@/components/usage/usageErrors";
import { USAGE_KEY, useUsageSettingsQuery } from "@/lib/query/queries/usage";
import { usageSetSettings } from "@/lib/api/usage";
import { subscribeConfig } from "@/components/pet/petConfig";
import { loadActiveId } from "@/components/pet/petRuntime";
import { poseKeysFor } from "@/lib/usage/sheet";
import type { UsageSettings, UsageSettingsPatch } from "@/types/usage";

/** 浮窗口径三档（顺序即下拉顺序；键族复用 `UsageSettings["miniBarRange"]`，不另立类型） */
const RANGE_OPTIONS: { value: UsageSettings["miniBarRange"]; key: string }[] = [
  { value: "today", key: "settings.usage.rangeToday" },
  { value: "last5h", key: "settings.usage.rangeLast5h" },
  { value: "last7d", key: "settings.usage.rangeLast7d" },
];

/** 数值域（与后端 `merge_patch` 的白名单**同值**；越界那边是整包拒绝，不是在服务端静默夹取） */
const TOOL_ROWS_RANGE = [1, 7] as const;
const RETENTION_RANGE = [1, 3650] as const;
const INTERVAL_RANGE = [1, 1440] as const;

/**
 * 手动过滤 + 夹取：先滤掉非数字，再夹进 `[min, max]`。空串（用户清空重打）返回 `null` —— 调用方
 * 不发 patch、也不改本地态（把「什么都没输入」当 0 或 1 提交是编造用户意图）。
 */
function clampDigits(raw: string, min: number, max: number): number | null {
  const digits = raw.replace(/\D/g, "");
  if (digits === "") return null;
  return Math.min(max, Math.max(min, Number(digits)));
}

/** serde 语义下的单条规则：`prefix` / `provider` 必须都是字符串（缺键或数字 → 整篇反序列化失败） */
function isProviderRule(v: unknown): v is { prefix: string; provider: string } {
  if (typeof v !== "object" || v === null) return false;
  const { prefix, provider } = v as { prefix?: unknown; provider?: unknown };
  return typeof prefix === "string" && typeof provider === "string";
}

/**
 * 规则文本的**提示用**判据，与后端 `merge_patch` 同判据（差别只在「返回布尔」与「返回 Err」）：
 *  * 空串合法（清空规则；后端 `parse_provider_rules` 对空串早退空表）；
 *  * 否则整篇 JSON 必须能反序列化成 `{rules:[{prefix, provider}]}`（serde 对 `Vec<ProviderRule>`
 *    是全有或全无：任一元素缺字段/类型不符 → 整篇失败 → 空表）；
 *  * 再按 `retain` 语义滤掉空 `prefix` / 空 `provider`，剩下的**必须非空**（空表在后端同样报错）。
 * 只用于提示，`false` **不阻断提交**（真正的裁决在后端，拒绝原因经 `usage-set-error` 显示）。
 */
function providerRulesValid(text: string): boolean {
  if (text.trim() === "") return true;
  let doc: unknown;
  try {
    doc = JSON.parse(text);
  } catch {
    return false;
  }
  if (typeof doc !== "object" || doc === null || Array.isArray(doc)) return false;
  const rules = (doc as { rules?: unknown }).rules ?? [];
  if (!Array.isArray(rules) || !rules.every(isProviderRule)) return false;
  return rules.some((r) => r.prefix.trim() !== "" && r.provider.trim() !== "");
}

/** 行式设置项（左标题 + 提示，右控件）：与设置页其它分区的行距/字号一致；`htmlFor` 关联右列控件 */
function SettingRow({
  label,
  hint,
  htmlFor,
  children,
}: {
  label: string;
  hint?: ReactNode;
  htmlFor?: string;
  children: ReactNode;
}) {
  return (
    <div className="flex items-center justify-between gap-4 py-2.5">
      <div className="flex-1">
        <label className="text-sm font-medium" htmlFor={htmlFor}>
          {label}
        </label>
        {hint ? <p className="text-muted-foreground mt-0.5 text-xs">{hint}</p> : null}
      </div>
      <div className="flex flex-none items-center gap-2">{children}</div>
    </div>
  );
}

export function UsageSection() {
  const { t } = useAppTranslation();
  const queryClient = useQueryClient();
  const settingsQuery = useUsageSettingsQuery();
  /** 本地表单态：首次读到设置即落地，之后由**每次写入返回的合并对象**驱动（见 `save`） */
  const [form, setForm] = useState<UsageSettings | null>(null);
  const [saveError, setSaveError] = useState<unknown>(null);
  /** 换宠物回退姿态的一次性提示（用户下次手动选姿态时清掉） */
  const [poseReset, setPoseReset] = useState(false);
  /** 写入序号：只让**最后一次**写入的合并对象回写本地态（连续改动时旧响应不得覆盖新值） */
  const seqRef = useRef(0);

  useEffect(() => {
    // 一次性落地：之后本地态由写入返回的合并对象驱动，查询重取（失效后）不再覆盖用户正在编辑的值
    if (settingsQuery.data && form === null) setForm(settingsQuery.data);
  }, [settingsQuery.data, form]);

  const save = useCallback(
    async (patch: UsageSettingsPatch) => {
      const seq = ++seqRef.current;
      setSaveError(null);
      try {
        const merged = await usageSetSettings(patch);
        // 「以它回写本地态」：后端返回的是合并后的**完整 8 字段**，本地 patch 合并会丢别处的改动
        if (seq === seqRef.current) setForm(merged);
        await queryClient.invalidateQueries({ queryKey: USAGE_KEY });
      } catch (e) {
        // 整包拒绝**不静默**：任何一次失败都显示原因（不做「旧响应丢弃」——吞掉失败比多显示一条过期
        // 错误更糟；下一次尝试开头会先清掉它）。原因走 usage.rpc.<code> 码表 + detail 通道。
        console.error("usageSetSettings failed:", e);
        setSaveError(e);
      }
    },
    [queryClient]
  );

  /** 即时保存：先乐观置位（受控控件不能等 IPC 往返），再让合并对象收口 */
  const change = (patch: UsageSettingsPatch) => {
    setForm((f) => (f ? { ...f, ...patch } : f));
    void save(patch);
  };

  const onNumberChange = (
    raw: string,
    [min, max]: readonly [number, number],
    key: "miniBarToolRows" | "detailRetentionDays" | "collectIntervalMin"
  ) => {
    const value = clampDigits(raw, min, max);
    if (value === null) return;
    change({ [key]: value } as UsageSettingsPatch);
  };

  // 换宠物 → 姿态回退 `random`（见文件头第三条纪律）。`subscribeConfig` 覆盖同窗口与跨窗口两条路径；
  // 只有**激活指针真的变了**才算换宠物（显隐 / 大小 / 置顶 / 音效等配置变更不触发）。
  const pose = form?.exportPose;
  useEffect(() => {
    let lastId = loadActiveId();
    return subscribeConfig(() => {
      const nextId = loadActiveId();
      if (nextId === lastId) return;
      lastId = nextId;
      if (pose === undefined || pose === "random") return; // 已是随机：不重复提交、也不打扰
      setPoseReset(true);
      setForm((f) => (f ? { ...f, exportPose: "random" } : f));
      void save({ exportPose: "random" });
    });
  }, [pose, save]);

  return (
    <div>
      <h2 className="text-lg font-semibold">{t("settings.usage.title")}</h2>
      <p className="text-muted-foreground mt-1 text-sm">{t("settings.usage.desc")}</p>

      {/* 保存失败：命令级整包拒绝（越界 / 形态不符 / 落库失败）→ 原因必须可见可重试（不静默） */}
      {saveError ? (
        <p className="mt-2 text-sm text-red-500" data-testid="usage-set-error">
          {t("settings.usage.saveFailed")}：{usageErrMsg(saveError, t)}
        </p>
      ) : null}

      {/* 三态：读失败（可见可重试） / 加载中 / 表单。已有表单时不再被「失效后重取的瞬时失败」清空
          （写入自己的失败另有 usage-set-error 显示） */}
      {!form ? (
        settingsQuery.isError ? (
          <div className="mt-2 flex items-center gap-2" data-testid="usage-load-error">
            <p className="text-sm text-red-500">{usageErrMsg(settingsQuery.error, t)}</p>
            <Button variant="outline" size="sm" onClick={() => void settingsQuery.refetch()}>
              {t("common.retry")}
            </Button>
          </div>
        ) : (
          <p className="text-muted-foreground mt-2 text-sm">{t("common.loading")}</p>
        )
      ) : (
        <div className="mt-2 space-y-0">
          <SettingRow
            label={t("settings.usage.enabled")}
            hint={t("settings.usage.enabledHint")}
            htmlFor="usage-enabled"
          >
            <Switch
              id="usage-enabled"
              data-testid="usage-enabled"
              checked={form.enabled}
              onCheckedChange={(v) => change({ enabled: v })}
            />
          </SettingRow>
          <div className="border-t" />

          <SettingRow label={t("settings.usage.miniBarRange")} htmlFor="usage-minibar-range">
            <select
              id="usage-minibar-range"
              data-testid="usage-minibar-range"
              className="bg-background h-8 rounded-md border px-2 text-sm"
              value={form.miniBarRange}
              onChange={(e) =>
                change({ miniBarRange: e.currentTarget.value as UsageSettings["miniBarRange"] })
              }
            >
              {RANGE_OPTIONS.map((o) => (
                <option key={o.value} value={o.value}>
                  {t(o.key)}
                </option>
              ))}
            </select>
          </SettingRow>
          <div className="border-t" />

          <SettingRow
            label={t("settings.usage.miniBarToolRows")}
            hint={t("settings.usage.miniBarToolRowsHint")}
            htmlFor="usage-minibar-tool-rows"
          >
            <Input
              id="usage-minibar-tool-rows"
              data-testid="usage-minibar-tool-rows"
              inputMode="numeric"
              className="h-8 w-16 text-right font-mono text-sm tabular-nums"
              value={form.miniBarToolRows}
              onChange={(e) =>
                onNumberChange(e.currentTarget.value, TOOL_ROWS_RANGE, "miniBarToolRows")
              }
            />
          </SettingRow>
          <div className="border-t" />

          <SettingRow
            label={t("settings.usage.detailRetentionDays")}
            hint={
              // testid：提示必须随值（且随夹取）实时更新 —— 用例 6 的唯一抓手
              <span data-testid="usage-detail-retention-hint">
                {t("settings.usage.detailRetentionHint", { days: form.detailRetentionDays })}
              </span>
            }
            htmlFor="usage-detail-retention-days"
          >
            <Input
              id="usage-detail-retention-days"
              data-testid="usage-detail-retention-days"
              inputMode="numeric"
              className="h-8 w-20 text-right font-mono text-sm tabular-nums"
              value={form.detailRetentionDays}
              onChange={(e) =>
                onNumberChange(e.currentTarget.value, RETENTION_RANGE, "detailRetentionDays")
              }
            />
          </SettingRow>
          <div className="border-t" />

          <SettingRow
            label={t("settings.usage.collectIntervalMin")}
            hint={t("settings.usage.collectIntervalHint")}
            htmlFor="usage-collect-interval-min"
          >
            <Input
              id="usage-collect-interval-min"
              data-testid="usage-collect-interval-min"
              inputMode="numeric"
              className="h-8 w-20 text-right font-mono text-sm tabular-nums"
              value={form.collectIntervalMin}
              onChange={(e) =>
                onNumberChange(e.currentTarget.value, INTERVAL_RANGE, "collectIntervalMin")
              }
            />
          </SettingRow>
          <div className="border-t" />

          {/* 供应商映射规则：JSON 文本域（仓内无 textarea 原语 → 原生元素 + 与 Input 同款主题 token） */}
          <div className="py-2.5">
            <label className="text-sm font-medium" htmlFor="usage-provider-map-rules">
              {t("settings.usage.providerMapRules")}
            </label>
            <p className="text-muted-foreground mt-0.5 text-xs">
              {t("settings.usage.providerMapRulesHint")}
            </p>
            <textarea
              id="usage-provider-map-rules"
              data-testid="usage-provider-map-rules"
              rows={3}
              spellCheck={false}
              className="border-input placeholder:text-muted-foreground focus-visible:border-ring focus-visible:ring-ring/50 dark:bg-input/30 mt-1.5 w-full rounded-md border bg-transparent px-3 py-1.5 font-mono text-xs shadow-xs outline-none focus-visible:ring-[3px]"
              placeholder={t("settings.usage.providerMapRulesPlaceholder")}
              value={form.providerMapRules}
              onChange={(e) => change({ providerMapRules: e.currentTarget.value })}
            />
            {/* 非法只提示不阻断（patch 照发，真正的裁决在后端） */}
            {!providerRulesValid(form.providerMapRules) ? (
              <p className="mt-1 text-xs text-amber-500" data-testid="usage-provider-map-invalid">
                {t("settings.usage.providerMapRulesInvalid")}
              </p>
            ) : null}
          </div>
          <div className="border-t" />

          {/* 分享图评语：留空用默认评语池；可用变量在占位符里点明 */}
          <div className="py-2.5">
            <label className="text-sm font-medium" htmlFor="usage-export-quote">
              {t("settings.usage.exportQuote")}
            </label>
            <Input
              id="usage-export-quote"
              data-testid="usage-export-quote"
              className="mt-1.5 h-8"
              placeholder={t("settings.usage.exportQuotePlaceholder")}
              value={form.exportQuote}
              onChange={(e) => change({ exportQuote: e.currentTarget.value })}
            />
          </div>
          <div className="border-t" />

          {/* 分享图姿态：键族取自 Task 12 的 `poseKeysFor(11)`（**不得**自造第二份）；`look` 项保留 */}
          <SettingRow label={t("settings.usage.exportPose")} htmlFor="usage-export-pose">
            <select
              id="usage-export-pose"
              data-testid="usage-export-pose"
              className="bg-background h-8 rounded-md border px-2 text-sm"
              value={form.exportPose}
              onChange={(e) => {
                setPoseReset(false); // 用户手动选了姿态 → 换宠物那条一次性提示完成使命
                change({ exportPose: e.currentTarget.value });
              }}
            >
              {poseKeysFor(11).map((key) => (
                <option key={key} value={key}>
                  {t(`usage.pose.${key}`)}
                </option>
              ))}
            </select>
          </SettingRow>

          {/* 换宠物回退提示（只在真的发生过回退时出现） */}
          {poseReset ? (
            <p className="text-xs text-amber-500" data-testid="usage-pose-reset">
              {t("settings.usage.poseResetOnSwitch")}
            </p>
          ) : null}
        </div>
      )}
    </div>
  );
}
