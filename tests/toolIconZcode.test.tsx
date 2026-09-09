import { render } from "@testing-library/react";
import { describe, it, expect } from "vitest";
import { ToolIcon } from "@/components/common/ToolIcon";

// ZCode 图标专项（新文件，不触碰既有 toolIcon.test.tsx）——与 workbuddy 专项同款模式
describe("ToolIcon zcode", () => {
  it("renders the brand blue-violet gradient instead of falling back to Claude", () => {
    const { container } = render(<ToolIcon toolId="zcode" />);
    const html = container.innerHTML;
    // 官方图标几何重绘（蓝紫渐变 #3B5BFD→#8A4FF5），不得回退为 Claude 的紫色 "C"
    expect(html).toContain("#3B5BFD");
    expect(html).toContain("#8A4FF5");
    expect(html).not.toContain("#6445A2");
  });

  it("zcode icon renders an svg glyph without crashing", () => {
    const { container } = render(<ToolIcon toolId="zcode" size={20} />);
    expect(container.querySelector("svg")).toBeTruthy();
  });
});
