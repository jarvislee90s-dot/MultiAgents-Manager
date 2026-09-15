// 预设编辑弹窗（新建/编辑共用，spec §7.2）：
// - 类型 Radio（通用/工具私有）+ 绑定工具原生 <select>（选项 = 已启用工具；编辑 tool-scope
//   预设时锁定——bound_tool 是原生技能项的归属标识，改绑会破坏预设身份语义）
// - 名称必填 + 描述 textarea（备忘录用途）
// - 套件列表 skill/MCP/plugin 三组复选（数据源 presetExtensions，按 kind 分组）：
//   通用 = 仅 MAM 资源（非 isNative），专属项带徽标但不禁选；
//   工具私有 = MAM 资源全列、「不适配」项置灰 + 原因 title（不适配 = 存在 binding 且
//   exclusiveTools 非空且不含绑定工具），并追加「原生技能」组（isNative && sourceTool === boundTool）
// - 转类型建议：通用模式勾选任一专属资源 → 提示条 + 「转工具私有」快捷按钮
// - 保存：新建 createPreset / 编辑 updatePreset；名称必填 + ≥1 项校验；
//   后端激活守卫（错误串含 PRESET_ACTIVE）→ presets.saveFailedActive toast；
//   成功 invalidate PRESETS_KEY（弹窗自包含，任何入口打开都刷新列表）
import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { useQueryClient } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { toast } from "sonner";
import { Loader2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { formatInvokeError } from "@/lib/invokeError";
import { createPreset, updatePreset } from "@/lib/api/preset";
import { PRESETS_KEY } from "@/lib/query/queries/presets";
import { useEnabledToolsQuery } from "@/lib/query/queries/tools";
import { useResourceBindingsQuery } from "@/lib/query/queries/bindings";
import type { PresetRecord } from "@/types/preset";
import type { ExtensionWithAssignments } from "@/types/extension";

interface PresetEditDialogProps {
  open: boolean;
  /** 编辑目标；null = 新建 */
  preset: PresetRecord | null;
  /** 套件列表数据源（经 ExtensionList → PresetList props 链透传） */
  presetExtensions?: ExtensionWithAssignments[];
  onClose: () => void;
  /** FR-24「存为预设」预填（T8 接线）：scope 预选 tool、绑定工具 = toolId、items 预勾选 */
  prefill?: { toolId: string; items: [string, string][] };
}

/** 选中项内部键：extensionId::kind 复合（预设条目即二元组，复合键可无损往返） */
const itemKey = (extensionId: string, kind: string) => `${extensionId}::${kind}`;

const parseItemKey = (key: string): [string, string] => {
  const i = key.lastIndexOf("::");
  return [key.slice(0, i), key.slice(i + 2)];
};

/** MAM 资源分组顺序与组头文案（组头复用既有 resources.*Count 计数文案，不新增 key） */
const KINDS: { kind: string; countKey: string }[] = [
  { kind: "skill", countKey: "resources.skillsCount" },
  { kind: "mcp", countKey: "resources.mcpsCount" },
  { kind: "plugin", countKey: "resources.pluginsCount" },
];

export function PresetEditDialog({
  open,
  preset,
  presetExtensions = [],
  onClose,
  prefill,
}: PresetEditDialogProps) {
  const { t } = useTranslation();
  const qc = useQueryClient();
  const { data: enabledTools = [] } = useEnabledToolsQuery();
  const { data: bindings = [] } = useResourceBindingsQuery();

  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [scope, setScope] = useState<"universal" | "tool">("universal");
  const [boundTool, setBoundTool] = useState("");
  // 选中集合：extensionId::kind 复合键（编辑回填时含 preset.items 中已不在列表的残留项，保存不丢）
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [saving, setSaving] = useState(false);

  // open 时按用途初始化表单：编辑回填 > FR-24 预填 > 新建空表单
  useEffect(() => {
    if (!open) return;
    if (preset) {
      setName(preset.name);
      setDescription(preset.description);
      setScope(preset.scope);
      setBoundTool(preset.boundTool ?? "");
      setSelected(new Set(preset.items.map((i) => itemKey(i.extensionId, i.kind))));
    } else if (prefill) {
      setName("");
      setDescription("");
      setScope("tool");
      setBoundTool(prefill.toolId);
      setSelected(new Set(prefill.items.map(([id, kind]) => itemKey(id, kind))));
    } else {
      setName("");
      setDescription("");
      setScope("universal");
      setBoundTool("");
      setSelected(new Set());
    }
  }, [open, preset, prefill]);

  // extensionId → 绑定信息（exclusiveTools 为逗号 join 串，与后端存储同形）
  const bindingById = useMemo(() => {
    const m = new Map<string, { exclusive: string[]; reason: string | null }>();
    for (const b of bindings) {
      m.set(b.extensionId, {
        exclusive: b.exclusiveTools
          .split(",")
          .map((s) => s.trim())
          .filter(Boolean),
        reason: b.reason,
      });
    }
    return m;
  }, [bindings]);

  /** 专属资源判定：存在 binding 且 exclusiveTools 非空（通用模式徽标 + 转类型提示的数据源） */
  const isExclusive = (extensionId: string): boolean =>
    (bindingById.get(extensionId)?.exclusive.length ?? 0) > 0;

  /** 不适配判定（工具私有模式）：专属且允许工具不含当前绑定工具 */
  const isMismatched = (ext: ExtensionWithAssignments): boolean => {
    if (scope !== "tool") return false;
    const b = bindingById.get(ext.id);
    return !!b && b.exclusive.length > 0 && !b.exclusive.includes(boundTool);
  };

  // 不适配原因 title：专属原因 + 允许工具列表（resources.binding.* 文案组合）
  const mismatchTitle = (ext: ExtensionWithAssignments): string => {
    const b = bindingById.get(ext.id);
    const tools = b?.exclusive.join(", ") ?? "";
    const reason = b?.reason?.trim();
    return reason
      ? `${t("resources.binding.reason")}: ${reason} · ${t("resources.binding.tools")}: ${tools}`
      : `${t("resources.binding.tools")}: ${tools}`;
  };

  // 转类型建议：通用模式下勾选了任一专属资源
  const showSwitchHint =
    scope === "universal" && [...selected].some((k) => isExclusive(parseItemKey(k)[0]));

  // MAM 资源（非原生）按 kind 分组，空组不渲染
  const mamGroups = KINDS.map(({ kind, countKey }) => ({
    kind,
    countKey,
    items: presetExtensions.filter((e) => e.kind === kind && !e.isNative),
  })).filter((g) => g.items.length > 0);
  // 原生技能组：仅工具私有模式且绑定工具已选时出现
  const nativeItems =
    scope === "tool" && boundTool
      ? presetExtensions.filter((e) => e.isNative && e.sourceTool === boundTool)
      : [];

  const toggle = (key: string, next: boolean) => {
    setSelected((prev) => {
      const s = new Set(prev);
      if (next) s.add(key);
      else s.delete(key);
      return s;
    });
  };

  // 编辑 tool-scope 预设时绑定工具锁定（bound_tool 是原生技能项的归属标识）
  const lockBoundTool = preset !== null && preset.scope === "tool";

  const canSave =
    !!name.trim() && selected.size > 0 && (scope !== "tool" || !!boundTool) && !saving;

  const handleSave = async () => {
    // 校验兜底（保存按钮已 disabled，此处防程序化触发；T9 用例锁：空名不触发 create）
    if (!name.trim()) {
      toast.error(t("presets.nameRequired"));
      return;
    }
    if (selected.size === 0) {
      toast.error(t("presets.selectAtLeastOne"));
      return;
    }
    if (scope === "tool" && !boundTool) return;
    const items = [...selected].map(parseItemKey);
    try {
      setSaving(true);
      if (preset) {
        await updatePreset(
          preset.id,
          name.trim(),
          description,
          scope,
          scope === "tool" ? boundTool : null,
          items
        );
        toast.success(t("presets.saved"));
      } else {
        await createPreset(
          name.trim(),
          items,
          description || undefined,
          scope,
          scope === "tool" ? boundTool : undefined
        );
        toast.success(t("presets.created", { name: name.trim() }));
      }
      await qc.invalidateQueries({ queryKey: PRESETS_KEY });
      // 托盘同步（Task 16）：预设增删/改名后重建托盘预设项；失败静默
      invoke("refresh_tray", { presetsLabel: t("tray.presetsLabel") }).catch(() => {});
      onClose();
    } catch (e) {
      // 后端守卫：预设正在应用中时错误串带 PRESET_ACTIVE 标记（commands/preset.rs）
      const msg = formatInvokeError(e, t);
      if (msg.includes("PRESET_ACTIVE")) toast.error(t("presets.saveFailedActive"));
      else toast.error(t("common.saveFailed", { error: msg }));
    } finally {
      setSaving(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={(v) => !v && onClose()}>
      <DialogContent aria-describedby={undefined} className="max-h-[80vh] max-w-lg overflow-y-auto">
        <DialogHeader>
          <DialogTitle className="text-sm">
            {preset ? t("presets.editTitle") : t("presets.createTitle")}
          </DialogTitle>
        </DialogHeader>

        <div className="space-y-3">
          {/* 类型：通用 / 工具私有（原生 radio） */}
          <div className="flex items-center gap-4">
            <span className="text-xs font-medium">{t("presets.type")}</span>
            <label className="flex cursor-pointer items-center gap-1.5 text-xs">
              <input
                type="radio"
                name="preset-scope"
                checked={scope === "universal"}
                onChange={() => setScope("universal")}
              />
              {t("presets.typeUniversal")}
            </label>
            <label className="flex cursor-pointer items-center gap-1.5 text-xs">
              <input
                type="radio"
                name="preset-scope"
                checked={scope === "tool"}
                onChange={() => setScope("tool")}
              />
              {t("presets.typeTool")}
            </label>
          </div>

          {/* 绑定工具：仅工具私有模式显示且必选；编辑 tool-scope 预设时锁定不可改 */}
          {scope === "tool" && (
            <div className="flex items-center gap-2">
              <label className="shrink-0 text-xs font-medium">{t("presets.boundTool")}</label>
              <select
                className="bg-background h-7 w-full rounded border px-1.5 text-xs disabled:cursor-not-allowed disabled:opacity-60"
                value={boundTool}
                disabled={lockBoundTool}
                onChange={(e) => setBoundTool(e.currentTarget.value)}
              >
                <option value="">--</option>
                {enabledTools.map((tool) => (
                  <option key={tool.id} value={tool.id}>
                    {tool.label}
                  </option>
                ))}
              </select>
            </div>
          )}

          {/* 名称（必填）/ 描述（选填 textarea，备忘录用途） */}
          <div className="space-y-1">
            <Input
              className="h-8 text-xs"
              value={name}
              placeholder={t("presets.namePlaceholder")}
              onChange={(e) => setName(e.currentTarget.value)}
            />
            <textarea
              className="h-16 w-full rounded border px-2 py-1 text-xs"
              value={description}
              placeholder={t("presets.descriptionPlaceholder")}
              onChange={(e) => setDescription(e.currentTarget.value)}
            />
          </div>

          {/* 转类型建议：通用模式勾选了专属资源 → 一键切工具私有（绑定工具需再手选） */}
          {showSwitchHint && (
            <div className="flex items-center justify-between gap-2 rounded border border-orange-500/30 bg-orange-500/10 p-2 text-xs">
              <span>{t("presets.switchToToolScopedHint")}</span>
              <Button
                size="sm"
                variant="outline"
                className="h-6 shrink-0 text-[10px]"
                onClick={() => setScope("tool")}
              >
                {t("presets.typeTool")}
              </Button>
            </div>
          )}

          {/* 套件列表：skill / MCP / plugin 三组 MAM 资源复选 + 工具私有模式的原生技能组 */}
          <div className="space-y-2">
            {mamGroups.map((group) => (
              <div key={group.kind}>
                <h4 className="text-muted-foreground mb-1 text-xs font-medium">
                  {t(group.countKey, { n: group.items.length })}
                </h4>
                <div className="space-y-1">
                  {group.items.map((ext) => {
                    const key = itemKey(ext.id, ext.kind);
                    const mismatched = isMismatched(ext);
                    const exclusive = isExclusive(ext.id);
                    return (
                      <label
                        key={key}
                        className={`flex items-center gap-2 text-xs ${
                          mismatched ? "cursor-not-allowed opacity-50" : "cursor-pointer"
                        }`}
                        title={mismatched ? mismatchTitle(ext) : undefined}
                      >
                        <Checkbox
                          checked={selected.has(key)}
                          disabled={mismatched}
                          onCheckedChange={(v) => toggle(key, v === true)}
                        />
                        <span className="truncate">{ext.name}</span>
                        {/* 专属徽标：仅通用模式展示（可见不禁选）；工具私有模式以置灰 + 原因表达 */}
                        {scope === "universal" && exclusive && (
                          <span className="shrink-0 rounded bg-orange-500/15 px-1.5 py-0.5 text-[10px] text-orange-600">
                            {t("presets.exclusiveBadge")}
                          </span>
                        )}
                      </label>
                    );
                  })}
                </div>
              </div>
            ))}

            {/* 原生技能组：isNative && sourceTool === boundTool 的项（工具私有预设独有） */}
            {scope === "tool" && nativeItems.length > 0 && (
              <div>
                <h4 className="text-muted-foreground mb-1 text-xs font-medium">
                  {t("presets.nativeSkills")}
                </h4>
                <div className="space-y-1">
                  {nativeItems.map((ext) => {
                    const key = itemKey(ext.id, ext.kind);
                    return (
                      <label key={key} className="flex cursor-pointer items-center gap-2 text-xs">
                        <Checkbox
                          checked={selected.has(key)}
                          onCheckedChange={(v) => toggle(key, v === true)}
                        />
                        <span className="truncate">{ext.name}</span>
                      </label>
                    );
                  })}
                </div>
              </div>
            )}
          </div>
        </div>

        <DialogFooter>
          {/* 取消恒可用（saving 中也可关） */}
          <Button size="sm" variant="outline" onClick={onClose}>
            {t("common.cancel")}
          </Button>
          <Button size="sm" onClick={handleSave} disabled={!canSave}>
            {saving && <Loader2 className="h-3 w-3 animate-spin" />}
            {t("common.save")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
