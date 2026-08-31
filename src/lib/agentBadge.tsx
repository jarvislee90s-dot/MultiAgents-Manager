// 工具徽标映射 — SessionCard 与通知浮窗共用（配色：codex 紫 / claude 橙 / opencode 灰白 / openclaw 灰 / kimi 天蓝）
import type { ComponentType } from "react";
import { ClaudeIcon, KimiIcon, OpenAIIcon, OpenCodeIcon } from "@/components/icons/BrandIcons";

export interface AgentBadge {
  label: string;
  className: string;
  Icon: ComponentType<{ className?: string }>;
}

/** 会话显示名的唯一命名源：主界面徽标、通知浮窗、历史面板统一引用。
 *  codex 区分桌面版（Codex APP）与命令行（Codex CLI），其余工具单一名称。 */
export function getAgentLabel(agentType: string, form?: string): string {
  if (agentType === "codex") return form === "app" ? "Codex APP" : "Codex CLI";
  if (agentType === "claude") return "Claude";
  if (agentType === "opencode") return "OpenCode";
  if (agentType === "openclaw") return "OpenClaw";
  if (agentType === "kimi") return "Kimi Code";
  return agentType;
}

export const AGENT_BADGE: Record<string, AgentBadge> = {
  claude: {
    label: "Claude",
    className: "border-orange-500/30 bg-orange-500/15 text-orange-400",
    Icon: ClaudeIcon,
  },
  codex: {
    label: "Codex",
    className: "border-purple-500/30 bg-purple-500/15 text-purple-400",
    Icon: OpenAIIcon,
  },
  opencode: {
    label: "OpenCode",
    className: "border-zinc-500/40 bg-zinc-800/80 text-zinc-100",
    Icon: OpenCodeIcon,
  },
  openclaw: {
    label: "OpenClaw",
    className: "border-gray-500/30 bg-gray-500/15 text-gray-300",
    Icon: OpenCodeIcon, // 无品牌素材，暂用占位图标，后续替换
  },
  kimi: {
    label: "Kimi Code",
    className: "border-sky-500/30 bg-sky-500/15 text-sky-400",
    Icon: KimiIcon,
  },
};
