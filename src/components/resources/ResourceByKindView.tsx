import { useState, type CSSProperties, type KeyboardEvent } from "react";
import { useTranslation } from "react-i18next";
import { useQueryClient } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { toast } from "sonner";
import { formatInvokeError } from "@/lib/invokeError";
import { ToolIcon } from "@/components/common/ToolIcon";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  Package,
  Link2,
  Plug,
  Info,
  Trash2,
  FileJson,
  FolderOpen,
  ArrowUpDown,
  ArrowUp,
  ArrowDown,
  ChevronDown,
  ChevronRight,
} from "lucide-react";
import {
  checkSkillTargetType,
  disableSkillForTool,
  enableSkillForTool,
  importMcpToSsot,
  saveMcpConfig,
} from "@/lib/api/resource";
import { useSsotResourcesQuery, SSOT_RESOURCES_KEY } from "@/lib/query/queries/resources";
import { useEnabledToolsQuery, type EnabledTool } from "@/lib/query/queries/tools";
import {
  useResourceBindingsQuery,
  useToolResidentsQuery,
  BINDINGS_KEY,
  TOOL_RESIDENTS_KEY,
} from "@/lib/query/queries/bindings";
import { useToggleMcpMutation } from "@/lib/query/mutations/resources";
import { uninstallResource } from "@/lib/api/manifest";
import { deleteResourceBinding, setResourceBinding, setToolResident } from "@/lib/api/preset";
import { Switch } from "@/components/ui/switch";
import { ManifestInstallDialog } from "./ManifestInstallDialog";
import type { ResourceBinding, SsotResource } from "@/types/extension";

type ResourceKind = "skill" | "mcp" | "plugin";
type SortDir = "none" | "asc" | "desc";

function formatSkillName(name: string): string {
  return name.includes("/") ? name.replace("/", ": ") : name;
}

/** 资源卡上下文的绑定键：与 extensions 表 id 同形（skill-<name> / mcp-<name> / plugin-<name>） */
function bindingKey(kind: ResourceKind, name: string): string {
  return `${kind}-${name}`;
}

/** 绑定的允许工具列表：逗号串拆分 + trim + 去空（与后端 tool_allowed 口径一致） */
function parseExclusiveTools(binding: ResourceBinding): string[] {
  return binding.exclusiveTools
    .split(",")
    .map((s) => s.trim())
    .filter(Boolean);
}

// ===== 行网格对齐（用户反馈：管理的工具一多，行内 flex 挤压换行导致纵列歪斜） =====
// 表头行与每个资源行共用同一网格模板：名字列固定宽 → 每行工具区起点一致；
// 工具区内所有单元格定宽 shrink-0 → 第 i 个工具永远落在同一条纵列上；
// 工具再多也不挤压换行，超出部分由区块容器（overflow-x-auto）横向滚动。
const SECTION_GRID_STYLE: CSSProperties = {
  display: "grid",
  gridTemplateColumns: "220px minmax(0, auto)",
};
const ALL_COL_CLS = "w-[64px] shrink-0"; // 「全部启用」列（表头占位 + 行内按钮同宽）
const TOOL_COL_CLS = "w-[168px] shrink-0"; // 每工具一列：启停钮 + 常驻锁同占一列

/** 表头行：左侧"名字 + 三态排序按钮"，右侧与行内工具列对齐的目录定位按钮。
 *  MCP 打开的是配置文件（FileJson 图标），Skill/插件打开目录（FolderOpen 图标）。 */
function SectionTableHeader(props: {
  kind: ResourceKind;
  tools: EnabledTool[];
  sortDir: SortDir;
  onToggleSort: () => void;
  onOpen: (toolId: string) => void;
}) {
  const { kind, tools, sortDir, onToggleSort, onOpen } = props;
  const { t } = useTranslation();
  const SortIco = sortDir === "asc" ? ArrowUp : sortDir === "desc" ? ArrowDown : ArrowUpDown;
  const isFile = kind === "mcp";
  const Ico = isFile ? FileJson : FolderOpen;
  const tooltip = isFile
    ? (tool: string) => t("resources.openMcpConfig", { tool })
    : (tool: string) => t("resources.openToolDir", { tool, kind });
  return (
    <div
      className="bg-muted/30 mb-1 w-max min-w-full items-center rounded border px-2 py-1"
      style={SECTION_GRID_STYLE}
    >
      <div className="bg-muted sticky left-2 z-10 flex items-center gap-1 border-r pr-2">
        <span className="text-xs font-medium">{t("resources.nameHeader")}</span>
        <Button
          variant="ghost"
          size="sm"
          className={`h-6 px-1.5 text-[10px] ${sortDir !== "none" ? "text-foreground" : "text-muted-foreground"}`}
          title={t("resources.sortByName")}
          aria-label={t("resources.sortByName")}
          onClick={onToggleSort}
        >
          <SortIco className="h-3 w-3" />
          {sortDir !== "none" && <span>{sortDir === "asc" ? "↑" : "↓"}</span>}
        </Button>
      </div>
      <div className="flex flex-nowrap gap-1">
        {/* 占位：与行内"全部启用"按钮列对齐 */}
        <div className={`h-6 ${ALL_COL_CLS}`} />
        {tools.map((tool) => {
          // 能力门：工具不支持该类资源（如 dsh 无 MCP 配置/插件目录）→ 显示「暂不支持」
          const supported =
            kind === "skill" || (kind === "mcp" ? tool.mcpSupported : tool.pluginSupported);
          if (!supported) {
            return (
              <span
                key={tool.id}
                className={`text-muted-foreground/60 flex h-6 items-center text-[10px] ${TOOL_COL_CLS}`}
                title={t("resources.kindNotSupported")}
              >
                {t("resources.kindNotSupported")}
              </span>
            );
          }
          return (
            <Button
              key={tool.id}
              variant="ghost"
              size="sm"
              className={`text-muted-foreground h-6 justify-start text-[10px] ${TOOL_COL_CLS}`}
              title={tooltip(tool.label)}
              aria-label={tooltip(tool.label)}
              onClick={() => onOpen(tool.id)}
            >
              <Ico className="mr-1 h-3 w-3" />
              {tool.label}
            </Button>
          );
        })}
      </div>
    </div>
  );
}

