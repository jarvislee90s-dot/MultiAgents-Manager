// tests/healthCheckCard.test.tsx — 一致性体检卡片（spec §13 呈现侧）
// 覆盖（final review Finding 1 回归锁）：组头批量 a/b 后，needs_manual 结果行按
// ReconcileOutcome.message 前缀 "{ext_id} @ {tool_id}: {detail}" 回映行键转橙无按钮——
// 历史缺陷：解析正则 /^(\S+) @ (\S+)/ 未锚定冒号，\S+ 贪婪吞掉 "codex:" 的冒号，
// 产出 "codex:|skill-x" 永不等于 driftKey 的 "codex|skill-x"，批量标橙静默失效
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { HealthCheckCard } from "@/components/resources/HealthCheckCard";
import type { PresetHealth } from "@/types/preset";

const { invokeMock, toastMock } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
  // 终审 Minor #3：捕获批量汇总 toast，锁定 fixed / needs_manual / failed 三分计数
  toastMock: { success: vi.fn(), error: vi.fn(), warning: vi.fn(), info: vi.fn() },
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke: invokeMock }));
vi.mock("sonner", () => ({ toast: toastMock }));

// tests/setup.ts 未初始化 i18n，显式引入并固定中文（文案按 zh 断言）
import i18n from "@/i18n";

beforeAll(async () => {
  await i18n.changeLanguage("zh");
});

// 体检 fixture：一条 codex 的 L2 漂移（extensionId 与批量 message 前缀严格同源，
// 回映成功的唯一凭证就是行键 codex|skill-x 命中）
const HEALTH: PresetHealth = {
  invariants: [],
  stashPending: [],
  drift: [{ toolId: "codex", kind: "L2", extensionId: "skill-x", path: "/tmp/ssot/skill-x" }],
};

// 批量处置结果（终审 Minor #3 三分计数）：1 修复 + 1 needs_manual + 1 失败（两者皆非）——
// needs_manual 必须单列汇总，不得并入失败计数；message 前缀 "{ext_id} @ {tool_id}: {detail}"
// 同 Rust reconcile.rs where_at 格式，needs_manual 行仍须回映到漂移行键 codex|skill-x
const BATCH_MIXED = [
  { fixed: true, needsManual: false, message: "skill-ok @ codex: 已按账本修复磁盘" },
  { fixed: false, needsManual: true, message: "skill-x @ codex: 内容不一致，需人工处理" },
  { fixed: false, needsManual: false, message: "skill-bad @ codex: 处置失败" },
];

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockImplementation(async (cmd: string) => {
    if (cmd === "get_preset_health") return HEALTH;
    if (cmd === "list_frontmatter_suggestions") return [];
    if (cmd === "list_enabled_tools")
      return [
        {
          id: "codex",
          label: "Codex CLI",
          skillToggleSupported: true,
          mcpSupported: true,
          pluginSupported: true,
        },
      ];
    if (cmd === "reconcile_tool_batch") return BATCH_MIXED;
    return [];
  });
});

function renderCard() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  render(
    <QueryClientProvider client={queryClient}>
      <HealthCheckCard />
    </QueryClientProvider>
  );
}

describe("HealthCheckCard（批量 needs-manual 行回映，Finding 1 回归锁）", () => {
  it("批量 a 后 needs_manual 行转橙无按钮：message 前缀回映行键不被尾缀冒号破坏", async () => {
    renderCard();
    // 漂移行就绪：extensionId 去 skill- 前缀展示（"skill-x" → "x"）
    expect(await screen.findByText("x")).toBeInTheDocument();
    // 处置前：行带 a/b 处置按钮
    expect(screen.getByRole("button", { name: "以账本为准修磁盘" })).toBeInTheDocument();
    // 点组头批量 a（「全部按账本修磁盘」≠ 行级「以账本为准修磁盘」，exact 匹配区分）
    fireEvent.click(screen.getByRole("button", { name: "全部按账本修磁盘" }));
    // 行转 needs-manual 态：橙色「需人工处理」徽标出现，行级 a/b 按钮消失
    const badge = await screen.findByText("需人工处理");
    const row = badge.closest("div")!;
    expect(row).toHaveClass("bg-amber-500/5");
    expect(screen.queryByText("以账本为准修磁盘")).not.toBeInTheDocument();
    expect(screen.queryByText("以磁盘为准回写账本")).not.toBeInTheDocument();
    // 批量确实以该工具调用（映射的数据源）
    expect(invokeMock).toHaveBeenCalledWith("reconcile_tool_batch", { toolId: "codex", mode: "a" });
    // 三分汇总（终审 Minor #3）：fixed=1 / needs_manual=1 / failed=1 各自入账，
    // needs_manual 不再并入失败（旧两分实现这里会报「2 项失败」且无需人工计数）
    await waitFor(() => expect(toastMock.warning).toHaveBeenCalledTimes(1));
    const summary = toastMock.warning.mock.calls[0][0] as string;
    expect(summary).toContain("1 项成功");
    expect(summary).toContain("1 项失败");
    expect(summary).toContain("需人工 1 项");
  });

  it("暂存回移成功后走统一失效路径（终审 Minor #7）：与 invalidateAfterFix 同款四 key + 托盘同步", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_preset_health")
        return {
          ...HEALTH,
          stashPending: [
            {
              id: 1,
              toolId: "codex",
              skillName: "skill-a",
              stashedPath: "/tmp/mam/stash/skill-a",
              originalPath: "/tmp/native/skill-a",
              createdAt: "2026-09-15T00:00:00Z",
              restoredAt: null,
            },
          ],
        };
      if (cmd === "list_frontmatter_suggestions") return [];
      if (cmd === "list_enabled_tools")
        return [
          {
            id: "codex",
            label: "Codex CLI",
            skillToggleSupported: true,
            mcpSupported: true,
            pluginSupported: true,
          },
        ];
      if (cmd === "restore_stash_entry") return null;
      return [];
    });
    renderCard();
    // 残留暂存行就绪（zh 词条「回移」）→ 点回移
    fireEvent.click(await screen.findByRole("button", { name: "回移" }));
    // 回移成功 → 复用 invalidateAfterDisposition 的完整失效集合；
    // refresh_tray 是该集合的可观测代理断言（旧实现只失效 preset-health，无托盘同步 → 红）
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("refresh_tray", { presetsLabel: "预设" })
    );
  });
});
