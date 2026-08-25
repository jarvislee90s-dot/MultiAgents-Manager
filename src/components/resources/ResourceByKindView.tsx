import { useState, useEffect } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { toast } from "sonner";
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
import { Package, Link2, Plug, Info, Trash2 } from "lucide-react";
import {
  listSsotResources,
  checkSkillTargetType,
  disableSkillForTool,
  enableSkillForTool,
  importMcpToSsot,
  saveMcpConfig,
} from "@/lib/api/resource";
import { uninstallResource } from "@/lib/api/manifest";
import type { SsotResources } from "@/types/extension";

const TOOLS = [
  { id: "claude", label: "Claude" },
  { id: "codex", label: "Codex" },
  { id: "opencode", label: "OpenCode" },
  { id: "openclaw", label: "OpenClaw" },
];

function formatSkillName(name: string): string {
  return name.includes("/") ? name.replace("/", ": ") : name;
}

type PendingDisable = {
  skillName: string;
  toolId: string;
  toolLabel: string;
  displayName: string;
  targetType: "symlink" | "native";
};

export function ResourceByKindView() {
  const { t } = useTranslation();
  const [resources, setResources] = useState<SsotResources | null>(null);
  const [search, setSearch] = useState("");
  const [pending, setPending] = useState<PendingDisable | null>(null);
  const [dialogOpen, setDialogOpen] = useState(false);
  const [mcpDialogOpen, setMcpDialogOpen] = useState(false);
  const [newMcp, setNewMcp] = useState({ name: "", command: "", args: "", env: "" });
  const [pendingUninstall, setPendingUninstall] = useState<{
    kind: string;
    name: string;
    count: number;
  } | null>(null);

  useEffect(() => {
    listSsotResources().then(setResources).catch(console.error);
  }, []);

  if (!resources) {
    return <div className="text-muted-foreground py-4 text-xs">{t("common.loading")}</div>;
  }

  const filteredSkills = resources.skills.filter((s) => {
    if (!search.trim()) return true;
    const q = search.toLowerCase();
    return [s.name, ...s.enabledTools].some((x) => x.toLowerCase().includes(q));
  });

  const handleToggleMcp = async (name: string, toolId: string, enabled: boolean) => {
    try {
      if (enabled) {
        // 启用前尝试自动导入到 SSOT（如果还未导入）
        try {
          await importMcpToSsot(name);
        } catch (_) {
          // 可能已导入或找不到配置，继续尝试启用
        }
      }
      await invoke("toggle_mcp_for_tool", { mcpName: name, toolId, enabled });
      toast.success(t(enabled ? "resources.enabled" : "resources.disabled", { name }));
      const fresh = await listSsotResources();
      setResources(fresh);
    } catch (e) {
      toast.error(t("common.operationFailed", { error: e }));
    }
  };

  const handleTogglePlugin = async (
    name: string,
    toolId: string,
    enabled: boolean,
    kind: string
  ) => {
    try {
      await invoke("toggle_plugin_for_tool", { pluginName: name, toolId, enabled, kind });
      toast.success(t(enabled ? "resources.enabled" : "resources.disabled", { name }));
      const fresh = await listSsotResources();
      setResources(fresh);
    } catch (e) {
      toast.error(t("common.operationFailed", { error: e }));
    }
  };

  const handleSkillToggle = async (skillName: string, toolId: string, enabled: boolean) => {
    if (!enabled) {
      // 灰 → 亮：直接启用
      try {
        await enableSkillForTool(skillName, toolId);
        toast.success(
          t("resources.enabledInTool", {
            name: formatSkillName(skillName),
            tool: TOOLS.find((tool) => tool.id === toolId)?.label,
          })
        );
        const fresh = await listSsotResources();
        setResources(fresh);
      } catch (e) {
        toast.error(t("resources.enableFailed", { error: e }));
      }
    } else {
      // 亮 → 灰：先检查类型，再弹窗
      try {
        const targetType = await checkSkillTargetType(toolId, skillName);
        const toolLabel = TOOLS.find((tool) => tool.id === toolId)?.label || toolId;
        setPending({
          skillName,
          toolId,
          toolLabel,
          displayName: formatSkillName(skillName),
          targetType: targetType as "symlink" | "native",
        });
        setDialogOpen(true);
      } catch (e) {
        toast.error(t("resources.checkFailed", { error: e }));
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
      const fresh = await listSsotResources();
      setResources(fresh);
    } catch (e) {
      toast.error(t("resources.removeFailed", { error: e }));
    } finally {
      setDialogOpen(false);
      setPending(null);
    }
  };

  const confirmUninstall = async () => {
    if (!pendingUninstall) return;
    try {
      await uninstallResource(pendingUninstall.kind, pendingUninstall.name);
      toast.success(t("resources.uninstallSuccess", { name: pendingUninstall.name }));
      const fresh = await listSsotResources();
      setResources(fresh);
    } catch (e) {
      toast.error(t("common.operationFailed", { error: e }));
    } finally {
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
      const fresh = await listSsotResources();
      setResources(fresh);
    } catch (e) {
      toast.error(t("resources.addMcpFailed", { error: e }));
    }
  };

  return (
    <>
      <div className="bg-card rounded-lg border p-4">
        <h3 className="mb-3 text-sm font-semibold">{t("resources.repoTitle")}</h3>

        {/* Skills */}
        <div className="mb-4">
          <div className="mb-2 flex items-center justify-between gap-2">
            <h4 className="flex items-center gap-2 text-sm font-semibold">
              <Package className="h-4 w-4" />
              {t("resources.skillsCount", { n: filteredSkills.length })}
            </h4>
            <input
              type="text"
              placeholder={t("resources.searchPlaceholder")}
              value={search}
              onChange={(e) => setSearch(e.currentTarget.value)}
              className="h-7 w-40 rounded border px-2 text-xs"
            />
          </div>
          {filteredSkills.length === 0 ? (
            <div className="text-muted-foreground flex items-center gap-2 py-4 text-xs">
              <Info className="h-3.5 w-3.5" />
              {t("resources.noSkillsHint")}
            </div>
          ) : (
            <div className="space-y-1">
              {filteredSkills.map((skill) => (
                <div
                  key={skill.name}
                  className="flex items-center justify-between rounded border p-2 text-sm"
                >
                  <div className="flex items-center gap-1">
                    <span className="font-medium">{formatSkillName(skill.name)}</span>
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
                        })
                      }
                    >
                      <Trash2 className="h-3 w-3" />
                    </Button>
                  </div>
                  <div className="flex gap-1">
                    {TOOLS.map((tool) => {
                      const enabled = skill.enabledTools.includes(tool.id);
                      return (
                        <Button
                          key={tool.id}
                          variant={enabled ? "default" : "ghost"}
                          size="sm"
                          className={`h-6 px-2 text-[10px] ${enabled ? "" : "text-muted-foreground opacity-50"}`}
                          title={`${tool.label}: ${enabled ? t("resources.enabledShort") : t("resources.disabledShort")}`}
                          onClick={() => handleSkillToggle(skill.name, tool.id, enabled)}
                        >
                          <ToolIcon toolId={tool.id} size={14} className="mr-1" />
                          {tool.label}
                        </Button>
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
          <h4 className="mb-2 flex items-center gap-2 text-sm font-semibold">
            <Link2 className="h-4 w-4" />
            {t("resources.mcpsCount", { n: resources.mcp.length })}
            <Button
              size="sm"
              variant="ghost"
              className="ml-auto h-6 px-2 text-[10px]"
              onClick={() => setMcpDialogOpen(true)}
            >
              {t("resources.addWithPlus")}
            </Button>
          </h4>
          {resources.mcp.length === 0 ? (
            <div className="text-muted-foreground flex items-center gap-2 py-4 text-xs">
              <Info className="h-3.5 w-3.5" />
              {t("mcp.empty")}
            </div>
          ) : (
            <div className="space-y-1">
              {resources.mcp.map((mcp) => (
                <div
                  key={mcp.name}
                  className="flex items-center justify-between rounded border p-2 text-sm"
                >
                  <div className="flex items-center gap-1">
                    <span className="font-medium">{mcp.name}</span>
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
                        })
                      }
                    >
                      <Trash2 className="h-3 w-3" />
                    </Button>
                  </div>
                  <div className="flex gap-1">
                    {TOOLS.map((tool) => {
                      const enabled = mcp.enabledTools.includes(tool.id);
                      return (
                        <Button
                          key={tool.id}
                          variant={enabled ? "default" : "ghost"}
                          size="sm"
                          className={`h-6 px-2 text-[10px] ${enabled ? "" : "text-muted-foreground opacity-50"}`}
                          onClick={() => handleToggleMcp(mcp.name, tool.id, !enabled)}
                        >
                          <ToolIcon toolId={tool.id} size={14} className="mr-1" />
                          {tool.label}
                        </Button>
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
          <h4 className="mb-2 flex items-center gap-2 text-sm font-semibold">
            <Plug className="h-4 w-4" />
            {t("resources.pluginsCount", { n: resources.plugins.length })}
          </h4>
          {resources.plugins.length === 0 ? (
            <div className="text-muted-foreground flex items-center gap-2 py-4 text-xs">
              <Info className="h-3.5 w-3.5" />
              {t("resources.noPlugins")}
            </div>
          ) : (
            <div className="space-y-1">
              {resources.plugins.map((plugin) => (
                <div
                  key={plugin.name}
                  className="flex items-center justify-between rounded border p-2 text-sm"
                >
                  <div className="flex items-center gap-1">
                    <span className="font-medium">{plugin.name}</span>
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
                        })
                      }
                    >
                      <Trash2 className="h-3 w-3" />
                    </Button>
                  </div>
                  <div className="flex gap-1">
                    {TOOLS.map((tool) => {
                      const enabled = plugin.enabledTools.includes(tool.id);
                      return (
                        <Button
                          key={tool.id}
                          variant={enabled ? "default" : "ghost"}
                          size="sm"
                          className={`h-6 px-2 text-[10px] ${enabled ? "" : "text-muted-foreground opacity-50"}`}
                          onClick={() => handleTogglePlugin(plugin.name, tool.id, !enabled, "file")}
                        >
                          <ToolIcon toolId={tool.id} size={14} className="mr-1" />
                          {tool.label}
                        </Button>
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

      {/* 卸载确认弹窗 */}
      <Dialog open={!!pendingUninstall} onOpenChange={(o) => !o && setPendingUninstall(null)}>
        <DialogContent className="max-w-sm">
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
            <Button variant="destructive" size="sm" onClick={confirmUninstall}>
              {t("resources.uninstall")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );
}
