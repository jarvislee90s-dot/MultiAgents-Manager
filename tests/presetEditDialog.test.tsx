// tests/presetEditDialog.test.tsx — 预设编辑弹窗（Task 7）
// 覆盖：空名保存不触发 create_preset；填名+勾项+工具私有类型 → create_preset 以含 meta
// 的 camelCase args 调用；工具私有模式不适配（exclusiveTools 不含绑定工具）资源置灰 +
// 原生技能分组（isNative && sourceTool === boundTool）
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { PresetEditDialog } from "@/components/presets/PresetEditDialog";
import type { ExtensionWithAssignments } from "@/types/extension";
import type { ResourceBinding } from "@/types/extension";

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: invokeMock }));

// tests/setup.ts 未初始化 i18n，显式引入并固定中文（文案按 zh 断言）
import i18n from "@/i18n";

beforeAll(async () => {
  await i18n.changeLanguage("zh");
});

const ext = (overrides: Partial<ExtensionWithAssignments>): ExtensionWithAssignments => ({
  id: "x",
  kind: "skill",
  name: "ext",
  description: null,
  sourcePath: "/tmp/x",
  sourceTool: null,
  suite: null,
  tags: null,
  isNative: false,
  assignments: [],
  ...overrides,
});

const EXTENSIONS: ExtensionWithAssignments[] = [
  ext({ id: "s1", kind: "skill", name: "skill-alpha" }),
  ext({ id: "m1", kind: "mcp", name: "mcp-exclusive" }),
  // 原生（未纳管）技能：sourceTool = codex → 工具私有（绑定 codex）时出现在「原生技能」组
  ext({ id: "n1", kind: "skill", name: "native-codex-skill", isNative: true, sourceTool: "codex" }),
];

/** mcp-exclusive 专属绑定：允许工具仅 claude（不含 codex → 绑定 codex 时不适配） */
const BINDINGS: ResourceBinding[] = [
  {
    extensionId: "m1",
    exclusiveTools: "claude",
    reason: "依赖 claude 专属环境",
    updatedAt: "2026-09-15T00:00:00Z",
  },
];

function renderDialog(onClose = vi.fn()) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  render(
    <QueryClientProvider client={queryClient}>
      <PresetEditDialog open preset={null} presetExtensions={EXTENSIONS} onClose={onClose} />
    </QueryClientProvider>
  );
  return onClose;
}

/** 勾选/取消资源复选项（资源名 → label 内的 radix checkbox） */
function checkItem(name: string, next = true) {
  const label = screen.getByText(name).closest("label")!;
  const checkbox = within(label).getByRole("checkbox");
  if (
    (checkbox as HTMLInputElement).getAttribute("data-state") === (next ? "checked" : "unchecked")
  )
    return; // 已是目标态
  fireEvent.click(checkbox);
}

/** 切到工具私有类型并选好绑定工具 */
function selectToolScope(toolId = "codex") {
  fireEvent.click(screen.getByLabelText("工具私有"));
  fireEvent.change(screen.getByRole("combobox"), { target: { value: toolId } });
}

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockImplementation(async (cmd: string) => {
    if (cmd === "list_enabled_tools")
      return [
        {
          id: "claude",
          label: "Claude Code",
          skillToggleSupported: true,
          mcpSupported: true,
          pluginSupported: true,
        },
        {
          id: "codex",
          label: "Codex CLI",
          skillToggleSupported: true,
          mcpSupported: true,
          pluginSupported: true,
        },
      ];
    if (cmd === "list_resource_bindings") return BINDINGS;
    if (cmd === "create_preset") return "new-preset-id";
    return [];
  });
});

describe("PresetEditDialog（新建/编辑共用）", () => {
  it("空名点保存 → create_preset 不被 invoke（保存按钮禁用兜底 + 校验守卫零调用）", async () => {
    renderDialog();
    // 等弹窗内容就绪（套件列表数据落地）
    expect(await screen.findByText("skill-alpha")).toBeInTheDocument();
    const save = screen.getByRole("button", { name: "保存" });
    // 名称空 → 保存按钮禁用（canSave 守卫）
    expect(save).toBeDisabled();
    fireEvent.click(save); // disabled 点击不触发 onClick
    // 程序化兜底路径也不该 invoke（handleSave 空名 toast 守卫同样不触发 create）
    expect(invokeMock).not.toHaveBeenCalledWith("create_preset", expect.anything());
  });

  it("填名+勾项+工具私有+绑定 codex → create_preset 以含 meta 的 camelCase args 调用", async () => {
    const onClose = renderDialog();
    expect(await screen.findByText("skill-alpha")).toBeInTheDocument();
    fireEvent.change(screen.getByPlaceholderText("预设组名称（如：前端开发）"), {
      target: { value: "前端组合" },
    });
    fireEvent.change(screen.getByPlaceholderText("描述这个预设组的用途（可选）"), {
      target: { value: "备忘" },
    });
    checkItem("skill-alpha");
    selectToolScope("codex");
    fireEvent.click(screen.getByRole("button", { name: "保存" }));
    // 后端契约：参数 camelCase（Tauri 自动映射 snake_case），items 为 [extensionId, kind] 二元组
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("create_preset", {
        name: "前端组合",
        items: [["s1", "skill"]],
        description: "备忘",
        scope: "tool",
        boundTool: "codex",
      })
    );
    // 保存成功 → invalidate 后回调关闭
    await waitFor(() => expect(onClose).toHaveBeenCalled());
  });

  it("工具私有模式不适配置灰：exclusiveTools 不含绑定工具的 MAM 资源 disabled；原生技能组按 sourceTool 归属出现", async () => {
    renderDialog();
    expect(await screen.findByText("skill-alpha")).toBeInTheDocument();
    selectToolScope("codex");
    // mcp-exclusive：允许工具仅 claude ≠ 绑定 codex → checkbox disabled
    const mismatchedLabel = screen.getByText("mcp-exclusive").closest("label")!;
    expect(within(mismatchedLabel).getByRole("checkbox")).toBeDisabled();
    // 不适配原因 title（resources.binding.* 组合文案）
    expect(mismatchedLabel).toHaveAttribute(
      "title",
      "专属原因: 依赖 claude 专属环境 · 允许工具: claude"
    );
    // 适配项不受影响
    const okLabel = screen.getByText("skill-alpha").closest("label")!;
    expect(within(okLabel).getByRole("checkbox")).not.toBeDisabled();
    // 原生技能组：isNative && sourceTool === boundTool 的项出现且可勾选
    expect(screen.getByText("原生技能")).toBeInTheDocument();
    const nativeLabel = screen.getByText("native-codex-skill").closest("label")!;
    expect(within(nativeLabel).getByRole("checkbox")).not.toBeDisabled();
  });
});
