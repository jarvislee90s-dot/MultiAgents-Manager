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

// 品牌底色 CSS 变量（Bug 4，M3 验收）：SVG 底色经 style 内联 var() 引用
// （presentation attribute 不支持 var()），浅色回退值=原色；暗色态由
// src/index.css / src/mobile/mobile.css 的 .dark 变量表覆盖（kimi 反色、
// claude 提亮，其余=原色）。桌面+移动共用本组件，一处改两板生效

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
      <rect width="20" height="20" rx="5" style={{ fill: "var(--tool-claude-bg, #6445A2)" }} />
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

// Codex — 白底黑终端提示符 + 绿下划线点缀（台账 2026-10-05：OpenAI 黑白单色系，
// 唯一点缀绿 #10A37F；提示符为自绘，不仿官方花结规避侵权）。夜间反转深底白符
function CodexIcon({ size }: { size: number }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 20 20"
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
    >
      <rect
        width="20"
        height="20"
        rx="5"
        style={{
          fill: "var(--tool-codex-bg, #ffffff)",
          stroke: "var(--tool-codex-rim, #1a1a1a)",
          strokeWidth: 1.3,
        }}
      />
      <path
        d="M5.5 12.5L9 9L5.5 5.5"
        style={{ stroke: "var(--tool-codex-fg, #111111)" }}
        strokeWidth="1.8"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
      <path d="M11 13H14.5" stroke="#10A37F" strokeWidth="1.8" strokeLinecap="round" />
    </svg>
  );
}

// OpenCode — 白底黑括号反形（台账 2026-10-05：终端灰度美学；夜间反转深底白符
// + 亮描边）。原橙底退役，橙归 Claude 独占
function OpenCodeIcon({ size }: { size: number }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 20 20"
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
    >
      <rect
        width="20"
        height="20"
        rx="5"
        style={{
          fill: "var(--tool-opencode-bg, #ffffff)",
          stroke: "var(--tool-opencode-rim, #2b2b28)",
          strokeWidth: 1.3,
        }}
      />
      <path
        d="M7 6L4 10L7 14"
        style={{ stroke: "var(--tool-opencode-fg, #111111)" }}
        strokeWidth="1.8"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
      <path
        d="M13 6L16 10L13 14"
        style={{ stroke: "var(--tool-opencode-fg, #111111)" }}
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
      <rect width="20" height="20" rx="5" style={{ fill: "var(--tool-openclaw-bg, #6366F1)" }} />
      <circle cx="10" cy="11" r="4.5" stroke="white" strokeWidth="1.5" />
      <circle cx="8" cy="10" r="0.8" fill="white" />
      <circle cx="12" cy="10" r="0.8" fill="white" />
      <path d="M6 6.5L8 4" stroke="white" strokeWidth="1.3" strokeLinecap="round" />
      <path d="M14 6.5L12 4" stroke="white" strokeWidth="1.3" strokeLinecap="round" />
    </svg>
  );
}

// Kimi Code — Moonshot 弦月。双变量（底+月牙）：浅色=深夜蓝底白月牙；暗色反色
// （底 #E8EDF8、月牙 #0B0E1A）——深夜蓝方底在深色卡底上 1.08 对比度即隐形
function KimiIcon({ size }: { size: number }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 20 20"
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
    >
      <rect width="20" height="20" rx="5" style={{ fill: "var(--tool-kimi-bg, #0B0E1A)" }} />
      {/* 弦月：外弧 + 内弧咬出月形（Feather moon 路径 24→20 等比缩放） */}
      <path
        d="M17.5 10.66A7.5 7.5 0 1 1 9.34 2.5 5.83 5.83 0 0 0 17.5 10.66Z"
        style={{ fill: "var(--tool-kimi-fg, #ffffff)" }}
      />
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
          <stop style={{ stopColor: "var(--tool-workbuddy-bg, #4AD06A)" }} />
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

// ZCode — 纯黑底白 Z（台账 2026-10-05：用户指定「黑底白字」，蓝紫渐变退役；
// 夜间深色卡上加亮描边防融化——rim 变量夜间为 #ECE9E2、白天透明）
function ZCodeIcon({ size }: { size: number }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 20 20"
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
    >
      <rect
        width="20"
        height="20"
        rx="5"
        style={{
          fill: "var(--tool-zcode-bg, #141413)",
          stroke: "var(--tool-zcode-rim, transparent)",
          strokeWidth: 1.4,
        }}
      />
      {/* 字母 Z 折线（横-斜-横一笔成型） */}
      <path
        d="M5.5 5.5h9L6.5 14.5h9"
        stroke="white"
        strokeWidth="2"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

// dsh（DeepSeek harness）— 品牌深蓝圆角方块 + 白色鲸鱼（台账 2026-10-05：
// 鲸鱼为 DeepSeek 主导 logo 特征，字形由字母 D 演化——平直竖笔为 D 之竖、
// 弧形身躯为 D 之碗，加尾鳍与眼点；纯自绘规避侵权）
function DshIcon({ size }: { size: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none">
      <rect width="24" height="24" rx="6" style={{ fill: "var(--tool-dsh-bg, #4D6BFE)" }} />
      <path
        d="M6.8 5.5h3.4c5 0 8.3 3 8.3 6.7 0 .6-.1 1.2-.3 1.7-1 3-4.3 5.1-8 5.1H6.8c-.7 0-1.3-.6-1.3-1.3V6.8c0-.7.6-1.3 1.3-1.3z"
        fill="#fff"
      />
      <path
        d="M17.9 8.7c1.6-.6 2.8-1.6 3.6-3.2.1 1.5-.3 2.7-1.1 3.6.8 1 1.2 2.1 1.1 3.6-.8-1.6-2-2.6-3.6-3.2z"
        fill="#fff"
      />
      <circle cx="7.9" cy="11.5" r="1" style={{ fill: "var(--tool-dsh-bg, #4D6BFE)" }} />
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
  zcode: ZCodeIcon,
  dsh: DshIcon,
};
