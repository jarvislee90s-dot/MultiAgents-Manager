import { render } from "@testing-library/react";
import { describe, it, expect } from "vitest";
import { AGENT_BADGE, getAgentLabel } from "@/lib/agentBadge";
import { ToolIcon } from "@/components/common/ToolIcon";
import { SUPPORTED_TOOLS } from "@/config/constants";

describe("getAgentLabel", () => {
  it("maps every supported tool to a display name", () => {
    expect(getAgentLabel("kimi")).toBe("Kimi Code");
    expect(getAgentLabel("claude")).toBe("Claude");
    expect(getAgentLabel("opencode")).toBe("OpenCode");
    expect(getAgentLabel("openclaw")).toBe("OpenClaw");
  });

  it("keeps codex app/cli distinction", () => {
    expect(getAgentLabel("codex", "app")).toBe("Codex APP");
    expect(getAgentLabel("codex", "cli")).toBe("Codex CLI");
    expect(getAgentLabel("codex")).toBe("Codex CLI");
  });

  it("falls back to the raw agent type for unknown tools", () => {
    expect(getAgentLabel("future-tool")).toBe("future-tool");
  });
});

describe("AGENT_BADGE", () => {
  it("has a badge entry for every supported tool", () => {
    for (const tool of SUPPORTED_TOOLS) {
      const badge = AGENT_BADGE[tool];
      expect(badge, `missing badge for ${tool}`).toBeDefined();
      expect(badge.label).toBeTruthy();
      expect(badge.className).toContain("border-");
    }
  });

  it("kimi badge renders its icon without crashing", () => {
    const { container } = render(<AGENT_BADGE.kimi.Icon className="h-4 w-4" />);
    expect(container.querySelector("svg")).toBeTruthy();
  });
});

describe("ToolIcon", () => {
  it("renders the kimi tool icon", () => {
    const { container } = render(<ToolIcon toolId="kimi" />);
    expect(container.querySelector("svg")).toBeTruthy();
  });

  it("falls back to the claude icon for unknown tools", () => {
    const { container } = render(<ToolIcon toolId="unknown-tool" />);
    expect(container.querySelector("svg")).toBeTruthy();
  });
});
