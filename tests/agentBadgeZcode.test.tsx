import { render } from "@testing-library/react";
import { describe, it, expect } from "vitest";
import { AGENT_BADGE, getAgentLabel } from "@/lib/agentBadge";

// ZCode 徽标专项（新文件，不触碰既有 agentBadge.test.tsx——该文件为基线零 diff，
// 其显式 label 断言清单在基线即不含 workbuddy，zcode 的存在性覆盖由其
// SUPPORTED_TOOLS 遍历承担；本文件补显式 label/渲染断言）
describe("agentBadge zcode", () => {
  it("maps zcode to its display name", () => {
    expect(getAgentLabel("zcode")).toBe("ZCode");
    // APP 形态不改变 ZCode 显示名（无 CLI/APP 双名之分）
    expect(getAgentLabel("zcode", "app")).toBe("ZCode");
  });

  it("has an indigo badge entry and renders its icon without crashing", () => {
    const badge = AGENT_BADGE.zcode;
    expect(badge).toBeDefined();
    expect(badge.label).toBe("ZCode");
    expect(badge.className).toContain("border-");
    const { container } = render(<badge.Icon className="h-4 w-4" />);
    expect(container.querySelector("svg")).toBeTruthy();
  });
});
