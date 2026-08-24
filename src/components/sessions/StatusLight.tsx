import { useTranslation } from "react-i18next";
import { cn } from "@/lib/utils";
import type { SessionStatus } from "@/types/session";

// label 为 i18n 键，渲染时经 t() 翻译
const STATUS_CONFIG: Record<
  SessionStatus,
  { color: string; glow: string; label: string; animation: string }
> = {
  // 红灯 = 待处理/待批准
  waiting: {
    color: "bg-red-500",
    glow: "shadow-[0_0_8px_2px_rgba(239,68,68,0.6)]",
    label: "status.waiting",
    animation: "animate-pulse",
  },
  // 黄灯 = 正在运行
  processing: {
    color: "bg-yellow-500",
    glow: "shadow-[0_0_8px_2px_rgba(234,179,8,0.5)]",
    label: "status.running",
    animation: "",
  },
  thinking: {
    color: "bg-yellow-500",
    glow: "shadow-[0_0_8px_2px_rgba(234,179,8,0.5)]",
    label: "status.running",
    animation: "",
  },
  compacting: {
    color: "bg-yellow-500",
    glow: "shadow-[0_0_8px_2px_rgba(234,179,8,0.5)]",
    label: "status.running",
    animation: "",
  },
  // 绿灯 = 已完成/无交互
  idle: {
    color: "bg-green-500",
    glow: "",
    label: "status.done",
    animation: "",
  },
  finished: {
    color: "bg-green-500",
    glow: "",
    label: "status.done",
    animation: "",
  },
};

export function StatusLight({
  status,
  size = "md",
}: {
  status: SessionStatus;
  size?: "sm" | "md";
}) {
  const { t } = useTranslation();
  const config = STATUS_CONFIG[status];
  const sizeClass = size === "sm" ? "h-2 w-2" : "h-3 w-3";

  return (
    <div className="flex items-center gap-2">
      <span
        className={cn(
          "inline-block rounded-full",
          sizeClass,
          config.color,
          config.glow,
          config.animation
        )}
      />
      {size === "md" && (
        <span className="text-muted-foreground text-xs font-medium">{t(config.label)}</span>
      )}
    </div>
  );
}

export { STATUS_CONFIG };
