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
