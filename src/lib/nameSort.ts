// 名称排序共享工具（「按资源」与「按工具」两视图复用，用户要求口径一致）：
// 三态循环——默认扫描序 → 升序 → 降序；排序按 name 的 zh localeCompare。
export type SortDir = "none" | "asc" | "desc";

export function cycleSortDir(dir: SortDir): SortDir {
  const order: SortDir[] = ["none", "asc", "desc"];
  return order[(order.indexOf(dir) + 1) % order.length];
}

export function applyNameSort<T extends { name: string }>(items: T[], dir: SortDir): T[] {
  if (dir === "none") return items;
  return [...items].sort((a, b) => {
    const cmp = a.name.localeCompare(b.name, "zh");
    return dir === "asc" ? cmp : -cmp;
  });
}
