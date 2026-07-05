import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Card } from "@/components/ui/card";
import { Package, Link2, FolderPlus, Info, RefreshCw } from "lucide-react";
import type { ExtensionWithAssignments } from "@/types/extension";
import { PresetList } from "@/components/PresetList";

const TOOLS = [
  { id: "claude", label: "Claude" },
  { id: "codex", label: "Codex" },
  { id: "opencode", label: "OpenCode" },
];


interface ImportStats {
  imported: number;
  skippedDup: number;
  sourceCounts: [string, number][];
}

export function ExtensionList() {
  const [extensions, setExtensions] = useState<ExtensionWithAssignments[]>([]);
  const [showInstall, setShowInstall] = useState(false);
  const [installPath, setInstallPath] = useState("");
  const [installName, setInstallName] = useState("");
  const [rescanning, setRescanning] = useState(false);
  const [repoSkills, setRepoSkills] = useState<string[]>([]);

  const load = async () => {
    try {
      const data = await invoke<ExtensionWithAssignments[]>("list_extensions_with_assignments");
      setExtensions(data);
    } catch (e) {
      console.error("Failed to load extensions:", e);
    }
  };

  useEffect(() => {
    load();
  }, []);

  // 窗口获焦时重新加载（用户从 Claude Code 装了新 skill 后切回看板时立即可见）
  useEffect(() => {
    const onFocus = () => load();
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  }, []);

  const skills = extensions.filter((e) => e.kind === "skill");
  const mcps = extensions.filter((e) => e.kind === "mcp");

  const toggleMcp = async (mcpName: string, toolId: string, enabled: boolean) => {
    try {
      await invoke("toggle_mcp_for_tool", { mcpName, toolId, enabled });
      toast.success(`${mcpName} 已${enabled ? "启用" : "禁用"} for ${toolId}`);
      load();
    } catch (e) {
      toast.error(`操作失败: ${e}`);
    }
  };

  const loadRepoSkills = async () => {
    try {
      const data = await invoke<string[]>("list_repo_skills");
      setRepoSkills(data);
    } catch (e) {
      console.error("Failed to load repo skills:", e);
    }
  };

  const handleRescan = async () => {
    setRescanning(true);
    try {
      const stats = await invoke<ImportStats>("rescan_skills");
      const totalFound = stats.sourceCounts.reduce((a, [, n]) => a + n, 0);
      if (stats.imported > 0) {
        toast.success(`已导入 ${stats.imported} 个新 skill（扫描 ${totalFound} 个，跳过 ${stats.skippedDup} 个重复）`);
      } else if (totalFound === 0) {
        toast.info("未发现新 skill");
      } else {
        toast.info(`扫描完成：${totalFound} 个 skill，无新增`);
      }
      await load();
    } catch (e) {
      toast.error(`扫描失败: ${e}`);
    } finally {
      setRescanning(false);
    }
  };

  const handleInstall = async () => {
    if (!installPath || !installName) {
      toast.error("请填写路径和名称");
      return;
    }
    try {
      await invoke("install_skill", { sourcePath: installPath, name: installName });
      toast.success(`Skill "${installName}" 安装成功`);
      setInstallPath("");
      setInstallName("");
      setShowInstall(false);
      load();
    } catch (e) {
      toast.error(`安装失败: ${e}`);
    }
  };

  return (
    <div className="space-y-4">
      {/* Skills */}
      <div>
        <div className="mb-2 flex items-center justify-between">
          <h3 className="flex items-center gap-2 text-sm font-semibold">
            <Package className="h-4 w-4" />
            Skill ({skills.length})
          </h3>
          <Button
            size="sm"
            variant="ghost"
            onClick={handleRescan}
            disabled={rescanning}
            title="从 ~/.claude/skills/、~/.agents/skills/、~/.config/opencode/skills/ 重新扫描并导入新 skill"
          >
            <RefreshCw className={`mr-1.5 h-3.5 w-3.5 ${rescanning ? "animate-spin" : ""}`} />
            重新扫描
          </Button>
          <Button
            size="sm"
            variant="outline"
            onClick={() => {
              const next = !showInstall;
              setShowInstall(next);
              if (next) loadRepoSkills();
            }}
          >
            <FolderPlus className="mr-1.5 h-3.5 w-3.5" />
            安装
          </Button>
        </div>

        {showInstall && (
          <Card className="mb-2 space-y-2 p-3">
            <div className="flex gap-2">
              <Input
                placeholder="源目录路径（如 /Users/jarvis/skills/my-skill）"
                value={installPath}
                onChange={(e) => setInstallPath(e.currentTarget.value)}
                className="flex-1"
              />
              <Input
                placeholder="名称"
                value={installName}
                onChange={(e) => setInstallName(e.currentTarget.value)}
                className="w-32"
              />
              <Button size="sm" onClick={handleInstall} disabled={!installPath || !installName}>
                确认
              </Button>
            </div>
            {repoSkills.length > 0 && (
              <div className="text-muted-foreground flex flex-wrap items-center gap-1 text-[10px]">
                <span>从仓库选：</span>
                {repoSkills.map((name) => (
                  <button
                    key={name}
                    className="hover:bg-accent rounded border px-1.5 py-0.5"
                    onClick={() => {
                      setInstallPath(`~/.mam/skills/${name}`);
                      setInstallName(name);
                    }}
                  >
                    {name}
                  </button>
                ))}
              </div>
            )}
          </Card>
        )}

        {skills.length === 0 ? (
          <div className="text-muted-foreground flex items-center gap-2 py-4 text-xs">
            <Info className="h-3.5 w-3.5" />
            暂无 skill。点击"安装"添加，或通过预设组管理。
          </div>
        ) : (
          <div className="space-y-1">
            {skills.map((skill) => (
              <div
                key={skill.id}
                className="flex items-center justify-between rounded border p-2 text-sm"
              >
                <span className="font-medium">{skill.name}</span>
                <span className="text-muted-foreground text-xs">
                  {skill.assignments.filter((a) => a.enabled).length} 个工具已启用
                </span>
              </div>
            ))}
          </div>
        )}
        <p className="text-muted-foreground/60 mt-1 text-[10px]">
          Skill 通过预设组分配，不支持单独启用/禁用
        </p>
      </div>

      {/* MCP */}
      <div>
        <h3 className="mb-2 flex items-center gap-2 text-sm font-semibold">
          <Link2 className="h-4 w-4" />
          MCP 服务器 ({mcps.length})
        </h3>
        {mcps.length === 0 ? (
          <div className="text-muted-foreground flex items-center gap-2 py-4 text-xs">
            <Info className="h-3.5 w-3.5" />
            暂无 MCP 服务器
          </div>
        ) : (
          <div className="space-y-1">
            {mcps.map((mcp) => (
              <div
                key={mcp.id}
                className="flex items-center justify-between rounded border p-2 text-sm"
              >
                <div className="min-w-0">
                  <span className="font-medium">{mcp.name}</span>
                  {mcp.description && (
                    <span className="text-muted-foreground ml-2 text-xs">{mcp.description}</span>
                  )}
                </div>
                <div className="flex shrink-0 gap-1">
                  {TOOLS.map((tool) => {
                    const assignment = mcp.assignments.find((a) => a.agentToolId === tool.id);
                    const enabled = assignment?.enabled ?? false;
                    return (
                      <Button
                        key={tool.id}
                        variant={enabled ? "default" : "outline"}
                        size="sm"
                        className="h-6 px-2 text-[10px]"
                        onClick={() => toggleMcp(mcp.name, tool.id, !enabled)}
                      >
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

      {/* 子 Agent */}
      <div>
        <h3 className="mb-2 text-sm font-semibold">子 Agent</h3>
        <div className="space-y-1">
          {TOOLS.map((tool) => (
            <div key={tool.id} className="flex items-center gap-2 border p-2 text-sm">
              <span className="font-medium">{tool.label}</span>
              <SubAgentPanel toolId={tool.id} />
            </div>
          ))}
        </div>
      </div>

      {/* 预设组 */}
      <PresetList extensions={extensions} />
    </div>
  );
}

// ===== 子 Agent 检测与分配 UI（T061）=====


interface SubAgent {
  id: string;
  name: string;
  agentToolId: string;
  format: string;
}

function SubAgentPanel({ toolId }: { toolId: string }) {
  const [subAgents, setSubAgents] = useState<SubAgent[]>([]);
  const [expanded, setExpanded] = useState(false);

  const load = async () => {
    try {
      const data = await invoke<SubAgent[]>("list_sub_agents", { toolId });
      setSubAgents(data);
    } catch (e) {
      console.error("Failed to load sub-agents:", e);
    }
  };

  useEffect(() => {
    if (expanded) load();
  }, [expanded, toolId]);

  if (!expanded) {
    return (
      <button
        onClick={() => setExpanded(true)}
        className="text-muted-foreground text-[10px] hover:text-foreground"
      >
        展开子 Agent
      </button>
    );
  }

  return (
    <div className="ml-4 space-y-1 border-l pl-2">
      {subAgents.length === 0 ? (
        <span className="text-muted-foreground text-[10px]">无子 Agent</span>
      ) : (
        subAgents.map((sa) => (
          <div key={sa.id} className="flex items-center gap-2 text-[10px]">
            <span className="text-muted-foreground">{sa.name}</span>
            <span className="text-muted-foreground/50">({sa.format})</span>
          </div>
        ))
      )}
      <button onClick={() => setExpanded(false)} className="text-muted-foreground text-[10px] hover:text-foreground">
        收起
      </button>
    </div>
  );
}
