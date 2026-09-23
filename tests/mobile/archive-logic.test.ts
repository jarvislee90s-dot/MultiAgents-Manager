import { describe, expect, it } from "vitest";
import { filterArchivedByProject, filterArchivedByTool } from "@/mobile/archive-logic";

const rows = [
  { agentType: "codex", projectName: "proj-a" },
  { agentType: "kimi", projectName: "proj-b" },
  { agentType: "codex", projectName: "proj-b" },
];

describe("archive-logic：工具×项目双维过滤", () => {
  it("all 直通；单工具过滤", () => {
    expect(filterArchivedByTool(rows, "all")).toHaveLength(3);
    expect(filterArchivedByTool(rows, "codex")).toHaveLength(2);
  });
  it("项目过滤 all 直通；单项目过滤；两维可组合（调用侧串联）", () => {
    expect(filterArchivedByProject(rows, "all")).toHaveLength(3);
    expect(filterArchivedByProject(rows, "proj-b")).toHaveLength(2);
    expect(filterArchivedByProject(filterArchivedByTool(rows, "codex"), "proj-b")).toHaveLength(1);
  });
});
