import { useState, useCallback, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Dialog, DialogContent, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Plus, Trash2, Edit2, Server } from "lucide-react";
import type { McpServerConfig } from "@/types/extension";

export function McpManager({ toolId }: { toolId: string }) {
  const [servers, setServers] = useState<Record<string, McpServerConfig>>({});
  const [showAdd, setShowAdd] = useState(false);
  const [editingName, setEditingName] = useState<string | null>(null);
  const [form, setForm] = useState<McpServerConfig>({ command: "", args: [], env: {} });

  const load = useCallback(async () => {
    try {
      const data = await invoke<{ servers: Record<string, McpServerConfig> }>("read_mcp_servers", { toolId });
      setServers(data.servers || {});
    } catch (e) {
      console.error("Failed to load MCP servers:", e);
      setServers({});
    }
  }, [toolId]);

  useEffect(() => {
    load();
  }, [load]);

  const handleAdd = async () => {
    if (!form.command || !editingName) {
      toast.error("请填写名称和命令");
      return;
    }
    try {
      await invoke("write_mcp_server", {
        toolId,
        mcpName: editingName,
        command: form.command,
        args: form.args,
        env: form.env,
      });
      toast.success(`MCP "${editingName}" 已保存`);
      setShowAdd(false);
      setEditingName(null);
      setForm({ command: "", args: [], env: {} });
      load();
    } catch (e) {
      toast.error(`保存失败: ${e}`);
    }
  };

  const handleDelete = async (name: string) => {
    try {
      await invoke("remove_mcp_server", { toolId, mcpName: name });
      toast.success(`MCP "${name}" 已删除`);
      load();
    } catch (e) {
      toast.error(`删除失败: ${e}`);
    }
  };

  const openEdit = (name: string, config: McpServerConfig) => {
    setEditingName(name);
    setForm(config);
    setShowAdd(true);
  };

  const openAdd = () => {
    setEditingName("");
    setForm({ command: "", args: [], env: {} });
    setShowAdd(true);
  };

  return (
    <div className="space-y-2">
      <div className="flex items-center justify-between">
        <h4 className="flex items-center gap-1.5 text-xs font-semibold">
          <Server className="h-3.5 w-3.5" />
          MCP 服务器
        </h4>
        <Button size="sm" variant="ghost" className="h-6 px-2 text-[10px]" onClick={openAdd}>
          <Plus className="mr-1 h-3 w-3" />
          添加
        </Button>
      </div>

      {Object.entries(servers).length === 0 ? (
        <p className="text-muted-foreground text-[10px]">暂无 MCP 服务器</p>
      ) : (
        <div className="space-y-1">
          {Object.entries(servers).map(([name, config]) => (
            <div key={name} className="flex items-center justify-between rounded border px-2 py-1 text-xs">
              <div className="min-w-0">
                <span className="font-medium">{name}</span>
                <span className="text-muted-foreground ml-1 text-[10px]">{config.command}</span>
              </div>
              <div className="flex shrink-0 gap-1">
                <Button size="sm" variant="ghost" className="h-5 w-5 p-0" onClick={() => openEdit(name, config)}>
                  <Edit2 className="h-3 w-3" />
                </Button>
                <Button size="sm" variant="ghost" className="h-5 w-5 p-0 text-red-500" onClick={() => handleDelete(name)}>
                  <Trash2 className="h-3 w-3" />
                </Button>
              </div>
            </div>
          ))}
        </div>
      )}

      <Dialog open={showAdd} onOpenChange={setShowAdd}>
        <DialogContent className="max-w-md">
          <DialogHeader>
            <DialogTitle className="text-sm">{editingName ? `编辑 ${editingName}` : "添加 MCP 服务器"}</DialogTitle>
          </DialogHeader>
          <div className="space-y-2">
            {editingName === "" && (
              <Input
                placeholder="名称（如 filesystem）"
                value={editingName}
                onChange={(e) => setEditingName(e.currentTarget.value)}
                className="text-xs"
              />
            )}
            <Input
              placeholder="命令（如 npx）"
              value={form.command}
              onChange={(e) => setForm((f) => ({ ...f, command: e.currentTarget.value }))}
              className="text-xs"
            />
            <Input
              placeholder="参数（逗号分隔，如 -y,@modelcontextprotocol/server-filesystem）"
              value={form.args.join(",")}
              onChange={(e) => setForm((f) => ({ ...f, args: e.currentTarget.value.split(",").filter(Boolean) }))}
              className="text-xs"
            />
            <div className="flex justify-end gap-2">
              <Button size="sm" variant="outline" onClick={() => setShowAdd(false)}>取消</Button>
              <Button size="sm" onClick={handleAdd}>保存</Button>
            </div>
          </div>
        </DialogContent>
      </Dialog>
    </div>
  );
}
