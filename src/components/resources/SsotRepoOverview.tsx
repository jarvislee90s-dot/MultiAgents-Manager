import { useEffect, useState } from "react";
import { Package, Link2, Plug } from "lucide-react";
import { listSsotResources } from "@/lib/api/resource";
import type { SsotResources } from "@/types/extension";

export function SsotRepoOverview() {
  const [resources, setResources] = useState<SsotResources | null>(null);

  useEffect(() => {
    listSsotResources().then(setResources).catch(console.error);
  }, []);

  if (!resources) return null;

  const totalSkills = resources.skills.length;
  const totalMcp = resources.mcp.length;
  const totalPlugins = resources.plugins.length;
  if (totalSkills + totalMcp + totalPlugins === 0) return null;

  return (
    <div className="rounded-lg border bg-card p-4">
      <h3 className="mb-3 text-sm font-semibold">MAM 仓库</h3>

      {/* Skills */}
      <div className="mb-3">
        <h4 className="mb-1 flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
          <Package className="h-3.5 w-3.5" />
          Skills ({totalSkills})
        </h4>
        <div className="space-y-0.5">
          {resources.skills.map((s) => (
            <div key={s.name} className="flex items-center justify-between rounded bg-accent/30 px-2 py-0.5 text-xs">
              <span>{s.name}</span>
              <span className="text-muted-foreground">
                {s.enabledTools.length > 0
                  ? `已接入 ${s.enabledTools.join(", ")}`
                  : "未启用"}
              </span>
            </div>
          ))}
        </div>
      </div>

      {/* MCP */}
      {totalMcp > 0 && (
        <div className="mb-3">
          <h4 className="mb-1 flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
            <Link2 className="h-3.5 w-3.5" />
            MCP ({totalMcp})
          </h4>
          <div className="space-y-0.5">
            {resources.mcp.map((m) => (
              <div key={m.name} className="flex items-center justify-between rounded bg-accent/30 px-2 py-0.5 text-xs">
                <span>{m.name}</span>
                <span className="text-muted-foreground">
                  {m.enabledTools.length > 0
                    ? `已接入 ${m.enabledTools.join(", ")}`
                    : "未启用"}
                </span>
              </div>
            ))}
          </div>
        </div>
      )}

      {/* Plugins */}
      {totalPlugins > 0 && (
        <div>
          <h4 className="mb-1 flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
            <Plug className="h-3.5 w-3.5" />
            Plugins ({totalPlugins})
          </h4>
          <div className="space-y-0.5">
            {resources.plugins.map((p) => (
              <div key={p.name} className="flex items-center justify-between rounded bg-accent/30 px-2 py-0.5 text-xs">
                <span>{p.name}</span>
                <span className="text-muted-foreground">
                  {p.enabledTools.length > 0
                    ? `已接入 ${p.enabledTools.join(", ")}`
                    : "未启用"}
                </span>
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