type PendingDisable = {
  skillName: string;
  toolId: string;
  toolLabel: string;
  displayName: string;
  targetType: "symlink" | "native";
};

/** 常驻锁开关（spec §7.4）：每资源行 × 每工具一枚，紧邻启停按钮。
 *  on = 该 (tool, extension) 进入常驻豁免名单——预设应用时免停用/免暂存；
 *  豁免语义在 Rust 侧实现并有测试护航，前端仅传参。
 *  判定走 useToolResidentsQuery（["tool-residents", toolId]）；写入后失效该键回读，
 *  checked 始终以 query 数据为准，失败 toast 后状态自然回弹，无需本地 optimistic。 */
function ResidentLockSwitch(props: { toolId: string; extensionId: string }) {
  const { t } = useTranslation();
  const qc = useQueryClient();
  const { data: residents = [] } = useToolResidentsQuery(props.toolId);
  const resident = residents.includes(props.extensionId);
  // 写入中禁用开关防连点
  const [saving, setSaving] = useState(false);
  const onToggle = async (next: boolean) => {
    setSaving(true);
    try {
      await setToolResident(props.toolId, props.extensionId, next);
      await qc.invalidateQueries({ queryKey: [...TOOL_RESIDENTS_KEY, props.toolId] });
    } catch (e) {
      toast.error(formatInvokeError(e, t));
    } finally {
      setSaving(false);
    }
  };
  return (
    <span className="flex items-center gap-1">
      <Switch
        checked={resident}
        disabled={saving}
        title={t("resources.resident.toggle")}
        aria-label={t("resources.resident.toggle")}
        onCheckedChange={onToggle}
      />
      {resident && (
        <span className="shrink-0 rounded bg-emerald-500/15 px-1 py-0.5 text-[9px] leading-none text-emerald-600 dark:text-emerald-400">
          {t("presets.residentBadge")}
        </span>
      )}
    </span>
  );
}

