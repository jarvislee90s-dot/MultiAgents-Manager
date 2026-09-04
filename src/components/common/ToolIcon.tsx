interface ToolIconProps {
  toolId: string;
  className?: string;
  size?: number;
}

export function ToolIcon({ toolId, className = "", size = 16 }: ToolIconProps) {
  const Svg = TOOL_SVGS[toolId] || TOOL_SVGS.claude;
  return (
    <span
      className={`inline-flex items-center justify-center ${className}`}
      style={{ minWidth: size }}
    >
      <Svg size={size} />
    </span>
  );
}

// Claude — purple "C" mark
function ClaudeIcon({ size }: { size: number }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 20 20"
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
    >
      <rect width="20" height="20" rx="5" fill="#6445A2" />
      <text
        x="10"
        y="14.5"
        textAnchor="middle"
        fill="white"
        fontSize="12"
        fontWeight="700"
        fontFamily="-apple-system, system-ui"
      >
        C
      </text>
    </svg>
  );
}

// Codex CLI — green terminal prompt
function CodexIcon({ size }: { size: number }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 20 20"
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
    >
      <rect width="20" height="20" rx="5" fill="#16A34A" />
      <path
        d="M5.5 12.5L9 9L5.5 5.5"
        stroke="white"
        strokeWidth="1.8"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
      <path d="M11 13H14.5" stroke="white" strokeWidth="1.8" strokeLinecap="round" />
    </svg>
  );
}

// OpenCode — orange angle brackets
function OpenCodeIcon({ size }: { size: number }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 20 20"
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
    >
      <rect width="20" height="20" rx="5" fill="#EA580C" />
      <path
        d="M7 6L4 10L7 14"
        stroke="white"
        strokeWidth="1.8"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
      <path
        d="M13 6L16 10L13 14"
        stroke="white"
        strokeWidth="1.8"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

// OpenClaw — indigo robot/claw
function OpenClawIcon({ size }: { size: number }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 20 20"
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
    >
      <rect width="20" height="20" rx="5" fill="#6366F1" />
      <circle cx="10" cy="11" r="4.5" stroke="white" strokeWidth="1.5" />
      <circle cx="8" cy="10" r="0.8" fill="white" />
      <circle cx="12" cy="10" r="0.8" fill="white" />
      <path d="M6 6.5L8 4" stroke="white" strokeWidth="1.3" strokeLinecap="round" />
      <path d="M14 6.5L12 4" stroke="white" strokeWidth="1.3" strokeLinecap="round" />
    </svg>
  );
}

// Kimi Code — Moonshot 弦月
function KimiIcon({ size }: { size: number }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 20 20"
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
    >
      <rect width="20" height="20" rx="5" fill="#0B0E1A" />
      {/* 弦月：外弧 + 内弧咬出月形（Feather moon 路径 24→20 等比缩放） */}
      <path d="M17.5 10.66A7.5 7.5 0 1 1 9.34 2.5 5.83 5.83 0 0 0 17.5 10.66Z" fill="white" />
    </svg>
  );
}

// WorkBuddy — 官方图标几何重绘（P2-10）：绿色渐变圆角方块 + 猫耳 + 双圆点眼，
// 取自 WorkBuddy.app/Contents/Resources/icon.icns（2026-09-04 实机取样）
function WorkBuddyIcon({ size }: { size: number }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 20 20"
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
    >
      <defs>
        <linearGradient id="wb-g" x1="3" y1="2" x2="17" y2="18" gradientUnits="userSpaceOnUse">
          <stop stopColor="#4AD06A" />
          <stop offset="1" stopColor="#0FBF8F" />
        </linearGradient>
      </defs>
      {/* 圆角方块底 */}
      <rect width="20" height="20" rx="5" fill="url(#wb-g)" />
      {/* 猫头轮廓（含双耳） */}
      <path
        d="M4.6 7.2 4.2 3.4c0-.4.4-.7.8-.5l3.4 1.9a7.6 7.6 0 0 1 2.9-.57c1.1 0 2.1.2 3 .57l3.4-1.9c.4-.2.8.1.8.5l-.4 3.8c.6 1 1 2.2 1 3.4 0 4.14-3.36 6.9-7.5 6.9S4.6 14.74 4.6 10.6c0-1.2.4-2.4 1-3.4Z"
        fill="white"
      />
      {/* 双圆点眼 */}
      <circle cx="8.2" cy="11.4" r="1.05" fill="url(#wb-g)" />
      <circle cx="12.6" cy="11.4" r="1.05" fill="url(#wb-g)" />
    </svg>
  );
}

const TOOL_SVGS: Record<string, React.FC<{ size: number }>> = {
  claude: ClaudeIcon,
  codex: CodexIcon,
  opencode: OpenCodeIcon,
  openclaw: OpenClawIcon,
  kimi: KimiIcon,
  workbuddy: WorkBuddyIcon,
};
