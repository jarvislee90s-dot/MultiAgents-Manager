// 工具徽标映射 — SessionCard 与通知浮窗共用（配色：codex 紫 / claude 橙 / opencode 灰白 / openclaw 灰）
import type { ComponentType } from "react";
import { ClaudeIcon, OpenAIIcon, OpenCodeIcon } from "@/components/icons/BrandIcons";

export interface AgentBadge {
  label: string;
  className: string;
  Icon: ComponentType<{ className?: string }>;
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
};