export function ResourceByKindView() {
  const { t } = useTranslation();
  const qc = useQueryClient();
  // 工具列由后端下发（勾选状态驱动，W5）
  const { data: tools = [] } = useEnabledToolsQuery();
  const { data: resources } = useSsotResourcesQuery();
  const toggleMcp = useToggleMcpMutation();
  const [search, setSearch] = useState("");
  const [pending, setPending] = useState<PendingDisable | null>(null);
  const [dialogOpen, setDialogOpen] = useState(false);
  const [mcpDialogOpen, setMcpDialogOpen] = useState(false);
  const [newMcp, setNewMcp] = useState({ name: "", command: "", args: "", env: "" });
  const [pendingUninstall, setPendingUninstall] = useState<{
    kind: string;
    name: string;
    count: number;
    // initial = 常规卸载确认；sharedLink = .agents 直链引用二级确认（spec §4.4）
    stage: "initial" | "sharedLink";
  } | null>(null);
  const [manifestDlgOpen, setManifestDlgOpen] = useState(false);
  const [manifestPath, setManifestPath] = useState("");
  const [installDlgPath, setInstallDlgPath] = useState<string | null>(null);
  const [installDlgOpen, setInstallDlgOpen] = useState(false);
  // —— 专属绑定编辑弹层（spec §6/§7.4）：徽标点击打开，保存/清除走 set/delete_resource_binding ——
  const [bindingDlgOpen, setBindingDlgOpen] = useState(false);
  const [bindingTarget, setBindingTarget] = useState<{
    extensionId: string;
    displayName: string;
  } | null>(null);
  const [bindingTools, setBindingTools] = useState<string[]>([]);
  const [bindingReason, setBindingReason] = useState("");
  const [bindingSaving, setBindingSaving] = useState(false);
  const [bindingClearing, setBindingClearing] = useState(false);
  const { data: bindings = [] } = useResourceBindingsQuery();
  // 资源能力门：工具 × 资源类型是否支持启停（后端 EnabledTool 标志下发）
  const kindSupported = (tool: EnabledTool, kind: string): boolean =>
    kind === "skill"
      ? tool.skillToggleSupported
      : kind === "mcp"
        ? tool.mcpSupported
        : tool.pluginSupported;
  const bindingOf = (extensionId: string): ResourceBinding | undefined =>
    bindings.find((b) => b.extensionId === extensionId);
  // 专属判定：绑定存在且允许工具列表非空；null = 非专属（通用可迁移）
  const exclusiveToolsOf = (extensionId: string): string[] | null => {
    const b = bindingOf(extensionId);
    if (!b) return null;
    const list = parseExclusiveTools(b);
    return list.length > 0 ? list : null;
  };
  // 不适配判定：专属 && 该工具不在允许列表（行内启停按钮据此置灰）
  const toolExcludedByBinding = (extensionId: string, toolId: string): boolean => {
    const ex = exclusiveToolsOf(extensionId);
    return ex !== null && !ex.includes(toolId);
  };
  // 不适配按钮 title：kindSupported 同款「工具名: …」前缀 + 专属原因 / 允许工具列表（复用既有键，不新增文案）
  const excludedTitle = (extensionId: string, toolLabel: string): string => {
    const reason = bindingOf(extensionId)?.reason?.trim() ?? "";
    const allow = `${t("resources.binding.tools")}: ${exclusiveToolsOf(extensionId)?.join(", ") ?? ""}`;
    return `${toolLabel}: ${reason ? `${reason} · ${allow}` : allow}`;
  };
  // 门控置灰按钮（spec §6/§7.4）：能力门（kindSupported）与专属门（toolExcludedByBinding）
  // 共用同一形态，仅 title 随原因变化——三区六处收敛到此
  const gatedToolButton = (tool: EnabledTool, title: string) => (
    <Button
      key={tool.id}
      disabled
      variant="ghost"
      size="sm"
      className={`text-muted-foreground h-6 justify-start px-2 text-[10px] opacity-40 ${TOOL_COL_CLS}`}
      title={title}
    >
      <ToolIcon toolId={tool.id} size={14} className="mr-1" />
      {tool.label}
    </Button>
  );
  // 名字排序：三态循环（默认扫描序 → 升序 → 降序），三种资源各自独立记忆
  const [sortDirs, setSortDirs] = useState<Record<ResourceKind, SortDir>>({
    skill: "none",
    mcp: "none",
    plugin: "none",
  });

  const toggleSort = (kind: ResourceKind) => {
    setSortDirs((prev) => {
      const order: SortDir[] = ["none", "asc", "desc"];
      const next = order[(order.indexOf(prev[kind]) + 1) % order.length];
      return { ...prev, [kind]: next };
    });
  };

  // 分组折叠（用户反馈：名单可能特别长，翻阅成本高）：段头整行可点收起/展开，
  // 状态存 localStorage——切到「按工具」视图再切回来也不丢
  const COLLAPSED_KEY = "mam.resourceView.collapsed";
  const [collapsed, setCollapsed] = useState<Record<ResourceKind, boolean>>(() => {
    try {
      const raw = localStorage.getItem(COLLAPSED_KEY);
      return raw
        ? (JSON.parse(raw) as Record<ResourceKind, boolean>)
        : { skill: false, mcp: false, plugin: false };
    } catch {
      return { skill: false, mcp: false, plugin: false };
    }
  });
  const toggleCollapsed = (kind: ResourceKind) => {
    setCollapsed((prev) => {
      const next = { ...prev, [kind]: !prev[kind] };
      try {
        localStorage.setItem(COLLAPSED_KEY, JSON.stringify(next));
      } catch {
        // 存储不可用（隐私模式等）→ 仅本会话内生效
      }
      return next;
    });
  };
  // 段头键盘可达：Enter/Space 等价点击
  const onSectionKeyDown = (kind: ResourceKind) => (e: KeyboardEvent) => {
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      toggleCollapsed(kind);
    }
  };

  const applySort = <T extends { name: string }>(items: T[], kind: ResourceKind): T[] => {
    const dir = sortDirs[kind];
    if (dir === "none") return items;
    return [...items].sort((a, b) => {
      const cmp = a.name.localeCompare(b.name, "zh");
      return dir === "asc" ? cmp : -cmp;
    });
  };

  /** 打开工具对应资源位置（skill/plugin = 目录，mcp = 配置文件） */
  const handleOpenResource = async (kind: ResourceKind, toolId: string) => {
    try {
      const path = await invoke<string>("open_tool_resource", { toolId, kind });
      toast.success(path);
    } catch (e) {
      toast.error(t("common.operationFailed", { error: formatInvokeError(e, t) }));
    }
  };

  const refresh = () => qc.invalidateQueries({ queryKey: SSOT_RESOURCES_KEY });

  if (!resources) {
    return <div className="text-muted-foreground py-4 text-xs">{t("common.loading")}</div>;
  }

  const filterFn = (r: { name: string; enabledTools: string[] }) => {
    if (!search.trim()) return true;
    const q = search.trim().toLowerCase();
    return [r.name, ...r.enabledTools].some((x) => x.toLowerCase().includes(q));
  };
  const filteredSkills = applySort(resources.skills.filter(filterFn), "skill");
  const filteredMcp = applySort(resources.mcp.filter(filterFn), "mcp");
  const filteredPlugins = applySort(resources.plugins.filter(filterFn), "plugin");

  const handleToggleMcp = async (name: string, toolId: string, enabled: boolean) => {
    const tool = tools.find((x) => x.id === toolId);
    if (tool && !tool.mcpSupported) {
      toast.info(t("resources.kindNotSupported"));
      return;
    }
    try {
      if (enabled) {
        // 启用前尝试自动导入到 SSOT（如果还未导入）
        try {
          await importMcpToSsot(name);
        } catch (_) {
          // 可能已导入或找不到配置，继续尝试启用
        }
      }
      await toggleMcp.mutateAsync({ mcpName: name, toolId, enabled });
      toast.success(t(enabled ? "resources.enabled" : "resources.disabled", { name }));
      await refresh();
    } catch (e) {
      toast.error(t("common.operationFailed", { error: formatInvokeError(e, t) }));
    }
  };

  const handleToggleAll = async (res: SsotResource, enable: boolean) => {
    let ok = 0;
    let skipped = 0;
    let failed = 0;
    for (const tool of tools) {
      // 能力门：不支持的 (tool, kind) 跳过（如 dsh 的 skill 启停/MCP/插件）
      if (!kindSupported(tool, res.kind)) continue;
      // 专属门（spec §6）：批量「启用」跳过不适配工具（行内按钮已置灰，此处防绕过）；停用不限
      if (
        enable &&
        toolExcludedByBinding(bindingKey(res.kind as ResourceKind, res.name), tool.id)
      ) {
        skipped++;
        continue;
      }
      const isEnabled = res.enabledTools.includes(tool.id);
      if (enable === isEnabled) continue;
      try {
        if (res.kind === "skill") {
          if (enable) {
            await enableSkillForTool(res.name, tool.id);
          } else {
            const ty = await checkSkillTargetType(tool.id, res.name);
            if (ty === "native") {
              skipped++; // 原生目录不批量删除，跳过
              continue;
            }
            await disableSkillForTool(tool.id, res.name);
          }
        } else if (res.kind === "mcp") {
          if (enable) {
            try {
              await importMcpToSsot(res.name);
            } catch (_) {
              /* 已导入 */
            }
          }
          await invoke("toggle_mcp_for_tool", {
            mcpName: res.name,
            toolId: tool.id,
            enabled: enable,
          });
        } else {
          await invoke("toggle_plugin_for_tool", {
            pluginName: res.name,
            toolId: tool.id,
            enabled: enable,
            kind: res.pluginType ?? "file",
          });
        }
        ok++;
      } catch (e) {
        failed++;
        console.error(e);
      }
    }
    if (failed > 0) toast.error(t("resources.batchFailed", { n: failed }));
    if (ok > 0) toast.success(t("resources.batchDone", { ok, skipped }));
    await refresh();
  };

  const handleTogglePlugin = async (
    name: string,
    toolId: string,
    enabled: boolean,
    kind: string
  ) => {
    const tool = tools.find((x) => x.id === toolId);
    if (tool && !tool.pluginSupported) {
      toast.info(t("resources.kindNotSupported"));
      return;
    }
    try {
      await invoke("toggle_plugin_for_tool", { pluginName: name, toolId, enabled, kind });
      toast.success(t(enabled ? "resources.enabled" : "resources.disabled", { name }));
      await refresh();
    } catch (e) {
      toast.error(t("common.operationFailed", { error: formatInvokeError(e, t) }));
    }
  };

  const handleSkillToggle = async (skillName: string, toolId: string, enabled: boolean) => {
    const tool = tools.find((x) => x.id === toolId);
    if (tool && !tool.skillToggleSupported) {
      toast.info(t("resources.kindNotSupported"));
      return;
    }
    if (!enabled) {
      // 灰 → 亮：直接启用
      try {
        await enableSkillForTool(skillName, toolId);
        toast.success(
          t("resources.enabledInTool", {
            name: formatSkillName(skillName),
            tool: tools.find((tool) => tool.id === toolId)?.label,
          })
        );
        await refresh();
      } catch (e) {
        toast.error(t("resources.enableFailed", { error: formatInvokeError(e, t) }));
      }
    } else {
      // 亮 → 灰：先检查类型，再弹窗
      try {
        const targetType = await checkSkillTargetType(toolId, skillName);
        const toolLabel = tools.find((tool) => tool.id === toolId)?.label || toolId;
        setPending({
          skillName,
          toolId,
          toolLabel,
          displayName: formatSkillName(skillName),
          targetType: targetType as "symlink" | "native",
        });
        setDialogOpen(true);
      } catch (e) {
        toast.error(t("resources.checkFailed", { error: formatInvokeError(e, t) }));
      }
    }
  };

  const confirmDisable = async () => {
    if (!pending) return;
    try {
      await disableSkillForTool(pending.toolId, pending.skillName);
      toast.success(
        t("resources.removedFromTool", { name: pending.displayName, tool: pending.toolLabel })
      );
      await refresh();
    } catch (e) {
      toast.error(t("resources.removeFailed", { error: formatInvokeError(e, t) }));
    } finally {
      setDialogOpen(false);
      setPending(null);
    }
  };

  /** 卸载确认：首次调用（force=false）后端检测到 ~/.agents/skills 直链引用时返回
   *  needsConfirmation（零改动），切换二级确认；用户「继续卸载」→ force=true 强制走原流程 */
  const confirmUninstall = async (force: boolean) => {
    if (!pendingUninstall) return;
    try {
      const outcome = await uninstallResource(pendingUninstall.kind, pendingUninstall.name, force);
      if (outcome.needsConfirmation) {
        setPendingUninstall({ ...pendingUninstall, stage: "sharedLink" });
        return;
      }
      toast.success(t("resources.uninstallSuccess", { name: pendingUninstall.name }));
      await refresh();
      setPendingUninstall(null);
    } catch (e) {
      toast.error(t("common.operationFailed", { error: formatInvokeError(e, t) }));
      setPendingUninstall(null);
    }
  };

  const handleAddMcp = async () => {
    if (!newMcp.name.trim() || !newMcp.command.trim()) {
      toast.error(t("mcp.nameAndCommandRequired"));
      return;
    }
    try {
      const args = newMcp.args.trim() ? newMcp.args.split(/\s+/).filter(Boolean) : [];
      const env: Record<string, string> = {};
      if (newMcp.env.trim()) {
        newMcp.env.split("\n").forEach((line) => {
          const idx = line.indexOf("=");
          if (idx > 0) env[line.slice(0, idx).trim()] = line.slice(idx + 1).trim();
        });
      }
      await saveMcpConfig(newMcp.name.trim(), newMcp.command.trim(), args, env);
      toast.success(t("resources.mcpAddedToRepo", { name: newMcp.name }));
      setMcpDialogOpen(false);
      setNewMcp({ name: "", command: "", args: "", env: "" });
      await refresh();
    } catch (e) {
      toast.error(t("resources.addMcpFailed", { error: formatInvokeError(e, t) }));
    }
  };

  /** 打开编辑弹层：以当前 binding 回填（无绑定 = 空表单） */
  const openBindingEditor = (extensionId: string, displayName: string) => {
    const b = bindingOf(extensionId);
    setBindingTarget({ extensionId, displayName });
    setBindingTools(b ? parseExclusiveTools(b) : []);
    setBindingReason(b?.reason ?? "");
    setBindingDlgOpen(true);
  };

  /** 资源名旁专属徽标（spec §6/§7.4）：仅当存在非空 exclusiveTools 绑定时显示；点击打开编辑弹层 */
  const renderExclusiveBadge = (kind: ResourceKind, name: string) => {
    const extensionId = bindingKey(kind, name);
    const ex = exclusiveToolsOf(extensionId);
    if (!ex) return null;
    return (
      <button
        type="button"
        className="shrink-0 cursor-pointer rounded bg-orange-500/15 px-1.5 py-0.5 text-[10px] text-orange-600 hover:bg-orange-500/25"
        title={`${t("presets.exclusiveBadge")}: ${ex.join(", ")}`}
        onClick={() =>
          openBindingEditor(extensionId, kind === "skill" ? formatSkillName(name) : name)
        }
      >
        {t("presets.exclusiveBadge")}
      </button>
    );
  };

  // 保存绑定：空勾选列表 = 清除语义（后端 exclusiveTools 空 = 通用可迁移），直接允许
  const handleBindingSave = async () => {
    if (!bindingTarget || bindingSaving || bindingClearing) return;
    setBindingSaving(true);
    try {
      await setResourceBinding(
        bindingTarget.extensionId,
        bindingTools,
        bindingReason.trim() || undefined
      );
      await qc.invalidateQueries({ queryKey: BINDINGS_KEY });
      // 无「绑定已保存」文案键，按不新增文案约束复用既有键组合提示
      const labels = bindingTools.map((id) => tools.find((x) => x.id === id)?.label ?? id);
      toast.success(
        labels.length > 0
          ? `${t("presets.exclusiveBadge")}: ${labels.join(", ")}`
          : t("common.deleted")
      );
      setBindingDlgOpen(false);
    } catch (e) {
      toast.error(formatInvokeError(e, t));
    } finally {
      setBindingSaving(false);
    }
  };

  // 清除绑定：删除整条绑定记录，恢复通用可迁移
  const handleBindingClear = async () => {
    if (!bindingTarget || bindingSaving || bindingClearing) return;
    setBindingClearing(true);
    try {
      await deleteResourceBinding(bindingTarget.extensionId);
      await qc.invalidateQueries({ queryKey: BINDINGS_KEY });
      toast.success(t("common.deleted"));
      setBindingDlgOpen(false);
    } catch (e) {
      toast.error(formatInvokeError(e, t));
    } finally {
      setBindingClearing(false);
    }
  };

  return (
    <>
      <div className="bg-card rounded-lg border p-4">
        <h3 className="mb-3 text-sm font-semibold">{t("resources.repoTitle")}</h3>

        <div className="mb-3 flex items-center justify-between gap-2">
          <input
            type="text"
            placeholder={t("resources.searchPlaceholder")}
            value={search}
            onChange={(e) => setSearch(e.currentTarget.value)}
            className="h-7 w-40 rounded border px-2 text-xs"
          />
          <Button
            size="sm"
            variant="outline"
            className="h-6 px-2 text-[10px]"
            onClick={() => setManifestDlgOpen(true)}
          >
            <FileJson className="mr-1 h-3 w-3" />
            {t("resources.installFromManifest")}
          </Button>
        </div>

        {/* Skills */}
        <div className="mb-4">
          <h4
            className="mb-2 flex cursor-pointer items-center gap-2 text-sm font-semibold select-none"
            aria-expanded={!collapsed.skill}
            title={collapsed.skill ? t("resources.expandSection") : t("resources.collapseSection")}
            onClick={() => toggleCollapsed("skill")}
            onKeyDown={onSectionKeyDown("skill")}
            tabIndex={0}
            role="button"
          >
            {collapsed.skill ? (
              <ChevronRight className="h-4 w-4" />
            ) : (
              <ChevronDown className="h-4 w-4" />
            )}
            <Package className="h-4 w-4" />
            {t("resources.skillsCount", { n: filteredSkills.length })}
          </h4>
          {collapsed.skill ? null : filteredSkills.length === 0 ? (
            <div className="text-muted-foreground flex items-center gap-2 py-4 text-xs">
              <Info className="h-3.5 w-3.5" />
              {t("resources.noSkillsHint")}
            </div>
          ) : (
            <div className="space-y-1 overflow-x-auto pb-1">
              <SectionTableHeader
                kind="skill"
                tools={tools}
                sortDir={sortDirs.skill}
                onToggleSort={() => toggleSort("skill")}
                onOpen={(toolId) => handleOpenResource("skill", toolId)}
              />
              {filteredSkills.map((skill) => (
                <div
                  key={skill.name}
                  className="w-max min-w-full items-center rounded border p-2 text-sm"
                  style={SECTION_GRID_STYLE}
                >
                  <div className="bg-background sticky left-2 z-10 flex flex-wrap items-center gap-x-1 gap-y-0.5 overflow-hidden border-r">
                    <span className="font-medium">{formatSkillName(skill.name)}</span>
                    {renderExclusiveBadge("skill", skill.name)}
                    {skill.brokenTools && skill.brokenTools.length > 0 && (
                      <span
                        className="rounded bg-amber-500/15 px-1.5 py-0.5 text-[10px] text-amber-500"
                        title={t("resources.linkBrokenTooltip", {
                          tools: skill.brokenTools.join(", "),
                        })}
                      >
                        {t("resources.linkBroken")}
                      </span>
                    )}
                    <Button
                      variant="ghost"
                      size="sm"
                      className="text-destructive h-6 px-1.5 text-[10px]"
                      title={t("resources.uninstall")}
                      aria-label={t("resources.uninstall")}
                      onClick={() =>
                        setPendingUninstall({
                          kind: "skill",
                          name: skill.name,
                          count: skill.enabledTools.length,
                          stage: "initial",
                        })
                      }
                    >
                      <Trash2 className="h-3 w-3" />
                    </Button>
                  </div>
                  <div className="flex flex-nowrap gap-1">
                    <Button
                      variant="ghost"
                      size="sm"
                      className={`h-6 px-1 text-[10px] ${ALL_COL_CLS}`}
                      title={
                        skill.enabledTools.length === tools.length
                          ? t("resources.allToolsOff")
                          : t("resources.allToolsOn")
                      }
                      onClick={() =>
                        handleToggleAll(skill, skill.enabledTools.length !== tools.length)
                      }
                    >
                      {skill.enabledTools.length === tools.length
                        ? t("resources.allToolsOff")
                        : t("resources.allToolsOn")}
                    </Button>
                    {tools.map((tool) => {
                      if (!kindSupported(tool, "skill")) {
                        return gatedToolButton(
                          tool,
                          `${tool.label}: ${t("resources.kindNotSupported")}`
                        );
                      }
                      // 专属门（spec §6/§7.4）：不适配工具置灰不可启停，title 说明原因
                      if (toolExcludedByBinding(bindingKey("skill", skill.name), tool.id)) {
                        return gatedToolButton(
                          tool,
                          excludedTitle(bindingKey("skill", skill.name), tool.label)
                        );
                      }
                      const enabled = skill.enabledTools.includes(tool.id);
                      return (
                        <span key={tool.id} className={`flex items-center gap-1 ${TOOL_COL_CLS}`}>
                          <Button
                            variant={enabled ? "default" : "ghost"}
                            size="sm"
                            className={`h-6 px-2 text-[10px] ${enabled ? "" : "text-muted-foreground opacity-50"}`}
                            title={`${tool.label}: ${enabled ? t("resources.enabledShort") : t("resources.disabledShort")}`}
                            onClick={() => handleSkillToggle(skill.name, tool.id, enabled)}
                          >
                            <ToolIcon toolId={tool.id} size={14} className="mr-1" />
                            {tool.label}
                          </Button>
                          {/* 常驻锁：紧邻启停按钮（spec §7.4） */}
                          <ResidentLockSwitch
                            toolId={tool.id}
                            extensionId={bindingKey("skill", skill.name)}
                          />
                        </span>
                      );
                    })}
                  </div>
                </div>
              ))}
            </div>
          )}
        </div>

        {/* MCP */}
        <div className="mb-4">
          <h4
            className="mb-2 flex cursor-pointer items-center gap-2 text-sm font-semibold select-none"
            aria-expanded={!collapsed.mcp}
            title={collapsed.mcp ? t("resources.expandSection") : t("resources.collapseSection")}
            onClick={() => toggleCollapsed("mcp")}
            onKeyDown={onSectionKeyDown("mcp")}
            tabIndex={0}
            role="button"
          >
            {collapsed.mcp ? (
              <ChevronRight className="h-4 w-4" />
            ) : (
              <ChevronDown className="h-4 w-4" />
            )}
            <Link2 className="h-4 w-4" />
            {t("resources.mcpsCount", { n: filteredMcp.length })}
            <Button
              size="sm"
              variant="ghost"
              className="ml-auto h-6 px-2 text-[10px]"
              onClick={(e) => {
                // 阻断冒泡：添加按钮不应触发分组折叠
                e.stopPropagation();
                setMcpDialogOpen(true);
              }}
            >
              {t("resources.addWithPlus")}
            </Button>
          </h4>
          {collapsed.mcp ? null : filteredMcp.length === 0 ? (
            <div className="text-muted-foreground flex items-center gap-2 py-4 text-xs">
              <Info className="h-3.5 w-3.5" />
              {t("mcp.empty")}
            </div>
          ) : (
            <div className="space-y-1 overflow-x-auto pb-1">
              <SectionTableHeader
                kind="mcp"
                tools={tools}
                sortDir={sortDirs.mcp}
                onToggleSort={() => toggleSort("mcp")}
                onOpen={(toolId) => handleOpenResource("mcp", toolId)}
              />
              {filteredMcp.map((mcp) => (
                <div
                  key={mcp.name}
                  className="w-max min-w-full items-center rounded border p-2 text-sm"
                  style={SECTION_GRID_STYLE}
                >
                  <div className="bg-background sticky left-2 z-10 flex flex-wrap items-center gap-x-1 gap-y-0.5 overflow-hidden border-r">
                    <span className="font-medium">{mcp.name}</span>
                    {renderExclusiveBadge("mcp", mcp.name)}
                    {mcp.sourceDisabled && (
                      <span
                        className="text-muted-foreground rounded border border-dashed px-1 text-[10px]"
                        title={t("resources.mcpSourceDisabledHint")}
                      >
                        {t("resources.mcpSourceDisabled")}
                      </span>
                    )}
                    <Button
                      variant="ghost"
                      size="sm"
                      className="text-destructive h-6 px-1.5 text-[10px]"
                      title={t("resources.uninstall")}
                      aria-label={t("resources.uninstall")}
                      onClick={() =>
                        setPendingUninstall({
                          kind: "mcp",
                          name: mcp.name,
                          count: mcp.enabledTools.length,
                          stage: "initial",
                        })
                      }
                    >
                      <Trash2 className="h-3 w-3" />
                    </Button>
                  </div>
                  <div className="flex flex-nowrap gap-1">
                    <Button
                      variant="ghost"
                      size="sm"
                      className={`h-6 px-1 text-[10px] ${ALL_COL_CLS}`}
                      title={
                        mcp.enabledTools.length === tools.length
                          ? t("resources.allToolsOff")
                          : t("resources.allToolsOn")
                      }
                      onClick={() => handleToggleAll(mcp, mcp.enabledTools.length !== tools.length)}
                    >
                      {mcp.enabledTools.length === tools.length
                        ? t("resources.allToolsOff")
                        : t("resources.allToolsOn")}
                    </Button>
                    {tools.map((tool) => {
                      if (!kindSupported(tool, "mcp")) {
                        return gatedToolButton(
                          tool,
                          `${tool.label}: ${t("resources.kindNotSupported")}`
                        );
                      }
                      // 专属门（spec §6/§7.4）：不适配工具置灰不可启停，title 说明原因
                      if (toolExcludedByBinding(bindingKey("mcp", mcp.name), tool.id)) {
                        return gatedToolButton(
                          tool,
                          excludedTitle(bindingKey("mcp", mcp.name), tool.label)
                        );
                      }
                      const enabled = mcp.enabledTools.includes(tool.id);
                      return (
                        <span key={tool.id} className={`flex items-center gap-1 ${TOOL_COL_CLS}`}>
                          <Button
                            variant={enabled ? "default" : "ghost"}
                            size="sm"
                            className={`h-6 px-2 text-[10px] ${enabled ? "" : "text-muted-foreground opacity-50"}`}
                            onClick={() => handleToggleMcp(mcp.name, tool.id, !enabled)}
                          >
                            <ToolIcon toolId={tool.id} size={14} className="mr-1" />
                            {tool.label}
                          </Button>
                          {/* 常驻锁：紧邻启停按钮（spec §7.4） */}
                          <ResidentLockSwitch
                            toolId={tool.id}
                            extensionId={bindingKey("mcp", mcp.name)}
                          />
                        </span>
                      );
                    })}
                  </div>
                </div>
              ))}
            </div>
          )}
        </div>

        {/* Plugins */}
        <div>
          <h4
            className="mb-2 flex cursor-pointer items-center gap-2 text-sm font-semibold select-none"
            aria-expanded={!collapsed.plugin}
            title={collapsed.plugin ? t("resources.expandSection") : t("resources.collapseSection")}
            onClick={() => toggleCollapsed("plugin")}
            onKeyDown={onSectionKeyDown("plugin")}
            tabIndex={0}
            role="button"
          >
            {collapsed.plugin ? (
              <ChevronRight className="h-4 w-4" />
            ) : (
              <ChevronDown className="h-4 w-4" />
            )}
            <Plug className="h-4 w-4" />
            {t("resources.pluginsCount", { n: filteredPlugins.length })}
          </h4>
          {collapsed.plugin ? null : filteredPlugins.length === 0 ? (
            <div className="text-muted-foreground flex items-center gap-2 py-4 text-xs">
              <Info className="h-3.5 w-3.5" />
              {t("resources.noPlugins")}
            </div>
          ) : (
            <div className="space-y-1 overflow-x-auto pb-1">
              <SectionTableHeader
                kind="plugin"
                tools={tools}
                sortDir={sortDirs.plugin}
                onToggleSort={() => toggleSort("plugin")}
                onOpen={(toolId) => handleOpenResource("plugin", toolId)}
              />
              {filteredPlugins.map((plugin) => (
                <div
                  key={plugin.name}
                  className="w-max min-w-full items-center rounded border p-2 text-sm"
                  style={SECTION_GRID_STYLE}
                >
                  <div className="bg-background sticky left-2 z-10 flex flex-wrap items-center gap-x-1 gap-y-0.5 overflow-hidden border-r">
                    <span className="font-medium">{plugin.name}</span>
                    {renderExclusiveBadge("plugin", plugin.name)}
                    <Button
                      variant="ghost"
                      size="sm"
                      className="text-destructive h-6 px-1.5 text-[10px]"
                      title={t("resources.uninstall")}
                      aria-label={t("resources.uninstall")}
                      onClick={() =>
                        setPendingUninstall({
                          kind: "plugin",
                          name: plugin.name,
                          count: plugin.enabledTools.length,
                          stage: "initial",
                        })
                      }
                    >
                      <Trash2 className="h-3 w-3" />
                    </Button>
                  </div>
                  <div className="flex flex-nowrap gap-1">
                    <Button
                      variant="ghost"
                      size="sm"
                      className={`h-6 px-1 text-[10px] ${ALL_COL_CLS}`}
                      title={
                        plugin.enabledTools.length === tools.length
                          ? t("resources.allToolsOff")
                          : t("resources.allToolsOn")
                      }
                      onClick={() =>
                        handleToggleAll(plugin, plugin.enabledTools.length !== tools.length)
                      }
                    >
                      {plugin.enabledTools.length === tools.length
                        ? t("resources.allToolsOff")
                        : t("resources.allToolsOn")}
                    </Button>
                    {tools.map((tool) => {
                      if (!kindSupported(tool, "plugin")) {
                        return gatedToolButton(
                          tool,
                          `${tool.label}: ${t("resources.kindNotSupported")}`
                        );
                      }
                      // 专属门（spec §6/§7.4）：不适配工具置灰不可启停，title 说明原因
                      if (toolExcludedByBinding(bindingKey("plugin", plugin.name), tool.id)) {
                        return gatedToolButton(
                          tool,
                          excludedTitle(bindingKey("plugin", plugin.name), tool.label)
                        );
                      }
                      const enabled = plugin.enabledTools.includes(tool.id);
                      return (
                        <span key={tool.id} className={`flex items-center gap-1 ${TOOL_COL_CLS}`}>
                          <Button
                            variant={enabled ? "default" : "ghost"}
                            size="sm"
                            className={`h-6 px-2 text-[10px] ${enabled ? "" : "text-muted-foreground opacity-50"}`}
                            onClick={() =>
                              handleTogglePlugin(
                                plugin.name,
                                tool.id,
                                !enabled,
                                plugin.pluginType ?? "file"
                              )
                            }
                          >
                            <ToolIcon toolId={tool.id} size={14} className="mr-1" />
                            {tool.label}
                          </Button>
                          {/* 常驻锁：紧邻启停按钮（spec §7.4） */}
                          <ResidentLockSwitch
                            toolId={tool.id}
                            extensionId={bindingKey("plugin", plugin.name)}
                          />
                        </span>
                      );
                    })}
                  </div>
                </div>
              ))}
            </div>
          )}
        </div>
      </div>

      {/* 确认弹窗 */}
      <Dialog open={dialogOpen} onOpenChange={setDialogOpen}>
        <DialogContent className="max-w-sm">
          {pending?.targetType === "native" ? (
            <>
              <DialogHeader>
                <DialogTitle className="text-red-600">
                  {t("resources.deleteNativeTitle")}
                </DialogTitle>
                <DialogDescription className="space-y-2 pt-2 text-sm">
                  <p className="text-red-500">
                    {t("resources.deleteNativeDesc1", { name: pending?.displayName })}
                  </p>
                  <p>{t("resources.deleteNativeDesc2", { tool: pending?.toolLabel })}</p>
                </DialogDescription>
              </DialogHeader>
              <DialogFooter className="gap-2">
                <Button
                  variant="outline"
                  size="sm"
                  onClick={() => {
                    setDialogOpen(false);
                    setPending(null);
                  }}
                >
                  {t("common.cancel")}
                </Button>
                <Button variant="destructive" size="sm" onClick={confirmDisable}>
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
                <Button
                  variant="outline"
                  size="sm"
                  onClick={() => {
                    setDialogOpen(false);
                    setPending(null);
                  }}
                >
                  {t("common.cancel")}
                </Button>
                <Button variant="default" size="sm" onClick={confirmDisable}>
                  {t("resources.removeLink")}
                </Button>
              </DialogFooter>
            </>
          )}
        </DialogContent>
      </Dialog>

      {/* 添加 MCP 弹窗 */}
      <Dialog open={mcpDialogOpen} onOpenChange={setMcpDialogOpen}>
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
                value={newMcp.name}
                onChange={(e) => setNewMcp({ ...newMcp, name: e.currentTarget.value })}
                placeholder="firecrawl"
                className="h-8 w-full rounded border px-2 text-xs"
              />
            </div>
            <div>
              <label className="text-xs font-medium">{t("mcp.commandLabel")}</label>
              <input
                value={newMcp.command}
                onChange={(e) => setNewMcp({ ...newMcp, command: e.currentTarget.value })}
                placeholder="npx"
                className="h-8 w-full rounded border px-2 text-xs"
              />
            </div>
            <div>
              <label className="text-xs font-medium">{t("mcp.argsLabelSpace")}</label>
              <input
                value={newMcp.args}
                onChange={(e) => setNewMcp({ ...newMcp, args: e.currentTarget.value })}
                placeholder="-y firecrawl-mcp"
                className="h-8 w-full rounded border px-2 text-xs"
              />
            </div>
            <div>
              <label className="text-xs font-medium">{t("mcp.envLabel")}</label>
              <textarea
                value={newMcp.env}
                onChange={(e) => setNewMcp({ ...newMcp, env: e.currentTarget.value })}
                placeholder="API_KEY=xxx"
                className="h-16 w-full rounded border px-2 text-xs"
              />
            </div>
          </div>
          <DialogFooter className="gap-2">
            <Button
              variant="outline"
              size="sm"
              onClick={() => {
                setMcpDialogOpen(false);
                setNewMcp({ name: "", command: "", args: "", env: "" });
              }}
            >
              {t("common.cancel")}
            </Button>
            <Button size="sm" onClick={handleAddMcp}>
              {t("resources.addToRepo")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* 卸载确认弹窗（stage=sharedLink 为 ~/.agents/skills 直链引用二级确认，spec §4.4；
          取消/关闭不产生任何变更） */}
      <Dialog open={!!pendingUninstall} onOpenChange={(o) => !o && setPendingUninstall(null)}>
        <DialogContent className="max-w-sm">
          {pendingUninstall?.stage === "sharedLink" ? (
            <>
              <DialogHeader>
                <DialogTitle className="text-red-600">
                  {t("resources.sharedLinkConfirmTitle")}
                </DialogTitle>
                <DialogDescription className="pt-2 text-sm">
                  {t("resources.sharedLinkConfirmDesc", { name: pendingUninstall?.name })}
                </DialogDescription>
              </DialogHeader>
              <DialogFooter className="gap-2">
                <Button variant="outline" size="sm" onClick={() => setPendingUninstall(null)}>
                  {t("common.cancel")}
                </Button>
                <Button variant="destructive" size="sm" onClick={() => confirmUninstall(true)}>
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
                    name: pendingUninstall?.name,
                    n: pendingUninstall?.count ?? 0,
                  })}
                </DialogDescription>
              </DialogHeader>
              <DialogFooter className="gap-2">
                <Button variant="outline" size="sm" onClick={() => setPendingUninstall(null)}>
                  {t("common.cancel")}
                </Button>
                <Button variant="destructive" size="sm" onClick={() => confirmUninstall(false)}>
                  {t("resources.uninstall")}
                </Button>
              </DialogFooter>
            </>
          )}
        </DialogContent>
      </Dialog>

      {/* 从 Manifest 安装路径弹窗 */}
      <Dialog open={manifestDlgOpen} onOpenChange={setManifestDlgOpen}>
        <DialogContent className="max-w-md">
          <DialogHeader>
            <DialogTitle>{t("resources.installFromManifest")}</DialogTitle>
          </DialogHeader>
          <div className="py-2">
            <label className="text-xs font-medium">{t("resources.manifestPathLabel")}</label>
            <input
              value={manifestPath}
              onChange={(e) => setManifestPath(e.currentTarget.value)}
              placeholder={t("resources.manifestPathPlaceholder")}
              className="h-8 w-full rounded border px-2 text-xs"
            />
          </div>
          <DialogFooter className="gap-2">
            <Button variant="outline" size="sm" onClick={() => setManifestDlgOpen(false)}>
              {t("common.cancel")}
            </Button>
            <Button
              size="sm"
              disabled={!manifestPath.trim()}
              onClick={() => {
                setInstallDlgPath(manifestPath.trim());
                setInstallDlgOpen(true);
                setManifestDlgOpen(false);
              }}
            >
              {t("common.confirm")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <ManifestInstallDialog
        path={installDlgPath}
        open={installDlgOpen}
        onOpenChange={setInstallDlgOpen}
        onInstalled={async () => {
          try {
            await refresh();
          } catch (e) {
            toast.error(t("common.operationFailed", { error: formatInvokeError(e, t) }));
          }
        }}
      />

      {/* 专属绑定编辑弹层（spec §6/§7.4）：工具复选（仅已启用工具，勾选 = exclusiveTools 成员）
          + 原因备注；保存空列表 = 清除语义；清除/保存进行中互相禁用，取消恒可用 */}
      <Dialog open={bindingDlgOpen} onOpenChange={(o) => !o && setBindingDlgOpen(false)}>
        <DialogContent className="max-w-sm">
          <DialogHeader>
            <DialogTitle>{t("resources.binding.editTitle")}</DialogTitle>
            <DialogDescription className="pt-1 text-xs">
              {bindingTarget?.displayName}
            </DialogDescription>
          </DialogHeader>
          <div className="space-y-3 py-2">
            <div>
              <label className="text-xs font-medium">{t("resources.binding.tools")}</label>
              <div className="mt-1 flex flex-wrap gap-x-3 gap-y-1">
                {tools.map((tool) => (
                  <label key={tool.id} className="flex cursor-pointer items-center gap-1 text-xs">
                    <input
                      type="checkbox"
                      checked={bindingTools.includes(tool.id)}
                      onChange={(e) => {
                        // 同步读取后再进 updater，避免 React 置空 currentTarget
                        const checked = e.currentTarget.checked;
                        setBindingTools((prev) =>
                          checked ? [...prev, tool.id] : prev.filter((id) => id !== tool.id)
                        );
                      }}
                    />
                    {tool.label}
                  </label>
                ))}
              </div>
            </div>
            <div>
              <label className="text-xs font-medium">{t("resources.binding.reason")}</label>
              <textarea
                value={bindingReason}
                onChange={(e) => setBindingReason(e.currentTarget.value)}
                placeholder={t("resources.binding.reasonPlaceholder")}
                className="h-16 w-full rounded border px-2 py-1 text-xs"
              />
            </div>
          </div>
          <DialogFooter className="gap-2">
            <Button variant="outline" size="sm" onClick={() => setBindingDlgOpen(false)}>
              {t("common.cancel")}
            </Button>
            <Button
              variant="outline"
              size="sm"
              disabled={bindingSaving || bindingClearing}
              onClick={handleBindingClear}
            >
              {t("resources.binding.clear")}
            </Button>
            <Button
              size="sm"
              disabled={bindingSaving || bindingClearing}
              onClick={handleBindingSave}
            >
              {t("resources.binding.save")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );
}
