import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Card } from "@/components/ui/card";
import { Layers, Plus, Trash2, Play, X } from "lucide-react";
import type { PresetRecord, PresetApplyResult } from "@/types/preset";
import type { ExtensionWithAssignments } from "@/types/extension";
import { CompatibilityDialog } from "./CompatibilityDialog";

const TOOLS = [
  { id: "claude", label: "Claude" },
  { id: "codex", label: "Codex" },
  { id: "opencode", label: "OpenCode" },
];

export function PresetList({ extensions }: { extensions: ExtensionWithAssignments[] }) {
  const [presets, setPresets] = useState<PresetRecord[]>([]);
  const [showCreate, setShowCreate] = useState(false);
  const [name, setName] = useState("");
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [compatibilityDialog, setCompatibilityDialog] = useState<{
    open: boolean;
    presetId: string;
    presetName: string;
    toolId: string;
  } | null>(null);

  const load = async () => {
    try {
      const data = await invoke<PresetRecord[]>("list_presets");
      setPresets(data);
    } catch (e) {
      console.error("Failed to load presets:", e);
    }
  };

  useEffect(() => { load(); }, []);

  const handleCreate = async () => {
    if (!name) {
      toast.error("请填写预设组名称");
      return;
    }
    if (selected.size === 0) {
      toast.error("请至少选择一个资源（skill、MCP 或插件）");
      return;
    }
    const items = Array.from(selected).map((id) => {
      const ext = extensions.find((e) => e.id === id);
      return [id, ext?.kind ?? "skill"] as [string, string];
    });
    try {
      await invoke("create_preset", { name, items });
      toast.success(`预设组 "${name}" 创建成功`);
      setName("");
      setSelected(new Set());
      setShowCreate(false);
      load();
    } catch (e) {
      toast.error(`创建失败: ${e}`);
    }
  };

  const handleApply = async (presetId: string, presetName: string, toolId: string) => {
    setCompatibilityDialog({
      open: true,
      presetId,
      presetName,
      toolId,
    });
  };

  const confirmApply = async () => {
    if (!compatibilityDialog) return;

    try {
      const result = await invoke<PresetApplyResult>("apply_preset", {
        presetId: compatibilityDialog.presetId,
        toolId: compatibilityDialog.toolId,
      });
      if (result.failures.length > 0) {
        toast.warning(`部分成功: ${result.successCount} 项成功, ${result.failures.length} 项失败`);
      } else if (result.conflicts.length > 0) {
        toast.info(`已应用 ${result.successCount} 项, ${result.conflicts.length} 项冲突跳过`);
      } else {
        toast.success(`"${compatibilityDialog.presetName}" 已应用到 ${compatibilityDialog.toolId}`);
      }
      load();
    } catch (e) {
      toast.error(`应用失败: ${e}`);
    }

    setCompatibilityDialog(null);
  };

  const handleDeactivate = async (presetId: string, presetName: string, toolId: string) => {
    try {
      await invoke("deactivate_preset", { presetId, toolId });
      toast.success(`"${presetName}" 已从 ${toolId} 取消`);
    } catch (e) {
      toast.error(`取消失败: ${e}`);
    }
  };

  const handleDelete = async (presetId: string) => {
    try {
      await invoke("delete_preset", { presetId });
      toast.success("已删除");
      load();
    } catch (e) {
      toast.error(`删除失败: ${e}`);
    }
  };

  const toggleSelect = (id: string) => {
    const next = new Set(selected);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    setSelected(next);
  };

  return (
    <div className="space-y-3">
      <div className="flex items-center justify-between">
        <h3 className="flex items-center gap-2 text-sm font-semibold">
          <Layers className="h-4 w-4" />
          预设组 ({presets.length})
        </h3>
        <Button size="sm" variant="outline" onClick={() => setShowCreate(!showCreate)}>
          <Plus className="mr-1.5 h-3.5 w-3.5" />
          创建
        </Button>
      </div>

      {showCreate && (
        <Card className="space-y-2 p-3">
          <Input placeholder="预设组名称（如：前端开发）" value={name} onChange={(e) => setName(e.currentTarget.value)} />
          <div className="max-h-40 space-y-1 overflow-y-auto">
            {extensions.map((ext) => (
              <label key={ext.id} className="flex cursor-pointer items-center gap-2 text-xs">
                <input type="checkbox" checked={selected.has(ext.id)} onChange={() => toggleSelect(ext.id)} />
                <span className="rounded bg-muted px-1.5 py-0.5 text-[10px]">{ext.kind}</span>
                <span>{ext.name}</span>
              </label>
            ))}
          </div>
          <Button size="sm" onClick={handleCreate}>确认创建</Button>
        </Card>
      )}

      {presets.length === 0 ? (
        <p className="text-muted-foreground py-2 text-xs">暂无预设组。创建一个将多个 skill/MCP 打包为组合。</p>
      ) : (
        <div className="space-y-1">
          {presets.map((preset) => (
            <div key={preset.id} className="rounded border p-2">
              <div className="mb-1 flex items-center justify-between">
                <span className="text-sm font-medium">{preset.name}</span>
                <button onClick={() => handleDelete(preset.id)} className="text-muted-foreground hover:text-red-500">
                  <Trash2 className="h-3.5 w-3.5" />
                </button>
              </div>
              <div className="text-muted-foreground mb-1.5 text-[10px]">
                {preset.items.map((i) => `${i.extensionName || i.extensionId} (${i.kind})`).join(" · ")}
              </div>
              <div className="flex flex-wrap gap-1">
                {TOOLS.map((tool) => (
                  <div key={tool.id} className="space-y-1">
                    <div className="flex gap-0.5">
                      <Button size="sm" variant="outline" className="h-6 px-2 text-[10px]"
                        onClick={() => handleApply(preset.id, preset.name, tool.id)}>
                        <Play className="mr-1 h-2.5 w-2.5" />
                        {tool.label}
                      </Button>
                      <Button size="sm" variant="ghost" className="h-6 w-6 p-0 text-[10px]"
                        onClick={() => handleDeactivate(preset.id, preset.name, tool.id)}>
                        <X className="h-2.5 w-2.5" />
                      </Button>
                    </div>
                    {/* 子 Agent 级操作 */}
                    <SubAgentPresetActions presetId={preset.id} presetName={preset.name} toolId={tool.id} />
                  </div>
                ))}
              </div>
            </div>
          ))}
        </div>
      )}

      {compatibilityDialog && (
        <CompatibilityDialog
          open={compatibilityDialog.open}
          presetId={compatibilityDialog.presetId}
          toolId={compatibilityDialog.toolId}
          toolName={TOOLS.find((t) => t.id === compatibilityDialog.toolId)?.label || compatibilityDialog.toolId}
          onClose={() => setCompatibilityDialog(null)}
          onConfirm={confirmApply}
        />
      )}
    </div>
  );
}

