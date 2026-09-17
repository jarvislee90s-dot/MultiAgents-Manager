import { useState, type CSSProperties, type KeyboardEvent, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { useQueryClient, useQueries } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { homeDir } from "@tauri-apps/api/path";
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
  type LucideIcon,
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
import { applyNameSort, cycleSortDir, type SortDir } from "@/lib/nameSort";
import { uninstallResource } from "@/lib/api/manifest";
import {
  deleteResourceBinding,
  listToolResidents,
  setResourceBinding,
  setToolResident,
} from "@/lib/api/preset";
import { ManifestInstallDialog } from "./ManifestInstallDialog";
import {
  EMPTY_NEW_MCP,
  McpAddDialog,
  SkillDisableConfirmDialog,
  UninstallConfirmDialog,
  type PendingDisable,
  type PendingUninstall,
} from "./ResourceDialogs";
import type { ResourceBinding, SsotResource } from "@/types/extension";

type ResourceKind = "skill" | "mcp" | "plugin";

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
const TOOL_COL_CLS = "w-[72px] shrink-0"; // 每工具一列：格内仅「启停图标钮 + 常驻锁」两个控件，工具名只活在表头

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
      className="bg-muted sticky top-0 z-20 mb-1 w-max min-w-full items-center rounded border px-2 py-1"
      style={SECTION_GRID_STYLE}
    >
      {/* 左上角格：行、列双向冻结（top-0 锁纵向页滚动，left-2 锁横向工具滚动） */}
      <div className="bg-muted sticky top-0 left-2 z-30 flex items-center gap-1 border-r pr-2">
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
                className={`text-muted-foreground/60 flex items-center justify-center text-[9px] leading-3 ${TOOL_COL_CLS}`}
                title={t("resources.kindNotSupported")}
              >
                {t("resources.kindNotSupported")}
              </span>
            );
          }
          return (
            <div key={tool.id} className={`flex flex-col items-center gap-0.5 ${TOOL_COL_CLS}`}>
              <Button
                variant="ghost"
                size="sm"
                className="text-muted-foreground h-5 w-full justify-center px-0"
                title={tooltip(tool.label)}
                aria-label={tooltip(tool.label)}
                onClick={() => onOpen(tool.id)}
              >
                <Ico className="h-3 w-3" />
              </Button>
              {/* 工具名只活在表头：行内格仅剩图标，纵向滚动时靠本锁定表头识列 */}
              <span className="text-muted-foreground w-full truncate text-center text-[9px] leading-3">
                {tool.label}
              </span>
            </div>
          );
        })}
      </div>
    </div>
  );
}

/** 常驻小字按钮（用户反馈 wave33 Item A，原 Switch 改造）：点亮态（常驻 on）即
 *  「常驻」徽标本体——default 变体小尺寸（h-5 px-1 text-[9px]）；未点亮 ghost + opacity-60。
 *  写入逻辑不变（spec §7.4）：setToolResident → 失效 tool-residents 回读，checked 始终以
 *  query 数据为准（由父级 ToolCell 下传），失败 toast 后状态自然回弹；写入中禁用防连点 */
function ResidentLockButton(props: { toolId: string; extensionId: string; resident: boolean }) {
  const { t } = useTranslation();
  const qc = useQueryClient();
  // 写入中禁用按钮防连点
  const [saving, setSaving] = useState(false);
  const onToggle = async () => {
    setSaving(true);
    try {
      await setToolResident(props.toolId, props.extensionId, !props.resident);
      await qc.invalidateQueries({ queryKey: [...TOOL_RESIDENTS_KEY, props.toolId] });
    } catch (e) {
      toast.error(formatInvokeError(e, t));
    } finally {
      setSaving(false);
    }
  };
  return (
    <Button
      variant={props.resident ? "default" : "ghost"}
      size="sm"
      className={`h-5 shrink-0 px-1 text-[9px] leading-none ${props.resident ? "" : "opacity-60"}`}
      disabled={saving}
      title={t("resources.resident.toggle")}
      aria-label={t("resources.resident.toggle")}
      onClick={() => void onToggle()}
    >
      {t("presets.residentBadge")}
    </Button>
  );
}

