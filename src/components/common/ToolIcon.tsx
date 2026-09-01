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

const TOOL_SVGS: Record<string, React.FC<{ size: number }>> = {
  claude: ClaudeIcon,
  codex: CodexIcon,
  opencode: OpenCodeIcon,
  openclaw: OpenClawIcon,
  kimi: KimiIcon,
};
