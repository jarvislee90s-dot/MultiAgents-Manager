// 项目名显示预算（卡片顶行）：中文 10 字 / 英文 15 字符封顶（15 单位，CJK 1.5/字）
import { describe, expect, it } from "vitest";
import { truncateProjectName } from "@/components/sessions/SessionCard";

describe("truncateProjectName", () => {
  it("英文名 15 字符内完整显示", () => {
    expect(truncateProjectName("core")).toBe("core");
    expect(truncateProjectName("deepseek-harnes")).toBe("deepseek-harnes");
  });

  it("英文名超过 15 字符截到 15 加省略号", () => {
    expect(truncateProjectName("deepseek-harness")).toBe("deepseek-harnes…");
    expect(truncateProjectName("MultiAgents-Manager")).toBe("MultiAgents-Man…");
  });

  it("中文名 10 字内完整显示", () => {
    expect(truncateProjectName("国新发展投资")).toBe("国新发展投资");
    expect(truncateProjectName("贵州铁路投资集团有限")).toBe("贵州铁路投资集团有限");
  });

  it("中文名超过 10 字截到 10 加省略号", () => {
    expect(truncateProjectName("贵州铁路投资集团有限责任公司")).toBe("贵州铁路投资集团有限…");
  });

  it("中英混排按宽度预算折算（10 个中文 = 15 单位）", () => {
    // 6 个中文（9）+ 7 个英文（7）= 16 > 15 → 截到第 6 个英文前（13 字符）
    expect(truncateProjectName("贵州铁路投资abc1234")).toBe("贵州铁路投资abc123…");
    // 6 个中文（9）+ 7 个英文（7）= 16：第 7 个英文处超预算
    expect(truncateProjectName("贵州铁路投资abcdefg")).toBe("贵州铁路投资abcdef…");
  });

  it("全角标点按 CJK 计入预算", () => {
    // 8 个全角字符（12）+ 3 个英文（3）= 15 ≤ 15 → 完整
    expect(truncateProjectName("每日入账（备用）abc")).toBe("每日入账（备用）abc");
    // 第 4 个英文处超预算（16 > 15）
    expect(truncateProjectName("每日入账（备用）abcd")).toBe("每日入账（备用）abc…");
  });

  it("空串与预算内短名原样返回", () => {
    expect(truncateProjectName("")).toBe("");
    expect(truncateProjectName("a")).toBe("a");
  });
});
