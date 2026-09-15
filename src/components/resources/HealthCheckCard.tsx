// 一致性体检卡片（spec §13 呈现侧，Task 15）：三源聚合——
// ① 账本-磁盘漂移：按工具分组，逐行 a/b/c 处置（c=暂不处理，纯前端收起）+ 组头批量；
//    needs_manual 处置结果行转橙色无按钮（L4 / L2 内容不一致，重新体检也不会消失）
// ② 快照不变量违背：逐条「一键修复」= restorePreset（fixAll，评审追记的专属 key）
// ③ 残留暂存：逐条回移 = restore_stash_entry（失败原样 toast，后端 message 已含原因）
// 无异常时折叠一行 + 「立即体检」（refetch）；标题处角标 = 未决差异总数
import { useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { HeartPulse, RefreshCw, RotateCcw, Undo2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import { formatInvokeError } from "@/lib/invokeError";
import { restorePreset, restoreStashEntry } from "@/lib/api/preset";
import { reconcileItem, reconcileToolBatch } from "@/lib/api/resource";
import { PRESET_HEALTH_KEY, usePresetHealthQuery } from "@/lib/query/queries/health";
import { ACTIVE_PRESETS_KEY, PRESETS_KEY } from "@/lib/query/queries/presets";
import { SSOT_RESOURCES_KEY } from "@/lib/query/queries/resources";
import { useEnabledToolsQuery } from "@/lib/query/queries/tools";
import type { DriftItem, StashEntryRecord } from "@/types/preset";

// 行键：同一 (tool, extension) 的漂移互斥（缺链/占位/多链/外链只居其一），二元组即唯一
const driftKey = (d: Pick<DriftItem, "toolId" | "extensionId">) => `${d.toolId}|${d.extensionId}`;

// 分类徽标：L1-L4 各一色（借 resources.health.L1-L4 文案）
function KindBadge({ kind, label }: { kind: DriftItem["kind"]; label: string }) {
  const styles: Record<DriftItem["kind"], string> = {
    L1: "bg-red-500/10 text-red-500",
    L2: "bg-amber-500/10 text-amber-500",
    L3: "bg-sky-500/10 text-sky-500",
    L4: "bg-purple-500/10 text-purple-500",
  };
  return (
    <span
      className={cn(
        "shrink-0 rounded px-1.5 py-0.5 text-[10px] font-medium",
        styles[kind] ?? "bg-muted text-muted-foreground"
      )}
    >
      {kind} {label}
    </span>
  );
}

export function HealthCheckCard() {
  const { t } = useTranslation();
  const qc = useQueryClient();
  const healthQuery = usePresetHealthQuery();
  const enabledToolsQuery = useEnabledToolsQuery();
  const drift = healthQuery.data?.drift ?? [];
  const invariants = healthQuery.data?.invariants ?? [];
  const stashPending = healthQuery.data?.stashPending ?? [];
  const hasIssues = drift.length + invariants.length + stashPending.length > 0;

  // c=暂不处理：纯前端收起集合（不调后端）；needs_manual：处置返回升级人工的行集合（转橙无按钮）
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const [manual, setManual] = useState<Set<string>>(new Set());
  // 在途态：行级 / 组头批量 / 不变量修复 / 暂存回移 各自独立，防并发点击
  const [rowPending, setRowPending] = useState<string | null>(null);
  const [batchPending, setBatchPending] = useState<string | null>(null);
  const [invPending, setInvPending] = useState<string | null>(null);
  const [stashPendingId, setStashPendingId] = useState<number | null>(null);

  // 处置后统一失效：体检三源 + 预设 / 激活预设 / SSOT 资源（账本回写与链接重建都影响这些视图）
  const invalidateAfterFix = async () => {
    await qc.invalidateQueries({ queryKey: PRESET_HEALTH_KEY });
    await qc.invalidateQueries({ queryKey: PRESETS_KEY });
    await qc.invalidateQueries({ queryKey: ACTIVE_PRESETS_KEY });
    await qc.invalidateQueries({ queryKey: SSOT_RESOURCES_KEY });
    // 托盘同步（Task 16）：处置可能改动激活预设，重建托盘选中态；失败静默
    invoke("refresh_tray", { presetsLabel: t("tray.presetsLabel") }).catch(() => {});
  };

  // 工具是否启用（② 不变量修复前置检查：disabled 工具按钮禁用 + title 提示先启用）
  const toolEnabled = (toolId: string) =>
    (enabledToolsQuery.data ?? []).some((tool) => tool.id === toolId);

  // ① 逐行处置：fixed→成功 toast；needs_manual→行转橙 + 橙色警示 toast（含后端 message）；失败→错误 toast
  const runItem = async (item: DriftItem, mode: "a" | "b") => {
    const key = driftKey(item);
    setRowPending(key);
    try {
      const outcome = await reconcileItem(item, mode);
      if (outcome.needsManual) {
        setManual((prev) => new Set(prev).add(key));
        toast.warning(`${t("resources.health.needsManual")}：${outcome.message}`);
      } else if (outcome.fixed) {
        toast.success(outcome.message);
      } else {
        toast.error(outcome.message);
      }
    } catch (e) {
      toast.error(formatInvokeError(e, t));
    } finally {
      setRowPending(null);
      await invalidateAfterFix();
    }
  };

  // ① 组头批量：结果按 message 前缀 "ext_id @ tool_id" 回映行键标橙，汇总 toast（复用既有计数文案）
  const runBatch = async (toolId: string, mode: "a" | "b") => {
    setBatchPending(`${toolId}|${mode}`);
    try {
      const outcomes = await reconcileToolBatch(toolId, mode);
      const fixed = outcomes.filter((o) => o.fixed).length;
      const nextManual = new Set(manual);
      for (const o of outcomes) {
        const m = /^(\S+) @ (\S+)/.exec(o.message);
        if (o.needsManual && m) nextManual.add(`${m[2]}|${m[1]}`);
      }
      setManual(nextManual);
      if (fixed === outcomes.length) {
        toast.success(t("resources.health.ok"));
      } else {
        toast.warning(t("presets.partialSuccess", { n: fixed, failed: outcomes.length - fixed }));
      }
    } catch (e) {
      toast.error(formatInvokeError(e, t));
    } finally {
      setBatchPending(null);
      await invalidateAfterFix();
    }
  };

  // ② 不变量修复：toolId 取条目文本 '：' 前缀（后端格式 "{tool}：快照存在但无激活预设"）；
  // toast 沿用 PresetList 开关「关」的同款 restore 文案（应用内一致）
  const fixInvariant = async (line: string) => {
    const toolId = line.split("：")[0]?.trim();
    if (!toolId) return;
    setInvPending(toolId);
    try {
      const rr = await restorePreset(toolId);
      toast.success(
        t("presets.restoreDone", { m: rr.restoredMam.length, n: rr.restoredNative.length })
      );
      if (rr.conflicts.length)
        toast.warning(t("presets.restoreConflicts"), { description: rr.conflicts.join("\n") });
    } catch (e) {
      toast.error(formatInvokeError(e, t));
    } finally {
      setInvPending(null);
      await qc.invalidateQueries({ queryKey: PRESET_HEALTH_KEY });
      await qc.invalidateQueries({ queryKey: ACTIVE_PRESETS_KEY });
      // 托盘同步（Task 16）：不变量修复 = restorePreset，托盘选中态随之翻关
      invoke("refresh_tray", { presetsLabel: t("tray.presetsLabel") }).catch(() => {});
    }
  };

  // ③ 暂存回移：成功报落位去向（沿用资源视图 toast.success(路径) 风格）；失败后端已含原因
  const restoreStash = async (entry: StashEntryRecord) => {
    setStashPendingId(entry.id);
    try {
      await restoreStashEntry(entry.id);
      toast.success(`${entry.skillName} → ${entry.originalPath}`);
      await qc.invalidateQueries({ queryKey: PRESET_HEALTH_KEY });
    } catch (e) {
      toast.error(formatInvokeError(e, t));
    } finally {
      setStashPendingId(null);
    }
  };

  // 漂移按 toolId 分组（保持扫描顺序）；组内滤掉已收起行
  const groups: { toolId: string; items: DriftItem[] }[] = [];
  for (const d of drift) {
    const g = groups.find((x) => x.toolId === d.toolId);
    if (g) g.items.push(d);
    else groups.push({ toolId: d.toolId, items: [d] });
  }

  return (
    <div className="space-y-2 rounded-md border p-3">
      {/* 标题行：标题 + 未决差异角标 + 立即体检 */}
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2">
          <HeartPulse className="h-4 w-4" />
          <span className="text-sm font-medium">{t("resources.health.title")}</span>
          {hasIssues && (
            <span
              className="rounded-full bg-amber-500/15 px-1.5 text-[10px] font-medium text-amber-500"
              title={t("resources.health.title")}
            >
              {drift.length + invariants.length + stashPending.length}
            </span>
          )}
        </div>
        <Button
          size="sm"
          variant="outline"
          className="h-7 px-2 text-[10px]"
          onClick={() => void healthQuery.refetch()}
          disabled={healthQuery.isFetching}
        >
          <RefreshCw className={cn("mr-1 h-3 w-3", healthQuery.isFetching && "animate-spin")} />
          {t("resources.health.runNow")}
        </Button>
      </div>

      {!hasIssues ? (
        // 无异常：折叠一行
        <p className="text-muted-foreground text-xs">{t("resources.health.ok")}</p>
      ) : (
        <>
          {/* ① 漂移明细：图例行（四类徽标）+ 按工具分组（组头批量 a/b） */}
          <div className="flex flex-wrap items-center gap-1.5">
            <KindBadge kind="L1" label={t("resources.health.L1")} />
            <KindBadge kind="L2" label={t("resources.health.L2")} />
            <KindBadge kind="L3" label={t("resources.health.L3")} />
            <KindBadge kind="L4" label={t("resources.health.L4")} />
          </div>
          {groups.map((g) => (
            <div key={g.toolId} className="rounded border">
              <div className="flex items-center justify-between gap-2 border-b px-2 py-1">
                <span className="text-xs font-medium">{g.toolId}</span>
                <div className="flex shrink-0 gap-1">
                  <Button
                    size="sm"
                    variant="outline"
                    className="h-6 px-1.5 text-[10px]"
                    disabled={batchPending !== null}
                    onClick={() => void runBatch(g.toolId, "a")}
                  >
                    {t("resources.health.batchA")}
                  </Button>
                  <Button
                    size="sm"
                    variant="outline"
                    className="h-6 px-1.5 text-[10px]"
                    disabled={batchPending !== null}
                    onClick={() => void runBatch(g.toolId, "b")}
                  >
                    {t("resources.health.batchB")}
                  </Button>
                </div>
              </div>
              {g.items.map((item) => {
                const key = driftKey(item);
                if (collapsed.has(key)) return null; // c=暂不处理：纯前端收起
                const isManual = manual.has(key);
                return (
                  <div
                    key={key}
                    className={cn(
                      "flex items-center gap-2 px-2 py-1.5",
                      isManual && "bg-amber-500/5"
                    )}
                  >
                    <KindBadge kind={item.kind} label={t(`resources.health.${item.kind}`)} />
                    <div className="min-w-0 flex-1">
                      <div className="truncate text-xs font-medium">
                        {item.extensionId.replace(/^skill-/, "")}
                      </div>
                      <div className="text-muted-foreground truncate text-[10px]">{item.path}</div>
                    </div>
                    {isManual ? (
                      // needs_manual：橙色态无按钮（MAM 不动手，现场已保留）
                      <span className="shrink-0 rounded bg-amber-500/15 px-1.5 py-0.5 text-[10px] font-medium text-amber-500">
                        {t("resources.health.needsManual")}
                      </span>
                    ) : (
                      <div className="flex shrink-0 gap-1">
                        <Button
                          size="sm"
                          variant="outline"
                          className="h-6 px-1.5 text-[10px]"
                          disabled={rowPending !== null}
                          onClick={() => void runItem(item, "a")}
                        >
                          {t("resources.health.fixA")}
                        </Button>
                        <Button
                          size="sm"
                          variant="outline"
                          className="h-6 px-1.5 text-[10px]"
                          disabled={rowPending !== null}
                          onClick={() => void runItem(item, "b")}
                        >
                          {t("resources.health.fixB")}
                        </Button>
                        <Button
                          size="sm"
                          variant="ghost"
                          className="h-6 px-1.5 text-[10px]"
                          onClick={() => setCollapsed((prev) => new Set(prev).add(key))}
                        >
                          {t("resources.health.fixC")}
                        </Button>
                      </div>
                    )}
                  </div>
                );
              })}
            </div>
          ))}

          {/* ② 快照不变量违背：逐条 + 修复钮（工具未启用 → 禁用 + title 提示先启用） */}
          {invariants.length > 0 && (
            <div className="space-y-1">
              <div className="text-xs font-medium text-amber-500">
                {t("resources.health.invariantBroken")}
              </div>
              {invariants.map((line) => {
                const toolId = line.split("：")[0]?.trim() ?? "";
                const enabled = toolEnabled(toolId);
                return (
                  <div key={line} className="flex items-center gap-2">
                    <span className="min-w-0 flex-1 truncate text-xs text-amber-500">{line}</span>
                    <Button
                      size="sm"
                      variant="outline"
                      className="h-6 shrink-0 px-1.5 text-[10px]"
                      disabled={!enabled || invPending !== null}
                      title={enabled ? undefined : t("errors.toolDisabled", { tools: toolId })}
                      onClick={() => void fixInvariant(line)}
                    >
                      <RotateCcw className="mr-1 h-3 w-3" />
                      {t("resources.health.fixAll")}
                    </Button>
                  </div>
                );
              })}
            </div>
          )}

          {/* ③ 残留暂存：逐条列出（技能名 + 原位路径摘要）+ 回移钮 */}
          {stashPending.length > 0 && (
            <div className="space-y-1">
              <div className="text-xs font-medium text-amber-500">
                {t("resources.health.stashPending")}
              </div>
              {stashPending.map((entry) => (
                <div key={entry.id} className="flex items-center gap-2">
                  <div className="min-w-0 flex-1">
                    <div className="truncate text-xs font-medium">{entry.skillName}</div>
                    <div className="text-muted-foreground truncate text-[10px]">
                      {entry.originalPath}
                    </div>
                  </div>
                  <Button
                    size="sm"
                    variant="outline"
                    className="h-6 shrink-0 px-1.5 text-[10px]"
                    disabled={stashPendingId !== null}
                    onClick={() => void restoreStash(entry)}
                  >
                    <Undo2 className="mr-1 h-3 w-3" />
                    {t("resources.health.restoreStash")}
                  </Button>
                </div>
              ))}
            </div>
          )}
        </>
      )}
    </div>
  );
}
