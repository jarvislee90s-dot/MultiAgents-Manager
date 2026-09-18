// tests/applyConfirmDialog.test.tsx — 预设应用确认弹窗（Task 5，含白板裁决）
// 覆盖：五段预览清单（计数表头 + 点名条目）渲染；空段整节隐藏；白板（无可启用项但有
// 停用/暂存）→ whiteboardWarning 强警示 + 确认按钮 destructive 样式
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { ApplyConfirmDialog } from "@/components/presets/ApplyConfirmDialog";
import type { ApplyPreview } from "@/types/preset";

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: invokeMock }));

// tests/setup.ts 未初始化 i18n，显式引入并固定中文（文案按 zh 断言）
import i18n from "@/i18n";

beforeAll(async () => {
  await i18n.changeLanguage("zh");
});

function renderDialog(onConfirm = vi.fn()) {
  const onClose = vi.fn();
  render(
    <ApplyConfirmDialog
      open
      presetId="p1"
      toolId="codex"
      toolName="Codex CLI"
      onClose={onClose}
      onConfirm={onConfirm}
    />
  );
  return { onClose, onConfirm };
}

const FULL_PREVIEW: ApplyPreview = {
  toEnable: ["skill-alpha", "skill-beta"],
  filtered: ["mcp-x: 工具不适用"],
  toDisable: ["mcp-old"],
  toStash: ["native-old"],
  residentExempt: ["resident-a"],
};

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockImplementation(async (cmd: string) => {
    if (cmd === "preview_apply_preset") return FULL_PREVIEW;
    return [];
  });
});

describe("ApplyConfirmDialog（dry-run 预览 + 白板裁决）", () => {
  it("正常 preview 五段渲染：各段计数表头 + 点名条目；确认按钮非 destructive；确认委托 onConfirm", async () => {
    const { onConfirm } = renderDialog();
    // 标题（工具名插值）+ 五段表头（计数来自 preview 数组长度）
    expect(await screen.findByText("应用预设组到 Codex CLI")).toBeInTheDocument();
    expect(screen.getByText("兼容的资源 (2)")).toBeInTheDocument();
    expect(screen.getByText("被过滤 (1)")).toBeInTheDocument();
    expect(screen.getByText("将停用 (1)")).toBeInTheDocument();
    expect(screen.getByText("将暂存 (1)")).toBeInTheDocument();
    expect(screen.getByText("常驻豁免 (1)")).toBeInTheDocument();
    // 点名条目原样渲染
    expect(screen.getByText("skill-alpha")).toBeInTheDocument();
    expect(screen.getByText("skill-beta")).toBeInTheDocument();
    expect(screen.getByText("mcp-x: 工具不适用")).toBeInTheDocument();
    expect(screen.getByText("mcp-old")).toBeInTheDocument();
    expect(screen.getByText("native-old")).toBeInTheDocument();
    // dry-run 命令入参（后端契约：camelCase）
    expect(invokeMock).toHaveBeenCalledWith("preview_apply_preset", {
      presetId: "p1",
      toolId: "codex",
    });
    // 非白板 → 无警示、默认样式确认键（variant 类以 bg-* 区分；基类含 aria-invalid:border-destructive，不能裸查 "destructive"）
    expect(screen.queryByText(/白板模式/)).not.toBeInTheDocument();
    const confirm = screen.getByRole("button", { name: "确认应用 (2 项)" });
    expect(confirm.className).not.toContain("bg-destructive");
    expect(confirm.className).toContain("bg-primary");
    fireEvent.click(confirm);
    await waitFor(() => expect(onConfirm).toHaveBeenCalledTimes(1));
    // 等 confirming 复位（finally 块），避免测试结束后残留状态更新
    await waitFor(() => expect(confirm).not.toBeDisabled());
  });

  it("空段隐藏：filtered 为空数组 → 「被过滤」整节不出现，其余段照常", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "preview_apply_preset") return { ...FULL_PREVIEW, filtered: [] };
      return [];
    });
    renderDialog();
    expect(await screen.findByText("兼容的资源 (2)")).toBeInTheDocument();
    expect(screen.queryByText("被过滤 (1)")).not.toBeInTheDocument();
    expect(screen.queryByText("被过滤 (0)")).not.toBeInTheDocument();
    expect(screen.getByText("将停用 (1)")).toBeInTheDocument();
    expect(screen.getByText("将暂存 (1)")).toBeInTheDocument();
    expect(screen.getByText("常驻豁免 (1)")).toBeInTheDocument();
  });

  it("白板：toEnable 空但有停用/暂存 → whiteboardWarning 警示 + 确认按钮 destructive 样式", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "preview_apply_preset")
        return {
          toEnable: [],
          filtered: [],
          toDisable: ["mcp-a", "mcp-b"],
          toStash: ["native-x"],
          residentExempt: ["resident-a"],
        };
      return [];
    });
    renderDialog();
    // 强警示文案（presets.whiteboardWarning，zh）出现
    expect(
      await screen.findByText(
        "白板模式：本次应用不会启用任何资源，仅停用/暂存现有资源（常驻项豁免）"
      )
    ).toBeInTheDocument();
    // 确认按钮 destructive 变体（variant 类 bg-destructive），计数为 0 仍可继续
    const confirm = screen.getByRole("button", { name: "确认应用 (0 项)" });
    expect(confirm.className).toContain("bg-destructive");
    // 停用/暂存点名照常渲染
    expect(screen.getByText("将停用 (2)")).toBeInTheDocument();
    expect(screen.getByText("将暂存 (1)")).toBeInTheDocument();
  });
});