/** 工具格单元（用户反馈 wave33 Item C）：启停图标钮 + 常驻小字钮合一，三区
 *  （skill / mcp / plugin）的「span + 启停 Button + 常驻钮」块收敛到此。
 *  residents 查询上收至本组件（每格一次，React Query 按 key 去重，与行级
 *  useQueries 同缓存）；常驻停用防护前端门（后端守卫的配套）：仅停用方向
 *  （enabled && 常驻 on）禁用按钮并换 title 提示，启用方向永不被门 */
function ToolCell(props: {
  tool: EnabledTool;
  extensionId: string;
  enabled: boolean;
  toggleTitle: string;
  onToggle: () => void;
}) {
  const { t } = useTranslation();
  const { data: residents = [] } = useToolResidentsQuery(props.tool.id);
  const resident = residents.includes(props.extensionId);
  const disableProtected = props.enabled && resident;
  return (
    <span className={`flex items-center justify-center gap-1 ${TOOL_COL_CLS}`}>
      <Button
        variant={props.enabled ? "default" : "ghost"}
        size="sm"
        className={`h-6 w-7 justify-center px-0 ${props.enabled ? "" : "text-muted-foreground opacity-50"}`}
        title={
          disableProtected
            ? `${props.tool.label}: ${t("resources.residentProtected")}`
            : props.toggleTitle
        }
        disabled={disableProtected}
        onClick={props.onToggle}
      >
        <ToolIcon toolId={props.tool.id} size={14} />
      </Button>
      {/* 常驻小字钮：紧邻启停按钮（spec §7.4） */}
      <ResidentLockButton
        toolId={props.tool.id}
        extensionId={props.extensionId}
        resident={resident}
      />
    </span>
  );
}

/** 三区段（skill / mcp / plugin）数据驱动渲染的 per-kind 配置：
 *  三段行渲染 ~90% 平行，仅有的机械差异（图标 / 计数与空态文案 / 名字展示 /
 *  名字旁附加徽标 / 行内单格启停语义 / 段头附加控件）全部显式化在这份配置里 */
