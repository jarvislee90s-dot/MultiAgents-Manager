// tests/healthCheckCard.test.tsx — 一致性体检卡片（spec §13 呈现侧）
// 覆盖（final review Finding 1 回归锁）：组头批量 a/b 后，needs_manual 结果行按
// ReconcileOutcome.message 前缀 "{ext_id} @ {tool_id}: {detail}" 回映行键转橙无按钮——
// 历史缺陷：解析正则 /^(\S+) @ (\S+)/ 未锚定冒号，\S+ 贪婪吞掉 "codex:" 的冒号，
// 产出 "codex:|skill-x" 永不等于 driftKey 的 "codex|skill-x"，批量标橙静默失效
import { fireEvent, render, screen } from "@testing-library/react";
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { HealthCheckCard } from "@/components/resources/HealthCheckCard";
import type { PresetHealth } from "@/types/preset";

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: invokeMock }));

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

// 批量处置结果：needs_manual=true，message 尾缀带冒号 detail（Rust reconcile.rs where_at 格式）
const BATCH_NEEDS_MANUAL = [
  { fixed: false, needsManual: true, message: "skill-x @ codex: 内容不一致，需人工处理" },
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
    if (cmd === "reconcile_tool_batch") return BATCH_NEEDS_MANUAL;
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
  });
});
