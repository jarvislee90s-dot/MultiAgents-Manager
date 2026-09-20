// 历史会话过滤纯函数（spec §7.2）：工具 × 项目双维。与 board-logic.ts 同风格：
// 不改入参。相对时间与状态中文**复用** board-logic 既有导出
// （formatRelativeTime / STATUS_LABELS），本文件不自造。
export type ArchiveToolFilter = string; // "all" 或工具 id

export function filterArchivedByTool<T extends { agentType: string }>(
  rows: T[],
  filter: ArchiveToolFilter,
): T[] {
  if (filter === "all") return rows;
  return rows.filter((r) => r.agentType === filter);
}

export function filterArchivedByProject<T extends { projectName: string }>(
  rows: T[],
  project: string,
): T[] {
  if (project === "all") return rows;
  return rows.filter((r) => r.projectName === project);
}