function SubAgentPresetActions({ presetId, presetName, toolId }: { presetId: string; presetName: string; toolId: string }) {
  const [subAgents, setSubAgents] = useState<string[]>([]);
  const [expanded, setExpanded] = useState(false);

  const loadSubAgents = async () => {
    try {
      const data = await invoke<string[]>("detect_subagents", { toolId });
      setSubAgents(data);
    } catch (e) {
      console.error("Failed to load subagents:", e);
    }
  };

  const handleApplyToSubagent = async (subAgentId: string) => {
    try {
      const result = await invoke<PresetApplyResult>("apply_preset_to_subagent", { presetId, toolId, subAgentId });
      if (result.failures.length > 0) {
        toast.warning(`部分成功: ${result.successCount} 项成功, ${result.failures.length} 项失败`);
      } else {
        toast.success(`"${presetName}" 已应用到 ${toolId}:${subAgentId}`);
      }
    } catch (e) {
      toast.error(`应用失败: ${e}`);
    }
  };

  const handleDeactivateFromSubagent = async (subAgentId: string) => {
    try {
      await invoke("deactivate_preset_from_subagent", { presetId, toolId, subAgentId });
      toast.success(`"${presetName}" 已从 ${toolId}:${subAgentId} 取消`);
    } catch (e) {
      toast.error(`取消失败: ${e}`);
    }
  };

  if (subAgents.length === 0) return null;

  return (
    <div>
      <button
        onClick={() => { setExpanded(!expanded); if (!expanded) loadSubAgents(); }}
        className="text-muted-foreground text-[10px] hover:text-foreground"
      >
        {expanded ? "收起子 Agent" : "子 Agent ▼"}
      </button>
      {expanded && (
        <div className="ml-2 space-y-0.5 border-l pl-2">
          {subAgents.map((sa) => (
            <div key={sa} className="flex items-center gap-1 text-[10px]">
              <span className="text-muted-foreground">{sa}</span>
              <Button size="sm" variant="ghost" className="h-4 px-1 text-[9px]"
                onClick={() => handleApplyToSubagent(sa)}>
                <Play className="mr-0.5 h-2 w-2" />应用
              </Button>
              <Button size="sm" variant="ghost" className="h-4 w-4 p-0 text-[9px]"
                onClick={() => handleDeactivateFromSubagent(sa)}>
                <X className="h-2 w-2" />
              </Button>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
