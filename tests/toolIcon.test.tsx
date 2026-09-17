import { render } from "@testing-library/react";
import { describe, it, expect } from "vitest";
import { ToolIcon } from "@/components/common/ToolIcon";

describe("ToolIcon workbuddy", () => {
  it("renders the official green gradient instead of falling back to Claude", () => {
    const { container } = render(<ToolIcon toolId="workbuddy" />);
    const html = container.innerHTML;
    // P2-10：官方图标几何重绘（绿色渐变 #4AD06A→#0FBF8F），不得回退为 Claude 的紫色 "C"
    expect(html).toContain("#4AD06A");
    expect(html).toContain("#0FBF8F");
    expect(html).not.toContain("#6445A2");
  });

  it("workbuddy icon renders an svg glyph without crashing", () => {
    const { container } = render(<ToolIcon toolId="workbuddy" size={20} />);
    expect(container.querySelector("svg")).toBeTruthy();
  });
});

// Bug 4（M3 验收）：暗色融底修复——SVG 底色/月牙改走 CSS 变量（presentation
// attribute 不支持 var()，故经 style 内联），浅色回退值保留原色；暗色反色由
// index.css / mobile.css 的 .dark 变量表驱动（一处组件，桌面+移动双板生效）
describe("ToolIcon kimi（Bug 4 暗色方底隐形修复）", () => {
  it("底色与月牙走 --tool-kimi-* CSS 变量，浅色回退值保留深夜蓝+白月牙", () => {
    const { container } = render(<ToolIcon toolId="kimi" />);
    const html = container.innerHTML;
    expect(html).toContain("--tool-kimi-bg");
    expect(html).toContain("--tool-kimi-fg");
    expect(html).toContain("#0B0E1A");
  });

  it("claude 底色走 --tool-claude-bg 变量（暗色态由变量表提亮）", () => {
    const { container } = render(<ToolIcon toolId="claude" />);
    expect(container.innerHTML).toContain("--tool-claude-bg");
    expect(container.innerHTML).toContain("#6445A2");
  });
});
