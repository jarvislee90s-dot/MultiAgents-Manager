// 消息折叠共享纯函数（2026-09-20 归档详情对齐批从 SessionDetail 迁出）：
// 活会话与归档详情共用同一套折叠摘要文案与可折叠判定，防两处漂移。
// 只放组件外纯函数——折叠状态（expandedOverride Map）仍归各页面自持。
import type { SessionMessage } from "./api";

/** 过程消息 kind：恒为可折叠对象（活会话运行中折叠语义的 wire 子集） */
export function isProcessKind(kind: string): boolean {
  return kind === "thinking" || kind === "tool-call" || kind === "tool-result";
}

/** 折叠行的摘要标签 */
export function collapsedLabel(m: SessionMessage): string {
  switch (m.kind) {
    case "thinking":
      return "思考过程";
    case "tool-call":
      return m.toolName ? `调用 ${m.toolName}` : "工具调用";
    case "tool-result":
      return "工具结果";
    case "assistant":
      return "更早的回复";
    case "plan":
      // 防御位：plan 一等卡片不可折叠（isToggleable/isCollapsed 恒展开），正常不渲染此头
      // （2026-09-23 main 侧批次丁/戊计划卡合入时移植，保持与活会话页同源）
      return "计划";
    default:
      return "已折叠消息";
  }
}
