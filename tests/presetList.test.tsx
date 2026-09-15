// tests/presetList.test.tsx — 预设组 v2 列表（Task 4 骨架 + Task 6 预设×工具开关）
// 覆盖：双分区渲染（通用 / 工具私有按绑定工具分组）、activePresets 驱动 Switch checked、
// 能力门控（工具不支持的资源类型 → 未激活开关 disabled；已激活保持可操作以走 restore）
import { render, screen, waitFor, within } from "@testing-library/react";
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { PresetList } from "@/components/presets/PresetList";
import type { EnabledTool } from "@/lib/query/queries/tools";

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: invokeMock }));

// tests/setup.ts 未初始化 i18n，显式引入并固定中文（文案按 zh 断言）
import i18n from "@/i18n";

beforeAll(async () => {
  await i18n.changeLanguage("zh");
});

const TOOL_CLAUDE: EnabledTool = {
  id: "claude",
  label: "Claude Code",
  skillToggleSupported: true,
  mcpSupported: true,
  pluginSupported: true,
};

/** 能力全关的工具（门控用例：任何资源类型都不支持启停） */
const TOOL_DSH: EnabledTool = {
  id: "dsh",
  label: "Dsh",
  skillToggleSupported: false,
  mcpSupported: false,
  pluginSupported: false,
};

const item = (extensionId: string, kind: string, extensionName: string) => ({
  extensionId,
  kind,
  extensionName,
});

const UNIVERSAL_PRESET = {
  id: "p-univ",
  name: "通用组合",
  description: "全工具通用",
  scope: "universal",
  boundTool: null,
  items: [item("s1", "skill", "skill-a")],
};

const CODEX_PRESET = {
  id: "p-codex",
  name: "Codex 专属",
  description: "",
  scope: "tool",
  boundTool: "codex",
  items: [item("s2", "skill", "skill-b")],
};

/** 挂载 PresetList（真实 React Query，invoke 全走 invokeMock），等待列表数据落地 */
async function renderPresetList(opts: {
  enabledTools?: EnabledTool[];
  presets?: unknown[];
  activePresets?: unknown[];
}) {
  invokeMock.mockImplementation(async (cmd: string) => {
    if (cmd === "list_enabled_tools") return opts.enabledTools ?? [TOOL_CLAUDE];
    if (cmd === "list_presets") return opts.presets ?? [];
    if (cmd === "list_active_presets") return opts.activePresets ?? [];
    // list_resource_bindings（编辑弹窗 hooks 挂载即拉）与其余命令一律空兜底
    return [];
  });
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  const utils = render(
    <QueryClientProvider client={queryClient}>
      <PresetList extensions={[]} />
    </QueryClientProvider>
  );
  // 等数据落地：预设名渲染 = list_presets 就绪；activePresets 查询已发出
  await utils.findByText(
    opts.presets?.[0]
      ? (opts.presets[0] as { name: string }).name
      : "暂无预设组。创建一个将多个 skill/MCP 打包为组合。"
  );
  await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("list_active_presets"));
  return utils;
}

/** 定位「预设卡片 × 工具行」的 Switch：预设名 → 卡片根 div → 工具名 span → 开关包装 span → switch */
function switchFor(presetName: string, toolLabel: string): HTMLElement {
  // 名字 span 所在 flex 行的父节点即卡片根
  const card = screen.getByText(presetName).closest("div")!.parentElement!;
  // 工具名 span 的父节点是「图标 + 名字 + 开关」包装 span
  const toolSpan = within(card).getByText(toolLabel).parentElement!;
  return within(toolSpan).getByRole("switch");
}

beforeEach(() => {
  invokeMock.mockReset();
});