type KindSectionConfig = {
  /** 段头图标 */
  icon: LucideIcon;
  /** 过滤 + 排序后的行列表 */
  items: SsotResource[];
  /** 段头计数文案（如「Skills (3)」） */
  countLabel: string;
  /** 空态提示文案 */
  emptyHint: string;
  /** 左列名字展示（skill 的目录名斜杠 → 「: 」展示） */
  displayName: (res: SsotResource) => string;
  /** 专属徽标与卸载钮之间的附加徽标（skill: 断链；mcp: 源已停用；plugin: 无） */
  extraBadge?: (res: SsotResource) => ReactNode;
  /** 行内单格启停（skill 亮→灰需弹窗确认故传 enabled 原值；mcp/plugin 直发取反） */
  onToggleTool: (res: SsotResource, toolId: string, enabled: boolean) => void;
  /** 段头右侧附加控件（仅 MCP 有「添加」按钮） */
  headerExtra?: ReactNode;
  /** 段落外层间距：plugin 段在最后无下边距（undefined → 不渲染 class 属性） */
  wrapperClass: string | undefined;
};

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
  const [newMcp, setNewMcp] = useState(EMPTY_NEW_MCP);
  const [pendingUninstall, setPendingUninstall] = useState<PendingUninstall | null>(null);
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
  // 常驻索引（用户反馈 wave33 Item C）：批量「全部停用」需跳过常驻 on 的工具；
  // useQueries 与 ToolCell 的单键查询同 key（["tool-residents", toolId]），
  // React Query 缓存去重共享，不会产生重复网络请求
  const residentsQueries = useQueries({
    queries: tools.map((tool) => ({
      queryKey: [...TOOL_RESIDENTS_KEY, tool.id],
      queryFn: () => listToolResidents(tool.id),
      staleTime: 10000,
    })),
  });
  const toolResidents = new Map<string, string[]>(
    tools.map((tool, i) => [tool.id, residentsQueries[i].data ?? []])
  );
  // 资源能力门：工具 × 资源类型是否支持启停（后端 EnabledTool 标志下发）
  const kindSupported = (tool: EnabledTool, kind: string): boolean =>
    kind === "skill"
      ? tool.skillToggleSupported
      : kind === "mcp"
        ? tool.mcpSupported
        : tool.pluginSupported;
  // 单格启停前的能力门（三段 handler 共用）：工具在列表中且不支持该类资源 →
  // toast 提示并返回 true（调用方早退）；工具不在列表不拦，交由后端处置
  const capabilityBlocked = (toolId: string, kind: ResourceKind): boolean => {
    const tool = tools.find((x) => x.id === toolId);
    if (tool && !kindSupported(tool, kind)) {
      toast.info(t("resources.kindNotSupported"));
      return true;
    }
    return false;
  };
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
      className={`text-muted-foreground h-6 justify-center px-0 opacity-40 ${TOOL_COL_CLS}`}
      title={title}
    >
      <ToolIcon toolId={tool.id} size={14} />
    </Button>
  );
  // 名字排序：三态循环（默认扫描序 → 升序 → 降序），三种资源各自独立记忆
  const [sortDirs, setSortDirs] = useState<Record<ResourceKind, SortDir>>({
    skill: "none",
    mcp: "none",
    plugin: "none",
  });

  const toggleSort = (kind: ResourceKind) => {
    setSortDirs((prev) => ({ ...prev, [kind]: cycleSortDir(prev[kind]) }));
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

  const applySort = <T extends { name: string }>(items: T[], kind: ResourceKind): T[] =>
    applyNameSort(items, sortDirs[kind]);

  /** 打开工具对应资源位置（skill/plugin = 目录，mcp = 配置文件） */
  const handleOpenResource = async (kind: ResourceKind, toolId: string) => {
    try {
      const path = await invoke<string>("open_tool_resource", { toolId, kind });
      toast.success(path);
    } catch (e) {
      toast.error(t("common.operationFailed", { error: formatInvokeError(e, t) }));
    }
  };

  /** 快捷跳转（用户反馈 wave33 Item B）：用系统文件管理器打开 MAM 仓库 /
   *  ~/.agents skill 目录。reveal_dir 后端白名单做词法前缀 + canonicalize 校验
   *  且不展开 ~，必须传绝对路径——homeDir() 取平台 home 现拼；目录不存在时
   *  后端 Err（message 含路径）→ formatInvokeError toast */
  const revealQuickJump = async (rel: ".mam/skills" | ".agents/skills") => {
    try {
      const home = await homeDir();
      // homeDir() 平台带尾斜杠不一（macOS 带 / Windows 不带），兼容两种拼接
      const path = home.endsWith("/") ? `${home}${rel}` : `${home}/${rel}`;
      await invoke("reveal_dir", { path });
    } catch (e) {
      toast.error(formatInvokeError(e, t));
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
    if (capabilityBlocked(toolId, "mcp")) return;
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
      // 常驻门（用户反馈 wave33 Item C）：批量「停用」跳过常驻 on 的工具（启用方向
      // 不限）——后端守卫已拒，此处前端配套先拦，被跳过计数沿用 T10 排他跳过先例
      // （并入 skipped 汇入 batchDone toast）
      if (
        !enable &&
        (toolResidents.get(tool.id) ?? []).includes(bindingKey(res.kind as ResourceKind, res.name))
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
    if (capabilityBlocked(toolId, "plugin")) return;
    try {
      await invoke("toggle_plugin_for_tool", { pluginName: name, toolId, enabled, kind });
      toast.success(t(enabled ? "resources.enabled" : "resources.disabled", { name }));
      await refresh();
    } catch (e) {
      toast.error(t("common.operationFailed", { error: formatInvokeError(e, t) }));
    }
  };

  const handleSkillToggle = async (skillName: string, toolId: string, enabled: boolean) => {
    if (capabilityBlocked(toolId, "skill")) return;
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
      setNewMcp(EMPTY_NEW_MCP);
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

  // per-kind 段落配置：三段的全部行为差异集中于此（共用渲染逻辑见 renderSection）
  const sectionConfig: Record<ResourceKind, KindSectionConfig> = {
    skill: {
      icon: Package,
      items: filteredSkills,
      countLabel: t("resources.skillsCount", { n: filteredSkills.length }),
      emptyHint: t("resources.noSkillsHint"),
      displayName: (res) => formatSkillName(res.name),
      extraBadge: (res) =>
        // 断链徽标：SSOT 存在但部分工具侧符号链接失效（琥珀色警示）
        res.brokenTools && res.brokenTools.length > 0 ? (
          <span
            className="rounded bg-amber-500/15 px-1.5 py-0.5 text-[10px] text-amber-500"
            title={t("resources.linkBrokenTooltip", {
              tools: res.brokenTools.join(", "),
            })}
          >
            {t("resources.linkBroken")}
          </span>
        ) : null,
      // skill 亮→灰需先 checkSkillTargetType 区分 symlink/native 弹窗，传 enabled 原值由 handler 分支
      onToggleTool: (res, toolId, enabled) => handleSkillToggle(res.name, toolId, enabled),
      wrapperClass: "mb-4",
    },
    mcp: {
      icon: Link2,
      items: filteredMcp,
      countLabel: t("resources.mcpsCount", { n: filteredMcp.length }),
      emptyHint: t("mcp.empty"),
      displayName: (res) => res.name,
      extraBadge: (res) =>
        // 源已停用徽标：MCP 存储 JSON 带 enable:false 原样入库（M7 口径）
        res.sourceDisabled ? (
          <span
            className="text-muted-foreground rounded border border-dashed px-1 text-[10px]"
            title={t("resources.mcpSourceDisabledHint")}
          >
            {t("resources.mcpSourceDisabled")}
          </span>
        ) : null,
      onToggleTool: (res, toolId, enabled) => handleToggleMcp(res.name, toolId, !enabled),
      headerExtra: (
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
      ),
      wrapperClass: "mb-4",
    },
    plugin: {
      icon: Plug,
      items: filteredPlugins,
      countLabel: t("resources.pluginsCount", { n: filteredPlugins.length }),
      emptyHint: t("resources.noPlugins"),
      displayName: (res) => res.name,
      onToggleTool: (res, toolId, enabled) =>
        handleTogglePlugin(res.name, toolId, !enabled, res.pluginType ?? "file"),
      wrapperClass: undefined, // plugin 段在最后，无下边距
    },
  };

  // 单段渲染：段头（折叠 + 计数 + 附加控件）→ 表头 → 行网格；
  // 行内「全部启停」与每工具格（能力门 → 专属门 → ToolCell）三段共用同一逻辑
  const renderSection = (kind: ResourceKind) => {
    const cfg = sectionConfig[kind];
    const Icon = cfg.icon;
    return (
      <div className={cfg.wrapperClass}>
        <h4
          className="mb-2 flex cursor-pointer items-center gap-2 text-sm font-semibold select-none"
          aria-expanded={!collapsed[kind]}
          title={collapsed[kind] ? t("resources.expandSection") : t("resources.collapseSection")}
          onClick={() => toggleCollapsed(kind)}
          onKeyDown={onSectionKeyDown(kind)}
          tabIndex={0}
          role="button"
        >
          {collapsed[kind] ? (
            <ChevronRight className="h-4 w-4" />
          ) : (
            <ChevronDown className="h-4 w-4" />
          )}
          <Icon className="h-4 w-4" />
          {cfg.countLabel}
          {cfg.headerExtra}
        </h4>
        {collapsed[kind] ? null : cfg.items.length === 0 ? (
          <div className="text-muted-foreground flex items-center gap-2 py-4 text-xs">
            <Info className="h-3.5 w-3.5" />
            {cfg.emptyHint}
          </div>
        ) : (
          <div className="max-h-[60vh] space-y-1 overflow-auto pb-1">
            <SectionTableHeader
              kind={kind}
              tools={tools}
              sortDir={sortDirs[kind]}
              onToggleSort={() => toggleSort(kind)}
              onOpen={(toolId) => handleOpenResource(kind, toolId)}
            />
            {cfg.items.map((res) => {
              const extensionId = bindingKey(kind, res.name);
              return (
                <div
                  key={res.name}
                  className="w-max min-w-full items-center rounded border p-2 text-sm"
                  style={SECTION_GRID_STYLE}
                >
                  <div className="bg-background sticky left-2 z-10 flex flex-wrap items-center gap-x-1 gap-y-0.5 overflow-hidden border-r">
                    <span className="font-medium">{cfg.displayName(res)}</span>
                    {renderExclusiveBadge(kind, res.name)}
                    {cfg.extraBadge?.(res)}
                    <Button
                      variant="ghost"
                      size="sm"
                      className="text-destructive h-6 px-1.5 text-[10px]"
                      title={t("resources.uninstall")}
                      aria-label={t("resources.uninstall")}
                      onClick={() =>
                        setPendingUninstall({
                          kind,
                          name: res.name,
                          count: res.enabledTools.length,
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
                        res.enabledTools.length === tools.length
                          ? t("resources.allToolsOff")
                          : t("resources.allToolsOn")
                      }
                      onClick={() => handleToggleAll(res, res.enabledTools.length !== tools.length)}
                    >
                      {res.enabledTools.length === tools.length
                        ? t("resources.allToolsOff")
                        : t("resources.allToolsOn")}
                    </Button>
                    {tools.map((tool) => {
                      // 能力门：工具不支持该类资源（如 dsh 无 MCP 配置/插件目录）→ 置灰占位
                      if (!kindSupported(tool, kind)) {
                        return gatedToolButton(
                          tool,
                          `${tool.label}: ${t("resources.kindNotSupported")}`
                        );
                      }
                      // 专属门（spec §6/§7.4）：不适配工具置灰不可启停，title 说明原因
                      if (toolExcludedByBinding(extensionId, tool.id)) {
                        return gatedToolButton(tool, excludedTitle(extensionId, tool.label));
                      }
                      const enabled = res.enabledTools.includes(tool.id);
                      return (
                        <ToolCell
                          key={tool.id}
                          tool={tool}
                          extensionId={extensionId}
                          enabled={enabled}
                          toggleTitle={`${tool.label}: ${enabled ? t("resources.enabledShort") : t("resources.disabledShort")}`}
                          onToggle={() => cfg.onToggleTool(res, tool.id, enabled)}
                        />
                      );
                    })}
                  </div>
                </div>
              );
            })}
          </div>
        )}
      </div>
    );
  };

  return (
    <>
      <div className="bg-card rounded-lg border p-4">
        <h3 className="mb-3 text-sm font-semibold">{t("resources.repoTitle")}</h3>

        <div className="mb-3 flex items-center justify-between gap-2">
          <div className="flex items-center gap-1">
            <input
              type="text"
              placeholder={t("resources.searchPlaceholder")}
              value={search}
              onChange={(e) => setSearch(e.currentTarget.value)}
              className="h-7 w-40 rounded border px-2 text-xs"
            />
            {/* 快捷跳转（用户反馈 wave33 Item B）：MAM 仓库 / ~/.agents 目录直达 */}
            <Button
              size="sm"
              variant="ghost"
              className="h-6 px-1.5 text-[10px]"
              title={t("resources.openMamRepo")}
              aria-label={t("resources.openMamRepo")}
              onClick={() => void revealQuickJump(".mam/skills")}
            >
              <FolderOpen className="mr-1 h-3 w-3" />
              {t("resources.openMamRepo")}
            </Button>
            <Button
              size="sm"
              variant="ghost"
              className="h-6 px-1.5 text-[10px]"
              title={t("resources.openAgentsDir")}
              aria-label={t("resources.openAgentsDir")}
              onClick={() => void revealQuickJump(".agents/skills")}
            >
              <FolderOpen className="mr-1 h-3 w-3" />
              {t("resources.openAgentsDir")}
            </Button>
          </div>
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

        {/* 矩阵图例（用户反馈 wave33 Item A）：一行说明点亮图标 / 常驻徽标 / 表头定位语义 */}
        <p className="text-muted-foreground mb-3 text-[10px] leading-tight">
          {t("resources.matrixHint")}
        </p>

        {/* 三区段（skill / mcp / plugin）：行为差异集中于 sectionConfig，共用 renderSection */}
        {renderSection("skill")}

        {renderSection("mcp")}

        {renderSection("plugin")}
      </div>

      {/* 确认弹窗：取消 = 关弹窗 + 清 pending；Dialog 自身关闭（X/esc）仅关弹窗（与原内联行为一致） */}
      <SkillDisableConfirmDialog
        open={dialogOpen}
        onOpenChange={setDialogOpen}
        pending={pending}
        onCancel={() => {
          setDialogOpen(false);
          setPending(null);
        }}
        onConfirm={confirmDisable}
      />

      {/* 添加 MCP 弹窗：受控表单，取消 = 关弹窗 + 重置草稿 */}
      <McpAddDialog
        open={mcpDialogOpen}
        onOpenChange={setMcpDialogOpen}
        value={newMcp}
        onChange={setNewMcp}
        onCancel={() => {
          setMcpDialogOpen(false);
          setNewMcp(EMPTY_NEW_MCP);
        }}
        onSubmit={handleAddMcp}
      />

      {/* 卸载确认弹窗：取消/关闭不产生任何变更，「继续卸载」带 force 二次确认 */}
      <UninstallConfirmDialog
        pending={pendingUninstall}
        onCancel={() => setPendingUninstall(null)}
        onConfirm={confirmUninstall}
      />

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
