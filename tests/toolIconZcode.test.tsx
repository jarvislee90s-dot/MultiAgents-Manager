import { render } from "@testing-library/react";
import { describe, it, expect } from "vitest";
import { ToolIcon } from "@/components/common/ToolIcon";

// ZCode 图标专项（新文件，不触碰既有 toolIcon.test.tsx）——与 workbuddy 专项同款模式
describe("ToolIcon zcode", () => {
  it("renders solid black with night-rim var（2026-10-05 台账：纯黑底白 Z，蓝紫渐变退役）", () => {
    const { container } = render(<ToolIcon toolId="zcode" />);
    const html = container.innerHTML;
    // 台账纯黑 #141413（回退值同源）+ 夜间亮描边变量（白天 transparent）
    expect(html).toContain("#141413");
    expect(html).toContain("--tool-zcode-rim");
    expect(html).not.toContain("#3B5BFD");
    expect(html).not.toContain("#8A4FF5");
    // 不得回退为 Claude 的橙色 "C"
    expect(html).not.toContain("#D97757");
  });

  it("zcode icon renders an svg glyph without crashing", () => {
    const { container } = render(<ToolIcon toolId="zcode" size={20} />);
    expect(container.querySelector("svg")).toBeTruthy();
  });
});
