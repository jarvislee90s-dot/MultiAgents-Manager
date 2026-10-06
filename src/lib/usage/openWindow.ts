// 用量大看板窗口的**唯一**打开入口（计划② Task 6 步骤 2）。
// 三个入口（主窗口标题栏图标 / 宠物右键菜单「📊 用量看板」/ Task 13 的浮窗「详情 »」）全部经这里
// 到达**同一窗口同一状态**（spec P1）——故窗口常量只此一处，任何一个入口都不得自带第二套几何。
//
// 幂等由 `createWindow` 自己保证（`src/lib/window.ts:164`）：窗口已存在时只做居中 + 聚焦/显示，
// **不重建、不重置时间范围与页签**（A-1/A-2 的「重复点击不新建第二个窗口」）。
// `parent: "main"` 只用于首次创建时的居中定位；从宠物窗口打开时父窗可能隐藏/最小化，
// `createWindow` 的 `calcCenterPosition` 会回落到屏幕居中（`{"center": true}`）。
import { createWindow } from "@/lib/window";

/** 窗口 label。与 `src-tauri/capabilities/default.json` 的 `windows` 白名单逐字一致（缺则 ACL 拒权限）。 */
export const USAGE_WINDOW_LABEL = "usage-dashboard";

/** 窗口 URL。与 `src/main.tsx` 的 `pageMap` 键逐字一致（漏注册会静默回落首页）。 */
export const USAGE_WINDOW_URL = "/usage";

/**
 * 打开（或聚焦）用量大看板。`theme` 由调用方按当前主题给：`system` 传 `undefined`（Tauri 默认跟随系统）。
 * 窗口尺寸/可缩放/可最大化/不可最小化等常量**逐字**来自计划② Task 6 步骤 2；`minimizable: false`
 * 必须与看板页的 `<TitleBar showMinimize={false}>` 一致（最大化键保留）。
 */
export async function openUsageDashboard(opts: {
  title: string;
  theme?: "dark" | "light";
}): Promise<void> {
  await createWindow(USAGE_WINDOW_LABEL, {
    title: opts.title,
    url: USAGE_WINDOW_URL,
    width: 1040,
    height: 720,
    minWidth: 840,
    minHeight: 560,
    resizable: true,
    maximizable: true,
    minimizable: false,
    decorations: false,
    transparent: true,
    shadow: false,
    parent: "main",
    theme: opts.theme,
  });
}