describe("PresetList（v2 双分区 + 预设×工具开关）", () => {
  it("双分区渲染：通用区 + 私有区按绑定工具分组（tool-scope 预设落在绑定工具组头下）", async () => {
    await renderPresetList({
      enabledTools: [
        TOOL_CLAUDE,
        {
          ...TOOL_DSH,
          id: "codex",
          label: "Codex CLI",
          skillToggleSupported: true,
          mcpSupported: true,
          pluginSupported: true,
        },
      ],
      presets: [UNIVERSAL_PRESET, CODEX_PRESET],
    });
    // 标题计数 + 两分区标题（zh 词条）
    expect(screen.getByText("预设组 (2)")).toBeInTheDocument();
    expect(screen.getByText("通用预设")).toBeInTheDocument();
    expect(screen.getByText("工具私有预设")).toBeInTheDocument();
    // 通用预设出现在通用区
    const universalSection = screen.getByText("通用预设").closest("section")!;
    expect(within(universalSection).getByText("通用组合")).toBeInTheDocument();
    // 私有区：组头 = 绑定工具名，预设卡片落在该组容器内
    const privateSection = screen.getByText("工具私有预设").closest("section")!;
    // "Codex CLI" 出现 3 处（组头 div + 开关行包装 span + 开关行标签 span），取组头 div
    const groupHeader = within(privateSection)
      .getAllByText("Codex CLI")
      .find((el) => el.tagName === "DIV")!;
    const group = groupHeader.parentElement!;
    expect(within(group).getByText("Codex 专属")).toBeInTheDocument();
  });

  it("activePresets 驱动 Switch checked：on 工具 checked、off 工具 unchecked（radix data-state）", async () => {
    await renderPresetList({
      enabledTools: [
        TOOL_CLAUDE,
        {
          ...TOOL_DSH,
          id: "codex",
          label: "Codex CLI",
          skillToggleSupported: true,
          mcpSupported: true,
          pluginSupported: true,
        },
      ],
      presets: [UNIVERSAL_PRESET, CODEX_PRESET],
      activePresets: [
        { toolId: "claude", presetId: "p-univ" },
        { toolId: "codex", presetId: "p-codex" },
      ],
    });
    // 通用预设 × claude = 激活 → checked；× codex = 未激活 → unchecked
    expect(switchFor("通用组合", "Claude Code")).toHaveAttribute("data-state", "checked");
    expect(switchFor("通用组合", "Codex CLI")).toHaveAttribute("data-state", "unchecked");
    // 私有预设 × 绑定工具 = 激活 → checked；非绑定工具不渲染开关（仅 claude/codex 两枚）
    expect(switchFor("Codex 专属", "Codex CLI")).toHaveAttribute("data-state", "checked");
  });

  it("能力门控：dsh 全能力 false → 未激活 mcp 开关 disabled 且 unchecked；已激活开关保持可操作（restore 路径）", async () => {
    const MCP_PRESET = {
      ...UNIVERSAL_PRESET,
      id: "p-mcp",
      name: "MCP 组合",
      items: [item("m1", "mcp", "mcp-x")],
    };
    await renderPresetList({
      enabledTools: [TOOL_CLAUDE, TOOL_DSH],
      presets: [MCP_PRESET],
      // dsh 已激活 p-mcp：门控不挡「关」（a5f4e2d 裁决）
      activePresets: [{ toolId: "dsh", presetId: "p-mcp" }],
    });
    // 未激活组合不存在（p-mcp 仅在 dsh 上激活），故门控开关 = checked + 可操作
    const activeGated = switchFor("MCP 组合", "Dsh");
    expect(activeGated).toHaveAttribute("data-state", "checked");
    expect(activeGated).not.toBeDisabled();
    // 支持该类型的 claude 开关不受门控影响（未激活 → unchecked + 可操作）
    expect(switchFor("MCP 组合", "Claude Code")).toHaveAttribute("data-state", "unchecked");
    expect(switchFor("MCP 组合", "Claude Code")).not.toBeDisabled();
  });

  it("能力门控（未激活）：预设含工具不支持的资源类型 → 该工具 Switch disabled 且未激活，title 带不支持原因", async () => {
    const MCP_PRESET = {
      ...UNIVERSAL_PRESET,
      id: "p-mcp",
      name: "MCP 组合",
      items: [item("m1", "mcp", "mcp-x")],
    };
    await renderPresetList({
      enabledTools: [TOOL_CLAUDE, TOOL_DSH],
      presets: [MCP_PRESET],
      activePresets: [],
    });
    // 未激活 + 门控 → disabled 且 unchecked，包装 span title = 「工具名: 暂不支持」（zh）
    const gated = switchFor("MCP 组合", "Dsh");
    expect(gated).toBeDisabled();
    expect(gated).toHaveAttribute("data-state", "unchecked");
    expect(screen.getByText("Dsh").parentElement).toHaveAttribute("title", "Dsh: 暂不支持");
    // 支持该类型的 claude 开关不受影响
    expect(switchFor("MCP 组合", "Claude Code")).not.toBeDisabled();
  });
});
