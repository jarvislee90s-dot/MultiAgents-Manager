import { render } from "@testing-library/react";
import { describe, it, expect } from "vitest";
import { ToolIcon } from "@/components/common/ToolIcon";

describe("ToolIcon workbuddy", () => {
  it("renders a dedicated workbuddy icon instead of falling back to Claude", () => {
    const { container } = render(<ToolIcon toolId="workbuddy" />);
    const html = container.innerHTML;
    // WorkBuddy 占位图标使用腾讯蓝，不得回退为 Claude 的紫色 "C"（品牌混淆）
    expect(html).toContain("#0052D9");
    expect(html).not.toContain("#6445A2");
  });

  it("workbuddy icon renders an svg glyph without crashing", () => {
    const { container } = render(<ToolIcon toolId="workbuddy" size={20} />);
    expect(container.querySelector("svg")).toBeTruthy();
  });
});
